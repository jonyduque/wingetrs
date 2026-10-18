//! Elevated helper runs a verified installer (Task 180).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::{BufReader, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wingetrs::bridge::elevated::{
    ElevatedBridge, ElevatedError, ElevatedRunner, Helper, Transport,
};
use wingetrs::bridge::worker::{
    self, ElevatedKind,
    ElevatedKind::{Burn, Exe, Inno, Nullsoft},
    INSTALLER_LINE_MAX, INSTALLER_LINES, InstallerRequest, Reply, Request, ScopeArg, WorkerOps,
    exe_params_allowed, msi_params_allowed, msi_params_allowed_in,
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

/// Operations that record every call; installers report `lines` and exit with `code`.
#[derive(Default)]
struct FakeOps {
    runs: Vec<(String, Vec<String>)>,
    installers: Vec<InstallerRequest>,
    lines: Vec<String>,
    code: i32,
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

    fn run_installer(
        &mut self,
        req: &InstallerRequest,
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<i32> {
        self.installers.push(req.clone());
        for l in &self.lines {
            on_line(l);
        }
        Ok(self.code)
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

#[test]
fn run_installer_reaches_ops_with_fields() {
    let mut ops = FakeOps {
        lines: vec!["primeira".into(), "segunda".into()],
        ..FakeOps::default()
    };
    let (r, replies) = serve_with(&line(&request(&valid())), &mut ops);
    r.unwrap();
    assert_eq!(ops.installers, vec![valid()]);
    assert!(ops.runs.is_empty());
    assert_eq!(
        replies,
        vec![
            Reply::Line {
                text: "primeira".into()
            },
            Reply::Line {
                text: "segunda".into()
            },
            Reply::End { code: 0 },
        ]
    );
}

#[test]
fn installer_lines_are_the_last_forty_and_clipped() {
    let mut lines: Vec<String> = (1..=50).map(|i| format!("linha {i}")).collect();
    lines.push("é".repeat(INSTALLER_LINE_MAX));
    let mut ops = FakeOps {
        lines,
        code: 1603,
        ..FakeOps::default()
    };
    let (r, replies) = serve_with(&line(&request(&valid())), &mut ops);
    r.unwrap();
    assert_eq!(replies.len(), INSTALLER_LINES + 1);
    assert_eq!(
        replies[0],
        Reply::Line {
            text: "linha 12".into()
        }
    );
    let Reply::Line { text } = &replies[INSTALLER_LINES - 1] else {
        panic!("{:?}", replies[INSTALLER_LINES - 1]);
    };
    assert!(text.len() <= INSTALLER_LINE_MAX && text.len() > INSTALLER_LINE_MAX - 2);
    assert!(text.chars().all(|c| c == 'é'));
    assert_eq!(replies[INSTALLER_LINES], Reply::End { code: 1603 });
}

#[test]
fn bad_fields_never_reach_ops() {
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
    let mut input: String = bad.iter().map(|r| line(&request(r))).collect();
    // Control: the last line is valid at every limit and does run.
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
    input.push_str(&line(&request(&edge)));
    let mut ops = FakeOps::default();
    assert_eq!(edge.source.len(), worker::SOURCE_MAX);
    let (r, replies) = serve_with(&input, &mut ops);
    r.unwrap();
    assert_eq!(ops.installers, vec![edge], "only the control ran");
    assert_eq!(replies.len(), bad.len() + 1, "{replies:?}");
    for (reply, req) in replies.iter().zip(&bad) {
        assert!(
            matches!(
                reply,
                Reply::Error {
                    not_found: false,
                    ..
                }
            ),
            "{req:?}: {reply:?}"
        );
    }
    for req in &bad {
        assert!(req.check(&roots()).is_err(), "{req:?}");
    }
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
        (Inno, "/VERYSILENT /TYPE=full"),
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
/// `serve` stops at `bad` without running it nor the valid request after it.
fn stops_at(bad: &str) {
    let input = format!("{bad}\n{}", line(&request(&valid())));
    let mut ops = FakeOps::default();
    let (r, replies) = serve_with(&input, &mut ops);
    assert!(r.is_err(), "{bad}");
    assert!(ops.installers.is_empty() && ops.runs.is_empty(), "{bad}");
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
    assert!(ops.installers.is_empty());
    let one = [
        Reply::Line {
            text: "winget".into(),
        },
        Reply::End { code: 7 },
    ];
    assert_eq!(replies, [one.clone(), one].concat());
}

/// A helper played by a thread running `serve` with `FakeOps` over in-memory pipes.
struct FakeHelper {
    reader: BufReader<std::io::PipeReader>,
    writer: Box<dyn Write + Send>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeHelper {
    fn spawn(seen: Arc<Mutex<Vec<InstallerRequest>>>) -> FakeHelper {
        let (req_r, req_w) = std::io::pipe().unwrap();
        let (resp_r, mut resp_w) = std::io::pipe().unwrap();
        let thread = std::thread::spawn(move || {
            let mut ops = FakeOps {
                lines: vec!["log do instalador".into()],
                code: 3010,
                ..FakeOps::default()
            };
            let _ = worker::serve(&mut BufReader::new(req_r), &mut resp_w, &mut ops);
            seen.lock().unwrap().extend(ops.installers);
        });
        FakeHelper {
            reader: BufReader::new(resp_r),
            writer: Box::new(req_w),
            thread: Some(thread),
        }
    }
}

impl Helper for FakeHelper {
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

impl Drop for FakeHelper {
    fn drop(&mut self) {
        self.writer = Box::new(std::io::sink());
        if let Some(t) = self.thread.take()
            && let Err(p) = t.join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(p);
        }
    }
}

#[derive(Clone, Default)]
struct FakeTransport {
    starts: Arc<AtomicUsize>,
    refuse: bool,
    seen: Arc<Mutex<Vec<InstallerRequest>>>,
}

impl Transport for FakeTransport {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.refuse {
            return Err("recusado".into());
        }
        Ok(Box::new(FakeHelper::spawn(Arc::clone(&self.seen))))
    }
}

#[test]
fn refused_runner_does_not_retry() {
    let t = FakeTransport {
        refuse: true,
        ..FakeTransport::default()
    };
    let bridge = ElevatedBridge::new(Box::new(t.clone()));
    let mut lines = Vec::new();
    for _ in 0..2 {
        let r = bridge.run_installer(&valid(), &mut |l: &str| lines.push(l.to_string()));
        assert_eq!(r, Err(ElevatedError::Refused));
    }
    assert_eq!(
        t.starts.load(Ordering::SeqCst),
        1,
        "one UAC prompt, never again"
    );
    assert!(lines.is_empty());
}

#[test]
fn runner_relays_lines_code_and_remote_refusals() {
    let t = FakeTransport::default();
    let bridge = ElevatedBridge::new(Box::new(t.clone()));
    let mut lines = Vec::new();
    let r = bridge.run_installer(&valid(), &mut |l: &str| lines.push(l.to_string()));
    assert_eq!(r, Ok(3010));
    assert_eq!(lines, ["log do instalador"]);
    let mut bad = valid();
    bad.sha256 = "x".into();
    let r = bridge.run_installer(&bad, &mut |_: &str| {});
    assert!(
        matches!(&r, Err(ElevatedError::Remote(m)) if m.contains("hash")),
        "{r:?}"
    );
    assert_eq!(t.starts.load(Ordering::SeqCst), 1);
    assert!(bridge.shutdown(Duration::from_secs(5)));
    drop(bridge);
    assert_eq!(*t.seen.lock().unwrap(), vec![valid()]);
}

#[cfg(windows)]
mod win {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    use std::time::SystemTime;
    use tempfile::TempDir;
    use wingetrs::bridge::staging_win::{self, HASH_MISMATCH, RUN_SDDL, RunDir};
    use wingetrs::winget_install::launch::{LaunchSpec, Launcher, Show};
    use wingetrs::winget_install::launch_win::ShellLauncher;
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

    /// `bytes` saved as the download cache does: `<dir>\blobs\<sha256>`.
    fn blob(dir: &Path, bytes: &[u8]) -> PathBuf {
        let sha: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let blobs = dir.join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        let path = blobs.join(sha);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn system32() -> PathBuf {
        staging_win::SystemEnv::query().unwrap().system32
    }

    fn exe_request(source: &Path, params: &str) -> InstallerRequest {
        InstallerRequest {
            source: source.to_str().unwrap().into(),
            sha256: sha_of(source),
            file_name: "setup.exe".into(),
            kind: ElevatedKind::Exe,
            params: params.into(),
        }
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
            })
            .unwrap();
        assert_eq!(code, 3);
        assert!(dir.path().join("ran.txt").is_file());
        drop(held);
    }

    #[test]
    fn copy_then_verify_rejects_tampered_source() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let source = blob(dir.path(), b"MZ instalador falso");
        let mut req = exe_request(&source, "/S");
        let right = req.sha256.clone();
        let other = dir.path().join("other");
        std::fs::write(&other, b"MZ outro instalador").unwrap();
        req.sha256 = sha_of(&other);
        let e = staging_win::stage(&root, TEST_SDDL, &req).unwrap_err();
        assert_eq!(e.to_string(), HASH_MISMATCH);
        assert!(entries(&root).is_empty(), "the execution folder is gone");

        req.sha256 = right.to_uppercase();
        let staged = staging_win::stage(&root, TEST_SDDL, &req).unwrap();
        let run = staged.run_dir().to_path_buf();
        assert_eq!(staged.file(), run.join("setup.exe"));
        assert_eq!(run.parent(), Some(root.as_path()));
        assert_eq!(
            std::fs::read(staged.file()).unwrap(),
            b"MZ instalador falso"
        );
        assert!(run.join("tmp").is_dir());
        assert!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(staged.file())
                .is_err(),
            "the copy is held open denying writes"
        );
        drop(staged);
        assert!(entries(&root).is_empty(), "removed after the run");
        assert!(source.is_file(), "the user's file is left alone");
    }

    #[test]
    fn copy_is_capped() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("blob");
        std::fs::write(&source, b"12345").unwrap();
        let sha = sha_of(&source);
        let e = staging_win::copy_hashed(&source, &dir.path().join("a"), &sha, 4).unwrap_err();
        assert!(e.to_string().contains("maior que 4"), "{e}");
        staging_win::copy_hashed(&source, &dir.path().join("b"), &sha, 5).unwrap();
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

    #[test]
    fn open_source_refuses_devices_and_reparse_points() {
        // A device is never read (here the open itself fails on the null device).
        assert!(staging_win::open_source(Path::new(r"\\.\NUL")).is_err());
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        junction(&dir.path().join("link"), &target);
        assert!(staging_win::open_source(&dir.path().join("link")).is_err());
        let file = blob(dir.path(), b"x");
        let link = dir.path().join("file-link");
        // A file symlink needs Developer Mode or the privilege; checked when available.
        if std::os::windows::fs::symlink_file(&file, &link).is_ok() {
            let e = staging_win::open_source(&link).unwrap_err();
            assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");
            assert!(e.to_string().contains("reanálise"), "{e}");
        } else {
            eprintln!("symlink_file indisponível: só a junção foi conferida");
        }
        staging_win::open_source(&file).unwrap();
    }
    #[test]
    fn log_lines_read_utf16_and_cut_the_partial_first_line() {
        let dir = TempDir::new().unwrap();
        let utf16 = dir.path().join("msi.log");
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(
            "um\r\ndois\r\n\r\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        std::fs::write(&utf16, bytes).unwrap();
        assert_eq!(staging_win::log_lines(&utf16, 40).unwrap(), ["um", "dois"]);

        let long = dir.path().join("inno.log");
        let text: String = (0..2000).map(|i| format!("linha {i:04}\r\n")).collect();
        std::fs::write(&long, &text).unwrap();
        let lines = staging_win::log_lines(&long, 40).unwrap();
        assert_eq!(lines.len(), 40);
        assert_eq!(lines[39], "linha 1999");
        assert_eq!(lines[0], "linha 1960");
        let all = staging_win::log_lines(&long, usize::MAX).unwrap();
        assert!(
            all.iter().all(|l| l.len() == 10),
            "no partial line: {:?}",
            all[0]
        );
        assert!(
            staging_win::log_lines(&dir.path().join("none.log"), 40)
                .unwrap()
                .is_empty()
        );
    }

    /// The full helper path minus elevation: a copy of `cmd.exe` as the "installer" reports
    /// which file ran, its `TEMP`/`TMP` and its working folder through `<LOGPATH>`.
    #[test]
    fn staged_installer_runs_the_copy_with_its_own_temp() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let source = blob(
            dir.path(),
            &std::fs::read(system32().join("cmd.exe")).unwrap(),
        );
        let src = source.parent().unwrap().to_path_buf();
        let req = exe_request(
            &source,
            // `!CMDCMDLINE!` (delayed expansion) is the command line as started, quotes and all.
            r#"/v /c (echo !CMDCMDLINE!& set TEMP& set TMP& cd& set USERNAME& set PATH)> "<LOGPATH>""#,
        );
        // Staging is tested here, not validation: `cmd` needs free parameters.
        let sys = staging_win::SystemEnv::query().unwrap();
        let staged = staging_win::stage(&root, TEST_SDDL, &req).unwrap();
        let run = staged.run_dir().to_path_buf();
        let copy = staged.file().to_path_buf();
        let mut lines = Vec::new();
        let code = staging_win::run_staged(
            &staged,
            &req,
            Path::new(r"C:\nao-usado\msiexec.exe"),
            &sys,
            &mut |l: &str| lines.push(l.to_string()),
        )
        .unwrap();
        assert_eq!(code, 0, "{lines:?}");
        let low = |s: &str| s.to_lowercase();
        let cmdline = lines.first().unwrap_or_else(|| panic!("{lines:?}"));
        assert!(
            low(cmdline).starts_with(&low(&format!("\"{}\" ", copy.display()))),
            "the copy ran: {cmdline}"
        );
        assert!(
            !low(cmdline).contains(&low(src.to_str().unwrap())),
            "{cmdline}"
        );
        let tmp = run.join("tmp");
        let want = |k: &str| format!("{k}={}", tmp.display());
        assert!(
            lines.iter().any(|l| low(l) == low(&want("TEMP"))),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| low(l) == low(&want("TMP"))),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| low(l) == low(run.to_str().unwrap())),
            "working folder: {lines:?}"
        );
        // Only the closed list reached it: no USERNAME, and the system PATH alone.
        assert!(
            !lines
                .iter()
                .any(|l| l.to_uppercase().starts_with("USERNAME=")),
            "{lines:?}"
        );
        let path = staging_win::installer_env(&sys, &tmp)
            .into_iter()
            .find(|(k, _)| k == "PATH")
            .unwrap()
            .1;
        let want_path = format!("PATH={}", path.to_str().unwrap());
        assert!(lines.iter().any(|l| low(l) == low(&want_path)), "{lines:?}");
        drop(staged);
        assert!(entries(&root).is_empty());
    }

    #[test]
    fn staged_installer_sends_the_last_forty_log_lines() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let source = blob(
            dir.path(),
            &std::fs::read(system32().join("cmd.exe")).unwrap(),
        );
        let req = exe_request(
            &source,
            r#"/c (for /l %i in (1,1,50) do @echo linha%i)> "<LOGPATH>" & exit /b 5"#,
        );
        let staged = staging_win::stage(&root, TEST_SDDL, &req).unwrap();
        let mut lines = Vec::new();
        let code = staging_win::run_staged(
            &staged,
            &req,
            Path::new(r"C:\nao-usado\msiexec.exe"),
            &staging_win::SystemEnv::query().unwrap(),
            &mut |l: &str| lines.push(l.trim().to_string()),
        )
        .unwrap();
        assert_eq!(code, 5);
        let want: Vec<String> = (11..=50).map(|i| format!("linha{i}")).collect();
        assert_eq!(lines, want);
    }

    #[test]
    fn msi_command_runs_msiexec_on_the_copy() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let source = blob(dir.path(), b"OLE falso");
        let req = InstallerRequest {
            source: source.to_str().unwrap().into(),
            sha256: sha_of(&source),
            file_name: "App-1.0.msi".into(),
            kind: ElevatedKind::Wix,
            params: r#"/quiet /norestart /log "<LOGPATH>""#.into(),
        };
        req.check(&[]).unwrap();
        let staged = staging_win::stage(&root, TEST_SDDL, &req).unwrap();
        let msiexec = Path::new(r"C:\Windows\System32\msiexec.exe");
        let (program, params) = staging_win::command_for(&staged, &req, msiexec).unwrap();
        assert_eq!(program, msiexec);
        let run = staged.run_dir();
        assert_eq!(
            params,
            format!(
                r#"/i "{}" /quiet /norestart /log "{}""#,
                run.join("App-1.0.msi").display(),
                run.join("install.log").display()
            )
        );
    }

    /// Live: the real helper (one UAC prompt) installs the small MSI in `WINGETRS_TEST_MSI`.
    #[test]
    #[ignore = "live: pede UAC e instala um MSI de teste"]
    fn live_run_installer_once() {
        let msi = PathBuf::from(
            std::env::var_os("WINGETRS_TEST_MSI")
                .expect("WINGETRS_TEST_MSI=<caminho de um MSI pequeno>"),
        );
        // The helper only reads `...\blobs\<sha256>`, as the download cache names it.
        let cache = TempDir::new().unwrap();
        let msi = blob(cache.path(), &std::fs::read(&msi).unwrap());
        let transport = wingetrs::bridge::elevated::SystemTransport::new(PathBuf::from(env!(
            "CARGO_BIN_EXE_wingetrs"
        )));
        let bridge = ElevatedBridge::new(Box::new(transport));
        let req = InstallerRequest {
            source: msi.to_str().unwrap().into(),
            sha256: sha_of(&msi),
            file_name: "teste.msi".into(),
            kind: ElevatedKind::Msi,
            params: r#"/quiet /norestart /log "<LOGPATH>""#.into(),
        };
        let mut lines = Vec::new();
        let r = bridge.run_installer(&req, &mut |l: &str| lines.push(l.to_string()));
        println!("{r:?}\n{}", lines.join("\n"));
        assert_eq!(r, Ok(0));
        assert!(!lines.is_empty(), "log lines");
        assert!(bridge.shutdown(Duration::from_secs(5)));
        let root = PathBuf::from(r"C:\Program Files\wingetrs\staging");
        let left: Vec<_> = std::fs::read_dir(&root).unwrap().collect();
        assert!(left.is_empty(), "{left:?}");
    }
}
