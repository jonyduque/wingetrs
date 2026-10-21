//! Task 213: `wingetrs self update`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use wingetrs::http::Http;
use wingetrs::selfupdate::{self, Outcome, SelfUpdateError, Version};
use wingetrs::winget_install::launch::{LaunchSpec, Launcher, Verb};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn v(major: u64, minor: u64, patch: u64) -> Version {
    Version {
        major,
        minor,
        patch,
    }
}

fn sum_line(body: &[u8]) -> String {
    format!("{}  wingetrs.exe\n", hex::encode(Sha256::digest(body)))
}

/// `<tmp>/app/wingetrs.exe` with `old` as content.
fn fake_exe(tmp: &Path, old: &[u8]) -> PathBuf {
    let dir = tmp.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("wingetrs.exe");
    std::fs::write(&exe, old).unwrap();
    exe
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

async fn serve(s: &MockServer, tag: &str, exe: &[u8], sum: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/{tag}/wingetrs.exe")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(exe.to_vec()))
        .mount(s)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{tag}/wingetrs.exe.sha256")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/octet-stream")
                .set_body_string(sum),
        )
        .mount(s)
        .await;
}

#[test]
fn newer_compares_numbers() {
    assert_eq!(selfupdate::newer("0.9.9", "v0.10.0"), Some(v(0, 10, 0)));
    assert_eq!(selfupdate::newer("0.2.1", "0.2.2"), Some(v(0, 2, 2)));
    assert_eq!(selfupdate::newer("1.9.0", "v1.10.0"), Some(v(1, 10, 0)));
    // Older and same (also with extra leading zeros) are not newer.
    assert_eq!(selfupdate::newer("0.10.0", "v0.9.9"), None);
    assert_eq!(selfupdate::newer("0.2.1", "v0.2.1"), None);
    assert_eq!(selfupdate::newer("0.2.1", "v0.02.001"), None);
    // Tags outside `v?X.Y.Z` never compare.
    for tag in [
        "latest",
        "v1.2",
        "v1.2.3.4",
        "1.2.3-rc1",
        "V1.2.3",
        " v1.2.3",
        "v1.2.x",
        "v1..3",
        "vv1.2.3",
        "v١.2.3",
        "",
    ] {
        assert_eq!(selfupdate::newer("0.0.1", tag), None, "{tag:?}");
    }
    assert_eq!(selfupdate::newer("not-a-version", "v9.9.9"), None);
    assert_eq!(v(0, 2, 2).to_string(), "0.2.2");
}

#[tokio::test]
async fn apply_replaces_exe() {
    let s = MockServer::start().await;
    let new = b"MZ new wingetrs".to_vec();
    serve(&s, "v0.2.2", &new, &sum_line(&new)).await;
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ old");
    let h = Http::new().unwrap();
    selfupdate::apply(&h, &s.uri(), "v0.2.2", &exe)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), new);
    let dir = exe.parent().unwrap();
    assert_eq!(names(dir), ["wingetrs.exe", "wingetrs.exe.old"]);
    assert_eq!(
        std::fs::read(selfupdate::old_path(&exe)).unwrap(),
        b"MZ old"
    );
    // A second update replaces the previous `.old`.
    let newer = b"MZ newer".to_vec();
    serve(&s, "v0.2.3", &newer, &sum_line(&newer)).await;
    selfupdate::apply(&h, &s.uri(), "v0.2.3", &exe)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), newer);
    assert_eq!(names(dir), ["wingetrs.exe", "wingetrs.exe.old"]);
    assert_eq!(std::fs::read(selfupdate::old_path(&exe)).unwrap(), new);
    // Startup cleanup removes the `.old`, silently when there is none.
    selfupdate::remove_old(&exe);
    selfupdate::remove_old(&exe);
    assert_eq!(names(dir), ["wingetrs.exe"]);
}

#[tokio::test]
async fn apply_hash_mismatch_keeps_exe() {
    let s = MockServer::start().await;
    let new = b"MZ new wingetrs".to_vec();
    serve(&s, "v0.2.2", &new, &sum_line(b"something else")).await;
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ old");
    let h = Http::new().unwrap();
    let r = selfupdate::apply(&h, &s.uri(), "v0.2.2", &exe).await;
    assert!(matches!(r, Err(SelfUpdateError::HashMismatch)), "{r:?}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old");
    // The download happened (the temporary existed) and was deleted.
    assert!(
        s.received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path() == "/v0.2.2/wingetrs.exe")
    );
    assert_eq!(names(exe.parent().unwrap()), ["wingetrs.exe"]);
}

#[tokio::test]
async fn apply_failed_download_leaves_no_temporary() {
    let s = MockServer::start().await;
    let new = b"MZ new".to_vec();
    Mock::given(method("GET"))
        .and(path("/v0.2.2/wingetrs.exe.sha256"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sum_line(&new)))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0.2.2/wingetrs.exe"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&s)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ old");
    let h = Http::new().unwrap();
    let r = selfupdate::apply(&h, &s.uri(), "v0.2.2", &exe).await;
    assert!(r.is_err(), "{r:?}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old");
    assert_eq!(names(exe.parent().unwrap()), ["wingetrs.exe"]);
}

#[tokio::test]
async fn apply_refuses_executable_over_the_cap() {
    let s = MockServer::start().await;
    let new = b"MZ 0123456789".to_vec();
    serve(&s, "v0.2.2", &new, &sum_line(&new)).await;
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ old");
    let h = Http::new().unwrap();
    let r = selfupdate::apply_capped(&h, &s.uri(), "v0.2.2", &exe, 8, &|_| {}).await;
    assert!(matches!(r, Err(SelfUpdateError::Other(_))), "{r:?}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old");
    assert_eq!(names(exe.parent().unwrap()), ["wingetrs.exe"]);
    // At the cap exactly it goes through.
    let cap = new.len() as u64;
    selfupdate::apply_capped(&h, &s.uri(), "v0.2.2", &exe, cap, &|_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), new);
}

#[tokio::test]
async fn apply_uses_pinned_tag_paths() {
    let s = MockServer::start().await;
    let new = b"MZ new".to_vec();
    serve(&s, "v0.2.2", &new, &sum_line(&new)).await;
    // A `latest/download` asset that would win if it were asked for.
    serve(&s, "latest/download", b"MZ evil", &sum_line(b"MZ evil")).await;
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ old");
    let h = Http::new().unwrap();
    selfupdate::apply(&h, &s.uri(), "v0.2.2", &exe)
        .await
        .unwrap();
    let mut paths: Vec<String> = s
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_string())
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        ["/v0.2.2/wingetrs.exe", "/v0.2.2/wingetrs.exe.sha256"]
    );
    assert_eq!(std::fs::read(&exe).unwrap(), new);
}

#[tokio::test]
async fn apply_refuses_bad_tag_without_request() {
    let s = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ old");
    let h = Http::new().unwrap();
    for tag in ["latest", "../v1.2.3", "v1.2"] {
        let r = selfupdate::apply(&h, &s.uri(), tag, &exe).await;
        assert!(matches!(r, Err(SelfUpdateError::BadTag(_))), "{tag}: {r:?}");
    }
    assert!(s.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn bad_sum_file_refused() {
    let new = b"MZ new".to_vec();
    let good = hex::encode(Sha256::digest(&new));
    let cases = [
        "not a hash  wingetrs.exe\n".to_string(),
        format!("{}  wingetrs.exe\n", &good[..63]),
        format!("{good}0  wingetrs.exe\n"),
        format!("{}  wingetrs.exe\n", good.replace(&good[..1], "g")),
        format!("{good}  other.exe\n"),
        String::new(),
    ];
    for sum in cases {
        let s = MockServer::start().await;
        serve(&s, "v0.2.2", &new, &sum).await;
        let tmp = tempfile::tempdir().unwrap();
        let exe = fake_exe(tmp.path(), b"MZ old");
        let h = Http::new().unwrap();
        let r = selfupdate::apply(&h, &s.uri(), "v0.2.2", &exe).await;
        assert!(
            matches!(r, Err(SelfUpdateError::BadSumFile)),
            "{sum:?}: {r:?}"
        );
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old");
        assert_eq!(names(exe.parent().unwrap()), ["wingetrs.exe"]);
    }
    // Accepted forms: bare hash, uppercase, `*name` (binary mode), CRLF.
    for sum in [
        format!("{good}\n"),
        format!("{}  wingetrs.exe\r\n", good.to_uppercase()),
        format!("{good} *wingetrs.exe"),
    ] {
        assert_eq!(
            selfupdate::parse_sum(sum.as_bytes()).map(hex::encode).ok(),
            Some(good.clone()),
            "{sum:?}"
        );
    }
}

#[test]
fn dev_build_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let release = tmp.path().join("repo").join("target").join("release");
    std::fs::create_dir_all(&release).unwrap();
    let exe = release.join("wingetrs.exe");
    std::fs::write(&exe, b"MZ").unwrap();
    assert!(selfupdate::refuse_dev_build(&exe).is_ok());
    std::fs::write(tmp.path().join("repo").join("Cargo.toml"), b"[package]").unwrap();
    let r = selfupdate::refuse_dev_build(&exe);
    assert!(matches!(r, Err(SelfUpdateError::DevBuild)), "{r:?}");
    assert_eq!(
        r.unwrap_err().to_string(),
        "build de desenvolvimento: atualize com cargo"
    );
}

struct FakeLauncher {
    code: std::io::Result<i32>,
    specs: Mutex<Vec<LaunchSpec>>,
    events: Mutex<Vec<&'static str>>,
}

impl FakeLauncher {
    fn new(code: std::io::Result<i32>) -> FakeLauncher {
        FakeLauncher {
            code,
            specs: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        }
    }
}

impl Launcher for FakeLauncher {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32> {
        self.specs.lock().unwrap().push(spec.clone());
        self.events.lock().unwrap().push("run");
        match &self.code {
            Ok(c) => Ok(*c),
            Err(e) => Err(std::io::Error::from_raw_os_error(e.raw_os_error().unwrap())),
        }
    }
}

#[test]
fn elevated_scope_uses_runas_self_apply() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ old");
    let l = FakeLauncher::new(Ok(21));
    let notice = Mutex::new(Vec::new());
    let r = selfupdate::elevated_apply(&l, &exe, &mut || {
        notice.lock().unwrap().push("notice");
        l.events.lock().unwrap().push("notice");
    });
    assert!(matches!(r, Err(SelfUpdateError::HashMismatch)), "{r:?}");
    let specs = l.specs.lock().unwrap();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].verb, Verb::RunAs);
    assert_eq!(specs[0].program, exe);
    assert_eq!(specs[0].params, "self apply-elevated");
    assert_eq!(specs[0].dir.as_deref(), exe.parent());
    // The elevation notice comes before the UAC prompt.
    assert_eq!(*l.events.lock().unwrap(), ["notice", "run"]);
    assert_eq!(notice.lock().unwrap().len(), 1);
    // Every exit code of the elevated process, and a refused UAC prompt.
    let cases: [(std::io::Result<i32>, &str); 6] = [
        (Ok(0), "Updated"),
        (Ok(10), "AlreadyLatest"),
        (Ok(20), "Offline"),
        (Ok(21), "HashMismatch"),
        (Ok(22), "Elevation(22)"),
        (
            Err(std::io::Error::from_raw_os_error(1223)),
            "ElevationRefused",
        ),
    ];
    for (code, want) in cases {
        let l = FakeLauncher::new(code);
        let got = match selfupdate::elevated_apply(&l, &exe, &mut || {}) {
            Ok(o) => format!("{o:?}"),
            Err(e) => format!("{e:?}"),
        };
        assert_eq!(got, want);
    }
    assert_eq!(names(exe.parent().unwrap()), ["wingetrs.exe"]);
}

#[tokio::test]
async fn elevated_side_fetches_tag_and_answers_by_exit_code() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jonyduque/wingetrs/releases/latest"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", "/jonyduque/wingetrs/releases/tag/v0.3.0"),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/jonyduque/wingetrs/releases/tag/v0.3.0"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
        .mount(&s)
        .await;
    let new = b"MZ v0.3.0".to_vec();
    serve(&s, "v0.3.0", &new, &sum_line(&new)).await;
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ old");
    let h = Http::new().unwrap();
    let dl = s.uri();
    // Already on the latest: nothing downloaded.
    let r = selfupdate::elevated_run(&h, &s.uri(), &dl, &exe, "0.3.0").await;
    assert_eq!(selfupdate::exit_code(&r), 10);
    assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old");
    let r = selfupdate::elevated_run(&h, &s.uri(), &dl, &exe, "0.2.1").await;
    assert_eq!(selfupdate::exit_code(&r), 0, "{r:?}");
    assert_eq!(std::fs::read(&exe).unwrap(), new);
    // The codes map back to what the unelevated side says.
    for code in [0, 10, 20, 21, 22, 7] {
        let back = selfupdate::from_exit_code(code);
        let again = selfupdate::exit_code(&back);
        assert_eq!(again, if code == 7 { 22 } else { code }, "{code}");
    }
    assert!(matches!(
        selfupdate::from_exit_code(0),
        Ok(Outcome::Updated)
    ));
    // Offline web base: code 20.
    let r = selfupdate::elevated_run(&h, "http://127.0.0.1:9", &dl, &exe, "0.2.1").await;
    assert_eq!(selfupdate::exit_code(&r), 20, "{r:?}");
}

#[test]
fn cli_parse_self_update() {
    use clap::{CommandFactory, Parser};
    use wingetrs::cli::{Cli, Command, SelfAction};
    let cli = Cli::try_parse_from(["wingetrs", "self", "update", "--check"]).unwrap();
    assert_eq!(
        cli.command,
        Some(Command::SelfCmd {
            action: SelfAction::Update {
                check: true,
                yes: false
            }
        })
    );
    let cli = Cli::try_parse_from(["wingetrs", "self", "update", "--yes"]).unwrap();
    assert_eq!(
        cli.command,
        Some(Command::SelfCmd {
            action: SelfAction::Update {
                check: false,
                yes: true
            }
        })
    );
    let cli = Cli::try_parse_from(["wingetrs", "self", "apply-elevated"]).unwrap();
    assert_eq!(
        cli.command,
        Some(Command::SelfCmd {
            action: SelfAction::ApplyElevated
        })
    );
    // `apply-elevated` takes no argument at all.
    assert!(Cli::try_parse_from(["wingetrs", "self", "apply-elevated", "v9.9.9"]).is_err());
    assert!(Cli::try_parse_from(["wingetrs", "self", "apply-elevated", "--tag", "v9"]).is_err());
    let mut root = Cli::command();
    let help = root
        .find_subcommand_mut("self")
        .unwrap()
        .render_help()
        .to_string();
    assert!(help.contains("update"), "{help}");
    assert!(!help.contains("apply-elevated"), "{help}");
}

const TEST_ROOT: &[u8] = include_bytes!("../fixtures/selfupdate/test-root.der");

#[test]
fn elevated_http_has_no_proxy_and_only_given_roots() {
    // Control: the ordinary client keeps the system/environment proxy.
    assert!(Http::new().unwrap().may_use_proxy());
    let h = Http::for_elevated(&[TEST_ROOT.to_vec()]).unwrap();
    assert!(!h.may_use_proxy());
    assert_eq!(h.pinned_roots(), Some(1));
    // The download copy `apply` uses keeps both.
    let dl = h.with_strict_download_redirects().unwrap();
    assert!(!dl.may_use_proxy());
    assert_eq!(dl.pinned_roots(), Some(1));
    assert_eq!(Http::new().unwrap().pinned_roots(), None);
    // Unusable roots are skipped; none usable is an error.
    let h = Http::for_elevated(&[b"garbage".to_vec(), TEST_ROOT.to_vec()]).unwrap();
    assert_eq!(h.pinned_roots(), Some(1));
    let r = Http::for_elevated(&[b"garbage".to_vec()]);
    assert!(
        matches!(r, Err(wingetrs::http::HttpError::UntrustedCertificate(_))),
        "{:?}",
        r.err()
    );
}

#[cfg(windows)]
#[test]
fn machine_roots_read_only_local_machine_stores() {
    use wingetrs::selfupdate::apply_win::{LOCAL_MACHINE, machine_roots_with};
    let calls = Mutex::new(Vec::new());
    let read = |location: u32, name: &str| -> std::io::Result<Vec<Vec<u8>>> {
        calls.lock().unwrap().push((location, name.to_string()));
        Ok(match name {
            "Root" => vec![b"a".to_vec(), b"b".to_vec()],
            _ => vec![b"b".to_vec(), b"c".to_vec()],
        })
    };
    let roots = machine_roots_with(&read).unwrap();
    assert_eq!(roots, [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    assert_eq!(
        *calls.lock().unwrap(),
        [
            (LOCAL_MACHINE, "Root".to_string()),
            (LOCAL_MACHINE, "AuthRoot".to_string())
        ]
    );
    // One store missing is skipped; none read is an error.
    let one = |_: u32, name: &str| -> std::io::Result<Vec<Vec<u8>>> {
        match name {
            "Root" => Ok(vec![b"a".to_vec()]),
            _ => Err(std::io::Error::other("no store")),
        }
    };
    assert_eq!(machine_roots_with(&one).unwrap(), [b"a".to_vec()]);
    let none = |_: u32, _: &str| -> std::io::Result<Vec<Vec<u8>>> {
        Err(std::io::Error::other("no store"))
    };
    assert!(machine_roots_with(&none).is_err());
    let empty = |_: u32, _: &str| -> std::io::Result<Vec<Vec<u8>>> { Ok(Vec::new()) };
    assert!(machine_roots_with(&empty).is_err());
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "live: reads the machine certificate stores and reaches github.com"]
async fn live_elevated_client_reaches_github() {
    let roots = wingetrs::selfupdate::apply_win::machine_roots().unwrap();
    let h = Http::for_elevated(&roots).unwrap();
    let tag = selfupdate::latest_tag(&h, selfupdate::WEB_BASE)
        .await
        .unwrap();
    assert!(selfupdate::parse_tag(&tag).is_some(), "{tag}");
    // Only a root that is not GitHub's: the chain fails with the clear error.
    let h = Http::for_elevated(&[TEST_ROOT.to_vec()]).unwrap();
    let r = selfupdate::latest_tag(&h, selfupdate::WEB_BASE).await;
    assert!(matches!(r, Err(SelfUpdateError::UntrustedRoot(_))), "{r:?}");
    assert!(
        r.unwrap_err()
            .to_string()
            .starts_with("o Windows não tem a raiz do GitHub no repositório da máquina"),
    );
}

#[test]
fn swap_failure_puts_old_exe_back() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ old");
    let dir = exe.parent().unwrap().to_path_buf();
    let temp = dir.join("wingetrs.exe.download-0123");
    std::fs::write(&temp, b"MZ new").unwrap();
    let calls = Mutex::new(Vec::new());
    let rename = |from: &Path, to: &Path| -> std::io::Result<()> {
        calls.lock().unwrap().push(from.to_path_buf());
        if from == temp.as_path() {
            return Err(std::io::Error::other("in use"));
        }
        std::fs::rename(from, to)
    };
    let r = selfupdate::swap_with(&temp, &exe, &rename);
    assert!(matches!(r, Err(SelfUpdateError::Io(_))), "{r:?}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old");
    assert_eq!(names(&dir), ["wingetrs.exe"]);
    assert_eq!(calls.lock().unwrap().len(), 3);
    // The first rename failing: nothing moved, the temporary deleted.
    std::fs::write(&temp, b"MZ new").unwrap();
    let fail_all = |_: &Path, _: &Path| -> std::io::Result<()> { Err(std::io::Error::other("x")) };
    assert!(selfupdate::swap_with(&temp, &exe, &fail_all).is_err());
    assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old");
    assert_eq!(names(&dir), ["wingetrs.exe"]);
}

#[test]
fn startup_cleanup_removes_only_stale_downloads_of_this_exe() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = fake_exe(tmp.path(), b"MZ");
    let dir = exe.parent().unwrap().to_path_buf();
    std::fs::write(dir.join("wingetrs.exe.download-aa"), b"x").unwrap();
    std::fs::write(dir.join("wingetrs.exe.old"), b"x").unwrap();
    std::fs::write(dir.join("other.exe.download-bb"), b"x").unwrap();
    std::fs::create_dir(dir.join("wingetrs.exe.download-dir")).unwrap();
    // Fresh (a download may still be writing it): kept.
    selfupdate::startup_cleanup(&exe);
    assert_eq!(
        names(&dir),
        [
            "other.exe.download-bb",
            "wingetrs.exe",
            "wingetrs.exe.download-aa",
            "wingetrs.exe.download-dir"
        ]
    );
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2 * 3600);
    selfupdate::remove_stale_downloads(&exe, later);
    assert_eq!(
        names(&dir),
        [
            "other.exe.download-bb",
            "wingetrs.exe",
            "wingetrs.exe.download-dir"
        ]
    );
}
