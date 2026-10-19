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
        (Burn, "/quiet PrependPath=1"),
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
