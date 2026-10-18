#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 147).
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::executor::{ExecCtx, ExecError, NoHook, Runners, ToolRun};
use wingetrs::install::extract::ExtractError;
use wingetrs::install::inno::{check_exit, child_env, count_files, decode_cp850, inno_args};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::plan::{InstallPlan, Step};
use wingetrs::install::planner::{PlanInput, plan_install};
use wingetrs::install::state::StateDb;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn paths() -> (TempDir, PathBuf, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let inst = t.path().join("setup.exe");
    let dest = t.path().join("out");
    (t, inst, dest)
}

#[test]
fn args_without_extract_dir() {
    let (_t, inst, dest) = paths();
    assert_eq!(
        inno_args(&inst, &dest, None).unwrap(),
        [
            "-x".to_string(),
            format!("-d{}", dest.display()),
            inst.display().to_string(),
            "-y".to_string(),
            "-c{app}".to_string(),
        ]
    );
}

#[test]
fn args_with_subdir() {
    let (_t, inst, dest) = paths();
    let a = inno_args(&inst, &dest, Some("xkb/types")).unwrap();
    assert_eq!(a.last().unwrap(), "-c{app}\\xkb\\types");
}

#[test]
fn args_with_brace_dir() {
    let (_t, inst, dest) = paths();
    let a = inno_args(&inst, &dest, Some("{tmp}")).unwrap();
    assert_eq!(a.last().unwrap(), "-c{tmp}");
    let a = inno_args(&inst, &dest, Some("{tmp}/sub")).unwrap();
    assert_eq!(a.last().unwrap(), "-c{tmp}\\sub");
}

#[test]
fn args_unsafe_dir_refused() {
    let (_t, inst, dest) = paths();
    assert!(matches!(
        inno_args(&inst, &dest, Some("..\\x")),
        Err(ExtractError::UnsafePath(_))
    ));
    // A `{name}` is accepted only as the first component, and `..` after it is refused too.
    for bad in [
        "a/{tmp}",
        "{tmp}/../x",
        "{a\\b}",
        "{}",
        "{tmp}x",
        "{a b}",
        "{..}",
    ] {
        assert!(
            matches!(
                inno_args(&inst, &dest, Some(bad)),
                Err(ExtractError::UnsafePath(_))
            ),
            "{bad}"
        );
    }
}

#[test]
fn cp850_decodes_pt_br() {
    assert_eq!(decode_cp850(&[0x6E, 0xC6, 0x6F]), "não");
    assert_eq!(decode_cp850(&[0x87]), "ç");
    assert_eq!(decode_cp850(&[0x82]), "é");
    assert_eq!(decode_cp850(&[0xA0]), "á");
    assert_eq!(decode_cp850(&[0x90]), "É");
    assert_eq!(decode_cp850(b"abc"), "abc");
}

#[test]
fn cp850_table_edges() {
    assert_eq!(decode_cp850(&[0x80, 0xFF, 0xB0, 0xF0]), "Ç\u{A0}░\u{AD}");
    // Every high byte decodes to exactly one char.
    let all: Vec<u8> = (0x80..=0xFF).collect();
    assert_eq!(decode_cp850(&all).chars().count(), 128);
}

#[test]
fn exit_zero_with_zero_files_is_missing_extract_dir() {
    assert!(matches!(
        check_exit(0, 0, Some("fonts"), &[]),
        Err(ExtractError::MissingExtractDir(d)) if d == "fonts"
    ));
    assert!(matches!(
        check_exit(0, 0, None, &[]),
        Err(ExtractError::MissingExtractDir(d)) if d == "{app}"
    ));
    assert!(check_exit(0, 3, None, &[]).is_ok());
}

#[test]
fn exit_codes_2_and_3() {
    let e = check_exit(2, 0, None, &[]).unwrap_err().to_string();
    assert!(e.contains("Inno Setup"), "{e}");
    let lines = ["Error:".to_string(), "Não foi possível".to_string()];
    let e = check_exit(3, 0, None, &lines).unwrap_err().to_string();
    assert!(e.ends_with("Não foi possível"), "{e}");
    let e = check_exit(7, 5, None, &lines).unwrap_err().to_string();
    assert!(
        e.contains("código 7") && e.ends_with("Não foi possível"),
        "{e}"
    );
}

#[test]
fn count_files_is_recursive() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("a").join("b")).unwrap();
    std::fs::write(t.path().join("x"), b"").unwrap();
    std::fs::write(t.path().join("a").join("b").join("y"), b"").unwrap();
    assert_eq!(count_files(t.path()).unwrap(), 2);
}

#[cfg(windows)]
#[test]
fn count_files_does_not_follow_junctions() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("z"), b"").unwrap();
    let root = t.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    wingetrs::install::link::create(&outside, &root.join("j")).unwrap();
    assert_eq!(count_files(&root).unwrap(), 0);
}

#[test]
fn child_env_is_a_fixed_path_only() {
    let env = child_env(
        Path::new(r"C:\Windows\System32"),
        Path::new(r"C:\W\apps\innounp\current\innounp.exe"),
    );
    assert_eq!(
        env,
        [(
            OsString::from("PATH"),
            OsString::from(r"C:\Windows\System32;C:\W\apps\innounp\current")
        )]
    );
}

fn plan_for(manifest: &str, layout: &Layout) -> InstallPlan {
    let m = resolve(manifest, Arch::X64).unwrap();
    plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &m,
        layout,
        shortcut_root: Path::new(r"C:\SM\wingetrs Apps"),
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
    })
    .unwrap()
}

fn extract_steps(plan: &InstallPlan) -> Vec<Step> {
    plan.steps
        .iter()
        .filter(|s| matches!(s, Step::Extract { .. } | Step::ExtractInno { .. }))
        .cloned()
        .collect()
}

#[test]
fn plan_uses_extract_inno_only_for_exe() {
    let l = Layout::new(r"C:\W");
    let m = |inno: bool| {
        serde_json::json!({
            "version": "1",
            "url": ["https://x/a.exe", "https://x/b.zip"],
            "hash": ["aa", "bb"],
            "innosetup": inno,
        })
        .to_string()
    };
    assert_eq!(
        extract_steps(&plan_for(&m(true), &l)),
        [Step::ExtractInno { index: 0 }, Step::Extract { index: 1 }]
    );
    assert_eq!(
        extract_steps(&plan_for(&m(false), &l)),
        [Step::Extract { index: 0 }, Step::Extract { index: 1 }]
    );
    // The extension is matched case-insensitively.
    let upper = serde_json::json!({
        "version": "1", "url": "https://x/A.EXE", "hash": "aa", "innosetup": true,
    })
    .to_string();
    assert_eq!(
        extract_steps(&plan_for(&upper, &l)),
        [Step::ExtractInno { index: 0 }]
    );
}

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

struct World {
    _server: MockServer,
    _t: TempDir,
    layout: Layout,
    data: DataDir,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    state_path: PathBuf,
    http: Http,
    plan: InstallPlan,
}

/// Serves a fake installer and plans `app` with `innosetup: true`.
async fn setup(extract_dir: Option<&str>) -> World {
    let server = MockServer::start().await;
    let t = tempfile::tempdir().unwrap();
    let layout = Layout::new(t.path().join("root"));
    std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
    std::fs::write(layout.launcher(), b"MZ").unwrap();
    let body = b"not really an installer".to_vec();
    let sha = hex::encode(Sha256::digest(&body));
    Mock::given(method("GET"))
        .and(path("/Xming-setup.exe"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&server)
        .await;
    let mut m = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/Xming-setup.exe", server.uri()),
        "hash": sha,
        "innosetup": true,
    });
    if let Some(d) = extract_dir {
        m["extract_dir"] = serde_json::json!(d);
    }
    let plan = plan_for(&m.to_string(), &layout);
    let state_path = t.path().join("state").join("state.db");
    StateDb::open(&state_path).unwrap();
    World {
        data: DataDir::new(layout.root()),
        shortcuts: FakeShortcuts::new(t.path().join("startmenu")),
        state_path,
        _server: server,
        _t: t,
        layout,
        env: FakeEnv::new(),
        trash: FakeTrash::new(),
        registry: FakeRegistry::new(),
        http: Http::new().unwrap(),
        plan,
    }
}

impl World {
    fn put_innounp(&self) {
        let dir = self.layout.current("innounp");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("innounp.exe"), b"").unwrap();
    }

    async fn install(&self, innounp: ToolRun) -> Result<(), ExecError> {
        let state: &mut StateDb = Box::leak(Box::new(StateDb::open(&self.state_path).unwrap()));
        let mut ctx = ExecCtx {
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
            hook: &NoHook,
            progress: &no_progress,
            in_use: &nothing_in_use,
            msiexec: None,
            launch: Default::default(),
            runners: Runners { innounp },
        };
        wingetrs::install::executor::execute_install(&self.plan, &mut ctx).await
    }
}

/// The `-d<dest>` argument of an innounp call.
fn dest_of(args: &[String]) -> PathBuf {
    PathBuf::from(args.iter().find_map(|a| a.strip_prefix("-d")).unwrap())
}

fn message(e: &ExecError) -> String {
    match e {
        ExecError::Failed { message, .. } => message.clone(),
        other => panic!("{other:?}"),
    }
}

static PLACES_CALLS: AtomicUsize = AtomicUsize::new(0);
static PLACES_SAW_APP: AtomicBool = AtomicBool::new(false);
fn places_files(exe: &Path, args: &[String], _dir: &Path) -> io::Result<(i32, Vec<String>)> {
    PLACES_CALLS.fetch_add(1, Ordering::SeqCst);
    assert!(exe.ends_with(r"innounp\current\innounp.exe"), "{exe:?}");
    if args.iter().any(|a| a == "-c{app}") {
        PLACES_SAW_APP.store(true, Ordering::SeqCst);
    }
    let d = dest_of(args);
    std::fs::create_dir_all(d.join("locale"))?;
    std::fs::write(d.join("Xming.exe"), b"MZ")?;
    std::fs::write(d.join("locale").join("x.txt"), b"x")?;
    Ok((0, vec!["#1 Xming.exe".into()]))
}

#[tokio::test]
async fn install_with_fake_innounp_places_files() {
    let w = setup(None).await;
    w.put_innounp();
    w.install(places_files).await.unwrap();
    let v = w.layout.version_dir("app", "1.0");
    assert!(v.join("Xming.exe").is_file());
    assert!(v.join("locale").join("x.txt").is_file());
    assert!(!v.join(".inno-0").exists());
    assert_eq!(PLACES_CALLS.load(Ordering::SeqCst), 1);
    assert!(PLACES_SAW_APP.load(Ordering::SeqCst));
}

static EMPTY_CALLS: AtomicUsize = AtomicUsize::new(0);
fn exits_zero_empty(_: &Path, args: &[String], _dir: &Path) -> io::Result<(i32, Vec<String>)> {
    EMPTY_CALLS.fetch_add(1, Ordering::SeqCst);
    assert!(dest_of(args).is_dir());
    Ok((0, Vec::new()))
}

#[tokio::test]
async fn fake_exit_zero_no_files_rolls_back() {
    let w = setup(Some("fonts")).await;
    w.put_innounp();
    let e = w.install(exits_zero_empty).await.unwrap_err();
    let m = message(&e);
    assert!(
        m.contains(&ExtractError::MissingExtractDir("fonts".into()).to_string()),
        "{m}"
    );
    assert_eq!(EMPTY_CALLS.load(Ordering::SeqCst), 1);
    assert!(!w.layout.app_dir("app").exists());
}

static FAIL3_CALLS: AtomicUsize = AtomicUsize::new(0);
fn exits_three_after_writing(
    _: &Path,
    args: &[String],
    _dir: &Path,
) -> io::Result<(i32, Vec<String>)> {
    FAIL3_CALLS.fetch_add(1, Ordering::SeqCst);
    std::fs::write(dest_of(args).join("partial.bin"), b"x")?;
    Ok((3, vec!["Não foi possível abrir".into()]))
}

#[tokio::test]
async fn nonzero_exit_rolls_back_partial_output() {
    let w = setup(None).await;
    w.put_innounp();
    let e = w.install(exits_three_after_writing).await.unwrap_err();
    let m = message(&e);
    assert!(m.ends_with("Não foi possível abrir"), "{m}");
    assert_eq!(FAIL3_CALLS.load(Ordering::SeqCst), 1);
    assert!(!w.layout.app_dir("app").exists());
}

static EXTRACT_TO_CALLS: AtomicUsize = AtomicUsize::new(0);
fn places_for_extract_to(_: &Path, args: &[String], _dir: &Path) -> io::Result<(i32, Vec<String>)> {
    EXTRACT_TO_CALLS.fetch_add(1, Ordering::SeqCst);
    std::fs::write(dest_of(args).join("a.txt"), b"a")?;
    Ok((0, Vec::new()))
}

#[tokio::test]
async fn extract_to_is_honored() {
    let server = MockServer::start().await;
    let mut w = setup(None).await;
    let body = b"inst".to_vec();
    let sha = hex::encode(Sha256::digest(&body));
    Mock::given(method("GET"))
        .and(path("/i.exe"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&server)
        .await;
    let m = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/i.exe", server.uri()),
        "hash": sha,
        "innosetup": true,
        "extract_to": "sub/dir",
    });
    w.plan = plan_for(&m.to_string(), &w.layout);
    w.put_innounp();
    w.install(places_for_extract_to).await.unwrap();
    let v = w.layout.version_dir("app", "1.0");
    assert!(v.join("sub").join("dir").join("a.txt").is_file());
    assert_eq!(EXTRACT_TO_CALLS.load(Ordering::SeqCst), 1);
}

static MISSING_CALLS: AtomicUsize = AtomicUsize::new(0);
fn counts_missing(_: &Path, _: &[String], _: &Path) -> io::Result<(i32, Vec<String>)> {
    MISSING_CALLS.fetch_add(1, Ordering::SeqCst);
    Ok((0, Vec::new()))
}

#[tokio::test]
async fn missing_innounp_fails_without_calling_runner() {
    let w = setup(None).await;
    let e = w.install(counts_missing).await.unwrap_err();
    let m = message(&e);
    assert!(m.contains("innounp não está instalado"), "{m}");
    assert_eq!(MISSING_CALLS.load(Ordering::SeqCst), 0);
    assert!(!w.layout.app_dir("app").exists());
}

#[test]
fn disabled_runner_refuses() {
    let r = Runners::disabled();
    let e = (r.innounp)(Path::new(r"C:\x.exe"), &[], Path::new(r"C:\")).unwrap_err();
    assert!(e.to_string().contains("innounp desativado"), "{e}");
}

/// A `.cmd` stand-in for innounp: the child sees only the fixed PATH, and the exit code and
/// CP850 output come back.
#[cfg(windows)]
#[test]
#[ignore = "live: roda cmd.exe com um innounp falso (.cmd)"]
fn fake_cmd_runner_sees_fixed_path() {
    let t = tempfile::Builder::new().prefix("t147-").tempdir().unwrap();
    let tool = t.path().join("tool");
    std::fs::create_dir_all(&tool).unwrap();
    let exe = tool.join("innounp.cmd");
    let mut script = b"@echo off\r\necho PATH=%PATH%\r\necho n".to_vec();
    script.push(0xC6);
    script.extend_from_slice(b"o\r\nexit /b 3\r\n");
    std::fs::write(&exe, script).unwrap();
    let (code, lines) = wingetrs::install::inno_win::run(&exe, &["-x".into()], t.path()).unwrap();
    assert_eq!(code, 3);
    let path_line = lines.iter().find(|l| l.starts_with("PATH=")).unwrap();
    let want = format!(";{}", tool.display());
    assert!(path_line.ends_with(&want), "{path_line}");
    assert_eq!(path_line.matches(';').count(), 1, "{path_line}");
    assert_eq!(lines.last().unwrap(), "não");
}

/// The real innounp over a real Inno Setup installer (the xming of probe D4): only with the
/// user's authorization. `WINGETRS_LIVE_INNOUNP` = innounp.exe, `WINGETRS_LIVE_INNO_SETUP` =
/// the installer.
#[cfg(windows)]
#[test]
#[ignore = "live: innounp sobre instalador real"]
fn real_innounp_over_real_installer() {
    let exe = PathBuf::from(std::env::var_os("WINGETRS_LIVE_INNOUNP").unwrap());
    let inst = PathBuf::from(std::env::var_os("WINGETRS_LIVE_INNO_SETUP").unwrap());
    let t = tempfile::Builder::new().prefix("t147-").tempdir().unwrap();
    let dest = t.path().join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let args = inno_args(&inst, &dest, None).unwrap();
    let (code, lines) = wingetrs::install::inno_win::run(&exe, &args, t.path()).unwrap();
    let files = count_files(&dest).unwrap();
    check_exit(code, files, None, &lines).unwrap();
    assert!(files > 0);
    // A filter that matches nothing is still exit code 0, with zero files (probe D4).
    let empty = t.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let args = inno_args(&inst, &empty, Some("{tmp}")).unwrap();
    let (code, lines) = wingetrs::install::inno_win::run(&exe, &args, t.path()).unwrap();
    let files = count_files(&empty).unwrap();
    assert!(matches!(
        check_exit(code, files, Some("{tmp}"), &lines),
        Err(ExtractError::MissingExtractDir(_)) | Ok(())
    ));
}

static INTO_TMP_CALLS: AtomicUsize = AtomicUsize::new(0);
fn counts_into_tmp(_: &Path, _: &[String], _: &Path) -> io::Result<(i32, Vec<String>)> {
    INTO_TMP_CALLS.fetch_add(1, Ordering::SeqCst);
    Ok((0, Vec::new()))
}

#[tokio::test]
async fn extract_to_into_the_temp_folder_is_refused() {
    let mut w = setup(None).await;
    let url = w.plan.steps.iter().find_map(|s| match s {
        Step::Download { source, .. } => Some(source.url.clone()),
        _ => None,
    });
    let sha = w.plan.steps.iter().find_map(|s| match s {
        Step::Download { source, .. } => Some(source.hash.hex.clone()),
        _ => None,
    });
    let m = serde_json::json!({
        "version": "1.0", "url": url.unwrap(), "hash": sha.unwrap(),
        "innosetup": true, "extract_to": ".INNO-0/x",
    });
    w.plan = plan_for(&m.to_string(), &w.layout);
    w.put_innounp();
    let e = w.install(counts_into_tmp).await.unwrap_err();
    let m = message(&e);
    assert!(m.contains("caminho inseguro"), "{m}");
    assert_eq!(INTO_TMP_CALLS.load(Ordering::SeqCst), 0);
    assert!(!w.layout.app_dir("app").exists());
}

static LINK_CALLS: AtomicUsize = AtomicUsize::new(0);
fn writes_a_junction(_: &Path, args: &[String], _: &Path) -> io::Result<(i32, Vec<String>)> {
    LINK_CALLS.fetch_add(1, Ordering::SeqCst);
    let d = dest_of(args);
    std::fs::write(d.join("a.txt"), b"a")?;
    // Outside the install: the junction must not be imported (nor followed).
    // `<root>\apps\app\.staging-1.0\.inno-0` -> `<root>\t147-outside`.
    let outside = d.ancestors().nth(4).unwrap().join("t147-outside");
    std::fs::create_dir_all(&outside)?;
    std::fs::write(outside.join("keep.txt"), b"k")?;
    std::fs::create_dir_all(d.join("sub"))?;
    wingetrs::install::link::create(&outside, &d.join("sub").join("j"))?;
    Ok((0, Vec::new()))
}

#[cfg(windows)]
#[tokio::test]
async fn link_in_extracted_tree_is_refused() {
    let w = setup(None).await;
    w.put_innounp();
    let e = w.install(writes_a_junction).await.unwrap_err();
    let m = message(&e);
    assert!(m.contains("caminho inseguro"), "{m}");
    assert_eq!(LINK_CALLS.load(Ordering::SeqCst), 1);
    assert!(!w.layout.app_dir("app").exists());
    let outside = w.layout.apps().parent().unwrap().join("t147-outside");
    assert!(outside.join("keep.txt").is_file());
}
