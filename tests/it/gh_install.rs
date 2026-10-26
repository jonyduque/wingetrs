//! Tasks 210-211: install, upgrade and remove apps from GitHub releases.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(windows)]

use std::collections::{BTreeSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::bridge::BridgeJob;
use wingetrs::bridge::worker::ScopeArg;
use wingetrs::cache::DataDir;
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::cli::Cli;
use wingetrs::cli::install_cmd::{self, Flags, Interrupts, Stop, Ui};
use wingetrs::cli::winget_cmd::WingetFlows;
use wingetrs::config::Behavior;
use wingetrs::core::SourceKind;
use wingetrs::github::install::{ArpKey, ArpView, GhEnv, GhTarget, INNO_ARGS, parse_target};
use wingetrs::github::{GhError, GhRepo};
use wingetrs::http::Http;
use wingetrs::install::executor::LaunchGate;
use wingetrs::install::executor::Runners;
use wingetrs::install::ops::{self, OpsCtx, ResidueScan, Resolution};
use wingetrs::install::progress::{Phase, ProgressEvent};
use wingetrs::install::state::{AppRecord, StateDb};
use wingetrs::install::target::Candidate;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::installed::Hive;
use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
use wingetrs::tui::app::{Done, InstallFail};
use wingetrs::tui::picker::PickerItem;
use wingetrs::update::LockWait;
use wingetrs::winget_install::engine::UpgradeError;
use wingetrs::winget_install::launch::{Elevation, LaunchSpec, Launcher, Verb};
use wingetrs::winget_install::ops::{InstallReport, WingetOpsError};
use wingetrs::winget_install::orphans::Orphan;
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan};
use wingetrs::winget_install::select::{Refusal, ScopeWanted};
use wingetrs::winget_install::uninstall::{
    UninstallPlan, UninstallReport, UninstallStatus, UninstallTarget,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---- PE and archive builders ----

const IMAGE_END: usize = 0x400 + 0x1000;

/// A PE with one section; `overlay` follows the image; `image_text` is written inside it.
fn pe(overlay: &[u8], image_text: &[u8]) -> Vec<u8> {
    let mut h = vec![0u8; 0x400];
    h[0] = b'M';
    h[1] = b'Z';
    h[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    h[0x80..0x84].copy_from_slice(b"PE\0\0");
    h[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
    h[0x94..0x96].copy_from_slice(&0xF0u16.to_le_bytes());
    let opt = 0x98;
    h[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
    h[opt + 68..opt + 70].copy_from_slice(&3u16.to_le_bytes());
    h[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
    let table = opt + 0xF0;
    h[table..table + 8].copy_from_slice(b".text\0\0\0");
    h[table + 16..table + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    h[table + 20..table + 24].copy_from_slice(&0x400u32.to_le_bytes());
    h.resize(IMAGE_END, 0x90);
    h[0x600..0x600 + image_text.len()].copy_from_slice(image_text);
    h.extend_from_slice(overlay);
    h
}

fn program() -> Vec<u8> {
    pe(&[], b"")
}

/// An Inno Setup installer (its marker at the start of the overlay).
fn inno() -> Vec<u8> {
    pe(b"Inno Setup Setup Data (6.2.0)", b"")
}

/// An Inno Setup installer whose manifest asks for an administrator.
fn inno_admin() -> Vec<u8> {
    pe(
        b"Inno Setup Setup Data (6.2.0)",
        br#"<requestedExecutionLevel level="requireAdministrator"/>"#,
    )
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, bytes) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

// ---- fakes ----

#[derive(Debug, Clone, Copy)]
enum Answer {
    Code(i32),
    /// The launch failed with this OS error (1223 = UAC refused).
    Os(i32),
}

/// The installer: records each launch, answers in order and, when told to, writes its ARP key.
struct FakeLauncher {
    specs: Mutex<Vec<LaunchSpec>>,
    answers: Mutex<VecDeque<Answer>>,
    writes_key: bool,
    /// `DisplayName` of the key it writes.
    key_name: Mutex<String>,
    /// `DisplayVersion` of the key it writes.
    key_version: Mutex<String>,
    keys: Arc<Mutex<Vec<ArpKey>>>,
    /// Whether the launched file existed with the asset's bytes while it "ran".
    seen: Mutex<Vec<Option<String>>>,
}

impl Launcher for FakeLauncher {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32> {
        self.specs.lock().unwrap().push(spec.clone());
        self.seen
            .lock()
            .unwrap()
            .push(std::fs::read(&spec.program).ok().map(|b| sha_hex(&b)));
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Answer::Code(0));
        match answer {
            Answer::Os(e) => Err(std::io::Error::from_raw_os_error(e)),
            Answer::Code(c) => {
                if self.writes_key && (c == 0 || c == 3010) {
                    // Inno writes `Tool_is1`, replacing the one an older version left.
                    let mut keys = self.keys.lock().unwrap();
                    keys.retain(|k| k.key != "Tool_is1");
                    keys.push(ArpKey {
                        hive: Hive::User,
                        key: "Tool_is1".into(),
                        name: self.key_name.lock().unwrap().clone(),
                        version: Some(self.key_version.lock().unwrap().clone()),
                    });
                }
                Ok(c)
            }
        }
    }
}

struct FakeArp {
    keys: Arc<Mutex<Vec<ArpKey>>>,
}

const UNINSTALL: &str = r#""C:\Users\u\AppData\Local\Programs\Tool\unins000.exe""#;

impl ArpView for FakeArp {
    fn keys(&self) -> std::io::Result<Vec<ArpKey>> {
        Ok(self.keys.lock().unwrap().clone())
    }
    fn uninstall_string(&self, _: Hive, key: &str) -> Option<String> {
        (key == "Tool_is1").then(|| UNINSTALL.to_string())
    }
}

struct TestUi {
    interactive: bool,
    confirm: bool,
    pick: Option<usize>,
    picked: Mutex<Vec<Vec<String>>>,
    said: Mutex<Vec<String>>,
}

impl TestUi {
    fn new(interactive: bool, pick: Option<usize>) -> TestUi {
        TestUi {
            interactive,
            confirm: true,
            pick,
            picked: Mutex::new(Vec::new()),
            said: Mutex::new(Vec::new()),
        }
    }
    fn text(&self) -> String {
        self.said.lock().unwrap().join("\n")
    }
}

impl Ui for TestUi {
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn confirm(&self, _: &str) -> anyhow::Result<bool> {
        Ok(self.confirm)
    }
    fn pick(&self, items: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        self.picked
            .lock()
            .unwrap()
            .push(items.into_iter().map(|i| i.label).collect());
        Ok(self.pick)
    }
    fn pick_many(&self, _: Vec<PickerItem>, _: &[bool]) -> anyhow::Result<Option<Vec<usize>>> {
        Ok(None)
    }
    fn output(&self, text: &str) {
        self.said.lock().unwrap().push(text.to_string());
    }
    fn plan(&self, text: &str) {
        self.said.lock().unwrap().push(text.to_string());
    }
}

fn no_tty() -> TestUi {
    TestUi::new(false, None)
}

/// Only GitHub releases: winget is never asked.
struct GhFlows {
    api: String,
    launcher: FakeLauncher,
    arp: FakeArp,
    elevated: fn() -> std::io::Result<bool>,
    gate: LaunchGate,
    backups: PathBuf,
    /// The ARP uninstallers that ran (the targets of their plans).
    uninstalled: Arc<Mutex<Vec<UninstallTarget>>>,
    /// What each uninstaller answers, in order (`Ok` removes its key); none left = `Ok`.
    uninstall_answers: Arc<Mutex<VecDeque<Result<(), String>>>>,
}

fn arp_row(k: &ArpKey) -> ListRow {
    let tag = match k.hive {
        Hive::User => "hkcu",
        Hive::MachineX64 => "hklm64",
        Hive::MachineX86 => "hklm32",
    };
    ListRow {
        name: k.name.clone(),
        app_key: format!("arp/{tag}/{}", k.key),
        origin: "arp".into(),
        installed_version: k.version.clone(),
        ..Default::default()
    }
}

fn plenty(_: &Path) -> Option<u64> {
    Some(u64::MAX)
}

fn not_elevated() -> std::io::Result<bool> {
    Ok(false)
}

fn elevated() -> std::io::Result<bool> {
    Ok(true)
}

impl WingetFlows for GhFlows {
    fn behavior(&self) -> Behavior {
        Behavior::default()
    }
    async fn plan_install(
        &self,
        _: &Candidate,
        _: ScopeWanted,
    ) -> anyhow::Result<Result<PlanOrBridge, Refusal>> {
        unreachable!("winget")
    }
    fn gate(&self) -> LaunchGate {
        self.gate.clone()
    }
    fn progress(&self) -> wingetrs::cli::winget_cmd::Sink {
        Arc::new(|_: &ProgressEvent| {})
    }
    fn install(
        &self,
        _: &WingetPlan,
    ) -> wingetrs::cli::winget_cmd::Op<Result<InstallReport, WingetOpsError>> {
        unreachable!("winget")
    }
    async fn install_bridge<I: Interrupts>(
        &self,
        _: &dyn Ui,
        _: &Candidate,
        _: Option<ScopeArg>,
        _: Option<&str>,
        _: Flags,
        _: Stop<'_, I>,
    ) -> anyhow::Result<()> {
        unreachable!("winget")
    }
    /// The Windows list: the fake ARP's keys as they are now.
    fn installed(&self) -> anyhow::Result<Vec<ListRow>> {
        Ok(self.arp.keys.lock().unwrap().iter().map(arp_row).collect())
    }
    async fn plan_uninstall(
        &self,
        row: &ListRow,
        target: &UninstallTarget,
    ) -> Result<UninstallPlan, String> {
        Ok(UninstallPlan {
            target: target.clone(),
            name: row.name.clone(),
            version: row.installed_version.clone(),
            command: None,
            elevation: Elevation::Never,
            shared: None,
        })
    }
    fn lock_install(
        &self,
    ) -> wingetrs::cli::winget_cmd::Op<Result<wingetrs::cli::winget_cmd::Held, WingetOpsError>>
    {
        Box::new(|| Box::pin(async { Ok(Box::new(()) as wingetrs::cli::winget_cmd::Held) }))
    }
    /// The ARP uninstaller: a success removes its key from the fake ARP.
    fn uninstall(&self, plan: &UninstallPlan) -> wingetrs::cli::winget_cmd::Uninstaller {
        let (done, answers, keys) = (
            Arc::clone(&self.uninstalled),
            Arc::clone(&self.uninstall_answers),
            Arc::clone(&self.arp.keys),
        );
        let target = plan.target.clone();
        Box::new(move |_: &dyn Fn(&ProgressEvent)| {
            done.lock().unwrap().push(target.clone());
            answers.lock().unwrap().pop_front().unwrap_or(Ok(()))?;
            if let UninstallTarget::Arp { hive, key } = &target {
                keys.lock()
                    .unwrap()
                    .retain(|k| !(k.hive == *hive && k.key == *key));
            }
            Ok(UninstallReport {
                status: UninstallStatus::Removed,
                warning: None,
                reboot: None,
            })
        })
    }
    fn remove_registry_only(
        &self,
        _: &UninstallPlan,
        _: &str,
    ) -> wingetrs::cli::winget_cmd::RecordRemover {
        // Built with every removal; never chosen here.
        Box::new(|_: &dyn Fn(&ProgressEvent)| unreachable!("registry-only removal"))
    }
    fn free_space(&self) -> fn(&Path) -> Option<u64> {
        plenty
    }
    fn pins(&self) -> BTreeSet<String> {
        BTreeSet::new()
    }
    async fn plan_upgrade(
        &self,
        _: &ListRow,
        _: &str,
        _: ScopeWanted,
    ) -> anyhow::Result<(Result<PlanOrBridge, Refusal>, Option<UninstallPlan>)> {
        unreachable!("winget")
    }
    fn upgrade(
        &self,
        _: &WingetPlan,
        _: Option<&UninstallPlan>,
    ) -> wingetrs::cli::winget_cmd::Op<Result<InstallReport, UpgradeError>> {
        unreachable!("winget")
    }
    fn upgrade_bridge(&self, _: &BridgeJob, _: &mut dyn FnMut(&str)) -> Result<Done, InstallFail> {
        unreachable!("winget")
    }
    fn orphans(&self) -> anyhow::Result<Vec<Orphan>> {
        unreachable!("winget")
    }
    fn backups(&self) -> PathBuf {
        self.backups.clone()
    }
    fn remove_orphan(
        &self,
        _: &Orphan,
        _: &str,
        _: &dyn Fn(&ProgressEvent),
    ) -> Result<PathBuf, String> {
        unreachable!("winget")
    }
    fn elevated(&self) -> fn() -> std::io::Result<bool> {
        self.elevated
    }
    fn github(&self) -> Option<GhEnv<'_>> {
        Some(GhEnv {
            api: self.api.clone(),
            web: self.api.clone(),
            launcher: &self.launcher,
            arp: &self.arp,
        })
    }
}

// ---- world ----

fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

type Sink = Box<dyn Fn(&ProgressEvent) + Send + Sync>;

struct World {
    server: MockServer,
    _t: TempDir,
    data: DataDir,
    http: Http,
    catalog: Catalog,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    phases: Arc<Mutex<Vec<Phase>>>,
    sink: Sink,
    flows: GhFlows,
}

/// One release asset: name and bytes (the digest is theirs unless `digest` says otherwise).
struct A {
    name: &'static str,
    bytes: Vec<u8>,
    digest: Option<String>,
}

fn a(name: &'static str, bytes: Vec<u8>) -> A {
    A {
        name,
        bytes,
        digest: None,
    }
}

impl World {
    async fn new() -> World {
        World::with(Vec::new(), true).await
    }

    /// `answers`: what the installer returns, in order; `writes_key`: whether a successful
    /// run writes its ARP key.
    async fn with(answers: Vec<Answer>, writes_key: bool) -> World {
        World::with_keys(answers, writes_key, Vec::new()).await
    }

    /// [`World::with`] whose ARP also has `extra` before the install.
    async fn with_keys(answers: Vec<Answer>, writes_key: bool, extra: Vec<ArpKey>) -> World {
        let t = tempfile::tempdir().unwrap();
        let data = DataDir::new(t.path().join("root"));
        let layout = data.install_layout();
        std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
        std::fs::write(layout.launcher(), program()).unwrap();
        let server = MockServer::start().await;
        let mut before = vec![ArpKey {
            hive: Hive::MachineX64,
            key: "Other".into(),
            name: "Other app".into(),
            version: Some("3.0".into()),
        }];
        before.extend(extra);
        let keys = Arc::new(Mutex::new(before));
        let phases = Arc::new(Mutex::new(Vec::new()));
        let seen = phases.clone();
        let sink: Sink = Box::new(move |ev: &ProgressEvent| {
            if let ProgressEvent::Phase(p) = ev {
                seen.lock().unwrap().push(p.clone());
            }
        });
        World {
            flows: GhFlows {
                api: server.uri(),
                launcher: FakeLauncher {
                    specs: Mutex::new(Vec::new()),
                    answers: Mutex::new(answers.into()),
                    writes_key,
                    key_name: Mutex::new("Tool 1.0.0".into()),
                    key_version: Mutex::new("1.0.0".into()),
                    keys: keys.clone(),
                    seen: Mutex::new(Vec::new()),
                },
                arp: FakeArp { keys },
                elevated: not_elevated,
                gate: LaunchGate::default(),
                backups: t.path().join("backups"),
                uninstalled: Arc::new(Mutex::new(Vec::new())),
                uninstall_answers: Arc::new(Mutex::new(VecDeque::new())),
            },
            server,
            data,
            http: Http::new().unwrap(),
            catalog: Catalog::open_in_memory().unwrap(),
            env: FakeEnv::new(),
            shortcuts: FakeShortcuts::new(t.path().join("startmenu")),
            trash: FakeTrash::new(),
            registry: FakeRegistry::new(),
            phases,
            sink,
            _t: t,
        }
    }

    /// Serves release `tag` of `o/tool` (as latest) and each asset's bytes.
    async fn release(&self, tag: &str, assets: Vec<A>) {
        let mut listed = Vec::new();
        for x in &assets {
            let url = format!(
                "https://github.com/o/tool/releases/download/{tag}/{}",
                x.name
            );
            // `Some("")`: an old release, no digest published.
            let digest = match x.digest.as_deref() {
                Some("") => serde_json::Value::Null,
                Some(d) => d.into(),
                None => format!("sha256:{}", sha_hex(&x.bytes)).into(),
            };
            listed.push(serde_json::json!({
                "name": x.name,
                "size": x.bytes.len(),
                "digest": digest,
                "browser_download_url": url,
            }));
            Mock::given(method("GET"))
                .and(path(format!("/o/tool/releases/download/{tag}/{}", x.name)))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(x.bytes.clone()))
                .mount(&self.server)
                .await;
        }
        let body = serde_json::json!({
            "tag_name": tag,
            "prerelease": false,
            "draft": false,
            "immutable": false,
            "assets": listed,
        });
        for at in ["latest".to_string(), format!("tags/{tag}")] {
            Mock::given(method("GET"))
                .and(path(format!("/repos/o/tool/releases/{at}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
                .mount(&self.server)
                .await;
        }
    }

    /// Puts `tool` in scoop source `source` with `manifest`.
    fn bucket_app(&mut self, source: &str, manifest: serde_json::Value) {
        let pkg = NewPackage {
            id: "tool".into(),
            name: "tool".into(),
            version: "1.2.0".into(),
            raw: Some(manifest.to_string()),
            ..Default::default()
        };
        self.catalog
            .replace_source(
                source,
                SourceKind::Scoop,
                "1",
                1,
                &[pkg],
                &WingetMaps::default(),
            )
            .unwrap();
    }

    fn ctx(&self) -> OpsCtx<'_> {
        OpsCtx {
            data: &self.data,
            http: &self.http,
            catalog: &self.catalog,
            effects: Effects {
                env: &self.env,
                shortcuts: &self.shortcuts,
                trash: &self.trash,
                registry: &self.registry,
            },
            wait: LockWait::NoWait,
            progress: &*self.sink,
            in_use: &nothing_in_use,
            residue: ResidueScan::none(),
            msiexec: wingetrs::install::executor::Msiexec::DISABLED,
            launch: Default::default(),
            runners: Runners::disabled(),
        }
    }

    async fn run(&self, ui: &dyn Ui, args: &[&str]) -> anyhow::Result<()> {
        let cli =
            Cli::try_parse_from(std::iter::once("wingetrs").chain(args.iter().copied())).unwrap();
        let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        install_cmd::execute(&self.ctx(), ui, cli.command.unwrap(), &mut rx, &self.flows).await
    }

    fn layout(&self) -> wingetrs::install::layout::Layout {
        self.data.install_layout()
    }

    fn record(&self, app: &str) -> Option<AppRecord> {
        let state = ops::read_state(&self.layout()).unwrap();
        state.get(app).unwrap().map(|(r, _)| r)
    }

    fn install_json(&self, app: &str) -> serde_json::Value {
        let p = self.layout().current(app).join("install.json");
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
    }

    fn launches(&self) -> Vec<LaunchSpec> {
        self.flows.launcher.specs.lock().unwrap().clone()
    }

    async fn api_requests(&self) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().starts_with("/repos/"))
            .count()
    }

    async fn requests(&self) -> usize {
        self.server.received_requests().await.unwrap().len()
    }
}

const ZIP: &str = "tool-1.2.0-windows.zip";

fn tool_zip() -> Vec<u8> {
    zip_of(&[
        ("tool-1.2.0/tool.exe", &program()),
        ("tool-1.2.0/README.md", b"leia"),
    ])
}

const SETUP: &str = "tool-1.0.0-windows-amd64-instalador.exe";
const PLAIN: &str = "tool-1.0.0-windows-amd64.exe";

/// The eprocia shape: an installer and the bare program of the same build, plus checksums.
async fn installer_release(w: &World, installer: Vec<u8>) {
    w.release(
        "v1.0.0",
        vec![
            a(SETUP, installer),
            a(PLAIN, program()),
            a("SHA256SUMS.txt", b"x".to_vec()),
        ],
    )
    .await;
}

const YES_UNREVIEWED: [&str; 4] = ["install", "gh:o/tool", "--yes", "--allow-unreviewed"];

// ---- tests ----

#[test]
fn gh_url_forms_parse() {
    let ok = |s: &str| -> (String, String, Option<String>) {
        let t: GhTarget = parse_target(s).unwrap().unwrap();
        (t.repo.owner, t.repo.repo, t.tag)
    };
    let repo = |o: &str, r: &str, tag: Option<&str>| {
        (o.to_string(), r.to_string(), tag.map(str::to_string))
    };
    let accepted = [
        ("gh:o/r", repo("o", "r", None)),
        ("gh:o/r@v1.0", repo("o", "r", Some("v1.0"))),
        (
            "https://github.com/jonyduque/eprocia",
            repo("jonyduque", "eprocia", None),
        ),
        (
            "https://github.com/jonyduque/eprocia/",
            repo("jonyduque", "eprocia", None),
        ),
        (
            "https://github.com/jonyduque/eprocia.git",
            repo("jonyduque", "eprocia", None),
        ),
        (
            "https://www.github.com/jonyduque/eprocia",
            repo("jonyduque", "eprocia", None),
        ),
        ("HTTPS://GitHub.com/o/r", repo("o", "r", None)),
        (
            "https://github.com/o/r/releases/tag/v0.3.0",
            repo("o", "r", Some("v0.3.0")),
        ),
        (
            "https://github.com/o/r/releases/tag/v0.3.0/",
            repo("o", "r", Some("v0.3.0")),
        ),
        (
            "https://github.com/o.x/r_y-z/releases/tag/release%2B1",
            repo("o.x", "r_y-z", Some("release+1")),
        ),
    ];
    for (input, want) in accepted {
        assert_eq!(ok(input), want, "{input}");
    }
    let refused = [
        "https://github.com/o/r/issues/1",
        "https://github.com/o/r/blob/main/x.rs",
        "https://github.com/o",
        "https://github.com/o/r/releases",
        "http://github.com/o/r",
        "https://github.com/o/r?tab=readme",
        "https://github.com/o w/r",
        "https://github.com/o/r/releases/tag/v1%2F2",
        "https://github.com/o/r/releases/tag/",
        "gh:o",
        "gh:o/r/x",
        "gh:o/r@",
        "gh:../r",
        "gh:o/r$",
    ];
    for input in refused {
        assert!(
            matches!(parse_target(input), Some(Err(_))),
            "{input}: {:?}",
            parse_target(input)
        );
    }
    for input in [
        "https://x@github.com/o/r",
        "https://github.com@evil.com/o/r",
        "https://github.com:443/o/r",
    ] {
        assert!(!matches!(parse_target(input), Some(Ok(_))), "{input}");
    }
    for input in [
        "ripgrep",
        "extras/vscode",
        "Git.Git",
        "https://example.com/o/r",
        "https://github.com.evil.example/o/r",
    ] {
        assert!(parse_target(input).is_none(), "{input}");
    }
    let t = parse_target("gh:Some/Repo").unwrap().unwrap();
    assert_eq!(t.bucket(), "github:Some/Repo");
    assert_eq!(
        wingetrs::github::install::app_name(&GhRepo {
            owner: "Some".into(),
            repo: "Repo".into()
        })
        .unwrap(),
        "repo"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_zip_installs_with_shim() {
    let w = World::new().await;
    w.release(
        "v1.2.0",
        vec![
            a(ZIP, tool_zip()),
            a("tool-1.2.0-linux-x86_64.tar.gz", b"linux".to_vec()),
        ],
    )
    .await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    let rec = w.record("tool").unwrap();
    assert_eq!(rec.bucket, "github:o/tool");
    assert_eq!(rec.version, "1.2.0");
    assert!(w.layout().shims().join("tool.shim").exists());
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(w.layout().current("tool").join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(m["extract_dir"], "tool-1.2.0");
    assert_eq!(m["bin"], serde_json::json!(["tool.exe"]));
    assert_eq!(m["hash"], format!("sha256:{}", sha_hex(&tool_zip())));
    assert_eq!(
        m["url"],
        "https://github.com/o/tool/releases/download/v1.2.0/tool-1.2.0-windows.zip"
            .replace("https://github.com", &w.server.uri())
    );
    assert!(w.launches().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_offers_bucket_manifest_first() {
    let mut w = World::new().await;
    w.bucket_app(
        "scoop:extras",
        serde_json::json!({
            "version": "1.2.0",
            "architecture": {"64bit": {
                "url": "https://github.com/O/Tool/releases/download/v1.2.0/tool.zip",
                "hash": "a".repeat(64),
            }},
            "bin": "tool.exe",
        }),
    );
    w.bucket_app(
        "scoop:main",
        serde_json::json!({
            "version": "1.2.0",
            "url": ["https://github.com/o/tool/releases/download/v1.2.0/tool.zip"],
            "hash": "a".repeat(64),
            "bin": "tool.exe",
        }),
    );
    // Another repository's manifest is not offered.
    w.bucket_app(
        "scoop:other",
        serde_json::json!({
            "version": "1.2.0",
            "url": "https://github.com/o/tool2/releases/download/v1.2.0/tool.zip",
            "hash": "a".repeat(64),
            "bin": "tool.exe",
        }),
    );
    // A terminal: the bucket manifests and the release are offered; Esc cancels.
    let ui = TestUi::new(true, None);
    w.run(&ui, &["install", "gh:o/tool"]).await.unwrap();
    assert_eq!(
        ui.picked.lock().unwrap().clone(),
        vec![vec![
            "manifesto do bucket extras/tool (revisado)".to_string(),
            "manifesto do bucket main/tool (revisado)".to_string(),
            "release direto (origem não revisada)".to_string(),
        ]]
    );
    assert_eq!(w.api_requests().await, 0);
    // --yes without a terminal: the bucket manifest, never the release.
    let ui = no_tty();
    w.run(&ui, &["install", "gh:o/tool", "--yes", "--dry-run"])
        .await
        .unwrap();
    assert!(ui.text().contains("extras"), "{}", ui.text());
    assert_eq!(w.api_requests().await, 0);
    // The release, when the user picks it.
    w.release("v1.2.0", vec![a(ZIP, tool_zip())]).await;
    let ui = TestUi::new(true, Some(2));
    w.run(&ui, &["install", "gh:o/tool", "--dry-run"])
        .await
        .unwrap();
    assert_eq!(w.api_requests().await, 1);
    // An explicit tag asks for the release: the bucket is skipped with a line.
    let ui = TestUi::new(true, None);
    w.run(&ui, &["install", "gh:o/tool@v1.2.0", "--dry-run"])
        .await
        .unwrap();
    assert_eq!(ui.picked.lock().unwrap().len(), 0);
    assert!(
        ui.text()
            .contains("extras/tool vem de um bucket, mas você pediu o release"),
        "{}",
        ui.text()
    );
    assert_eq!(w.api_requests().await, 2);
    assert!(
        ui.text().contains("origem não revisada: o/tool"),
        "{}",
        ui.text()
    );
    assert_eq!(w.record("tool"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_ambiguous_without_tty_lists_assets() {
    let w = World::new().await;
    let gui = zip_of(&[("tool-gui.exe", &program())]);
    w.release(
        "v1.2.0",
        vec![
            a("tool-cli-windows.zip", tool_zip()),
            a("tool-gui-windows.zip", gui),
        ],
    )
    .await;
    let e = w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap_err();
    let text = e.to_string();
    assert!(text.contains("tool-cli-windows.zip"), "{text}");
    assert!(text.contains("tool-gui-windows.zip"), "{text}");
    assert!(text.contains("--asset"), "{text}");
    assert_eq!(w.record("tool"), None);
    // A terminal: the user picks among them.
    let ui = TestUi::new(true, Some(1));
    w.run(&ui, &["install", "gh:o/tool", "--dry-run"])
        .await
        .unwrap();
    assert_eq!(
        ui.picked.lock().unwrap().clone(),
        vec![vec![
            "tool-cli-windows.zip".to_string(),
            "tool-gui-windows.zip".to_string()
        ]]
    );
    assert!(
        ui.text().contains("Arquivo: tool-gui-windows.zip"),
        "{}",
        ui.text()
    );
    // A package the user picks is still out of scope.
    let w = World::new().await;
    w.release(
        "v1.2.0",
        vec![
            a("tool-cli-windows.zip", tool_zip()),
            a("tool-gui-windows.msi", b"msi".to_vec()),
        ],
    )
    .await;
    let ui = TestUi::new(true, Some(1));
    let e = w
        .run(&ui, &["install", "gh:o/tool", "--dry-run"])
        .await
        .unwrap_err();
    assert!(
        e.to_string().contains("instalador MSI: fora do escopo"),
        "{e}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_asset_flag_picks_by_name() {
    let w = World::new().await;
    let gui = zip_of(&[("tool-gui.exe", &program())]);
    w.release(
        "v1.2.0",
        vec![
            a("tool-cli-windows.zip", tool_zip()),
            a("tool-gui-windows.zip", gui),
        ],
    )
    .await;
    let e = w
        .run(
            &no_tty(),
            &[
                "install",
                "gh:o/tool",
                "--asset",
                "nope.zip",
                "--yes",
                "--allow-unreviewed",
            ],
        )
        .await
        .unwrap_err();
    assert!(e.to_string().contains("tool-gui-windows.zip"), "{e}");
    let old = World::new().await;
    old.release(
        "v1.2.0",
        vec![A {
            name: "tool-old-windows.zip",
            bytes: tool_zip(),
            digest: Some(String::new()),
        }],
    )
    .await;
    let e = old
        .run(
            &no_tty(),
            &[
                "install",
                "gh:o/tool",
                "--asset",
                "tool-old-windows.zip",
                "--yes",
                "--allow-unreviewed",
            ],
        )
        .await
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "o arquivo tool-old-windows.zip não tem SHA-256 publicado (release antigo); o wingetrs não instala sem conferir"
    );
    let e = w
        .run(&no_tty(), &["install", "ripgrep", "--asset", "x.zip"])
        .await
        .unwrap_err();
    assert!(e.to_string().contains("--asset só vale"), "{e}");
    w.run(
        &no_tty(),
        &[
            "install",
            "gh:o/tool",
            "--asset",
            "tool-gui-windows.zip",
            "--yes",
            "--allow-unreviewed",
        ],
    )
    .await
    .unwrap();
    assert_eq!(w.record("tool").unwrap().version, "1.2.0");
    assert!(w.layout().shims().join("tool-gui.shim").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_requires_allow_unreviewed() {
    let w = World::new().await;
    w.release("v1.2.0", vec![a(ZIP, tool_zip())]).await;
    let ui = no_tty();
    let e = w
        .run(&ui, &["install", "gh:o/tool", "--yes"])
        .await
        .unwrap_err();
    assert!(e.to_string().contains("--allow-unreviewed"), "{e}");
    assert!(
        ui.text().contains("origem não revisada: o/tool"),
        "{}",
        ui.text()
    );
    assert!(ui.text().contains("release imutável: não"), "{}", ui.text());
    assert_eq!(w.record("tool"), None);
    assert!(!w.layout().app_dir("tool").exists());
    // The dry run shows the file, URL and digest without the flag.
    let ui = no_tty();
    w.run(&ui, &["install", "gh:o/tool", "--dry-run"])
        .await
        .unwrap();
    let text = ui.text();
    assert!(text.contains(ZIP), "{text}");
    assert!(text.contains(&sha_hex(&tool_zip())), "{text}");
    assert!(text.contains("/o/tool/releases/download/v1.2.0/"), "{text}");
    // The installer path refuses the same way.
    let w = World::new().await;
    installer_release(&w, inno()).await;
    let e = w
        .run(&no_tty(), &["install", "gh:o/tool", "--yes"])
        .await
        .unwrap_err();
    assert!(e.to_string().contains("--allow-unreviewed"), "{e}");
    assert!(w.launches().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_name_clash_refused() {
    let w = World::new().await;
    w.release("v1.2.0", vec![a(ZIP, tool_zip())]).await;
    let db = w.layout().state_db();
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let mut state = StateDb::open(&db).unwrap();
    let mut rec = AppRecord {
        app: "tool".into(),
        bucket: "main".into(),
        version: "0.9".into(),
        arch: None,
        installed_at: 0,
        manifest_sha256: String::new(),
    };
    state.record(&rec, &[]).unwrap();
    let e = w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap_err();
    assert!(
        e.to_string().contains("já existe um app tool de main"),
        "{e}"
    );
    // Already installed from the same release repository.
    rec.bucket = "github:o/tool".into();
    state.record(&rec, &[]).unwrap();
    let e = w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap_err();
    assert!(e.to_string().contains("já está instalado"), "{e}");
    assert_eq!(w.requests().await, 0);
    // `plan_for_raw` refuses the same clash.
    rec.bucket = "main".into();
    state.record(&rec, &[]).unwrap();
    let raw = serde_json::json!({"version": "1.2.0", "url": "https://x/t.zip", "hash": "a".repeat(64), "bin": "t.exe"}).to_string();
    let e = ops::plan_for_raw(&w.ctx(), "tool", "github:o/tool", &raw, false).unwrap_err();
    assert!(
        e.to_string().contains("já existe um app tool de main"),
        "{e}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_digest_mismatch_installs_nothing() {
    let w = World::new().await;
    w.release(
        "v1.2.0",
        vec![A {
            name: ZIP,
            bytes: tool_zip(),
            digest: Some(format!("sha256:{}", "0".repeat(64))),
        }],
    )
    .await;
    assert!(w.run(&no_tty(), &YES_UNREVIEWED).await.is_err());
    assert_eq!(w.record("tool"), None);
    assert!(!w.layout().app_dir("tool").exists());
    assert!(!w.layout().shims().join("tool.shim").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_for_and_plan_for_raw_same_plan() {
    let mut w = World::new().await;
    let raw = serde_json::json!({
        "version": "1.2.0",
        "url": "https://example.invalid/tool.zip",
        "hash": format!("sha256:{}", "a".repeat(64)),
        "extract_dir": "tool-1.2.0",
        "bin": ["tool.exe"],
        "env_add_path": ".",
    });
    w.bucket_app("scoop:main", raw.clone());
    let ctx = w.ctx();
    let Resolution::One(cand) = ops::resolve_target(&ctx, "tool", None).unwrap() else {
        panic!("tool not found");
    };
    let mut via_catalog = ops::plan_for(&ctx, &cand, false).unwrap();
    assert_eq!(via_catalog.len(), 1);
    let raw_text = w
        .catalog
        .get("scoop:main", "tool")
        .unwrap()
        .unwrap()
        .raw
        .unwrap();
    let via_raw = ops::plan_for_raw(&ctx, "tool", "main", &raw_text, false).unwrap();
    assert_eq!(via_catalog.pop().unwrap(), via_raw);
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_installer_is_default() {
    for code in [0, 3010] {
        let w = World::with(vec![Answer::Code(code)], true).await;
        installer_release(&w, inno()).await;
        let ui = no_tty();
        w.run(&ui, &YES_UNREVIEWED).await.unwrap();
        let launches = w.launches();
        assert_eq!(launches.len(), 1, "{code}");
        assert_eq!(launches[0].params, INNO_ARGS, "{code}");
        assert_eq!(launches[0].verb, Verb::Open, "{code}");
        assert_eq!(
            launches[0].program.file_name().unwrap().to_str().unwrap(),
            SETUP
        );
        // The launched copy held the verified bytes.
        assert_eq!(
            w.flows.launcher.seen.lock().unwrap()[0],
            Some(sha_hex(&inno()))
        );
        let rec = w.record("tool").unwrap();
        assert_eq!(rec.bucket, "github:o/tool");
        assert_eq!(rec.version, "1.0.0");
        let j = w.install_json("tool");
        assert_eq!(j["mode"], "installer", "{code}");
        assert_eq!(j["arp"]["key"], "Tool_is1");
        assert_eq!(j["arp"]["hive"], "user");
        assert_eq!(j["uninstall_string"], UNINSTALL);
        assert!(!ui.text().contains("o instalador falhou"), "{}", ui.text());
        assert!(
            ui.text()
                .contains("Se falhar: instala só tool-1.0.0-windows-amd64.exe"),
            "{}",
            ui.text()
        );
        assert!(
            !w.layout()
                .shims()
                .join(format!("{}.shim", &PLAIN[..PLAIN.len() - 4]))
                .exists()
        );
    }
}

// One thread: the job log is read from this thread's events.
#[tokio::test]
async fn gh_installer_failure_falls_back_to_exe() {
    let cases = [
        (Answer::Code(1), "saiu com o código 1"),
        (Answer::Os(1223), "autorização de administrador recusada"),
        (Answer::Os(2), "não iniciou: "),
    ];
    for (answer, reason) in cases {
        let log = crate::spans::Log::default();
        let installed = log.install();
        let w = World::with(vec![answer], true).await;
        installer_release(&w, inno()).await;
        let ui = no_tty();
        w.run(&ui, &YES_UNREVIEWED).await.unwrap();
        drop(installed);
        assert_eq!(w.launches().len(), 1, "{reason}");
        let text = ui.text();
        assert!(
            text.contains(&format!("o instalador falhou ({reason}")),
            "{text}"
        );
        assert!(text.contains("); instalando só o executável"), "{text}");
        let rec = w.record("tool").unwrap();
        assert_eq!(rec.bucket, "github:o/tool");
        assert!(
            w.layout()
                .shims()
                .join("tool-1.0.0-windows-amd64.shim")
                .exists(),
            "{reason}"
        );
        assert_eq!(w.install_json("tool").get("mode"), None, "{reason}");
        // Both attempts in the job log: the installer failed, the program installed.
        let logged = log.text();
        let ends: Vec<&str> = logged.lines().filter(|l| l.contains("job.end")).collect();
        assert_eq!(ends.len(), 2, "{logged}");
        assert!(
            ends[0].contains("falhou") && ends[0].contains(reason.trim_end()),
            "{logged}"
        );
        assert!(ends[1].contains("instalado"), "{logged}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_fallback_failure_installs_nothing() {
    let w = World::with(vec![Answer::Code(1)], true).await;
    w.release(
        "v1.0.0",
        vec![
            a(SETUP, inno()),
            // Fetched only by the fallback, and it does not match its digest.
            A {
                name: "tool-1.0.0-windows-amd64.zip",
                bytes: tool_zip(),
                digest: Some(format!("sha256:{}", "0".repeat(64))),
            },
        ],
    )
    .await;
    let e = w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap_err();
    assert!(
        e.to_string().contains(
            "o instalador falhou (saiu com o código 1) e a instalação só do executável também falhou"
        ),
        "{e:#}"
    );
    assert_eq!(w.record("tool"), None);
    assert!(!w.layout().app_dir("tool").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_installer_without_arp_is_success_with_warning() {
    let w = World::with(vec![Answer::Code(0)], false).await;
    installer_release(&w, inno()).await;
    let ui = no_tty();
    w.run(&ui, &YES_UNREVIEWED).await.unwrap();
    assert_eq!(w.launches().len(), 1);
    let text = ui.text();
    assert!(
        text.contains(
            "o instalador não se registrou em Programas instalados; para remover, use o desinstalador do próprio programa"
        ),
        "{text}"
    );
    assert!(!text.contains("o instalador falhou"), "{text}");
    let j = w.install_json("tool");
    assert_eq!(j["mode"], "installer");
    assert_eq!(j["arp"], serde_json::Value::Null);
    assert_eq!(w.record("tool").unwrap().bucket, "github:o/tool");
    // The program was not installed a second time.
    assert!(
        !w.layout()
            .shims()
            .join("tool-1.0.0-windows-amd64.shim")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_installer_reusing_existing_arp_key_is_recorded() {
    // An update: the key an older version left gets a new `DisplayVersion`.
    let old = ArpKey {
        hive: Hive::User,
        key: "Tool_is1".into(),
        name: "Tool 0.9".into(),
        version: Some("0.9".into()),
    };
    let w = World::with_keys(vec![Answer::Code(0)], true, vec![old]).await;
    installer_release(&w, inno()).await;
    let ui = no_tty();
    w.run(&ui, &YES_UNREVIEWED).await.unwrap();
    let j = w.install_json("tool");
    assert_eq!(j["mode"], "installer");
    assert_eq!(j["arp"]["key"], "Tool_is1");
    assert_eq!(j["uninstall_string"], UNINSTALL);
    assert!(!ui.text().contains("não se registrou"), "{}", ui.text());
    // Only the version tells it: a `DisplayName` without the app's name.
    let old = ArpKey {
        hive: Hive::User,
        key: "Tool_is1".into(),
        name: "Ferramenta 0.9".into(),
        version: Some("0.9".into()),
    };
    let w = World::with_keys(vec![Answer::Code(0)], true, vec![old]).await;
    *w.flows.launcher.key_name.lock().unwrap() = "Ferramenta 1.0".into();
    installer_release(&w, inno()).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    assert_eq!(w.install_json("tool")["arp"]["key"], "Tool_is1");
    // A reinstall that leaves its key as it was: the key naming the app (any case).
    let same = ArpKey {
        hive: Hive::MachineX86,
        key: "{ABC}".into(),
        name: "TOOL desktop".into(),
        version: Some("1.0.0".into()),
    };
    let w = World::with_keys(vec![Answer::Code(0)], false, vec![same]).await;
    installer_release(&w, inno()).await;
    let ui = no_tty();
    w.run(&ui, &YES_UNREVIEWED).await.unwrap();
    let j = w.install_json("tool");
    assert_eq!(j["arp"]["key"], "{ABC}");
    assert_eq!(j["arp"]["hive"], "machine32");
    assert!(!ui.text().contains("não se registrou"), "{}", ui.text());
    assert!(!ui.text().contains("o instalador falhou"), "{}", ui.text());
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_installer_without_program_fails_clean() {
    let w = World::with(vec![Answer::Code(1)], true).await;
    w.release("v1.0.0", vec![a(SETUP, inno())]).await;
    let e = w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap_err();
    assert!(
        e.to_string()
            .contains("o instalador falhou: saiu com o código 1"),
        "{e}"
    );
    assert_eq!(w.launches().len(), 1);
    assert_eq!(w.record("tool"), None);
    assert!(!w.layout().app_dir("tool").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_installer_never_elevated() {
    // An installer whose manifest asks for an administrator: launched with `open`, its own
    // UAC prompt announced by the phase.
    let w = World::with(vec![Answer::Code(0)], true).await;
    installer_release(&w, inno_admin()).await;
    let ui = no_tty();
    w.run(&ui, &YES_UNREVIEWED).await.unwrap();
    assert!(
        ui.text()
            .contains("O instalador pode pedir permissão de administrador"),
        "{}",
        ui.text()
    );
    let phases = w.phases.lock().unwrap().clone();
    assert!(
        phases.iter().any(
            |p| matches!(p, Phase::ReleaseInstaller { name, asks_admin: true } if name == "tool")
        ),
        "{phases:?}"
    );
    // A plain installer: the running phase, never the UAC one.
    let w2 = World::with(vec![Answer::Code(1)], true).await;
    installer_release(&w2, inno()).await;
    w2.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    let phases = w2.phases.lock().unwrap().clone();
    assert!(
        phases.iter().any(
            |p| matches!(p, Phase::ReleaseInstaller { name, asks_admin: false } if name == "tool")
        ),
        "{phases:?}"
    );
    assert!(
        !phases.iter().any(|p| matches!(
            p,
            Phase::ReleaseInstaller {
                asks_admin: true,
                ..
            }
        )),
        "{phases:?}"
    );
    for spec in w.launches().iter().chain(w2.launches().iter()) {
        assert_eq!(spec.verb, Verb::Open);
    }
    assert_eq!(w.launches().len() + w2.launches().len(), 2);
    // From an administrator terminal the installer never runs.
    let mut w3 = World::with(vec![Answer::Code(0)], true).await;
    w3.flows.elevated = elevated;
    installer_release(&w3, inno()).await;
    let e = w3.run(&no_tty(), &YES_UNREVIEWED).await.unwrap_err();
    assert!(e.to_string().contains("como administrador"), "{e}");
    assert!(w3.launches().is_empty());
    assert_eq!(w3.record("tool"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_scope_machine_refused() {
    let w = World::new().await;
    installer_release(&w, inno()).await;
    let e = w
        .run(
            &no_tty(),
            &[
                "install",
                "gh:o/tool",
                "--scope",
                "machine",
                "--yes",
                "--allow-unreviewed",
            ],
        )
        .await
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("releases do GitHub instalam só para o usuário"),
        "{e}"
    );
    assert_eq!(w.requests().await, 0);
    assert!(w.launches().is_empty());
}

#[test]
fn gh_installer_phase_says_it_cannot_be_cancelled() {
    use wingetrs::install::progress::{ELEVATION_NOTICE, phase_status, phase_title};
    let no_cancel = "o instalador não pode ser cancelado pelo wingetrs; feche a janela dele";
    for asks_admin in [false, true] {
        let p = Phase::ReleaseInstaller {
            name: "tool".into(),
            asks_admin,
        };
        let title = phase_title(&p);
        assert!(title.contains(no_cancel), "{title}");
        assert!(
            title.contains("instalador de tool rodando")
                || title.contains("Instalador de tool rodando"),
            "{title}"
        );
        assert_eq!(title.contains(ELEVATION_NOTICE), asks_admin, "{title}");
        let status = phase_status(&p, std::time::Duration::from_secs(5));
        assert!(
            status.contains(no_cancel) && status.contains("00:05"),
            "{status}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_installer_block_matches_scoop_installer_file() {
    use wingetrs::cli::output::{plan_third_party, third_party_block};
    let mut w = World::with(vec![Answer::Code(0)], true).await;
    installer_release(&w, inno()).await;
    let ui = no_tty();
    w.run(&ui, &["install", "gh:o/tool", "--dry-run"])
        .await
        .unwrap();
    // The same installer as a bucket's `installer.file`.
    let args: Vec<&str> = INNO_ARGS.split_whitespace().collect();
    w.bucket_app(
        "scoop:main",
        serde_json::json!({
            "version": "1.0.0",
            "url": format!("https://example.invalid/{SETUP}"),
            "hash": "a".repeat(64),
            "installer": {"file": SETUP, "args": args},
        }),
    );
    let ctx = w.ctx();
    let Resolution::One(cand) = ops::resolve_target(&ctx, "tool", None).unwrap() else {
        panic!("tool not found");
    };
    let plan = ops::plan_for(&ctx, &cand, false).unwrap().pop().unwrap();
    let block = third_party_block(&plan_third_party(
        &plan,
        &wingetrs::install::script_ast::LexerOnly,
    ));
    assert!(!block.is_empty());
    assert!(
        ui.text().contains(&block),
        "{}
---
{block}",
        ui.text()
    );
    assert!(w.launches().is_empty());
}

#[test]
fn gh_arp_name_rule_is_whole_word_and_unique() {
    use wingetrs::github::install::{installed_key, names_app};
    let key = |k: &str, name: &str| ArpKey {
        hive: Hive::MachineX64,
        key: k.into(),
        name: name.into(),
        version: Some("1".into()),
    };
    // Nothing new or changed: only the name can tell.
    let pick = |keys: Vec<ArpKey>, app: &str| installed_key(&keys.clone(), keys, app, "1.0.0");
    let got = pick(vec![key("gh", "GitHub Desktop"), key("git", "Git")], "git");
    assert_eq!(got.map(|k| k.key).as_deref(), Some("git"));
    assert_eq!(
        pick(vec![key("gh", "GitHub Desktop"), key("gt", "Gitea")], "git"),
        None
    );
    assert_eq!(
        pick(vec![key("a", "Foo"), key("b", "Foo Tools")], "foo"),
        None
    );
    // Unnamed candidates: only the one candidate; a changed one only at the release's version.
    let run =
        |before: Vec<ArpKey>, after: Vec<ArpKey>| installed_key(&before, after, "tool", "1.0.0");
    let other = key("o", "Other app");
    assert_eq!(
        run(
            vec![other.clone()],
            vec![other.clone(), key("n1", "Setup A"), key("n2", "Setup B")]
        ),
        None
    );
    assert_eq!(
        run(
            vec![other.clone()],
            vec![other.clone(), key("n1", "Ferramenta")]
        )
        .map(|k| k.key)
        .as_deref(),
        Some("n1")
    );
    let edge = |v: &str| ArpKey {
        version: Some(v.into()),
        ..key("edge", "Microsoft Edge")
    };
    assert_eq!(run(vec![edge("120.0")], vec![edge("121.0")]), None);
    assert_eq!(
        run(vec![edge("0.9")], vec![edge("1.0.0")])
            .map(|k| k.key)
            .as_deref(),
        Some("edge")
    );
    // An unchanged key is taken by its name only at the release's version.
    let desktop = ArpKey {
        version: Some("3.4".into()),
        ..key("gd", "GitHub Desktop")
    };
    assert_eq!(
        installed_key(
            std::slice::from_ref(&desktop),
            vec![desktop.clone()],
            "desktop",
            "1.0.0"
        ),
        None
    );
    let reinstall = ArpKey {
        version: Some("1.0.0".into()),
        ..key("abc", "TOOL desktop")
    };
    assert_eq!(
        run(vec![reinstall.clone()], vec![reinstall])
            .map(|k| k.key)
            .as_deref(),
        Some("abc")
    );
    // A changed key with the name: only at the release's version (repo `edge`).
    let ms_edge = |v: &str| ArpKey {
        version: Some(v.into()),
        ..key("edge", "Microsoft Edge")
    };
    assert_eq!(
        installed_key(&[ms_edge("120.0")], vec![ms_edge("121.0")], "edge", "1.0.0"),
        None
    );
    assert_eq!(
        installed_key(&[ms_edge("0.9")], vec![ms_edge("1.0.0")], "edge", "1.0.0")
            .map(|k| k.key)
            .as_deref(),
        Some("edge")
    );
    // A new unnamed key and a changed one: two candidates, none is taken.
    assert_eq!(
        run(
            vec![edge("0.9")],
            vec![edge("1.0.0"), key("n1", "Ferramenta")]
        ),
        None
    );
    for (name, app, want) in [
        ("Git", "git", true),
        ("git-lfs", "git", true),
        ("Tool 1.0.0", "tool", true),
        ("x.TOOL", "tool", true),
        ("my-tool helper", "my-tool", true),
        ("GitHub Desktop", "git", false),
        ("Gitea", "git", false),
        ("Toolbox", "tool", false),
        ("Multitool", "tool", false),
        ("Git", "", false),
    ] {
        assert_eq!(names_app(name, app), want, "{name} / {app}");
    }
}

const EPROCIA: [&str; 5] = [
    "eprocia-0.3.0-windows-amd64-instalador.exe",
    "eprocia-0.3.0-windows-amd64.exe",
    "install.ps1",
    "LICENSE",
    "SHA256SUMS.txt",
];

#[test]
fn gh_pick_only_installable_formats() {
    use wingetrs::github::Asset;
    use wingetrs::github::pick::{Pick, pick};
    let assets: Vec<Asset> = EPROCIA
        .iter()
        .enumerate()
        .map(|(i, n)| Asset {
            name: n.to_string(),
            size: 10,
            sha256: [i as u8; 32],
            url: format!("https://github.com/jonyduque/eprocia/releases/download/v0.3.0/{n}"),
        })
        .collect();
    let Pick::Choose(left) = pick(&assets, wingetrs::install::manifest::Arch::X64) else {
        panic!("expected a choice");
    };
    let names: Vec<&str> = left.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, EPROCIA[..2].to_vec());
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_installer_named_pair_needs_no_asset_flag() {
    let w = World::new().await;
    w.release(
        "v0.3.0",
        vec![
            a(EPROCIA[0], inno()),
            a(EPROCIA[1], program()),
            a(EPROCIA[2], b"Write-Host x".to_vec()),
            a(EPROCIA[3], b"MIT".to_vec()),
            a(EPROCIA[4], b"sums".to_vec()),
        ],
    )
    .await;
    let ui = TestUi::new(true, None);
    w.run(
        &ui,
        &["install", "gh:o/tool", "--allow-unreviewed", "--dry-run"],
    )
    .await
    .unwrap();
    let text = ui.text();
    assert!(ui.picked.lock().unwrap().is_empty(), "{text}");
    assert!(
        text.contains(&format!(
            "Roda o instalador Inno Setup (origem não revisada): {}",
            EPROCIA[0]
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("Se falhar: instala só {}", EPROCIA[1])),
        "{text}"
    );
    // Only the installer was downloaded: the name ordered, `classify_pe` confirmed.
    let fetched: Vec<String> = w
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_string())
        .filter(|p| p.contains("/releases/download/"))
        .collect();
    assert_eq!(
        fetched,
        vec![format!("/o/tool/releases/download/v0.3.0/{}", EPROCIA[0])]
    );
    assert!(w.launches().is_empty());
}
// ---- Task 211: upgrade, list and remove release apps ----

impl World {
    /// `<web>/o/tool/releases/latest` redirects to the page of `tag` (no API request).
    async fn latest_tag(&self, tag: &str) {
        Mock::given(method("GET"))
            .and(path("/o/tool/releases/latest"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("/o/tool/releases/tag/{tag}")),
            )
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/o/tool/releases/tag/{tag}")))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
            .mount(&self.server)
            .await;
    }

    /// A newer release replaces what the server published: release `tag` (API and
    /// downloads) and the redirect to it.
    async fn publish(&self, tag: &str, assets: Vec<A>) {
        self.server.reset().await;
        self.release(tag, assets).await;
        self.latest_tag(tag).await;
    }

    /// `ops::upgradable` with the releases checked through the redirect.
    async fn upgradable(&self) -> ops::UpgradeList {
        let mut list = ops::upgradable(&self.ctx()).unwrap();
        ops::check_releases(&self.http, &self.server.uri(), &mut list).await;
        list
    }

    fn uninstalled(&self) -> Vec<UninstallTarget> {
        self.flows.uninstalled.lock().unwrap().clone()
    }

    fn manifest_url(&self, app: &str) -> String {
        let m: serde_json::Value = serde_json::from_slice(
            &std::fs::read(self.layout().current(app).join("manifest.json")).unwrap(),
        )
        .unwrap();
        m["url"].as_str().unwrap().to_string()
    }
}

fn zip_at(version: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let exe = format!("tool-{version}/tool.exe");
    let program = program();
    let mut entries: Vec<(&str, &[u8])> = vec![(exe.as_str(), &program)];
    entries.extend_from_slice(extra);
    zip_of(&entries)
}

fn upgradable_row(app: &str, from: &str, to: &str) -> (String, String, String) {
    (app.to_string(), from.to_string(), to.to_string())
}

fn wingetrs_row(app: &str, version: &str) -> ListRow {
    ListRow {
        name: app.into(),
        app_key: format!("wingetrs/{app}"),
        origin: "wingetrs".into(),
        installed_version: Some(version.into()),
        ..Default::default()
    }
}

const UPGRADE_TOOL: [&str; 4] = ["upgrade", "tool", "--yes", "--allow-unreviewed"];

#[tokio::test(flavor = "multi_thread")]
async fn upgradable_uses_redirect_tag() {
    let w = World::new().await;
    w.release("v1.2.0", vec![a(ZIP, tool_zip())]).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    w.server.reset().await;
    w.latest_tag("v1.3.0").await;
    let list = w.upgradable().await;
    assert_eq!(
        list.upgradable,
        vec![upgradable_row("tool", "1.2.0", "1.3.0")]
    );
    assert!(list.skipped.is_empty(), "{:?}", list.skipped);
    // The redirect and the tag page only: the API limit is not spent.
    assert_eq!(w.api_requests().await, 0);
    assert_eq!(w.requests().await, 2);
    // Without a check the app is never "no longer in the catalog".
    let unchecked = ops::upgradable(&w.ctx()).unwrap();
    assert!(unchecked.skipped.is_empty(), "{:?}", unchecked.skipped);
    assert_eq!(unchecked.releases.len(), 1);
    // The same tag: nothing to upgrade.
    w.server.reset().await;
    w.latest_tag("v1.2.0").await;
    assert!(w.upgradable().await.upgradable.is_empty());
    // `list`: the repository is the row's source; `--upgradable` checks the version.
    let mut rows = vec![
        wingetrs_row("tool", "1.2.0"),
        arp_row(&ArpKey {
            hive: Hive::User,
            key: "Other".into(),
            name: "Other".into(),
            version: None,
        }),
    ];
    ops::release_rows(&w.layout(), &w.http, None, &mut rows).await;
    assert_eq!(
        rows[0].correlation,
        Correlation::Matched {
            source: "github:o/tool".into(),
            package_id: "tool".into(),
            rule: Rule::ScoopManifest,
        }
    );
    assert_eq!(rows[0].state, UpdateState::Unknown);
    assert_eq!(rows[1].correlation, Correlation::Unmatched);
    w.server.reset().await;
    w.latest_tag("v1.3.0").await;
    let uri = w.server.uri();
    ops::release_rows(&w.layout(), &w.http, Some(&uri), &mut rows).await;
    assert_eq!(
        rows[0].state,
        UpdateState::Available {
            latest: "1.3.0".into()
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_keeps_asset_pattern() {
    let w = World::new().await;
    w.release(
        "v1.2.0",
        vec![
            a(ZIP, tool_zip()),
            a(
                "tool-1.2.0-windows-debug.zip",
                zip_at("1.2.0", &[("tool-1.2.0/tool.pdb", b"pdb")]),
            ),
        ],
    )
    .await;
    w.run(
        &no_tty(),
        &[&YES_UNREVIEWED[..], &["--asset", ZIP]].concat(),
    )
    .await
    .unwrap();
    w.publish(
        "v1.3.0",
        vec![
            a(
                "tool-1.3.0-windows-debug.zip",
                zip_at("1.3.0", &[("tool-1.3.0/tool.pdb", b"pdb")]),
            ),
            a("tool-1.3.0-windows.zip", zip_at("1.3.0", &[])),
        ],
    )
    .await;
    // Never implied by `--yes`.
    let e = w
        .run(&no_tty(), &["upgrade", "tool", "--yes"])
        .await
        .unwrap_err();
    assert!(e.to_string().contains("--allow-unreviewed"), "{e:#}");
    assert_eq!(w.record("tool").unwrap().version, "1.2.0");
    // The saved name with the new version: no question, even with two Windows files.
    let ui = no_tty();
    w.run(&ui, &UPGRADE_TOOL).await.unwrap();
    assert!(ui.picked.lock().unwrap().is_empty());
    let rec = w.record("tool").unwrap();
    assert_eq!(rec.version, "1.3.0");
    assert_eq!(rec.bucket, "github:o/tool");
    assert!(
        w.manifest_url("tool")
            .ends_with("/o/tool/releases/download/v1.3.0/tool-1.3.0-windows.zip"),
        "{}",
        w.manifest_url("tool")
    );
    assert!(w.layout().shims().join("tool.shim").exists());
    assert!(w.launches().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_pattern_changed_asks() {
    let w = World::new().await;
    w.release("v1.2.0", vec![a(ZIP, tool_zip())]).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    // One file for this host, but not under the saved name.
    w.publish(
        "v1.3.0",
        vec![a("tool-v1.3.0-x64.zip", zip_at("1.3.0", &[]))],
    )
    .await;
    let e = w.run(&no_tty(), &UPGRADE_TOOL).await.unwrap_err();
    let text = format!("{e:#}");
    assert!(text.contains("tool-1.3.0-windows.zip"), "{text}");
    assert!(text.contains("tool-v1.3.0-x64.zip"), "{text}");
    assert_eq!(w.record("tool").unwrap().version, "1.2.0");
    // A terminal: the user picks.
    let ui = TestUi::new(true, Some(0));
    w.run(&ui, &["upgrade", "tool", "--allow-unreviewed"])
        .await
        .unwrap();
    assert_eq!(
        ui.picked.lock().unwrap().clone(),
        vec![vec!["tool-v1.3.0-x64.zip".to_string()]]
    );
    assert_eq!(w.record("tool").unwrap().version, "1.3.0");
    assert!(w.manifest_url("tool").ends_with("/tool-v1.3.0-x64.zip"));
}

#[tokio::test(flavor = "multi_thread")]
async fn rate_limited_upgrade_reports_reset() {
    let w = World::new().await;
    w.release("v1.2.0", vec![a(ZIP, tool_zip())]).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    w.server.reset().await;
    w.latest_tag("v1.3.0").await;
    Mock::given(method("GET"))
        .and(path("/repos/o/tool/releases/latest"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", "1700000000"),
        )
        .mount(&w.server)
        .await;
    let e = w.run(&no_tty(), &UPGRADE_TOOL).await.unwrap_err();
    assert!(
        matches!(
            e.downcast_ref::<GhError>(),
            Some(GhError::RateLimited {
                reset: Some(1_700_000_000)
            })
        ),
        "{e:#}"
    );
    let at = wingetrs::github::reset_clock(1_700_000_000);
    assert!(e.to_string().contains(&format!("até {at}")), "{e:#}");
    assert_eq!(wingetrs::github::clock_at(1_700_000_000, 0), "22:13");
    assert_eq!(
        wingetrs::github::clock_at(1_700_000_000, -3 * 3600),
        "19:13"
    );
    assert_eq!(w.record("tool").unwrap().version, "1.2.0");
    // The list: the app's line says it was not checked, and why.
    w.server.reset().await;
    Mock::given(method("GET"))
        .and(path("/o/tool/releases/latest"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&w.server)
        .await;
    let list = w.upgradable().await;
    assert!(list.upgradable.is_empty());
    assert_eq!(
        list.skipped,
        vec![(
            "tool".to_string(),
            "não conferido (limite do GitHub)".to_string()
        )]
    );
    let mut rows = vec![wingetrs_row("tool", "1.2.0")];
    let uri = w.server.uri();
    ops::release_rows(&w.layout(), &w.http, Some(&uri), &mut rows).await;
    assert_eq!(rows[0].state, UpdateState::Unknown);
    let text = wingetrs::cli::output::list_text(&rows, true);
    assert!(text.contains("não conferido (limite do GitHub)"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_github_app() {
    let w = World::new().await;
    w.release("v1.2.0", vec![a(ZIP, tool_zip())]).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    assert!(w.layout().shims().join("tool.shim").exists());
    // `--forget` is only for an app its installer put in place.
    let e = w
        .run(&no_tty(), &["uninstall", "tool", "--forget", "--yes"])
        .await
        .unwrap_err();
    assert!(e.to_string().contains("--forget"), "{e:#}");
    assert!(w.record("tool").is_some());
    w.run(&no_tty(), &["uninstall", "github:o/tool", "--yes"])
        .await
        .unwrap();
    assert_eq!(w.record("tool"), None);
    assert!(!w.layout().app_dir("tool").exists());
    assert!(!w.layout().shims().join("tool.shim").exists());
    assert!(w.uninstalled().is_empty());
    assert!(w.launches().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn version_compared_semantically() {
    let w = World::new().await;
    w.release("v1.9.0", vec![a(ZIP, tool_zip())]).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    for (tag, newer) in [
        ("v1.10.0", Some("1.10.0")),
        ("v1.8.10", None),
        ("v1.9", None),
        ("1.9.0.0", None),
    ] {
        w.server.reset().await;
        w.latest_tag(tag).await;
        let want: Vec<_> = newer
            .map(|v| upgradable_row("tool", "1.9.0", v))
            .into_iter()
            .collect();
        assert_eq!(w.upgradable().await.upgradable, want, "{tag}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_installer_app_runs_arp_uninstaller() {
    let w = World::with(vec![Answer::Code(0)], true).await;
    installer_release(&w, inno()).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    // The TUI does not remove it: its uninstaller runs from the terminal.
    let e = wingetrs::tui::jobs::plan_remove(&w.ctx(), "tool", false)
        .err()
        .unwrap();
    assert!(e.contains("wingetrs uninstall tool"), "{e}");
    // A dry run removes nothing.
    w.run(&no_tty(), &["uninstall", "tool", "--dry-run"])
        .await
        .unwrap();
    assert!(w.uninstalled().is_empty());
    assert!(w.record("tool").is_some());
    w.run(&no_tty(), &["uninstall", "tool", "--yes"])
        .await
        .unwrap();
    assert_eq!(
        w.uninstalled(),
        vec![UninstallTarget::Arp {
            hive: Hive::User,
            key: "Tool_is1".into(),
        }]
    );
    assert_eq!(w.record("tool"), None);
    assert!(!w.layout().app_dir("tool").exists());
    // Only the install launched an installer.
    assert_eq!(w.launches().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_installer_app_key_gone_cleans_record() {
    let w = World::with(vec![Answer::Code(0)], true).await;
    installer_release(&w, inno()).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    // Removed outside wingetrs: its key is gone.
    w.flows
        .arp
        .keys
        .lock()
        .unwrap()
        .retain(|k| k.key != "Tool_is1");
    let ui = no_tty();
    w.run(&ui, &["uninstall", "tool", "--yes"]).await.unwrap();
    assert!(w.uninstalled().is_empty());
    assert_eq!(w.record("tool"), None);
    assert!(!w.layout().app_dir("tool").exists());
    assert!(ui.text().contains("já foi removido"), "{}", ui.text());
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_installer_without_arp_refuses_and_forget_works() {
    let w = World::with(vec![Answer::Code(0)], false).await;
    installer_release(&w, inno()).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    assert_eq!(w.install_json("tool")["arp"], serde_json::Value::Null);
    let e = w
        .run(&no_tty(), &["uninstall", "tool", "--yes"])
        .await
        .unwrap_err();
    let text = e.to_string();
    assert!(
        text.contains(
            "o wingetrs não sabe desinstalar tool: o instalador não se registrou em Programas instalados"
        ),
        "{text}"
    );
    assert!(text.contains("wingetrs uninstall tool --forget"), "{text}");
    assert!(w.record("tool").is_some());
    assert!(w.layout().app_dir("tool").exists());
    w.run(&no_tty(), &["uninstall", "tool", "--forget", "--yes"])
        .await
        .unwrap();
    assert_eq!(w.record("tool"), None);
    assert!(!w.layout().app_dir("tool").exists());
    assert!(w.uninstalled().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_installer_failure_keeps_record() {
    let w = World::with(vec![Answer::Code(0)], true).await;
    installer_release(&w, inno()).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    w.flows
        .uninstall_answers
        .lock()
        .unwrap()
        .push_back(Err("o desinstalador saiu com o código 5".into()));
    let e = w
        .run(&no_tty(), &["uninstall", "tool", "--yes"])
        .await
        .unwrap_err();
    assert!(e.to_string().contains("código 5"), "{e:#}");
    assert_eq!(w.uninstalled().len(), 1);
    assert!(w.record("tool").is_some());
    assert!(w.layout().app_dir("tool").exists());
    assert_eq!(w.install_json("tool")["arp"]["key"], "Tool_is1");
}

const SETUP_NEW: &str = "tool-1.1.0-windows-amd64-instalador.exe";
const PLAIN_NEW: &str = "tool-1.1.0-windows-amd64.exe";

/// `tool` 1.0.0 put in place by its installer, and release 1.1.0 (same shape) published.
async fn installer_app_and_newer(answers: Vec<Answer>) -> World {
    let w = World::with(answers, true).await;
    installer_release(&w, inno()).await;
    w.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    *w.flows.launcher.key_name.lock().unwrap() = "Tool 1.1.0".into();
    *w.flows.launcher.key_version.lock().unwrap() = "1.1.0".into();
    w.publish(
        "v1.1.0",
        vec![
            a(SETUP_NEW, inno()),
            a(PLAIN_NEW, program()),
            a("SHA256SUMS.txt", b"x".to_vec()),
        ],
    )
    .await;
    w
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_installer_app_runs_new_installer_and_rerecords_arp() {
    let w = installer_app_and_newer(vec![Answer::Code(0), Answer::Code(0)]).await;
    assert_eq!(
        w.upgradable().await.upgradable,
        vec![upgradable_row("tool", "1.0.0", "1.1.0")]
    );
    let ui = no_tty();
    w.run(&ui, &UPGRADE_TOOL).await.unwrap();
    let launches = w.launches();
    assert_eq!(launches.len(), 2);
    assert_eq!(
        launches[1].program.file_name().unwrap().to_str().unwrap(),
        SETUP_NEW
    );
    assert_eq!(launches[1].params, INNO_ARGS);
    let rec = w.record("tool").unwrap();
    assert_eq!(rec.version, "1.1.0");
    assert_eq!(rec.bucket, "github:o/tool");
    let j = w.install_json("tool");
    assert_eq!(j["mode"], "installer");
    assert_eq!(j["version"], "1.1.0");
    assert_eq!(j["arp"]["key"], "Tool_is1");
    assert_eq!(j["uninstall_string"], UNINSTALL);
    assert!(!w.layout().version_dir("tool", "1.0.0").exists());
    assert!(!ui.text().contains("continua no Windows"), "{}", ui.text());
    // The new record removes like the first one.
    w.run(&no_tty(), &["uninstall", "tool", "--yes"])
        .await
        .unwrap();
    assert_eq!(w.uninstalled().len(), 1);
    assert_eq!(w.record("tool"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_installer_failure_falls_back_and_says_old_program_stays() {
    let w = installer_app_and_newer(vec![Answer::Code(0), Answer::Code(1)]).await;
    let ui = no_tty();
    w.run(&ui, &UPGRADE_TOOL).await.unwrap();
    assert_eq!(w.launches().len(), 2);
    let rec = w.record("tool").unwrap();
    assert_eq!(rec.version, "1.1.0");
    // A portable app now.
    assert_eq!(w.install_json("tool").get("mode"), None);
    assert!(
        w.layout()
            .shims()
            .join("tool-1.1.0-windows-amd64.shim")
            .exists()
    );
    let text = ui.text();
    assert!(
        text.contains("o instalador falhou (saiu com o código 1"),
        "{text}"
    );
    assert!(
        text.contains(
            "a versão anterior, instalada pelo instalador, continua no Windows: Tool 1.0.0 em Programas instalados"
        ),
        "{text}"
    );
    // Its removal is the portable one: no uninstaller of the old key.
    w.run(&no_tty(), &["uninstall", "tool", "--yes"])
        .await
        .unwrap();
    assert!(w.uninstalled().is_empty());
}
