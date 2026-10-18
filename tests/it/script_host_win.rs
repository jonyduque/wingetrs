//! Phase 2c tests (Task 160).
//!
//! `PwshHost` runs `fake_pwsh.cmd`, written per test into a temp folder: it records its
//! arguments, `set SCOOP`, `set PSModulePath`, `set PATH` and `cd` into a file and exits with
//! the code asked. Nothing here runs PowerShell nor reads the real policy keys.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::executor::{
    ExecCtx, ExecError, LaunchGate, NoHook, Runners, ScriptHost, ScriptJob, StepHook,
};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, HookKind, resolve};
use wingetrs::install::plan::{InstallPlan, Step};
use wingetrs::install::planner::{PlanInput, plan_install};
use wingetrs::install::script_host_win::{PwshHost, child_path};
use wingetrs::install::script_policy::{GpoValues, gpo_blocks, refuse_if_elevated};
use wingetrs::install::state::StateDb;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Writes `fake_pwsh.cmd` into `dir`: records into `out`, runs `extra` (cmd lines), exits `code`.
fn fake_pwsh(dir: &Path, out: &Path, code: i32, extra: &str) -> PathBuf {
    let o = out.display();
    let text = format!(
        "@echo off\r\n\
         >\"{o}\" echo ARGS=%*\r\n\
         >>\"{o}\" set SCOOP 2>&1\r\n\
         >>\"{o}\" set PSModulePath 2>&1\r\n\
         >>\"{o}\" set PATH 2>&1\r\n\
         >>\"{o}\" echo CWD=%CD%\r\n\
         {extra}\
         exit /b {code}\r\n"
    );
    let p = dir.join("fake_pwsh.cmd");
    std::fs::write(&p, text).unwrap();
    p
}

struct Unit {
    t: TempDir,
    workdir: PathBuf,
    out: PathBuf,
}

fn unit() -> Unit {
    let t = tempfile::tempdir().unwrap();
    let workdir = t
        .path()
        .join("root")
        .join("tmp")
        .join("script-app-post_install-0");
    std::fs::create_dir_all(&workdir).unwrap();
    let out = t.path().join("record.txt");
    Unit { t, workdir, out }
}

impl Unit {
    fn job(&self) -> ScriptJob {
        ScriptJob {
            app: "app".into(),
            hook: HookKind::PostInstall,
            workdir: self.workdir.clone(),
            runner: self.workdir.join("runner.ps1"),
            env: vec![
                ("SCOOP".into(), self.t.path().join("root").into_os_string()),
                // Planted: the host must drop it even when asked to set it.
                ("PSModulePath".into(), OsString::from(r"C:\planted\modules")),
            ],
        }
    }

    fn host(&self, code: i32, extra: &str) -> PwshHost {
        PwshHost::with_program(fake_pwsh(self.t.path(), &self.out, code, extra))
    }

    fn record(&self) -> Vec<String> {
        // `set` of a missing variable answers in the console code page: read it lossy.
        String::from_utf8_lossy(&std::fs::read(&self.out).unwrap())
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn value(&self, name: &str) -> Option<String> {
        let prefix = format!("{}=", name.to_ascii_lowercase());
        self.record().into_iter().find_map(|l| {
            l.to_ascii_lowercase()
                .starts_with(&prefix)
                .then(|| l[prefix.len()..].to_string())
        })
    }
}

#[test]
fn args_exact() {
    let u = unit();
    let out = u.host(0, "").run(&u.job()).unwrap();
    assert_eq!(out.code, 0);
    let expected = format!(
        "ARGS=-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File {}",
        u.workdir.join("runner.ps1").display()
    );
    assert_eq!(u.record()[0], expected);
}

#[test]
fn env_scoop_set_psmodulepath_removed() {
    let u = unit();
    u.host(0, "").run(&u.job()).unwrap();
    assert_eq!(
        u.value("SCOOP").as_deref(),
        Some(u.t.path().join("root").to_str().unwrap())
    );
    assert_eq!(u.value("PSModulePath"), None, "{:?}", u.record());
}

#[test]
fn env_path_is_fixed_not_inherited() {
    let u = unit();
    let program = fake_pwsh(u.t.path(), &u.out, 0, "");
    PwshHost::with_program(program.clone())
        .run(&u.job())
        .unwrap();
    let path = u.value("PATH").unwrap();
    let expected = child_path(&program).unwrap();
    assert_eq!(path, expected.to_str().unwrap());
    let first = path.split(';').next().unwrap().to_ascii_lowercase();
    assert!(first.ends_with(r"\system32"), "{path}");
    assert!(
        path.to_ascii_lowercase()
            .contains(&u.t.path().to_str().unwrap().to_ascii_lowercase()),
        "the helper's own folder is on PATH: {path}"
    );
    // The test process inherits the user's PATH (cargo's own folder at least): not passed on.
    let inherited = std::env::var("PATH").unwrap();
    for entry in inherited.split(';').filter(|e| !e.is_empty()) {
        if !expected
            .to_str()
            .unwrap()
            .split(';')
            .any(|x| x.eq_ignore_ascii_case(entry))
        {
            assert!(
                !path
                    .to_ascii_lowercase()
                    .contains(&entry.to_ascii_lowercase()),
                "{entry} leaked into {path}"
            );
        }
    }
}

#[test]
fn cwd_is_workdir() {
    let u = unit();
    u.host(0, "").run(&u.job()).unwrap();
    let cwd = u.value("CWD").unwrap();
    assert!(
        cwd.eq_ignore_ascii_case(u.workdir.to_str().unwrap()),
        "{cwd} != {}",
        u.workdir.display()
    );
}

#[test]
fn exit_code_and_lines() {
    let u = unit();
    let out = u
        .host(3, "echo linha1\r\necho linha2\r\n")
        .run(&u.job())
        .unwrap();
    assert_eq!(out.code, 3);
    assert_eq!(out.lines, vec!["linha1".to_string(), "linha2".to_string()]);
}

#[test]
fn policy_error_mapped() {
    let u = unit();
    let extra = "echo Arquivo runner.ps1 nao pode ser carregado. UnauthorizedAccess segredo-QX\r\n";
    let err = u.host(1, extra).run(&u.job()).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("política de execução"), "{text}");
    assert!(
        !text.contains("segredo-QX"),
        "the output is not dumped: {text}"
    );
    // Exit 1 without a policy line is an ordinary failure, with its lines.
    let out = u.host(1, "echo outra falha\r\n").run(&u.job()).unwrap();
    assert_eq!(out.code, 1);
    assert_eq!(out.lines, vec!["outra falha".to_string()]);
    // A policy-looking line with another code is the script's own business.
    let out = u.host(2, extra).run(&u.job()).unwrap();
    assert_eq!(out.code, 2);
}

#[test]
fn gpo_blocks_cases() {
    let g = |me: Option<u32>, mp: Option<&str>, ue: Option<u32>, up: Option<&str>| GpoValues {
        machine_enable_scripts: me,
        machine_policy: mp.map(str::to_string),
        user_enable_scripts: ue,
        user_policy: up.map(str::to_string),
    };
    let blocked = |v: &GpoValues| gpo_blocks(v).expect("blocks");
    let m = blocked(&g(Some(0), None, None, None));
    assert!(m.contains("política de grupo (GPO)"), "{m}");
    assert!(m.contains("peça ao administrador"), "{m}");
    assert!(blocked(&g(None, Some("AllSigned"), None, None)).contains("AllSigned"));
    assert!(blocked(&g(None, Some("Restricted"), None, None)).contains("Restricted"));
    assert!(blocked(&g(None, None, Some(0), None)).contains("GPO"));
    assert!(blocked(&g(None, None, None, Some("allsigned"))).contains("allsigned"));
    assert_eq!(
        gpo_blocks(&g(Some(1), Some("RemoteSigned"), None, None)),
        None
    );
    assert_eq!(gpo_blocks(&g(None, Some("Unrestricted"), None, None)), None);
    assert_eq!(gpo_blocks(&g(None, Some("Bypass"), None, None)), None);
    assert_eq!(gpo_blocks(&GpoValues::default()), None);
    // Machine RemoteSigned + user AllSigned: blocked by the user's.
    let m = blocked(&g(None, Some("RemoteSigned"), None, Some("AllSigned")));
    assert!(m.contains("(AllSigned)"), "{m}");
    // Machine AllSigned + user Bypass: blocked by the machine's (it comes first).
    let m = blocked(&g(None, Some("AllSigned"), None, Some("Bypass")));
    assert!(m.contains("(AllSigned)"), "{m}");
    // Machine before user when both block.
    let m = blocked(&g(None, Some("Restricted"), None, Some("AllSigned")));
    assert!(m.contains("(Restricted)"), "{m}");
}

#[test]
fn elevated_refused() {
    let e = refuse_if_elevated(true).unwrap_err();
    assert_eq!(
        e,
        "o wingetrs não roda scripts de manifesto com o processo elevado"
    );
    assert_eq!(refuse_if_elevated(false), Ok(()));
}

// --- Through the executor -------------------------------------------------------------

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            z.start_file(*name, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

struct World {
    _server: MockServer,
    t: TempDir,
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

async fn world() -> World {
    let server = MockServer::start().await;
    let t = tempfile::tempdir().unwrap();
    let layout = Layout::new(t.path().join("root"));
    let data = DataDir::new(layout.root());
    let zip = zip_of(&[("tool.txt", b"hi")]);
    let sha = hex::encode(Sha256::digest(&zip));
    Mock::given(method("GET"))
        .and(path("/pkg.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
        .mount(&server)
        .await;
    let manifest = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/pkg.zip", server.uri()),
        "hash": sha,
        "post_install": "info 'x'",
    })
    .to_string();
    let shortcuts = FakeShortcuts::new(t.path().join("startmenu"));
    let root = wingetrs::install::traits::Shortcuts::root(&shortcuts);
    let plan = plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &resolve(&manifest, Arch::X64).unwrap(),
        layout: &layout,
        shortcut_root: &root,
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
    })
    .unwrap();
    World {
        _server: server,
        state_path: t.path().join("state").join("state.db"),
        t,
        layout,
        data,
        env: FakeEnv::new(),
        shortcuts,
        trash: FakeTrash::new(),
        registry: FakeRegistry::new(),
        http: Http::new().unwrap(),
        plan,
    }
}

impl World {
    async fn install(
        &self,
        host: PwshHost,
        hook: &dyn StepHook,
        gate: LaunchGate,
    ) -> Result<(), ExecError> {
        let mut state = StateDb::open(&self.state_path).unwrap();
        let scripts: Arc<dyn ScriptHost> = Arc::new(host);
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
            state: &mut state,
            hook,
            progress: &no_progress,
            in_use: &nothing_in_use,
            msiexec: None,
            launch: gate,
            runners: Runners {
                scripts: Some(scripts),
                ..Runners::disabled()
            },
        };
        wingetrs::install::executor::execute_install(&self.plan, &mut ctx).await
    }

    fn script_dirs(&self) -> Vec<String> {
        // A dropped install also removes `<root>	mp` when nothing else is in it.
        let Ok(rd) = std::fs::read_dir(self.layout.root().join("tmp")) else {
            return Vec::new();
        };
        rd.flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("script-"))
            .collect()
    }
}

#[tokio::test]
async fn workdir_removed_with_code_1() {
    let w = world().await;
    let out = w.t.path().join("record.txt");
    let host = PwshHost::with_program(fake_pwsh(w.t.path(), &out, 1, "echo falhou\r\n"));
    let r = w.install(host, &NoHook, LaunchGate::default()).await;
    let e = r.unwrap_err().to_string();
    assert!(e.contains("código 1"), "{e}");
    assert!(out.is_file(), "the fake ran");
    // `<root>\tmp` exists (the folder was made there) and holds no `script-*`.
    assert!(w.layout.root().join("tmp").is_dir());
    assert!(w.script_dirs().is_empty(), "{:?}", w.script_dirs());
}

/// Cancels the install right before its hook step (a Ctrl+C before PowerShell starts).
struct CancelBeforeHook(LaunchGate);
impl StepHook for CancelBeforeHook {
    fn before(&self, _i: usize, step: &Step) -> Result<(), ExecError> {
        if matches!(step, Step::Hook { .. }) {
            assert!(self.0.try_cancel(), "nothing launched yet");
        }
        Ok(())
    }
}

#[tokio::test]
async fn cancel_before_launch_runs_nothing() {
    let w = world().await;
    let out = w.t.path().join("record.txt");
    let host = PwshHost::with_program(fake_pwsh(w.t.path(), &out, 0, ""));
    let gate = LaunchGate::default();
    let hook = CancelBeforeHook(gate.clone());
    let r = w.install(host, &hook, gate.clone()).await;
    let e = r.unwrap_err().to_string();
    assert!(e.contains("cancelada antes de o script começar"), "{e}");
    assert!(!out.exists(), "PowerShell never started");
    assert!(w.script_dirs().is_empty(), "{:?}", w.script_dirs());
    assert!(!w.layout.version_dir("app", "1.0").exists(), "rolled back");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_after_launch_waits_for_the_hook() {
    let w = world().await;
    let out = w.t.path().join("record.txt");
    let started = w.t.path().join("started.txt");
    let done = w.t.path().join("done.txt");
    // Starts, then keeps running ~2 s (ping waits 1 s between its echoes), then marks the end.
    let extra = format!(
        ">\"{}\" echo 1\r\n\"%SystemRoot%\\System32\\ping.exe\" -n 3 127.0.0.1 >nul\r\n>\"{}\" echo 1\r\n",
        started.display(),
        done.display()
    );
    let host = PwshHost::with_program(fake_pwsh(w.t.path(), &out, 0, &extra));
    let gate = LaunchGate::default();
    let wait_started = async {
        while !started.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let race = async {
        tokio::select! {
            r = w.install(host, &NoHook, gate.clone()) => Some(r),
            () = wait_started => None,
        }
    };
    // The drop of the install (the Ctrl+C) waits for the hook; never forever.
    let raced = tokio::time::timeout(Duration::from_secs(60), race)
        .await
        .expect("the hook ends on its own");
    assert!(
        raced.is_none(),
        "the install was dropped while the hook ran"
    );
    assert!(done.is_file(), "the hook ran to its end, nobody killed it");
    assert!(!gate.try_cancel(), "the launch won: the cancel was refused");
    assert!(w.script_dirs().is_empty(), "{:?}", w.script_dirs());
}
