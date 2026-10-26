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
use wingetrs::github::GhRepo;
use wingetrs::github::install::{ArpKey, ArpView, GhEnv, GhTarget, INNO_ARGS, parse_target};
use wingetrs::http::Http;
use wingetrs::install::executor::Runners;
use wingetrs::install::ops::{self, OpsCtx, ResidueScan, Resolution};
use wingetrs::install::progress::{Phase, ProgressEvent};
use wingetrs::install::state::{AppRecord, StateDb};
use wingetrs::install::target::Candidate;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::installed::Hive;
use wingetrs::installed::correlate::ListRow;
use wingetrs::tui::app::{Done, InstallFail};
use wingetrs::tui::picker::PickerItem;
use wingetrs::update::LockWait;
use wingetrs::winget_install::engine::UpgradeError;
use wingetrs::winget_install::launch::{LaunchSpec, Launcher, Verb};
use wingetrs::winget_install::ops::{InstallReport, WingetOpsError};
use wingetrs::winget_install::orphans::Orphan;
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan};
use wingetrs::winget_install::select::{Refusal, ScopeWanted};
use wingetrs::winget_install::uninstall::{UninstallPlan, UninstallTarget};
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
                    self.keys.lock().unwrap().push(ArpKey {
                        hive: Hive::User,
                        key: "Tool_is1".into(),
                        name: "Tool 1.0.0".into(),
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
    fn gate(&self) -> wingetrs::install::executor::LaunchGate {
        unreachable!("winget")
    }
    fn progress(&self) -> wingetrs::cli::winget_cmd::Sink {
        unreachable!("winget")
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
    fn installed(&self) -> anyhow::Result<Vec<ListRow>> {
        unreachable!("winget")
    }
    async fn plan_uninstall(
        &self,
        _: &ListRow,
        _: &UninstallTarget,
    ) -> Result<UninstallPlan, String> {
        unreachable!("winget")
    }
    fn lock_install(
        &self,
    ) -> wingetrs::cli::winget_cmd::Op<Result<wingetrs::cli::winget_cmd::Held, WingetOpsError>>
    {
        unreachable!("winget")
    }
    fn uninstall(&self, _: &UninstallPlan) -> wingetrs::cli::winget_cmd::Uninstaller {
        unreachable!("winget")
    }
    fn remove_registry_only(
        &self,
        _: &UninstallPlan,
        _: &str,
    ) -> wingetrs::cli::winget_cmd::RecordRemover {
        unreachable!("winget")
    }
    fn free_space(&self) -> fn(&Path) -> Option<u64> {
        unreachable!("winget")
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
        unreachable!("winget")
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
    /// run adds its ARP key.
    async fn with(answers: Vec<Answer>, writes_key: bool) -> World {
        let t = tempfile::tempdir().unwrap();
        let data = DataDir::new(t.path().join("root"));
        let layout = data.install_layout();
        std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
        std::fs::write(layout.launcher(), program()).unwrap();
        let server = MockServer::start().await;
        let keys = Arc::new(Mutex::new(vec![ArpKey {
            hive: Hive::MachineX64,
            key: "Other".into(),
            name: "Other app".into(),
        }]));
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
                    keys: keys.clone(),
                    seen: Mutex::new(Vec::new()),
                },
                arp: FakeArp { keys },
                elevated: not_elevated,
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
            let digest = x
                .digest
                .clone()
                .unwrap_or_else(|| format!("sha256:{}", sha_hex(&x.bytes)));
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
        Mock::given(method("GET"))
            .and(path("/repos/o/tool/releases/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&self.server)
            .await;
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

#[tokio::test(flavor = "multi_thread")]
async fn gh_installer_failure_falls_back_to_exe() {
    let cases = [
        (Answer::Code(1), true, "saiu com o código 1"),
        (
            Answer::Os(1223),
            true,
            "autorização de administrador recusada",
        ),
        (
            Answer::Code(0),
            false,
            "nenhum registro novo em Programas instalados",
        ),
    ];
    for (answer, writes_key, reason) in cases {
        let w = World::with(vec![answer], writes_key).await;
        installer_release(&w, inno()).await;
        let ui = no_tty();
        w.run(&ui, &YES_UNREVIEWED).await.unwrap();
        assert_eq!(w.launches().len(), 1, "{reason}");
        let text = ui.text();
        assert!(
            text.contains(&format!(
                "o instalador falhou ({reason}); instalando só o executável"
            )),
            "{text}"
        );
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
    }
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
        phases
            .iter()
            .any(|p| matches!(p, Phase::SelfElevating { name } if name == "tool")),
        "{phases:?}"
    );
    // A plain installer: the running phase, never the UAC one.
    let w2 = World::with(vec![Answer::Code(1)], true).await;
    installer_release(&w2, inno()).await;
    w2.run(&no_tty(), &YES_UNREVIEWED).await.unwrap();
    let phases = w2.phases.lock().unwrap().clone();
    assert!(
        phases
            .iter()
            .any(|p| matches!(p, Phase::RunningInstaller { name } if name == "tool")),
        "{phases:?}"
    );
    assert!(
        !phases
            .iter()
            .any(|p| matches!(p, Phase::SelfElevating { .. })),
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
