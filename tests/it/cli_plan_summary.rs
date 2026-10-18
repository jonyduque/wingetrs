#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::PathBuf;
use wingetrs::cli::output::{self, Phases};
use wingetrs::cli::style;
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, HashAlgo, HashSpec, Source};
use wingetrs::install::plan::{
    InstallPlan, PersistKind, PersistSpec, ShimKind, ShimSpec, ShortcutSpec, Step, UninstallPlan,
};
use wingetrs::install::progress::{Phase, ProgressEvent};
use wingetrs::install::state::{Artifact, ArtifactKind};
use wingetrs::tui::theme::{GlyphSet, Glyphs};

const MIB: u64 = 1024 * 1024;

fn ascii() -> &'static Glyphs {
    Glyphs::for_set(GlyphSet::Ascii)
}

fn source(file: &str) -> Source {
    Source {
        url: format!("https://example.invalid/{file}"),
        file_name: file.to_string(),
        hash: HashSpec {
            algo: HashAlgo::Sha256,
            hex: "ab".repeat(32),
        },
        extract_dir: None,
        extract_to: None,
    }
}

fn shim(name: &str) -> Step {
    Step::Shim(ShimSpec {
        name: name.into(),
        target: PathBuf::from(r"C:\R\apps\app\current").join(format!("{name}.exe")),
        args: None,
        kind: ShimKind::Exe,
    })
}

fn plan(steps: Vec<Step>) -> InstallPlan {
    InstallPlan {
        app: "app".into(),
        bucket: "main".into(),
        version: "2.0".into(),
        arch: Some(Arch::X64),
        upgrade_from: None,
        root: PathBuf::from(r"C:\R"),
        manifest_sha256: "00".into(),
        third_party_uninstaller: false,
        steps,
    }
}

fn summary(p: &InstallPlan) -> String {
    style::with_enabled(false, || {
        output::plan_summary(p, &Layout::new(p.root.clone()), ascii(), None, false)
    })
}

#[test]
fn summary_hides_internal_steps() {
    let p = plan(vec![
        Step::EnsureLauncher,
        Step::Download {
            index: 0,
            source: source("app-2.0.zip"),
        },
        Step::Extract { index: 0 },
        Step::Promote,
        Step::LinkCurrent,
        shim("a"),
        shim("b"),
        Step::EnsureShimsOnPath,
        Step::RecordState,
    ]);
    let t = summary(&p);
    let numbered: Vec<&str> = t
        .lines()
        .filter(|l| l.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert_eq!(numbered.len(), 2, "{t}");
    assert!(numbered[0].contains("Comandos: a, b"), "{t}");
    assert!(numbered[1].contains("PATH do usuário"), "{t}");
    assert!(!t.contains("example.invalid"), "{t}");
    assert!(!t.contains("app-2.0.zip"), "{t}");
    assert!(!t.contains("state.db"), "{t}");
    assert!(!t.contains("--allow-msiexec"), "{t}");
}

#[test]
fn summary_keeps_every_outside_effect() {
    let p = plan(vec![
        Step::Persist(PersistSpec {
            in_version: PathBuf::from("data"),
            in_persist: PathBuf::from("data"),
            kind: PersistKind::Dir,
        }),
        Step::Shortcut(ShortcutSpec {
            link: PathBuf::from(r"C:\SM\App Tool.lnk"),
            target: PathBuf::from(r"C:\R\apps\app\current\app.exe"),
            args: None,
            icon: None,
        }),
        Step::EnvSet {
            name: "APP_HOME".into(),
            value: r"C:\R\apps\app\current".into(),
        },
        Step::EnvAddPath {
            entry: PathBuf::from(r"C:\R\apps\app\current\bin"),
        },
        Step::RemoveOldVersion {
            version: "1.0".into(),
        },
    ]);
    let t = summary(&p);
    assert!(t.contains("Dados mantidos entre versões: data"), "{t}");
    assert!(t.contains("Atalho no Menu Iniciar: App Tool"), "{t}");
    assert!(!t.contains(".lnk"), "{t}");
    assert!(
        t.contains(r"Variável de usuário APP_HOME = C:\R\apps\app\current"),
        "{t}"
    );
    assert!(
        t.contains(r"PATH do usuário -> C:\R\apps\app\current\bin"),
        "{t}"
    );
    assert!(t.contains("Remove a versão 1.0"), "{t}");
}

#[test]
fn summary_upgrade_header_uses_arrow() {
    let mut p = plan(vec![]);
    p.upgrade_from = Some("1.0".into());
    let t = summary(&p);
    assert!(t.lines().next().unwrap().contains("1.0 -> 2.0"), "{t}");
    assert_eq!(t.lines().count(), 1, "{t}");
}

#[test]
fn summary_msi_note() {
    let with = |file: &str| {
        summary(&plan(vec![Step::Download {
            index: 0,
            source: source(file),
        }]))
    };
    assert!(with("setup.msi").contains("pede --allow-msiexec"));
    assert!(!with("setup.zip").contains("--allow-msiexec"));
    let p = plan(vec![Step::Download {
        index: 0,
        source: source("setup.msi"),
    }]);
    let allowed = style::with_enabled(false, || {
        output::plan_summary(&p, &Layout::new(p.root.clone()), ascii(), None, true)
    });
    assert!(
        allowed.contains("usa o Windows Installer (msiexec /a)"),
        "{allowed}"
    );
    assert!(!allowed.contains("pede --allow-msiexec"), "{allowed}");
}

#[test]
fn summary_omits_path_lines_already_on_the_user_path() {
    let p = plan(vec![
        Step::EnsureShimsOnPath,
        Step::EnvAddPath {
            entry: PathBuf::from(r"C:\R\apps\app\current\bin"),
        },
    ]);
    let with = |path: Option<&str>| {
        style::with_enabled(false, || {
            output::plan_summary(&p, &Layout::new(p.root.clone()), ascii(), path, false)
        })
    };
    let absent = with(Some(r"C:\Windows"));
    assert!(absent.contains(r"-> C:\R\shims"), "{absent}");
    assert!(absent.contains(r"-> C:\R\apps\app\current\bin"), "{absent}");
    let only_shims = with(Some(r"C:\Windows;c:\r\SHIMS\"));
    assert!(!only_shims.contains(r"C:\R\shims"), "{only_shims}");
    assert!(only_shims.contains(r"current\bin"), "{only_shims}");
    let both = with(Some(r"C:\R\shims;C:\R\apps\app\current\bin"));
    assert_eq!(both.lines().count(), 1, "{both}");
}

#[test]
fn bold_numbers_when_colored() {
    let p = plan(vec![shim("a")]);
    let layout = Layout::new(p.root.clone());
    let on = style::with_enabled(true, || {
        output::plan_summary(&p, &layout, ascii(), None, false)
    });
    assert!(on.contains(&style::paint_raw(style::NAME, "1")), "{on:?}");
    let off = summary(&p);
    assert!(!off.contains('\x1b'), "{off:?}");
}

fn dl(url: &str, done: u64, total: Option<u64>) -> ProgressEvent {
    ProgressEvent::Download {
        url: url.into(),
        done,
        total,
    }
}

fn installer() -> ProgressEvent {
    ProgressEvent::Phase(Phase::Installer {
        log: "msiexec.log".into(),
    })
}

fn line(ph: &Phases) -> String {
    ph.line(0, ascii()).unwrap()
}

#[test]
fn installer_label_is_not_a_download() {
    let mut ph = Phases::new();
    ph.update(&dl(
        "https://example.invalid/x.msi",
        5 * MIB,
        Some(10 * MIB),
    ));
    let l = line(&ph);
    assert!(l.contains("Baixando") && l.contains("50%"), "{l}");
    assert!(l.contains("5.0 / 10.0 MiB"), "{l}");
    ph.update(&installer());
    let l = line(&ph);
    assert!(l.contains("Windows Installer (msiexec /a)"), "{l}");
    assert!(
        l.contains("não pode ser cancelado") && l.contains("00:0"),
        "{l}"
    );
    assert!(!l.contains("Baixando"), "{l}");
}

#[test]
fn cached_download_shows_no_zero_line() {
    let mut ph = Phases::new();
    assert!(ph.line(0, ascii()).is_none());
    ph.update(&installer());
    let l = line(&ph);
    assert!(!l.contains("Baixando") && !l.contains("0.0 MiB"), "{l}");
}

#[test]
fn installing_phase_after_every_download_finished() {
    let mut ph = Phases::new();
    ph.update(&dl("u1", 3 * MIB, Some(3 * MIB)));
    ph.update(&dl("u2", MIB, Some(2 * MIB)));
    assert!(line(&ph).contains("Baixando"));
    ph.update(&dl("u2", 2 * MIB, Some(2 * MIB)));
    let l = line(&ph);
    assert!(l.contains("Instalando…") && !l.contains("Baixando"), "{l}");
    ph.update(&dl("u3", 0, None));
    assert!(line(&ph).contains("Baixando 5.0 MiB"));
}

#[test]
fn download_bar_has_twenty_columns() {
    let mut ph = Phases::new();
    ph.update(&dl("u", 5 * MIB, Some(20 * MIB)));
    let l = line(&ph);
    assert!(l.contains("[#####---------------] 25%"), "{l}");
}

fn art(kind: ArtifactKind, target: &str) -> Artifact {
    Artifact {
        kind,
        target: target.into(),
        previous: None,
    }
}

fn uplan(purge: bool, remove_path: bool) -> UninstallPlan {
    UninstallPlan {
        app: "app".into(),
        version: "2.0".into(),
        purge,
        artifacts: vec![
            art(ArtifactKind::Shim, "a"),
            art(ArtifactKind::Shim, "b"),
            art(ArtifactKind::Shortcut, r"C:\SM\App Tool.lnk"),
            art(ArtifactKind::Path, r"C:\R\shims"),
            art(ArtifactKind::Env, "APP_HOME"),
            art(ArtifactKind::Persist, "data"),
        ],
        path_users: vec![],
        remove_path_entry: remove_path,
    }
}

fn utext(p: &UninstallPlan) -> String {
    style::with_enabled(false, || {
        output::uninstall_plan_text(p, &Layout::new(r"C:\R"), ascii())
    })
}

#[test]
fn uninstall_summary() {
    let t = utext(&uplan(false, true));
    let want = [
        "Desinstalar app 2.0",
        "  1. Apaga a pasta do app",
        "  2. Comandos: a, b",
        "  3. Atalho no Menu Iniciar: App Tool",
        r"  4. PATH do usuário -> remove C:\R\shims",
        "  5. Variável de usuário APP_HOME volta ao valor anterior",
        "  6. Dados mantidos: data (use --purge para apagar)",
    ];
    assert_eq!(t.lines().collect::<Vec<_>>(), want, "{t}");
    let t = utext(&uplan(true, true));
    assert!(t.contains("Apaga os dados: data"), "{t}");
    let t = utext(&uplan(false, false));
    assert!(!t.contains("remove C:"), "{t}");
}

#[test]
fn uninstall_summary_keeps_shared_path_entry() {
    let mut p = uplan(false, true);
    p.path_users = vec![(r"C:\R\shims".into(), vec!["b".into(), "c".into()])];
    let t = utext(&p);
    assert!(
        t.contains(r"PATH do usuário mantém C:\R\shims (ainda usado por: b, c)"),
        "{t}"
    );
    assert!(!t.contains("remove C:"), "{t}");
}

#[test]
fn new_terminal_hint_only_when_folder_missing_from_path() {
    let p = plan(vec![shim("a"), shim("b"), Step::EnsureShimsOnPath]);
    let l = Layout::new(r"C:\R");
    let missing = std::ffi::OsString::from(r"C:\Windows;C:\Tools");
    let hint = output::new_terminal_hint(&p, &l, Some(&missing)).unwrap();
    assert!(
        hint.contains("Abra um terminal novo para usar: a, b"),
        "{hint}"
    );
    let present = std::ffi::OsString::from(r"C:\Windows;c:\r\SHIMS\");
    assert!(output::new_terminal_hint(&p, &l, Some(&present)).is_none());
    assert!(output::new_terminal_hint(&plan(vec![shim("a")]), &l, None).is_none());
}

#[test]
fn new_terminal_hint_without_commands_still_warns_about_path() {
    let p = plan(vec![Step::EnvAddPath {
        entry: PathBuf::from(r"C:\R\apps\app\current\bin"),
    }]);
    let l = Layout::new(r"C:\R");
    let path = std::ffi::OsString::from(r"C:\Windows");
    let hint = output::new_terminal_hint(&p, &l, Some(&path)).unwrap();
    assert!(
        hint.contains("Abra um terminal novo para o PATH novo valer."),
        "{hint}"
    );
}
