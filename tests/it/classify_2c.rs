//! Phase 2c classification of Scoop manifests (Task 142).
use std::collections::BTreeSet;

use wingetrs::install::classify_2c::{
    Classification, SubCase, classify, commands_in, script_commands,
};

fn class(raw: &str) -> Option<Classification> {
    match classify(raw) {
        Ok(c) => c,
        Err(e) => panic!("valid JSON in test: {e}"),
    }
}

fn set(cases: &[SubCase]) -> BTreeSet<SubCase> {
    cases.iter().copied().collect()
}

#[test]
fn none_for_simple_manifest() {
    let raw = r#"{"version":"1.0","url":"https://x/a.zip","hash":"00","bin":"a.exe"}"#;
    assert_eq!(class(raw), None);
    // `innosetup: false` and null keys are not phase 2c either.
    let raw = r#"{"version":"1.0","url":"https://x/a.zip","innosetup":false,"post_install":null}"#;
    assert_eq!(class(raw), None);
}

#[test]
fn one_fixture_per_layer() {
    let cases = [
        (r#"{"psmodule":{"name":"m"}}"#, SubCase::Psm),
        (r#"{"innosetup":true}"#, SubCase::Inno),
        (
            r#"{"installer":{"file":"setup.exe","args":["/S"]}}"#,
            SubCase::File,
        ),
        (r#"{"installer":{"file":"install.ps1"}}"#, SubCase::Ps1),
        (r#"{"pre_install":"Add-Path $dir"}"#, SubCase::Hook),
        (r#"{"post_uninstall":["Remove-Item x"]}"#, SubCase::Hook),
        (
            r#"{"uninstaller":{"script":"Remove-Item x"}}"#,
            SubCase::Hook,
        ),
        (
            r#"{"uninstaller":{"file":"setup.cmd","args":"/u"}}"#,
            SubCase::File,
        ),
        (r#"{"uninstaller":{"file":"Uninstall.PS1"}}"#, SubCase::Ps1),
        (r#"{"installer":{"args":["/S"]}}"#, SubCase::File),
    ];
    for (raw, want) in cases {
        let c = class(raw).unwrap_or_else(|| panic!("{raw}: expected {want:?}"));
        assert_eq!(c.cases, set(&[want]), "{raw}");
        assert_eq!(c.worst, want, "{raw}");
    }
}

#[test]
fn variant_only_key_counts() {
    let raw = r#"{"version":"1","architecture":{"64bit":{"url":"https://x/a.zip","post_install":"Add-Path $dir"},"32bit":{"url":"https://x/b.zip"}}}"#;
    let c = class(raw).expect("variant key must count");
    assert_eq!(c.worst, SubCase::Hook);
    assert_eq!(c.cases, set(&[SubCase::Hook]));
}

#[test]
fn worst_wins() {
    let raw = r#"{"innosetup":true,"post_install":"Add-Path $dir"}"#;
    let c = class(raw).expect("2c");
    assert_eq!(c.worst, SubCase::Hook);
    assert_eq!(c.cases, set(&[SubCase::Inno, SubCase::Hook]));
    // Order of risk: psm < inno < file < ps1 < hook.
    assert!(SubCase::Psm < SubCase::Inno);
    assert!(SubCase::Inno < SubCase::File);
    assert!(SubCase::File < SubCase::Ps1);
    assert!(SubCase::Ps1 < SubCase::Hook);
    let raw =
        r#"{"psmodule":{"name":"m"},"architecture":{"64bit":{"installer":{"file":"x.ps1"}}}}"#;
    let c = class(raw).expect("2c");
    assert_eq!(c.worst, SubCase::Ps1);
    assert_eq!(c.cases, set(&[SubCase::Psm, SubCase::Ps1]));
}

#[test]
fn installer_file_ps1_vs_exe() {
    let c = class(r#"{"installer":{"file":"x.ps1"}}"#).expect("2c");
    assert_eq!(c.cases, set(&[SubCase::Ps1]));
    let c = class(r#"{"installer":{"file":"setup.exe","args":"/S"}}"#).expect("2c");
    assert_eq!(c.cases, set(&[SubCase::File]));
    let c = class(r#"{"installer":{"script":"Add-Path $dir"}}"#).expect("2c");
    assert_eq!(c.cases, set(&[SubCase::Hook]));
    // Script and file together: Scoop runs both.
    let c = class(r#"{"installer":{"script":"x","file":"setup.msi"}}"#).expect("2c");
    assert_eq!(c.cases, set(&[SubCase::File, SubCase::Hook]));
}

#[test]
fn commands_skip_comments_strings_herestrings() {
    let script = [
        "# Add-Path x",
        "'shim'",
        "$text = @'",
        "don't",
        "ensure $dir",
        "'@",
        "Add-Path $dir",
        "Remove-Item x | Out-Null",
    ];
    let raw = serde_json::json!({ "post_install": script }).to_string();
    assert_eq!(
        script_commands(&raw),
        vec!["Add-Path", "Remove-Item", "Out-Null"]
    );
}

#[test]
fn commands_cover_all_script_places_and_separators() {
    let raw = serde_json::json!({
        "pre_install": "Get-Foo; Set-Bar",
        "installer": { "script": ["if (Test-Path $x) { New-Item y }", "$a = Join-Path $dir 'x'"] },
        "uninstaller": { "script": "a && b || c" },
        "architecture": { "64bit": { "post_uninstall": "@\"\nshim\n\"@\nRemove-Path" } },
        "pre_uninstall": "<# Hidden #> Visible; $h = @{ Key = 'v'; Other = 1 }; function Helper { x }",
    })
    .to_string();
    assert_eq!(
        script_commands(&raw),
        vec![
            "Get-Foo",
            "Set-Bar",
            "Test-Path",
            "New-Item",
            "Join-Path",
            "Visible",
            "x",
            "a",
            "b",
            "c",
            "Remove-Path",
        ]
    );
}

#[test]
fn invalid_json_is_err() {
    assert!(classify("{not json").is_err());
    assert!(script_commands("{not json").is_empty());
}

#[test]
fn installer_file_ps1_case_path_and_trailing_space() {
    for file in [r"bin\SETUP.PS1", "x.ps1 "] {
        let raw = serde_json::json!({ "installer": { "file": file } }).to_string();
        let c = class(&raw).unwrap_or_else(|| panic!("{file}: expected S-ps1"));
        assert_eq!(c.cases, set(&[SubCase::Ps1]), "{file}");
    }
}

#[test]
fn commands_inside_subexpressions() {
    let script = [
        r#"Write-Host "dir: $(appdir x) and `$(not-this)""#,
        "$v = $(versiondir y)",
        "$t = @\"",
        "path=$(currentdir z)",
        "\"@",
        "'$(literal q)'",
    ];
    let raw = serde_json::json!({ "post_install": script }).to_string();
    assert_eq!(
        script_commands(&raw),
        vec!["Write-Host", "appdir", "versiondir", "currentdir"]
    );
    // Deep nesting is capped, never a stack overflow.
    let deep = format!("\"{}\"", "$(\"".repeat(10_000));
    let raw = serde_json::json!({ "post_install": deep }).to_string();
    assert!(script_commands(&raw).is_empty());
}

#[test]
fn functions_defined_in_scripts_are_not_commands() {
    let raw = serde_json::json!({
        "pre_install": ["function CreateFile($n) { New-Item $n }", "CreateFile 'a'"],
        "post_install": ["createfile 'b'", "filter global:Pick { $_ }", "1 | Pick | Out-Null"],
    })
    .to_string();
    assert_eq!(script_commands(&raw), vec!["New-Item", "Out-Null"]);
}

#[test]
fn braced_variables_and_while_are_not_commands() {
    let raw = serde_json::json!({
        "post_install": ["${Env:ProgramFiles}/x | Out-Null", "while ($i) { Start-Sleep 1 }"],
    })
    .to_string();
    assert_eq!(script_commands(&raw), vec!["Out-Null", "Start-Sleep"]);
}

#[test]
fn hash_glued_to_a_word_is_not_a_comment() {
    // pwsh: `a#b` is one word; `=#` opens a comment, so `@'` there is no here-string.
    assert_eq!(commands_in("Foo a#b; Baz"), vec!["Foo", "Baz"]);
    let script = "$x =#@'\nStart-Process \"$dir\\x.exe\" -Verb RunAs\n'@'";
    assert_eq!(commands_in(script), vec!["Start-Process"]);
}
