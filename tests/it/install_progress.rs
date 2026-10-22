//! Typed install progress phases and live signs (Task 141).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use wingetrs::cache::DataDir;
use wingetrs::cli::output::{PhaseLog, Phases};
use wingetrs::http::Http;
use wingetrs::install::download::Progress;
use wingetrs::install::executor::{ExecCtx, ExtractFn, Msiexec, NoHook};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::msi_queue::Attempt;
use wingetrs::install::msiexec_log::{LogTail, TAIL_WINDOW, read_tail};
use wingetrs::install::ops::{self, OpsCtx, ResidueScan};
use wingetrs::install::planner::{PlanInput, plan_install};
use wingetrs::install::progress::{Phase, ProgressEvent};
use wingetrs::install::state::StateDb;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::tui::theme::{GlyphSet, Glyphs};
use wingetrs::update::LockWait;
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

/// A served `tool` 1.0 zip, its plan, and the download already in the blob cache (as after
/// `ops::prefetch`).
struct Tool {
    _server: MockServer,
    t: tempfile::TempDir,
    layout: Layout,
    data: DataDir,
    http: Http,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    plan: wingetrs::install::plan::InstallPlan,
}

impl Tool {
    async fn new() -> Tool {
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
        Tool {
            _server: server,
            t,
            layout,
            data,
            http,
            env: FakeEnv::new(),
            shortcuts,
            trash: FakeTrash::new(),
            registry: FakeRegistry::new(),
            plan,
        }
    }

    fn effects(&self) -> Effects<'_> {
        Effects {
            env: &self.env,
            shortcuts: &self.shortcuts,
            trash: &self.trash,
            registry: &self.registry,
        }
    }

    /// Runs the plan with `extract`, reporting to `progress`.
    async fn install(&self, progress: Progress<'_>, extract: ExtractFn) {
        let mut state = StateDb::open(&self.layout.state_db()).unwrap();
        let mut ctx = ExecCtx {
            layout: &self.layout,
            data: &self.data,
            http: &self.http,
            effects: self.effects(),
            state: &mut state,
            hook: &NoHook,
            progress,
            in_use: &nothing_in_use,
            msiexec: None,
            launch: Default::default(),
            runners: wingetrs::install::executor::Runners::disabled(),
        };
        wingetrs::install::executor::execute_install_with(&self.plan, &mut ctx, extract)
            .await
            .unwrap();
    }
}

/// `{:?}` of an event (`Phase` debug text), so traces compare as strings.
fn trace_line(ev: &ProgressEvent) -> String {
    match ev {
        ProgressEvent::Download { url, .. } => format!("download {url}"),
        ProgressEvent::Phase(p) => format!("{p:?}"),
    }
}

#[tokio::test]
async fn cached_install_emits_phases_in_order() {
    let tool = Tool::new().await;
    let record = |ev: &ProgressEvent| TRACE.lock().unwrap().push(trace_line(ev));
    tool.install(&record, traced_extract).await;
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

#[tokio::test]
async fn uninstall_emits_removing_then_scanning() {
    let tool = Tool::new().await;
    tool.install(&|_| {}, wingetrs::install::extract::extract)
        .await;
    assert!(tool.layout.app_dir("tool").exists());
    let trace: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let app_dir = tool.layout.app_dir("tool");
    let record = |ev: &ProgressEvent| {
        // Each phase with whether the app folder was still there when it was reported.
        let line = format!("{} dir={}", trace_line(ev), app_dir.exists());
        trace.lock().unwrap().push(line);
    };
    let catalog = wingetrs::catalog::Catalog::open_in_memory().unwrap();
    let ctx = OpsCtx {
        data: &tool.data,
        http: &tool.http,
        catalog: &catalog,
        effects: tool.effects(),
        wait: LockWait::NoWait,
        progress: &record,
        in_use: &nothing_in_use,
        residue: ResidueScan::none(),
        msiexec: Msiexec::DISABLED,
        launch: Default::default(),
        runners: wingetrs::install::executor::Runners::disabled(),
    };
    let plan = ops::plan_uninstall(&ctx, "tool", false).unwrap();
    ops::uninstall(&ctx, &plan).unwrap();
    assert!(!app_dir.exists());
    let removing = Phase::Removing { app: "tool".into() };
    assert_eq!(
        *trace.lock().unwrap(),
        [
            format!("{removing:?} dir=true"),
            format!("{:?} dir=false", Phase::Scanning),
        ]
    );
    drop(tool.t);
}

static ATTEMPTS: AtomicU32 = AtomicU32::new(0);

fn never_busy() -> bool {
    false
}

/// 1618 on the first launch; then an administrative tree with one file.
fn busy_then_done(_: &Path, target: &Path, _: &Path) -> std::io::Result<Attempt> {
    if ATTEMPTS.fetch_add(1, Ordering::SeqCst) == 0 {
        return Ok(Attempt::Busy1618);
    }
    std::fs::create_dir_all(target)?;
    std::fs::write(target.join("tool.txt"), b"tool")?;
    Ok(Attempt::Done)
}

#[test]
fn installer_retry_reports_each_attempt_log() {
    let t = tempfile::tempdir().unwrap();
    let layout = Layout::new(t.path().join("root"));
    std::fs::create_dir_all(layout.root()).unwrap();
    let msi = t.path().join("m-1.0.msi");
    std::fs::write(&msi, vec![0u8; 4096]).unwrap();
    let staging = layout.staging_dir("m", "1.0");
    std::fs::create_dir_all(&staging).unwrap();
    let source = wingetrs::install::extract::ExtractSource {
        file: &msi,
        file_name: "m-1.0.msi",
        extract_dir: None,
        extract_to: None,
    };
    let fake = Msiexec {
        busy: never_busy,
        launch: busy_then_done,
    };
    let phases: Mutex<Vec<Phase>> = Mutex::new(Vec::new());
    let record = |p: Phase| phases.lock().unwrap().push(p);
    // Waits the first retry delay of the policy (5 s).
    wingetrs::install::executor::extract_with_msiexec(
        fake, &layout, "m", &source, &staging, &record,
    )
    .unwrap();
    assert_eq!(std::fs::read(staging.join("tool.txt")).unwrap(), b"tool");
    assert_eq!(
        *phases.lock().unwrap(),
        [
            Phase::Installer {
                log: layout.msiexec_log("m", 1),
            },
            Phase::WaitingInstaller,
            Phase::Installer {
                log: layout.msiexec_log("m", 2),
            },
            Phase::Placing,
        ]
    );
    assert!(layout.msiexec_log("m", 2).ends_with("msiexec-m-2.log"));
}
/// Exits 1603 after writing a log whose end has `Error 1304` (a path past MAX_PATH).
fn fails_on_long_path(_: &Path, _: &Path, log: &Path) -> std::io::Result<Attempt> {
    std::fs::create_dir_all(log.parent().unwrap())?;
    let lines = [
        file_line("ok.txt"),
        r"MSI (s) (A0:B4): Product: Rust -- Error 1304. Error writing to file: C:\x\long.html"
            .to_string(),
        "MSI (s) (A0:B4): Product: Rust -- Installation failed.".to_string(),
    ];
    std::fs::write(log, utf16_log(&lines))?;
    Ok(Attempt::Failed(1603))
}

/// Exits 1603 with a log that says nothing about paths.
fn fails_otherwise(_: &Path, _: &Path, log: &Path) -> std::io::Result<Attempt> {
    std::fs::create_dir_all(log.parent().unwrap())?;
    let lines = ["MSI (s): Product: Rust -- Error 1316.".to_string()];
    std::fs::write(log, utf16_log(&lines))?;
    Ok(Attempt::Failed(1603))
}

#[test]
fn installer_1603_error_names_the_path_limit_only_for_1304() {
    let t = tempfile::tempdir().unwrap();
    let layout = Layout::new(t.path().join("root"));
    std::fs::create_dir_all(layout.root()).unwrap();
    let msi = t.path().join("m-1.0.msi");
    std::fs::write(&msi, vec![0u8; 4096]).unwrap();
    let staging = layout.staging_dir("m", "1.0");
    let source = wingetrs::install::extract::ExtractSource {
        file: &msi,
        file_name: "m-1.0.msi",
        extract_dir: None,
        extract_to: None,
    };
    let run = |launch: fn(&Path, &Path, &Path) -> std::io::Result<Attempt>| {
        let fake = Msiexec {
            busy: never_busy,
            launch,
        };
        wingetrs::install::executor::extract_with_msiexec(
            fake,
            &layout,
            "m",
            &source,
            &staging,
            &|_| {},
        )
        .unwrap_err()
    };
    let hint = "o caminho de algum arquivo passou de 260 caracteres (limite do Windows Installer); use uma raiz mais curta";
    let e = run(fails_on_long_path);
    assert_eq!(
        e,
        format!("msiexec /a: o msiexec terminou com código 1603; {hint}")
    );
    let e = run(fails_otherwise);
    assert_eq!(e, "msiexec /a: o msiexec terminou com código 1603");
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
        (
            Phase::WaitingInstaller,
            "Aguardando o Windows Installer (outro instalador em uso) · 00:02",
        ),
        (
            Phase::Removing { app: "rust".into() },
            "Removendo rust · 00:02",
        ),
        (Phase::Scanning, "Procurando resíduos · 00:02"),
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

    // A zero window, even on an odd size (a half-written unit): nothing read, no overflow.
    let mut odd = log.clone();
    odd.push(b'F');
    for bytes in [log.clone(), odd] {
        let size = bytes.len() as u64;
        let t = read_tail(&mut Cursor::new(bytes.clone()), 0, size).unwrap();
        assert_eq!((t.size, t.last_file, t.new_files), (size, None, 0));
        let text = wingetrs::install::msiexec_log::tail_text(&mut Cursor::new(bytes), 0).unwrap();
        assert_eq!(text, "");
    }
}

#[test]
fn msiexec_1603_with_error_1304_explains_max_path() {
    use wingetrs::install::msiexec_log::{ERROR_WINDOW, failure_hint, tail_text};
    let mut lines: Vec<String> = (0..2000).map(|i| file_line(&format!("f{i}.txt"))).collect();
    lines.push(
        r"MSI (s) (A0:B4) [12:07:00:001]: Product: Rust -- Error 1304. Error writing to file: C:\x\SourceDir\share\doc\rust\html\very\long.html"
            .into(),
    );
    lines.extend((0..50).map(|i| format!("MSI (s) (A0:B4): Rollback: line {i}")));
    lines.push("MSI (s) (A0:B4): Product: Rust -- Installation failed.".into());
    let log = utf16_log(&lines);
    assert!(
        log.len() as u64 > ERROR_WINDOW,
        "the head of the log is never read"
    );
    let text = tail_text(&mut Cursor::new(log.clone()), ERROR_WINDOW).unwrap();
    let hint = "o caminho de algum arquivo passou de 260 caracteres (limite do Windows Installer); use uma raiz mais curta";
    assert_eq!(failure_hint(1603, &text), Some(hint));
    // Another exit code, or 1603 without 1304: nothing added.
    assert_eq!(failure_hint(1618, &text), None);
    let other = utf16_log(&["MSI (s): Product: Rust -- Error 1316.".to_string()]);
    let text = tail_text(&mut Cursor::new(other), ERROR_WINDOW).unwrap();
    assert_eq!(failure_hint(1603, &text), None);
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
    update(
        &mut a,
        Msg::InstallerRunning(PathBuf::from("msiexec-rust-1.log")),
    );
    assert!(a.installer_running);
    a.now = base + Duration::from_secs(3 + 192);
    let text = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(
        text.ends_with(
            "Instalar rust Windows Installer (msiexec /a) · 03:12 · não pode ser cancelado"
        ),
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
// ---- Task 201: every path's first phase, and the notice before every UAC prompt ----

use wingetrs::bridge::elevated::{
    ElevatedBridge, ElevatedError, ElevatedRunner, Helper, REFUSED_NOTE, Transport,
};
use wingetrs::bridge::worker::MachineHive;
use wingetrs::bridge::{BridgeJob, BridgeOp, Tool as BridgeTool};
use wingetrs::install::executor::LaunchGate;
use wingetrs::install::progress::{
    ELEVATION_GRANTED, ELEVATION_NOTICE, ELEVATION_REFUSED, phase_status, phase_title,
};
use wingetrs::installed::correlate::InstallScope;
use wingetrs::winget_install::launch::{LaunchSpec, Launcher, Verb};
use wingetrs::winget_install::msix::{AddResult, MsixError, MsixOps};
use wingetrs::winget_install::ops::{WingetCtx, WingetOpsError};
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan};
use wingetrs::winget_install::select::{Host, Scope, ScopeWanted};
use wingetrs::winget_install::uninstall::{UninstallPlan, UninstallTarget};
use wingetrs::winget_install::uninstall_cmd::Rule;

type Trace = std::sync::Arc<Mutex<Vec<String>>>;

/// Records each launch (with its verb) into the shared trace; the installer "succeeds".
struct TraceLauncher(Trace);

impl Launcher for TraceLauncher {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32> {
        self.0
            .lock()
            .unwrap()
            .push(format!("launch {:?}", spec.verb));
        Ok(0)
    }
}

/// No install flow asks the session's helper anything.
struct NoHelper;
impl ElevatedRunner for NoHelper {}

struct NoMsix;
impl MsixOps for NoMsix {
    fn add(&self, _file: &Path) -> Result<AddResult, MsixError> {
        Ok(AddResult::Installed)
    }
    fn remove(&self, _family: &str) -> Result<usize, MsixError> {
        Ok(0)
    }
}

/// A served winget installer of `kind` and its native plan.
struct Native {
    _server: MockServer,
    t: tempfile::TempDir,
    data: DataDir,
    http: Http,
    plan: WingetPlan,
    trace: Trace,
}

impl Native {
    async fn new(kind: &str, scope: Option<&str>, wanted: Scope, extra: &str) -> Native {
        let body: Vec<u8> = (0..64 * 1024).map(|i| (i * 31 % 251) as u8).collect();
        let sha = hex::encode(Sha256::digest(&body));
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/setup"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;
        let scope = scope.map_or(String::new(), |s| format!("  Scope: {s}\n"));
        let yaml = format!(
            "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Contoso App\nPublisher: Contoso\nInstallers:\n- Architecture: x64\n  InstallerType: {kind}\n{scope}  InstallerUrl: {}/setup\n  InstallerSha256: {sha}\n{extra}",
            server.uri()
        );
        let m = wingetrs::sources::winget::manifest::parse(yaml.as_bytes()).unwrap();
        let host = Host {
            arch: "x64".into(),
            os: [10, 0, 22631, 0],
            locale: "pt-BR".into(),
        };
        let plan = match wingetrs::winget_install::plan::plan(
            &m,
            &host,
            ScopeWanted::Prefer(wanted),
            None,
        )
        .unwrap()
        {
            PlanOrBridge::Native(p) => p,
            PlanOrBridge::Bridge(why) => panic!("bridge: {why}"),
            PlanOrBridge::Portable(s) => panic!("portable: {s:?}"),
        };
        let t = tempfile::tempdir().unwrap();
        let data = DataDir::new(t.path().join("root"));
        Native {
            _server: server,
            t,
            data,
            http: Http::new().unwrap(),
            plan,
            trace: Trace::default(),
        }
    }

    /// Runs the native install with `wait`; phases and launches land in `self.trace`.
    async fn install(&self, wait: LockWait) -> Result<(), WingetOpsError> {
        let launcher = TraceLauncher(std::sync::Arc::clone(&self.trace));
        let trace = std::sync::Arc::clone(&self.trace);
        let progress = move |ev: &ProgressEvent| {
            if let ProgressEvent::Phase(p) = ev {
                trace.lock().unwrap().push(format!("{p:?}"));
            }
        };
        let ctx = WingetCtx {
            data: &self.data,
            http: &self.http,
            wait,
            progress: &progress,
            gate: LaunchGate::default(),
            launcher: &launcher,
            elevated: &NoHelper,
            msix: &NoMsix,
            scan: &Vec::new,
            msiexec: self.t.path().join("System32").join("msiexec.exe"),
            busy: &|| false,
            sleep: &|_| {},
        };
        wingetrs::winget_install::ops::install(&ctx, &self.plan)
            .await
            .map(|_| ())
    }

    fn trace(&self) -> Vec<String> {
        self.trace.lock().unwrap().clone()
    }
}

fn bridge_job(scope: Option<InstallScope>) -> BridgeJob {
    BridgeJob {
        op: BridgeOp::Upgrade,
        tool: BridgeTool::Winget,
        target: "Pub.App".into(),
        name: "Contoso App".into(),
        installed: Some("1.0".into()),
        available: Some("2.0".into()),
        scope,
    }
}

/// A started helper whose channel is empty: enough to see the start.
struct IdleHelper {
    reader: Cursor<Vec<u8>>,
    writer: Vec<u8>,
}

impl Helper for IdleHelper {
    fn reader(&mut self) -> &mut dyn std::io::BufRead {
        &mut self.reader
    }
    fn writer(&mut self) -> &mut dyn Write {
        &mut self.writer
    }
    fn wait_exit(&mut self, _limit: Duration) -> bool {
        true
    }
    fn alive(&mut self) -> bool {
        true
    }
}

/// The UAC prompt of the helper: recorded in the trace; accepted or refused.
struct UacTransport {
    trace: Trace,
    refuse: bool,
}

impl Transport for UacTransport {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        self.trace.lock().unwrap().push("uac".into());
        if self.refuse {
            Err("recusado".into())
        } else {
            Ok(Box::new(IdleHelper {
                reader: Cursor::new(Vec::new()),
                writer: Vec::new(),
            }))
        }
    }
}

/// The trace of `exec_for` of `job` on `bridge`: its lines, after what the transport recorded.
fn exec_for_lines(bridge: &ElevatedBridge, job: &BridgeJob, trace: &Trace) -> bool {
    let mut unused = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| Ok((0, Vec::new()));
    let chosen = bridge.exec_for(job, &mut unused, &mut |l: &str| {
        trace.lock().unwrap().push(l.to_string())
    });
    chosen.elevated()
}

#[tokio::test]
async fn every_path_emits_first_phase_immediately() {
    // Scoop install: `Preparando` before the `install.lock` wait (held: NoWait gives up).
    let tool = Tool::new().await;
    let catalog = wingetrs::catalog::Catalog::open_in_memory().unwrap();
    let events: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let record = |ev: &ProgressEvent| events.lock().unwrap().push(trace_line(ev));
    let ctx = OpsCtx {
        data: &tool.data,
        http: &tool.http,
        catalog: &catalog,
        effects: tool.effects(),
        wait: LockWait::NoWait,
        progress: &record,
        in_use: &nothing_in_use,
        residue: ResidueScan::none(),
        msiexec: Msiexec::DISABLED,
        launch: Default::default(),
        runners: wingetrs::install::executor::Runners::disabled(),
    };
    let held = wingetrs::cache::UpdateLock::try_acquire(&tool.layout.install_lock())
        .unwrap()
        .unwrap();
    let r = ops::apply(&ctx, std::slice::from_ref(&tool.plan)).await;
    assert!(r.is_err(), "the lock is held");
    assert_eq!(
        *events.lock().unwrap(),
        [format!("{:?}", Phase::Preparing)],
        "scoop install"
    );
    drop(held);

    // Scoop uninstall: `Removendo` before the lock wait.
    tool.install(&|_| {}, wingetrs::install::extract::extract)
        .await;
    let plan = ops::plan_uninstall(&ctx, "tool", false).unwrap();
    events.lock().unwrap().clear();
    let held = wingetrs::cache::UpdateLock::try_acquire(&tool.layout.install_lock())
        .unwrap()
        .unwrap();
    assert!(ops::uninstall(&ctx, &plan).is_err(), "the lock is held");
    assert_eq!(
        *events.lock().unwrap(),
        [format!("{:?}", Phase::Removing { app: "tool".into() })],
        "scoop uninstall"
    );
    drop(held);

    // Native winget install (exe and MSI alike; MSIX shares the path): `Preparando` before
    // the `install.lock` wait.
    let n = Native::new("inno", Some("user"), Scope::User, "").await;
    let held = wingetrs::cache::UpdateLock::try_acquire(&n.data.install_layout().install_lock())
        .unwrap()
        .unwrap();
    let r = tokio::time::timeout(Duration::from_secs(30), n.install(LockWait::NoWait))
        .await
        .expect("NoWait never waits");
    assert!(matches!(r, Err(WingetOpsError::Busy)), "{r:?}");
    assert_eq!(
        n.trace(),
        [format!("{:?}", Phase::Preparing)],
        "winget install"
    );
    drop(held);

    // Native removal: `Preparando` while the lock is still awaited.
    let events: Mutex<Vec<ProgressEvent>> = Mutex::new(Vec::new());
    let progress = |ev: &ProgressEvent| events.lock().unwrap().push(ev.clone());
    let gate = LaunchGate::default();
    let steps = wingetrs::winget_install::removal::Steps {
        gate: &gate,
        progress: &progress,
        backups: Path::new("backups"),
        stamp: "1",
        free_space: &|_| None,
    };
    let plan = UninstallPlan {
        target: UninstallTarget::Msix {
            family: "Pub.App_8wekyb3d8bbwe".into(),
        },
        name: "Contoso App".into(),
        version: None,
        command: None,
        elevation: wingetrs::winget_install::launch::Elevation::Never,
        shared: None,
    };
    let removal = wingetrs::winget_install::removal::remove(
        &steps,
        std::future::pending::<Result<(), WingetOpsError>>(),
        &plan,
        wingetrs::winget_install::removal::RemovalChoice::Remove,
        |_| unreachable!("the lock never came"),
        |_| unreachable!("the lock never came"),
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), removal)
            .await
            .is_err()
    );
    assert_eq!(
        *events.lock().unwrap(),
        [ProgressEvent::Phase(Phase::Preparing)],
        "winget removal"
    );

    // Bridge, CLI: the tool's phase before the tool runs.
    let events: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let progress = |ev: &ProgressEvent| events.lock().unwrap().push(trace_line(ev));
    wingetrs::cli::output::bridge_progress(&progress, &|_| {}, "winget", |_| {
        events.lock().unwrap().push("run".into());
    });
    let bridge = Phase::Bridge {
        tool: "winget".into(),
    };
    assert_eq!(
        *events.lock().unwrap(),
        [format!("{bridge:?}"), "run".to_string()],
        "bridge (CLI)"
    );

    // Bridge, TUI: the job's clock runs from its start, before any line.
    use wingetrs::tui::app::{Action, App, Msg, Queued, update};
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    let base = Instant::now();
    a.now = base;
    a.queue.push_back(Queued {
        id: 1,
        label: "Atualizar Contoso App".into(),
        action: Action::Bridge(bridge_job(None)),
    });
    update(&mut a, Msg::RetryQueue);
    a.now = base + Duration::from_secs(2);
    let text = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(
        text.ends_with(&phase_status(&bridge, Duration::from_secs(2))),
        "bridge (TUI): {text}"
    );
    assert!(text.contains("· 00:02 · não pode ser cancelado"), "{text}");

    // Orphan record, TUI: `Preparando` before the `install.lock` wait.
    let n = Native::new("inno", Some("user"), Scope::User, "").await;
    let held = wingetrs::cache::UpdateLock::try_acquire(&n.data.install_layout().install_lock())
        .unwrap()
        .unwrap();
    let r = tokio::time::timeout(Duration::from_secs(30), n.remove_orphan(LockWait::NoWait))
        .await
        .expect("NoWait never waits");
    assert!(
        matches!(r, Err(wingetrs::tui::app::InstallFail::Busy)),
        "{r:?}"
    );
    assert_eq!(
        n.trace(),
        [format!("{:?}", Phase::Preparing)],
        "orphan (TUI)"
    );
    drop(held);
}

struct NoClock;
impl wingetrs::winget_install::uninstall::Clock for NoClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
    fn sleep(&self, _: Duration) {}
}

struct NoArp;
impl wingetrs::winget_install::orphans::ArpRegistry for NoArp {
    fn export(&self, _: &[String]) -> std::io::Result<()> {
        Ok(())
    }
    fn delete_user_key(&self, _: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn still_orphan(&self, _: wingetrs::installed::Hive, _: &str) -> bool {
        true
    }
}

impl Native {
    /// The TUI's job removing an HKLM orphan record, with `wait`; phases land in the trace.
    async fn remove_orphan(
        &self,
        wait: LockWait,
    ) -> Result<wingetrs::tui::app::Done, wingetrs::tui::app::InstallFail> {
        let launcher = TraceLauncher(std::sync::Arc::clone(&self.trace));
        let trace = std::sync::Arc::clone(&self.trace);
        let progress = move |ev: &ProgressEvent| {
            if let ProgressEvent::Phase(p) = ev {
                trace.lock().unwrap().push(format!("{p:?}"));
            }
        };
        let msiexec = self.t.path().join("System32").join("msiexec.exe");
        let env = wingetrs::tui::jobs::WingetEnv {
            install: WingetCtx {
                data: &self.data,
                http: &self.http,
                wait,
                progress: &progress,
                gate: LaunchGate::default(),
                launcher: &launcher,
                elevated: &NoHelper,
                msix: &NoMsix,
                scan: &Vec::new,
                msiexec: msiexec.clone(),
                busy: &|| false,
                sleep: &|_| {},
            },
            uninstall: wingetrs::winget_install::uninstall::UninstallCtx {
                launcher: &launcher,
                elevated: &NoHelper,
                msix: &NoMsix,
                key_exists: &|_, _| false,
                clock: &NoClock,
                user_expand: &|_| None,
                msiexec: &msiexec,
                helper_line: &|_| {},
                bridge: &no_bridge,
            },
            registry: &NoArp,
            backups: self.t.path(),
            free_space: &|_| None,
            previous: &|_, _| Err("sem plano".into()),
            stamp: "1".into(),
        };
        let o = wingetrs::winget_install::orphans::Orphan {
            hive: wingetrs::installed::Hive::MachineX64,
            key: "App".into(),
            name: "Contoso App".into(),
            missing: String::new(),
        };
        wingetrs::tui::jobs::run_winget_action(&env, wingetrs::tui::app::Action::RemoveOrphan(o))
            .await
    }
}

#[tokio::test]
async fn elevation_notice_before_every_uac() {
    // The session's helper (bridge jobs): the notice, then the UAC prompt, then "granted".
    let trace = Trace::default();
    let bridge = ElevatedBridge::new(Box::new(UacTransport {
        trace: std::sync::Arc::clone(&trace),
        refuse: false,
    }));
    let job = bridge_job(Some(InstallScope::Machine));
    assert!(exec_for_lines(&bridge, &job, &trace));
    assert_eq!(
        *trace.lock().unwrap(),
        [ELEVATION_NOTICE, "uac", ELEVATION_GRANTED],
        "helper of a bridge job"
    );
    // The helper runs: no second prompt, no second notice.
    trace.lock().unwrap().clear();
    assert!(exec_for_lines(&bridge, &job, &trace));
    assert!(trace.lock().unwrap().is_empty(), "{trace:?}");

    // The helper of a removal (HKLM uninstaller): the notice before its UAC prompt.
    let trace = Trace::default();
    let bridge = ElevatedBridge::new(Box::new(UacTransport {
        trace: std::sync::Arc::clone(&trace),
        refuse: false,
    }));
    let _lost = bridge.run_uninstaller_lines(
        MachineHive::Hklm64,
        "{K}",
        Rule::Msi,
        false,
        &mut |l: &str| trace.lock().unwrap().push(l.to_string()),
    );
    let seen = trace.lock().unwrap().clone();
    assert_eq!(
        seen[..3],
        [ELEVATION_NOTICE, "uac", ELEVATION_GRANTED],
        "helper of a removal: {seen:?}"
    );

    // The helper deleting an HKLM key (orphan, duplicate record): the notice before its UAC.
    let trace = Trace::default();
    let bridge = ElevatedBridge::new(Box::new(UacTransport {
        trace: std::sync::Arc::clone(&trace),
        refuse: false,
    }));
    let _lost = bridge.delete_arp_key_lines(
        MachineHive::Hklm64,
        "App",
        wingetrs::bridge::worker::DeleteReason::Orphan,
        &mut |l: &str| trace.lock().unwrap().push(l.to_string()),
    );
    let seen = trace.lock().unwrap().clone();
    assert_eq!(
        seen[..3],
        [ELEVATION_NOTICE, "uac", ELEVATION_GRANTED],
        "helper of a key deletion: {seen:?}"
    );

    // Refused: the notice, the prompt, then the refusal (the job runs unelevated).
    let trace = Trace::default();
    let bridge = ElevatedBridge::new(Box::new(UacTransport {
        trace: std::sync::Arc::clone(&trace),
        refuse: true,
    }));
    assert!(!exec_for_lines(&bridge, &job, &trace));
    assert_eq!(
        *trace.lock().unwrap(),
        [ELEVATION_NOTICE, "uac", REFUSED_NOTE]
    );
    assert!(REFUSED_NOTE.starts_with(ELEVATION_REFUSED));
    let refused = bridge
        .run_uninstaller_lines(MachineHive::Hklm64, "{K}", Rule::Msi, false, &mut |_| {})
        .unwrap_err();
    assert_eq!(refused, ElevatedError::Refused);
    assert_eq!(refused.to_string(), ELEVATION_REFUSED);

    // Refused before any prompt opens (wingetrs outside an admin-only folder): no notice,
    // also through the CLI's wrapper.
    struct BeforePrompt;
    impl Transport for BeforePrompt {
        fn start(&self) -> Result<Box<dyn Helper>, String> {
            Err(wingetrs::bridge::helper_folder::refusal_text(Path::new(
                r"C:\Users\u\Downloads",
            )))
        }
        fn start_with_notice(&self, _: &mut dyn FnMut()) -> Result<Box<dyn Helper>, String> {
            self.start()
        }
    }
    let trace = Trace::default();
    let bridge = ElevatedBridge::new(Box::new(
        wingetrs::bridge::helper_folder::NoticeTransport::new(
            Box::new(BeforePrompt),
            Box::new(|_| {}),
        ),
    ));
    assert!(!exec_for_lines(&bridge, &job, &trace));
    let seen = trace.lock().unwrap().clone();
    assert!(!seen.iter().any(|l| l == ELEVATION_NOTICE), "{seen:?}");

    // `runas` per installer: the notice phase before the launch.
    let n = Native::new("inno", Some("machine"), Scope::Machine, "").await;
    n.install(LockWait::Block(Duration::from_secs(30)))
        .await
        .unwrap();
    let trace = n.trace();
    let at = |what: &str| {
        trace
            .iter()
            .position(|l| l == what)
            .unwrap_or_else(|| panic!("{what} missing: {trace:?}"))
    };
    let waiting = format!("{:?}", Phase::WaitingElevation);
    assert!(
        at(&waiting) < at(&format!("launch {:?}", Verb::RunAs)),
        "{trace:?}"
    );
    assert!(phase_title(&Phase::WaitingElevation).starts_with(ELEVATION_NOTICE));

    // An installer that elevates itself (`elevatesSelf`; `elevationRequired` with a user
    // scope): launched unelevated, the notice in its phase before the launch.
    for (scope, wanted, requirement) in [
        (None, Scope::User, "elevatesSelf"),
        (Some("user"), Scope::User, "elevationRequired"),
    ] {
        let extra = format!("  ElevationRequirement: {requirement}\n");
        let n = Native::new("inno", scope, wanted, &extra).await;
        n.install(LockWait::Block(Duration::from_secs(30)))
            .await
            .unwrap();
        let trace = n.trace();
        let phase = Phase::SelfElevating {
            name: "Contoso App".into(),
        };
        let at = |what: &str| {
            trace
                .iter()
                .position(|l| l == what)
                .unwrap_or_else(|| panic!("{what} missing: {trace:?}"))
        };
        assert!(
            at(&format!("{phase:?}")) < at(&format!("launch {:?}", Verb::Open)),
            "{requirement}: {trace:?}"
        );
        assert!(phase_title(&phase).starts_with(ELEVATION_NOTICE));
        assert!(phase_status(&phase, Duration::ZERO).ends_with("não pode ser cancelado"));
    }

    // Neither: no notice.
    let n = Native::new("inno", Some("user"), Scope::User, "").await;
    n.install(LockWait::Block(Duration::from_secs(30)))
        .await
        .unwrap();
    let trace = n.trace();
    assert!(
        !trace.iter().any(|l| l.contains("Elevat")),
        "no prompt, no notice: {trace:?}"
    );
}

#[test]
fn elevation_notice_same_text_cli_tui() {
    use wingetrs::tui::app::{Action, App, Msg, Queued, Running, update};
    let base = Instant::now();

    // CLI: the progress line of the phase.
    let mut ph = Phases::new();
    ph.update_at(&ProgressEvent::Phase(Phase::WaitingElevation), base);
    let cli = ph
        .line_at(0, ascii(), base + Duration::from_secs(4))
        .unwrap();
    assert!(cli.contains(ELEVATION_NOTICE), "{cli}");
    assert!(cli.contains("· 00:04"), "{cli}");

    // TUI: the status bar of a native job.
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.now = base;
    a.running = Some(Running {
        id: 1,
        label: "Instalar Contoso App".into(),
        progress: None,
        action: None,
    });
    update(&mut a, Msg::InstallPhase(Phase::WaitingElevation));
    a.now = base + Duration::from_secs(4);
    let tui = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(tui.contains(ELEVATION_NOTICE), "{tui}");
    assert!(tui.contains("· 00:04"), "{tui}");

    // Bridge, CLI: the helper's notice line becomes the phase (not a printed line); the next
    // line goes back to the tool's phase.
    let shown: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let ph = Mutex::new(Phases::new());
    let progress = |ev: &ProgressEvent| ph.lock().unwrap().update_at(ev, base);
    wingetrs::cli::output::bridge_progress(
        &progress,
        &|l: &str| shown.lock().unwrap().push(l.to_string()),
        "winget",
        |on_line| {
            on_line(ELEVATION_NOTICE);
            let line = ph
                .lock()
                .unwrap()
                .line_at(0, ascii(), base + Duration::from_secs(1))
                .unwrap();
            assert!(line.contains(ELEVATION_NOTICE), "{line}");
            on_line(ELEVATION_GRANTED);
        },
    );
    let line = ph
        .lock()
        .unwrap()
        .line_at(0, ascii(), base + Duration::from_secs(1))
        .unwrap();
    assert!(line.contains("winget rodando"), "{line}");
    assert_eq!(
        *shown.lock().unwrap(),
        [format!("  {ELEVATION_GRANTED}")],
        "the notice is the phase, not a line"
    );

    // Bridge, TUI: the same notice line becomes the status bar's phase, with its clock.
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.now = base;
    a.queue.push_back(Queued {
        id: 1,
        label: "Atualizar Contoso App".into(),
        action: Action::Bridge(bridge_job(Some(InstallScope::Machine))),
    });
    update(&mut a, Msg::RetryQueue);
    update(&mut a, Msg::BridgeLine(ELEVATION_NOTICE.to_string()));
    a.now = base + Duration::from_secs(4);
    let tui = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(tui.contains(ELEVATION_NOTICE), "{tui}");
    assert!(tui.contains("· 00:04"), "{tui}");
    update(&mut a, Msg::BridgeLine(ELEVATION_GRANTED.to_string()));
    let tui = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(!tui.contains(ELEVATION_NOTICE), "{tui}");
    assert!(tui.contains(ELEVATION_GRANTED), "{tui}");

    // A refused prompt reads the same on both sides.
    assert_eq!(
        wingetrs::winget_install::codes::UAC_REFUSED,
        ELEVATION_REFUSED
    );
    assert_eq!(WingetOpsError::UacRefused.to_string(), ELEVATION_REFUSED);
}

/// Task 201: a Scoop hook or package installer cannot be cancelled once started: the CLI line
/// and the TUI status bar say so, and past `NATIVE_QUIET` the CLI warns a window may wait.
#[test]
fn package_scripts_and_installers_are_uncancellable_in_cli_and_tui() {
    use wingetrs::cli::output::NATIVE_QUIET;
    use wingetrs::tui::app::{App, Msg, Running, job_uncancellable, update};
    for phase in [
        Phase::RunningScript {
            hook: "post_install".into(),
        },
        Phase::RunningPackageInstaller {
            file: "setup.exe".into(),
        },
    ] {
        let base = Instant::now();
        let mut ph = Phases::new();
        ph.update_at(&ProgressEvent::Phase(phase.clone()), base);
        let line = ph
            .line_at(0, ascii(), base + Duration::from_secs(5))
            .unwrap();
        assert!(line.ends_with("· 00:05 · não pode ser cancelado"), "{line}");
        let late = base + NATIVE_QUIET + Duration::from_secs(60);
        let line = ph.line_at(0, ascii(), late).unwrap();
        assert!(
            line.contains("pode haver uma janela esperando resposta"),
            "{line}"
        );

        let mut a = App::new(Vec::new(), GlyphSet::Ascii);
        a.now = base;
        a.running = Some(Running {
            id: 1,
            label: "Instalar app".into(),
            progress: None,
            action: None,
        });
        update(&mut a, Msg::InstallPhase(phase.clone()));
        assert!(job_uncancellable(&a), "{phase:?}");
        a.now = base + Duration::from_secs(5);
        let tui = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
        assert!(
            tui.ends_with(&phase_status(&phase, Duration::from_secs(5))),
            "{tui}"
        );
    }
}

/// Task 201: a self-elevating installer keeps the UAC notice for at most
/// `SELF_ELEVATION_WAIT` (its own prompt cannot be seen), then says it may still ask; the
/// same in the CLI and the TUI (injected clock).
#[test]
fn self_elevating_notice_gives_way_after_two_minutes() {
    use wingetrs::install::progress::{SELF_ELEVATION_LATER, SELF_ELEVATION_WAIT};
    use wingetrs::tui::app::{App, Msg, Running, update};
    assert_eq!(SELF_ELEVATION_WAIT, Duration::from_secs(120));
    let phase = Phase::SelfElevating {
        name: "Contoso App".into(),
    };
    let base = Instant::now();
    let mut ph = Phases::new();
    ph.update_at(&ProgressEvent::Phase(phase.clone()), base);
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.now = base;
    a.running = Some(Running {
        id: 1,
        label: "Instalar Contoso App".into(),
        progress: None,
        action: None,
    });
    update(&mut a, Msg::InstallPhase(phase));
    let before = base + SELF_ELEVATION_WAIT - Duration::from_secs(1);
    let after = base + SELF_ELEVATION_WAIT;
    let cli = ph.line_at(0, ascii(), before).unwrap();
    assert!(cli.contains(ELEVATION_NOTICE), "{cli}");
    assert!(cli.contains("· 01:59"), "{cli}");
    a.now = before;
    let tui = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(tui.contains(ELEVATION_NOTICE), "{tui}");
    let cli = ph.line_at(0, ascii(), after).unwrap();
    assert!(!cli.contains(ELEVATION_NOTICE), "{cli}");
    assert!(cli.contains(SELF_ELEVATION_LATER), "{cli}");
    assert!(cli.contains("· 02:00 · não pode ser cancelado"), "{cli}");
    a.now = after;
    let tui = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(!tui.contains(ELEVATION_NOTICE), "{tui}");
    assert!(tui.contains(SELF_ELEVATION_LATER), "{tui}");
}

/// The bridge runner of an `UninstallCtx` whose uninstall never goes through winget.
fn no_bridge(
    _: &str,
    _: &[String],
    _: &mut dyn FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    panic!("winget bridge not expected")
}
// ---- Task 216: one download formatting for the CLI and the TUI, themed CLI line, phases of
// `update` and `list` ----

use wingetrs::cli::output::progress_frame;
use wingetrs::cli::picker::fake;
use wingetrs::cli::style::{self, ProgressStyles, from_theme, with_enabled};
use wingetrs::install::progress::{DownloadMeter, download_parts, download_text, phase_text};
use wingetrs::tui::theme::{ColorDepth, Theme};

fn unicode() -> &'static Glyphs {
    Glyphs::for_set(GlyphSet::Unicode)
}

/// 3.1 MiB per second for 3 s of a 27 MiB download, at `base + 0..=3 s`.
const STEP: u64 = 31 * MIB / 10;

#[test]
fn bar_parts_unicode_and_ascii() {
    let now = Instant::now();
    let mut m = DownloadMeter::default();
    m.record_at(
        "https://example.invalid/rust.msi",
        5 * MIB,
        Some(20 * MIB),
        now,
    );
    let p = download_parts(&m, now, unicode());
    assert_eq!(p.filled, "█████");
    assert_eq!(p.empty, "░".repeat(15));
    assert_eq!(p.tail, "25%  5.0 / 20.0 MiB");
    let p = download_parts(&m, now, ascii());
    assert_eq!(
        download_text("Baixando", &p),
        "Baixando [#####---------------] 25%  5.0 / 20.0 MiB"
    );
}

/// The TUI app with an action running, at `now`.
fn tui_running(now: Instant) -> wingetrs::tui::app::App {
    use wingetrs::tui::app::{App, Running};
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.now = now;
    a.running = Some(Running {
        id: 1,
        label: "Instalar rust".into(),
        progress: None,
        action: None,
    });
    a
}

/// What follows `label ` in `line`.
fn after<'a>(line: &'a str, label: &str) -> &'a str {
    let at = line
        .find(label)
        .unwrap_or_else(|| panic!("{label} in {line}"));
    &line[at + label.len()..]
}

#[test]
fn cli_and_tui_share_download_text() {
    use wingetrs::tui::app::{Msg, update};
    let base = Instant::now();
    let s = |n: u64| base + Duration::from_secs(n);
    let mut ph = Phases::new();
    let mut a = tui_running(base);
    for i in 0..=3 {
        ph.update_at(&dl(i * STEP, Some(27 * MIB)), s(i));
        a.now = s(i);
        update(
            &mut a,
            Msg::InstallProgress {
                label: "rust.msi".into(),
                done: i * STEP,
                total: Some(27 * MIB),
            },
        );
    }
    for (at, end) in [(3, " · 3.1 MiB/s · ~6 s"), (15, " · sem dados há 12 s")] {
        let cli = ph.line_at(0, ascii(), s(at)).unwrap();
        a.now = s(at);
        let tui = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
        let (c, t) = (after(&cli, "Baixando "), after(&tui, "rust.msi "));
        assert_eq!(c, t);
        assert!(c.ends_with(end), "{c}");
        println!("t216 {at}s: {}", fake::plain(cli.as_bytes()));
    }
}

#[test]
fn tui_download_shows_size_speed_and_stall() {
    use wingetrs::tui::app::{Msg, update};
    let base = Instant::now();
    let s = |n: u64| base + Duration::from_secs(n);
    let mut a = tui_running(base);
    for i in 0..=3 {
        a.now = s(i);
        update(
            &mut a,
            Msg::InstallProgress {
                label: "rust.msi".into(),
                done: i * STEP,
                total: Some(27 * MIB),
            },
        );
    }
    let text = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(text.contains("9.3 / 27.0 MiB · 3.1 MiB/s"), "{text}");
    a.now = s(15);
    let text = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(text.ends_with(" · sem dados há 12 s"), "{text}");
    // Unknown total: size and speed, no bar.
    let mut a = tui_running(base);
    for (at, done) in [(0, 0), (2, 4 * MIB)] {
        a.now = s(at);
        update(
            &mut a,
            Msg::InstallProgress {
                label: "rust.msi".into(),
                done,
                total: None,
            },
        );
    }
    let text = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
    assert!(text.ends_with("rust.msi 4.0 MiB · 2.0 MiB/s"), "{text}");
    assert!(!text.contains("bytes"), "{text}");
}

/// A download line at 25% of 20 MiB, and the same line after 12 s without data.
fn download_phases(base: Instant) -> Phases {
    let mut ph = Phases::new();
    ph.update_at(&dl(5 * MIB, Some(20 * MIB)), base);
    ph
}

fn accent_styles() -> ProgressStyles {
    ProgressStyles::of(&Theme::new(true, ColorDepth::Ansi256))
}

#[test]
fn progress_frame_paints_with_theme_tokens() {
    let base = Instant::now();
    let ph = download_phases(base);
    let parts = ph.parts_at(0, unicode(), base).unwrap();
    let (out, width) = with_enabled(true, || progress_frame(&parts, 200, &accent_styles()));
    let spin = unicode().spinner[0];
    assert!(out.contains(&format!("\x1b[38;5;99m{spin}")), "{out:?}");
    let plain = ph.line_at(0, unicode(), base).unwrap();
    assert_eq!(fake::plain(out.as_bytes()), plain);
    assert_eq!(width, plain.chars().count());
    println!("t216 frame: {}", out.replace('\x1b', "<ESC>"));
}

#[test]
fn progress_frame_plain_without_color() {
    let base = Instant::now();
    let ph = download_phases(base);
    let parts = ph.parts_at(0, unicode(), base).unwrap();
    let (out, _) = with_enabled(false, || progress_frame(&parts, 200, &accent_styles()));
    assert!(!out.contains('\x1b'), "{out:?}");
    assert_eq!(out, ph.line_at(0, unicode(), base).unwrap());
}

#[test]
fn progress_frame_cuts_before_painting() {
    let base = Instant::now();
    let ph = download_phases(base);
    let at = base + Duration::from_secs(12);
    let parts = ph.parts_at(0, unicode(), at).unwrap();
    let full = ph.line_at(0, unicode(), at).unwrap();
    assert!(full.ends_with(" · sem dados há 12 s"), "{full}");
    let (out, width) = with_enabled(true, || progress_frame(&parts, 30, &accent_styles()));
    let plain = fake::plain(out.as_bytes());
    assert_eq!(plain.chars().count(), 30, "{plain}");
    assert_eq!(width, 30);
    assert!(full.starts_with(&plain), "{plain} / {full}");
    // Every escape sequence ends (final byte `@`..`~`) before the end of the string.
    let mut rest = out.as_str();
    while let Some(i) = rest.find("\x1b[") {
        rest = &rest[i + 2..];
        let end = rest
            .find(|c: char| ('@'..='~').contains(&c))
            .unwrap_or_else(|| panic!("unterminated sequence in {out:?}"));
        rest = &rest[end + 1..];
    }
}

#[test]
fn from_theme_maps_rgb_indexed_and_none() {
    use ratatui::style::{Color, Modifier, Style};
    let rgb = from_theme(Style::new().fg(Color::Rgb(1, 2, 3)));
    assert_eq!(rgb.get_fg_color(), Some(anstyle::RgbColor(1, 2, 3).into()));
    let idx = from_theme(Style::new().fg(Color::Indexed(99)));
    assert_eq!(idx.get_fg_color(), Some(anstyle::Ansi256Color(99).into()));
    assert_eq!(from_theme(Style::new()), anstyle::Style::new());
    let bold = from_theme(Style::new().add_modifier(Modifier::BOLD));
    assert!(bold.get_effects().contains(anstyle::Effects::BOLD));
    assert_eq!(style::paint_raw(idx, "x"), "\x1b[38;5;99mx\x1b[0m");
}

/// The phases `run_update_with` reports, in order.
type PhaseTrace = std::sync::Arc<Mutex<Vec<Phase>>>;

fn phase_sink(trace: &PhaseTrace) -> impl Fn(&ProgressEvent) + Send + Sync + use<> {
    let trace = std::sync::Arc::clone(trace);
    move |ev| {
        if let ProgressEvent::Phase(p) = ev {
            trace.lock().unwrap().push(p.clone());
        }
    }
}

#[tokio::test]
async fn update_reports_phases_in_order() {
    use wingetrs::update::run_update_with;
    let (s, ep) = crate::update::setup().await;
    // Every source answers, so a second run without `force` finds them all up to date.
    Mock::given(method("GET"))
        .and(path("/extras.tgz"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(include_bytes!("../fixtures/scoop/Main-mini.tar.gz").to_vec())
                .insert_header("ETag", "\"e1\""),
        )
        .with_priority(1)
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let http = Http::new().unwrap();
    let trace = PhaseTrace::default();
    let sink = phase_sink(&trace);
    run_update_with(&data, &ep, &http, true, LockWait::NoWait, &sink)
        .await
        .unwrap();
    assert_eq!(
        *trace.lock().unwrap(),
        [
            Phase::Preparing,
            Phase::FetchingCatalog { sources: 3 },
            Phase::WritingCatalog
        ]
    );
    trace.lock().unwrap().clear();
    run_update_with(&data, &ep, &http, false, LockWait::NoWait, &sink)
        .await
        .unwrap();
    assert_eq!(
        *trace.lock().unwrap(),
        [Phase::Preparing, Phase::FetchingCatalog { sources: 0 }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn update_waits_for_lock_with_phase() {
    use wingetrs::update::run_update_with;
    let (_s, ep) = crate::update::setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let held = wingetrs::cache::UpdateLock::try_acquire(&data.lock_path())
        .unwrap()
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Phase>();
    let job = tokio::spawn(async move {
        let sink = move |ev: &ProgressEvent| {
            if let ProgressEvent::Phase(p) = ev {
                let _ = tx.send(p.clone());
            }
        };
        let http = Http::new().unwrap();
        run_update_with(
            &data,
            &ep,
            &http,
            true,
            LockWait::Block(Duration::from_secs(600)),
            &sink,
        )
        .await
        .map(|_| ())
    });
    let mut next = async || {
        tokio::time::timeout(Duration::from_secs(60), rx.recv())
            .await
            .expect("a phase in time")
    };
    assert_eq!(next().await, Some(Phase::Preparing));
    assert_eq!(next().await, Some(Phase::WaitingUpdate));
    drop(held);
    tokio::time::timeout(Duration::from_secs(60), job)
        .await
        .expect("the update goes on once the lock is free")
        .unwrap()
        .unwrap();
}

#[test]
fn new_phase_titles() {
    use wingetrs::install::progress::phase_status;
    let two = Duration::from_secs(2);
    for (phase, text) in [
        (
            Phase::WaitingUpdate,
            "Aguardando outra atualização do catálogo · 00:02",
        ),
        (
            Phase::FetchingCatalog { sources: 1 },
            "Baixando o catálogo · 00:02",
        ),
        (
            Phase::FetchingCatalog { sources: 3 },
            "Baixando os catálogos (3 fontes) · 00:02",
        ),
        (Phase::WritingCatalog, "Gravando o catálogo · 00:02"),
        (Phase::ReadingInstalled, "Lendo os apps instalados · 00:02"),
        (Phase::Correlating, "Comparando com o catálogo · 00:02"),
    ] {
        assert_eq!(phase_text(&phase, two), text);
        assert_eq!(phase_status(&phase, two), text);
    }
}
