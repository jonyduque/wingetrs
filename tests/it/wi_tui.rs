#![allow(clippy::unwrap_used, clippy::expect_used)]
//! TUI for winget packages (Task 187).

use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use wingetrs::bridge::{BridgeJob, BridgeOp, Tool};
use wingetrs::config::{ConfirmMode, WingetEngine};
use wingetrs::core::{PackageEntry, SourceKind};
use wingetrs::install::progress::{Phase, ProgressEvent};
use wingetrs::installed::Hive;
use wingetrs::installed::correlate::{Correlation, ListRow, OtherRecord, UpdateState};
use wingetrs::sources::winget::manifest::{self, MergedManifest};
use wingetrs::tui::app::{
    Action, App, BACKUP_ITEM, Cmd, Modal, Msg, NATIVE_QUIT, REGISTRY_ONLY_ITEM, Running, Tab,
    job_uncancellable, update,
};
use wingetrs::tui::jobs::{ProgressThrottle, progress_msg};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Theme};
use wingetrs::tui::view::view;
use wingetrs::winget_install::launch::Elevation;
use wingetrs::winget_install::orphans::Orphan;
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan, plan};
use wingetrs::winget_install::previous::RemovalRequest;
use wingetrs::winget_install::select::{Host, Scope, ScopeWanted};
use wingetrs::winget_install::uninstall::{SharedData, UninstallPlan, UninstallTarget};

const NOT_OURS: &str = "só apps instalados pelo wingetrs podem ser removidos ou atualizados aqui";

fn key(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

fn entry(id: &str) -> PackageEntry {
    PackageEntry {
        source: SourceKind::Winget,
        source_name: "winget".into(),
        id: id.into(),
        name: "Contoso App".into(),
        moniker: None,
        version: "1.0".into(),
        publisher: None,
        description: None,
        tags: vec![],
        commands: vec![],
    }
}

fn search_app() -> App {
    let mut a = App::new(vec![entry("Pub.App")], GlyphSet::Ascii);
    a.settings.behavior.confirm = ConfirmMode::Always;
    a
}

fn host() -> Host {
    Host {
        arch: "x64".into(),
        os: [10, 0, 22631, 0],
        locale: "pt-BR".into(),
    }
}

/// A manifest with one x64 installer (as `wi_install_flow::manifest_of`).
fn manifest_of(kind: &str, scope: &str) -> MergedManifest {
    manifest_with(kind, scope, "")
}

/// `manifest_of` with `root` at the top of the manifest.
fn manifest_with(kind: &str, scope: &str, root: &str) -> MergedManifest {
    let sha = "a".repeat(64);
    let yaml = format!(
        "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Contoso App\nPublisher: Contoso\n{root}Installers:\n- Architecture: x64\n  InstallerType: {kind}\n  Scope: {scope}\n  InstallerUrl: https://h/setup\n  InstallerSha256: {sha}\n"
    );
    manifest::parse(yaml.as_bytes()).unwrap()
}

fn native(kind: &str, scope: Scope) -> WingetPlan {
    let name = match scope {
        Scope::Machine => "machine",
        Scope::User => "user",
    };
    match plan(
        &manifest_of(kind, name),
        &host(),
        ScopeWanted::Prefer(scope),
        None,
    )
    .unwrap()
    {
        PlanOrBridge::Native(p) => p,
        PlanOrBridge::Bridge(why) => panic!("bridge: {why}"),
    }
}

fn bridge_job() -> BridgeJob {
    BridgeJob {
        tool: Tool::Winget,
        op: BridgeOp::Install {
            scope: None,
            agreements: Vec::new(),
        },
        target: "Pub.App".into(),
        name: "Contoso App".into(),
        installed: None,
        available: Some("1.0".into()),
        scope: None,
    }
}

/// Items (name and manager) and notes of the open confirmation, one per line.
fn confirm_text(a: &App) -> String {
    match &a.modal {
        Some(Modal::Confirm { items, notes, .. }) => items
            .iter()
            .map(|i| format!("{} {}", i.name, i.manager))
            .chain(notes.iter().cloned())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => panic!("confirmation not open"),
    }
}

fn confirm_actions(a: &App) -> Vec<Action> {
    match &a.modal {
        Some(Modal::Confirm { actions, .. }) => actions.clone(),
        _ => panic!("confirmation not open"),
    }
}

fn items(a: &App) -> Vec<(String, bool)> {
    match &a.modal {
        Some(Modal::Confirm { items, .. }) => {
            items.iter().map(|i| (i.name.clone(), i.checked)).collect()
        }
        _ => panic!("confirmation not open"),
    }
}

fn menu_items(a: &App) -> Vec<(String, bool, Option<String>)> {
    match &a.modal {
        Some(Modal::Actions { picker, .. }) => picker
            .items()
            .iter()
            .map(|i| (i.label.clone(), i.selectable, i.note.clone()))
            .collect(),
        _ => panic!("actions menu not open"),
    }
}

/// Moves the open actions menu to the item `label` and presses Enter.
fn pick(a: &mut App, label: &str) -> Vec<Cmd> {
    let at = menu_items(a)
        .iter()
        .position(|(l, _, _)| l == label)
        .unwrap_or_else(|| panic!("{label} not in {:?}", menu_items(a)));
    for _ in 0..20 {
        let Some(Modal::Actions { picker, .. }) = &a.modal else {
            panic!("actions menu closed");
        };
        if picker.selected() == at {
            return update(a, key(KeyCode::Enter));
        }
        let code = if picker.selected() < at {
            KeyCode::Down
        } else {
            KeyCode::Up
        };
        update(a, key(code));
    }
    panic!("could not reach {label}");
}

fn installed_row(name: &str, app_key: &str, origin: &str) -> ListRow {
    ListRow {
        name: name.into(),
        app_key: app_key.into(),
        origin: origin.into(),
        installed_version: Some("1".into()),
        correlation: Correlation::Unmatched,
        state: UpdateState::Unknown,
        location: "HKCU".into(),
        ..Default::default()
    }
}

fn on_installed(rows: Vec<ListRow>) -> App {
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.settings.behavior.confirm = ConfirmMode::Always;
    a.installed = rows;
    a.tab = Tab::Installed;
    a
}

fn render(a: &App, w: u16, h: u16) -> String {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| view(a, &theme, f)).unwrap();
    format!("{}", t.backend())
}

#[test]
fn confirm_shows_pede_uac_for_machine_plan() {
    let mut a = search_app();
    assert_eq!(
        update(&mut a, key(KeyCode::Char('i'))),
        vec![Cmd::PlanWingetInstall {
            id: "Pub.App".into()
        }]
    );
    let machine = native("msi", Scope::Machine);
    assert_eq!(machine.elevation, Elevation::Helper);
    let cmds = update(
        &mut a,
        Msg::WingetInstallPlanned {
            bridge: Ok((bridge_job(), Vec::new())),
            native: Ok(PlanOrBridge::Native(machine.clone())),
        },
    );
    assert!(cmds.is_empty(), "{cmds:?}");
    let text = confirm_text(&a);
    assert!(text.contains("pede UAC"), "{text}");
    assert!(text.contains("Contoso App 1.0 (msi, machine)"), "{text}");
    assert_eq!(confirm_actions(&a), vec![Action::WingetInstall(machine)]);

    let mut b = search_app();
    update(&mut b, key(KeyCode::Char('i')));
    let user = native("inno", Scope::User);
    update(
        &mut b,
        Msg::WingetInstallPlanned {
            bridge: Ok((bridge_job(), Vec::new())),
            native: Ok(PlanOrBridge::Native(user)),
        },
    );
    let text = confirm_text(&b);
    assert!(!text.contains("pede UAC"), "{text}");
    assert!(text.contains("Contoso App 1.0 (inno, user)"), "{text}");
}

fn orphan_row() -> ListRow {
    ListRow {
        stale: Some(r"D:\Steam\jogo\unins000.exe".into()),
        ..installed_row("Jogo", "arp/hkcu/Jogo", "arp")
    }
}

fn orphan() -> Orphan {
    Orphan {
        hive: Hive::User,
        key: "Jogo".into(),
        name: "Jogo".into(),
        missing: r"D:\Steam\jogo\unins000.exe".into(),
    }
}

#[test]
fn orphan_item_never_prechecked() {
    let mut a = on_installed(vec![orphan_row()]);
    a.backups_dir = PathBuf::from(r"C:\dados\backups");
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    assert!(pick(&mut a, "Remover registro órfão").is_empty());
    assert_eq!(
        items(&a),
        [("Remover registro órfão de Jogo".to_string(), false)]
    );
    let text = confirm_text(&a);
    assert!(text.contains(r"D:\Steam\jogo\unins000.exe"), "{text}");
    assert!(
        text.contains("O app pode ter mudado de lugar (ex.: Steam em outra pasta)."),
        "{text}"
    );
    assert!(text.contains(r"C:\dados\backups"), "{text}");
    // Enter with nothing marked: nothing runs, the dialog stays.
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    assert!(matches!(a.modal, Some(Modal::Confirm { .. })));
    assert!(a.running.is_none() && a.queue.is_empty());
    // Marked by the user: it runs.
    update(&mut a, key(KeyCode::Char(' ')));
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::Execute(Action::RemoveOrphan(orphan()))]
    );

    // "Never ask" does not skip it.
    let mut n = on_installed(vec![orphan_row()]);
    n.settings.behavior.confirm = ConfirmMode::Never;
    update(&mut n, key(KeyCode::Enter));
    assert!(pick(&mut n, "Remover registro órfão").is_empty());
    assert_eq!(
        items(&n),
        [("Remover registro órfão de Jogo".to_string(), false)]
    );
    assert!(n.running.is_none() && n.queue.is_empty());

    // An orphan among the other records has its own item.
    let mut o = on_installed(vec![ListRow {
        others: vec![OtherRecord {
            app_key: "arp/hklm64/Jogo".into(),
            origin: "arp".into(),
            location: "HKLM".into(),
            installed_version: Some("1".into()),
            state: UpdateState::Unknown,
            stale: Some(r"C:\Velho\unins.exe".into()),
        }],
        ..installed_row("Jogo", "arp/hkcu/Jogo", "arp")
    }]);
    update(&mut o, key(KeyCode::Enter));
    assert!(pick(&mut o, "Remover registro órfão (HKLM)").is_empty());
    update(&mut o, key(KeyCode::Char(' ')));
    assert_eq!(
        update(&mut o, key(KeyCode::Enter)),
        vec![Cmd::Execute(Action::RemoveOrphan(Orphan {
            hive: Hive::MachineX64,
            missing: r"C:\Velho\unins.exe".into(),
            ..orphan()
        }))]
    );
}

#[test]
fn remove_enabled_for_arp_and_msix_rows() {
    for (origin, app_key) in [("arp", "arp/hkcu/Foo"), ("msix", "msix/Foo_abc")] {
        let mut a = on_installed(vec![installed_row("Foo", app_key, origin)]);
        update(&mut a, key(KeyCode::Enter));
        let remove = menu_items(&a)
            .into_iter()
            .find(|(l, _, _)| l == "Remover")
            .unwrap();
        assert_eq!(remove, ("Remover".to_string(), true, None), "{origin}");
        assert_eq!(
            pick(&mut a, "Remover"),
            vec![Cmd::PlanWingetUninstall(RemovalRequest {
                app_key: app_key.into(),
                name: "Foo".into(),
                version: Some("1".into()),
                package: None,
                others: Vec::new(),
                is_main: true,
            })],
            "{origin}"
        );
        // `u` plans the same removal.
        let mut u = on_installed(vec![installed_row("Foo", app_key, origin)]);
        assert_eq!(
            update(&mut u, key(KeyCode::Char('u'))),
            vec![Cmd::PlanWingetUninstall(RemovalRequest {
                app_key: app_key.into(),
                name: "Foo".into(),
                version: Some("1".into()),
                package: None,
                others: Vec::new(),
                is_main: true,
            })],
            "{origin}"
        );
    }
    let mut s = on_installed(vec![installed_row("7zip", "scoop/7zip", "scoop")]);
    update(&mut s, key(KeyCode::Enter));
    let remove = menu_items(&s)
        .into_iter()
        .find(|(l, _, _)| l == "Remover")
        .unwrap();
    assert_eq!(
        remove,
        ("Remover".to_string(), false, Some(NOT_OURS.to_string()))
    );
}

#[test]
fn engine_winget_routes_bridge() {
    let plan = native("inno", Scope::User);
    let planned = |a: &mut App, native: Result<PlanOrBridge, String>| {
        assert_eq!(
            update(a, key(KeyCode::Char('i'))),
            vec![Cmd::PlanWingetInstall {
                id: "Pub.App".into()
            }]
        );
        update(
            a,
            Msg::WingetInstallPlanned {
                bridge: Ok((bridge_job(), Vec::new())),
                native,
            },
        )
    };

    let mut w = search_app();
    w.settings.behavior.winget_engine = WingetEngine::Winget;
    assert!(planned(&mut w, Ok(PlanOrBridge::Native(plan.clone()))).is_empty());
    let actions = confirm_actions(&w);
    assert_eq!(actions, vec![Action::Bridge(bridge_job())]);
    assert!(matches!(&actions[0], Action::Bridge(j) if matches!(j.op, BridgeOp::Install { .. })));
    // The user's setting: no "why" note.
    assert!(!confirm_text(&w).contains("Pelo winget"));

    let mut n = search_app();
    n.settings.behavior.winget_engine = WingetEngine::Wingetrs;
    assert!(planned(&mut n, Ok(PlanOrBridge::Native(plan.clone()))).is_empty());
    assert_eq!(confirm_actions(&n), vec![Action::WingetInstall(plan)]);

    // Native engine, but the planner hands it to winget: the bridge, with the reason.
    let mut b = search_app();
    assert!(planned(&mut b, Ok(PlanOrBridge::Bridge("instalador burn".into()))).is_empty());
    assert_eq!(confirm_actions(&b), vec![Action::Bridge(bridge_job())]);
    assert!(confirm_text(&b).contains("Pelo winget: instalador burn"));

    // Native engine and a refusal: nothing to confirm, the reason in a toast.
    let mut r = search_app();
    assert!(planned(&mut r, Err("nenhum instalador para x64".into())).is_empty());
    assert!(r.modal.is_none());
    assert!(
        r.toasts
            .iter()
            .any(|t| t.text.contains("nenhum instalador para x64"))
    );
    // The winget engine does not care about the refusal.
    let mut wr = search_app();
    wr.settings.behavior.winget_engine = WingetEngine::Winget;
    assert!(planned(&mut wr, Err("nenhum instalador para x64".into())).is_empty());
    assert_eq!(confirm_actions(&wr), vec![Action::Bridge(bridge_job())]);
}

#[test]
fn status_bar_waiting_uac_text() {
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.running = Some(Running {
        id: 1,
        label: "Instalando Contoso App".into(),
        progress: None,
        action: Some(Action::WingetInstall(native("msi", Scope::Machine))),
    });
    let now = Instant::now();
    let msg = progress_msg(
        &ProgressThrottle::new(now),
        now,
        &ProgressEvent::Phase(Phase::WaitingElevation),
    )
    .unwrap();
    update(&mut a, msg);
    assert!(job_uncancellable(&a));
    let screen = render(&a, 200, 16);
    let bar = screen.lines().last().unwrap();
    assert!(bar.contains("Instalando Contoso App"), "{bar}");
    assert!(bar.contains("Aguardando o UAC"), "{bar}");
    assert!(bar.contains("não pode ser cancelado"), "{bar}");
}

#[test]
fn stale_text_points_to_action() {
    let mut a = on_installed(vec![orphan_row()]);
    // Wide enough for the note on one line of the details panel.
    a.size = (400, 30);
    let screen = render(&a, 400, 30);
    let flat: String = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("Sobra provável"), "{screen}");
    assert!(flat.contains("Remover registro órfão"), "{screen}");
    assert!(!flat.contains("fase 2d"), "{screen}");
}

fn kindle_plan(registry_only: bool) -> UninstallPlan {
    UninstallPlan {
        target: UninstallTarget::Arp {
            hive: Hive::User,
            key: "Kindle".into(),
        },
        name: "Kindle".into(),
        version: Some("1.40".into()),
        command: None,
        elevation: Elevation::Never,
        shared: Some(SharedData {
            folders: vec![PathBuf::from(r"C:\Users\t\AppData\Local\Amazon")],
            size: None,
            registry_only_allowed: registry_only,
            registry_only_refused: None,
        }),
    }
}

fn kindle_row() -> ListRow {
    ListRow {
        others: vec![OtherRecord {
            app_key: "arp/hklm32/Kindle".into(),
            origin: "arp".into(),
            location: "HKLM (32 bits)".into(),
            installed_version: Some("1.30".into()),
            state: UpdateState::Unknown,
            stale: None,
        }],
        ..installed_row("Kindle", "arp/hkcu/Kindle", "arp")
    }
}

#[test]
fn kindle_items() {
    // The row's other record: not the principal one.
    let mut a = on_installed(vec![kindle_row()]);
    update(&mut a, key(KeyCode::Enter));
    assert_eq!(
        pick(&mut a, "Remover o registro HKLM (32 bits)"),
        vec![Cmd::PlanWingetUninstall(RemovalRequest {
            app_key: "arp/hklm32/Kindle".into(),
            name: "Kindle".into(),
            version: Some("1.30".into()),
            package: None,
            others: vec!["arp/hkcu/Kindle".into()],
            is_main: false,
        })]
    );
    update(&mut a, Msg::WingetUninstallPlanned(Ok(kindle_plan(true))));
    assert_eq!(
        items(&a),
        [
            ("Kindle".to_string(), true),
            (BACKUP_ITEM.to_string(), true),
            (REGISTRY_ONLY_ITEM.to_string(), false),
        ]
    );
    assert_eq!(BACKUP_ITEM, "Copiar pastas para backups antes");
    assert_eq!(REGISTRY_ONLY_ITEM, "Remover só o registro");
    // Marking "only the registry" replaces the uninstaller.
    update(&mut a, key(KeyCode::Down));
    update(&mut a, key(KeyCode::Down));
    update(&mut a, key(KeyCode::Char(' ')));
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::Execute(Action::WingetRegistryOnly(kindle_plan(true)))]
    );

    // The principal record: no "only the registry"; unchecking the copy drops the folders.
    let mut m = on_installed(vec![kindle_row()]);
    m.settings.behavior.confirm = ConfirmMode::Never;
    update(&mut m, key(KeyCode::Enter));
    assert_eq!(
        pick(&mut m, "Remover"),
        vec![Cmd::PlanWingetUninstall(RemovalRequest {
            app_key: "arp/hkcu/Kindle".into(),
            name: "Kindle".into(),
            version: Some("1".into()),
            package: None,
            others: vec!["arp/hklm32/Kindle".into()],
            is_main: true,
        })]
    );
    // The shared-data choice is shown even with "never ask".
    update(&mut m, Msg::WingetUninstallPlanned(Ok(kindle_plan(false))));
    assert_eq!(
        items(&m),
        [
            ("Kindle".to_string(), true),
            (BACKUP_ITEM.to_string(), true),
        ]
    );
    update(&mut m, key(KeyCode::Down));
    update(&mut m, key(KeyCode::Char(' ')));
    let mut without_copy = kindle_plan(false);
    without_copy.shared.as_mut().unwrap().folders.clear();
    assert_eq!(
        update(&mut m, key(KeyCode::Enter)),
        vec![Cmd::Execute(Action::WingetUninstall(without_copy))]
    );
}

/// `UserFolders` comes from the known folders: an `APPDATA` planted in the environment of a
/// child process (this test, run again) changes nothing; this binary's environment is never
/// changed.
#[cfg(windows)]
#[test]
fn user_folders_ignore_planted_appdata() {
    use wingetrs::winget_install::folders_win::user_folders;
    const CHILD: &str = "WINGETRS_T187_CHILD";
    let root = PathBuf::from(r"C:\dados\wingetrs");
    let mine = user_folders(&root).unwrap();
    if std::env::var_os(CHILD).is_some() {
        println!("T187 {}", mine.app_data.display());
        return;
    }
    assert_eq!(mine.wingetrs_root, root);
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "wi_tui::user_folders_ignore_planted_appdata",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env("APPDATA", r"C:\t187-planted\Roaming")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(!text.contains("t187-planted"), "{text}");
    assert!(
        text.contains(&format!("T187 {}", mine.app_data.display())),
        "{text}"
    );
}

#[test]
fn upgrade_native_follows_engine() {
    let row = ListRow {
        correlation: Correlation::Matched {
            source: "winget".into(),
            package_id: "Pub.App".into(),
            rule: wingetrs::installed::correlate::Rule::ProductCode,
        },
        latest: Some("1.0".into()),
        state: UpdateState::Available {
            latest: "1.0".into(),
        },
        installed_version: Some("0.9".into()),
        ..installed_row("Contoso App", "arp/hkcu/Contoso", "arp")
    };
    let upgrade = match plan(
        &manifest_of("inno", "user"),
        &host(),
        ScopeWanted::Prefer(Scope::User),
        Some("0.9".into()),
    )
    .unwrap()
    {
        PlanOrBridge::Native(p) => p,
        PlanOrBridge::Bridge(why) => panic!("bridge: {why}"),
    };
    let loaded = |a: &mut App, latest: &str| {
        update(
            a,
            Msg::NativeUpgradeLoaded {
                app_key: "arp/hkcu/Contoso".into(),
                latest: latest.into(),
                plan: upgrade.clone(),
            },
        );
    };

    // Engine `wingetrs` with a native plan for the offered version: the native upgrade.
    let mut a = on_installed(vec![row.clone()]);
    loaded(&mut a, "1.0");
    update(&mut a, key(KeyCode::Enter));
    assert_eq!(menu_items(&a)[0], ("Atualizar".to_string(), true, None));
    assert!(pick(&mut a, "Atualizar").is_empty());
    assert_eq!(
        confirm_actions(&a),
        vec![Action::WingetUpgrade {
            plan: upgrade.clone(),
            row_key: "arp/hkcu/Contoso".into(),
        }]
    );
    let text = confirm_text(&a);
    assert!(text.contains("Contoso App 1.0 (inno, user)"), "{text}");

    // Engine `winget`: the bridge, plan or not.
    let mut w = on_installed(vec![row.clone()]);
    w.settings.behavior.winget_engine = WingetEngine::Winget;
    loaded(&mut w, "1.0");
    update(&mut w, key(KeyCode::Enter));
    assert_eq!(
        menu_items(&w)[0],
        ("Atualizar (via winget)".to_string(), true, None)
    );
    assert!(pick(&mut w, "Atualizar (via winget)").is_empty());
    assert!(matches!(
        confirm_actions(&w).as_slice(),
        [Action::Bridge(j)] if j.op == BridgeOp::Upgrade
    ));

    // A plan for another catalog version, or none: the bridge.
    let mut old = on_installed(vec![row.clone()]);
    loaded(&mut old, "0.95");
    update(&mut old, key(KeyCode::Enter));
    assert_eq!(menu_items(&old)[0].0, "Atualizar (via winget)");
    let mut none = on_installed(vec![row]);
    update(&mut none, key(KeyCode::Enter));
    assert_eq!(menu_items(&none)[0].0, "Atualizar (via winget)");
}

const EULA: &str = "Agreements:\n- AgreementLabel: EULA\n  AgreementUrl: https://x/eula\n";

fn plan_from(m: &MergedManifest, from: Option<&str>) -> WingetPlan {
    match plan(
        m,
        &host(),
        ScopeWanted::Prefer(Scope::User),
        from.map(str::to_string),
    )
    .unwrap()
    {
        PlanOrBridge::Native(p) => p,
        PlanOrBridge::Bridge(why) => panic!("bridge: {why}"),
    }
}

fn ctrl_c() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
}

#[test]
fn licenses_always_confirm_native() {
    let with_eula = plan_from(&manifest_with("inno", "user", EULA), None);
    let mut a = search_app();
    a.settings.behavior.confirm = ConfirmMode::Never;
    update(&mut a, key(KeyCode::Char('i')));
    update(
        &mut a,
        Msg::WingetInstallPlanned {
            bridge: Ok((bridge_job(), Vec::new())),
            native: Ok(PlanOrBridge::Native(with_eula.clone())),
        },
    );
    let text = confirm_text(&a);
    assert!(text.contains("Licença: EULA — https://x/eula"), "{text}");
    assert_eq!(confirm_actions(&a), vec![Action::WingetInstall(with_eula)]);
    // Without agreements, "never ask" queues it at once.
    let plain = plan_from(&manifest_of("inno", "user"), None);
    let mut q = search_app();
    q.settings.behavior.confirm = ConfirmMode::Never;
    update(&mut q, key(KeyCode::Char('i')));
    assert_eq!(
        update(
            &mut q,
            Msg::WingetInstallPlanned {
                bridge: Ok((bridge_job(), Vec::new())),
                native: Ok(PlanOrBridge::Native(plain.clone())),
            },
        ),
        vec![Cmd::Execute(Action::WingetInstall(plain))]
    );

    // The native upgrade too.
    let upgrade = plan_from(&manifest_with("inno", "user", EULA), Some("0.9"));
    let row = ListRow {
        correlation: Correlation::Matched {
            source: "winget".into(),
            package_id: "Pub.App".into(),
            rule: wingetrs::installed::correlate::Rule::ProductCode,
        },
        latest: Some("1.0".into()),
        state: UpdateState::Available {
            latest: "1.0".into(),
        },
        installed_version: Some("0.9".into()),
        ..installed_row("Contoso App", "arp/hkcu/Contoso", "arp")
    };
    let mut u = on_installed(vec![row]);
    u.settings.behavior.confirm = ConfirmMode::Never;
    update(
        &mut u,
        Msg::NativeUpgradeLoaded {
            app_key: "arp/hkcu/Contoso".into(),
            latest: "1.0".into(),
            plan: upgrade.clone(),
        },
    );
    update(&mut u, key(KeyCode::Enter));
    assert!(pick(&mut u, "Atualizar").is_empty());
    let text = confirm_text(&u);
    assert!(text.contains("Licença: EULA — https://x/eula"), "{text}");
    assert_eq!(
        confirm_actions(&u),
        vec![Action::WingetUpgrade {
            plan: upgrade,
            row_key: "arp/hkcu/Contoso".into(),
        }]
    );
}

#[test]
fn bridge_reason_toasts_without_confirmation() {
    let mut a = search_app();
    a.settings.behavior.confirm = ConfirmMode::Never;
    update(&mut a, key(KeyCode::Char('i')));
    let cmds = update(
        &mut a,
        Msg::WingetInstallPlanned {
            bridge: Ok((bridge_job(), Vec::new())),
            native: Ok(PlanOrBridge::Bridge("instalador burn".into())),
        },
    );
    assert_eq!(cmds, vec![Cmd::Execute(Action::Bridge(bridge_job()))]);
    assert!(a.modal.is_none());
    assert!(
        a.toasts
            .iter()
            .any(|t| t.text == "Pelo winget: instalador burn"),
        "{:?}",
        a.toasts.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
}

fn running(action: Action) -> App {
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.running = Some(Running {
        id: 1,
        label: "x".into(),
        progress: None,
        action: Some(action),
    });
    a
}

fn removing(name: &str) -> Msg {
    Msg::InstallPhase(Phase::Removing { app: name.into() })
}

#[test]
fn launched_removal_is_uncancellable() {
    let plan = UninstallPlan {
        shared: None,
        ..kindle_plan(false)
    };
    for action in [
        Action::WingetUninstall(plan.clone()),
        Action::WingetRegistryOnly(plan),
        Action::RemoveOrphan(orphan()),
    ] {
        let mut a = running(action.clone());
        assert!(!job_uncancellable(&a), "{action:?}");
        update(&mut a, removing("Kindle"));
        assert!(job_uncancellable(&a), "{action:?}");
        assert!(update(&mut a, ctrl_c()).is_empty());
        match &a.modal {
            Some(Modal::ConfirmQuit { text }) => assert_eq!(text, NATIVE_QUIT),
            _ => panic!("quit dialog not open"),
        }
        // A second Ctrl+C does not leave it behind, nor does it cancel anything.
        assert!(!update(&mut a, ctrl_c()).contains(&Cmd::Quit));
        let cmds = update(&mut a, key(KeyCode::Enter));
        assert!(!cmds.contains(&Cmd::CancelNative), "{cmds:?}");
        assert!(a.quit_after_running);
    }
    // Not a native removal: the phase does not lock it.
    let mut s = running(Action::Bridge(bridge_job()));
    update(&mut s, removing("7zip"));
    assert!(!job_uncancellable(&s));
}

#[test]
fn quitting_cancels_native_job_before_launch() {
    use wingetrs::install::executor::LaunchGate;
    let install = Action::WingetInstall(native("inno", Scope::User));
    // Downloading: confirming the quit cancels the job's launch gate.
    let mut a = running(install.clone());
    assert!(update(&mut a, ctrl_c()).is_empty());
    assert_eq!(update(&mut a, key(KeyCode::Enter)), vec![Cmd::CancelNative]);
    // A second Ctrl+C cancels it too, then quits.
    let mut b = running(install.clone());
    update(&mut b, ctrl_c());
    assert_eq!(update(&mut b, ctrl_c()), vec![Cmd::CancelNative, Cmd::Quit]);
    // Launched: nothing to cancel.
    let mut c = running(install);
    update(
        &mut c,
        Msg::InstallPhase(Phase::RunningInstaller { name: "x".into() }),
    );
    update(&mut c, ctrl_c());
    assert!(update(&mut c, key(KeyCode::Enter)).is_empty());
    // Not a native job: never sent.
    let mut d = running(Action::Bridge(bridge_job()));
    update(&mut d, ctrl_c());
    assert!(update(&mut d, key(KeyCode::Enter)).is_empty());
    // The gate the cancel goes to: once cancelled, the installer is never launched.
    let gate = LaunchGate::default();
    assert!(gate.try_cancel());
    assert!(!gate.try_launch());
}

/// Task 193: the TUI and the CLI plan the same removal of the same nullsoft ARP row (one
/// request, one planning function, the NSIS hint from the manifest in both).
#[test]
fn tui_and_cli_plan_the_same_nullsoft_removal() {
    use wingetrs::installed::correlate::Rule as MatchRule;
    use wingetrs::winget_install::elevated_rules::KnownFolders;
    use wingetrs::winget_install::previous::plain_removal_plan;
    use wingetrs::winget_install::uninstall::UserFolders;
    use wingetrs::winget_install::uninstall_cmd::{ArpDetails, Rule};
    let row = ListRow {
        correlation: Correlation::Matched {
            source: "winget".into(),
            package_id: "Pub.App".into(),
            rule: MatchRule::ProductCode,
        },
        ..kindle_row()
    };
    let mut a = on_installed(vec![row.clone()]);
    update(&mut a, key(KeyCode::Enter));
    let cmds = pick(&mut a, "Remover");
    let [Cmd::PlanWingetUninstall(tui)] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    let cli = RemovalRequest::of_row(&row);
    assert_eq!(*tui, cli);
    assert_eq!(cli.package.as_deref(), Some("Pub.App"));

    let yaml = format!(
        "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Kindle\nPublisher: Amazon\nInstallers:\n- Architecture: x64\n  InstallerType: nullsoft\n  InstallerUrl: https://h/setup.exe\n  InstallerSha256: {}\n",
        "a".repeat(64)
    );
    let m = manifest::parse(yaml.as_bytes()).unwrap();
    let details = |hive: Hive, key: &str| -> Result<Option<ArpDetails>, String> {
        Ok(Some(ArpDetails {
            hive,
            key: key.to_string(),
            display_name: Some("Kindle".into()),
            publisher: Some("Amazon".into()),
            display_version: Some("1".into()),
            windows_installer: false,
            system_component: false,
            quiet_uninstall_string: None,
            uninstall_string: Some(r#""C:\Users\u\AppData\Local\Kindle\uninst.exe""#.into()),
            inno_app_path: None,
            bundle_cache_path: None,
            bundle_upgrade_code: None,
            install_location: None,
        }))
    };
    let folders = || {
        Ok((
            KnownFolders {
                program_files: PathBuf::from(r"C:\Program Files"),
                program_files_x86: PathBuf::from(r"C:\Program Files (x86)"),
                system_root: PathBuf::from(r"C:\Windows"),
                program_data: PathBuf::from(r"C:\ProgramData"),
                user_profiles: PathBuf::from(r"C:\Users"),
            },
            UserFolders {
                local_app_data: PathBuf::from(r"C:\Users\u\AppData\Local"),
                app_data: PathBuf::from(r"C:\Users\u\AppData\Roaming"),
                documents: PathBuf::from(r"C:\Users\u\Documents"),
                wingetrs_root: PathBuf::from(r"C:\Users\u\AppData\Local\wingetrs"),
            },
        ))
    };
    let tui_plan = plain_removal_plan(tui, Some(&m), &details, &folders, &|_| false).unwrap();
    let cli_plan = plain_removal_plan(&cli, Some(&m), &details, &folders, &|_| false).unwrap();
    assert_eq!(tui_plan, cli_plan);
    let c = cli_plan.command.as_ref().unwrap();
    assert_eq!((c.rule, c.interactive), (Rule::Nsis, false), "{c:?}");
    assert!(c.args.contains("/S"), "{c:?}");
    // The other ARP record makes the data shared, in both.
    assert!(cli_plan.shared.is_some());
    // Without a readable manifest there is no hint (the plain uninstaller).
    let no_hint = plain_removal_plan(&cli, None, &details, &folders, &|_| false).unwrap();
    assert_eq!(no_hint.command.unwrap().rule, Rule::Interactive);
}
