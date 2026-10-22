//! The elevation allowlist of installers (Task 180) and the helper that no longer runs them
//! (Task 196).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::BufReader;
use std::path::Path;

use wingetrs::bridge::worker::{
    self, ElevatedKind,
    ElevatedKind::{Burn, Exe, Inno, Nullsoft},
    INSTALLER_REMOVED, InstallerRequest, Reply, Request, ScopeArg, WorkerOps, exe_params_allowed,
    msi_params_allowed, msi_params_allowed_in,
};
use wingetrs::bridge::{self, BridgeJob, BridgeOp, Tool};
use wingetrs::installed::correlate::InstallScope;

const SHA: &str = "0123456789abcdef0123456789ABCDEF0123456789abcdef0123456789abcdef";

/// The `Program Files` folders the fake helper allows install folders in.
fn roots() -> Vec<String> {
    vec![r"C:\Program Files".into(), r"C:\Program Files (x86)".into()]
}

fn valid() -> InstallerRequest {
    InstallerRequest {
        source: format!(r"C:\U\blobs\{}", SHA.to_ascii_lowercase()),
        sha256: SHA.into(),
        file_name: "App-1.0.msi".into(),
        kind: ElevatedKind::Msi,
        params: r#"/quiet /norestart /log "<LOGPATH>" ALLUSERS=1"#.into(),
    }
}

fn request(r: &InstallerRequest) -> Request {
    Request::RunInstaller {
        source: r.source.clone(),
        sha256: r.sha256.clone(),
        file_name: r.file_name.clone(),
        kind: r.kind,
        params: r.params.clone(),
    }
}

fn line(req: &Request) -> String {
    worker::encode(req).unwrap()
}

/// Operations that record every winget run.
#[derive(Default)]
struct FakeOps {
    runs: Vec<(String, Vec<String>)>,
}

impl WorkerOps for FakeOps {
    fn run(
        &mut self,
        program: &str,
        args: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<(i32, Vec<String>)> {
        self.runs.push((program.to_string(), args.to_vec()));
        on_line("winget");
        Ok((7, Vec::new()))
    }

    fn program_files(&self) -> Vec<String> {
        roots()
    }
}

/// Runs `serve` over `input` with `ops`; serve's result and the replies written.
fn serve_with(input: &str, ops: &mut FakeOps) -> (std::io::Result<()>, Vec<Reply>) {
    let mut out = Vec::new();
    let r = worker::serve(&mut BufReader::new(input.as_bytes()), &mut out, ops);
    let replies = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| worker::decode::<Reply>(l).unwrap())
        .collect();
    (r, replies)
}

/// Task 196: the helper never runs an installer. A `run_installer` request with every field
/// valid gets the fixed error and nothing runs; the channel stays open for the next request.
#[test]
fn worker_refuses_run_installer() {
    let input = format!(
        "{}{}",
        line(&request(&valid())),
        line(&Request::Upgrade {
            target: "Git.Git".into()
        })
    );
    let mut ops = FakeOps::default();
    let (r, replies) = serve_with(&input, &mut ops);
    r.unwrap();
    assert_eq!(ops.runs.len(), 1, "only the upgrade ran: {:?}", ops.runs);
    assert_eq!(ops.runs[0].0, "winget", "{:?}", ops.runs);
    assert!(
        !ops.runs[0].1.iter().any(|a| a.contains("blobs")),
        "{:?}",
        ops.runs
    );
    assert_eq!(
        replies,
        vec![
            Reply::Error {
                message: INSTALLER_REMOVED.into(),
                not_found: false,
            },
            Reply::Line {
                text: "winget".into(),
            },
            Reply::End { code: 7 },
        ]
    );
}

#[test]
fn bad_fields_fail_the_elevation_check() {
    let with = |f: &dyn Fn(&mut InstallerRequest)| {
        let mut r = valid();
        f(&mut r);
        r
    };
    let exe = |name: &str| {
        let name = name.to_string();
        with(&move |r| {
            r.kind = ElevatedKind::Exe;
            r.params = "/S".into();
            r.file_name = name.clone();
        })
    };
    let exe_params = |params: &str| {
        let params = params.to_string();
        with(&move |r| {
            r.kind = ElevatedKind::Inno;
            r.file_name = "setup.exe".into();
            r.params = params.clone();
        })
    };
    let bad = [
        with(&|r| r.source = r"U\blobs\a".into()),
        with(&|r| r.source = r"C:\U\..\Windows\a".into()),
        with(&|r| r.source = r"C:\U/../a".into()),
        with(&|r| r.source = r"\\h\s\a".into()),
        with(&|r| r.source = r"\\?\C:\a".into()),
        with(&|r| r.source = r"C:\U\a:ads".into()),
        with(&|r| r.source = r"C:\U\CON".into()),
        with(&|r| {
            r.source = format!(
                r"C:\{}\blobs\{}",
                "a".repeat(32_694),
                r.sha256.to_ascii_lowercase()
            )
        }),
        with(&|r| r.source = r"C:\U\blobs\a".into()),
        with(&|r| r.source = format!(r"C:\U\cache\{}", r.sha256.to_ascii_lowercase())),
        with(&|r| r.source = format!(r"C:\U\blobs\{SHA}")),
        with(&|r| r.source = format!(r"C:\U\{}", r.sha256.to_ascii_lowercase())),
        with(&|r| r.sha256 = SHA[..63].into()),
        with(&|r| r.sha256 = format!("g{}", &SHA[1..])),
        with(&|r| r.file_name = r"..\a.msi".into()),
        exe("a.msi"),
        with(&|r| r.file_name = "a.exe".into()),
        exe("a b.exe"),
        exe(&format!("{}.exe", "a".repeat(129))),
        exe(".a.exe"),
        exe("a.exe.txt"),
        with(&|r| r.params = "/quiet\n/norestart".into()),
        with(&|r| r.params = "/x {G}".into()),
        with(&|r| r.params = r"TRANSFORMS=C:\x.mst".into()),
        with(&|r| r.params = r#"INSTALLDIR="C:\Program Files\App\""#.into()),
        exe_params(r#"/LOADINF="a.inf""#),
        exe_params(r"/DIR=C:\Users\u\App"),
        exe_params(r"/S /D=C:\Users\u\App"),
        exe_params(r"/LOG=C:\t.log"),
    ];
    for req in &bad {
        assert!(req.check(&roots()).is_err(), "{req:?}");
    }
    // Control: valid at every limit.
    let edge = with(&|r| {
        r.kind = ElevatedKind::Exe;
        r.params = "/S".into();
        r.file_name = format!("{}.exe", "a".repeat(128));
        r.source = format!(
            r"C:\{}\blobs\{}",
            "a".repeat(32_693),
            r.sha256.to_ascii_lowercase()
        );
        r.sha256 = r.sha256.to_ascii_uppercase();
    });
    assert_eq!(edge.source.len(), worker::SOURCE_MAX);
    assert_eq!(edge.check(&roots()), Ok(()));
}

#[test]
fn msi_params_allowlist() {
    for ok in [
        r#"/quiet /norestart /log "<LOGPATH>""#,
        "ALLUSERS=1 ADDLOCAL=ALL",
        "/passive",
        "",
        r#"/QN /qb /promptrestart REBOOT=ReallySuppress MSIINSTALLPERUSER="" allusers="2""#,
    ] {
        assert!(msi_params_allowed(ok), "{ok}");
    }
    // Install folders: only inside Program Files (from the known folders).
    for ok in [
        r#"INSTALLDIR="C:\Program Files\App""#,
        r#"TARGETDIR="c:\program files (x86)\App\sub""#,
    ] {
        assert!(msi_params_allowed_in(ok, &roots()), "{ok}");
        assert!(!msi_params_allowed(ok), "no roots: {ok}");
    }
    for bad in [
        r#"INSTALLDIR="C:\Program Files""#,
        r"INSTALLDIR=C:\Program Files Evil\x",
        r"INSTALLDIR=C:\Program Files\..\Users\x",
        r"INSTALLDIR=C:\Users\u\App",
        r"TARGETDIR=\h\s\App",
        r"INSTALLDIR=App",
    ] {
        assert!(!msi_params_allowed_in(bad, &roots()), "{bad}");
    }
    for bad in [
        r#"/i "C:\x.msi""#,
        "/x {G}",
        "/x",
        // Bare switches: refused on their own, not because of the token after them.
        "/i",
        "/a",
        "/p",
        "/package",
        "/uninstall",
        "/update",
        "/jm",
        "/l*v",
        "/a x",
        "/p p.msp",
        "/fa",
        "/package x",
        r#"/l*v "C:\t.log""#,
        r#"/log "C:\t.log""#,
        r#"TRANSFORMS="a"#,
        "/log",
        "/j u x",
        "/uninstall x",
        "/update x",
        "-x {G}",
        "1A=1",
        r#"ALLUSERS="1"2""#,
        r#"ALLUSERS=1"2"#,
        r#"ALLUSERS=1"""#,
        r#"ADDLOCAL="a"b"c""#,
        // Property names outside the closed list.
        r"TRANSFORMS=C:\x.mst",
        r"PATCH=C:\p.msp",
        "ACTION=ADMIN",
        "REINSTALL=ALL",
        "TRANSFORMS=:x",
        "my.prop=1",
        // Values outside each property's list.
        "ALLUSERS=3",
        "MSIINSTALLPERUSER=2",
        "REBOOT=Now",
        r"ADDLOCAL=C:\x",
        "ADDLOCAL=",
        "/quiet\t/x",
        "/quiet /log \"<LOGPATH>\" /qn \"<LOGPATH>\"",
    ] {
        assert!(!msi_params_allowed(bad), "{bad}");
    }
}

#[test]
fn exe_params_allowlist() {
    let ok = [
        (
            Inno,
            r#"/SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG="<LOGPATH>""#,
        ),
        (
            Inno,
            r#"/silent /closeapplications /nocancel /allusers /DIR="C:\Program Files (x86)\App""#,
        ),
        (Inno, "/CURRENTUSER /LOG=<LOGPATH>"),
        (Nullsoft, "/S"),
        (Nullsoft, r"/S /D=C:\Program Files\My App"),
        (Burn, r#"/quiet /norestart /log "<LOGPATH>""#),
        (Burn, r#"-passive -log "<LOGPATH>""#),
        (
            Exe,
            r#"/silent --install -q /mode=unattended /TYPE:full /log "<LOGPATH>""#,
        ),
        (
            Exe,
            r#"/COMPONENTS="a,b" /TASKS=!desktopicon /out=<LOGPATH>"#,
        ),
        (Inno, ""),
    ];
    for (kind, params) in ok {
        assert!(
            exe_params_allowed(kind, params, &roots()),
            "{kind:?} {params}"
        );
    }
    let bad = [
        // Unknown switches and loose positional arguments.
        (Inno, "/VERYSILENT /PASSWORD=x"),
        (Inno, "/VERYSILENT setup.iss"),
        (Nullsoft, "/S /NCRC"),
        (Nullsoft, "/s"),
        (Burn, "/quiet /layout x"),
        (Burn, "/quiet extra"),
        (Exe, "loose"),
        (Exe, r#"/silent "<LOGPATH>" "<LOGPATH>""#),
        // Paths outside the execution folder or Program Files.
        (Exe, "//server/share/x"),
        (Exe, "/LOG=//s/x"),
        (Exe, r"/f1C:\x.iss"),
        (Exe, "/config=a/b"),
        (Exe, r"/config=a\b"),
        (Exe, "/config=..x"),
        (Exe, "/config=c:x"),
        (Exe, r#"/config="a\b""#),
        (Exe, r"\\h\s\x"),
        (Inno, "/LOG=//s/x"),
        (Inno, r"/LOG=C:\t.log"),
        (Inno, "/DIR=C:/Users/x"),
        (Inno, "/DIR=C:/Program Files/x"),
        (Inno, r"/DIR=C:\Users\u\App"),
        (Inno, r#"/DIR="C:\Program Files""#),
        (Nullsoft, r"/S /D=C:\Users\u\App"),
        (Nullsoft, r"/S /D=C:\Program Files\..\Users\u"),
        (Nullsoft, "/S /D=//s/Program Files/x"),
        (Burn, r#"/log "C:\t.log""#),
        (Burn, "/log //s/x"),
        // Configuration files.
        (Exe, "/LOADINF=a.inf"),
        (Exe, "-saveinf=a.inf"),
        (Inno, r#"/LOADINF="a.inf""#),
        (Inno, r#"/S "unclosed"#),
    ];
    for (kind, params) in bad {
        assert!(
            !exe_params_allowed(kind, params, &roots()),
            "{kind:?} {params}"
        );
    }
    // Without the known folders no install folder passes.
    assert!(!exe_params_allowed(
        Nullsoft,
        r"/S /D=C:\Program Files\App",
        &[]
    ));
}
/// Task 190 addendum: `/norestart` for nullsoft (GPXSee) and Burn's exact
/// `InstallAllUsers=1` (Python, machine scope) pass; nothing near them does.
#[test]
fn norestart_and_install_all_users_pass_as_exact_literals() {
    for (kind, params) in [
        (Nullsoft, "/S /norestart"),
        (Nullsoft, r"/S /NORESTART /D=C:\Program Files\App"),
        (Burn, "/quiet InstallAllUsers=1"),
        (Burn, r#"/passive installallusers=1 /log "<LOGPATH>""#),
    ] {
        assert!(
            exe_params_allowed(kind, params, &roots()),
            "{kind:?} {params}"
        );
    }
    for (kind, params) in [
        (Nullsoft, "/S /norestartx"),
        (Nullsoft, "/S norestart"),
        (Burn, "/quiet InstallAllUsers=0"),
        (Burn, "/quiet InstallAllUsers=1x"),
        (Burn, r"/quiet TargetDir=C:\x"),
        (Burn, "/quiet PrependPath=2"),
        (Inno, "/VERYSILENT InstallAllUsers=1"),
        // `<LOGPATH>` only as a whole value, never inside other text.
        (Exe, "/out=x<LOGPATH>"),
        (Exe, r#"/out="<LOGPATH>x""#),
    ] {
        assert!(
            !exe_params_allowed(kind, params, &roots()),
            "{kind:?} {params}"
        );
    }
}

/// Task 203 item 9: Burn's `PrependPath=1` and `PrependPath=0` (Python, machine scope) pass
/// as exact literals, like `InstallAllUsers=1`; no prefix nor other value does.
#[test]
fn prepend_path_passes_as_exact_literals() {
    for params in [
        "/quiet InstallAllUsers=1 PrependPath=1",
        "/passive PrependPath=0",
        "/quiet prependpath=1",
        "/quiet PREPENDPATH=0 /norestart",
    ] {
        assert!(exe_params_allowed(Burn, params, &roots()), "{params}");
    }
    for (kind, params) in [
        (Burn, "/quiet PrependPath=2"),
        (Burn, "/quiet PrependPath=1x"),
        (Burn, "/quiet PrependPath=10"),
        (Burn, "/quiet PrependPath="),
        (Burn, "/quiet PrependPath"),
        (Burn, "/quiet XPrependPath=1"),
        (Burn, r"/quiet PrependPath=1\x"),
        (Inno, "/VERYSILENT PrependPath=1"),
        (Nullsoft, "/S PrependPath=1"),
    ] {
        assert!(
            !exe_params_allowed(kind, params, &roots()),
            "{kind:?} {params}"
        );
    }
}

/// `serve` stops at `bad` without running it nor the valid request after it.
fn stops_at(bad: &str) {
    let input = format!("{bad}\n{}", line(&request(&valid())));
    let mut ops = FakeOps::default();
    let (r, replies) = serve_with(&input, &mut ops);
    assert!(r.is_err(), "{bad}");
    assert!(ops.runs.is_empty(), "{bad}");
    assert!(replies.is_empty(), "{bad}: {replies:?}");
}

#[test]
fn unknown_kind_and_extra_field_close() {
    let good = line(&request(&valid()));
    // The valid line, as written, decodes: the cases below differ only in what they test.
    assert!(worker::decode::<Request>(good.trim_end()).is_ok());
    stops_at(
        &good
            .trim_end()
            .replace(r#""kind":"msi""#, r#""kind":"msix""#),
    );
    stops_at(
        &good
            .trim_end()
            .replace(r#""kind":"msi""#, r#""kind":"msi","cmd":"calc""#),
    );
    stops_at(
        &good
            .trim_end()
            .replace(r#","params":"#, r#","cmd":"calc","params":"#),
    );
}

#[test]
fn upgrade_and_install_still_work() {
    let job = |op| BridgeJob {
        op,
        tool: Tool::Winget,
        target: "Git.Git".into(),
        name: "Git.Git".into(),
        installed: None,
        available: None,
        scope: None,
    };
    let input = format!(
        "{}{}",
        line(&Request::Upgrade {
            target: "Git.Git".into()
        }),
        line(&Request::Install {
            target: "Git.Git".into(),
            scope: Some(ScopeArg::Machine),
        })
    );
    let mut ops = FakeOps::default();
    let (r, replies) = serve_with(&input, &mut ops);
    r.unwrap();
    let install = BridgeOp::Install {
        scope: Some(InstallScope::Machine),
        agreements: Vec::new(),
    };
    assert_eq!(
        ops.runs,
        vec![
            bridge::command(&job(BridgeOp::Upgrade), Path::new(""), ""),
            bridge::command(&job(install), Path::new(""), ""),
        ]
    );
    assert!(
        ops.runs[1]
            .1
            .windows(2)
            .any(|w| w == ["--scope", "machine"])
    );
    let one = [
        Reply::Line {
            text: "winget".into(),
        },
        Reply::End { code: 7 },
    ];
    assert_eq!(replies, [one.clone(), one].concat());
}

/// Task 196 (from Task 197): a helper that cannot start because wingetrs is not in an
/// administrator-only folder gives `NotAdminFolder` with the refusal text alone (never "no
/// permission" or a refused UAC), now and on every later request of the session, and a
/// machine-wide bridge job is told that text instead of the generic note.
#[test]
fn folder_refusal_is_not_a_refused_uac() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wingetrs::bridge::elevated::{
        ElevatedBridge, ElevatedError, ElevatedRunner, Helper, REFUSED_NOTE, Transport,
    };
    use wingetrs::bridge::helper_folder::refusal_text;
    use wingetrs::bridge::worker::{DeleteReason, MachineHive};

    struct FolderRefused(String, Arc<AtomicUsize>);
    impl Transport for FolderRefused {
        fn start(&self) -> Result<Box<dyn Helper>, String> {
            self.1.fetch_add(1, Ordering::SeqCst);
            Err(self.0.clone())
        }
        // Refused before any prompt opens: no UAC notice (Task 201).
        fn start_with_notice(&self, _: &mut dyn FnMut()) -> Result<Box<dyn Helper>, String> {
            self.start()
        }
    }
    let text = refusal_text(Path::new(r"C:\Users\u\Downloads"));
    let starts = Arc::new(AtomicUsize::new(0));
    let bridge = ElevatedBridge::new(Box::new(FolderRefused(text.clone(), Arc::clone(&starts))));
    for _ in 0..2 {
        let e = bridge
            .delete_arp_key(MachineHive::Hklm64, "App", DeleteReason::Orphan)
            .unwrap_err();
        assert_eq!(e, ElevatedError::NotAdminFolder(text.clone()));
        assert_eq!(e.to_string(), text);
        assert_eq!(e.context("falha ao apagar App"), text);
    }
    assert_eq!(starts.load(Ordering::SeqCst), 1, "asked once per session");

    // A refused UAC keeps its own error.
    let other = ElevatedBridge::new(Box::new(FolderRefused(
        "recusado".into(),
        Arc::new(AtomicUsize::new(0)),
    )));
    let e = other
        .delete_arp_key(MachineHive::Hklm64, "App", DeleteReason::Orphan)
        .unwrap_err();
    assert_eq!(e, ElevatedError::Refused);

    // A machine-wide winget job through the bridge: the folder text, not the generic note.
    let fresh = ElevatedBridge::new(Box::new(FolderRefused(
        text.clone(),
        Arc::new(AtomicUsize::new(0)),
    )));
    let job = BridgeJob {
        op: BridgeOp::Upgrade,
        tool: Tool::Winget,
        target: "Git.Git".into(),
        name: "Git.Git".into(),
        installed: None,
        available: None,
        scope: Some(InstallScope::Machine),
    };
    let mut current = |_: &str,
                       _: &[String],
                       _: &mut dyn FnMut(&str)|
     -> std::io::Result<(i32, Vec<String>)> { Ok((0, Vec::new())) };
    let mut lines = Vec::new();
    let chosen = fresh.exec_for(&job, &mut current, &mut |l: &str| lines.push(l.to_string()));
    assert!(!chosen.elevated());
    assert_eq!(lines, std::slice::from_ref(&text));
    assert!(!lines.iter().any(|l| l == REFUSED_NOTE));
}

#[cfg(windows)]
mod win {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;
    use wingetrs::bridge::staging_win::{self, RUN_SDDL, RunDir};
    use wingetrs::winget_install::launch::{LaunchSpec, Launcher, Show, Verb};
    use wingetrs::winget_install::launch_win::ShellLauncher;
    use wingetrs::winget_install::switches;
    use wingetrs::winget_install::verified_win::VerifiedFile;

    /// `RUN_SDDL` plus the owner (the unelevated test user), who may then write inside.
    const TEST_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)";

    fn sha_of(path: &Path) -> String {
        Sha256::digest(std::fs::read(path).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn entries(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect()
    }

    /// Premise of the design (brief step 0): a file held open by `VerifiedFile` (sharing only
    /// reads) can still be started by `ShellExecuteExW`, and cannot be written meanwhile.
    #[test]
    fn verified_handle_still_launches() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("probe.cmd");
        std::fs::write(
            &script,
            "@echo off\r\necho ran> \"%~dp0ran.txt\"\r\nexit /b 3\r\n",
        )
        .unwrap();
        let held = VerifiedFile::open(&script, &sha_of(&script)).unwrap();
        assert!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&script)
                .is_err()
        );
        let code = ShellLauncher
            .run(&LaunchSpec {
                program: script.clone(),
                params: String::new(),
                dir: Some(dir.path().to_path_buf()),
                show: Show::Hidden,
                verb: Verb::Open,
            })
            .unwrap();
        assert_eq!(code, 3);
        assert!(dir.path().join("ran.txt").is_file());
        drop(held);
    }

    #[test]
    fn staging_dacl_has_no_users() {
        if crate::non_elevated::skip_if_elevated_in_ci("o administrador grava no staging") {
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let run = RunDir::create(&root, RUN_SDDL).unwrap();
        let name = run
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(name.len(), 32);
        assert!(name.bytes().all(|b| b.is_ascii_hexdigit()));
        let sddl = staging_win::dacl_sddl(run.path()).unwrap();
        assert!(sddl.starts_with("D:P"), "{sddl}");
        let sids: Vec<&str> = sddl
            .split('(')
            .skip(1)
            .map(|ace| ace.trim_end_matches(')').rsplit(';').next().unwrap())
            .collect();
        assert!(!sids.is_empty(), "{sddl}");
        assert!(sids.iter().all(|s| *s == "SY" || *s == "BA"), "{sddl}");
        assert!(sids.contains(&"SY") && sids.contains(&"BA"), "{sddl}");
        // The unelevated creator cannot plant anything in it.
        let e = std::fs::write(run.path().join("version.dll"), b"x").unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");
        // Test cleanup only: the owner may still rewrite the DACL; the helper runs as admin.
        let reset = std::process::Command::new("icacls")
            .arg(run.path())
            .arg("/reset")
            .output()
            .unwrap();
        assert!(reset.status.success(), "{reset:?}");
        drop(run);
        assert!(entries(&root).is_empty());
    }

    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "mklink /J");
    }

    #[test]
    fn staging_root_refuses_reparse_points() {
        let dir = TempDir::new().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();

        let pf = dir.path().join("pf1");
        std::fs::create_dir(&pf).unwrap();
        junction(&pf.join("wingetrs"), &elsewhere);
        let e = staging_win::staging_root(&pf).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");
        assert!(entries(&elsewhere).is_empty(), "nothing created through it");

        let pf = dir.path().join("pf2");
        std::fs::create_dir_all(pf.join("wingetrs")).unwrap();
        junction(&pf.join("wingetrs").join("staging"), &elsewhere);
        let e = staging_win::staging_root(&pf).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");

        let pf = dir.path().join("pf3");
        std::fs::create_dir(&pf).unwrap();
        let root = staging_win::staging_root(&pf).unwrap();
        assert_eq!(root, pf.join("wingetrs").join("staging"));
        assert!(root.is_dir());
        assert_eq!(
            staging_win::staging_root(&pf).unwrap(),
            root,
            "again: reused"
        );
    }

    #[test]
    fn prune_removes_only_stale_run_folders() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let run = root.join("0123456789abcdef0123456789abcdef");
        std::fs::create_dir(&run).unwrap();
        std::fs::write(run.join("setup.exe"), b"x").unwrap();
        let keep = [
            root.join("keep"),
            root.join("ABCDEF0123456789ABCDEF0123456789"),
            root.join("0123456789abcdef0123456789abcde"),
        ];
        for k in &keep {
            std::fs::create_dir(k).unwrap();
        }
        let file = root.join("fedcba9876543210fedcba9876543210");
        std::fs::write(&file, b"x").unwrap();
        let now = SystemTime::now();
        assert_eq!(staging_win::prune(root, now), 0, "fresh: kept");
        assert!(run.is_dir());
        let later = now + Duration::from_secs(25 * 60 * 60);
        assert_eq!(staging_win::prune(root, later), 1);
        assert!(!run.exists());
        assert!(keep.iter().all(|k| k.is_dir()));
        assert!(file.is_file());
    }

    fn fake_system() -> staging_win::SystemEnv {
        let p = |s: &str| PathBuf::from(s);
        staging_win::SystemEnv {
            windows: p(r"C:\Windows"),
            system32: p(r"C:\Windows\System32"),
            program_files: p(r"C:\Program Files"),
            program_files_x86: p(r"C:\Program Files (x86)"),
            program_w6432: p(r"C:\Program Files"),
            common_files: p(r"C:\Program Files\Common Files"),
            common_files_x86: p(r"C:\Program Files (x86)\Common Files"),
            common_w6432: p(r"C:\Program Files\Common Files"),
            program_data: p(r"C:\ProgramData"),
            public: p(r"C:\Users\Public"),
            profile: p(r"C:\Users\u"),
            app_data: p(r"C:\Users\u\AppData\Roaming"),
            local_app_data: p(r"C:\Users\u\AppData\Local"),
            architecture: "AMD64".into(),
            level: 6,
            revision: 0x9e0a,
            processors: 8,
        }
    }

    #[test]
    fn installer_env_is_a_closed_list() {
        let tmp = Path::new(r"C:\Program Files\wingetrs\staging\ab\tmp");
        let env = staging_win::installer_env(&fake_system(), tmp);
        let get = |k: &str| {
            env.iter()
                .find(|(n, _)| n.to_str() == Some(k))
                .map(|(_, v)| v.to_str().unwrap().to_string())
        };
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.to_str().unwrap()).collect();
        assert_eq!(
            keys,
            [
                "SystemRoot",
                "windir",
                "SystemDrive",
                "ComSpec",
                "PATH",
                "PATHEXT",
                "PSModulePath",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramW6432",
                "CommonProgramFiles",
                "CommonProgramFiles(x86)",
                "CommonProgramW6432",
                "ProgramData",
                "ALLUSERSPROFILE",
                "PUBLIC",
                "OS",
                "PROCESSOR_ARCHITECTURE",
                "PROCESSOR_LEVEL",
                "PROCESSOR_REVISION",
                "NUMBER_OF_PROCESSORS",
                "USERPROFILE",
                "APPDATA",
                "LOCALAPPDATA",
                "TEMP",
                "TMP",
            ]
        );
        assert_eq!(
            get("PATH").unwrap(),
            r"C:\Windows\System32;C:\Windows;C:\Windows\System32\Wbem;C:\Windows\System32\WindowsPowerShell\v1.0"
        );
        assert_eq!(get("ComSpec").unwrap(), r"C:\Windows\System32\cmd.exe");
        assert_eq!(get("SystemDrive").unwrap(), "C:");
        assert_eq!(
            get("PSModulePath").unwrap(),
            r"C:\Windows\System32\WindowsPowerShell\v1.0\Modules"
        );
        assert_eq!(get("PROCESSOR_REVISION").unwrap(), "9e0a");
        assert_eq!(get("TEMP").unwrap(), tmp.to_str().unwrap());
        assert_eq!(get("TMP").unwrap(), tmp.to_str().unwrap());
        // Nothing of this process's environment leaks in (it has USERNAME, a user PATH...).
        for (k, _) in std::env::vars_os() {
            let k = k.to_string_lossy();
            if !keys.iter().any(|n| n.eq_ignore_ascii_case(&k)) {
                assert!(get(&k).is_none(), "{k}");
            }
        }
        assert!(get("HTTP_PROXY").is_none() && get("USERNAME").is_none());
    }

    /// A command line split the way the C runtime splits `argv` (quotes group, `\"` is a
    /// quote, backslashes before a quote halve).
    fn split_command(line: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut cur = String::new();
        let mut in_arg = false;
        let mut quoted = false;
        let mut slashes = 0usize;
        for c in line.chars() {
            match c {
                '\\' => {
                    slashes += 1;
                    in_arg = true;
                    continue;
                }
                '"' => {
                    cur.extend(std::iter::repeat_n('\\', slashes / 2));
                    if slashes % 2 == 1 {
                        cur.push('"');
                    } else {
                        quoted = !quoted;
                    }
                    in_arg = true;
                }
                ' ' | '\t' if !quoted => {
                    cur.extend(std::iter::repeat_n('\\', slashes));
                    if in_arg {
                        args.push(std::mem::take(&mut cur));
                    }
                    in_arg = false;
                }
                _ => {
                    cur.extend(std::iter::repeat_n('\\', slashes));
                    cur.push(c);
                    in_arg = true;
                }
            }
            slashes = 0;
        }
        cur.extend(std::iter::repeat_n('\\', slashes));
        if in_arg {
            args.push(cur);
        }
        args
    }

    /// Task 190, on the path of Task 196: with a log folder holding a space (a user profile
    /// like `C:\Users\Ana Maria`), the log path reaches Inno, Burn and exe installers as one
    /// argument, whether the manifest quoted `<LOGPATH>` or not.
    #[test]
    fn log_placeholder_with_space_stays_one_argument() {
        let log = Path::new(r"C:\Users\Ana Maria\AppData\Local\wingetrs\log\installers\a.log");
        let cases = [
            (Inno, r#"/VERYSILENT /LOG="<LOGPATH>""#, "/LOG="),
            (Inno, "/VERYSILENT /LOG=<LOGPATH>", "/LOG="),
            (Burn, r#"/quiet /log "<LOGPATH>""#, ""),
            (Burn, "/quiet /log <LOGPATH>", ""),
            (Exe, "/S /out=<LOGPATH>", "/out="),
            (Exe, "/S /key:<LOGPATH>", "/key:"),
            (Exe, "/S /log <LOGPATH>", ""),
        ];
        for (kind, params, prefix) in cases {
            let line = switches::fill_log(params, log).unwrap();
            let args = split_command(&line);
            let want = format!("{prefix}{}", log.display());
            assert_eq!(
                args.iter().filter(|a| **a == want).count(),
                1,
                "{kind:?} {params}: {args:?}"
            );
        }
    }
    /// Task 190: the elevated winget gets an environment built from scratch: variables this
    /// process already has (cargo's `CARGO_PKG_NAME`, the user's `USERNAME`, its `TEMP`) do not
    /// reach it, and its `TEMP` is inside a fresh execution folder, removed afterwards. The
    /// process environment is only read, never changed (tests run in parallel).
    #[test]
    fn winget_env_is_built_from_scratch() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let script = dir.path().join("winget.cmd");
        std::fs::write(&script, "@echo off\r\nset\r\n").unwrap();
        let sys = staging_win::SystemEnv::query().unwrap();
        let inherited = ["CARGO_PKG_NAME", "USERNAME"];
        for name in inherited {
            assert!(std::env::var_os(name).is_some(), "{name} missing here");
        }
        let my_temp = std::env::var("TEMP").unwrap();
        let mut lines = Vec::new();
        // Through the helper's own winget entry point (only the exe lookup is left out).
        let (code, _) = wingetrs::bridge::elevate_win::run_winget_at(
            &script,
            &[],
            &root,
            TEST_SDDL,
            &sys,
            &mut |l: &str| lines.push(l.to_string()),
        )
        .unwrap();
        assert_eq!(code, 0, "{lines:?}");
        for name in inherited {
            let prefix = format!("{name}=");
            assert!(
                !lines.iter().any(|l| l.to_uppercase().starts_with(&prefix)),
                "{name} reached the tool: {lines:?}"
            );
        }
        let temp = lines
            .iter()
            .find_map(|l| l.strip_prefix("TEMP="))
            .unwrap_or_else(|| panic!("{lines:?}"));
        assert!(!temp.eq_ignore_ascii_case(&my_temp), "{temp}");
        let temp = Path::new(temp);
        assert_eq!(temp.file_name().unwrap(), staging_win::TMP_NAME);
        let run = temp.parent().unwrap();
        assert_eq!(run.parent().unwrap(), root.as_path(), "{lines:?}");
        assert_eq!(run.file_name().unwrap().len(), 32);
        assert!(entries(&root).is_empty(), "execution folder removed");
    }

    /// Task 202 (mutation 3): a staged installer runs from its run folder, with `TEMP`/`TMP` in
    /// a new `tmp` inside it; a program outside that folder, or another program than the staged
    /// installer, runs nothing.
    #[test]
    fn staged_installer_runs_in_its_run_folder() {
        use wingetrs::bridge::signed_install::Staged;
        let dir = TempDir::new().unwrap();
        let run = dir.path().join("run");
        std::fs::create_dir(&run).unwrap();
        let script =
            "@echo off\r\n(echo %CD%& echo %TEMP%& echo %TMP%)> \"%~dp0seen.txt\"\r\nexit /b 7\r\n";
        let installer = run.join("installer.cmd");
        std::fs::write(&installer, script).unwrap();
        let sys = staging_win::SystemEnv::query().unwrap();
        let staged = Staged {
            program: installer.clone(),
            params: String::new(),
            dir: run.clone(),
            installer: installer.clone(),
            kind: ElevatedKind::Exe,
        };
        let mut lines = Vec::new();
        let end =
            staging_win::run_staged(&staged, &sys, Duration::from_secs(30), &mut |l: &str| {
                lines.push(l.to_string())
            })
            .unwrap();
        assert_eq!(end.code, 7, "{lines:?}");
        let seen = std::fs::read_to_string(run.join("seen.txt")).unwrap();
        let seen: Vec<&str> = seen.lines().map(str::trim_end).collect();
        let tmp = run.join(staging_win::TMP_NAME);
        let same = |a: &str, b: &Path| a.eq_ignore_ascii_case(b.to_str().unwrap());
        assert!(same(seen[0], &run), "{seen:?}");
        assert!(same(seen[1], &tmp), "{seen:?}");
        assert!(same(seen[2], &tmp), "{seen:?}");

        let elsewhere = dir.path().join("installer.cmd");
        std::fs::write(&elsewhere, script).unwrap();
        let outside = Staged {
            program: elsewhere.clone(),
            installer: elsewhere.clone(),
            ..staged.clone()
        };
        let other = Staged {
            program: sys.system32.join("cmd.exe"),
            ..staged.clone()
        };
        for bad in [outside, other] {
            let e = staging_win::run_staged(&bad, &sys, Duration::from_secs(30), &mut |_| {})
                .unwrap_err();
            assert_eq!(
                e.kind(),
                std::io::ErrorKind::PermissionDenied,
                "{bad:?}: {e}"
            );
        }
        assert!(!dir.path().join("seen.txt").exists());
    }

    /// The real verifier refuses a file that is not a signed package, and a catalog-shaped zip
    /// without a signature.
    #[test]
    fn real_verifier_refuses_unsigned_files() {
        use wingetrs::bridge::catalog_verify_win::WinTrustVerifier;
        use wingetrs::bridge::signed_install::CatalogVerifier;
        let dir = TempDir::new().unwrap();
        let junk = dir.path().join("junk.msix");
        std::fs::write(&junk, b"not a package").unwrap();
        let e = WinTrustVerifier.verify(&junk).unwrap_err();
        assert!(e.contains("WinVerifyTrust"), "{e}");
        let zip = dir.path().join("zip.msix");
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("AppxManifest.xml", zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut z, b"<Package/>").unwrap();
            z.finish().unwrap();
        }
        std::fs::write(&zip, buf.into_inner()).unwrap();
        let e = WinTrustVerifier.verify(&zip).unwrap_err();
        assert!(e.contains("WinVerifyTrust"), "{e}");
    }

    /// The real catalog: signature, signer, age and identity as the helper checks them.
    #[test]
    #[ignore = "live: downloads source2.msix from the winget CDN"]
    fn live_real_catalog_verifies() {
        use wingetrs::bridge::catalog_verify_win::WinTrustVerifier;
        use wingetrs::bridge::signed_install::{
            CatalogVerifier, MSIX_MAX, SIGNER_NAME, check_identity, check_signer,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let roots = wingetrs::selfupdate::apply_win::machine_roots().unwrap();
        let http = wingetrs::http::Http::for_elevated(&roots).unwrap();
        let bytes = rt
            .block_on(http.get_limited(wingetrs::sources::winget::SOURCE2_URL, MSIX_MAX))
            .unwrap();
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("source2.msix");
        std::fs::write(&file, &bytes).unwrap();
        let verified = WinTrustVerifier.verify(&file).unwrap();
        assert_eq!(verified.bytes, bytes);
        let signer = verified.signer;
        assert_eq!(signer.common_name, SIGNER_NAME);
        assert_eq!(signer.organization, SIGNER_NAME);
        check_signer(&signer, SystemTime::now()).unwrap();
        check_identity(&bytes).unwrap();
        // One byte changed inside the package: refused.
        let mut bad = bytes.clone();
        let at = bad.len() / 2;
        bad[at] ^= 0x01;
        let file = dir.path().join("bad.msix");
        std::fs::write(&file, &bad).unwrap();
        assert!(WinTrustVerifier.verify(&file).is_err());
    }
}

#[test]
fn worker_allows_progress_switches_exactly() {
    assert!(msi_params_allowed("/passive /norestart"));
    assert!(!msi_params_allowed("/passivex /norestart"));
    assert!(exe_params_allowed(
        ElevatedKind::Inno,
        "/SP- /SILENT /NORESTART",
        &[]
    ));
    assert!(!exe_params_allowed(ElevatedKind::Inno, "/SILENTX", &[]));
    assert!(exe_params_allowed(
        ElevatedKind::Burn,
        "/passive /norestart",
        &[]
    ));
    assert!(!exe_params_allowed(ElevatedKind::Burn, "/passivex", &[]));
}
// ---- Task 202: the helper derives the installer from the signed catalog ----

mod signed {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime};
    use wingetrs::bridge::signed_install::{
        CATALOG_FILE, CATALOG_MAX_AGE, CHECKING_CATALOG, CatalogVerifier, DOWNLOADING_INSTALLER,
        SIGNER_NAME, SOURCE_IDENTITY, SignedCtx, SignedError, Signer, Staged, Verified,
        check_signer, prepare,
    };
    use wingetrs::bridge::worker::{ModeArg, PackageRequest};
    use wingetrs::http::Http;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ID: &str = "Pub.App";
    const REL: &str = "manifests/p/Pub/App/1.0/m";

    /// The installer the CDN serves.
    fn body() -> Vec<u8> {
        (0..70_000).map(|i| (i * 7 % 253) as u8).collect()
    }

    fn sha_hex(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    /// (name, bytes) entries of a zip, deflated.
    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            for (name, bytes) in files {
                z.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(bytes).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    /// A v2 index with `id` -> `hash` only.
    fn index_db(id: &str, hash: &[u8], major: &str) -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("i.db");
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch("create table metadata(name text, value text); create table packages(id text, hash blob);")
            .unwrap();
        c.execute("insert into metadata values('majorVersion', ?1)", [major])
            .unwrap();
        c.execute(
            "insert into packages values(?1, ?2)",
            rusqlite::params![id, hash],
        )
        .unwrap();
        drop(c);
        std::fs::read(&p).unwrap()
    }

    /// A catalog msix: `AppxManifest.xml` with `identity`, the index and its block map (the
    /// index bytes stored are `stored`, the block map describes `db`).
    fn msix_of(db: &[u8], stored: &[u8], identity: &str) -> Vec<u8> {
        let mut map = String::from(r#"<BlockMap><File Name="Public\index.db">"#);
        for c in db.chunks(65536) {
            map.push_str(&format!(
                r#"<Block Hash="{}"/>"#,
                STANDARD.encode(Sha256::digest(c))
            ));
        }
        map.push_str(&format!(
            r#"<FileHash Hash="{}"/></File></BlockMap>"#,
            STANDARD.encode(Sha256::digest(db))
        ));
        let appx = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"><Identity Name="{identity}" Publisher="CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US" Version="2026.1019.2000.5"/></Package>"#
        );
        zip_of(&[
            ("AppxManifest.xml", appx.as_bytes()),
            ("Public/index.db", stored),
            ("AppxBlockMap.xml", map.as_bytes()),
        ])
    }

    /// An MSZIP container of `data` in one block (the `versionData.mszyml` format).
    fn mszip_of(data: &[u8]) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::DeflateEncoder;
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).unwrap();
        let deflate = enc.finish().unwrap();
        let mut out = vec![0x0A, 0x51, 0xE5, 0xC0, 0, 0, 0, 0];
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(&[0u8; 8]);
        out.extend_from_slice(&((deflate.len() + 2) as u32).to_le_bytes());
        out.extend_from_slice(b"CK");
        out.extend_from_slice(&deflate);
        out
    }

    /// The manifest: [0] inno machine, [1] inno user, [2] msi machine, [3] exe machine whose
    /// switch names a path (outside the allowlist), [4] wix machine served over plain http.
    fn manifest_yaml(url: &str, sha: &str, http_url: &str) -> String {
        format!(
            "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Contoso App\nPublisher: Contoso\nInstallers:\n\
             - Architecture: x64\n  InstallerType: inno\n  Scope: machine\n  InstallerUrl: {url}\n  InstallerSha256: {sha}\n\
             - Architecture: x64\n  InstallerType: inno\n  Scope: user\n  InstallerUrl: {url}\n  InstallerSha256: {sha}\n\
             - Architecture: x86\n  InstallerType: msi\n  Scope: machine\n  InstallerUrl: {url}\n  InstallerSha256: {sha}\n\
             - Architecture: arm64\n  InstallerType: exe\n  Scope: machine\n  InstallerUrl: {url}\n  InstallerSha256: {sha}\n  InstallerSwitches:\n    Silent: /S /CONFIG=C:\\x\n    SilentWithProgress: /S /CONFIG=C:\\x\n\
             - Architecture: x64\n  InstallerType: wix\n  Scope: machine\n  InstallerUrl: {http_url}\n  InstallerSha256: {sha}\n"
        )
    }

    /// What the fake CDN serves; each field can be broken by a test.
    struct Cdn {
        msix: Vec<u8>,
        vd: Vec<u8>,
        /// Hash the index gives for `ID` (the `versionData` path is built from it).
        hash: Vec<u8>,
        manifest: Vec<u8>,
        installer: Vec<u8>,
        /// `vD` entries (version, rP, s256H) of `versionData`.
        entries: Vec<(String, String, String)>,
        /// How many times the installer may be fetched (`None`: any).
        installer_hits: Option<u64>,
        identity: String,
        /// What the verifier answers as the verified bytes (`None`: the file's).
        verified: Option<Vec<u8>>,
    }

    impl Cdn {
        /// A consistent catalog for the server at `base` (its `http://` URI).
        fn new(base: &str) -> Cdn {
            let installer = body();
            let sha = sha_hex(&installer);
            let manifest = manifest_yaml(&format!("{base}/setup"), &sha, &format!("{base}/setup"))
                .into_bytes();
            let mut c = Cdn {
                msix: Vec::new(),
                vd: Vec::new(),
                hash: Vec::new(),
                entries: vec![("1.0".into(), REL.into(), sha_hex(&manifest))],
                manifest,
                installer,
                installer_hits: None,
                identity: SOURCE_IDENTITY.into(),
                verified: None,
            };
            c.seal();
            c
        }

        /// `versionData` from `entries`, the index from its hash, the msix from the index.
        fn seal(&mut self) {
            let mut yaml = String::from("vD:\n");
            for (v, rp, s) in &self.entries {
                yaml.push_str(&format!("- v: '{v}'\n  rP: {rp}\n  s256H: {s}\n"));
            }
            self.vd = mszip_of(yaml.as_bytes());
            self.hash = Sha256::digest(&self.vd).to_vec();
            let db = index_db(ID, &self.hash, "2");
            self.msix = msix_of(&db, &db, &self.identity);
        }

        async fn mount(&self, server: &MockServer) {
            let get = |p: String, b: Vec<u8>| {
                Mock::given(method("GET"))
                    .and(path(p))
                    .respond_with(ResponseTemplate::new(200).set_body_bytes(b))
            };
            get(format!("/cdn/{CATALOG_FILE}"), self.msix.clone())
                .mount(server)
                .await;
            get(
                format!(
                    "/cdn/packages/{ID}/{}/versionData.mszyml",
                    hex::encode(&self.hash[..4])
                ),
                self.vd.clone(),
            )
            .mount(server)
            .await;
            get(format!("/cdn/{REL}"), self.manifest.clone())
                .mount(server)
                .await;
            let setup = get("/setup".into(), self.installer.clone());
            match self.installer_hits {
                Some(n) => setup.expect(n).mount(server).await,
                None => setup.mount(server).await,
            }
        }
    }

    fn microsoft(now: SystemTime) -> Signer {
        Signer {
            common_name: SIGNER_NAME.into(),
            organization: SIGNER_NAME.into(),
            not_before: now - Duration::from_secs(24 * 60 * 60),
        }
    }

    /// Answers `result` and records each file it was given with its bytes at that moment.
    struct FakeVerifier {
        result: Result<Signer, String>,
        seen: Mutex<Vec<(PathBuf, Vec<u8>)>>,
        /// Bytes answered as verified instead of the file's (what its handle would have read).
        bytes: Option<Vec<u8>>,
    }

    impl FakeVerifier {
        fn new(result: Result<Signer, String>) -> FakeVerifier {
            FakeVerifier {
                result,
                seen: Mutex::new(Vec::new()),
                bytes: None,
            }
        }
    }

    impl CatalogVerifier for FakeVerifier {
        fn verify(&self, msix: &Path) -> Result<Verified, String> {
            let bytes = std::fs::read(msix).unwrap_or_default();
            self.seen
                .lock()
                .unwrap()
                .push((msix.to_path_buf(), bytes.clone()));
            let bytes = self.bytes.clone().unwrap_or(bytes);
            self.result.clone().map(|signer| Verified { signer, bytes })
        }
    }

    fn request() -> PackageRequest {
        PackageRequest {
            id: ID.into(),
            version: "1.0".into(),
            installer: 0,
            scope: ScopeArg::Machine,
            mode: ModeArg::SilentWithProgress,
            upgrade: false,
        }
    }

    const MSIEXEC: &str = r"C:\Windows\System32\msiexec.exe";

    /// One `prepare` run.
    struct Run {
        result: Result<Staged, SignedError>,
        dir: tempfile::TempDir,
        lines: Vec<String>,
        verifier: FakeVerifier,
        server: MockServer,
    }

    impl Run {
        fn names(&self) -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(self.dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }
    }

    /// Runs `prepare` for `req` against `cdn` (after `edit` saw the server's base URI) with a
    /// verifier answering `signer` (`None`: Microsoft's), at `now`, http installers `allow_http`.
    async fn run_with(
        edit: impl FnOnce(&mut Cdn, &str),
        req: PackageRequest,
        signer: Option<Result<Signer, String>>,
        allow_http: bool,
    ) -> Run {
        let server = MockServer::start().await;
        let mut cdn = Cdn::new(&server.uri());
        edit(&mut cdn, &server.uri());
        cdn.mount(&server).await;
        let now = SystemTime::now();
        let mut verifier = FakeVerifier::new(signer.unwrap_or_else(|| Ok(microsoft(now))));
        verifier.bytes = cdn.verified.clone();
        let dir = tempfile::tempdir().unwrap();
        let http = Http::new().unwrap();
        let base = format!("{}/cdn/", server.uri());
        let pf = roots();
        let msiexec = PathBuf::from(MSIEXEC);
        let ctx = SignedCtx {
            http: &http,
            cdn_base: &base,
            verifier: &verifier,
            now,
            program_files: &pf,
            msiexec: &msiexec,
            allow_http,
        };
        let mut lines = Vec::new();
        let result = prepare(&ctx, &req, dir.path(), &mut |l: &str| {
            lines.push(l.to_string())
        })
        .await;
        Run {
            result,
            dir,
            lines,
            verifier,
            server,
        }
    }

    async fn run(req: PackageRequest) -> Run {
        run_with(|_, _| {}, req, None, true).await
    }

    /// The installer at the request's position runs from the run folder, with the command line
    /// of the manifest's switches; the catalog went through the verifier as the file the helper
    /// wrote there; the catalog files are gone and the folder holds only the installer.
    #[tokio::test]
    async fn signed_catalog_stages_the_manifest_installer() {
        let r = run(request()).await;
        let staged = r.result.as_ref().unwrap();
        let dir = r.dir.path();
        assert_eq!(staged.dir, dir);
        assert_eq!(staged.installer, dir.join("installer.exe"));
        assert_eq!(staged.program, staged.installer);
        assert_eq!(staged.kind, ElevatedKind::Inno);
        assert_eq!(std::fs::read(&staged.installer).unwrap(), body());
        assert!(staged.params.contains("/SILENT"), "{}", staged.params);
        assert!(
            !staged.params.to_ascii_lowercase().contains("/log"),
            "{}",
            staged.params
        );
        assert_eq!(r.names(), ["installer.exe"]);
        let seen = r.verifier.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, dir.join(CATALOG_FILE));
        let served = Cdn::new(&r.server.uri()).msix;
        assert_eq!(seen[0].1, served, "the verifier saw the downloaded catalog");
        assert_eq!(r.lines, [CHECKING_CATALOG, DOWNLOADING_INSTALLER]);

        // An MSI: the system msiexec with `/i` on the copy in the run folder.
        let r = run(PackageRequest {
            installer: 2,
            ..request()
        })
        .await;
        let staged = r.result.as_ref().unwrap();
        assert_eq!(staged.program, Path::new(MSIEXEC));
        assert_eq!(staged.installer, r.dir.path().join("installer.msi"));
        let want = format!("/i \"{}\"", staged.installer.display());
        assert!(staged.params.starts_with(&want), "{}", staged.params);
        assert!(staged.params.contains("/passive"), "{}", staged.params);
    }

    /// Mutation 1: a catalog whose signature the verifier refuses stops everything: no
    /// installer downloaded, nothing staged, the run folder left empty.
    #[tokio::test]
    async fn refused_signature_downloads_no_installer() {
        let r = run_with(
            |c, _| c.installer_hits = Some(0),
            request(),
            Some(Err("assinatura inválida".into())),
            true,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(matches!(e, SignedError::Signature(_)), "{e:?}");
        assert!(e.to_string().contains("assinatura inválida"), "{e}");
        assert!(r.names().is_empty(), "{:?}", r.names());
        assert_eq!(r.lines, [CHECKING_CATALOG]);
        drop(r.server); // verifies `expect(0)` on the installer
    }

    /// Each check of the signer has a case that fails only on it.
    #[test]
    fn signer_name_and_age() {
        let now = SystemTime::now();
        assert!(check_signer(&microsoft(now), now).is_ok());
        let other = |cn: &str, o: &str| Signer {
            common_name: cn.into(),
            organization: o.into(),
            ..microsoft(now)
        };
        for s in [
            other("Microsoft Windows", SIGNER_NAME),
            other(SIGNER_NAME, "Contoso"),
            other("microsoft corporation", SIGNER_NAME),
        ] {
            let e = check_signer(&s, now).unwrap_err();
            assert!(matches!(e, SignedError::Signature(_)), "{s:?}: {e:?}");
        }
        let at = |age: Duration| Signer {
            not_before: now - age,
            ..microsoft(now)
        };
        assert!(check_signer(&at(CATALOG_MAX_AGE - Duration::from_secs(60)), now).is_ok());
        let e = check_signer(&at(CATALOG_MAX_AGE + Duration::from_secs(60)), now).unwrap_err();
        assert!(e.to_string().contains("mais de 7 dias"), "{e}");
        // A certificate dated after `now` (clock behind) passes.
        let future = Signer {
            not_before: now + Duration::from_secs(3600),
            ..microsoft(now)
        };
        assert!(check_signer(&future, now).is_ok());
    }

    /// The signer check runs on what the verifier answered: an old or foreign signer stops
    /// the flow before the installer.
    #[tokio::test]
    async fn old_or_foreign_signer_stops_the_flow() {
        let now = SystemTime::now();
        for signer in [
            Signer {
                not_before: now - CATALOG_MAX_AGE - Duration::from_secs(3600),
                ..microsoft(now)
            },
            Signer {
                common_name: "Contoso".into(),
                ..microsoft(now)
            },
        ] {
            let r = run_with(
                |c, _| c.installer_hits = Some(0),
                request(),
                Some(Ok(signer.clone())),
                true,
            )
            .await;
            let e = r.result.as_ref().unwrap_err();
            assert!(matches!(e, SignedError::Signature(_)), "{signer:?}: {e:?}");
            assert!(r.names().is_empty(), "{:?}", r.names());
        }
    }

    /// Another package signed by Microsoft is not the winget catalog.
    #[tokio::test]
    async fn foreign_identity_is_refused() {
        let r = run_with(
            |c, _| {
                c.identity = "Microsoft.Other.Package".into();
                c.installer_hits = Some(0);
                c.seal();
            },
            request(),
            None,
            true,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(matches!(e, SignedError::Signature(_)), "{e:?}");
        assert!(e.to_string().contains("Microsoft.Other.Package"), "{e}");

        // The identity is read from the bytes the signature was verified on, not from the
        // path again: a good catalog on disk does not save a foreign one that was verified.
        let r = run_with(
            |c, _| {
                let good = c.msix.clone();
                c.identity = "Microsoft.Other.Package".into();
                c.seal();
                c.verified = Some(std::mem::replace(&mut c.msix, good));
                c.installer_hits = Some(0);
            },
            request(),
            None,
            true,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(e.to_string().contains("Microsoft.Other.Package"), "{e}");
    }

    /// The index must match the block map (the signed part): a changed index is refused.
    #[tokio::test]
    async fn index_outside_the_block_map_is_refused() {
        let r = run_with(
            |c, _| {
                let db = index_db(ID, &c.hash, "2");
                let other = index_db(ID, &[7u8; 32], "2");
                c.msix = msix_of(&db, &other, SOURCE_IDENTITY);
                c.installer_hits = Some(0);
            },
            request(),
            None,
            true,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(matches!(e, SignedError::Catalog(_)), "{e:?}");
    }

    #[tokio::test]
    async fn package_missing_from_the_index_is_refused() {
        let r = run(PackageRequest {
            id: "Other.App".into(),
            ..request()
        })
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(matches!(e, SignedError::NotInCatalog(_)), "{e:?}");
        // An index of another major version.
        let r = run_with(
            |c, _| {
                let db = index_db(ID, &c.hash, "3");
                c.msix = msix_of(&db, &db, SOURCE_IDENTITY);
            },
            request(),
            None,
            true,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(e.to_string().contains("índice v3"), "{e}");
    }

    /// `versionData` and the manifest are each checked against the hash that leads to them.
    #[tokio::test]
    async fn version_data_and_manifest_are_checked_by_hash() {
        // versionData served is not the one the index names.
        let r = run_with(
            |c, _| {
                let extra = mszip_of(b"vD: []\n");
                c.vd = extra;
                c.installer_hits = Some(0);
            },
            request(),
            None,
            true,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(e.to_string().contains("versionData"), "{e}");
        // The manifest served differs from its `s256H`.
        let r = run_with(
            |c, _| {
                c.manifest.extend_from_slice(b"# changed\n");
                c.installer_hits = Some(0);
            },
            request(),
            None,
            true,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(matches!(e, SignedError::Integrity(_)), "{e:?}");
        assert!(e.to_string().contains("manifesto"), "{e}");
    }

    /// Versions compare as versions: `1.0.0` finds `1.0`; with both listed the one written as
    /// asked wins; a missing version is refused.
    #[tokio::test]
    async fn version_is_found_by_version_not_text() {
        let r = run(PackageRequest {
            version: "1.0.0".into(),
            ..request()
        })
        .await;
        assert!(r.result.is_ok(), "{:?}", r.result);
        let r = run(PackageRequest {
            version: "1.1".into(),
            ..request()
        })
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(matches!(e, SignedError::VersionMissing(_)), "{e:?}");
        // `1.0.0` listed first with a broken path, `1.0` after it: asking `1.0` takes `1.0`.
        let r = run_with(
            |c, _| {
                let good = c.entries[0].clone();
                c.entries = vec![("1.0.0".into(), "../x".into(), good.2.clone()), good];
                c.seal();
            },
            request(),
            None,
            true,
        )
        .await;
        assert!(r.result.is_ok(), "{:?}", r.result);
        // And asking `1.0.0` takes the `1.0.0` entry, whose path is refused.
        let r = run_with(
            |c, _| {
                let good = c.entries[0].clone();
                c.entries = vec![("1.0.0".into(), "../x".into(), good.2.clone()), good];
                c.seal();
            },
            PackageRequest {
                version: "1.0.0".into(),
                ..request()
            },
            None,
            true,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(e.to_string().contains("caminho do manifesto"), "{e}");
    }

    /// Mutation 2: the SHA-256 that decides is the manifest's. A served installer that differs
    /// from it is refused and removed, whatever the request carried.
    #[tokio::test]
    async fn installer_not_matching_the_manifest_is_refused() {
        let r = run_with(
            |c, _| c.installer = b"MZ planted".to_vec(),
            request(),
            None,
            true,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(matches!(e, SignedError::Integrity(_)), "{e:?}");
        assert!(e.to_string().contains("SHA-256 do manifesto"), "{e}");
        assert!(r.names().is_empty(), "{:?}", r.names());
    }

    /// The request names only a position: one past the end, a user-scope installer, one whose
    /// switches leave the allowlist, or one on plain http are refused before any download.
    #[tokio::test]
    async fn only_elevated_allowlisted_https_installers() {
        let cases = [
            (5, "inexistente"),
            (1, "não roda como administrador"),
            (3, "fora da lista permitida"),
        ];
        for (installer, why) in cases {
            let r = run_with(
                |c, _| c.installer_hits = Some(0),
                PackageRequest {
                    installer,
                    ..request()
                },
                None,
                true,
            )
            .await;
            let e = r.result.as_ref().unwrap_err();
            assert!(matches!(e, SignedError::Refused(_)), "{installer}: {e:?}");
            assert!(e.to_string().contains(why), "{installer}: {e}");
            assert!(r.names().is_empty(), "{:?}", r.names());
        }
        let r = run_with(
            |c, _| c.installer_hits = Some(0),
            PackageRequest {
                installer: 4,
                ..request()
            },
            None,
            false,
        )
        .await;
        let e = r.result.as_ref().unwrap_err();
        assert!(e.to_string().contains("sem https"), "{e}");
        // The same installer when plain http is allowed (only the tests' server).
        let r = run(PackageRequest {
            installer: 4,
            ..request()
        })
        .await;
        assert!(r.result.is_ok(), "{:?}", r.result);
    }

    /// A request that fails its own check never reaches the network.
    #[tokio::test]
    async fn invalid_request_reaches_nothing() {
        for req in [
            PackageRequest {
                id: "..\\x".into(),
                ..request()
            },
            PackageRequest {
                version: r"1.0\..\x".into(),
                ..request()
            },
            PackageRequest {
                scope: ScopeArg::User,
                ..request()
            },
        ] {
            let r = run_with(|c, _| c.installer_hits = Some(0), req.clone(), None, true).await;
            let e = r.result.as_ref().unwrap_err();
            assert!(matches!(e, SignedError::Request(_)), "{req:?}: {e:?}");
            assert!(r.verifier.seen.lock().unwrap().is_empty());
        }
    }

    /// Equivalence (two paths, one command line): the helper's command line for the plan's
    /// request is the plan's own, with and without the `Upgrade` switch.
    #[tokio::test]
    async fn helper_command_line_equals_the_plan() {
        use wingetrs::winget_install::ops::package_request;
        use wingetrs::winget_install::plan::{PlanOrBridge, command_for, plan};
        use wingetrs::winget_install::select::{Host, Scope, ScopeWanted};
        let server = MockServer::start().await;
        let base = server.uri();
        let mut cdn = Cdn::new(&base);
        let sha = sha_hex(&cdn.installer);
        let yaml = format!(
            "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Contoso App\nPublisher: Contoso\nInstallers:\n\
             - Architecture: x64\n  InstallerType: exe\n  Scope: machine\n  InstallerUrl: {base}/setup\n  InstallerSha256: {sha}\n  InstallerSwitches:\n    Silent: /S\n    SilentWithProgress: /S /progress\n    Upgrade: /upgrade\n"
        );
        cdn.manifest = yaml.clone().into_bytes();
        cdn.entries = vec![("1.0".into(), REL.into(), sha_hex(yaml.as_bytes()))];
        cdn.seal();
        cdn.mount(&server).await;
        let m = wingetrs::sources::winget::manifest::parse(yaml.as_bytes()).unwrap();
        let host = Host {
            arch: "x64".into(),
            os: [10, 0, 22631, 0],
            locale: "pt-BR".into(),
        };
        let p = match plan(
            &m,
            &host,
            ScopeWanted::Prefer(Scope::Machine),
            Some("0.9".into()),
        )
        .unwrap()
        {
            PlanOrBridge::Native(p) => p,
            other => panic!("{other:?}"),
        };
        let mut fresh = p.clone();
        fresh.command =
            Some(command_for(&fresh.installer, fresh.mode, fresh.elevation, false).unwrap());
        for (plan, upgrade) in [(&p, true), (&fresh, false)] {
            let cmd = plan.command.as_ref().unwrap();
            let req = package_request(plan, cmd).unwrap();
            assert_eq!(req.upgrade, upgrade, "{}", cmd.args);
            let now = SystemTime::now();
            let verifier = FakeVerifier::new(Ok(microsoft(now)));
            let dir = tempfile::tempdir().unwrap();
            let http = Http::new().unwrap();
            let cdn_base = format!("{base}/cdn/");
            let pf = roots();
            let msiexec = PathBuf::from(MSIEXEC);
            let ctx = SignedCtx {
                http: &http,
                cdn_base: &cdn_base,
                verifier: &verifier,
                now,
                program_files: &pf,
                msiexec: &msiexec,
                allow_http: true,
            };
            let staged = prepare(&ctx, &req, dir.path(), &mut |_| {}).await.unwrap();
            assert_eq!(staged.params, cmd.args);
        }
        assert!(p.command.as_ref().unwrap().args.contains("/upgrade"));
    }

    /// The protocol: `install_package` reaches the operations with the request's fields, its
    /// lines and exit code go back; a field the request may not carry (the old `sha256`)
    /// closes the channel with nothing run; a request failing its check is answered without
    /// running anything.
    #[derive(Default)]
    struct InstallOps {
        requests: Vec<PackageRequest>,
    }

    impl WorkerOps for InstallOps {
        fn run(
            &mut self,
            _program: &str,
            _args: &[String],
            _on_line: &mut dyn FnMut(&str),
        ) -> std::io::Result<(i32, Vec<String>)> {
            panic!("no winget here")
        }
        fn install_package(
            &mut self,
            req: &PackageRequest,
            on_line: &mut dyn FnMut(&str),
        ) -> std::io::Result<i32> {
            self.requests.push(req.clone());
            on_line(CHECKING_CATALOG);
            Ok(3010)
        }
    }

    fn serve_install(input: &str) -> (std::io::Result<()>, Vec<Reply>, Vec<PackageRequest>) {
        let mut ops = InstallOps::default();
        let mut out = Vec::new();
        let r = worker::serve(&mut BufReader::new(input.as_bytes()), &mut out, &mut ops);
        let replies = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| worker::decode::<Reply>(l).unwrap())
            .collect();
        (r, replies, ops.requests)
    }

    fn install_line(req: &PackageRequest) -> String {
        line(&Request::InstallPackage {
            id: req.id.clone(),
            version: req.version.clone(),
            installer: req.installer,
            scope: req.scope,
            mode: req.mode,
            upgrade: req.upgrade,
        })
    }

    #[test]
    fn install_package_protocol() {
        let (r, replies, reqs) = serve_install(&install_line(&request()));
        r.unwrap();
        assert_eq!(reqs, [request()]);
        assert_eq!(
            replies,
            [
                Reply::Line {
                    text: CHECKING_CATALOG.into()
                },
                Reply::End { code: 3010 }
            ]
        );

        // Mutation 2: the old field, carrying the SHA-256 of what the requester wants run.
        let with_sha = install_line(&request())
            .trim_end()
            .trim_end_matches('}')
            .to_string()
            + &format!(",\"sha256\":\"{}\"}}\n", sha_hex(b"MZ planted"));
        assert!(
            with_sha.contains("\"op\":\"install_package\""),
            "{with_sha}"
        );
        let (r, replies, reqs) = serve_install(&with_sha);
        assert!(r.is_err(), "{replies:?}");
        assert!(
            replies.is_empty() && reqs.is_empty(),
            "{replies:?} {reqs:?}"
        );

        let bad = PackageRequest {
            scope: ScopeArg::User,
            ..request()
        };
        let (r, replies, reqs) = serve_install(&install_line(&bad));
        r.unwrap();
        assert!(reqs.is_empty(), "{reqs:?}");
        assert!(
            matches!(&replies[..], [Reply::Error { message, .. }] if message.contains("todos os usuários")),
            "{replies:?}"
        );
    }
}
