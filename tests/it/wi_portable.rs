#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Native portable and zip (Task 189).

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::bridge::BridgeJob;
use wingetrs::bridge::worker::ScopeArg;
use wingetrs::cache::DataDir;
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::cli::install_cmd::{Flags, Interrupts, Stop, Ui};
use wingetrs::cli::winget_cmd::{self, WingetFlows};
use wingetrs::config::{Behavior, ConfirmMode, InstallerMode};
use wingetrs::core::SourceKind;
use wingetrs::http::Http;
use wingetrs::install::executor::LaunchGate;
use wingetrs::install::layout::Layout;
use wingetrs::install::ops::{OpsCtx, ResidueScan, apply, plan_manifest, upgradable};
use wingetrs::install::plan::{InstallPlan, ShimKind, Step};
use wingetrs::install::state::StateDb;
use wingetrs::install::target::Candidate;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::installed::correlate::ListRow;
use wingetrs::installed::correlate::{Correlation, UpdateState, correlate};
use wingetrs::sources::winget::manifest;
use wingetrs::tui::app::{Action, App, Cmd, Done, InstallFail, Modal, Msg, Planned, update};
use wingetrs::tui::jobs;
use wingetrs::tui::picker::PickerItem;
use wingetrs::tui::theme::GlyphSet;
use wingetrs::update::LockWait;
use wingetrs::winget_install::engine::UpgradeError;
use wingetrs::winget_install::engine::{InstalledFacts, plan_upgrade};
use wingetrs::winget_install::ops::{InstallReport, WingetOpsError};
use wingetrs::winget_install::orphans::Orphan;
use wingetrs::winget_install::plan::WingetPlan;
use wingetrs::winget_install::plan::{PlanOrBridge, plan_with_mode};
use wingetrs::winget_install::portable::{
    PortableSpec, WINGET_BUCKET, install_plan, plan_from_catalog,
};
use wingetrs::winget_install::select::Refusal;
use wingetrs::winget_install::select::{Host, Scope, ScopeWanted};
use wingetrs::winget_install::uninstall::{UninstallPlan, UninstallTarget};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

fn host() -> Host {
    Host {
        arch: "x64".into(),
        os: [10, 0, 22631, 0],
        locale: "en-US".into(),
    }
}

const ID: &str = "Pub.App";

struct World {
    server: MockServer,
    _t: TempDir,
    data: DataDir,
    layout: Layout,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    http: Http,
    catalog: Catalog,
}

impl World {
    async fn new() -> World {
        let server = MockServer::start().await;
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        let layout = Layout::new(&root);
        std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
        std::fs::write(layout.launcher(), fake_pe()).unwrap();
        let shortcuts = FakeShortcuts::new(t.path().join("startmenu"));
        World {
            server,
            data: DataDir::new(&root),
            layout,
            _t: t,
            env: FakeEnv::new(),
            shortcuts,
            trash: FakeTrash::new(),
            registry: FakeRegistry::new(),
            http: Http::new().unwrap(),
            catalog: Catalog::open_in_memory().unwrap(),
        }
    }

    fn ctx(&self) -> OpsCtx<'_> {
        OpsCtx {
            data: &self.data,
            http: &self.http,
            catalog: &self.catalog,
            effects: Effects {
                env: &self.env,
                shortcuts: &self.shortcuts,
                trash: &self.trash,
                registry: &self.registry,
            },
            wait: LockWait::Block(std::time::Duration::from_millis(200)),
            progress: &no_progress,
            in_use: &nothing_in_use,
            residue: ResidueScan::none(),
            msiexec: wingetrs::install::executor::Msiexec::DISABLED,
            launch: Default::default(),
            runners: wingetrs::install::executor::Runners::disabled(),
        }
    }

    /// Serves `body` at `/dl/<name>` and returns its URL.
    async fn serve(&self, name: &str, body: Vec<u8>) -> String {
        Mock::given(method("GET"))
            .and(path(format!("/dl/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&self.server)
            .await;
        format!("{}/dl/{name}", self.server.uri())
    }

    fn plan(&self, spec: &PortableSpec) -> InstallPlan {
        install_plan(&self.ctx(), spec, false).unwrap()
    }
}

/// One x64 installer of `Pub.App` 1.0; `extra` holds its other keys (two-space indented).
fn yaml(url: &str, sha: &str, extra: &str) -> manifest::MergedManifest {
    let text = format!(
        "PackageIdentifier: {ID}\nPackageVersion: '1.0'\nPackageName: Pub App\nInstallers:\n- Architecture: x64\n  InstallerUrl: {url}\n  InstallerSha256: {sha}\n{extra}"
    );
    manifest::parse(text.as_bytes()).unwrap()
}

fn planned(m: &manifest::MergedManifest) -> Result<PlanOrBridge, String> {
    plan_with_mode(
        m,
        &host(),
        ScopeWanted::Prefer(Scope::User),
        None,
        InstallerMode::default(),
    )
    .map_err(|e| e.0)
}

fn portable_spec(m: &manifest::MergedManifest) -> PortableSpec {
    match planned(m) {
        Ok(PlanOrBridge::Portable(spec)) => spec,
        other => panic!("expected portable, got {other:?}"),
    }
}

fn shims(plan: &InstallPlan) -> Vec<(String, PathBuf, ShimKind)> {
    plan.steps
        .iter()
        .filter_map(|s| match s {
            Step::Shim(s) => Some((s.name.clone(), s.target.clone(), s.kind)),
            _ => None,
        })
        .collect()
}

fn download_names(plan: &InstallPlan) -> Vec<String> {
    plan.steps
        .iter()
        .filter_map(|s| match s {
            Step::Download { source, .. } => Some(source.file_name.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn portable_exe_becomes_app_with_shim() {
    let w = World::new().await;
    let exe = fake_pe();
    let sha = sha256_hex(&exe);
    let url = w.serve("tool.exe", exe.clone()).await;
    let m = yaml(
        &url,
        &sha,
        "  InstallerType: portable\n  Commands:\n  - x\n",
    );
    let spec = portable_spec(&m);
    let plan = w.plan(&spec);
    assert_eq!(plan.app, ID);
    assert_eq!(plan.bucket, WINGET_BUCKET);
    assert_eq!(plan.version, "1.0");
    assert_eq!(download_names(&plan), vec!["tool.exe".to_string()]);
    let current = w.layout.current(ID);
    assert_eq!(
        shims(&plan),
        vec![("x".to_string(), current.join("tool.exe"), ShimKind::Exe)]
    );

    apply(&w.ctx(), std::slice::from_ref(&plan)).await.unwrap();
    let vdir = w.layout.root().join("apps").join(ID).join("1.0");
    assert_eq!(std::fs::read(vdir.join("tool.exe")).unwrap(), exe);
    let shim = std::fs::read_to_string(w.layout.shims().join("x.shim")).unwrap();
    assert!(shim.contains("tool.exe"), "{shim}");
    let state = StateDb::open(&w.layout.state_db()).unwrap();
    let (rec, _) = state.get(ID).unwrap().unwrap();
    assert_eq!(
        (rec.bucket.as_str(), rec.version.as_str()),
        (WINGET_BUCKET, "1.0")
    );
}

#[tokio::test]
async fn zip_nested_portable_files() {
    let w = World::new().await;
    let exe = fake_pe();
    let zip = zip_of(&[("bin/x.exe", &exe), ("readme.txt", b"hi")]);
    let sha = sha256_hex(&zip);
    let url = w.serve("pkg.zip", zip).await;
    let m = yaml(
        &url,
        &sha,
        r"  InstallerType: zip
  NestedInstallerType: portable
  NestedInstallerFiles:
  - RelativeFilePath: bin\x.exe
    PortableCommandAlias: x
",
    );
    let plan = w.plan(&portable_spec(&m));
    let current = w.layout.current(ID);
    assert_eq!(
        shims(&plan),
        vec![(
            "x".to_string(),
            current.join("bin").join("x.exe"),
            ShimKind::Exe
        )]
    );

    apply(&w.ctx(), std::slice::from_ref(&plan)).await.unwrap();
    let vdir = w.layout.version_dir(ID, "1.0");
    assert_eq!(std::fs::read(vdir.join("bin").join("x.exe")).unwrap(), exe);
    assert!(w.layout.shims().join("x.shim").is_file());
}

#[tokio::test]
async fn nested_path_escape_refused() {
    let w = World::new().await;
    let zip = zip_of(&[("x.exe", &fake_pe())]);
    let sha = sha256_hex(&zip);
    let url = w.serve("pkg.zip", zip).await;
    let m = yaml(
        &url,
        &sha,
        r"  InstallerType: zip
  NestedInstallerType: portable
  NestedInstallerFiles:
  - RelativeFilePath: ..\..\x.exe
    PortableCommandAlias: x
",
    );
    match planned(&m) {
        Err(why) => assert!(why.contains("caminho fora do pacote"), "{why}"),
        other => panic!("expected refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn zip_with_msi_nested_stays_bridge() {
    let w = World::new().await;
    let zip = zip_of(&[("setup.msi", b"msi")]);
    let sha = sha256_hex(&zip);
    let url = w.serve("pkg.zip", zip).await;
    let m = yaml(
        &url,
        &sha,
        r"  InstallerType: zip
  NestedInstallerType: msi
  NestedInstallerFiles:
  - RelativeFilePath: setup.msi
",
    );
    match planned(&m) {
        Ok(PlanOrBridge::Bridge(why)) => assert!(why.contains("tipo zip"), "{why}"),
        other => panic!("expected bridge, got {other:?}"),
    }
}

#[tokio::test]
async fn hash_mismatch_installs_nothing() {
    let w = World::new().await;
    let exe = fake_pe();
    let url = w.serve("tool.exe", exe).await;
    let other = sha256_hex(b"not the served bytes");
    let m = yaml(
        &url,
        &other,
        "  InstallerType: portable\n  Commands:\n  - x\n",
    );
    let plan = w.plan(&portable_spec(&m));

    assert!(apply(&w.ctx(), std::slice::from_ref(&plan)).await.is_err());
    // `portable_exe_becomes_app_with_shim` shows these exist after a good download.
    assert!(!w.layout.app_dir(ID).exists());
    assert!(!w.layout.shims().join("x.shim").exists());
    let state = StateDb::open(&w.layout.state_db()).unwrap();
    assert!(state.get(ID).unwrap().is_none());
}

/// A manifest of `id` `ver` with one x64 installer at `url`.
fn with(id: &str, ver: &str, url: &str, sha: &str, extra: &str) -> manifest::MergedManifest {
    let text = format!(
        "PackageIdentifier: {id}\nPackageVersion: '{ver}'\nInstallers:\n- Architecture: x64\n  InstallerUrl: {url}\n  InstallerSha256: {sha}\n{extra}"
    );
    manifest::parse(text.as_bytes()).unwrap()
}

fn nested(n: usize) -> String {
    let files: String = (0..n)
        .map(|i| format!("  - RelativeFilePath: f{i}.exe\n"))
        .collect();
    format!(
        "  InstallerType: zip\n  NestedInstallerType: portable\n  NestedInstallerFiles:\n{files}"
    )
}

#[tokio::test]
async fn each_refusal_one_fixture() {
    let w = World::new().await;
    let sha = sha256_hex(b"x");
    let url = format!("{}/dl/tool.exe", w.server.uri());
    let portable = "  InstallerType: portable\n";
    let refused = |m: manifest::MergedManifest, want: &str| match planned(&m) {
        Err(why) => assert!(why.contains(want), "{want}: {why}"),
        other => panic!("{want}: expected refusal, got {other:?}"),
    };
    refused(
        with("Pub.App.", "1.0", &url, &sha, portable),
        "identificador inválido",
    );
    refused(with(ID, "1/0", &url, &sha, portable), "versão inválida");
    refused(
        with(ID, "1.0", &format!("{url}#x"), &sha, portable),
        "URL do instalador",
    );
    refused(
        with(ID, "1.0", "ftp://h/tool.exe", &sha, portable),
        "URL do instalador",
    );
    refused(with(ID, "1.0", &url, "AA", portable), "InstallerSha256");
    let bad_alias = "  InstallerType: portable\n  Commands:\n  - a:b\n";
    refused(
        with(ID, "1.0", &url, &sha, bad_alias),
        "apelido de comando inválido",
    );
    let no_files = "  InstallerType: zip\n  NestedInstallerType: portable\n";
    refused(
        with(ID, "1.0", &url, &sha, no_files),
        "sem NestedInstallerFiles",
    );
    refused(
        with(ID, "1.0", &url, &sha, &nested(65)),
        "65 arquivos aninhados",
    );
    // the same fixture with 64 files passes
    let m = with(ID, "1.0", &url, &sha, &nested(64));
    assert_eq!(shims(&w.plan(&portable_spec(&m))).len(), 64);
}

#[tokio::test]
async fn plain_portable_without_alias_is_named_by_its_exe() {
    let w = World::new().await;
    let sha = sha256_hex(b"x");
    let portable = "  InstallerType: portable\n";
    let m = yaml(
        &format!("{}/dl/Tool-1.0.exe", w.server.uri()),
        &sha,
        portable,
    );
    let plan = w.plan(&portable_spec(&m));
    assert_eq!(download_names(&plan), vec!["Tool-1.0.exe".to_string()]);
    assert_eq!(shims(&plan)[0].0, "Tool-1.0");
    // a URL without a plain `.exe` name: the file is named by the id
    let m = yaml(
        &format!("{}/dl/get?file=tool%20x.exe", w.server.uri()),
        &sha,
        portable,
    );
    let plan = w.plan(&portable_spec(&m));
    assert_eq!(download_names(&plan), vec![format!("{ID}.exe")]);
    assert_eq!(shims(&plan)[0].0, ID);
}

#[tokio::test]
async fn newer_version_upgrades_the_app() {
    let mut w = World::new().await;
    let alias = "  InstallerType: portable\n  Commands:\n  - x\n";
    let one = fake_pe();
    let url = w.serve("tool.exe", one.clone()).await;
    let plan = w.plan(&portable_spec(&yaml(&url, &sha256_hex(&one), alias)));
    apply(&w.ctx(), std::slice::from_ref(&plan)).await.unwrap();

    // the winget catalog has 2.0 now: listed as an upgrade of a wingetrs app
    w.catalog
        .replace_source(
            "winget",
            SourceKind::Winget,
            "v",
            1,
            &[NewPackage {
                id: ID.into(),
                name: "Pub App".into(),
                version: "2.0".into(),
                ..Default::default()
            }],
            &WingetMaps::default(),
        )
        .unwrap();
    assert_eq!(
        upgradable(&w.ctx()).unwrap().upgradable,
        [(ID.to_string(), "1.0".to_string(), "2.0".to_string())]
    );
    let apps = wingetrs::install::state::scan_wingetrs(&w.layout);
    let rows = correlate(
        &apps,
        &w.catalog,
        &wingetrs::installed::normalize::NameNormalizer::new(),
        &wingetrs::installed::winget_tracking::Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    let row = rows.iter().find(|r| r.name == ID).unwrap();
    assert_eq!(row.origin, "wingetrs");
    assert!(
        matches!(&row.correlation, Correlation::Matched { source, package_id, .. } if source == "winget" && package_id == ID),
        "{:?}",
        row.correlation
    );
    assert!(
        matches!(&row.state, UpdateState::Available { latest } if latest == "2.0"),
        "{:?}",
        row.state
    );

    let two = [fake_pe(), b"v2".to_vec()].concat();
    let url = w.serve("tool2.exe", two.clone()).await;
    let plan = w.plan(&portable_spec(&with(
        ID,
        "2.0",
        &url,
        &sha256_hex(&two),
        alias,
    )));
    assert_eq!(plan.upgrade_from.as_deref(), Some("1.0"));
    apply(&w.ctx(), std::slice::from_ref(&plan)).await.unwrap();
    let vdir = w.layout.version_dir(ID, "2.0");
    assert_eq!(std::fs::read(vdir.join("tool2.exe")).unwrap(), two);
    assert!(!w.layout.version_dir(ID, "1.0").exists());
    assert!(upgradable(&w.ctx()).unwrap().upgradable.is_empty());
}

#[tokio::test]
async fn same_name_of_another_bucket_is_refused() {
    let w = World::new().await;
    let exe = fake_pe();
    let url = w.serve("tool.exe", exe.clone()).await;
    let m = yaml(&url, &sha256_hex(&exe), "  InstallerType: portable\n");
    let spec = portable_spec(&m);
    apply(&w.ctx(), &[w.plan(&spec)]).await.unwrap();
    let e = plan_manifest(&w.ctx(), ID, "main", &spec.manifest, false).unwrap_err();
    assert!(e.to_string().contains("a partir do bucket winget"), "{e}");
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

/// Puts `Pub.App` `ver` in the winget catalog with `manifest_yaml` served by the CDN mock;
/// returns the CDN base.
async fn publish(w: &mut World, ver: &str, manifest_yaml: &str) -> String {
    let rel = format!("manifests/p/Pub/App/{ver}/m");
    let vd = format!(
        "vD:\n- v: '{ver}'\n  rP: {rel}\n  s256H: {}\n",
        sha256_hex(manifest_yaml.as_bytes())
    );
    let vd = mszip_of(vd.as_bytes());
    let hash = Sha256::digest(&vd).to_vec();
    Mock::given(method("GET"))
        .and(path(format!(
            "/packages/{ID}/{}/versionData.mszyml",
            hex::encode(&hash[..4])
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vd))
        .mount(&w.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{rel}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(manifest_yaml.as_bytes().to_vec()))
        .mount(&w.server)
        .await;
    w.catalog
        .replace_source(
            "winget",
            SourceKind::Winget,
            "v",
            1,
            &[NewPackage {
                id: ID.into(),
                name: "Pub App".into(),
                version: ver.into(),
                winget_hash: Some(hash),
                ..Default::default()
            }],
            &WingetMaps::default(),
        )
        .unwrap();
    format!("{}/", w.server.uri())
}

fn manifest_text(ver: &str, url: &str, sha: &str, extra: &str) -> String {
    format!(
        "PackageIdentifier: {ID}\nPackageVersion: '{ver}'\nInstallers:\n- Architecture: x64\n  InstallerUrl: {url}\n  InstallerSha256: {sha}\n{extra}"
    )
}

#[tokio::test]
async fn upgrade_plan_reads_the_catalog_manifest() {
    let mut w = World::new().await;
    let alias = "  InstallerType: portable\n  Commands:\n  - x\n";
    let one = fake_pe();
    let url = w.serve("tool.exe", one.clone()).await;
    let plan = w.plan(&portable_spec(&yaml(&url, &sha256_hex(&one), alias)));
    apply(&w.ctx(), std::slice::from_ref(&plan)).await.unwrap();

    let two = [fake_pe(), b"v2".to_vec()].concat();
    let url2 = w.serve("tool2.exe", two.clone()).await;
    let text = manifest_text("2.0", &url2, &sha256_hex(&two), alias);
    let base = publish(&mut w, "2.0", &text).await;
    let plan = plan_from_catalog(&w.ctx(), &base, &host(), ID, false)
        .await
        .unwrap();
    assert_eq!(
        (plan.version.as_str(), plan.upgrade_from.as_deref()),
        ("2.0", Some("1.0"))
    );
    assert_eq!(download_names(&plan), vec!["tool2.exe".to_string()]);

    // the catalog's newer version is no longer portable: refused, never planned as another kind
    let text = manifest_text("3.0", &url2, &sha256_hex(&two), "  InstallerType: msi\n");
    let base = publish(&mut w, "3.0", &text).await;
    let e = plan_from_catalog(&w.ctx(), &base, &host(), ID, false)
        .await
        .unwrap_err();
    assert!(
        e.to_string().contains("não é mais um pacote portable"),
        "{e}"
    );
}

#[test]
fn portable_with_machine_scope_required_goes_to_winget() {
    let sha = sha256_hex(b"x");
    let m = with(
        ID,
        "1.0",
        "https://h/tool.exe",
        &sha,
        "  InstallerType: portable\n",
    );
    match plan_with_mode(
        &m,
        &host(),
        ScopeWanted::Only(Scope::Machine),
        None,
        InstallerMode::default(),
    ) {
        Ok(PlanOrBridge::Bridge(why)) => assert!(why.contains("escopo máquina"), "{why}"),
        other => panic!("expected bridge, got {other:?}"),
    }
    // an installer declared for the machine, without a scope asked for
    let declared = with(
        ID,
        "1.0",
        "https://h/tool.exe",
        &sha,
        "  InstallerType: portable\n  Scope: machine\n",
    );
    match planned(&declared) {
        Ok(PlanOrBridge::Bridge(why)) => assert!(why.contains("escopo máquina"), "{why}"),
        other => panic!("expected bridge, got {other:?}"),
    }
    // the same manifest with the user scope required is a wingetrs app
    assert!(matches!(
        plan_with_mode(
            &m,
            &host(),
            ScopeWanted::Only(Scope::User),
            None,
            InstallerMode::default(),
        ),
        Ok(PlanOrBridge::Portable(_))
    ));
}

#[test]
fn upgrade_of_a_portable_record_of_the_windows_list_goes_to_winget() {
    let sha = sha256_hex(b"x");
    let m = with(
        ID,
        "2.0",
        "https://h/tool.exe",
        &sha,
        "  InstallerType: portable\n",
    );
    match plan_upgrade(
        &m,
        &host(),
        ScopeWanted::Prefer(Scope::User),
        "1.0",
        &InstalledFacts::default(),
        InstallerMode::default(),
    ) {
        Ok(PlanOrBridge::Bridge(why)) => assert!(why.contains("portable"), "{why}"),
        other => panic!("expected bridge, got {other:?}"),
    }
}
// ---- CLI and TUI wiring (review of Task 189) ----

const EULA: &str = "Agreements:\n- AgreementLabel: EULA\n  AgreementUrl: https://x/eula\n";

/// A portable `Pub.App` `ver` whose root carries `root` (e.g. its agreements).
fn portable_with_root(w: &World, ver: &str, root: &str) -> PortableSpec {
    let text = format!(
        "PackageIdentifier: {ID}\nPackageVersion: '{ver}'\nPackageName: Pub App\n{root}Installers:\n- Architecture: x64\n  InstallerUrl: {}/dl/tool.exe\n  InstallerSha256: {}\n  InstallerType: portable\n  Commands:\n  - x\n",
        w.server.uri(),
        sha256_hex(b"x")
    );
    portable_spec(&manifest::parse(text.as_bytes()).unwrap())
}

struct NoUi;

impl Ui for NoUi {
    fn interactive(&self) -> bool {
        false
    }
    fn confirm(&self, _: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    fn pick(&self, _: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        Ok(None)
    }
    fn pick_many(&self, _: Vec<PickerItem>, _: &[bool]) -> anyhow::Result<Option<Vec<usize>>> {
        Ok(None)
    }
}

/// Answers `planned` and records the spec `install_portable` receives; nothing else is asked.
struct PortableFlows {
    planned: PlanOrBridge,
    received: std::cell::RefCell<Vec<PortableSpec>>,
}

impl WingetFlows for PortableFlows {
    fn behavior(&self) -> Behavior {
        Behavior::default()
    }
    async fn plan_install(
        &self,
        _: &Candidate,
        _: ScopeWanted,
    ) -> anyhow::Result<Result<PlanOrBridge, Refusal>> {
        Ok(Ok(self.planned.clone()))
    }
    async fn install_portable<I: Interrupts>(
        &self,
        _: &dyn Ui,
        spec: &PortableSpec,
        _: Flags,
        _: Stop<'_, I>,
    ) -> anyhow::Result<()> {
        self.received.borrow_mut().push(spec.clone());
        Ok(())
    }
    fn gate(&self) -> LaunchGate {
        unreachable!("portable")
    }
    fn progress(&self) -> winget_cmd::Sink {
        unreachable!("portable")
    }
    fn install(&self, _: &WingetPlan) -> winget_cmd::Op<Result<InstallReport, WingetOpsError>> {
        unreachable!("portable")
    }
    async fn install_bridge<I: Interrupts>(
        &self,
        _: &dyn Ui,
        _: &Candidate,
        _: Option<ScopeArg>,
        _: Option<&str>,
        _: Flags,
        _: Stop<'_, I>,
    ) -> anyhow::Result<()> {
        unreachable!("portable")
    }
    fn installed(&self) -> anyhow::Result<Vec<ListRow>> {
        unreachable!("portable")
    }
    async fn plan_uninstall(
        &self,
        _: &ListRow,
        _: &UninstallTarget,
    ) -> Result<UninstallPlan, String> {
        unreachable!("portable")
    }
    fn lock_install(&self) -> winget_cmd::Op<Result<winget_cmd::Held, WingetOpsError>> {
        unreachable!("portable")
    }
    fn uninstall(&self, _: &UninstallPlan) -> winget_cmd::Uninstaller {
        unreachable!("portable")
    }
    fn remove_registry_only(&self, _: &UninstallPlan, _: &str) -> winget_cmd::RecordRemover {
        unreachable!("portable")
    }
    fn free_space(&self) -> fn(&Path) -> Option<u64> {
        unreachable!("portable")
    }
    fn pins(&self) -> std::collections::BTreeSet<String> {
        std::collections::BTreeSet::new()
    }
    async fn plan_upgrade(
        &self,
        _: &ListRow,
        _: &str,
        _: ScopeWanted,
    ) -> anyhow::Result<(Result<PlanOrBridge, Refusal>, Option<UninstallPlan>)> {
        unreachable!("portable")
    }
    fn upgrade(
        &self,
        _: &WingetPlan,
        _: Option<&UninstallPlan>,
    ) -> winget_cmd::Op<Result<InstallReport, UpgradeError>> {
        unreachable!("portable")
    }
    fn upgrade_bridge(&self, _: &BridgeJob, _: &mut dyn FnMut(&str)) -> Result<Done, InstallFail> {
        unreachable!("portable")
    }
    fn orphans(&self) -> anyhow::Result<Vec<Orphan>> {
        unreachable!("portable")
    }
    fn backups(&self) -> PathBuf {
        unreachable!("portable")
    }
    fn remove_orphan(
        &self,
        _: &Orphan,
        _: &str,
        _: &dyn Fn(&wingetrs::install::progress::ProgressEvent),
    ) -> Result<PathBuf, String> {
        unreachable!("portable")
    }
}

fn candidate() -> Candidate {
    Candidate {
        entry: wingetrs::core::PackageEntry {
            source: SourceKind::Winget,
            source_name: "winget".into(),
            id: ID.into(),
            name: "Pub App".into(),
            moniker: None,
            version: "1.0".into(),
            publisher: None,
            description: None,
            tags: Vec::new(),
            commands: Vec::new(),
        },
        selectable: true,
        note: None,
    }
}

/// The CLI's `install` lines and the specs its flows were handed.
async fn cli_install(spec: &PortableSpec, f: Flags) -> (Vec<String>, Vec<PortableSpec>) {
    let flows = PortableFlows {
        planned: PlanOrBridge::Portable(spec.clone()),
        received: Default::default(),
    };
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let gate = LaunchGate::default();
    let mut out = Vec::new();
    winget_cmd::install(
        &flows,
        &NoUi,
        &candidate(),
        None,
        f,
        Stop::new(&mut rx, &gate),
        &mut |l: &str| out.push(l.to_string()),
    )
    .await
    .unwrap();
    (out, flows.received.into_inner())
}

#[tokio::test]
async fn cli_portable_prints_agreements_and_hands_the_spec_over() {
    let w = World::new().await;
    let spec = portable_with_root(&w, "1.0", EULA);
    let yes = Flags {
        yes: true,
        ..Default::default()
    };
    let (out, received) = cli_install(&spec, yes).await;
    let text = out.join("\n");
    assert!(
        text.contains("Licença aceita: EULA — https://x/eula"),
        "{text}"
    );
    assert_eq!(received, vec![spec.clone()]);
    // without --yes the agreement is shown, not accepted
    let (out, received) = cli_install(&spec, Flags::default()).await;
    let text = out.join("\n");
    assert!(text.contains("EULA — https://x/eula"), "{text}");
    assert!(!text.contains("Licença aceita"), "{text}");
    assert_eq!(received, vec![spec]);
    // no agreements: no license line
    let bare = portable_with_root(&w, "1.0", "");
    let (out, received) = cli_install(&bare, yes).await;
    assert!(!out.join("\n").contains("EULA"), "{out:?}");
    assert_eq!(received, vec![bare]);
}

fn tui_app(confirm: ConfirmMode) -> App {
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.settings.behavior.confirm = confirm;
    a
}

#[tokio::test]
async fn tui_engine_portable_asks_for_the_2a_plan() {
    let w = World::new().await;
    let spec = portable_with_root(&w, "1.0", "");
    for keep in [false, true] {
        let mut a = tui_app(ConfirmMode::Always);
        a.settings.behavior.keep_old = keep;
        let cmds = update(
            &mut a,
            Msg::WingetInstallPlanned {
                bridge: Err("sem ponte".into()),
                native: Ok(PlanOrBridge::Portable(spec.clone())),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::PlanPortable {
                spec: spec.clone(),
                keep
            }]
        );
        assert!(a.modal.is_none());
    }
}

#[tokio::test]
async fn tui_portable_planned_each_arm() {
    let w = World::new().await;
    let spec = portable_with_root(&w, "1.0", "");
    let planned = || jobs::plan_portable(&w.ctx(), &spec, false);
    let license = "Licença: EULA — https://x/eula".to_string();

    // agreements: the confirmation opens even with confirmations turned off, and shows them
    let mut a = tui_app(ConfirmMode::Never);
    let cmds = update(
        &mut a,
        Msg::PortablePlanned {
            result: planned(),
            agreements: vec![license.clone()],
        },
    );
    assert!(cmds.is_empty(), "{cmds:?}");
    match &a.modal {
        Some(Modal::Confirm { notes, actions, .. }) => {
            assert_eq!(notes, &vec![license.clone()]);
            assert!(
                matches!(actions.as_slice(), [Action::Apply(_)]),
                "{actions:?}"
            );
        }
        _ => panic!("expected the confirmation"),
    }

    // no agreements, confirmations off: it starts at once, like any 2a plan
    let mut a = tui_app(ConfirmMode::Never);
    let cmds = update(
        &mut a,
        Msg::PortablePlanned {
            result: planned(),
            agreements: Vec::new(),
        },
    );
    assert!(a.modal.is_none());
    assert!(
        matches!(cmds.as_slice(), [Cmd::Execute(Action::Apply(_))]),
        "{cmds:?}"
    );

    // no agreements, confirmations on: the usual confirmation, without notes
    let mut a = tui_app(ConfirmMode::Always);
    let cmds = update(
        &mut a,
        Msg::PortablePlanned {
            result: planned(),
            agreements: Vec::new(),
        },
    );
    assert!(cmds.is_empty(), "{cmds:?}");
    match &a.modal {
        Some(Modal::Confirm { notes, .. }) => assert!(notes.is_empty(), "{notes:?}"),
        _ => panic!("expected the confirmation"),
    }

    // a failed plan is a toast, with or without agreements
    let mut a = tui_app(ConfirmMode::Never);
    let cmds = update(
        &mut a,
        Msg::PortablePlanned {
            result: Err("falhou".into()),
            agreements: vec![license],
        },
    );
    assert!(cmds.is_empty() && a.modal.is_none(), "{cmds:?}");
}

/// The text of a TUI plan.
fn planned_text(p: Result<Planned, String>) -> String {
    match p {
        Ok(Planned::Ready { text, .. }) => text,
        Ok(_) => panic!("expected Ready"),
        Err(e) => panic!("{e}"),
    }
}

#[tokio::test]
async fn cli_and_tui_portable_plans_are_the_same() {
    let w = World::new().await;
    let exe = fake_pe();
    let url = w.serve("tool.exe", exe.clone()).await;
    let alias = "  InstallerType: portable\n  Commands:\n  - x\n";
    let one = portable_spec(&yaml(&url, &sha256_hex(&exe), alias));
    apply(&w.ctx(), &[w.plan(&one)]).await.unwrap();
    let two = portable_spec(&with(ID, "2.0", &url, &sha256_hex(&exe), alias));

    let mut texts = Vec::new();
    for keep in [false, true] {
        let behavior = Behavior {
            keep_old: keep,
            ..Behavior::default()
        };
        let cli = winget_cmd::portable_plan(&w.ctx(), &two, behavior)
            .unwrap()
            .to_string();
        let mut a = tui_app(ConfirmMode::Always);
        a.settings.behavior = behavior;
        let cmds = update(
            &mut a,
            Msg::WingetInstallPlanned {
                bridge: Err("sem ponte".into()),
                native: Ok(PlanOrBridge::Portable(two.clone())),
            },
        );
        let [Cmd::PlanPortable { spec, keep: k }] = cmds.as_slice() else {
            panic!("{cmds:?}");
        };
        let tui = planned_text(jobs::plan_portable(&w.ctx(), spec, *k));
        assert_eq!(cli, tui, "keep_old = {keep}");
        texts.push(cli);
    }
    // the setting reaches both plans: keeping 1.0 is a different plan
    assert_ne!(texts[0], texts[1]);
}

/// Runs a TUI planning job as `spawn_job` does: on a thread outside the runtime, blocking on it.
fn tui_upgrade_world() -> (tokio::runtime::Runtime, World, jobs::WingetSrc) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut w = rt.block_on(World::new());
    let alias = "  InstallerType: portable\n  Commands:\n  - x\n";
    let one = fake_pe();
    let url = rt.block_on(w.serve("tool.exe", one.clone()));
    let plan = w.plan(&portable_spec(&yaml(&url, &sha256_hex(&one), alias)));
    rt.block_on(apply(&w.ctx(), std::slice::from_ref(&plan)))
        .unwrap();
    let two = [fake_pe(), b"v2".to_vec()].concat();
    let url2 = rt.block_on(w.serve("tool2.exe", two.clone()));
    let text = manifest_text("2.0", &url2, &sha256_hex(&two), alias);
    let base = rt.block_on(publish(&mut w, "2.0", &text));
    let src = jobs::WingetSrc {
        handle: rt.handle().clone(),
        cdn_base: base,
    };
    (rt, w, src)
}

fn upgrade_groups(p: Result<Planned, String>) -> Vec<Vec<InstallPlan>> {
    match p {
        Ok(Planned::Ready {
            action: Action::Apply(groups),
            ..
        }) => groups,
        Ok(other) => panic!("expected Apply, got {other:?}"),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn tui_jobs_plan_the_portable_upgrade_from_the_catalog() {
    let (_rt, w, src) = tui_upgrade_world();
    let ctx = w.ctx();
    let check = |groups: Vec<Vec<InstallPlan>>| {
        let [group] = groups.as_slice() else {
            panic!("{groups:?}")
        };
        let [plan] = group.as_slice() else {
            panic!("{group:?}")
        };
        assert_eq!(
            (plan.version.as_str(), plan.upgrade_from.as_deref()),
            ("2.0", Some("1.0"))
        );
        assert_eq!(download_names(plan), vec!["tool2.exe".to_string()]);
    };
    // the upgrade of one row (source `winget`)
    check(upgrade_groups(jobs::plan_install(
        &ctx, &src, "winget", ID, false,
    )));
    // "upgrade all"
    check(upgrade_groups(jobs::plan_upgrade_all(&ctx, &src, false)));
    // a batch item
    let batch = jobs::plan_batch(
        &ctx,
        &src,
        &[Cmd::PlanInstall {
            source_name: "winget".into(),
            id: ID.into(),
            keep: false,
        }],
    );
    let [one] = <[_; 1]>::try_from(batch).unwrap();
    check(upgrade_groups(one));
    // an id that wingetrs did not install from winget
    let e = jobs::plan_install(&ctx, &src, "winget", "Other.App", false).unwrap_err();
    assert!(
        e.contains("não é um pacote do winget instalado pelo wingetrs"),
        "{e}"
    );
}
