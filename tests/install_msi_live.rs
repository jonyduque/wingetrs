#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(windows)]
//! Own MSI extraction against the `msiexec /a` fallback on real packages (vectors of the
//! Task 119 probe). Runs the Windows Installer: only with `--ignored`, never in the gate.
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::executor::{Msiexec, extract_with_msiexec};
use wingetrs::install::extract::{self, ExtractSource};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::msiexec_win::{installer_busy, msiexec_admin};

/// `(app, url, sha256)`: small MSIs whose own layout matched `msiexec /a` in the probe.
const VECTORS: [(&str, &str, &str); 3] = [
    (
        "ncftpclient",
        "https://www.ncftp.com/public_ftp/ncftp/older_binaries/Setup%20NcFTP%203.2.6.msi",
        "b034c6c11cb2b888733e047ae9df494c8255e3d9ddfd0cbb2a6ccd98b02c79f2",
    ),
    (
        "isocreator",
        "https://download.sourceforge.net/project/iso-creator-cs/iso-creator-cs/binary-installer/IsoCreator.msi",
        "f01a0a824ddeeb46ade73e2bfc3141a0fee650089a6c4522ab38895d97351634",
    ),
    (
        "pkgconf",
        "https://github.com/pkgconf/pkgconf/releases/download/pkgconf-3.0.8/pkgconf-x64-3.0.8.msi",
        "a236479bbe403b202816745bef19afe2d6240606f28ab1f008ba0f33fbfae43e",
    ),
];

fn no_progress(_: &str, _: u64, _: Option<u64>) {}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: msiexec"]
async fn own_extraction_matches_msiexec_admin() {
    let t = tempfile::tempdir().unwrap();
    let data = DataDir::new(t.path());
    let layout = Layout::new(t.path());
    let http = Http::new().unwrap();
    let real = Msiexec {
        busy: installer_busy,
        launch: msiexec_admin,
    };
    for (app, url, sha) in VECTORS {
        let manifest = serde_json::json!({ "version": "1.0", "url": url, "hash": sha });
        let source = resolve(&manifest.to_string(), Arch::X64)
            .unwrap()
            .sources
            .remove(0);
        let file = wingetrs::install::download::fetch_all(
            &http,
            &data,
            std::slice::from_ref(&source),
            &no_progress,
        )
        .await
        .unwrap()
        .remove(0);
        let src = ExtractSource {
            file: &file,
            file_name: &source.file_name,
            extract_dir: None,
            extract_to: None,
        };
        let own = t.path().join("own").join(app);
        extract::extract_all(std::slice::from_ref(&src), &own).unwrap();
        let fallback = t.path().join("msiexec").join(app);
        tokio::task::block_in_place(|| {
            extract_with_msiexec(real, &layout, app, &src, &fallback, &|| {})
        })
        .unwrap();
        let tree = |p: &std::path::Path| wingetrs::install::snapshot::snapshot(p).unwrap();
        let (own_tree, fallback_tree) = (tree(&own), tree(&fallback));
        assert!(!own_tree.is_empty(), "{app}");
        assert_eq!(own_tree, fallback_tree, "{app}");
    }
}
