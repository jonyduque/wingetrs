#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 151).
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::executor::{
    ExecCtx, ExecError, NoHook, Runners, StepHook, ToolRun, execute_install, execute_uninstall,
};
use wingetrs::install::installer_args::installer_args;
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::plan::{InstallPlan, Step, UninstallPlan};
use wingetrs::install::planner::{PlanError, PlanInput, plan_install};
use wingetrs::install::state::StateDb;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{Effects, EnvStore, EnvValue, Shortcuts};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fake_pe(subsystem: u16) -> Vec<u8> {
    let mut b = vec![0u8; 0x80 + 24 + 96];
    b[0] = b'M';
    b[1] = b'Z';
    b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    let off = 0x80 + 24 + 68;
    b[off..off + 2].copy_from_slice(&subsystem.to_le_bytes());
    b
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
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

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

struct World {
    server: MockServer,
    _t: TempDir,
    layout: Layout,
    data: DataDir,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    state: StateDb,
    state_path: PathBuf,
    http: Http,
}

async fn setup() -> World {
    let server = MockServer::start().await;
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("root");
    let layout = Layout::new(&root);
    std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
    std::fs::write(layout.launcher(), fake_pe(3)).unwrap();
    let env = FakeEnv::new();
    env.set(
        "Path",
        &EnvValue {
            value: r"C:\Existing".into(),
            expand: true,
        },
    )
    .unwrap();
    let state_path = t.path().join("state").join("state.db");
    World {
        server,
        data: DataDir::new(&root),
        layout,
        env,
        shortcuts: FakeShortcuts::new(t.path().join("startmenu")),
        trash: FakeTrash::new(),
        registry: FakeRegistry::new(),
        state: StateDb::open(&state_path).unwrap(),
        state_path,
        http: Http::new().unwrap(),
        _t: t,
    }
}

impl World {
    /// Serves a zip at `/pkg.zip`; returns the manifest JSON with `extra` merged in.
    async fn manifest(&self, extra: serde_json::Value) -> String {
        let zip = zip_of(&[
            ("pkg/tool.exe", &fake_pe(3)),
            ("pkg/setup.exe", &fake_pe(3)),
            ("pkg/unins.exe", &fake_pe(3)),
        ]);
        let sha = sha256_hex(&zip);
        Mock::given(method("GET"))
            .and(path("/pkg.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
            .mount(&self.server)
            .await;
        let mut m = serde_json::json!({
            "version": "1.0",
            "url": format!("{}/pkg.zip", self.server.uri()),
            "hash": sha,
            "extract_dir": "pkg",
            "bin": "tool.exe",
        });
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        m.to_string()
    }

    fn plan(&self, manifest: &str) -> InstallPlan {
        let m = resolve(manifest, Arch::X64).unwrap();
        let sha = sha256_hex(manifest.as_bytes());
        plan_install(&PlanInput {
            app: "app",
            bucket: "main",
            manifest: &m,
            layout: &self.layout,
            shortcut_root: &self.shortcuts.root(),
            installed: None,
            launcher_present: true,
            keep_old: false,
            manifest_sha256: &sha,
            ast: &wingetrs::install::script_ast::LexerOnly,
        })
        .unwrap()
    }

    fn ctx(&self, installer: Option<ToolRun>) -> ExecCtx<'_> {
        self.ctx_hook(installer, &NoHook)
    }

    fn ctx_hook<'a>(&'a self, installer: Option<ToolRun>, hook: &'a dyn StepHook) -> ExecCtx<'a> {
        let state: &mut StateDb = Box::leak(Box::new(StateDb::open(&self.state_path).unwrap()));
        ExecCtx {
            layout: &self.layout,
            data: &self.data,
            http: &self.http,
            effects: Effects {
                env: &self.env,
                shortcuts: &self.shortcuts,
                trash: &self.trash,
                registry: &self.registry,
            },
            state,
            hook,
            progress: &no_progress,
            in_use: &nothing_in_use,
            msiexec: None,
            launch: Default::default(),
            runners: Runners {
                ast: std::sync::Arc::new(wingetrs::install::script_ast::LexerOnly),
                installer,
                ..Runners::disabled()
            },
        }
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn args_substitute_only_dir_global_version() {
    let vdir = Path::new(r"C:\r\apps\app\1.0");
    let got = installer_args(
        &strs(&["/D=$dir", "$global", "$version", "$persist_dir", "$app"]),
        vdir,
        "1.0",
    )
    .unwrap();
    assert_eq!(
        got,
        strs(&[
            r"/D=C:\r\apps\app\1.0",
            "False",
            "1.0",
            "$persist_dir",
            "$app"
        ])
    );
    // case-sensitive, like String.Replace
    let got = installer_args(&strs(&["$DIR"]), vdir, "1.0").unwrap();
    assert_eq!(got, strs(&["$DIR"]));
}

#[test]
fn args_split_like_scoop() {
    let d = Path::new("x");
    assert_eq!(
        installer_args(&strs(&[r"/S /D=C:\x y"]), d, "1").unwrap(),
        strs(&["/S", r"/D=C:\x y"])
    );
    assert_eq!(
        installer_args(&strs(&["\"q\""]), d, "1").unwrap(),
        strs(&["q"])
    );
    assert_eq!(
        installer_args(&strs(&["C:/Program Files/x"]), d, "1").unwrap(),
        strs(&["C:/Program Files/x"])
    );
}

#[test]
fn args_with_control_characters_are_refused() {
    let r = installer_args(&strs(&["/S\n"]), Path::new("x"), "1");
    assert!(matches!(r, Err(PlanError::UnsafeArgs(_))), "{r:?}");
}

fn plan_for(manifest: &serde_json::Value) -> Result<InstallPlan, PlanError> {
    let t = tempfile::tempdir().unwrap();
    let layout = Layout::new(t.path().join("root"));
    let m = resolve(&manifest.to_string(), Arch::X64).unwrap();
    plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &m,
        layout: &layout,
        shortcut_root: t.path(),
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
        ast: &wingetrs::install::script_ast::LexerOnly,
    })
}

fn exe_manifest(installer: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "version": "1.0",
        "url": "https://example.invalid/setup.exe",
        "hash": "0".repeat(64),
        "installer": installer,
    })
}

#[test]
fn file_defaults_to_download_name() {
    let plan = plan_for(&exe_manifest(serde_json::json!({"args": ["/S"]}))).unwrap();
    let promote = plan
        .steps
        .iter()
        .position(|s| matches!(s, Step::Promote))
        .unwrap();
    assert!(
        matches!(&plan.steps[promote + 1], Step::RunInstaller { file, .. } if file == "setup.exe"),
        "{:?}",
        plan.steps[promote + 1]
    );
}

#[test]
fn msi_file_refused() {
    let r = plan_for(&exe_manifest(serde_json::json!({"file": "x.MSI"})));
    assert!(
        matches!(&r, Err(PlanError::Unsupported(m)) if m.contains(".msi")),
        "{r:?}"
    );
}

#[test]
fn ps1_plans_a_ps1_step_and_a_ps1_uninstaller() {
    // `installer.file` `.ps1` runs through the script host since Task 158.
    let plan = plan_for(&exe_manifest(serde_json::json!({"file": "x.ps1"}))).unwrap();
    assert!(
        plan.steps
            .iter()
            .any(|s| matches!(s, Step::RunPs1Installer { file, .. } if file == "x.ps1"))
    );
    // `installer.script` runs through the script host since Task 156.
    assert!(plan_for(&exe_manifest(serde_json::json!({"script": "a"}))).is_ok());
    // A `.ps1` uninstaller is planned since Task 159 (it runs at removal time).
    let mut m = exe_manifest(serde_json::json!({"file": "x.ps1"}));
    m["uninstaller"] = m["installer"].take();
    assert!(plan_for(&m).is_ok());
}

#[test]
fn unsafe_default_file_or_empty_file_is_unsafe_path() {
    // `file` defaults to the download name, here taken from the URL fragment.
    let mut m = exe_manifest(serde_json::json!({"args": ["/S"]}));
    m["url"] = "https://example.invalid/a.zip#/..\\x.exe".into();
    let r = plan_for(&m);
    assert!(
        matches!(r, Err(PlanError::UnsafePath(_))),
        "fragment: {r:?}"
    );
    let r = plan_for(&exe_manifest(serde_json::json!({"file": ""})));
    assert!(matches!(r, Err(PlanError::UnsafePath(_))), "empty: {r:?}");
}

#[test]
fn unsafe_installer_file_is_a_manifest_error() {
    let m = exe_manifest(serde_json::json!({"file": "..\\x.exe"}));
    assert!(resolve(&m.to_string(), Arch::X64).is_err());
}

static SEEN: Mutex<Vec<(PathBuf, Vec<String>, PathBuf)>> = Mutex::new(Vec::new());

fn fake_ok(exe: &Path, args: &[String], dir: &Path) -> std::io::Result<(i32, Vec<String>)> {
    assert!(exe.is_file(), "the installer exists while it runs");
    SEEN.lock()
        .unwrap()
        .push((exe.to_path_buf(), args.to_vec(), dir.to_path_buf()));
    Ok((0, Vec::new()))
}

#[tokio::test]
async fn runs_with_consent_and_deletes_file() {
    for keep in [false, true] {
        let s = setup().await;
        let m = s
            .manifest(serde_json::json!({"installer": {
                "file": "setup.exe",
                "args": ["/D=$dir", "$global", "$version", "$persist_dir", "$app"],
                "keep": keep,
            }}))
            .await;
        let plan = s.plan(&m);
        SEEN.lock().unwrap().clear();
        let mut ctx = s.ctx(Some(fake_ok));
        execute_install(&plan, &mut ctx).await.unwrap();
        let vdir = s.layout.version_dir("app", "1.0");
        let seen = SEEN.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].2, vdir, "cwd is the version folder");
        assert_eq!(seen[0].0, vdir.join("setup.exe"));
        assert_eq!(
            seen[0].1,
            strs(&[
                &format!("/D={}", vdir.display()),
                "False",
                "1.0",
                "$persist_dir",
                "$app"
            ])
        );
        assert_eq!(vdir.join("setup.exe").exists(), keep, "keep={keep}");
        assert!(vdir.join("tool.exe").exists());
    }
}

fn fake_exit3(_: &Path, _: &[String], _: &Path) -> std::io::Result<(i32, Vec<String>)> {
    Ok((3, vec!["boom".into()]))
}

#[tokio::test]
async fn exit_3_rolls_back_with_warning() {
    let s = setup().await;
    let m = s
        .manifest(serde_json::json!({"installer": {"file": "setup.exe"}}))
        .await;
    let plan = s.plan(&m);
    let mut ctx = s.ctx(Some(fake_exit3));
    let e = execute_install(&plan, &mut ctx).await.unwrap_err();
    let text = e.to_string();
    assert!(text.contains("código 3"), "{text}");
    assert!(text.contains("não foi desfeito"), "{text}");
    assert!(!s.layout.app_dir("app").exists(), "apps\\app gone");
    assert!(s.state.get("app").unwrap().is_none());
}

#[tokio::test]
async fn no_consent_fails_before_anything() {
    let s = setup().await;
    let m = s
        .manifest(serde_json::json!({"installer": {"file": "setup.exe"}}))
        .await;
    let plan = s.plan(&m);
    let mut ctx = s.ctx(None);
    let e = execute_install(&plan, &mut ctx).await.unwrap_err();
    assert!(matches!(e, ExecError::NeedsScriptConsent { .. }), "{e}");
    // The flag does not exist yet: the text must not point at it.
    assert!(!e.to_string().contains("--allow-scripts"), "{e}");
    assert!(e.to_string().contains("precisa da sua confirmação"), "{e}");
    assert_eq!(s.server.received_requests().await.unwrap().len(), 0);
    assert!(!s.layout.app_dir("app").exists());
}

#[tokio::test]
async fn installer_outside_version_dir_refused() {
    let s = setup().await;
    let m = s.manifest(serde_json::json!({})).await;
    let plan = s.plan(&m);
    let promote = plan
        .steps
        .iter()
        .position(|s| matches!(s, Step::Promote))
        .unwrap();
    for file in ["..\\x.exe", "nope.exe"] {
        let mut p = plan.clone();
        p.steps.insert(
            promote + 1,
            Step::RunInstaller {
                file: file.into(),
                args: Vec::new(),
                keep: true,
            },
        );
        // Had it run, the fake would answer "código 3": the message tells which check refused.
        let mut ctx = s.ctx(Some(fake_exit3));
        let e = execute_install(&p, &mut ctx).await.unwrap_err();
        let text = e.to_string();
        assert!(matches!(e, ExecError::Failed { .. }), "{file}: {e}");
        assert!(
            text.contains("inseguro") || text.contains("não existe"),
            "{file}: {text}"
        );
        assert!(!s.layout.app_dir("app").exists());
    }
}

/// Creates the junction `<vdir>\sub` -> `target` right before the installer step.
struct JunctionBefore {
    link: PathBuf,
    target: PathBuf,
}
impl StepHook for JunctionBefore {
    fn before(&self, _: usize, step: &Step) -> Result<(), ExecError> {
        if matches!(step, Step::RunInstaller { .. }) {
            wingetrs::install::link::create(&self.target, &self.link)
                .map_err(|e| ExecError::Hook(e.to_string()))?;
        }
        Ok(())
    }
}

#[tokio::test]
async fn installer_through_junction_out_of_version_dir_refused() {
    let s = setup().await;
    let m = s
        .manifest(serde_json::json!({"installer": {"file": "sub\\x.exe"}}))
        .await;
    let plan = s.plan(&m);
    let outside = s._t.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("x.exe"), fake_pe(3)).unwrap();
    let hook = JunctionBefore {
        link: s.layout.version_dir("app", "1.0").join("sub"),
        target: outside.clone(),
    };
    let mut ctx = s.ctx_hook(Some(fake_exit3), &hook);
    let e = execute_install(&plan, &mut ctx).await.unwrap_err();
    assert!(e.to_string().contains("fora da pasta da versão"), "{e}");
    assert!(
        outside.join("x.exe").exists(),
        "the outside file is untouched"
    );
    assert!(!s.layout.app_dir("app").exists());
}

#[tokio::test]
async fn uninstaller_alone_needs_consent_to_install() {
    let s = setup().await;
    let m = s
        .manifest(serde_json::json!({"uninstaller": {"file": "unins.exe"}}))
        .await;
    let plan = s.plan(&m);
    assert!(plan.third_party_uninstaller);
    assert!(
        !plan
            .steps
            .iter()
            .any(|s| matches!(s, Step::RunInstaller { .. }))
    );
    let mut ctx = s.ctx(None);
    let e = execute_install(&plan, &mut ctx).await.unwrap_err();
    assert!(matches!(e, ExecError::NeedsScriptConsent { .. }), "{e}");
    assert_eq!(s.server.received_requests().await.unwrap().len(), 0);
    assert!(!s.layout.app_dir("app").exists());
}

fn fake_zero(_: &Path, _: &[String], _: &Path) -> std::io::Result<(i32, Vec<String>)> {
    Ok((0, Vec::new()))
}

static MARKER: AtomicUsize = AtomicUsize::new(0);
fn fake_marker(_: &Path, _: &[String], _: &Path) -> std::io::Result<(i32, Vec<String>)> {
    MARKER.fetch_add(1, Ordering::SeqCst);
    Ok((0, Vec::new()))
}

const SKIPPED: &str = "o desinstalador do app não rodou: ";
const LEFT_BEHIND: &str = "o que ele instalou fora da pasta pode ter ficado";
/// Installs an app with an uninstaller, applies `on_plan` to the plan and `after_install` to
/// the stored manifest (`m` is the original text), removes the app and returns the notes.
async fn removal_notes(
    on_plan: fn(&mut InstallPlan),
    after_install: fn(&Path, &str),
) -> Vec<String> {
    let s = setup().await;
    let m = s
        .manifest(serde_json::json!({"uninstaller": {"file": "unins.exe"}}))
        .await;
    let mut plan = s.plan(&m);
    on_plan(&mut plan);
    let mut ctx = s.ctx(Some(fake_zero));
    execute_install(&plan, &mut ctx).await.unwrap();
    drop(ctx);
    after_install(
        &s.layout.version_dir("app", "1.0").join("manifest.json"),
        &m,
    );
    let up = UninstallPlan::from_state(&s.state, "app", false)
        .unwrap()
        .unwrap();
    let before = MARKER.load(Ordering::SeqCst);
    let mut ctx = s.ctx(Some(fake_marker));
    let notes = execute_uninstall(&up, &mut ctx).unwrap();
    assert_eq!(MARKER.load(Ordering::SeqCst), before, "no uninstaller ran");
    assert!(!s.layout.app_dir("app").exists(), "removal went on");
    notes
}

#[tokio::test]
async fn skipped_uninstaller_note_when_manifest_is_gone() {
    let notes = removal_notes(|_| {}, |p, _| std::fs::remove_file(p).unwrap()).await;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains(SKIPPED) && notes[0].contains("não existe"),
        "{notes:?}"
    );
    assert!(notes[0].ends_with(LEFT_BEHIND), "{notes:?}");
}

#[tokio::test]
async fn skipped_uninstaller_note_when_manifest_was_altered() {
    let notes = removal_notes(
        |_| {},
        |p, m| std::fs::write(p, m.replace("unins.exe", "evil.exe")).unwrap(),
    )
    .await;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains(SKIPPED) && notes[0].contains("manifesto gravado foi alterado"),
        "{notes:?}"
    );
    assert!(notes[0].ends_with(LEFT_BEHIND), "{notes:?}");
}

#[tokio::test]
async fn skipped_uninstaller_note_when_manifest_does_not_resolve() {
    let notes = removal_notes(
        |plan| {
            // A written manifest whose hash matches the record but that `resolve` refuses.
            plan.manifest_sha256 = sha256_hex(b"{}");
            for st in &mut plan.steps {
                if let Step::WriteManifest { json } = st {
                    *json = "{}".to_string();
                }
            }
        },
        |_, _| {},
    )
    .await;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains(SKIPPED) && notes[0].contains("não pôde ser lido"),
        "{notes:?}"
    );
    assert!(notes[0].ends_with(LEFT_BEHIND), "{notes:?}");
}
static UN_CODE: AtomicUsize = AtomicUsize::new(1);
static UN_SEEN: Mutex<Vec<(PathBuf, Vec<String>)>> = Mutex::new(Vec::new());

fn fake_uninstaller(exe: &Path, args: &[String], _: &Path) -> std::io::Result<(i32, Vec<String>)> {
    if exe.ends_with("unins.exe") {
        UN_SEEN
            .lock()
            .unwrap()
            .push((exe.to_path_buf(), args.to_vec()));
        return Ok((UN_CODE.load(Ordering::SeqCst) as i32, Vec::new()));
    }
    Ok((0, Vec::new()))
}

#[tokio::test]
async fn uninstaller_runs_before_removal() {
    let s = setup().await;
    let m = s
        .manifest(serde_json::json!({
            "installer": {"file": "setup.exe"},
            "uninstaller": {"file": "unins.exe", "args": ["/x", "$dir"], "keep": true},
        }))
        .await;
    let plan = s.plan(&m);
    let mut ctx = s.ctx(Some(fake_uninstaller));
    execute_install(&plan, &mut ctx).await.unwrap();
    let vdir = s.layout.version_dir("app", "1.0");
    let up = UninstallPlan::from_state(&s.state, "app", false)
        .unwrap()
        .unwrap();

    // no consent: nothing runs, nothing is removed
    let mut ctx = s.ctx(None);
    let e = execute_uninstall(&up, &mut ctx).unwrap_err();
    assert!(matches!(e, ExecError::NeedsScriptConsent { .. }), "{e}");
    assert!(vdir.join("tool.exe").exists());

    UN_CODE.store(1, Ordering::SeqCst);
    let mut ctx = s.ctx(Some(fake_uninstaller));
    let e = execute_uninstall(&up, &mut ctx).unwrap_err();
    assert!(e.to_string().contains("código 1"), "{e}");
    assert!(vdir.join("tool.exe").exists(), "app stays");
    assert!(s.layout.shims().join("tool.exe").exists(), "shim stays");
    assert!(s.state.get("app").unwrap().is_some());

    UN_CODE.store(0, Ordering::SeqCst);
    UN_SEEN.lock().unwrap().clear();
    let mut ctx = s.ctx(Some(fake_uninstaller));
    execute_uninstall(&up, &mut ctx).unwrap();
    assert!(!s.layout.app_dir("app").exists());
    assert!(s.state.get("app").unwrap().is_none());
    let seen = UN_SEEN.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].1, strs(&["/x", &vdir.display().to_string()]));
}
/// `installer_win::run` with a fake `.cmd`: exit code, cwd, the fixed PATH and the arguments.
#[cfg(windows)]
#[test]
fn installer_win_runs_a_cmd_with_fixed_path_cwd_and_args() {
    if crate::non_elevated::skip_if_elevated_in_ci("recusa instalador com o processo elevado") {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("app dir");
    let work = t.path().join("work");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let exe = app.join("fake.cmd");
    std::fs::write(
        &exe,
        "@echo off\r\necho cwd=%CD%\r\necho path=%PATH%\r\necho a1=%1\r\necho a2=%2\r\nexit /b 3\r\n",
    )
    .unwrap();
    let (code, lines) =
        wingetrs::install::installer_win::run(&exe, &strs(&["/S", "x y"]), &work).unwrap();
    assert_eq!(code, 3, "{lines:?}");
    let find = |p: &str| {
        lines
            .iter()
            .find_map(|l| l.trim().strip_prefix(p).map(str::to_string))
            .unwrap_or_else(|| panic!("no {p} line in {lines:?}"))
    };
    assert!(find("cwd=").eq_ignore_ascii_case(&work.display().to_string()));
    let root = std::env::var("SystemRoot").unwrap();
    let want = format!(r"{root}\System32;{}", app.display());
    assert!(find("path=").eq_ignore_ascii_case(&want), "{lines:?}");
    assert_eq!(find("a1="), "/S");
    assert_eq!(find("a2="), "\"x y\"");
}

/// Task 191: `installer.file`/`uninstaller.file` never run with an elevated token.
#[cfg(windows)]
#[test]
fn file_installer_refused_when_elevated() {
    static LAUNCHES: AtomicUsize = AtomicUsize::new(0);
    fn fake_launch(_: &Path, _: &[String], _: &Path) -> std::io::Result<(i32, Vec<String>)> {
        LAUNCHES.fetch_add(1, Ordering::SeqCst);
        Ok((0, Vec::new()))
    }
    let t = tempfile::tempdir().unwrap();
    let exe = t.path().join("setup.exe");
    let run = |elevated: fn() -> std::io::Result<bool>| {
        wingetrs::install::installer_win::run_with(&exe, &[], t.path(), elevated, fake_launch)
    };
    let e = run(|| Ok(true)).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(
        e.to_string(),
        "o wingetrs não roda scripts de manifesto com o processo elevado"
    );
    assert_eq!(LAUNCHES.load(Ordering::SeqCst), 0);
    // An error reading the token is not taken for "not elevated".
    assert!(run(|| Err(std::io::Error::other("token"))).is_err());
    assert_eq!(LAUNCHES.load(Ordering::SeqCst), 0);
    assert_eq!(run(|| Ok(false)).unwrap().0, 0);
    assert_eq!(LAUNCHES.load(Ordering::SeqCst), 1);
}
