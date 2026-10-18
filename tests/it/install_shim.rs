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
        format!(
            "\u{feff}$path = '{}'\r\n$shimArgs = @()\r\n& $path @shimArgs @args\r\n",
            ps1.display()
        )
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

#[test]
fn remove_shim_removes_only_its_own_files() {
    let (t, layout) = setup();
    let target = t.path().join("a.exe");
    std::fs::write(&target, fake_pe(3)).unwrap();
    create_shim(&layout, &spec("app", &target, None, ShimKind::Exe)).unwrap();
    create_shim(&layout, &spec("app2", &target, None, ShimKind::Exe)).unwrap();
    std::fs::write(layout.shims().join("app.cmd"), "x").unwrap();
    std::fs::write(layout.shims().join("app.ps1"), "x").unwrap();
    remove_shim(&layout, "app").unwrap();
    for e in ["exe", "shim", "cmd", "ps1"] {
        assert!(!layout.shims().join(format!("app.{e}")).exists(), "{e}");
    }
    assert!(layout.shims().join("app2.exe").exists());
    assert!(layout.shims().join("app2.shim").exists());
}

#[test]
fn recreation_updates_args_and_flips_gui_console() {
    let (t, layout) = setup();
    let target = t.path().join("a.exe");
    std::fs::write(&target, fake_pe(3)).unwrap();
    let sp = |a| spec("app", &target, a, ShimKind::Exe);
    create_shim(&layout, &sp(Some("--a"))).unwrap();
    create_shim(&layout, &sp(Some("--b"))).unwrap();
    let shim = layout.shims().join("app.shim");
    assert!(
        std::fs::read_to_string(&shim)
            .unwrap()
            .ends_with("args = --b\r\n")
    );
    let launcher = std::fs::read(layout.launcher()).unwrap();
    std::fs::write(&target, fake_pe(2)).unwrap();
    create_shim(&layout, &sp(None)).unwrap();
    let exe = layout.shims().join("app.exe");
    assert_eq!(pe_subsystem(&std::fs::read(&exe).unwrap()), Some(2));
    assert_eq!(std::fs::read(layout.launcher()).unwrap(), launcher);
    std::fs::write(&target, fake_pe(3)).unwrap();
    create_shim(&layout, &sp(None)).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), launcher);
    assert_eq!(std::fs::read(layout.launcher()).unwrap(), launcher);
}

#[test]
fn invalid_launcher_with_gui_target_is_an_error() {
    let (t, layout) = setup();
    std::fs::write(layout.launcher(), b"not a pe").unwrap();
    let target = t.path().join("g.exe");
    std::fs::write(&target, fake_pe(2)).unwrap();
    let err = create_shim(&layout, &spec("g", &target, None, ShimKind::Exe)).unwrap_err();
    assert!(matches!(err, ShimError::Launcher(_)), "{err:?}");
}

#[test]
fn pe_beyond_header_window_is_console() {
    let (t, layout) = setup();
    let target = t.path().join("big.exe");
    let mut b = fake_pe(2);
    b.resize(10_000, 0);
    b[0x3C..0x40].copy_from_slice(&5000u32.to_le_bytes());
    b[5000..5004].copy_from_slice(b"PE\0\0");
    let off = 5000 + 24 + 68;
    b[off..off + 2].copy_from_slice(&2u16.to_le_bytes());
    std::fs::write(&target, b).unwrap();
    create_shim(&layout, &spec("big", &target, None, ShimKind::Exe)).unwrap();
    assert_eq!(
        std::fs::read(layout.shims().join("big.exe")).unwrap(),
        std::fs::read(layout.launcher()).unwrap()
    );
}

#[test]
fn unsafe_names_are_refused_and_nothing_escapes() {
    let (t, layout) = setup();
    let target = t.path().join("a.exe");
    std::fs::write(&target, fake_pe(3)).unwrap();
    for name in [r"..\evil", "../evil", r"a\b", "CON", "x.", ""] {
        for kind in [ShimKind::Exe, ShimKind::Bat, ShimKind::Ps1, ShimKind::Jar] {
            let err = create_shim(&layout, &spec(name, &target, None, kind)).unwrap_err();
            assert!(
                matches!(err, ShimError::Io(ref m) if m.contains("inseguro")),
                "{name}: {err:?}"
            );
        }
        assert!(
            matches!(remove_shim(&layout, name), Err(ShimError::Io(_))),
            "{name}"
        );
    }
    assert!(!t.path().join("evil.exe").exists());
    assert!(!t.path().join("evil.shim").exists());
    let n = std::fs::read_dir(layout.shims()).unwrap().count();
    assert_eq!(n, 1, "only .launcher");
}

#[test]
fn wrappers_with_args_exact_text() {
    let (t, layout) = setup();
    let shims = layout.shims();
    let ps1 = t.path().join("x.ps1");
    create_shim(&layout, &spec("p", &ps1, Some("-a b"), ShimKind::Ps1)).unwrap();
    let p = ps1.display();
    assert_eq!(
        std::fs::read_to_string(shims.join("p.ps1")).unwrap(),
        format!("\u{feff}$path = '{p}'\r\n$shimArgs = @('-a', 'b')\r\n& $path @shimArgs @args\r\n")
    );
    assert_eq!(
        std::fs::read_to_string(shims.join("p.cmd")).unwrap(),
        format!(
            "@rem {p}\r\n@echo off\r\nwhere /q pwsh.exe\r\nif %errorlevel% equ 0 (\r\n    pwsh.exe -noprofile -ex unrestricted -file \"{p}\" -a b %*\r\n) else (\r\n    powershell.exe -noprofile -ex unrestricted -file \"{p}\" -a b %*\r\n)\r\n"
        )
    );
    let jar = t.path().join("lib").join("x.jar");
    create_shim(&layout, &spec("j", &jar, Some("-v"), ShimKind::Jar)).unwrap();
    assert_eq!(
        std::fs::read_to_string(shims.join("j.cmd")).unwrap(),
        format!(
            "@rem {0}\r\n@pushd {1}\r\n@java -jar \"{0}\" -v %*\r\n@popd\r\n",
            jar.display(),
            jar.parent().unwrap().display()
        )
    );
}
/// Task 191: shim arguments pass an allowlist (one rule for the manifest `bin` and the hook
/// `shim` effect).
#[test]
fn shim_args_refuse_shell_syntax() {
    use wingetrs::install::shim::check_args;
    for ok in [
        "--flag x",
        r#"--flag "C:\Program Files\x""#,
        r#"--user-data-dir="C:\Users\a\apps\brave\current\User Data""#,
        "-jar a.jar -Xmx=1g,a+b",
    ] {
        assert_eq!(check_args(ok), Ok(()), "{ok}");
    }
    for bad in [
        "x & calc",
        "%PATH%",
        "$(calc)",
        "'; calc; '",
        "{calc}",
        "\u{201C}calc\u{201D}",
        "a | b",
        "a > b",
        "a ^ b",
        "a; calc",
        "`calc`",
        "@x",
        "# x",
        r#""a & b""#,
        r#""unclosed"#,
        "a\r\nargs = evil",
        "a\tb",
    ] {
        assert!(check_args(bad).is_err(), "{bad:?}");
    }
}

/// The `.ps1` shim never interpolates: a `'` in the target is doubled, a quoted argument is
/// one literal.
#[test]
fn ps1_shim_uses_literal_array() {
    let (t, layout) = setup();
    let dir = t.path().join("it's");
    std::fs::create_dir_all(&dir).unwrap();
    let ps1 = dir.join("x.ps1");
    let args = r#"--flag "C:\Program Files\x" -b"#;
    create_shim(&layout, &spec("p", &ps1, Some(args), ShimKind::Ps1)).unwrap();
    let want = format!(
        "\u{feff}$path = '{}'\r\n$shimArgs = @('--flag', 'C:\\Program Files\\x', '-b')\r\n& $path @shimArgs @args\r\n",
        ps1.display().to_string().replace('\'', "''")
    );
    assert_eq!(
        std::fs::read_to_string(layout.shims().join("p.ps1")).unwrap(),
        want
    );
}

/// Plans a manifest of version `version` whose `bin` passes `args`, under the wingetrs root
/// `root`; returns its shim step or the planning error.
fn plan_shim(
    root: &Path,
    version: &str,
    args: &str,
) -> Result<ShimSpec, wingetrs::install::planner::PlanError> {
    use wingetrs::install::manifest::{Arch, resolve};
    use wingetrs::install::plan::Step;
    use wingetrs::install::planner::{PlanInput, plan_install};
    let m = serde_json::json!({
        "version": version,
        "url": "https://example.invalid/t.zip",
        "hash": "0".repeat(64),
        "bin": [["t.ps1", "t", args]],
    });
    let m = resolve(&m.to_string(), Arch::X64).unwrap();
    let layout = Layout::new(root);
    let shortcuts = root.join("startmenu");
    let plan = plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &m,
        layout: &layout,
        shortcut_root: &shortcuts,
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
    })?;
    Ok(plan
        .steps
        .into_iter()
        .find_map(|s| match s {
            Step::Shim(sp) => Some(sp),
            _ => None,
        })
        .unwrap())
}

/// Task 191: `$version` comes from the manifest: in shim arguments only a plain value.
#[test]
fn shim_args_version_with_cmd_syntax_refused() {
    use wingetrs::install::planner::PlanError;
    let t = tempfile::tempdir().unwrap();
    let r = plan_shim(t.path(), "1.0&calc", "--v $version");
    assert!(matches!(r, Err(PlanError::UnsafeArgs(_))), "{r:?}");
    // Not used in the arguments: the version does not matter to them.
    assert!(plan_shim(t.path(), "1.0&calc", "--v x").is_ok());
    assert_eq!(
        plan_shim(t.path(), "1.0-rc.1+b2", "--v $version")
            .unwrap()
            .args
            .as_deref(),
        Some("--v 1.0-rc.1+b2")
    );
}

/// Task 191: a user profile folder with `&` and parentheses reaches the shims only inside
/// quotes (`.cmd`) or a single-quoted literal (`.ps1`); one with `%` refuses the shim.
#[test]
fn shim_args_dir_with_ampersand_stays_inert() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Tom&Jerry (x)");
    let spec = plan_shim(&root, "1.0", r#"--data $dir\data --name="a b" -q"#).unwrap();
    let dir = root.join("apps").join("app").join("current");
    let data = format!(r"{}\data", dir.display());
    assert_eq!(
        spec.args.as_deref(),
        Some(format!(r#"--data "{data}" "--name=a b" -q"#).as_str())
    );
    let layout = Layout::new(&root);
    create_shim(&layout, &spec).unwrap();
    let cmd = std::fs::read_to_string(layout.shims().join("t.cmd")).unwrap();
    assert!(
        cmd.contains(&format!(
            r#"-file "{}" --data "{data}" "--name=a b" -q %*"#,
            spec.target.display()
        )),
        "{cmd}"
    );
    let ps1 = std::fs::read_to_string(layout.shims().join("t.ps1")).unwrap();
    assert!(
        ps1.contains(&format!(
            "$shimArgs = @('--data', '{data}', '--name=a b', '-q')"
        )),
        "{ps1}"
    );
    let r = plan_shim(&t.path().join("100%"), "1.0", r"--data $dir\data");
    assert!(
        matches!(r, Err(wingetrs::install::planner::PlanError::UnsafeArgs(_))),
        "{r:?}"
    );
}
/// Task 191 review: PowerShell reads U+2018-U+201B as single quotes too. A `’` in the target,
/// in the arguments or in our folders is doubled in the `.ps1` shim, and the file has a BOM so
/// PowerShell 5.1 does not read UTF-8 bytes (`ő` = C5 91) as CP1252 quotes.
#[test]
fn ps1_shim_typographic_quotes_and_bom() {
    let (t, layout) = setup();
    let dir = t.path().join("a\u{2019};calc;\u{2019}b");
    std::fs::create_dir_all(&dir).unwrap();
    let ps1 = dir.join("x.ps1");
    create_shim(
        &layout,
        &spec("p", &ps1, Some("\u{2018};calc;\u{201B}"), ShimKind::Ps1),
    )
    .unwrap();
    let bytes = std::fs::read(layout.shims().join("p.ps1")).unwrap();
    assert_eq!(&bytes[..3], b"\xEF\xBB\xBF");
    let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
    let target = ps1
        .display()
        .to_string()
        .replace('\u{2019}', "\u{2019}\u{2019}");
    assert_eq!(
        text,
        format!(
            "$path = '{target}'\r\n$shimArgs = @('\u{2018}\u{2018};calc;\u{201B}\u{201B}')\r\n& $path @shimArgs @args\r\n"
        )
    );
}

/// The same through planning: `$dir` under a profile with `’` and `ő` reaches the `.ps1` as a
/// literal (quotes doubled, UTF-8 kept behind a BOM) and the `.cmd` inside double quotes.
#[test]
fn ps1_shim_dir_with_typographic_quote_and_o_double_acute() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Jo\u{2019}\u{0151}");
    let spec = plan_shim(&root, "1.0", r"--data $dir\data").unwrap();
    let data = format!(
        r"{}\data",
        root.join("apps").join("app").join("current").display()
    );
    assert_eq!(
        spec.args.as_deref(),
        Some(format!(r#"--data "{data}""#).as_str())
    );
    let layout = Layout::new(&root);
    create_shim(&layout, &spec).unwrap();
    let bytes = std::fs::read(layout.shims().join("t.ps1")).unwrap();
    assert_eq!(&bytes[..3], b"\xEF\xBB\xBF");
    assert!(bytes.windows(2).any(|w| w == [0xC5, 0x91]), "ő stays UTF-8");
    let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
    let doubled = data.replace('\u{2019}', "\u{2019}\u{2019}");
    assert!(
        text.contains(&format!("$shimArgs = @('--data', '{doubled}')")),
        "{text}"
    );
}

/// Task 191 review: a quoted argument ending in `\` gets its trailing backslashes doubled, or
/// the C runtime would read `\"` as a literal quote and merge the next arguments.
#[test]
fn shim_args_trailing_backslash_doubled() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("r");
    let spec = plan_shim(&root, "1.0", r#"--out "$dir\" -q"#).unwrap();
    let dir = root.join("apps").join("app").join("current");
    assert_eq!(
        spec.args.as_deref(),
        Some(format!(r#"--out "{}\\" -q"#, dir.display()).as_str())
    );
    // The `.ps1` reads the same text back by the C runtime rules: one backslash.
    let layout = Layout::new(&root);
    create_shim(&layout, &spec).unwrap();
    let ps1 = std::fs::read_to_string(layout.shims().join("t.ps1")).unwrap();
    assert!(
        ps1.contains(&format!(
            r"$shimArgs = @('--out', '{}\', '-q')",
            dir.display()
        )),
        "{ps1}"
    );
}
