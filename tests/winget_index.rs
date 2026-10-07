#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::sources::winget::index::{IndexError, read_index, validate_msix};
use wingetrs::sources::winget::{WingetFetch, fetch_index};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MSIX: &[u8] = include_bytes!("fixtures/winget/source2-mini.msix");
const CORRUPT: &[u8] = include_bytes!("fixtures/winget/source2-mini-corrupt.msix");
const DB: &[u8] = include_bytes!("fixtures/winget/index-mini.db");

#[test]
fn valid_msix_yields_exact_index_bytes() {
    assert_eq!(validate_msix(MSIX).unwrap(), DB);
}

#[test]
fn corrupt_block_is_rejected() {
    match validate_msix(CORRUPT) {
        Err(IndexError::Integrity { block }) => assert_eq!(block, (DB.len() / 2) / 65536),
        other => panic!("expected Integrity, got {other:?}"),
    }
}

#[test]
fn not_a_zip_is_an_error() {
    assert!(matches!(validate_msix(b"hello"), Err(IndexError::Zip(_))));
}

#[test]
fn reads_packages_and_all_maps() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("i.db");
    std::fs::write(&p, DB).unwrap();
    let imp = read_index(&p).unwrap();
    assert_eq!(imp.packages.len(), 20);
    assert_eq!(imp.maps.productcodes.len(), 6025);
    assert_eq!(imp.maps.upgradecodes.len(), 13);
    assert_eq!(imp.maps.pfns.len(), 5);
    assert_eq!(imp.maps.norm_names.len(), 29);
    assert_eq!(imp.maps.norm_publishers.len(), 22);
    let tag_total: usize = imp.packages.iter().map(|p| p.tags.len()).sum();
    let cmd_total: usize = imp.packages.iter().map(|p| p.commands.len()).sum();
    assert_eq!((tag_total, cmd_total), (289, 12));
    let py = imp
        .packages
        .iter()
        .find(|p| p.id == "Python.Python.3.13")
        .unwrap();
    assert_eq!(py.version, "3.13.15");
    assert_eq!(py.arp_max_version.as_deref(), Some("3.13.15150.0"));
    let pt = imp
        .packages
        .iter()
        .find(|p| p.id == "Microsoft.PowerToys")
        .unwrap();
    assert_eq!(
        hex::encode(&pt.winget_hash.as_ref().unwrap()[..4]),
        "d193b67a"
    );
    let mut seven: Vec<&str> = imp
        .maps
        .norm_names
        .iter()
        .filter(|(_, id)| id == "7zip.7zip")
        .map(|(k, _)| k.as_str())
        .collect();
    seven.sort();
    assert_eq!(seven, ["7zip", "7zip(X64)"]);
}

#[test]
fn unsupported_major_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("i.db");
    std::fs::write(&p, DB).unwrap();
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute(
        "update metadata set value='3' where name='majorVersion'",
        [],
    )
    .unwrap();
    drop(c);
    assert!(matches!(read_index(&p), Err(IndexError::UnsupportedVersion(v)) if v == "3"));
}

#[tokio::test]
async fn fetch_index_end_to_end_and_304() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/source2.msix"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(MSIX.to_vec())
                .insert_header("ETag", "\"e1\""),
        )
        .up_to_n_times(1)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/source2.msix"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let url = format!("{}/source2.msix", s.uri());
    let http = Http::new().unwrap();
    let entry = match fetch_index(&http, &data, &url, None).await.unwrap() {
        WingetFetch::Modified { import, entry } => {
            assert_eq!(import.packages.len(), 20);
            entry
        }
        WingetFetch::NotModified => panic!("expected Modified"),
    };
    assert!(matches!(
        fetch_index(&http, &data, &url, Some(&entry)).await.unwrap(),
        WingetFetch::NotModified
    ));
    let leftovers = std::fs::read_dir(data.tmp_dir()).unwrap().count();
    assert_eq!(leftovers, 0, "temporary index.db must be removed");
}

#[tokio::test]
async fn fetch_index_corrupt_download_is_integrity_error() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(CORRUPT.to_vec()))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let r = fetch_index(&Http::new().unwrap(), &data, &s.uri(), None).await;
    assert!(r.is_err());
}
