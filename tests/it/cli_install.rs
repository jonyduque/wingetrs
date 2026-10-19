#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(windows)]
//! `install`/`uninstall`/`upgrade`/`repair`/`bucket` through the real binary: a tempdir as
//! `%LOCALAPPDATA%`, a pre-built catalog with a fake bucket, `wiremock` for the download,
//! and the test registry key / Start Menu folder so nothing real is touched.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::{HttpEntry, HttpState, now_unix};
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::core::SourceKind;
use wingetrs::install::layout::Layout;
use wingetrs::install::state::StateDb;
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

/// The smallest MSI the own extractor refuses: files outside the package (word count 0).
fn uncompressed_msi() -> Vec<u8> {
    use msi::{Package, PackageType};
    let mut p = Package::create(PackageType::Installer, std::io::Cursor::new(Vec::new())).unwrap();
    p.summary_info_mut().set_word_count(0);
    p.into_inner().unwrap().into_inner()
}

/// Deletes this test's own registry subkey on drop, also when an assertion panics.
struct OwnKey(String);

impl Drop for OwnKey {
    fn drop(&mut self) {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(&self.0);
    }
}

pub(crate) struct Lab {
    _t: TempDir,
    local: PathBuf,
    pub(crate) root: PathBuf,
    uri: String,
    key: OwnKey,
}

impl Lab {
    /// `solo` exists only in `main` (served by `server`); `x` exists in `main` and `extras`;
    /// `m` (only in `main`) is an MSI the own extractor refuses.
    pub(crate) async fn new(server: &MockServer, tag: &str) -> Lab {
        let t = tempfile::tempdir().unwrap();
        let local = t.path().to_path_buf();
        let root = local.join("wingetrs");
        let uri = server.uri();
        let zip = zip_of(&[("pkg/solo.exe", &fake_pe(3))]);
        let sha = hex::encode(Sha256::digest(&zip));
        Mock::given(method("GET"))
            .and(path("/solo-1.0.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
            .mount(server)
            .await;
        let manifest = |id: &str| {
            serde_json::json!({
                "version": "1.0",
                "url": format!("{uri}/{id}-1.0.zip"),
                "hash": sha,
                "extract_dir": "pkg",
                "bin": format!("{id}.exe"),
            })
            .to_string()
        };
        let pkg = |id: &str| NewPackage {
            id: id.into(),
            name: id.into(),
            version: "1.0".into(),
            raw: Some(manifest(id)),
            ..Default::default()
        };
        std::fs::create_dir_all(&root).unwrap();
        let mut catalog = Catalog::open(&root.join("catalog.db")).unwrap();
        let put = |c: &mut Catalog, bucket: &str, pkgs: &[NewPackage]| {
            c.replace_source(
                &format!("scoop:{bucket}"),
                SourceKind::Scoop,
                "v",
                1,
                pkgs,
                &WingetMaps::default(),
            )
            .unwrap();
        };
        let msi = uncompressed_msi();
        let msi_sha = hex::encode(Sha256::digest(&msi));
        Mock::given(method("GET"))
            .and(path("/m-1.0.msi"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(msi))
            .mount(server)
            .await;
        let m = NewPackage {
            id: "m".into(),
            name: "m".into(),
            version: "1.0".into(),
            raw: Some(
                serde_json::json!({
                    "version": "1.0",
                    "url": format!("{uri}/m-1.0.msi"),
                    "hash": msi_sha,
                })
                .to_string(),
            ),
            ..Default::default()
        };
        put(&mut catalog, "main", &[pkg("solo"), pkg("x"), m]);
        put(&mut catalog, "extras", &[pkg("x")]);
        drop(catalog);
        // Fresh http.json: no detached background refresh may rewrite the catalog mid-test.
        let mut state = HttpState::default();
        let fresh = HttpEntry {
            checked_at: now_unix(),
            ..Default::default()
        };
        for u in [
            format!("{uri}/source2.msix"),
            format!("{uri}/main.tgz"),
            format!("{uri}/extras.tgz"),
        ] {
            state.entries.insert(u, fresh.clone());
        }
        state.save(&root.join("http.json")).unwrap();
        // The shim launcher is already there (it would otherwise be downloaded from GitHub).
        let layout = Layout::new(&root);
        std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
        std::fs::write(layout.launcher(), fake_pe(3)).unwrap();
        let key = OwnKey(format!(
            r"Software\wingetrs-test\env-cli-{}-{tag}",
            std::process::id()
        ));
        Lab {
            _t: t,
            local,
            root,
            uri,
            key,
        }
    }

    pub(crate) fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_wingetrs"))
            .args(args)
            .stdin(Stdio::null())
            .env("LOCALAPPDATA", &self.local)
            .env("WINGETRS_TEST_ENV_KEY", &self.key.0)
            .env("WINGETRS_TEST_SHORTCUT_DIR", self.local.join("startmenu"))
            .env(
                "WINGETRS_WINGET_INDEX_URL",
                format!("{}/source2.msix", self.uri),
            )
            .env("WINGETRS_SCOOP_BASE_URL", &self.uri)
            .output()
            .unwrap()
    }

    /// Relative path -> file length (`None` for directories) for everything under the root
    /// except the log folder, which every run writes.
    fn snapshot(&self) -> BTreeMap<String, Option<u64>> {
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Option<u64>>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                // The log folder is written by every run; `catalog.db-shm/-wal` appear when
                // the catalog is first opened, whatever the command does.
                if rel == "log" || rel.starts_with("log/") || rel.starts_with("catalog.db-") {
                    continue;
                }
                if p.is_dir() {
                    out.insert(rel, None);
                    walk(&p, root, out);
                } else {
                    out.insert(rel, Some(e.metadata().unwrap().len()));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.root, &self.root, &mut out);
        out
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_prints_the_plan_and_leaves_the_root_untouched() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "dry").await;
    // One throwaway run creates whatever folders every run creates (data dir, log).
    assert!(
        tokio::task::block_in_place(|| lab.run(&["bucket", "list"]))
            .status
            .success()
    );
    let before = lab.snapshot();
    let out = tokio::task::block_in_place(|| lab.run(&["install", "solo", "--dry-run", "--yes"]));
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert!(out.status.success(), "{se}");
    assert!(so.contains("Instalar solo 1.0"), "{so}");
    assert!(so.contains("Simulação"), "{so}");
    assert_eq!(lab.snapshot(), before, "dry-run changed the root");
    assert!(
        !lab.root.join("state.db").exists(),
        "dry-run created state.db"
    );
    // Same for the other two mutating commands.
    let out = tokio::task::block_in_place(|| lab.run(&["upgrade", "--all", "--dry-run", "--yes"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(lab.snapshot(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn ambiguous_target_without_a_terminal_fails_listing_bucket_ids() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "amb").await;
    let before = lab.snapshot();
    for args in [&["install", "x", "--yes"][..], &["install", "x"][..]] {
        let out = tokio::task::block_in_place(|| lab.run(args));
        let se = text(&out.stderr);
        assert!(!out.status.success(), "{args:?}");
        assert!(se.contains("main/x") && se.contains("extras/x"), "{se}");
        assert!(!se.contains("panicked"), "{se}");
    }
    assert_eq!(lab.snapshot().get("apps"), before.get("apps"));
    assert!(!lab.root.join("apps").join("x").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_target_suggests_and_fails() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "nf").await;
    let out = tokio::task::block_in_place(|| lab.run(&["install", "sol", "--yes"]));
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("nenhum pacote encontrado"),
        "{}",
        text(&out.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn confirmation_without_a_terminal_is_refused_without_yes() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "noyes").await;
    let out = tokio::task::block_in_place(|| lab.run(&["install", "solo"]));
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("--yes"), "{}", text(&out.stderr));
    assert!(!lab.root.join("apps").join("solo").exists());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test (test key, deleted on exit)"]
async fn install_then_uninstall_restores_the_root() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "cycle").await;
    assert!(
        tokio::task::block_in_place(|| lab.run(&["bucket", "list"]))
            .status
            .success()
    );
    let before = lab.snapshot();

    let out = tokio::task::block_in_place(|| lab.run(&["install", "solo", "--yes"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("Instalado: solo 1.0"),
        "{}",
        text(&out.stdout)
    );
    assert!(lab.root.join("apps").join("solo").join("1.0").exists());
    assert!(lab.root.join("shims").join("solo.exe").exists());

    let out = tokio::task::block_in_place(|| lab.run(&["upgrade"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("Nada para atualizar"),
        "{}",
        text(&out.stdout)
    );

    let out = tokio::task::block_in_place(|| lab.run(&["uninstall", "solo", "--yes"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("Desinstalado: solo 1.0"),
        "{}",
        text(&out.stdout)
    );

    // The brief's list, plus `install.lock` (the lock file stays by design) and the now
    // empty `apps` folder, which uninstall does not remove.
    let ignored = [
        "blobs",
        "catalog.db",
        "search.idx",
        "http.json",
        "state.db",
        "log",
        "install.lock",
    ];
    let strip = |m: BTreeMap<String, Option<u64>>| -> BTreeMap<String, Option<u64>> {
        m.into_iter()
            .filter(|(k, _)| {
                // Only the (empty) folder itself: anything inside `apps` must be gone.
                k != "apps"
                    && !ignored
                        .iter()
                        .any(|i| k == i || k.starts_with(&format!("{i}/")))
            })
            .collect()
    };
    assert_eq!(strip(lab.snapshot()), strip(before));
    let state = StateDb::open(&lab.root.join("state.db")).unwrap();
    assert!(state.apps().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn repair_on_an_empty_root_reports_nothing_to_do() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "repair").await;
    let out = tokio::task::block_in_place(|| lab.run(&["repair"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("Reconstruídos: nenhum"),
        "{}",
        text(&out.stdout)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bucket_add_list_rm_round_trip() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "bucket").await;
    let ok = |args: &[&str]| {
        let o = tokio::task::block_in_place(|| lab.run(args));
        assert!(o.status.success(), "{args:?}: {}", text(&o.stderr));
        text(&o.stdout)
    };
    // One aligned row per bucket: name, then repository.
    let has = |listed: &str, row: [&str; 2]| listed.lines().any(|l| l.split_whitespace().eq(row));
    let listed = ok(&["bucket", "list"]);
    assert!(has(&listed, ["main", "ScoopInstaller/Main"]), "{listed}");
    // A repository that is not well known needs --yes without a terminal.
    let o =
        tokio::task::block_in_place(|| lab.run(&["bucket", "add", "mine", "someone/scoop-mine"]));
    assert!(!o.status.success());
    ok(&["bucket", "add", "mine", "someone/scoop-mine", "--yes"]);
    assert!(has(
        &ok(&["bucket", "list"]),
        ["mine", "someone/scoop-mine"]
    ));
    ok(&["bucket", "rm", "mine"]);
    assert!(!ok(&["bucket", "list"]).contains("mine"));
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_dry_run_leaves_the_root_untouched_and_takes_no_lock() {
    use wingetrs::install::state::AppRecord;
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "udry").await;
    {
        let mut db = StateDb::open(&lab.root.join("state.db")).unwrap();
        db.record(
            &AppRecord {
                app: "solo".into(),
                bucket: "main".into(),
                version: "1.0".into(),
                arch: None,
                installed_at: 1,
                manifest_sha256: "00".into(),
            },
            &[],
        )
        .unwrap();
    }
    assert!(
        tokio::task::block_in_place(|| lab.run(&["bucket", "list"]))
            .status
            .success()
    );
    let before = lab.snapshot();
    let out = tokio::task::block_in_place(|| lab.run(&["uninstall", "solo", "--dry-run", "--yes"]));
    let so = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        so.contains("Desinstalar solo 1.0") && so.contains("Simulação"),
        "{so}"
    );
    assert_eq!(lab.snapshot(), before);
    assert!(!lab.root.join("install.lock").exists());
}

/// Two apps installed at 0.9 whose catalog version is 1.0, so both are upgradable.
fn install_old(lab: &Lab, apps: &[&str]) {
    use wingetrs::install::state::AppRecord;
    let mut db = StateDb::open(&lab.root.join("state.db")).unwrap();
    for app in apps {
        db.record(
            &AppRecord {
                app: (*app).into(),
                bucket: "main".into(),
                version: "0.9".into(),
                arch: None,
                installed_at: 1,
                manifest_sha256: "00".into(),
            },
            &[],
        )
        .unwrap();
        let cur = lab.root.join("apps").join(app).join("current");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::write(cur.join("install.json"), b"{}").unwrap();
    }
}

fn pin(lab: &Lab, key: &str) {
    pins::set(&wingetrs::cache::DataDir::new(&lab.root), key, true).unwrap();
}

use wingetrs::pins;

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_all_skips_pinned() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "pinall").await;
    install_old(&lab, &["solo", "x"]);
    pin(&lab, &pins::key("wingetrs", "solo"));
    // `x` exists in two buckets in the catalog; the state says `main`, so it resolves.
    let out = tokio::task::block_in_place(|| lab.run(&["upgrade", "--all", "--dry-run", "--yes"]));
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert!(out.status.success(), "{se}");
    assert!(so.contains("fixado, pulado: solo"), "{so}");
    assert!(so.contains("x 1.0"), "{so}");
    assert!(!so.contains("solo 1.0"), "{so}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_upgrade_pinned_app_refuses() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "pinone").await;
    install_old(&lab, &["solo"]);
    pin(&lab, &pins::key("wingetrs", "solo"));
    let out = tokio::task::block_in_place(|| lab.run(&["upgrade", "solo", "--yes"]));
    assert!(!out.status.success());
    assert!(
        text(&out.stderr)
            .contains("solo está com a versão fixada\n→ desafixe na TUI (menu de ações)"),
        "{}",
        text(&out.stderr)
    );
    assert!(!lab.root.join("apps").join("solo").join("1.0").exists());
    // Without arguments the list marks it.
    let out = tokio::task::block_in_place(|| lab.run(&["upgrade"]));
    assert!(
        text(&out.stdout).contains("solo (fixado)"),
        "{}",
        text(&out.stdout)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_and_tui_use_the_same_key() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
    use wingetrs::tui::app::{App, Cmd, Msg, Tab, update};
    use wingetrs::tui::theme::GlyphSet;
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "samekey").await;
    install_old(&lab, &["solo"]);
    // The key the TUI would write for a wingetrs row...
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.tab = Tab::Installed;
    a.installed = vec![ListRow {
        name: "solo".into(),
        app_key: String::new(),
        not_in_catalog: false,
        origin: "wingetrs".into(),
        installed_version: Some("0.9".into()),
        correlation: Correlation::Matched {
            source: "scoop:main".into(),
            package_id: "solo".into(),
            rule: Rule::ScoopManifest,
        },
        latest: None,
        state: UpdateState::UpToDate,
        tiebreak: None,
        ..Default::default()
    }];
    let k = |c| Msg::Key(KeyEvent::new(c, KeyModifiers::NONE));
    update(&mut a, k(KeyCode::Enter));
    let cmds = update(&mut a, k(KeyCode::Enter));
    let Some(Cmd::SetPin { key, .. }) = cmds.into_iter().next() else {
        panic!("no SetPin");
    };
    // ...is the one the CLI consults.
    pin(&lab, &key);
    let out = tokio::task::block_in_place(|| lab.run(&["upgrade", "solo", "--yes"]));
    assert!(
        text(&out.stderr).contains("versão fixada"),
        "{}",
        text(&out.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn yes_does_not_allow_msiexec() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "msiexec").await;
    let out = tokio::task::block_in_place(|| lab.run(&["install", "m", "--yes"]));
    let se = text(&out.stderr);
    assert!(!out.status.success(), "{se}");
    assert!(
        se.contains(
            "m: este MSI precisa do Windows Installer (MSI sem compressão (arquivos fora do pacote)); rode de novo com --allow-msiexec. O msiexec /a não pode ser cancelado depois de começar e grava arquivos em C:\\Windows\\Installer."
        ),
        "{se}"
    );
    assert!(!lab.root.join("apps").join("m").exists());
    // With the flag the fallback runs; under `WINGETRS_TEST_ENV_KEY` its launcher refuses
    // instead of starting msiexec.
    let out =
        tokio::task::block_in_place(|| lab.run(&["install", "m", "--yes", "--allow-msiexec"]));
    let se = text(&out.stderr);
    assert!(!out.status.success(), "{se}");
    assert!(se.contains("msiexec desativado neste ambiente"), "{se}");
    assert!(!se.contains("rode de novo com --allow-msiexec"), "{se}");
    assert!(!lab.root.join("apps").join("m").exists());
}

/// `--scope` on a Scoop app: what failed, then what to do (Task 199).
#[tokio::test(flavor = "multi_thread")]
async fn scope_on_a_scoop_app_says_what_to_do() {
    let server = MockServer::start().await;
    let lab = Lab::new(&server, "t199-scope").await;
    let out = tokio::task::block_in_place(|| {
        lab.run(&["install", "solo", "--scope", "user", "--dry-run"])
    });
    assert!(!out.status.success());
    assert_eq!(
        text(&out.stderr),
        "✗ --scope só vale para pacotes do winget\n→ rode de novo sem --scope\n"
    );
}
