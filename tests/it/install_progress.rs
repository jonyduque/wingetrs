//! Typed install progress phases and live signs (Task 141).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use wingetrs::cache::DataDir;
use wingetrs::cli::output::{PhaseLog, Phases};
use wingetrs::http::Http;
use wingetrs::install::executor::{ExecCtx, NoHook};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::msiexec_log::{LogTail, TAIL_WINDOW, read_tail};
use wingetrs::install::planner::{PlanInput, plan_install};
use wingetrs::install::progress::{Phase, ProgressEvent};
use wingetrs::install::state::StateDb;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::tui::theme::{GlyphSet, Glyphs};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MIB: u64 = 1024 * 1024;

fn ascii() -> &'static Glyphs {
    Glyphs::for_set(GlyphSet::Ascii)
}

fn fake_pe() -> Vec<u8> {
    let mut b = vec![0u8; 0x80 + 24 + 96];
    b[0] = b'M';
    b[1] = b'Z';
    b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    let off = 0x80 + 24 + 68;
    b[off..off + 2].copy_from_slice(&3u16.to_le_bytes());
    b
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            z.start_file(*name, o).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

/// What the executor did, in order: phases, downloads, and the extraction itself.
static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn traced_extract(
    file: &Path,
    name: &str,
    dir: Option<&str>,
    to: Option<&str>,
    staging: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    TRACE.lock().unwrap().push("extract".into());
    wingetrs::install::extract::extract(file, name, dir, to, staging)
}

#[tokio::test]
async fn cached_install_emits_phases_in_order() {
    let server = MockServer::start().await;
    let zip = zip_of(&[("pkg/tool.exe", &fake_pe())]);
    let sha = hex::encode(Sha256::digest(&zip));
    Mock::given(method("GET"))
        .and(path("/tool-1.0.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
        .mount(&server)
        .await;
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("root");
    let layout = Layout::new(&root);
    std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
    std::fs::write(layout.launcher(), fake_pe()).unwrap();
    let data = DataDir::new(&root);
    let http = Http::new().unwrap();
    let manifest = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/tool-1.0.zip", server.uri()),
        "hash": sha,
        "extract_dir": "pkg",
        "bin": "tool.exe",
    })
    .to_string();
    let m = resolve(&manifest, Arch::X64).unwrap();
    let shortcuts = FakeShortcuts::new(t.path().join("startmenu"));
    let shortcut_root = {
        use wingetrs::install::traits::Shortcuts;
        shortcuts.root()
    };
    let plan = plan_install(&PlanInput {
        app: "tool",
        bucket: "main",
        manifest: &m,
        layout: &layout,
        shortcut_root: &shortcut_root,
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
    })
    .unwrap();
    // The download is already in the blob cache (as after `ops::prefetch`).
    let sources: Vec<_> = plan
        .steps
        .iter()
        .filter_map(|s| match s {
            wingetrs::install::plan::Step::Download { source, .. } => Some(source.clone()),
            _ => None,
        })
        .collect();
    wingetrs::install::download::fetch_all(&http, &data, &sources, &|_| {})
        .await
        .unwrap();

    let record = |ev: &ProgressEvent| {
        let line = match ev {
            ProgressEvent::Download { url, .. } => format!("download {url}"),
            ProgressEvent::Phase(p) => format!("{p:?}"),
        };
        TRACE.lock().unwrap().push(line);
    };
    let env = FakeEnv::new();
    let (trash, registry) = (FakeTrash::new(), FakeRegistry::new());
    let mut state = StateDb::open(&t.path().join("state.db")).unwrap();
    let mut ctx = ExecCtx {
        layout: &layout,
        data: &data,
        http: &http,
        effects: Effects {
            env: &env,
            shortcuts: &shortcuts,
            trash: &trash,
            registry: &registry,
        },
        state: &mut state,
        hook: &NoHook,
        progress: &record,
        in_use: &nothing_in_use,
        msiexec: None,
    };
    wingetrs::install::executor::execute_install_with(&plan, &mut ctx, traced_extract)
        .await
        .unwrap();
    let trace = TRACE.lock().unwrap().clone();
    let extracting = format!(
        "{:?}",
        Phase::Extracting {
            file: "tool-1.0.zip".into(),
            engine: "ZIP".into(),
        }
    );
    assert_eq!(
        trace,
        vec![
            extracting,
            "extract".to_string(),
            format!("{:?}", Phase::Placing),
            format!("{:?}", Phase::Linking),
            format!("{:?}", Phase::Recording),
        ],
        "{trace:#?}"
    );
}

#[test]
fn phase_line_has_spinner_and_clock() {
    let base = Instant::now();
    let mut ph = Phases::new();
    let extracting = ProgressEvent::Phase(Phase::Extracting {
        file: "rust-1.90.0-x86_64-pc-windows-msvc.msi".into(),
        engine: "MSI".into(),
    });
    ph.update_at(&extracting, base);
    let at4 = base + Duration::from_secs(4);
    let l0 = ph.line_at(0, ascii(), at4).unwrap();
    assert_eq!(
        l0,
        format!(
            "{} Extraindo rust-1.90.0-x86_64-pc-windows-msvc.msi (MSI) · 00:04",
            ascii().spinner[0]
        )
    );
    // The spinner moves with the frame; the same phase again keeps its clock.
    let l1 = ph.line_at(1, ascii(), at4).unwrap();
    assert!(l1.starts_with(ascii().spinner[1]), "{l1}");
    ph.update_at(&extracting, at4);
    let l = ph
        .line_at(0, ascii(), base + Duration::from_secs(65))
        .unwrap();
    assert!(l.ends_with("· 01:05"), "{l}");
    // Every phase has its text and clock.
    for (phase, text) in [
        (Phase::Preparing, "Preparando · 00:02"),
        (Phase::Placing, "Colocando os arquivos · 00:02"),
        (Phase::Linking, "Criando comandos · 00:02"),
        (Phase::Recording, "Registrando · 00:02"),
    ] {
        let mut ph = Phases::new();
        ph.update_at(&ProgressEvent::Phase(phase), base);
        let l = ph
            .line_at(0, ascii(), base + Duration::from_secs(2))
            .unwrap();
        assert!(l.ends_with(text), "{l}");
    }
}

fn dl(done: u64, total: Option<u64>) -> ProgressEvent {
    ProgressEvent::Download {
        url: "https://example.invalid/rust.msi".into(),
        done,
        total,
    }
}

#[test]
fn download_line_speed_eta_and_stall() {
    let base = Instant::now();
    let s = |n: u64| base + Duration::from_secs(n);
    let mut ph = Phases::new();
    let step = 31 * MIB / 10;
    for i in 0..=3 {
        ph.update_at(&dl(i * step, Some(27 * MIB)), s(i));
    }
    let l = ph.line_at(0, ascii(), s(3)).unwrap();
    assert!(l.contains("Baixando [######--------------] 34%"), "{l}");
    assert!(l.contains("9.3 / 27.0 MiB · 3.1 MiB/s · ~6 s"), "{l}");
    assert!(!l.contains("sem dados"), "{l}");
    // Nothing new for 12 s: the speed fades and the line says so.
    let l = ph.line_at(0, ascii(), s(15)).unwrap();
    assert!(l.ends_with(" · sem dados há 12 s"), "{l}");
    assert!(!l.contains("MiB/s"), "{l}");
    // 9 s is not a stall yet.
    assert!(!ph.line_at(0, ascii(), s(12)).unwrap().contains("sem dados"));
    // Unknown total: speed without estimate.
    let mut ph = Phases::new();
    ph.update_at(&dl(0, None), s(0));
    ph.update_at(&dl(4 * MIB, None), s(2));
    let l = ph.line_at(0, ascii(), s(2)).unwrap();
    assert!(l.ends_with("Baixando 4.0 MiB · 2.0 MiB/s"), "{l}");
}

/// A verbose msiexec log: UTF-16LE with BOM, `lines` joined by CRLF.
fn utf16_log(lines: &[String]) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for l in lines {
        for u in format!("{l}\r\n").encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out
}

fn file_line(rel: &str) -> String {
    format!(
        "MSI (s) (A0:B4) [12:00:01:123]: File: C:\\x\\SourceDir\\{rel};\tTo be installed;\tWon't patch;\tNo existing file"
    )
}

#[test]
fn msiexec_log_tail_reads_last_file() {
    let mut lines = vec!["=== Verbose logging started ===".to_string()];
    lines.extend((0..49).map(|i| file_line(&format!("share\\doc\\f{i}.html"))));
    let before_last = utf16_log(&lines).len() as u64;
    lines.push(file_line("a\\b.txt"));
    lines.push("MSI (s) (A0:B4) [12:00:01:124]: Note: 1: 2205 2:  3: Patch".into());
    let log = utf16_log(&lines);
    let size = log.len() as u64;
    assert_eq!(size % 2, 0);

    let t = read_tail(&mut Cursor::new(log.clone()), TAIL_WINDOW * 4, 0).unwrap();
    assert_eq!(t.size, size);
    assert_eq!(t.last_file.as_deref(), Some("a\\b.txt"));
    assert_eq!((t.new_files, t.estimated), (50, false));
    // Since the previous look, only the last line is new.
    let t = read_tail(&mut Cursor::new(log.clone()), TAIL_WINDOW * 4, before_last).unwrap();
    assert_eq!((t.new_files, t.estimated), (1, false));

    // The window starts at an odd offset (mid-character): still decodes.
    let window = 601;
    assert_eq!((size - window) % 2, 1);
    let t = read_tail(&mut Cursor::new(log.clone()), window, 0).unwrap();
    assert_eq!(t.last_file.as_deref(), Some("a\\b.txt"));
    // More was written than the window holds: the count is an estimate.
    assert!(t.estimated && t.new_files > 1, "{t:?}");

    // A unit half written at the end (msiexec is still writing).
    let mut partial = log.clone();
    partial.push(b'F');
    let t = read_tail(&mut Cursor::new(partial), window, 0).unwrap();
    assert_eq!(t.last_file.as_deref(), Some("a\\b.txt"));

    // No file line yet.
    let empty = utf16_log(&["=== Verbose logging started ===".to_string()]);
    let t = read_tail(&mut Cursor::new(empty), TAIL_WINDOW, 0).unwrap();
    assert_eq!((t.last_file, t.new_files), (None, 0));
}

fn installer_at(base: Instant) -> Phases {
    let mut ph = Phases::new();
    let ev = ProgressEvent::Phase(Phase::Installer {
        log: PathBuf::from("m.log"),
    });
    ph.update_at(&ev, base);
    ph
}

fn tail(size: u64, new_files: u64, last: &str) -> Option<LogTail> {
    Some(LogTail {
        size,
        last_file: Some(last.into()),
        new_files,
        estimated: false,
    })
}

#[test]
fn installer_idle_warning_after_60s() {
    let base = Instant::now();
    let s = |n: u64| base + Duration::from_secs(n);
    let log = Path::new("m.log");
    let mut ph = installer_at(base);
    // Before the log is read: phase, clock and "not cancellable".
    let l = ph.line_at(0, ascii(), s(0)).unwrap();
    assert!(l.ends_with("Windows Installer (msiexec /a) · 00:00 · não pode ser cancelado"));
    ph.log_read_at(log, tail(4000, 18_304, "share\\doc\\x.html"), s(1));
    let l = ph.line_at(0, ascii(), s(30)).unwrap();
    assert!(
        l.ends_with(
            "Windows Installer (msiexec /a) · 00:30 · 18.304 arquivos · share\\doc\\x.html"
        ),
        "{l}"
    );
    // Size unchanged for 61 s.
    ph.log_read_at(log, tail(4000, 0, "share\\doc\\x.html"), s(62));
    let l = ph.line_at(0, ascii(), s(62)).unwrap();
    assert!(
        l.ends_with(
            " · sem atividade há 1 min; o Windows Installer pode estar esperando outro instalador"
        ),
        "{l}"
    );
    // Growth clears it.
    ph.log_read_at(log, tail(6000, 2, "z.txt"), s(63));
    let l = ph.line_at(0, ascii(), s(63)).unwrap();
    assert!(
        !l.contains("sem atividade") && l.contains("18.306 arquivos"),
        "{l}"
    );
    // A read error leaves only the phase and the clock.
    ph.log_read_at(log, None, s(64));
    let l = ph.line_at(0, ascii(), s(200)).unwrap();
    assert!(
        !l.contains("sem atividade") && !l.contains("arquivos"),
        "{l}"
    );
    // A look at another (older) log is ignored.
    ph.log_read_at(Path::new("old.log"), tail(1, 5, "old"), s(201));
    assert!(!ph.line_at(0, ascii(), s(201)).unwrap().contains("old"));
}

#[test]
fn installer_long_path_is_cut_from_the_left() {
    let base = Instant::now();
    let mut ph = installer_at(base);
    let long = format!(
        "share\\doc\\rust\\html\\{}\\constant._MM_FROUND_FLOOR.html",
        "x".repeat(120)
    );
    ph.log_read_at(Path::new("m.log"), tail(10, 3, &long), base);
    let l = ph.line_at(0, ascii(), base).unwrap();
    assert_eq!(l.chars().count(), 100, "{l}");
    assert!(
        l.contains("· …x") && l.ends_with("constant._MM_FROUND_FLOOR.html"),
        "{l}"
    );
}

#[test]
fn installer_notice_is_shown_once_after_30s() {
    let base = Instant::now();
    let s = |n: u64| base + Duration::from_secs(n);
    let mut ph = installer_at(base);
    assert_eq!(ph.take_notice_at(s(29)), None);
    assert_eq!(
        ph.take_notice_at(s(30)),
        Some(
            "O Windows Installer pode levar vários minutos em pacotes grandes; não feche o terminal."
        )
    );
    assert_eq!(ph.take_notice_at(s(31)), None);
    // Not for other phases.
    let mut other = Phases::new();
    other.update_at(&ProgressEvent::Phase(Phase::Placing), base);
    assert_eq!(other.take_notice_at(s(40)), None);
}

#[test]
fn not_a_tty_prints_one_line_per_phase() {
    let mut log = PhaseLog::new();
    let events = [
        ProgressEvent::Phase(Phase::Preparing),
        dl(1, Some(10)),
        dl(5, Some(10)),
        ProgressEvent::Phase(Phase::Extracting {
            file: "x.msi".into(),
            engine: "MSI".into(),
        }),
        ProgressEvent::Phase(Phase::Installer {
            log: PathBuf::from("m.log"),
        }),
        ProgressEvent::Phase(Phase::Placing),
        ProgressEvent::Phase(Phase::Placing),
        ProgressEvent::Phase(Phase::Linking),
        ProgressEvent::Phase(Phase::Recording),
    ];
    let lines: Vec<String> = events.iter().filter_map(|e| log.line(e)).collect();
    assert_eq!(
        lines,
        [
            "Preparando…",
            "Baixando…",
            "Extraindo x.msi (MSI)…",
            "Windows Installer (msiexec /a)…",
            "Colocando os arquivos…",
            "Criando comandos…",
            "Registrando…",
        ]
    );
    assert!(
        lines
            .iter()
            .all(|l| !l.contains('\r') && !l.contains('\x1b'))
    );
}

#[test]
fn tui_status_shows_phase_and_clock() {
    use wingetrs::tui::app::{App, Msg, Running, update};
    let base = Instant::now();
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.now = base;
    a.running = Some(Running {
        id: 1,
        label: "Instalar rust".into(),
        progress: None,
        action: None,
    });
    update(&mut a, Msg::InstallPhase(Phase::Preparing));
    a.now = base + Duration::from_secs(3);
    let text = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(text.ends_with("Instalar rust Preparando · 00:03"), "{text}");
    update(&mut a, Msg::InstallerRunning);
    assert!(a.installer_running);
    a.now = base + Duration::from_secs(3 + 192);
    let text = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(
        text.ends_with("Instalar rust Windows Installer (msiexec /a) · 03:12"),
        "{text}"
    );
    assert!(!text.contains('\x1b'), "{text}");
    // The same phase again keeps its clock.
    update(&mut a, Msg::InstallPhase(Phase::Placing));
    a.now += Duration::from_secs(5);
    update(&mut a, Msg::InstallPhase(Phase::Placing));
    a.now += Duration::from_secs(5);
    let text = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(text.ends_with("Colocando os arquivos · 00:10"), "{text}");
}
