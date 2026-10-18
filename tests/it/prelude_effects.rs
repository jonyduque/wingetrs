#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 153). Prelude group 2 (intercepted global effects) and the
//! `effects.jsonl` parser. Tests that run a real PowerShell use synthetic scripts in a temp root.

use std::path::{Path, PathBuf};
use std::process::Command;

use wingetrs::install::prelude::{CONTRACT_VARS, PRELUDE};
use wingetrs::install::script_effects::{EffectsError, ScriptEffect, parse_effects};

struct Out {
    code: i32,
    out: String,
    effects: Vec<ScriptEffect>,
}

/// Temp layout: `work` holds the scripts, `root` is `$__wr_root`, `sc` is `$__wr_shortcut_root`.
struct Dirs {
    _t: tempfile::TempDir,
    work: PathBuf,
    root: PathBuf,
    sc: PathBuf,
}

fn dirs() -> Dirs {
    let t = tempfile::tempdir().unwrap();
    let work = t.path().join("work");
    let root = t.path().join("root");
    let sc = t.path().join("startmenu").join("wingetrs Apps");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    Dirs {
        _t: t,
        work,
        root,
        sc,
    }
}

fn pwsh() -> String {
    wingetrs::bridge::find_pwsh(std::env::var_os("PATH").as_deref())
}

/// Runs `body` after dot-sourcing the prelude; `seed` is the JSON text of `$__wr_env_seed`.
fn run_seed(d: &Dirs, seed: Option<&str>, body: &str) -> Out {
    let work: &Path = &d.work;
    std::fs::write(work.join("prelude.ps1"), PRELUDE).unwrap();
    let effects = work.join("effects.jsonl");
    let mut s = String::from("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n");
    s += &format!("$__wr_root = '{}'\n", d.root.display());
    s += &format!("$__wr_effects = '{}'\n", effects.display());
    match seed {
        Some(j) => s += &format!("$__wr_env_seed = '{}'\n", j.replace('\'', "''")),
        None => s += "$__wr_env_seed = $null\n",
    }
    s += &format!("$__wr_shortcut_root = '{}'\n", d.sc.display());
    s += "$__wr_helpers = @{}\n";
    for v in CONTRACT_VARS {
        s += &format!("if (-not (Test-Path variable:{v})) {{ throw 'missing {v}' }}\n");
    }
    s += &format!(". '{}'\n{body}\n", work.join("prelude.ps1").display());
    let script = work.join("t.ps1");
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
    let effects = match std::fs::read(&effects) {
        Ok(b) => {
            assert!(
                !b.starts_with(&[0xEF, 0xBB, 0xBF]),
                "effects.jsonl has a BOM"
            );
            assert!(!b.contains(&b'\r'), "effects.jsonl has CR");
            parse_effects(std::str::from_utf8(&b).unwrap()).unwrap()
        }
        Err(_) => Vec::new(),
    };
    let _ = std::fs::remove_file(work.join("effects.jsonl"));
    Out {
        code: o.status.code().unwrap_or(-1),
        out,
        effects,
    }
}

fn run(d: &Dirs, body: &str) -> Out {
    run_seed(d, None, body)
}

fn lines(o: &Out) -> Vec<String> {
    o.out.lines().map(|l| l.trim_end().to_string()).collect()
}

fn s(v: &str) -> String {
    v.to_string()
}

#[test]
#[ignore = "live: roda pwsh real"]
fn set_env_records_and_overlay() {
    let d = dirs();
    let o = run(&d, "Set-EnvVar 'A' 'v'; Get-EnvVar 'A'");
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec!["v"], "{}", o.out);
    assert_eq!(
        o.effects,
        vec![ScriptEffect::SetEnv {
            name: s("A"),
            value: Some(s("v"))
        }]
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn set_env_null_removes() {
    let d = dirs();
    let o = run(
        &d,
        "Set-EnvVar 'A' $null; if ($null -eq (Get-EnvVar 'A')) { 'null' }",
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec!["null"], "{}", o.out);
    assert_eq!(
        o.effects,
        vec![ScriptEffect::SetEnv {
            name: s("A"),
            value: None
        }]
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn get_env_from_seed_and_unknown_throws() {
    let d = dirs();
    let seed = r#"{"JAVA_HOME":"C:\\J","NUL":null}"#;
    let o = run_seed(
        &d,
        Some(seed),
        "Get-EnvVar 'JAVA_HOME'; Get-EnvVar 'java_home'; if ($null -eq (Get-EnvVar 'NUL')) { 'null' }",
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec![r"C:\J", r"C:\J", "null"], "{}", o.out);
    assert!(o.effects.is_empty());
    // the session overlay wins over the seed
    let o = run_seed(
        &d,
        Some(seed),
        "Set-EnvVar 'JAVA_HOME' 'D:\\K'; Get-EnvVar 'JAVA_HOME'",
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec![r"D:\K"], "{}", o.out);
    let o = run_seed(&d, Some(seed), "Get-EnvVar 'ZZ'");
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(
        o.out.contains("n\u{e3}o foi pr\u{e9}-carregada"),
        "{}",
        o.out
    );
    let o = run(&d, "Get-EnvVar 'ZZ'");
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(
        o.out.contains("n\u{e3}o foi pr\u{e9}-carregada"),
        "{}",
        o.out
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn global_throws_for_each() {
    let d = dirs();
    for body in [
        "Set-EnvVar a b -Global",
        "Get-EnvVar a -Global",
        "Add-Path x -Global",
        "Remove-Path x -Global",
        "add_first_in_path x $true",
        "remove_from_path x $true",
        "env a $true",
        "env a $true v",
        "shim x $true",
        "shortcut_folder $true",
        "startmenu_shortcut x y '' $null $true",
    ] {
        let o = run(&d, body);
        assert_ne!(o.code, 0, "{body}: {}", o.out);
        assert!(
            o.out.contains("global n\u{e3}o suportado"),
            "{body}: {}",
            o.out
        );
        assert!(o.effects.is_empty(), "{body}");
    }
}

#[test]
#[ignore = "live: roda pwsh real"]
fn add_path_records_and_updates_session() {
    let d = dirs();
    let dir = d.root.display().to_string();
    let o = run(
        &d,
        &format!("Add-Path -Path \"{dir}\\bin\"; $env:PATH.Split(';')[0]"),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec![format!(r"{dir}\bin")], "{}", o.out);
    assert_eq!(
        o.effects,
        vec![ScriptEffect::AddPath {
            var: s("PATH"),
            entries: vec![format!(r"{dir}\bin")],
            prepend: true,
            force: false
        }]
    );
    // several entries in one call
    let o = run(&d, "Add-Path -Path 'C:\\a','C:\\b'");
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(
        o.effects,
        vec![ScriptEffect::AddPath {
            var: s("PATH"),
            entries: vec![s(r"C:\a"), s(r"C:\b")],
            prepend: true,
            force: false
        }]
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn add_path_target_env_var() {
    let d = dirs();
    let o = run(
        &d,
        "$before = $env:PATH; Add-Path -Path 'C:\\mods' -TargetEnvVar 'PSModulePath'; $env:PSModulePath.Split(';')[0]; ($env:PATH -eq $before)",
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec![r"C:\mods", "True"], "{}", o.out);
    assert_eq!(
        o.effects,
        vec![ScriptEffect::AddPath {
            var: s("PSModulePath"),
            entries: vec![s(r"C:\mods")],
            prepend: true,
            force: false
        }]
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn remove_path_records() {
    let d = dirs();
    let o = run(
        &d,
        "$env:PATH = 'C:\\keep;C:\\gone'; Remove-Path -Path 'C:\\gone'; $env:PATH",
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec![r"C:\keep"], "{}", o.out);
    assert_eq!(
        o.effects,
        vec![ScriptEffect::RemovePath {
            var: s("PATH"),
            entries: vec![s(r"C:\gone")]
        }]
    );
    let o = run(&d, "Remove-Path -Path 'C:\\m' -TargetEnvVar 'PSModulePath'");
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(
        o.effects,
        vec![ScriptEffect::RemovePath {
            var: s("PSModulePath"),
            entries: vec![s(r"C:\m")]
        }]
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn deprecated_aliases() {
    let d = dirs();
    let dir = d.root.display().to_string();
    let o = run(
        &d,
        &format!(
            "add_first_in_path '{dir}' $false; remove_from_path '{dir}' $false; env 'X' $false 'v'; env 'X' $false"
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o).last().map(String::as_str), Some("v"), "{}", o.out);
    assert_eq!(
        o.effects,
        vec![
            ScriptEffect::AddPath {
                var: s("PATH"),
                entries: vec![dir.clone()],
                prepend: true,
                force: true
            },
            ScriptEffect::RemovePath {
                var: s("PATH"),
                entries: vec![dir.clone()]
            },
            ScriptEffect::SetEnv {
                name: s("X"),
                value: Some(s("v"))
            },
        ]
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn shim_records_without_touching_path() {
    let d = dirs();
    let dir = d.root.display().to_string();
    std::fs::write(d.root.join("x.exe"), b"MZ").unwrap();
    std::fs::write(d.root.join("Tool.cmd"), b"@echo").unwrap();
    let o = run(
        &d,
        &format!(
            "$before = $env:PATH; shim \"{dir}\\x.exe\" $false 'x'; shim \"{dir}\\Tool.cmd\" $false '' '--a b'; ($env:PATH -eq $before)"
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec!["True"], "{}", o.out);
    assert_eq!(
        o.effects,
        vec![
            ScriptEffect::Shim {
                path: format!(r"{dir}\x.exe"),
                name: s("x"),
                args: None
            },
            ScriptEffect::Shim {
                path: format!(r"{dir}\Tool.cmd"),
                name: s("tool"),
                args: Some(s("--a b"))
            },
        ]
    );
    assert!(!d.root.join("shims").exists(), "shim created the shims dir");
    // missing target aborts like Scoop, without recording
    let o = run(&d, &format!("shim \"{dir}\\nope.exe\" $false"));
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.effects.is_empty());
}

#[test]
#[ignore = "live: roda pwsh real"]
fn rm_shim_records() {
    let d = dirs();
    std::fs::create_dir_all(d.root.join("shims")).unwrap();
    std::fs::write(d.root.join("shims").join("x.shim"), b"path = x").unwrap();
    let o = run(&d, "rm_shim 'x' (shimdir $false) 'app'");
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(o.effects, vec![ScriptEffect::RmShim { name: s("x") }]);
    assert!(
        d.root.join("shims").join("x.shim").exists(),
        "rm_shim deleted a file"
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn shortcuts() {
    let d = dirs();
    let dir = d.root.display().to_string();
    let sc = d.sc.display().to_string();
    std::fs::write(d.root.join("app.exe"), b"MZ").unwrap();
    std::fs::write(d.root.join("app.ico"), b"ico").unwrap();
    let o = run(
        &d,
        &format!(
            "shortcut_folder $false\nstartmenu_shortcut \"{dir}\\app.exe\" 'App' '--x' \"{dir}\\app.ico\" $false\nstartmenu_shortcut \"{dir}\\app.exe\" 'Sub\\App2' '' $null $false"
        ),
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec![sc.clone()], "{}", o.out);
    assert!(d.sc.is_dir(), "shortcut_folder must create the folder");
    assert_eq!(
        o.effects,
        vec![
            ScriptEffect::Shortcut {
                target: format!(r"{dir}\app.exe"),
                name: s("App"),
                args: Some(s("--x")),
                icon: Some(format!(r"{dir}\app.ico"))
            },
            ScriptEffect::Shortcut {
                target: format!(r"{dir}\app.exe"),
                name: s(r"Sub\App2"),
                args: None,
                icon: None
            },
        ]
    );
    // the folder exists, so a created .lnk would be visible here
    assert_eq!(std::fs::read_dir(&d.sc).unwrap().count(), 0);
}

#[test]
fn parser_limits_and_errors() {
    let ok = r#"{"op":"rm_shim","name":"a"}"#;
    let text = format!("{ok}\n{ok}\n{{\"op\":\"zz\"}}\n");
    assert_eq!(parse_effects(&text), Err(EffectsError::UnknownOp(3)));
    let text = format!("{ok}\n{{\"op\":\n{ok}\n");
    assert_eq!(parse_effects(&text), Err(EffectsError::Invalid(2)));
    let text = format!("{ok}\n").repeat(1001);
    assert_eq!(parse_effects(&text), Err(EffectsError::TooMany));
    let text = format!("{ok}\n").repeat(1000);
    assert_eq!(parse_effects(&text).unwrap().len(), 1000);
    // exact limit: a whole line of 65 536 bytes is accepted, 65 537 is refused
    let wrap = r#"{"op":"rm_shim","name":""}"#.len();
    let at_limit = format!(
        "{{\"op\":\"rm_shim\",\"name\":\"{}\"}}",
        "a".repeat(65_536 - wrap)
    );
    assert_eq!(at_limit.len(), 65_536);
    assert_eq!(parse_effects(&at_limit).unwrap().len(), 1);
    let long = format!(
        "{{\"op\":\"rm_shim\",\"name\":\"{}\"}}",
        "a".repeat(65_537 - wrap)
    );
    assert_eq!(long.len(), 65_537);
    assert_eq!(parse_effects(&long), Err(EffectsError::TooLong(1)));
    // every key is required (a nullable one too) and unknown keys are refused
    for bad in [
        r#"{"op":"set_env","name":"PATH"}"#,
        r#"{"op":"set_env","name":"A","value":"v","extra":1}"#,
        r#"{"op":"add_path","var":"PATH","entries":["C:/a"],"prepend":true}"#,
        r#"{"op":"add_path","var":"PATH","entries":["C:/a"],"prepend":true,"force":false,"x":0}"#,
        r#"{"op":"remove_path","var":"PATH"}"#,
        r#"{"op":"remove_path","var":"PATH","entries":[],"force":true}"#,
        r#"{"op":"shim","path":"C:/x.exe","name":"x"}"#,
        r#"{"op":"shim","path":"C:/x.exe","name":"x","args":null,"global":true}"#,
        r#"{"op":"rm_shim"}"#,
        r#"{"op":"rm_shim","name":"x","app":"y"}"#,
        r#"{"op":"shortcut","target":"C:/a.exe","name":"A","args":null}"#,
        r#"{"op":"shortcut","target":"C:/a.exe","name":"A","icon":null}"#,
        r#"{"op":"shortcut","target":"C:/a.exe","name":"A","args":null,"icon":null,"dir":"x"}"#,
    ] {
        assert_eq!(parse_effects(bad), Err(EffectsError::Invalid(1)), "{bad}");
    }
    let text = format!("\u{feff}{ok}\n");
    assert_eq!(
        parse_effects(&text),
        Ok(vec![ScriptEffect::RmShim { name: s("a") }])
    );
    assert_eq!(parse_effects(""), Ok(vec![]));
    // empty lines are skipped but still counted for the line number
    let text = format!("\n{ok}\r\n\n{{\"op\":\"zz\"}}");
    assert_eq!(parse_effects(&text), Err(EffectsError::UnknownOp(4)));
    // missing op, non-object and a known op missing a required field are invalid
    assert_eq!(
        parse_effects(r#"{"name":"a"}"#),
        Err(EffectsError::Invalid(1))
    );
    assert_eq!(parse_effects("[1]"), Err(EffectsError::Invalid(1)));
    assert_eq!(
        parse_effects(r#"{"op":"shim","name":"a"}"#),
        Err(EffectsError::Invalid(1))
    );
    // every op decodes
    let all = [
        r#"{"op":"set_env","name":"A","value":"v"}"#,
        r#"{"op":"set_env","name":"A","value":null}"#,
        r#"{"op":"add_path","var":"PATH","entries":["C:\\a"],"prepend":true,"force":false}"#,
        r#"{"op":"remove_path","var":"PATH","entries":["C:\\a","C:\\b"]}"#,
        r#"{"op":"shim","path":"C:\\x.exe","name":"x","args":null}"#,
        r#"{"op":"rm_shim","name":"x"}"#,
        r#"{"icon":null,"name":"App","op":"shortcut","target":"C:\\a.exe","args":"-y"}"#,
    ]
    .join("\n");
    assert_eq!(
        parse_effects(&all),
        Ok(vec![
            ScriptEffect::SetEnv {
                name: s("A"),
                value: Some(s("v"))
            },
            ScriptEffect::SetEnv {
                name: s("A"),
                value: None
            },
            ScriptEffect::AddPath {
                var: s("PATH"),
                entries: vec![s(r"C:\a")],
                prepend: true,
                force: false
            },
            ScriptEffect::RemovePath {
                var: s("PATH"),
                entries: vec![s(r"C:\a"), s(r"C:\b")]
            },
            ScriptEffect::Shim {
                path: s(r"C:\x.exe"),
                name: s("x"),
                args: None
            },
            ScriptEffect::RmShim { name: s("x") },
            ScriptEffect::Shortcut {
                target: s(r"C:\a.exe"),
                name: s("App"),
                args: Some(s("-y")),
                icon: None
            },
        ])
    );
}

#[test]
fn prelude_still_ascii() {
    assert!(PRELUDE.is_ascii());
}

fn add(var: &str, entry: &str, force: bool) -> ScriptEffect {
    ScriptEffect::AddPath {
        var: s(var),
        entries: vec![s(entry)],
        prepend: true,
        force,
    }
}

#[test]
#[ignore = "live: roda pwsh real"]
fn add_path_force_flag() {
    let d = dirs();
    let o = run(
        &d,
        r"$env:PATH = 'C:\x;C:\a'
Add-Path -Path 'C:\a'; $env:PATH
add_first_in_path 'C:\a' $false; $env:PATH
Add-Path -Path 'C:\x' -Force; $env:PATH",
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(
        lines(&o),
        vec![r"C:\x;C:\a", r"C:\a;C:\x", r"C:\x;C:\a"],
        "{}",
        o.out
    );
    assert_eq!(
        o.effects,
        vec![
            add("PATH", r"C:\a", false),
            add("PATH", r"C:\a", true),
            add("PATH", r"C:\x", true),
        ]
    );
}

#[test]
#[ignore = "live: roda pwsh real"]
fn path_overlay_and_passthru() {
    let d = dirs();
    let seed = r#"{"PATH":"C:\\a;C:\\b","EMPTY":null}"#;
    // the session PATH differs from the stored value: -PassThru reports the stored one
    let o = run_seed(
        &d,
        Some(seed),
        r"$env:PATH = 'C:\s'
'p1=' + (Remove-Path -Path 'C:\b' -PassThru)
'g1=' + (Get-EnvVar 'PATH')
'p2=' + (Remove-Path -Path 'C:\s' -PassThru)
Add-Path -Path 'C:\n'; 'g2=' + (Get-EnvVar 'PATH')
Add-Path -Path 'C:\a'; 'g3=' + (Get-EnvVar 'PATH')
add_first_in_path 'C:\a' $false; 'g4=' + (Get-EnvVar 'PATH')
Remove-Path -Path 'C:\a','C:\n'; if ($null -eq (Get-EnvVar 'PATH')) { 'g5=null' }
Add-Path -Path 'C:\e' -TargetEnvVar 'EMPTY'; 'g6=' + (Get-EnvVar 'EMPTY')
Add-Path -Path 'C:\q' -TargetEnvVar 'UNK'; try { Get-EnvVar 'UNK' } catch { 'unk' }
if ($null -eq (Remove-Path -Path 'C:\q' -TargetEnvVar 'UNK2' -PassThru)) { 'p3=null' }",
    );
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(
        lines(&o),
        vec![
            r"p1=C:\b",
            r"g1=C:\a",
            "p2=",
            r"g2=C:\n;C:\a",
            r"g3=C:\n;C:\a",
            r"g4=C:\a;C:\n",
            "g5=null",
            r"g6=C:\e",
            "unk",
            "p3=null",
        ],
        "{}",
        o.out
    );
    assert_eq!(o.effects.len(), 9);
}

/// `shim` only records the effect: the executor keeps the shims dir on the PATH
/// (`EnsureShimsOnPath`); the prelude never puts the target's folder on it.
#[test]
fn shim_body_never_touches_path() {
    let start = PRELUDE.find("function shim(").expect("shim defined");
    let body = &PRELUDE[start..];
    let end = body.find("\n}").expect("shim body closed");
    let body = &body[..end];
    assert!(body.contains("__wr_effect @{ op = 'shim'"), "{body}");
    for bad in ["Add-Path", "add_first_in_path", "env:PATH", "Set-EnvVar"] {
        assert!(!body.contains(bad), "shim body contains {bad}: {body}");
    }
}
