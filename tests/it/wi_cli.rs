#![allow(clippy::unwrap_used, clippy::expect_used)]
//! CLI for winget packages (Task 186).
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::CommandFactory;
use crossterm::event::Event;
use crossterm::event::KeyCode::{Char, Down, Enter};
use tempfile::TempDir;
use wingetrs::bridge::BridgeJob;
use wingetrs::bridge::elevated::{ElevatedError, ElevatedRunner};
use wingetrs::bridge::worker::ScopeArg;
use wingetrs::bridge::worker::{DeleteReason, MachineHive};
use wingetrs::cache::DataDir;
use wingetrs::catalog::Catalog;
use wingetrs::cli::Cli;
use wingetrs::cli::install_cmd::{self, Flags, Interrupts, Stop, Ui};
use wingetrs::cli::output::Phases;
use wingetrs::cli::picker::fake::{Screen, ScriptedTerm, ctrl_c, key, plain};
use wingetrs::cli::winget_cmd::{
    self, CleanupArgs, Held, Op, RecordRemover, Sink, Uninstaller, WingetFlows,
};
use wingetrs::config::Behavior;
use wingetrs::core::{PackageEntry, SourceKind};
use wingetrs::http::Http;
use wingetrs::install::executor::LaunchGate;
use wingetrs::install::ops::{OpsCtx, ResidueScan};
use wingetrs::install::progress::{Phase, ProgressEvent, phase_title};
use wingetrs::install::target::Candidate;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::installed::Hive;
use wingetrs::installed::correlate::{Correlation, ListRow, Rule};
use wingetrs::sources::winget::manifest;
use wingetrs::tui::app::{Done, InstallFail};
use wingetrs::tui::picker::PickerItem;
use wingetrs::tui::theme::{GlyphSet, Glyphs};
use wingetrs::update::LockWait;
use wingetrs::winget_install::codes;
use wingetrs::winget_install::elevated_rules::KnownFolders;
use wingetrs::winget_install::engine::UpgradeError;
use wingetrs::winget_install::launch::Elevation;
use wingetrs::winget_install::launch::{LaunchSpec, Launcher};
use wingetrs::winget_install::msix::{AddResult, MsixError, MsixOps};
use wingetrs::winget_install::ops::{InstallReport, WingetOpsError};
use wingetrs::winget_install::orphans::Orphan;
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan, plan};
use wingetrs::winget_install::select::{Host, Refusal, Scope, ScopeWanted};
use wingetrs::winget_install::uninstall::{
    Clock, MsiQueue, SharedData, UninstallCtx, UninstallPlan, UninstallReport, UninstallStatus,
    UninstallTarget, UserFolders,
};
use wingetrs::winget_install::uninstall_cmd::{
    ArpDetails, Rule as UninstallRule, UninstallCommand,
};

// ---- fakes ----

#[derive(Default)]
struct FakeUi {
    interactive: bool,
    confirm: bool,
    pick: Option<usize>,
    picks: Cell<usize>,
    picked_items: RefCell<Vec<Vec<String>>>,
    initials: RefCell<Vec<Vec<bool>>>,
    notices: RefCell<Vec<String>>,
}

impl Ui for FakeUi {
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn confirm(&self, _: &str) -> anyhow::Result<bool> {
        Ok(self.confirm)
    }
    fn pick(&self, items: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        self.picks.set(self.picks.get() + 1);
        self.picked_items
            .borrow_mut()
            .push(items.into_iter().map(|i| i.label).collect());
        Ok(self.pick)
    }
    fn pick_many(
        &self,
        _: Vec<PickerItem>,
        initial: &[bool],
    ) -> anyhow::Result<Option<Vec<usize>>> {
        self.initials.borrow_mut().push(initial.to_vec());
        Ok(None)
    }
    fn notice(&self, text: &str) {
        self.notices.borrow_mut().push(text.to_string());
    }
}

/// What a fake native install does on its thread (given the launch gate).
type InstallHook = Arc<dyn Fn(&LaunchGate) -> Result<InstallReport, WingetOpsError> + Send + Sync>;

/// Shared, in order: `lock`, `phase:<title>`, `run`, `registry-only`.
type Steps = Arc<Mutex<Vec<String>>>;

/// What a fake `install.lock` wait does on its thread (given the launch gate).
type LockHook = Arc<dyn Fn(&LaunchGate) -> Result<(), WingetOpsError> + Send + Sync>;

/// What a fake uninstaller does on its thread before it ends.
type RunHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Fake {
    planned: Option<PlanOrBridge>,
    rows: Vec<ListRow>,
    orphans: Vec<Orphan>,
    installs: Arc<AtomicUsize>,
    install_hook: Option<InstallHook>,
    bridged: Cell<usize>,
    planned_targets: RefCell<Vec<UninstallTarget>>,
    uninstalled: Arc<Mutex<Vec<UninstallTarget>>>,
    removed: RefCell<Vec<String>>,
    backups: PathBuf,
    pins: BTreeSet<String>,
    gate: LaunchGate,
    /// Every phase the commands announced.
    events: Arc<Mutex<Vec<ProgressEvent>>>,
    steps: Steps,
    /// The plans' shared data (Kindle) and elevation.
    shared: Option<SharedData>,
    elevation: Option<Elevation>,
    lock_hook: Option<LockHook>,
    run_hook: Option<RunHook>,
}

fn report_ok() -> InstallReport {
    InstallReport {
        outcome: codes::Outcome::Ok,
        found: None,
        warning: None,
        log: None,
    }
}

fn plenty(_: &Path) -> Option<u64> {
    Some(u64::MAX)
}

impl WingetFlows for Fake {
    fn behavior(&self) -> Behavior {
        Behavior::default()
    }
    fn gate(&self) -> LaunchGate {
        self.gate.clone()
    }
    fn progress(&self) -> Sink {
        let (events, steps) = (Arc::clone(&self.events), Arc::clone(&self.steps));
        Arc::new(move |ev: &ProgressEvent| {
            if let ProgressEvent::Phase(p) = ev {
                steps
                    .lock()
                    .unwrap()
                    .push(format!("phase:{}", phase_title(p)));
            }
            events.lock().unwrap().push(ev.clone());
        })
    }
    async fn plan_install(
        &self,
        _: &Candidate,
        _: ScopeWanted,
    ) -> anyhow::Result<Result<PlanOrBridge, Refusal>> {
        Ok(Ok(self.planned.clone().expect("no plan in this test")))
    }
    fn install(&self, _: &WingetPlan) -> Op<Result<InstallReport, WingetOpsError>> {
        let (n, hook, gate) = (
            Arc::clone(&self.installs),
            self.install_hook.clone(),
            self.gate.clone(),
        );
        Box::new(move || {
            Box::pin(async move {
                n.fetch_add(1, Ordering::SeqCst);
                match hook {
                    Some(h) => h(&gate),
                    None => Ok(report_ok()),
                }
            })
        })
    }
    async fn install_bridge<I: Interrupts>(
        &self,
        _: &dyn Ui,
        _: &Candidate,
        _: Option<ScopeArg>,
        _: Flags,
        _: Stop<'_, I>,
    ) -> anyhow::Result<()> {
        self.bridged.set(self.bridged.get() + 1);
        Ok(())
    }
    fn installed(&self) -> anyhow::Result<Vec<ListRow>> {
        Ok(self.rows.clone())
    }
    async fn plan_uninstall(
        &self,
        row: &ListRow,
        target: &UninstallTarget,
    ) -> Result<UninstallPlan, String> {
        self.planned_targets.borrow_mut().push(target.clone());
        Ok(UninstallPlan {
            target: target.clone(),
            name: row.name.clone(),
            version: row.installed_version.clone(),
            command: None,
            elevation: self.elevation.unwrap_or(Elevation::Never),
            shared: self.shared.clone(),
        })
    }
    fn lock_install(&self) -> Op<Result<Held, WingetOpsError>> {
        let (steps, hook, gate) = (
            Arc::clone(&self.steps),
            self.lock_hook.clone(),
            self.gate.clone(),
        );
        Box::new(move || {
            Box::pin(async move {
                if let Some(h) = hook {
                    h(&gate)?;
                }
                steps.lock().unwrap().push("lock".into());
                Ok(Box::new(()) as Held)
            })
        })
    }
    fn uninstall(&self, plan: &UninstallPlan) -> Uninstaller {
        let (done, steps) = (Arc::clone(&self.uninstalled), Arc::clone(&self.steps));
        let (target, backups, hook) = (
            plan.target.clone(),
            self.backups.clone(),
            self.run_hook.clone(),
        );
        Box::new(move |_: &dyn Fn(&ProgressEvent)| {
            // Whether the folder copy already exists when the uninstaller runs.
            let copied = std::fs::read_dir(&backups).is_ok_and(|mut d| d.next().is_some());
            steps.lock().unwrap().push(format!("run copied={copied}"));
            if let Some(h) = hook {
                h();
            }
            done.lock().unwrap().push(target);
            Ok(UninstallReport {
                status: UninstallStatus::Removed,
                warning: None,
                reboot: None,
            })
        })
    }
    fn remove_registry_only(&self, plan: &UninstallPlan, ts: &str) -> RecordRemover {
        let steps = Arc::clone(&self.steps);
        let file = self.backups.join(ts).join(format!("{}.reg", plan.name));
        Box::new(move || {
            steps.lock().unwrap().push("registry-only".into());
            Ok(file)
        })
    }
    fn free_space(&self) -> fn(&Path) -> Option<u64> {
        plenty
    }
    fn pins(&self) -> BTreeSet<String> {
        self.pins.clone()
    }
    async fn plan_upgrade(
        &self,
        _: &ListRow,
        _: &str,
        _: ScopeWanted,
    ) -> anyhow::Result<(Result<PlanOrBridge, Refusal>, Option<UninstallPlan>)> {
        Ok((Ok(self.planned.clone().expect("no plan")), None))
    }
    fn upgrade(
        &self,
        _: &WingetPlan,
        _: Option<&UninstallPlan>,
    ) -> Op<Result<InstallReport, UpgradeError>> {
        unreachable!("no upgrade in these tests")
    }
    fn upgrade_bridge(&self, _: &BridgeJob, _: &mut dyn FnMut(&str)) -> Result<Done, InstallFail> {
        unreachable!("no upgrade in these tests")
    }
    fn orphans(&self) -> anyhow::Result<Vec<Orphan>> {
        Ok(self.orphans.clone())
    }
    fn backups(&self) -> PathBuf {
        self.backups.clone()
    }
    fn remove_orphan(&self, o: &Orphan, ts: &str) -> Result<PathBuf, String> {
        self.removed.borrow_mut().push(o.key.clone());
        Ok(self.backups.join(ts).join(format!("{}.reg", o.key)))
    }
}
// ---- helpers ----

fn orphan(key: &str) -> Orphan {
    Orphan {
        hive: Hive::User,
        key: key.into(),
        name: format!("App {key}"),
        missing: format!(r"C:\Gone\{key}\uninst.exe"),
    }
}

fn row(name: &str, app_key: &str, winget_id: Option<&str>) -> ListRow {
    ListRow {
        name: name.into(),
        app_key: app_key.into(),
        origin: "arp".into(),
        installed_version: Some("1.0".into()),
        correlation: match winget_id {
            Some(id) => Correlation::Matched {
                source: "winget".into(),
                package_id: id.into(),
                rule: Rule::ProductCode,
            },
            None => Correlation::Unmatched,
        },
        location: app_key.into(),
        ..Default::default()
    }
}

fn candidate() -> Candidate {
    Candidate {
        entry: PackageEntry {
            source: SourceKind::Winget,
            source_name: "winget".into(),
            id: "Pub.App".into(),
            name: "Contoso App".into(),
            moniker: None,
            version: "1.0".into(),
            publisher: None,
            description: None,
            tags: Vec::new(),
            commands: Vec::new(),
        },
        selectable: true,
        note: None,
    }
}

fn native_plan() -> WingetPlan {
    let yaml = format!(
        "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Contoso App\nPublisher: Contoso\nAgreements:\n- AgreementLabel: EULA\n  AgreementUrl: https://x/eula\nInstallers:\n- Architecture: x64\n  InstallerType: inno\n  Scope: user\n  InstallerUrl: https://h/setup.exe\n  InstallerSha256: {}\n",
        "a".repeat(64)
    );
    let m = manifest::parse(yaml.as_bytes()).unwrap();
    let host = Host {
        arch: "x64".into(),
        os: [10, 0, 22631, 0],
        locale: "pt-BR".into(),
    };
    match plan(&m, &host, ScopeWanted::Prefer(Scope::User), None).unwrap() {
        PlanOrBridge::Native(p) => p,
        PlanOrBridge::Bridge(why) => panic!("bridge: {why}"),
    }
}

/// Interrupts that never fire (the sender stays alive).
fn quiet() -> (
    tokio::sync::mpsc::UnboundedSender<()>,
    tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    tokio::sync::mpsc::unbounded_channel()
}

fn yes() -> Flags {
    Flags {
        yes: true,
        ..Default::default()
    }
}

fn cleanup(fake: &Fake, ui: &dyn Ui, a: CleanupArgs) -> (anyhow::Result<()>, Vec<String>) {
    let mut out = Vec::new();
    let r = winget_cmd::cleanup(fake, ui, a, &mut |l: &str| out.push(l.to_string()));
    (r, out)
}

// ---- tests ----

#[test]
fn help_lists_cleanup_and_orphans() {
    let mut root = Cli::command();
    root.build();
    let help = root.render_long_help().to_string();
    assert!(help.contains("cleanup"), "{help}");
    assert!(
        !help.contains("--orphans"),
        "the root help lists no subcommand flag"
    );
    let sub = root
        .find_subcommand_mut("cleanup")
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(sub.contains("--orphans"), "{sub}");
}

#[test]
fn cleanup_requires_orphans_flag() {
    let fake = Fake {
        orphans: vec![orphan("A"), orphan("B")],
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (r, _) = cleanup(
        &fake,
        &ui,
        CleanupArgs {
            yes: true,
            ..Default::default()
        },
    );
    let e = r.unwrap_err().to_string();
    assert!(e.contains("--orphans"), "{e}");
    assert!(fake.removed.borrow().is_empty());
}

#[test]
fn orphans_never_preselected() {
    let fake = Fake {
        orphans: vec![orphan("A"), orphan("B"), orphan("C")],
        ..Default::default()
    };
    let ui = FakeUi {
        interactive: true,
        ..Default::default()
    };
    let (r, out) = cleanup(
        &fake,
        &ui,
        CleanupArgs {
            orphans: true,
            ..Default::default()
        },
    );
    r.unwrap();
    assert_eq!(*ui.initials.borrow(), vec![vec![false, false, false]]);
    assert!(fake.removed.borrow().is_empty());
    let text = out.join("\n");
    assert!(
        text.contains(r"Órfão: App A — HKCU — falta C:\Gone\A\uninst.exe"),
        "{text}"
    );
    assert!(text.contains("O app pode ter mudado de lugar (ex.: Steam em outra pasta)."));
}

#[test]
fn yes_with_orphans_removes_each() {
    let t = TempDir::new().unwrap();
    let fake = Fake {
        orphans: vec![orphan("A"), orphan("B")],
        backups: t.path().join("backups"),
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (r, out) = cleanup(
        &fake,
        &ui,
        CleanupArgs {
            orphans: true,
            yes: true,
            dry_run: false,
        },
    );
    r.unwrap();
    assert_eq!(
        *fake.removed.borrow(),
        vec!["A".to_string(), "B".to_string()]
    );
    let text = out.join("\n");
    assert!(text.contains("Removido: App A (backup "), "{text}");
    assert!(ui.initials.borrow().is_empty(), "--yes never asks");

    let dry = Fake {
        orphans: vec![orphan("A"), orphan("B")],
        backups: t.path().join("backups"),
        ..Default::default()
    };
    let (r, out) = cleanup(
        &dry,
        &ui,
        CleanupArgs {
            orphans: true,
            yes: true,
            dry_run: true,
        },
    );
    r.unwrap();
    assert!(dry.removed.borrow().is_empty());
    assert!(out.iter().any(|l| l == "Simulação: nada foi alterado."));
}

#[tokio::test]
async fn native_install_prints_plan_and_logs_agreements() {
    let log = crate::spans::Log::default();
    let _installed = log.install();
    let p = native_plan();
    let fake = Fake {
        planned: Some(PlanOrBridge::Native(p.clone())),
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (_tx, mut rx) = quiet();
    let gate = LaunchGate::default();
    let mut out = Vec::new();
    winget_cmd::install(
        &fake,
        &ui,
        &candidate(),
        None,
        yes(),
        Stop::new(&mut rx, &gate),
        &mut |l: &str| out.push(l.to_string()),
    )
    .await
    .unwrap();
    let text = out.join("\n");
    assert!(text.contains(p.to_string().trim_end()), "{text}");
    assert!(
        text.contains("Licença aceita: EULA — https://x/eula"),
        "{text}"
    );
    assert!(text.contains("Instalado: Contoso App 1.0"), "{text}");
    assert_eq!(fake.installs.load(Ordering::SeqCst), 1);
    let logged = log.text();
    let start = logged.lines().find(|l| l.contains("job.start")).unwrap();
    assert!(start.contains("agreements=EULA https://x/eula"), "{logged}");
}

#[tokio::test]
async fn bridge_reason_is_printed() {
    let fake = Fake {
        planned: Some(PlanOrBridge::Bridge(
            "tipo zip: instalado pelo winget".into(),
        )),
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (_tx, mut rx) = quiet();
    let gate = LaunchGate::default();
    let mut out = Vec::new();
    winget_cmd::install(
        &fake,
        &ui,
        &candidate(),
        None,
        yes(),
        Stop::new(&mut rx, &gate),
        &mut |l: &str| out.push(l.to_string()),
    )
    .await
    .unwrap();
    let text = out.join("\n");
    assert!(text.contains("Instalando pelo winget: tipo zip"), "{text}");
    assert_eq!(fake.bridged.get(), 1);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
}

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

#[tokio::test]
async fn uninstall_falls_back_to_installed_rows() {
    let t = TempDir::new().unwrap();
    let data = DataDir::new(t.path().join("root"));
    let http = Http::new().unwrap();
    let catalog = Catalog::open_in_memory().unwrap();
    let (env, shortcuts, trash, registry) = (
        FakeEnv::new(),
        FakeShortcuts::new(t.path().join("sm")),
        FakeTrash::new(),
        FakeRegistry::new(),
    );
    let ctx = OpsCtx {
        data: &data,
        http: &http,
        catalog: &catalog,
        effects: Effects {
            env: &env,
            shortcuts: &shortcuts,
            trash: &trash,
            registry: &registry,
        },
        wait: LockWait::NoWait,
        progress: &no_progress,
        in_use: &nothing_in_use,
        residue: ResidueScan::none(),
        msiexec: wingetrs::install::executor::Msiexec::DISABLED,
        launch: Default::default(),
        runners: wingetrs::install::executor::Runners::disabled(),
    };
    let fake = Fake {
        rows: vec![
            row("Other", "arp/hkcu/Other", None),
            row("Contoso App", "arp/hklm32/{G}", Some("Pub.App")),
        ],
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (_tx, mut rx) = quiet();
    // Not an app of wingetrs (no state at all): the Windows list answers.
    let cmd = wingetrs::cli::Command::Uninstall {
        target: "pub.app".into(),
        purge: false,
        dry_run: false,
        yes: true,
        allow_scripts: false,
    };
    install_cmd::execute(&ctx, &ui, cmd, &mut rx, &fake)
        .await
        .unwrap();
    let want = UninstallTarget::Arp {
        hive: Hive::MachineX86,
        key: "{G}".into(),
    };
    assert_eq!(*fake.planned_targets.borrow(), vec![want.clone()]);
    assert_eq!(*fake.uninstalled.lock().unwrap(), vec![want]);
}

#[tokio::test]
async fn purge_refused_for_winget() {
    let fake = Fake {
        rows: vec![row("Contoso App", "arp/hklm32/{G}", Some("Pub.App"))],
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (_tx, mut rx) = quiet();
    let gate = LaunchGate::default();
    let e = winget_cmd::uninstall(
        &fake,
        &ui,
        "Pub.App",
        true,
        yes(),
        Stop::new(&mut rx, &gate),
        &mut |_: &str| {},
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(e.contains("--purge só vale"), "{e}");
    assert!(fake.uninstalled.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ambiguous_name_needs_terminal() {
    let rows = vec![
        row("Amazon Kindle", "arp/hkcu/Kindle", None),
        row("Amazon Kindle", "arp/hklm32/Kindle", None),
    ];
    let fake = Fake {
        rows: rows.clone(),
        ..Default::default()
    };
    let (_tx, mut rx) = quiet();
    let gate = LaunchGate::default();
    // `--yes` never picks, even with a terminal that could.
    let ui = FakeUi {
        interactive: true,
        pick: Some(0),
        ..Default::default()
    };
    let e = winget_cmd::uninstall(
        &fake,
        &ui,
        "amazon kindle",
        false,
        yes(),
        Stop::new(&mut rx, &gate),
        &mut |_: &str| {},
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(e.contains("mais de um app instalado"), "{e}");
    assert_eq!(ui.picks.get(), 0);
    assert!(fake.planned_targets.borrow().is_empty());
    // No terminal (and no --yes): the same error.
    let ui = FakeUi {
        pick: Some(0),
        ..Default::default()
    };
    let e = winget_cmd::uninstall(
        &fake,
        &ui,
        "amazon kindle",
        false,
        Flags::default(),
        Stop::new(&mut rx, &gate),
        &mut |_: &str| {},
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(e.contains("mais de um app instalado"), "{e}");
    assert_eq!(ui.picks.get(), 0);
    assert!(fake.planned_targets.borrow().is_empty());

    // With a terminal (and without --yes) the user picks; the pick is what gets planned.
    let ui = FakeUi {
        interactive: true,
        confirm: false,
        pick: Some(1),
        ..Default::default()
    };
    let fake = Fake {
        rows,
        ..Default::default()
    };
    winget_cmd::uninstall(
        &fake,
        &ui,
        "Amazon Kindle",
        false,
        Flags::default(),
        Stop::new(&mut rx, &gate),
        &mut |_: &str| {},
    )
    .await
    .unwrap();
    assert_eq!(ui.picks.get(), 1);
    assert_eq!(
        *fake.planned_targets.borrow(),
        vec![UninstallTarget::Arp {
            hive: Hive::MachineX86,
            key: "Kindle".into()
        }]
    );
    assert!(
        fake.uninstalled.lock().unwrap().is_empty(),
        "declined at the confirmation"
    );
}

/// `UserFolders` comes from the known folders: an `APPDATA` planted in the environment of a
/// child process (this test, run again) changes nothing.
#[cfg(windows)]
#[test]
fn user_folders_ignore_the_environment() {
    use wingetrs::winget_install::folders_win::user_folders;
    const CHILD: &str = "WINGETRS_T186_CHILD";
    let root = Path::new(r"C:\wingetrs-root");
    let mine = user_folders(root).unwrap();
    if std::env::var_os(CHILD).is_some() {
        println!("T186 {}", mine.app_data.display());
        println!("T186 {}", mine.local_app_data.display());
        return;
    }
    assert_eq!(mine.wingetrs_root, root);
    let planted = r"C:\planted-by-test\Roaming";
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "wi_cli::user_folders_ignore_the_environment",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env("APPDATA", planted)
        .env("LOCALAPPDATA", r"C:\planted-by-test\Local")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(!text.contains("planted-by-test"), "{text}");
    assert!(
        text.contains(&format!("T186 {}", mine.app_data.display())),
        "{text}"
    );
    assert!(
        text.contains(&format!("T186 {}", mine.local_app_data.display())),
        "{text}"
    );
}

#[cfg(windows)]
#[test]
fn free_space_reads_the_nearest_existing_folder() {
    use wingetrs::winget_install::folders_win::free_space;
    let t = TempDir::new().unwrap();
    let here = free_space(t.path()).unwrap();
    assert!(here > 0);
    // The backup destination does not exist yet: its volume answers.
    let later = free_space(&t.path().join("not").join("yet")).unwrap();
    assert!(later > 0);
    assert!(!t.path().join("not").exists(), "nothing created");
}

fn upgrade_error(fake: &Fake, target: &str) -> String {
    let ui = FakeUi::default();
    let (_tx, mut rx) = quiet();
    let gate = LaunchGate::default();
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(winget_cmd::upgrade(
        fake,
        &ui,
        target,
        yes(),
        Stop::new(&mut rx, &gate),
        &mut |_: &str| {},
    ))
    .unwrap_err()
    .to_string()
}

#[test]
fn upgrade_needs_a_winget_match() {
    let fake = Fake {
        rows: vec![row("Contoso App", "arp/hkcu/Contoso", None)],
        planned: Some(PlanOrBridge::Native(native_plan())),
        ..Default::default()
    };
    let e = upgrade_error(&fake, "Contoso App");
    assert!(e.contains("não foi associado a um pacote do winget"), "{e}");
}

#[test]
fn upgrade_refuses_a_pinned_app() {
    let fake = Fake {
        rows: vec![row("Contoso App", "arp/hkcu/Contoso", Some("Pub.App"))],
        planned: Some(PlanOrBridge::Native(native_plan())),
        pins: BTreeSet::from([wingetrs::pins::key("arp", "Contoso App")]),
        ..Default::default()
    };
    let e = upgrade_error(&fake, "Pub.App");
    assert!(e.contains("versão fixada"), "{e}");
}
// ---- Task 193 ----

/// Runs `winget_cmd::install` of `native_plan()` with `fake`, under a deadline.
async fn install_with(
    fake: &Fake,
    ui: &FakeUi,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<()>,
    f: Flags,
) -> (anyhow::Result<()>, Vec<String>) {
    let gate = fake.gate.clone();
    let mut out = Vec::new();
    let r = tokio::time::timeout(
        Duration::from_secs(30),
        winget_cmd::install(
            fake,
            ui,
            &candidate(),
            None,
            f,
            Stop::new(rx, &gate),
            &mut |l: &str| out.push(l.to_string()),
        ),
    )
    .await
    .expect("the install ended within the deadline");
    (r, out)
}

#[tokio::test]
async fn cli_ctrl_c_while_waiting_cancels() {
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let started_tx = Mutex::new(started_tx);
    let launched = Arc::new(AtomicUsize::new(0));
    let l = Arc::clone(&launched);
    let hook: InstallHook = Arc::new(move |gate: &LaunchGate| {
        // Waiting for another installer (a blocking wait, as `lock_msi`): it ends after about
        // 10 s (1000 naps) or once Ctrl+C cancelled the gate.
        started_tx.lock().unwrap().send(()).unwrap();
        for _ in 0..1000 {
            if gate.cancelled() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !gate.try_launch() {
            return Err(WingetOpsError::Cancelled);
        }
        l.fetch_add(1, Ordering::SeqCst);
        Ok(report_ok())
    });
    let fake = Fake {
        planned: Some(PlanOrBridge::Native(native_plan())),
        install_hook: Some(hook),
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (tx, mut rx) = quiet();
    let sender = std::thread::spawn(move || {
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        tx.send(()).unwrap();
        tx
    });
    let (r, out) = install_with(&fake, &ui, &mut rx, yes()).await;
    let _tx = sender.join().unwrap();
    let e = r.unwrap_err();
    assert!(e.is::<install_cmd::Cancelled>(), "{e:#}");
    assert!(out.iter().any(|l| l.contains("Cancelado.")), "{out:?}");
    assert_eq!(launched.load(Ordering::SeqCst), 0, "nothing was launched");
    assert_eq!(
        *ui.notices.borrow(),
        vec![winget_cmd::CANCELLING_NOTICE.to_string()]
    );
}

#[tokio::test]
async fn cli_ctrl_c_after_launch_prints_notice_once() {
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let started_tx = Mutex::new(started_tx);
    let release_rx = Mutex::new(release_rx);
    let hook: InstallHook = Arc::new(move |gate: &LaunchGate| {
        assert!(gate.try_launch());
        started_tx.lock().unwrap().send(()).unwrap();
        // The installer runs until the test lets it end.
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        Ok(report_ok())
    });
    let fake = Fake {
        planned: Some(PlanOrBridge::Native(native_plan())),
        install_hook: Some(hook),
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (tx, mut rx) = quiet();
    let sender = std::thread::spawn(move || {
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        // Two Ctrl+C while the installer runs, both sent before it ends.
        tx.send(()).unwrap();
        tx.send(()).unwrap();
        release_tx.send(()).unwrap();
        tx
    });
    let (r, out) = install_with(&fake, &ui, &mut rx, yes()).await;
    let _tx = sender.join().unwrap();
    r.unwrap();
    assert!(
        out.iter().any(|l| l.contains("Instalado: Contoso App 1.0")),
        "{out:?}"
    );
    assert_eq!(
        *ui.notices.borrow(),
        vec![install_cmd::uncancellable(Some("instalador"))]
    );
}

fn known() -> KnownFolders {
    KnownFolders {
        program_files: PathBuf::from(r"C:\Program Files"),
        program_files_x86: PathBuf::from(r"C:\Program Files (x86)"),
        system_root: PathBuf::from(r"C:\Windows"),
        program_data: PathBuf::from(r"C:\ProgramData"),
        user_profiles: PathBuf::from(r"C:\Users"),
    }
}

fn user_folders() -> UserFolders {
    UserFolders {
        local_app_data: PathBuf::from(r"C:\Users\u\AppData\Local"),
        app_data: PathBuf::from(r"C:\Users\u\AppData\Roaming"),
        documents: PathBuf::from(r"C:\Users\u\Documents"),
        wingetrs_root: PathBuf::from(r"C:\Users\u\AppData\Local\wingetrs"),
    }
}

/// An NSIS record of the user (only `UninstallString`, no quiet one).
fn nsis_record(hive: Hive, key: &str) -> Result<Option<ArpDetails>, String> {
    Ok(Some(ArpDetails {
        hive,
        key: key.to_string(),
        display_name: Some("Contoso App".into()),
        publisher: Some("Contoso".into()),
        display_version: Some("0.9".into()),
        windows_installer: false,
        system_component: false,
        quiet_uninstall_string: None,
        uninstall_string: Some(r#""C:\Users\u\AppData\Local\Contoso\uninst.exe""#.into()),
        inno_app_path: None,
        bundle_cache_path: None,
        bundle_upgrade_code: None,
        install_location: None,
    }))
}

fn manifest_of(kind: &str, extra: &str) -> manifest::MergedManifest {
    let yaml = format!(
        "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Contoso App\nPublisher: Contoso\nInstallers:\n- Architecture: x64\n  InstallerType: {kind}\n  Scope: user\n  UpgradeBehavior: uninstallPrevious\n  InstallerUrl: https://h/setup.exe\n  InstallerSha256: {}\n{extra}",
        "a".repeat(64)
    );
    manifest::parse(yaml.as_bytes()).unwrap()
}

fn plan_of(kind: &str) -> WingetPlan {
    let host = Host {
        arch: "x64".into(),
        os: [10, 0, 22631, 0],
        locale: "pt-BR".into(),
    };
    match plan(
        &manifest_of(kind, ""),
        &host,
        ScopeWanted::Prefer(Scope::User),
        None,
    )
    .unwrap()
    {
        PlanOrBridge::Native(p) => p,
        PlanOrBridge::Bridge(why) => panic!("bridge: {why}"),
    }
}

#[test]
fn upgrade_previous_plan_same_in_cli_and_tui() {
    use wingetrs::winget_install::engine::InstalledKind;
    use wingetrs::winget_install::previous::{installed_facts_of, previous_plan};
    // `run_win` (CLI) and `tui::run` (TUI) both call these, through `previous_win`.
    let folders = || Ok((known(), user_folders()));
    let is_dir = |_: &str| false;

    // Nullsoft: the removal before the upgrade is silent (`/S`).
    let nsis = plan_of("nullsoft");
    let p = previous_plan("arp/hkcu/Contoso", &nsis, &nsis_record, &folders, &is_dir).unwrap();
    let c = p.command.unwrap();
    assert_eq!(c.rule, UninstallRule::Nsis, "{c:?}");
    assert!(c.args.contains("/S"), "{c:?}");
    assert!(!c.interactive);
    // Another type: no NSIS hint, the plain uninstaller (with its window).
    let inno = plan_of("inno");
    let p = previous_plan("arp/hkcu/Contoso", &inno, &nsis_record, &folders, &is_dir).unwrap();
    assert_eq!(p.command.unwrap().rule, UninstallRule::Interactive);

    // MSIX: the package family, named after the plan; the facts say MSIX.
    let mut msix = plan_of("msix");
    msix.upgrade_from = Some("0.9".into());
    let none = |_: Hive, _: &str| -> Result<Option<ArpDetails>, String> {
        panic!("an MSIX package has no ARP record")
    };
    let p = previous_plan(
        "msix/Contoso.App_8wekyb3d8bbwe",
        &msix,
        &none,
        &folders,
        &is_dir,
    )
    .unwrap();
    assert_eq!(
        p.target,
        UninstallTarget::Msix {
            family: "Contoso.App_8wekyb3d8bbwe".into()
        }
    );
    assert_eq!(
        (p.name.as_str(), p.version.as_deref()),
        ("Contoso App", Some("0.9"))
    );
    let facts = installed_facts_of("msix/Contoso.App_8wekyb3d8bbwe", &none);
    assert_eq!(facts.kind, Some(InstalledKind::Msix));
    // An ARP record: its own facts (an NSIS record says nothing of its family).
    let facts = installed_facts_of("arp/hkcu/Contoso", &nsis_record);
    assert_eq!(facts.kind, None);
}

#[test]
fn uninstall_takes_the_nsis_hint_from_the_manifest() {
    use wingetrs::winget_install::previous::{Removal, manifest_nsis, removal_plan};
    assert!(manifest_nsis(&manifest_of("nullsoft", "")));
    assert!(!manifest_nsis(&manifest_of("inno", "")));
    assert!(!manifest_nsis(&manifest_of("exe", "")));
    // A nested nullsoft installer in a zip counts.
    let nested = manifest_of(
        "zip",
        "  NestedInstallerType: nullsoft\n  NestedInstallerFiles:\n  - RelativeFilePath: setup.exe\n",
    );
    assert!(manifest_nsis(&nested));
    let folders = || Ok((known(), user_folders()));
    let removal = |nsis| Removal {
        name: "Graphviz",
        version: None,
        nsis,
        others: &[],
        is_main: true,
    };
    let p = removal_plan(
        "arp/hkcu/Graphviz",
        removal(true),
        &nsis_record,
        &folders,
        &|_| false,
    )
    .unwrap();
    let c = p.command.unwrap();
    assert_eq!((c.rule, c.interactive), (UninstallRule::Nsis, false));
    assert!(c.args.contains("/S"), "{c:?}");
}

/// A record named `Amazon Kindle` whose data another installation shares.
fn kindle(t: &TempDir, registry_only: bool) -> Fake {
    let data = t.path().join("Kindle");
    std::fs::create_dir_all(data.join("My Kindle Content")).unwrap();
    std::fs::write(data.join("My Kindle Content").join("book.azw"), b"livro").unwrap();
    Fake {
        rows: vec![row("Amazon Kindle", "arp/hkcu/Kindle", None)],
        backups: t.path().join("backups"),
        shared: Some(SharedData {
            folders: if registry_only {
                Vec::new()
            } else {
                vec![data]
            },
            size: None,
            registry_only_allowed: registry_only,
            registry_only_refused: None,
        }),
        ..Default::default()
    }
}

async fn uninstall_with(fake: &Fake, ui: &dyn Ui, f: Flags) -> (anyhow::Result<()>, Vec<String>) {
    let (_tx, mut rx) = quiet();
    let gate = fake.gate.clone();
    let mut out = Vec::new();
    let r = tokio::time::timeout(
        Duration::from_secs(30),
        winget_cmd::uninstall(
            fake,
            ui,
            "Amazon Kindle",
            false,
            f,
            Stop::new(&mut rx, &gate),
            &mut |l: &str| out.push(l.to_string()),
        ),
    )
    .await
    .expect("the removal ended within the deadline");
    (r, out)
}

#[tokio::test]
async fn cli_kindle_yes_refused() {
    let t = TempDir::new().unwrap();
    let fake = kindle(&t, false);
    // A terminal that could pick: `--yes` still never chooses.
    let ui = FakeUi {
        interactive: true,
        pick: Some(0),
        ..Default::default()
    };
    let (r, _) = uninstall_with(&fake, &ui, yes()).await;
    let e = r.unwrap_err().to_string();
    assert!(e.contains(winget_cmd::SHARED_YES_REFUSED), "{e}");
    assert_eq!(ui.picks.get(), 0);
    assert!(fake.uninstalled.lock().unwrap().is_empty());
    assert!(
        fake.steps.lock().unwrap().is_empty(),
        "nothing locked or run"
    );
    assert!(!fake.backups.exists(), "nothing copied");
}

#[tokio::test]
async fn cli_kindle_backup_then_remove() {
    let t = TempDir::new().unwrap();
    let fake = kindle(&t, false);
    let ui = FakeUi {
        interactive: true,
        pick: Some(0),
        ..Default::default()
    };
    let (r, out) = uninstall_with(&fake, &ui, Flags::default()).await;
    r.unwrap();
    assert_eq!(
        ui.picked_items.borrow()[0],
        vec![
            "Copiar as pastas para backups e remover".to_string(),
            "Remover sem copiar as pastas".to_string(),
            "Cancelar".to_string(),
        ]
    );
    // Copied under `install.lock`, before the uninstaller ran.
    assert_eq!(
        *fake.steps.lock().unwrap(),
        vec![
            "phase:Preparando".to_string(),
            "lock".to_string(),
            "phase:Copiando as pastas de dados para backups".to_string(),
            "phase:Desinstalador de Amazon Kindle rodando (pode abrir janelas)".to_string(),
            "run copied=true".to_string(),
        ]
    );
    let copies: Vec<PathBuf> = std::fs::read_dir(&fake.backups)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(copies.len(), 1, "{copies:?}");
    let dest = &copies[0];
    assert!(dest.to_string_lossy().ends_with("-pastas"), "{dest:?}");
    let folder = std::fs::read_dir(dest)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::read(folder.join("My Kindle Content").join("book.azw")).unwrap(),
        b"livro"
    );
    assert_eq!(fake.uninstalled.lock().unwrap().len(), 1);
    let text = out.join("\n");
    assert!(text.contains("Desinstalado: Amazon Kindle 1.0"), "{text}");
    assert!(
        text.contains(&format!("Pastas copiadas para {}", dest.display())),
        "{text}"
    );
}

#[tokio::test]
async fn cli_kindle_registry_only_and_cancel() {
    let t = TempDir::new().unwrap();
    let fake = kindle(&t, true);
    let ui = FakeUi {
        interactive: true,
        pick: Some(1),
        ..Default::default()
    };
    let (r, out) = uninstall_with(&fake, &ui, Flags::default()).await;
    r.unwrap();
    assert_eq!(
        ui.picked_items.borrow()[0],
        vec![
            "Remover sem copiar as pastas".to_string(),
            "Remover só o registro".to_string(),
            "Cancelar".to_string(),
        ]
    );
    assert_eq!(
        *fake.steps.lock().unwrap(),
        vec![
            "phase:Preparando".to_string(),
            "lock".to_string(),
            "phase:Removendo Amazon Kindle".to_string(),
            "registry-only".to_string(),
        ]
    );
    assert!(fake.uninstalled.lock().unwrap().is_empty());
    assert!(
        out.join("\n").contains("registro de Amazon Kindle"),
        "{out:?}"
    );

    // "Cancelar" (the last item) removes nothing.
    let fake = kindle(&t, true);
    let ui = FakeUi {
        interactive: true,
        pick: Some(2),
        ..Default::default()
    };
    let (r, out) = uninstall_with(&fake, &ui, Flags::default()).await;
    r.unwrap();
    assert!(fake.steps.lock().unwrap().is_empty());
    assert!(out.iter().any(|l| l.contains("Cancelado.")), "{out:?}");
}

#[tokio::test]
async fn cli_uninstall_shows_phases() {
    let glyphs = Glyphs::for_set(GlyphSet::Ascii);
    for (elevation, title) in [
        (
            Elevation::Never,
            "Desinstalador de Contoso App rodando (pode abrir janelas)".to_string(),
        ),
        (Elevation::Helper, phase_title(&Phase::WaitingElevation)),
    ] {
        let fake = Fake {
            rows: vec![row("Contoso App", "arp/hklm32/{G}", Some("Pub.App"))],
            elevation: Some(elevation),
            ..Default::default()
        };
        let ui = FakeUi::default();
        let (_tx, mut rx) = quiet();
        let gate = fake.gate.clone();
        tokio::time::timeout(
            Duration::from_secs(30),
            winget_cmd::uninstall(
                &fake,
                &ui,
                "Pub.App",
                false,
                yes(),
                Stop::new(&mut rx, &gate),
                &mut |_: &str| {},
            ),
        )
        .await
        .unwrap()
        .unwrap();
        // A phase while waiting for `install.lock`, then the running phase, then the
        // uninstaller.
        assert_eq!(
            *fake.steps.lock().unwrap(),
            vec![
                "phase:Preparando".to_string(),
                "lock".to_string(),
                format!("phase:{title}"),
                "run copied=false".to_string()
            ]
        );
        // What the CLI line draws from it: the phase, its clock, and that it cannot be
        // cancelled.
        let events = fake.events.lock().unwrap().clone();
        assert_eq!(events.len(), 2, "{events:?}");
        let mut line = Phases::new();
        let t0 = Instant::now();
        line.update_at(&events[1], t0);
        let shown = line
            .line_at(0, glyphs, t0 + Duration::from_secs(3))
            .unwrap();
        assert!(shown.contains(&format!("{title} · 00:03")), "{shown}");
        assert!(shown.contains("não pode ser cancelado"), "{shown}");
        // Past five minutes without an end: a window may be waiting for an answer.
        let late = line
            .line_at(0, glyphs, t0 + Duration::from_secs(6 * 60))
            .unwrap();
        assert!(
            late.contains("rodando há 6 min; pode haver uma janela"),
            "{late}"
        );
    }
    // The wait for the record to go: its clock, no window warning (it is a fixed wait).
    let checking = Phase::CheckingRemoval {
        name: "Contoso App".into(),
    };
    assert_eq!(
        phase_title(&checking),
        "Conferindo a remoção de Contoso App (até 2 min)"
    );
    let mut line = Phases::new();
    let t0 = Instant::now();
    line.update_at(&ProgressEvent::Phase(checking), t0);
    let shown = line
        .line_at(0, glyphs, t0 + Duration::from_secs(400))
        .unwrap();
    assert!(
        shown.contains("· 06:40 · não pode ser cancelado"),
        "{shown}"
    );
    assert!(!shown.contains("janela"), "{shown}");
}

#[tokio::test]
async fn winget_dry_run_shows_url_and_sha256() {
    let p = native_plan();
    let fake = Fake {
        planned: Some(PlanOrBridge::Native(p.clone())),
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (_tx, mut rx) = quiet();
    let dry = Flags {
        dry_run: true,
        ..Default::default()
    };
    let (r, out) = install_with(&fake, &ui, &mut rx, dry).await;
    r.unwrap();
    let text = out.join("\n");
    assert!(text.contains("https://h/setup.exe"), "{text}");
    assert!(
        text.contains(&format!("SHA-256: {}", "a".repeat(64))),
        "{text}"
    );
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    // Not in the normal summary.
    assert!(!winget_cmd::native_text(&p, yes()).contains("SHA-256"));
}

#[test]
fn winget_wording_says_things_once() {
    assert_eq!(
        wingetrs::winget_install::plan::bridge_reason("tipo zip: instalado pelo winget"),
        "tipo zip"
    );
    assert_eq!(
        wingetrs::winget_install::plan::bridge_reason("outro motivo"),
        "outro motivo"
    );
    assert_eq!(
        wingetrs::winget_install::plan::name_version("CrystalDiskMark 9.0.3", Some("9.0.3")),
        "CrystalDiskMark 9.0.3"
    );
    assert_eq!(
        wingetrs::winget_install::plan::name_version("CrystalDiskMark", Some("9.0.3")),
        "CrystalDiskMark 9.0.3"
    );
    assert_eq!(
        wingetrs::winget_install::plan::name_version("App", None),
        "App"
    );
    // jq installed by winget: its uninstaller is winget itself, not a window.
    let plan = UninstallPlan {
        target: UninstallTarget::Arp {
            hive: Hive::User,
            key: "jqlang.jq".into(),
        },
        name: "jq 1.7.1".into(),
        version: Some("1.7.1".into()),
        command: Some(UninstallCommand {
            rule: UninstallRule::Interactive,
            program: "winget".into(),
            args: "uninstall --product-code jqlang.jq".into(),
            interactive: true,
        }),
        elevation: Elevation::Never,
        shared: None,
    };
    let text = plan.to_string();
    assert!(text.starts_with("Desinstalar jq 1.7.1\n"), "{text}");
    assert!(!text.contains("abre a janela"), "{text}");
    assert!(text.contains("remove pelo winget"), "{text}");
    let mut other = plan.clone();
    other.command.as_mut().unwrap().program = r"C:\jq\uninst.exe".into();
    assert!(other.to_string().contains("abre a janela do desinstalador"));

    let mut root = Cli::command();
    root.build();
    let help = root
        .find_subcommand_mut("install")
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(!help.contains("próprio winget"), "{help}");
}
/// Counts launches; never runs anything.
#[derive(Default)]
struct CountingLauncher {
    runs: AtomicUsize,
}

impl Launcher for CountingLauncher {
    fn run(&self, _: &LaunchSpec) -> std::io::Result<i32> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(0)
    }
}

struct NoElevated;

impl ElevatedRunner for NoElevated {
    fn run_uninstaller(
        &self,
        _: MachineHive,
        _: &str,
        _: UninstallRule,
        _: bool,
    ) -> Result<i32, ElevatedError> {
        panic!("a user MSI runs unelevated")
    }
    fn delete_arp_key(
        &self,
        _: MachineHive,
        _: &str,
        _: DeleteReason,
    ) -> Result<(), ElevatedError> {
        panic!("not used")
    }
}

struct NoMsix;

impl MsixOps for NoMsix {
    fn add(&self, _: &Path) -> Result<AddResult, MsixError> {
        panic!("not used")
    }
    fn remove(&self, _: &str) -> Result<usize, MsixError> {
        panic!("not used")
    }
}

struct NoClock;

impl Clock for NoClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
    fn sleep(&self, _: Duration) {}
}

#[test]
fn cli_uninstall_of_msi_waits_for_msi_lock() {
    let t = TempDir::new().unwrap();
    let lock = t.path().join("msi.lock");
    // Another install holds the Windows Installer queue.
    let held = wingetrs::cache::UpdateLock::try_acquire(&lock)
        .unwrap()
        .unwrap();
    let mut record = nsis_record(Hive::User, "{11111111-2222-3333-4444-555555555555}")
        .unwrap()
        .unwrap();
    record.windows_installer = true;
    let plan = wingetrs::winget_install::uninstall::plan_uninstall(
        &record,
        false,
        &[],
        true,
        &known(),
        &user_folders(),
        &|_| false,
    )
    .unwrap();
    assert_eq!(plan.command.as_ref().unwrap().rule, UninstallRule::Msi);
    let launcher = CountingLauncher::default();
    let events: Mutex<Vec<ProgressEvent>> = Mutex::new(Vec::new());
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        s.spawn(|| {
            let gone = |_: Hive, _: &str| false;
            let u = UninstallCtx {
                launcher: &launcher,
                elevated: &NoElevated,
                msix: &NoMsix,
                key_exists: &gone,
                clock: &NoClock,
                user_expand: &|_| None,
                msiexec: Path::new(r"C:\Windows\System32\msiexec.exe"),
                helper_line: &|_| {},
            };
            let queue = MsiQueue {
                lock: lock.clone(),
                busy: &|| false,
                sleep: &|_| {},
            };
            let progress = |ev: &ProgressEvent| events.lock().unwrap().push(ev.clone());
            let r =
                wingetrs::winget_install::removal::run_uninstaller(&u, &queue, &plan, &progress);
            done_tx.send(r.map(|r| r.status)).unwrap();
        });
        // While the lock is held, msiexec is never launched.
        assert!(
            done_rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "the uninstall did not wait for msi.lock"
        );
        assert_eq!(launcher.runs.load(Ordering::SeqCst), 0);
        drop(held);
        let r = done_rx
            .recv_timeout(Duration::from_secs(20))
            .expect("the uninstall ended once msi.lock was free");
        assert_eq!(r, Ok(UninstallStatus::Removed));
    });
    assert_eq!(launcher.runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        *events.lock().unwrap(),
        vec![ProgressEvent::Phase(Phase::CheckingRemoval {
            name: "Contoso App".into()
        })]
    );
}

// ---- Task 193 review: Ctrl+C during a removal, install.lock never dropped ----

async fn uninstall_contoso(
    fake: &Fake,
    ui: &FakeUi,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<()>,
) -> (anyhow::Result<()>, Vec<String>) {
    let gate = fake.gate.clone();
    let mut out = Vec::new();
    let r = tokio::time::timeout(
        Duration::from_secs(30),
        winget_cmd::uninstall(
            fake,
            ui,
            "Pub.App",
            false,
            yes(),
            Stop::new(rx, &gate),
            &mut |l: &str| out.push(l.to_string()),
        ),
    )
    .await
    .expect("the removal ended within the deadline");
    (r, out)
}

#[tokio::test]
async fn cli_uninstall_ctrl_c_while_waiting_cancels() {
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let started_tx = Mutex::new(started_tx);
    let hook: LockHook = Arc::new(move |gate: &LaunchGate| {
        // Another operation holds `install.lock`; it is let go (after about 10 s at most)
        // right as Ctrl+C arrives: the lock is taken, but the launch gate must still refuse.
        // (`lock_install_for` itself gives up on a cancel: `lock_install_for_gives_up_on_cancel`.)
        started_tx.lock().unwrap().send(()).unwrap();
        for _ in 0..1000 {
            if gate.cancelled() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    });
    let fake = Fake {
        rows: vec![row("Contoso App", "arp/hkcu/Contoso", Some("Pub.App"))],
        lock_hook: Some(hook),
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (tx, mut rx) = quiet();
    let sender = std::thread::spawn(move || {
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        tx.send(()).unwrap();
        tx
    });
    let (r, out) = uninstall_contoso(&fake, &ui, &mut rx).await;
    let _tx = sender.join().unwrap();
    let e = r.unwrap_err();
    assert!(e.is::<install_cmd::Cancelled>(), "{e:#}");
    assert!(out.iter().any(|l| l.contains("Cancelado.")), "{out:?}");
    assert!(
        fake.uninstalled.lock().unwrap().is_empty(),
        "nothing removed"
    );
    assert_eq!(
        *fake.steps.lock().unwrap(),
        vec!["phase:Preparando".to_string(), "lock".to_string()],
        "nothing launched"
    );
    assert_eq!(
        *ui.notices.borrow(),
        vec![winget_cmd::CANCELLING_NOTICE.to_string()]
    );
}

#[tokio::test]
async fn cli_uninstall_ctrl_c_after_launch_prints_notice_once() {
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let started_tx = Mutex::new(started_tx);
    let release_rx = Mutex::new(release_rx);
    let hook: RunHook = Arc::new(move || {
        started_tx.lock().unwrap().send(()).unwrap();
        // The uninstaller runs until the test lets it end.
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
    });
    let fake = Fake {
        rows: vec![row("Contoso App", "arp/hkcu/Contoso", Some("Pub.App"))],
        run_hook: Some(hook),
        ..Default::default()
    };
    let ui = FakeUi::default();
    let (tx, mut rx) = quiet();
    let sender = std::thread::spawn(move || {
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        tx.send(()).unwrap();
        tx.send(()).unwrap();
        release_tx.send(()).unwrap();
        tx
    });
    let (r, out) = uninstall_contoso(&fake, &ui, &mut rx).await;
    let _tx = sender.join().unwrap();
    r.unwrap();
    assert!(
        out.iter()
            .any(|l| l.contains("Desinstalado: Contoso App 1.0")),
        "{out:?}"
    );
    assert_eq!(fake.uninstalled.lock().unwrap().len(), 1);
    assert_eq!(
        *ui.notices.borrow(),
        vec![install_cmd::uncancellable(Some("desinstalador"))]
    );
}

/// A held `install.lock` and Ctrl+C: the wait gives up quickly, every slice was awaited (no
/// thread left to take the lock after "Cancelado.").
#[tokio::test]
async fn lock_install_for_gives_up_on_cancel() {
    use wingetrs::install::layout::Layout;
    use wingetrs::winget_install::ops::lock_install_for;
    let t = TempDir::new().unwrap();
    let layout = Layout::new(t.path());
    std::fs::create_dir_all(layout.install_lock().parent().unwrap()).unwrap();
    let held = wingetrs::cache::UpdateLock::try_acquire(&layout.install_lock())
        .unwrap()
        .unwrap();
    let gate = LaunchGate::default();
    let (r, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            lock_install_for(&layout, LockWait::Block(Duration::from_secs(60)), &gate),
            async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                assert!(gate.try_cancel());
            }
        )
    })
    .await
    .expect("the wait gave up within the deadline");
    assert!(matches!(r, Err(WingetOpsError::Cancelled)), "{:?}", r.err());
    drop(held);
    // Nobody is still waiting for it: the lock stays free.
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        wingetrs::cache::UpdateLock::try_acquire(&layout.install_lock())
            .unwrap()
            .is_some()
    );
    // Not cancelled and free: taken.
    let gate = LaunchGate::default();
    assert!(
        lock_install_for(&layout, LockWait::NoWait, &gate)
            .await
            .is_ok()
    );
}
// ---- every question is the inline selector (Task 200) ----

/// The real CLI `Ui` on a scripted terminal: `keys` answer, the drawing is kept.
fn scripted_ui(keys: Vec<Event>) -> (install_cmd::TtyUi, Arc<Mutex<Screen>>) {
    let term = ScriptedTerm::new(keys);
    let screen = term.screen();
    let cfg = wingetrs::config::Config {
        glyphs: "ascii".into(),
        ..Default::default()
    };
    (install_cmd::TtyUi::with_term(&cfg, Box::new(term)), screen)
}

#[tokio::test]
async fn kindle_question_uses_selector() {
    let t = TempDir::new().unwrap();
    let fake = kindle(&t, true);
    let (ui, screen) = scripted_ui(vec![key(Down), key(Enter)]);
    let (r, _) = uninstall_with(&fake, &ui, Flags::default()).await;
    r.unwrap();
    assert_eq!(
        *fake.steps.lock().unwrap(),
        vec![
            "phase:Preparando".to_string(),
            "lock".to_string(),
            "phase:Removendo Amazon Kindle".to_string(),
            "registry-only".to_string(),
        ]
    );
    let text = plain(&screen.lock().unwrap().out);
    assert!(
        text.starts_with(
            "> Remover sem copiar as pastas\r\n  Remover só o registro\r\n  Cancelar\r\n"
        ),
        "{text:?}"
    );
    assert!(text.contains("> Remover só o registro\r\n"), "{text:?}");
    // Esc (or Ctrl+C) in the selector cancels: nothing locked or run.
    let fake = kindle(&t, true);
    let (ui, screen) = scripted_ui(vec![ctrl_c()]);
    let (r, out) = uninstall_with(&fake, &ui, Flags::default()).await;
    r.unwrap();
    assert!(fake.steps.lock().unwrap().is_empty());
    assert!(out.iter().any(|l| l.contains("Cancelado.")), "{out:?}");
    assert!(!screen.lock().unwrap().raw);
}

#[test]
fn orphans_question_uses_selector() {
    let t = TempDir::new().unwrap();
    let fake = Fake {
        orphans: vec![orphan("A"), orphan("B"), orphan("C")],
        backups: t.path().join("backups"),
        ..Default::default()
    };
    let (ui, screen) = scripted_ui(vec![key(Down), key(Char(' ')), key(Enter)]);
    let (r, _) = cleanup(
        &fake,
        &ui,
        CleanupArgs {
            orphans: true,
            ..Default::default()
        },
    );
    r.unwrap();
    assert_eq!(*fake.removed.borrow(), vec!["B".to_string()]);
    let text = plain(&screen.lock().unwrap().out);
    assert!(text.starts_with("> [ ] App A  HKCU"), "{text:?}");
    assert!(text.ends_with("> App B\r\n"), "{text:?}");
}

#[tokio::test]
async fn ambiguous_installed_uses_selector() {
    let fake = Fake {
        rows: vec![
            row("Amazon Kindle", "arp/hkcu/Kindle", None),
            row("Amazon Kindle", "arp/hklm32/Kindle", None),
        ],
        ..Default::default()
    };
    let (_tx, mut rx) = quiet();
    let gate = LaunchGate::default();
    // Down + Enter picks the second record; Enter on "Continuar?" keeps the default "Não".
    let (ui, screen) = scripted_ui(vec![key(Down), key(Enter), key(Enter)]);
    winget_cmd::uninstall(
        &fake,
        &ui,
        "Amazon Kindle",
        false,
        Flags::default(),
        Stop::new(&mut rx, &gate),
        &mut |_: &str| {},
    )
    .await
    .unwrap();
    assert_eq!(
        *fake.planned_targets.borrow(),
        vec![UninstallTarget::Arp {
            hive: Hive::MachineX86,
            key: "Kindle".into()
        }]
    );
    assert!(fake.uninstalled.lock().unwrap().is_empty());
    let s = screen.lock().unwrap();
    assert_eq!(s.raw_enabled, 2, "two questions, both the selector");
    let text = plain(&s.out);
    assert!(text.contains("Continuar? Não\r\n"), "{text:?}");
}
