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
    Action, App, BACKUP_ITEM, Cmd, Modal, Msg, REGISTRY_ONLY_ITEM, Running, Tab, job_uncancellable,
    update,
};
use wingetrs::tui::jobs::{ProgressThrottle, progress_msg};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Theme};
use wingetrs::tui::view::view;
use wingetrs::winget_install::launch::Elevation;
use wingetrs::winget_install::orphans::Orphan;
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan, plan};
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
    let sha = "a".repeat(64);
    let yaml = format!(
        "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Contoso App\nPublisher: Contoso\nInstallers:\n- Architecture: x64\n  InstallerType: {kind}\n  Scope: {scope}\n  InstallerUrl: https://h/setup\n  InstallerSha256: {sha}\n"
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
            vec![Cmd::PlanWingetUninstall {
                app_key: app_key.into(),
                name: "Foo".into(),
                others: Vec::new(),
                is_main: true,
            }],
            "{origin}"
        );
        // `u` plans the same removal.
        let mut u = on_installed(vec![installed_row("Foo", app_key, origin)]);
        assert_eq!(
            update(&mut u, key(KeyCode::Char('u'))),
            vec![Cmd::PlanWingetUninstall {
                app_key: app_key.into(),
                name: "Foo".into(),
                others: Vec::new(),
                is_main: true,
            }],
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
        vec![Cmd::PlanWingetUninstall {
            app_key: "arp/hklm32/Kindle".into(),
            name: "Kindle".into(),
            others: vec!["arp/hkcu/Kindle".into()],
            is_main: false,
        }]
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
        vec![Cmd::PlanWingetUninstall {
            app_key: "arp/hkcu/Kindle".into(),
            name: "Kindle".into(),
            others: vec!["arp/hklm32/Kindle".into()],
            is_main: true,
        }]
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

#[cfg(windows)]
#[test]
fn user_folders_ignore_planted_appdata() {
    use wingetrs::winget_install::folders_win::user_folders;
    let root = PathBuf::from(r"C:\dados\wingetrs");
    let before = user_folders(&root).unwrap();
    let fake = std::env::temp_dir().join("t187-fake-appdata");
    let old = std::env::var_os("APPDATA");
    // SAFETY: only this test changes APPDATA, and it puts it back below.
    unsafe { std::env::set_var("APPDATA", &fake) };
    let after = user_folders(&root);
    match old {
        // SAFETY: as above.
        Some(v) => unsafe { std::env::set_var("APPDATA", v) },
        // SAFETY: as above.
        None => unsafe { std::env::remove_var("APPDATA") },
    }
    let after = after.unwrap();
    assert_eq!(after, before);
    assert_ne!(after.app_data, fake);
    assert_eq!(after.wingetrs_root, root);
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
