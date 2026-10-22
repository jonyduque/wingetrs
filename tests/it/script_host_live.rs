//! Phase 2c tests (Task 160).
//!
//! Live: runs the real PowerShell (`PwshHost::locate`) on a synthetic hook. Agents never run
//! these without the user asking (`cargo test --release -- --ignored script_host_live`).
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::env_win::RegEnvStore;
use wingetrs::install::executor::{ExecCtx, NoHook, Runners, ScriptHost, ScriptJob, ScriptOutcome};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::planner::{PlanInput, plan_install};
use wingetrs::install::script_host_win::PwshHost;
use wingetrs::install::state::StateDb;
use wingetrs::install::traits::fake::{FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{Effects, EnvStore, Shortcuts};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// What the real host left in the work folder before the executor removed it.
struct Recording {
    inner: PwshHost,
    marker: PathBuf,
    seen: Mutex<Option<(String, bool)>>,
}

impl ScriptHost for Recording {
    fn run(&self, job: &ScriptJob) -> std::io::Result<ScriptOutcome> {
        let r = self.inner.run(job);
        let effects =
            std::fs::read_to_string(job.workdir.join("effects.jsonl")).unwrap_or_default();
        *self.seen.lock().unwrap() = Some((effects, self.marker.is_file()));
        r
    }
}

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

/// First level of `%USERPROFILE%\scoop` (names and modification times).
fn scoop_listing() -> BTreeMap<String, Option<std::time::SystemTime>> {
    let scoop = PathBuf::from(std::env::var_os("USERPROFILE").unwrap()).join("scoop");
    match std::fs::read_dir(scoop) {
        Ok(rd) => rd
            .flatten()
            .map(|e| {
                let modified = e.metadata().and_then(|m| m.modified()).ok();
                (e.file_name().to_string_lossy().into_owned(), modified)
            })
            .collect(),
        Err(_) => BTreeMap::new(),
    }
}

/// Removes the `WINGETRS_TEST_ENV_KEY`-style key on every way out.
struct DropTestKey(String);

impl Drop for DropTestKey {
    fn drop(&mut self) {
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.0);
    }
}

#[tokio::test]
#[ignore = "live: roda o PowerShell real com script sintético"]
async fn real_pwsh_runs_synthetic_hook_inside_the_sandbox() {
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
    let hook = r#"Set-Content -LiteralPath "$dir\marker.txt" -Value 'ok'; Set-Content -LiteralPath "$dir\orig.txt" -Value $original_dir -NoNewline; Add-Path -Path "$dir\bin""#;
    let manifest = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/pkg.zip", server.uri()),
        "hash": sha,
        "post_install": hook,
    })
    .to_string();
    let shortcuts = FakeShortcuts::new(t.path().join("startmenu"));
    let plan = plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &resolve(&manifest, Arch::X64).unwrap(),
        layout: &layout,
        shortcut_root: &shortcuts.root(),
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
        ast: &wingetrs::install::script_ast::LexerOnly,
    })
    .unwrap();
    let version_dir = layout.version_dir("app", "1.0");
    let recording = Arc::new(Recording {
        inner: PwshHost::locate(),
        marker: version_dir.join("marker.txt"),
        seen: Mutex::new(None),
    });
    let scoop_before = scoop_listing();
    // The registry store under a test key, as `WINGETRS_TEST_ENV_KEY` selects it: the real
    // `HKCU\Environment` is never opened.
    let test_key = DropTestKey(format!(
        r"Software\wingetrs-test\t160-live-{}",
        std::process::id()
    ));
    let env = RegEnvStore::for_test_key(&test_key.0);
    let (trash, registry) = (FakeTrash::new(), FakeRegistry::new());
    let http = Http::new().unwrap();
    let mut state = StateDb::open(&t.path().join("state").join("state.db")).unwrap();
    let scripts: Arc<dyn ScriptHost> = recording.clone();
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
        progress: &no_progress,
        in_use: &nothing_in_use,
        msiexec: None,
        launch: Default::default(),
        runners: Runners {
            ast: std::sync::Arc::new(wingetrs::install::script_ast::LexerOnly),
            scripts: Some(scripts),
            ..Runners::disabled()
        },
    };
    let result = wingetrs::install::executor::execute_install(&plan, &mut ctx).await;
    let (effects, marker) = recording
        .seen
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| panic!("the hook never ran: {result:?}"));
    assert!(marker, "the hook wrote into $dir: {result:?}");
    // In post_install `$dir` is the `current` junction and `$original_dir` the version folder.
    let line = effects
        .lines()
        .find(|l| l.contains("add_path"))
        .unwrap_or_else(|| panic!("no add_path effect in {effects:?}"));
    let v: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(v["var"], "PATH");
    assert_eq!(
        v["entries"][0].as_str().unwrap(),
        layout.current("app").join("bin").to_str().unwrap()
    );
    let orig = std::fs::read_to_string(version_dir.join("orig.txt")).unwrap_or_default();
    assert_eq!(
        orig,
        version_dir.to_str().unwrap(),
        "$original_dir is the version folder"
    );
    assert_eq!(scoop_listing(), scoop_before, "~/scoop untouched");
    // The effect was applied by wingetrs, to the test key only.
    result.unwrap();
    let path = env
        .get("Path")
        .unwrap()
        .expect("PATH in the test key")
        .value;
    assert!(
        path.split(';')
            .any(|e| e.eq_ignore_ascii_case(layout.current("app").join("bin").to_str().unwrap())),
        "{path}"
    );
}

/// Every file named `name` under `dir` (recursive, no reparse points followed).
fn files_named(dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            out.extend(files_named(&e.path(), name));
        } else if e.file_name().eq_ignore_ascii_case(name) {
            out.push(e.path());
        }
    }
    out
}

#[tokio::test]
#[ignore = "live: pwsh"]
async fn constant_global_blocks_reassignment() {
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
    let hook = "$global = $true; if ($global) { New-Item (Join-Path $dir 'RAN') }";
    let manifest = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/pkg.zip", server.uri()),
        "hash": sha,
        "post_install": hook,
    })
    .to_string();
    let shortcuts = FakeShortcuts::new(t.path().join("startmenu"));
    let plan = plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &resolve(&manifest, Arch::X64).unwrap(),
        layout: &layout,
        shortcut_root: &shortcuts.root(),
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
        ast: &wingetrs::install::script_ast::LexerOnly,
    })
    .unwrap();
    let version_dir = layout.version_dir("app", "1.0");
    let recording = Arc::new(Recording {
        inner: PwshHost::locate(),
        marker: version_dir.join("RAN"),
        seen: Mutex::new(None),
    });
    let test_key = DropTestKey(format!(
        r"Software\wingetrs-test\t164-live-{}",
        std::process::id()
    ));
    let env = RegEnvStore::for_test_key(&test_key.0);
    let (trash, registry) = (FakeTrash::new(), FakeRegistry::new());
    let http = Http::new().unwrap();
    let mut state = StateDb::open(&t.path().join("state").join("state.db")).unwrap();
    let scripts: Arc<dyn ScriptHost> = recording.clone();
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
        progress: &no_progress,
        in_use: &nothing_in_use,
        msiexec: None,
        launch: Default::default(),
        runners: Runners {
            ast: std::sync::Arc::new(wingetrs::install::script_ast::LexerOnly),
            scripts: Some(scripts),
            ..Runners::disabled()
        },
    };
    let result = wingetrs::install::executor::execute_install(&plan, &mut ctx).await;
    let (_, marker) = recording
        .seen
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| panic!("the hook never ran: {result:?}"));
    assert!(result.is_err(), "the hook must fail on `$global = $true`");
    assert!(!marker, "the if ($global) block ran: {result:?}");
    assert!(files_named(t.path(), "RAN").is_empty());
}

// ---- Task 227: PowerShell's parser on synthetic scripts (analysed, never run) ----

/// The forms the lexer reads wrong or not at all (ARMADILHAS), one per line.
const HOLES: &str = "Start-Process notepad \u{2013}Verb RunAs
i`ex 'x'
Start-Job { Remove-Item C:\\Windows\\x }
&(\"i\" + \"ex\") 'y'
.\\setup.ps1 /S
cmd /c \"del x\"
sal z iex
if ($global) { Set-ItemProperty HKLM:\\x -Name a -Value 1 }
[IO.File]::WriteAllText(\"$env:ProgramFiles\\x\", 'y')
& $dir\\helper.ps1
$a = 'abc\u{2019}; Write-Output HIDDEN; \u{2018}x'
ForEach-Object -Parallel { reg add HKCU\\x } -InputObject 1
";

fn real_parser() -> wingetrs::install::script_ast_win::PwshAst {
    wingetrs::install::script_ast_win::PwshAst::for_program(PwshHost::locate().program().into())
}

#[test]
#[ignore = "live: real PowerShell parser"]
fn real_parser_sees_the_holes() {
    use wingetrs::install::prelude::PRELUDE_FUNCTIONS;
    use wingetrs::install::script_ast::{self, AstGate};
    use wingetrs::install::script_scan::{self, CallRoots, FlagKind};
    let ast = real_parser();
    let facts = ast.facts(HOLES).unwrap();
    let pwsh7 = ast
        .program()
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("pwsh.exe"));
    // `` `e `` is ESC in pwsh 7: `i`ex` is another name there; 5.1 reads `iex`.
    let iex_on_2 = facts
        .commands
        .iter()
        .any(|c| c.line == 2 && c.name.as_deref() == Some("iex"));
    assert_eq!(iex_on_2, !pwsh7, "{:?}", facts.commands);
    // Line 8 is dead code: no flag of the parser there.
    let own = script_ast::ast_flags(&facts, PRELUDE_FUNCTIONS);
    assert!(!own.iter().any(|f| f.line == 8), "{own:?}");
    let (report, _) =
        script_scan::scan_gated(HOLES, PRELUDE_FUNCTIONS, &CallRoots::default(), &ast);
    for (line, kind) in [
        (1, FlagKind::Elevation),
        (3, FlagKind::SystemWrite),
        (4, FlagKind::CalledFile),
        (6, FlagKind::CalledFile),
        (7, FlagKind::Alias),
        (9, FlagKind::SystemWrite),
        (12, FlagKind::RegistryUser),
    ] {
        assert!(
            report
                .flags
                .iter()
                .any(|f| f.line == line && f.kind == kind),
            "line {line} {kind:?}: {:?}",
            report.flags
        );
    }
}

#[test]
#[ignore = "live: real PowerShell parser"]
fn real_parser_runs_nothing() {
    use wingetrs::install::prelude::PRELUDE_FUNCTIONS;
    use wingetrs::install::script_ast::{self, AstGate};
    use wingetrs::install::script_scan::{FlagKind, Verdict};
    let t = tempfile::tempdir().unwrap();
    let tmp = t.path().display().to_string();
    std::fs::write(
        t.path().join("mark.psm1"),
        format!("Set-Content '{tmp}\\marker.txt' x\r\n"),
    )
    .unwrap();
    let script = format!(
        "using module '{tmp}\\mark.psm1'\nclass A {{ }}\nSet-Content '{tmp}\\marker2.txt' y"
    );
    let facts = real_parser().facts(&script).unwrap();
    assert!(t.path().join("mark.psm1").is_file());
    assert!(!t.path().join("marker.txt").exists());
    assert!(!t.path().join("marker2.txt").exists());
    let flags = script_ast::ast_flags(&facts, PRELUDE_FUNCTIONS);
    assert!(
        flags
            .iter()
            .any(|f| f.kind == FlagKind::DynamicCode && f.verdict == Verdict::Refuse),
        "{flags:?}"
    );
}

#[test]
#[ignore = "live: real PowerShell parser"]
fn real_parser_batch_reuses_process() {
    use std::time::Instant;
    use wingetrs::install::script_ast::AstGate;
    let ast = real_parser();
    let start = Instant::now();
    ast.facts("Write-Output 0").unwrap();
    let first = start.elapsed();
    for i in 1..20 {
        ast.facts(&format!("Write-Output {i}")).unwrap();
    }
    let total = start.elapsed();
    println!(
        "primeira {first:?}, total {total:?}, demais {:?}",
        total - first
    );
    assert!(total - first < first * 19, "{first:?} {total:?}");
}
/// A `using` the parser would resolve is refused and nothing it names is loaded: the module
/// that writes a sentinel when loaded never runs.
#[test]
#[ignore = "live: real PowerShell parser"]
fn real_parser_never_resolves_using() {
    use wingetrs::install::prelude::PRELUDE_FUNCTIONS;
    use wingetrs::install::script_scan::{self, CallRoots, Verdict};
    let t = tempfile::tempdir().unwrap();
    let tmp = t.path().display().to_string();
    std::fs::write(
        t.path().join("m.psm1"),
        format!("Set-Content '{tmp}\\sentinel.txt' x\r\n"),
    )
    .unwrap();
    let ast = real_parser();
    for script in [
        "using assembly C:\\x\\nao_existe.dll\nWrite-Output 1".to_string(),
        format!("using module '{tmp}\\m.psm1'\nWrite-Output 1"),
        "using module .\\m.psm1\nWrite-Output 1".to_string(),
    ] {
        let (report, _) =
            script_scan::scan_gated(&script, PRELUDE_FUNCTIONS, &CallRoots::default(), &ast);
        assert_eq!(
            report.verdict,
            Verdict::Refuse,
            "{script}: {:?}",
            report.flags
        );
        assert!(t.path().join("m.psm1").is_file());
        assert!(!t.path().join("sentinel.txt").exists(), "{script}");
    }
}

/// The host reports `param($global)` and `foreach ($global ...)` as bindings (`SET`).
#[test]
#[ignore = "live: real PowerShell parser"]
fn real_parser_reports_global_bindings() {
    use wingetrs::install::prelude::PRELUDE_FUNCTIONS;
    use wingetrs::install::script_ast::{self, AstGate};
    use wingetrs::install::script_scan::{FlagKind, Verdict};
    let ast = real_parser();
    for script in [
        "param($global)\nif ($global) { Remove-Item C:\\Windows\\x }",
        "function f($global) { }\nif ($global) { Remove-Item C:\\Windows\\x }",
        "foreach ($global in 1) { }\nif ($global) { Remove-Item C:\\Windows\\x }",
        "Write-Output 1 -ov global\nif ($global) { Remove-Item C:\\Windows\\x }",
    ] {
        let facts = ast.facts(script).unwrap();
        let flags = script_ast::ast_flags(&facts, PRELUDE_FUNCTIONS);
        for kind in [FlagKind::GlobalName, FlagKind::SystemWrite] {
            assert!(
                flags
                    .iter()
                    .any(|f| f.kind == kind && f.verdict == Verdict::Refuse),
                "{script}: {kind:?} {flags:?}"
            );
        }
    }
}
