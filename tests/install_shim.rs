#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::Write;
use std::path::Path;

use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::layout::Layout;
use wingetrs::install::plan::{ShimKind, ShimSpec};
use wingetrs::install::shim::{
    ShimError, create_shim, ensure_launcher_from, pe_subsystem, remove_shim, set_pe_subsystem,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Minimal PE-looking buffer: MZ header with e_lfanew = 0x80, "PE\0\0", optional header subsystem at +24+68.
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

#[test]
fn reads_and_patches_subsystem() {
    let mut b = fake_pe(3);
    assert_eq!(pe_subsystem(&b), Some(3));
    assert!(set_pe_subsystem(&mut b, 2));
    assert_eq!(pe_subsystem(&b), Some(2));
    let off = 0x80 + 24 + 68;
    assert_eq!(&b[off..off + 2], &[2, 0]);
}

#[test]
fn invalid_or_short_files_are_not_touched() {
    for mut b in [
        vec![],
        b"MZ".to_vec(),
        {
            let mut x = fake_pe(3);
            x[0x80] = b'X';
            x
        },
        {
            let mut x = fake_pe(3);
            x[0x3C..0x40].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());
            x
        },
    ] {
        let before = b.clone();
        assert_eq!(pe_subsystem(&b), None);
        assert!(!set_pe_subsystem(&mut b, 2));
        assert_eq!(b, before);
    }
}

fn setup() -> (tempfile::TempDir, Layout) {
    let t = tempfile::tempdir().unwrap();
    let layout = Layout::new(t.path());
    std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
    std::fs::write(layout.launcher(), fake_pe(3)).unwrap();
    (t, layout)
}

fn spec(name: &str, target: &Path, args: Option<&str>, kind: ShimKind) -> ShimSpec {
    ShimSpec {
        name: name.into(),
        target: target.to_path_buf(),
        args: args.map(Into::into),
        kind,
    }
}

#[test]
fn console_exe_shim_matches_launcher() {
    let (t, layout) = setup();
    let target = t.path().join("app.exe");
    std::fs::write(&target, fake_pe(3)).unwrap();
    let files = create_shim(&layout, &spec("app", &target, Some("--x"), ShimKind::Exe)).unwrap();
    assert_eq!(files.len(), 2);
    let shims = layout.shims();
    assert_eq!(
        std::fs::read(shims.join("app.exe")).unwrap(),
        std::fs::read(layout.launcher()).unwrap()
    );
    let text = std::fs::read_to_string(shims.join("app.shim")).unwrap();
    assert_eq!(
        text,
        format!("path = \"{}\"\r\nargs = --x\r\n", target.display())
    );
}

#[test]
fn shim_without_args_has_no_args_line() {
    let (t, layout) = setup();
    let target = t.path().join("a.exe");
    std::fs::write(&target, fake_pe(3)).unwrap();
    create_shim(&layout, &spec("a", &target, None, ShimKind::Exe)).unwrap();
    let text = std::fs::read_to_string(layout.shims().join("a.shim")).unwrap();
    assert_eq!(text, format!("path = \"{}\"\r\n", target.display()));
}

#[test]
fn gui_target_gets_patched_copy_and_launcher_stays() {
    let (t, layout) = setup();
    let target = t.path().join("gui.exe");
    std::fs::write(&target, fake_pe(2)).unwrap();
    create_shim(&layout, &spec("gui", &target, None, ShimKind::Exe)).unwrap();
    let shim_exe = std::fs::read(layout.shims().join("gui.exe")).unwrap();
    assert_eq!(pe_subsystem(&shim_exe), Some(2));
    let launcher = std::fs::read(layout.launcher()).unwrap();
    assert_eq!(pe_subsystem(&launcher), Some(3));
}

#[test]
fn script_wrappers_and_remove() {
    let (t, layout) = setup();
    let shims = layout.shims();
    let bat = t.path().join("x.bat");
    create_shim(&layout, &spec("b", &bat, Some("-a"), ShimKind::Bat)).unwrap();
    assert_eq!(
        std::fs::read_to_string(shims.join("b.cmd")).unwrap(),
        format!("@rem {0}\r\n@\"{0}\" -a %*\r\n", bat.display())
    );
    let ps1 = t.path().join("x.ps1");
    create_shim(&layout, &spec("p", &ps1, None, ShimKind::Ps1)).unwrap();
    assert_eq!(
        std::fs::read_to_string(shims.join("p.ps1")).unwrap(),
        format!("& \"{}\" @args\r\n", ps1.display())
    );
    let cmd = std::fs::read_to_string(shims.join("p.cmd")).unwrap();
    assert!(cmd.contains("pwsh.exe") && cmd.contains("powershell.exe"));
    assert!(cmd.contains(&format!(
        "-noprofile -ex unrestricted -file \"{}\" %*",
        ps1.display()
    )));
    let jar = t.path().join("lib").join("x.jar");
    create_shim(&layout, &spec("j", &jar, None, ShimKind::Jar)).unwrap();
    assert_eq!(
        std::fs::read_to_string(shims.join("j.cmd")).unwrap(),
        format!(
            "@rem {0}\r\n@pushd {1}\r\n@java -jar \"{0}\" %*\r\n@popd\r\n",
            jar.display(),
            jar.parent().unwrap().display()
        )
    );
    remove_shim(&layout, "p").unwrap();
    assert!(!shims.join("p.ps1").exists() && !shims.join("p.cmd").exists());
    remove_shim(&layout, "p").unwrap();
}

fn launcher_zip() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        z.start_file("shim.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(&fake_pe(3)).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn sha256_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

#[tokio::test]
async fn ensure_launcher_downloads_and_verifies_hash() {
    let zip = launcher_zip();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/l.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip.clone()))
        .mount(&server)
        .await;
    let url = format!("{}/l.zip", server.uri());
    let http = Http::new().unwrap();

    let t = tempfile::tempdir().unwrap();
    let layout = Layout::new(t.path());
    let data = DataDir::new(t.path());
    let bad = "0".repeat(64);
    let err = ensure_launcher_from(&http, &data, &layout, &url, &bad)
        .await
        .unwrap_err();
    assert!(matches!(err, ShimError::Launcher(_)), "{err:?}");
    assert!(!layout.launcher().exists());

    ensure_launcher_from(&http, &data, &layout, &url, &sha256_hex(&zip))
        .await
        .unwrap();
    assert_eq!(std::fs::read(layout.launcher()).unwrap(), fake_pe(3));
}
