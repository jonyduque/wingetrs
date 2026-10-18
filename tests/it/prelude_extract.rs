#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 154). Prelude group 3: extraction through helpers, external commands
//! and persist. Helpers are fake `.cmd` files written by the test (they record `%*` in a file
//! and create files in the destination); no real helper runs. Real PowerShell is `live`.

use std::path::{Path, PathBuf};
use std::process::Command;

use wingetrs::install::inno::inno_args;
use wingetrs::install::prelude::{CONTRACT_VARS, PRELUDE};

struct Out {
    code: i32,
    out: String,
}

/// Temp layout: `work` holds scripts, archives and fakes; `root` is `$__wr_root`;
/// `dest` is an extraction destination (not created).
struct Dirs {
    _t: tempfile::TempDir,
    work: PathBuf,
    root: PathBuf,
    dest: PathBuf,
}

fn dirs() -> Dirs {
    let t = tempfile::tempdir().unwrap();
    let work = t.path().join("work");
    let root = t.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let dest = t.path().join("dest");
    Dirs {
        _t: t,
        work,
        root,
        dest,
    }
}

fn pwsh() -> String {
    wingetrs::bridge::find_pwsh(std::env::var_os("PATH").as_deref())
}

/// Writes a fake helper `.cmd`: records `%*` in `rec`, runs `extra` (cmd lines), exits `code`.
fn fake(d: &Dirs, name: &str, rec: &Path, extra: &str, code: i32) -> PathBuf {
    let p = d.work.join(name);
    let text = format!(
        "@echo off\r\n>\"{}\" echo %*\r\n{extra}\r\nexit /b {code}\r\n",
        rec.display()
    );
    std::fs::write(&p, text).unwrap();
    p
}

/// cmd lines that create `dest` with one file in it.
fn creates_file(d: &Dirs) -> String {
    format!("mkdir \"{0}\"\r\n>\"{0}\\f.txt\" echo x", d.dest.display())
}

/// Runs `body` after dot-sourcing the prelude; `helpers` fills `$__wr_helpers`.
fn run(d: &Dirs, helpers: &[(&str, &Path)], body: &str) -> Out {
    std::fs::write(d.work.join("prelude.ps1"), PRELUDE).unwrap();
    let mut s = String::from("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n");
    s += &format!("$__wr_root = '{}'\n", d.root.display());
    s += &format!(
        "$__wr_effects = '{}'\n",
        d.work.join("effects.jsonl").display()
    );
    s += "$__wr_env_seed = $null\n";
    s += &format!("$__wr_shortcut_root = '{}'\n", d.work.join("sc").display());
    s += "$__wr_helpers = @{}\n";
    for (k, v) in helpers {
        s += &format!("$__wr_helpers['{k}'] = '{}'\n", v.display());
    }
    for v in CONTRACT_VARS {
        s += &format!("if (-not (Test-Path variable:{v})) {{ throw 'missing {v}' }}\n");
    }
    s += &format!(". '{}'\n{body}\n", d.work.join("prelude.ps1").display());
    let script = d.work.join("t.ps1");
    std::fs::write(&script, s).unwrap();
    let o = Command::new(pwsh())
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .output()
        .unwrap();
    let mut out = String::from_utf8_lossy(&o.stdout).into_owned();
    out += &String::from_utf8_lossy(&o.stderr);
    Out {
        code: o.status.code().unwrap_or(-1),
        out,
    }
}

fn lines(o: &Out) -> Vec<String> {
    o.out.lines().map(|l| l.trim_end().to_string()).collect()
}

fn recorded(rec: &Path) -> String {
    std::fs::read_to_string(rec).unwrap().trim_end().to_string()
}

#[test]
#[ignore = "live: roda pwsh real"]
fn expand_7zip_calls_helper_with_scoop_args() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let h = fake(&d, "7z.cmd", &rec, "", 0);
    let arc = d.work.join("a.7z");
    let o = run(
        &d,
        &[("7zip", &h)],
        &format!(
            "Expand-7zipArchive -Path '{}' -DestinationPath '{}\\'",
            arc.display(),
            d.dest.display()
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    // .cmd is a legacy command: drive-letter arguments are quoted, as in the original
    assert_eq!(
        recorded(&rec),
        format!(
            "x \"{}\" -o\"{}\" -xr!*.nsis -y",
            arc.display(),
            d.dest.display()
        )
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn expand_7zip_extract_dir_moves_subdir() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let extra = format!(
        "mkdir \"{0}\\sub\"\r\n>\"{0}\\sub\\a.txt\" echo x",
        d.dest.display()
    );
    let h = fake(&d, "7z.cmd", &rec, &extra, 0);
    let arc = d.work.join("a.7z");
    let o = run(
        &d,
        &[("7zip", &h)],
        &format!(
            "Expand-7zipArchive -Path '{}' -DestinationPath '{}' -ExtractDir 'sub'\n\
             Test-Path -LiteralPath '{}\\a.txt'\nTest-Path -LiteralPath '{}\\sub'",
            arc.display(),
            d.dest.display(),
            d.dest.display(),
            d.dest.display()
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    let l: Vec<String> = lines(&o)
        .into_iter()
        .filter(|l| l == "True" || l == "False")
        .collect();
    assert_eq!(l, vec!["True", "False"], "{}", o.out);
    assert!(recorded(&rec).contains("-ir!sub\\*"), "{}", recorded(&rec));
}

#[test]
#[ignore = "live: roda pwsh real"]
fn extract_dir_dotdot_throws() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let h = fake(&d, "7z.cmd", &rec, "", 0);
    let arc = d.work.join("a.7z");
    let o = run(
        &d,
        &[("7zip", &h)],
        &format!(
            "Expand-7zipArchive -Path '{}' -DestinationPath '{}' -ExtractDir '..\\x'",
            arc.display(),
            d.dest.display()
        ),
    );
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("inseguro"), "{}", o.out);
    assert!(!rec.exists(), "the helper must not run");
}

#[test]
#[ignore = "live: roda pwsh real"]
fn expand_7zip_removal_deletes_archive() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let h = fake(&d, "7z.cmd", &rec, "", 0);
    let arc = d.work.join("a.7z");
    std::fs::write(&arc, b"x").unwrap();
    let o = run(
        &d,
        &[("7zip", &h)],
        &format!(
            "Expand-7zipArchive -Path '{}' -DestinationPath '{}' -Removal",
            arc.display(),
            d.dest.display()
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert!(!arc.exists(), "the archive must be removed");
}

#[test]
#[ignore = "live: roda pwsh real"]
fn expand_7zip_nonzero_exit_throws() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let h = fake(&d, "7z.cmd", &rec, "", 2);
    let arc = d.work.join("a.7z");
    std::fs::write(&arc, b"x").unwrap();
    let o = run(
        &d,
        &[("7zip", &h)],
        &format!(
            "Expand-7zipArchive -Path '{}' -DestinationPath '{}' -Removal",
            arc.display(),
            d.dest.display()
        ),
    );
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("(código 2)"), "{}", o.out);
    assert!(arc.exists(), "a failed extraction keeps the archive");
}

#[test]
#[ignore = "live: roda pwsh real"]
fn msi_uses_lessmsi_never_msiexec() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let h = fake(&d, "lessmsi.cmd", &rec, &creates_file(&d), 0);
    let arc = d.work.join("a.msi");
    let o = run(
        &d,
        &[("lessmsi", &h)],
        &format!(
            "Expand-MsiArchive -Path '{}' -DestinationPath '{}'",
            arc.display(),
            d.dest.display()
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(
        recorded(&rec),
        format!("x \"{}\" \"{}\\\"", arc.display(), d.dest.display())
    );
    assert!(!PRELUDE.to_lowercase().contains("msiexec"));
}

#[test]
#[ignore = "live: roda pwsh real"]
fn inno_args_equal_rust_and_zero_files_throws() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let inst = d.work.join("setup.exe");
    for extract in [None, Some("sub"), Some("{tmp}")] {
        let h = fake(&d, "innounp.cmd", &rec, &creates_file(&d), 0);
        let ed = extract.map_or(String::new(), |e| format!(" -ExtractDir '{e}'"));
        let o = run(
            &d,
            &[("innounp", &h)],
            &format!(
                "Expand-InnoArchive -Path '{}' -DestinationPath '{}'{ed}",
                inst.display(),
                d.dest.display()
            ),
        );
        assert_eq!(o.code, 0, "{extract:?}: {}", o.out);
        let want = inno_args(&inst, &d.dest, extract).unwrap().join(" ");
        assert_eq!(recorded(&rec).replace('"', ""), want, "{extract:?}");
        std::fs::remove_dir_all(&d.dest).unwrap();
    }
    // exit 0 without extracting anything
    let h = fake(&d, "innounp.cmd", &rec, "", 0);
    let o = run(
        &d,
        &[("innounp", &h)],
        &format!(
            "Expand-InnoArchive -Path '{}' -DestinationPath '{}'",
            inst.display(),
            d.dest.display()
        ),
    );
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("nenhum arquivo extraído"), "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn dark_args() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let h = fake(&d, "dark.cmd", &rec, "", 0);
    let arc = d.work.join("a.exe");
    let o = run(
        &d,
        &[("dark", &h)],
        &format!(
            "Expand-DarkArchive -Path '{}' -DestinationPath '{}'",
            arc.display(),
            d.dest.display()
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(
        recorded(&rec),
        format!("-nologo -x \"{}\" \"{}\"", d.dest.display(), arc.display())
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn zip_native() {
    use std::io::Write;
    let d = dirs();
    let zp = d.work.join("a.zip");
    let mut buf = Vec::new();
    {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("a.txt", o).unwrap();
        z.write_all(b"A").unwrap();
        z.start_file("sub/b.txt", o).unwrap();
        z.write_all(b"B").unwrap();
        z.finish().unwrap();
    }
    std::fs::write(&zp, &buf).unwrap();
    let o = run(
        &d,
        &[],
        &format!(
            "Expand-ZipArchive -Path '{}' -DestinationPath '{}'",
            zp.display(),
            d.dest.display()
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(std::fs::read(d.dest.join("a.txt")).unwrap(), b"A");
    assert_eq!(
        std::fs::read(d.dest.join("sub").join("b.txt")).unwrap(),
        b"B"
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn invoke_external_command_codes() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let ok = fake(&d, "ok.cmd", &rec, "", 0);
    let three = fake(&d, "three.cmd", &rec, "", 3);
    let o = run(
        &d,
        &[],
        &format!(
            "Invoke-ExternalCommand '{}'\nInvoke-ExternalCommand '{}'\n\
             Invoke-ExternalCommand '{}' -ContinueExitCodes @{{3='ok'}}",
            ok.display(),
            three.display(),
            three.display()
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    let l: Vec<String> = lines(&o)
        .into_iter()
        .filter(|l| l == "True" || l == "False")
        .collect();
    assert_eq!(l, vec!["True", "False", "True"], "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn invoke_external_command_runas_throws() {
    let d = dirs();
    // a file that does not exist: if the refusal were gone nothing could be elevated
    let missing = d.work.join("nao-existe.exe");
    let o = run(
        &d,
        &[],
        &format!("Invoke-ExternalCommand '{}' -RunAs", missing.display()),
    );
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("elevação não suportada"), "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn invoke_external_command_refuses_windows_installer() {
    let d = dirs();
    let o = run(&d, &[], "Invoke-ExternalCommand 'msiexec.exe' @('/?')");
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("Windows Installer"), "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn missing_helper_throws_named() {
    let d = dirs();
    let arc = d.work.join("a.7z");
    let body = format!(
        "Expand-7zipArchive -Path '{}' -DestinationPath '{}'",
        arc.display(),
        d.dest.display()
    );
    let o = run(&d, &[], &body);
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("auxiliar '7zip' não instalado"), "{}", o.out);
    // listed but the file does not exist
    let gone = d.work.join("7z.exe");
    let o = run(&d, &[("7zip", &gone)], &body);
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("auxiliar '7zip' não instalado"), "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn persist_data_moves_and_links() {
    let d = dirs();
    let app = d.root.join("apps").join("app").join("1.0");
    let pdir = d.root.join("persist").join("app");
    std::fs::create_dir_all(app.join("data")).unwrap();
    std::fs::write(app.join("data").join("f.txt"), b"F").unwrap();
    std::fs::create_dir_all(&pdir).unwrap();
    let o = run(
        &d,
        &[],
        &format!(
            "$dir = '{0}'\n$manifest = [pscustomobject]@{{ persist = 'data' }}\n\
             persist_data $manifest $dir '{1}' | Out-Null\n\
             (Get-Item -LiteralPath '{0}\\data').LinkType\n\
             Get-Content -LiteralPath '{0}\\data\\f.txt'\n\
             Test-Path -LiteralPath '{1}\\data\\f.txt'\n\
             unlink_persist_data $manifest $dir\n\
             Test-Path -LiteralPath '{0}\\data'\n\
             Test-Path -LiteralPath '{1}\\data\\f.txt'",
            app.display(),
            pdir.display()
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    let l: Vec<String> = lines(&o)
        .into_iter()
        .filter(|l| !l.starts_with("Persisting"))
        .collect();
    assert_eq!(
        l,
        vec!["Junction", "F", "True", "False", "True"],
        "{}",
        o.out
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn persist_rejects_paths_outside() {
    let d = dirs();
    let app = d.root.join("apps").join("app").join("1.0");
    std::fs::create_dir_all(&app).unwrap();
    let pdir = d.root.join("persist").join("app");
    for def in [
        "'..\\x'",
        ",@('data', '..\\y')",
        "'C:\\x'",
        "@('ok', '..\\z')",
    ] {
        let o = run(
            &d,
            &[],
            &format!(
                "$dir = '{}'\n$manifest = [pscustomobject]@{{ persist = {def} }}\n\
                 persist_data $manifest $dir '{}'",
                app.display(),
                pdir.display()
            ),
        );
        assert_ne!(o.code, 0, "{def}: {}", o.out);
        assert!(o.out.contains("inseguro"), "{def}: {}", o.out);
    }
    // nothing was created: every definition is checked before the first link
    assert_eq!(std::fs::read_dir(&app).unwrap().count(), 0);
    assert!(!pdir.exists());
}

#[test]
#[ignore = "live: roda pwsh real"]
fn inno_installer_inside_destination_does_not_count() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    std::fs::create_dir_all(&d.dest).unwrap();
    let inst = d.dest.join("setup.exe");
    std::fs::write(&inst, b"x").unwrap();
    let h = fake(&d, "innounp.cmd", &rec, "", 0);
    let o = run(
        &d,
        &[("innounp", &h)],
        &format!(
            "Expand-InnoArchive -Path '{}' -DestinationPath '{}'",
            inst.display(),
            d.dest.display()
        ),
    );
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("nenhum arquivo extraído"), "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn dark_manifest_names_cannot_leave_their_folder() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let h = fake(&d, "dark.cmd", &rec, "", 0);
    let arc = d.work.join("a.exe");
    let (ux, ac) = (d.dest.join("UX"), d.dest.join("AttachedContainer"));
    std::fs::create_dir_all(&ux).unwrap();
    std::fs::create_dir_all(&ac).unwrap();
    std::fs::write(ac.join("a0"), b"0").unwrap();
    std::fs::write(ux.join("a"), b"a").unwrap();
    std::fs::write(ac.join("p"), b"p").unwrap();
    std::fs::write(d.dest.join("x"), b"outside-ux").unwrap();
    std::fs::write(d.dest.join("x2"), b"outside-payload").unwrap();
    std::fs::write(
        ux.join("manifest.xml"),
        "<BurnManifest><UX><Payload SourcePath=\"a\" FilePath=\"b\"/>\
         <Payload SourcePath=\"..\\x\" FilePath=\"y\"/></UX>\
         <Payload SourcePath=\"p\" FilePath=\"q\"/>\
         <Payload SourcePath=\"..\\x2\" FilePath=\"y2\"/></BurnManifest>",
    )
    .unwrap();
    let o = run(
        &d,
        &[("dark", &h)],
        &format!(
            "Expand-DarkArchive -Path '{}' -DestinationPath '{}'",
            arc.display(),
            d.dest.display()
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    // safe entries are renamed, the ones that leave their folder are skipped
    assert!(ux.join("b").exists() && ac.join("q").exists(), "{}", o.out);
    assert!(d.dest.join("x").exists() && !d.dest.join("y").exists());
    assert!(d.dest.join("x2").exists() && !d.dest.join("y2").exists());
}

#[test]
#[ignore = "live: roda pwsh real"]
fn inner_tar_name_cannot_leave_destination() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    // lists "..\a.tar"; any call whose arguments mention a.tar (the recursion) exits 9
    let extra = "echo %* | findstr /c:\"a.tar\" >nul && exit /b 9\r\necho ..\\a.tar";
    let h = fake(&d, "7z.cmd", &rec, extra, 0);
    let arc = d.work.join("x.tgz");
    let o = run(
        &d,
        &[("7zip", &h)],
        &format!(
            "Expand-7zipArchive -Path '{}' -DestinationPath '{}'",
            arc.display(),
            d.dest.display()
        ),
    );
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("inseguro"), "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn safe_rel_refuses_reserved_empty_and_trailing() {
    let d = dirs();
    let cases = [
        "CON",
        "a\\nul.txt",
        "COM1",
        "lpt9.x",
        "a.",
        "a ",
        "",
        "..\\x",
        "C:x",
        "\\x",
        "a\\..\\b",
        "sub",
        "a\\b.txt",
        "{app}\\x",
        "console",
        "com10",
    ];
    let list = cases
        .iter()
        .map(|c| format!("'{c}'"))
        .collect::<Vec<_>>()
        .join(",");
    let o = run(
        &d,
        &[],
        &format!(
            "foreach ($c in @({list})) {{ try {{ [void](__wr_safe_rel $c); 'ok' }} catch {{ 'refused' }} }}"
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    let want: Vec<&str> = std::iter::repeat_n("refused", 11)
        .chain(std::iter::repeat_n("ok", 5))
        .collect();
    assert_eq!(lines(&o), want, "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn expand_7zip_removal_deletes_split_parts_only() {
    let d = dirs();
    let rec = d.work.join("rec.txt");
    let h = fake(&d, "7z.cmd", &rec, "", 0);
    let w = &d.work;
    for n in [
        "a.7z.001",
        "a.7z.002",
        "b.7z.001",
        "x.part1.rar",
        "x.part2.rar",
        "y.part1.rar",
    ] {
        std::fs::write(w.join(n), b"x").unwrap();
    }
    for first in ["a.7z.001", "x.part1.rar"] {
        let o = run(
            &d,
            &[("7zip", &h)],
            &format!(
                "Expand-7zipArchive -Path '{}' -DestinationPath '{}' -Removal",
                w.join(first).display(),
                d.dest.display()
            ),
        );
        assert_eq!(o.code, 0, "{}", o.out);
    }
    let left = |n: &str| w.join(n).exists();
    assert!(!left("a.7z.001") && !left("a.7z.002"));
    assert!(!left("x.part1.rar") && !left("x.part2.rar"));
    assert!(
        left("b.7z.001") && left("y.part1.rar"),
        "unrelated parts stay"
    );
}
