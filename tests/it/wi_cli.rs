#![allow(clippy::unwrap_used, clippy::expect_used)]
//! CLI for winget packages (Task 186).
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::CommandFactory;
use tempfile::TempDir;
use wingetrs::bridge::BridgeJob;
use wingetrs::bridge::worker::ScopeArg;
use wingetrs::cache::DataDir;
use wingetrs::catalog::Catalog;
use wingetrs::cli::Cli;
use wingetrs::cli::install_cmd::{self, Flags, Interrupts, Stop, Ui};
use wingetrs::cli::winget_cmd::{self, CleanupArgs, WingetFlows};
use wingetrs::config::Behavior;
use wingetrs::core::{PackageEntry, SourceKind};
use wingetrs::http::Http;
use wingetrs::install::executor::LaunchGate;
use wingetrs::install::ops::{OpsCtx, ResidueScan};
use wingetrs::install::target::Candidate;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::installed::Hive;
use wingetrs::installed::correlate::{Correlation, ListRow, Rule};
use wingetrs::sources::winget::manifest;
use wingetrs::tui::app::{Done, InstallFail};
use wingetrs::tui::picker::PickerItem;
use wingetrs::update::LockWait;
use wingetrs::winget_install::codes;
use wingetrs::winget_install::engine::UpgradeError;
use wingetrs::winget_install::launch::Elevation;
use wingetrs::winget_install::ops::{InstallReport, WingetOpsError};
use wingetrs::winget_install::orphans::Orphan;
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan, plan};
use wingetrs::winget_install::select::{Host, Refusal, Scope, ScopeWanted};
use wingetrs::winget_install::uninstall::{
    UninstallPlan, UninstallReport, UninstallStatus, UninstallTarget,
};

// ---- fakes ----

#[derive(Default)]
struct FakeUi {
    interactive: bool,
    confirm: bool,
    pick: Option<usize>,
    picks: Cell<usize>,
    initials: RefCell<Vec<Vec<bool>>>,
}

impl Ui for FakeUi {
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn confirm(&self, _: &str) -> anyhow::Result<bool> {
        Ok(self.confirm)
    }
    fn pick(&self, _: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        self.picks.set(self.picks.get() + 1);
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
}

#[derive(Default)]
struct Fake {
    planned: Option<PlanOrBridge>,
    rows: Vec<ListRow>,
    orphans: Vec<Orphan>,
    installs: Cell<usize>,
    bridged: Cell<usize>,
    planned_targets: RefCell<Vec<UninstallTarget>>,
    uninstalled: RefCell<Vec<UninstallTarget>>,
    removed: RefCell<Vec<String>>,
    backups: PathBuf,
    pins: BTreeSet<String>,
}

impl WingetFlows for Fake {
    fn behavior(&self) -> Behavior {
        Behavior::default()
    }
    async fn plan_install(
        &self,
        _: &Candidate,
        _: ScopeWanted,
    ) -> anyhow::Result<Result<PlanOrBridge, Refusal>> {
        Ok(Ok(self.planned.clone().expect("no plan in this test")))
    }
    async fn install(&self, _: &WingetPlan) -> Result<InstallReport, WingetOpsError> {
        self.installs.set(self.installs.get() + 1);
        Ok(InstallReport {
            outcome: codes::Outcome::Ok,
            found: None,
            warning: None,
            log: None,
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
    fn plan_uninstall(
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
            elevation: Elevation::Never,
            shared: None,
        })
    }
    fn uninstall(&self, plan: &UninstallPlan) -> Result<UninstallReport, String> {
        self.uninstalled.borrow_mut().push(plan.target.clone());
        Ok(UninstallReport {
            status: UninstallStatus::Removed,
            warning: None,
            reboot: None,
        })
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
    async fn upgrade(
        &self,
        _: &WingetPlan,
        _: Option<&UninstallPlan>,
    ) -> Result<InstallReport, UpgradeError> {
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

fn cleanup(fake: &Fake, ui: &FakeUi, a: CleanupArgs) -> (anyhow::Result<()>, Vec<String>) {
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
    assert_eq!(fake.installs.get(), 1);
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
    assert_eq!(fake.installs.get(), 0);
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
    };
    install_cmd::execute(&ctx, &ui, cmd, &mut rx, &fake)
        .await
        .unwrap();
    let want = UninstallTarget::Arp {
        hive: Hive::MachineX86,
        key: "{G}".into(),
    };
    assert_eq!(*fake.planned_targets.borrow(), vec![want.clone()]);
    assert_eq!(*fake.uninstalled.borrow(), vec![want]);
}

#[test]
fn purge_refused_for_winget() {
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
    .unwrap_err()
    .to_string();
    assert!(e.contains("--purge só vale"), "{e}");
    assert!(fake.uninstalled.borrow().is_empty());
}

#[test]
fn ambiguous_name_needs_terminal() {
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
        fake.uninstalled.borrow().is_empty(),
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
