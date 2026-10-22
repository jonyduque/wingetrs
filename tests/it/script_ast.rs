//! Task 227: facts of PowerShell's parser (wire format), the AST rules and their union with the
//! lexer. No PowerShell runs here: the facts come from literal wire text or a fake gate.
// The brief's literal `assert_eq!(.., false)` is kept as written.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::bool_assert_comparison
)]
use wingetrs::install::prelude::PRELUDE_FUNCTIONS;
use wingetrs::install::script_ast::{self, AstError, AstFacts, AstGate};
use wingetrs::install::script_scan::{self, CallRoots, FlagKind, Verdict};

fn flags(wire: &str) -> Vec<(FlagKind, Verdict)> {
    let facts = script_ast::decode(wire).expect("wire");
    script_ast::ast_flags(&facts, PRELUDE_FUNCTIONS)
        .into_iter()
        .map(|f| (f.kind, f.verdict))
        .collect()
}

struct Fake(std::collections::HashMap<String, Result<String, AstError>>);
impl AstGate for Fake {
    fn facts(&self, script: &str) -> Result<AstFacts, AstError> {
        match self.0.get(script) {
            Some(Ok(w)) => Ok(script_ast::decode(w).expect("wire")),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(AstFacts::default()),
        }
    }
}

fn fake(pairs: &[(&str, Result<&str, AstError>)]) -> Fake {
    Fake(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone().map(str::to_string)))
            .collect(),
    )
}

#[test]
fn decode_round_trip() {
    let f = script_ast::decode(
        "CMD\t3\tnone\tStringConstant\tB\tRemove-Item\ts:C:\\Windows\\x\nEND\t1\n",
    )
    .unwrap();
    assert_eq!(f.commands.len(), 1);
    assert_eq!(f.commands[0].line, 3);
    assert_eq!(f.commands[0].name.as_deref(), Some("Remove-Item"));
    assert_eq!(f.commands[0].chain, "B");
    assert_eq!(
        f.commands[0].args,
        vec![script_ast::Arg::Str(r"C:\Windows\x".into())]
    );
    assert_eq!(
        script_ast::decode("CMD\t%0A\n")
            .unwrap_err()
            .to_string()
            .is_empty(),
        false
    );
    assert_eq!(
        script_ast::decode("CMD\t1\tnone\tStringConstant\t.\ta%09b\n")
            .unwrap()
            .commands[0]
            .name
            .as_deref(),
        Some("a\tb")
    );
}

/// Every malformed record is an error, never a dropped fact.
#[test]
fn decode_refuses_malformed_records() {
    for wire in [
        "XYZ\t1\n",
        "CMD\t1\tbang\tStringConstant\t.\tx\n",
        "CMD\t1\tnone\tStringConstant\t.\tx\tz:y\n",
        "CMD\t1\tnone\tStringConstant\t.\tx\tnocolon\n",
        "CMD\t1\tnone\tStringConstant\n",
        "MEM\t1\t2\t.\t[x]\ty\n",
        "SET\t1\t.\n",
        "FN\t1\t.\tf\textra\n",
        "ERR\tx\tId\n",
        "USE\t1\n",
        "CMD\t1\tnone\tStringConstant\t.\ta%41b\n",
        "CMD\t1\tnone\tStringConstant\t.\ta%0\n",
    ] {
        assert!(
            matches!(script_ast::decode(wire), Err(AstError::Protocol(_))),
            "{wire:?}"
        );
    }
    let f = script_ast::decode(
        "MEM\t2\t1\tB\t[IO.File]\tWriteAllText\te:$env:x\t?:$a + 'b'\nSET\t3\tG\tglobal:x\nFN\t4\t.\tf\nUSE\t1\tModule\nERR\t9\tMissingEndCurlyBrace\n",
    )
    .unwrap();
    assert_eq!(f.members[0].target, "[IO.File]");
    assert!(f.members[0].is_static);
    assert_eq!(
        f.members[0].args,
        vec![
            script_ast::Arg::Expandable("$env:x".into()),
            script_ast::Arg::Other("$a + 'b'".into())
        ]
    );
    assert_eq!(f.assignments[0].name, "global:x");
    assert_eq!(f.functions[0].name, "f");
    assert_eq!(f.usings[0].kind, "module");
    assert_eq!(f.errors[0].line, 9);
}

#[test]
fn backtick_iex_is_dynamic() {
    let got = flags("CMD\t2\tnone\tStringConstant\t.\tiex\ts:x\n");
    assert!(
        got.contains(&(FlagKind::DynamicCode, Verdict::ConfirmStrong)),
        "{got:?}"
    );
}

#[test]
fn en_dash_runas_refused() {
    let got = flags("CMD\t1\tnone\tStringConstant\t.\tStart-Process\ts:notepad\tp:Verb\ts:RunAs\n");
    assert!(
        got.contains(&(FlagKind::Elevation, Verdict::Refuse)),
        "{got:?}"
    );
}

#[test]
fn job_block_is_live_code() {
    let got = flags(
        "CMD\t3\tnone\tStringConstant\t.\tStart-Job\t?:ScriptBlock\nCMD\t3\tnone\tStringConstant\tB\tRemove-Item\ts:C:\\Windows\\x\n",
    );
    assert!(
        got.contains(&(FlagKind::SystemWrite, Verdict::Refuse)),
        "{got:?}"
    );
}

#[test]
fn paren_head_is_computed() {
    let got = flags("CMD\t4\tamp\tParen\t.\t\ts:y\n");
    assert!(
        got.contains(&(FlagKind::CalledFile, Verdict::ConfirmStrong)),
        "{got:?}"
    );
}

#[test]
fn dead_global_branch_has_no_ast_flag() {
    assert!(flags("CMD\t8\tnone\tStringConstant\tG\tSet-ItemProperty\ts:HKLM:\\x\n").is_empty());
    for chain in ["BG", "GB"] {
        let got = flags(&format!(
            "CMD\t8\tnone\tStringConstant\t{chain}\tSet-ItemProperty\ts:HKLM:\\x\n"
        ));
        assert!(
            got.contains(&(FlagKind::Hklm, Verdict::Refuse)),
            "{chain}: {got:?}"
        );
    }
}

#[test]
fn syntax_error_refused() {
    assert_eq!(
        flags("ERR\t5\tMissingEndCurlyBrace\n"),
        vec![(FlagKind::SyntaxError, Verdict::Refuse)]
    );
}

#[test]
fn using_module_refused() {
    let got = flags("USE\t1\tmodule\n");
    assert!(
        got.contains(&(FlagKind::DynamicCode, Verdict::Refuse)),
        "{got:?}"
    );
    assert!(flags("USE\t1\tnamespace\n").is_empty());
}

#[test]
fn encoded_command_refused() {
    let got = flags("CMD\t1\tnone\tStringConstant\t.\tpwsh\tp:enc\ts:QQA=\n");
    assert!(
        got.contains(&(FlagKind::HiddenCode, Verdict::Refuse)),
        "{got:?}"
    );
}

#[test]
fn global_assignment_refused() {
    for wire in [
        "SET\t1\tB\tglobal\n",
        "CMD\t1\tnone\tStringConstant\t.\tsv\ts:global\ts:1\n",
    ] {
        let got = flags(wire);
        assert!(
            got.contains(&(FlagKind::GlobalName, Verdict::Refuse)),
            "{wire:?}: {got:?}"
        );
    }
}

#[test]
fn prelude_redefinition_refused() {
    let wire = format!("FN\t1\t.\t{}\n", PRELUDE_FUNCTIONS[0]);
    let got = flags(&wire);
    assert!(
        got.contains(&(FlagKind::PreludeRedefinition, Verdict::Refuse)),
        "{got:?}"
    );
}

/// One fact per rule of the table that the tests above do not reach, each alone.
#[test]
fn each_rule_has_its_fact() {
    let cases: &[(&str, FlagKind, Verdict)] = &[
        (
            "CMD\t1\tnone\tStringConstant\t.\tgsudo\ts:x\n",
            FlagKind::Elevation,
            Verdict::Refuse,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tGet-Item\te:HKEY_LOCAL_MACHINE\\$x\n",
            FlagKind::Hklm,
            Verdict::Refuse,
        ),
        (
            "MEM\t1\t1\t.\t[Microsoft.Win32.Registry]\tLocalMachine\n",
            FlagKind::Hklm,
            Verdict::Refuse,
        ),
        (
            "MEM\t1\t1\t.\t[System.IO.File]\tCopy\ts:a\te:$env:windir\\x\n",
            FlagKind::SystemWrite,
            Verdict::Refuse,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tC:\\Windows\\System32\\msiexec.exe\ts:/i\n",
            FlagKind::Msiexec,
            Verdict::Refuse,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tsaps\ts:msiexec.exe\n",
            FlagKind::Msiexec,
            Verdict::Refuse,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tscoop\ts:install\n",
            FlagKind::ScoopCommand,
            Verdict::Refuse,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tadd_bucket\ts:x\n",
            FlagKind::UnknownScoopFunction,
            Verdict::Refuse,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tiwr\ts:x\n",
            FlagKind::Download,
            Verdict::ConfirmStrong,
        ),
        (
            "MEM\t1\t0\t.\t$wc\tDownloadString\ts:x\n",
            FlagKind::Download,
            Verdict::ConfirmStrong,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tschtasks.exe\ts:/create\n",
            FlagKind::ScheduledTaskOrService,
            Verdict::ConfirmStrong,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tnal\ts:a\ts:b\n",
            FlagKind::Alias,
            Verdict::ConfirmStrong,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tcmd\ts:/C\ts:x\n",
            FlagKind::CalledFile,
            Verdict::ConfirmStrong,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tpowershell.exe\tp:Command\ts:x\n",
            FlagKind::CalledFile,
            Verdict::ConfirmStrong,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tStart-Process\ts:x.exe\n",
            FlagKind::CalledFile,
            Verdict::ConfirmStrong,
        ),
        (
            "MEM\t1\t1\t.\t[Diagnostics.Process]\tStart\ts:x\n",
            FlagKind::CalledFile,
            Verdict::ConfirmStrong,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tpowershell\tp:EncodedCommand\ts:QQA=\n",
            FlagKind::HiddenCode,
            Verdict::Refuse,
        ),
        (
            "MEM\t1\t1\t.\t[ScriptBlock]\tCreate\ts:x\n",
            FlagKind::DynamicCode,
            Verdict::ConfirmStrong,
        ),
        (
            "MEM\t1\t1\t.\t[System.Management.Automation.PowerShell]\tCreate\n",
            FlagKind::DynamicCode,
            Verdict::ConfirmStrong,
        ),
        (
            "MEM\t1\t1\t.\t[RunspaceFactory]\tCreateRunspace\n",
            FlagKind::DynamicCode,
            Verdict::ConfirmStrong,
        ),
        (
            "MEM\t1\t0\t.\t$ExecutionContext.InvokeCommand\tInvokeScript\ts:x\n",
            FlagKind::DynamicCode,
            Verdict::ConfirmStrong,
        ),
        (
            "MEM\t1\t1\t.\t[Reflection.Assembly]\tLoadFile\ts:x\n",
            FlagKind::DynamicCode,
            Verdict::Refuse,
        ),
        (
            "MEM\t1\t1\t.\t[Activator]\tCreateInstance\t?:$t\n",
            FlagKind::DynamicCode,
            Verdict::Refuse,
        ),
        (
            "MEM\t1\t1\t.\t[Environment]\tSetEnvironmentVariable\ts:a\ts:b\ts:User\n",
            FlagKind::EnvironmentVariable,
            Verdict::Refuse,
        ),
        (
            "MEM\t1\t1\t.\t[System.Environment]\tSetEnvironmentVariable\ts:a\ts:b\n",
            FlagKind::EnvironmentVariable,
            Verdict::ConfirmStrong,
        ),
        (
            "SET\t1\t.\tscript:global\n",
            FlagKind::GlobalName,
            Verdict::Refuse,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\t__wr_effect\ts:x\n",
            FlagKind::PreludeInternal,
            Verdict::Refuse,
        ),
        (
            "CMD\t1\tnone\tStringConstant\t.\tWrite-Output\te:$__wr_effects\n",
            FlagKind::PreludeInternal,
            Verdict::Refuse,
        ),
        (
            "MEM\t1\t0\t.\t$__wr\tx\n",
            FlagKind::PreludeInternal,
            Verdict::Refuse,
        ),
        (
            "SET\t1\t.\t__wr_effects\n",
            FlagKind::PreludeInternal,
            Verdict::Refuse,
        ),
    ];
    for (wire, kind, verdict) in cases {
        let got = flags(wire);
        assert!(got.contains(&(*kind, *verdict)), "{wire:?}: {got:?}");
    }
    // Look-alikes that are not the rule.
    for wire in [
        "CMD\t1\tnone\tStringConstant\t.\tsc\ts:x\ts:y\n",
        "CMD\t1\tnone\tStringConstant\t.\tWrite-Output\ts:runas2\n",
        "CMD\t1\tamp\tStringConstant\t.\tx.exe\n",
        "CMD\t1\tamp\tExpandableString\t.\t\n",
        "CMD\t1\tdot\tScriptBlock\t.\t\n",
        "MEM\t1\t1\t.\t[IO.File]\tReadAllText\ts:C:\\Windows\\x\n",
        "SET\t1\t.\tglobal:x\n",
        "FN\t1\t.\tmy_function\n",
    ] {
        assert!(flags(wire).is_empty(), "{wire:?}: {:?}", flags(wire));
    }
}

#[test]
fn gated_union_never_relaxes() {
    let (report, _) = script_scan::scan_gated(
        "Start-Process x -Verb RunAs",
        PRELUDE_FUNCTIONS,
        &CallRoots::default(),
        &Fake(Default::default()),
    );
    assert_eq!(report.verdict, Verdict::Refuse);
}

#[test]
fn gated_adds_ast_refusal() {
    let text = "i`ex 'x'";
    let iex = fake(&[(text, Ok("CMD\t1\tnone\tStringConstant\t.\tiex\ts:x\n"))]);
    let (report, _) = script_scan::scan_gated(text, PRELUDE_FUNCTIONS, &CallRoots::default(), &iex);
    assert!(
        report.flags.iter().any(|f| f.kind == FlagKind::DynamicCode),
        "{:?}",
        report.flags
    );
    let runas = fake(&[(
        text,
        Ok("CMD\t1\tnone\tStringConstant\t.\tStart-Process\tp:Verb\ts:RunAs\n"),
    )]);
    let (report, _) =
        script_scan::scan_gated(text, PRELUDE_FUNCTIONS, &CallRoots::default(), &runas);
    assert_eq!(report.verdict, Verdict::Refuse);
}

#[test]
fn gated_parser_error_refuses() {
    for e in [
        AstError::Timeout(10),
        AstError::NoPowerShell,
        AstError::Elevated,
    ] {
        let gate = fake(&[("Write-Output ok", Err(e.clone()))]);
        let (report, _) = script_scan::scan_gated(
            "Write-Output ok",
            PRELUDE_FUNCTIONS,
            &CallRoots::default(),
            &gate,
        );
        assert_eq!(report.verdict, Verdict::Refuse, "{e:?}");
        let failed: Vec<_> = report
            .flags
            .iter()
            .filter(|f| f.kind == FlagKind::ParserFailed)
            .collect();
        assert_eq!(failed.len(), 1, "{e:?}: {:?}", report.flags);
        assert_eq!(failed[0].detail, e.to_string());
    }
}

#[test]
fn gated_scans_called_files_with_ast() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("helper.ps1"), "Write-Output h").unwrap();
    let gate = fake(&[(
        "Write-Output h",
        Ok("CMD\t1\tnone\tStringConstant\t.\tStart-Process\tp:Verb\ts:RunAs\n"),
    )]);
    let roots = CallRoots {
        dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let (report, called) =
        script_scan::scan_gated("& \"$dir\\helper.ps1\"", PRELUDE_FUNCTIONS, &roots, &gate);
    assert_eq!(report.verdict, Verdict::Refuse, "{:?}", report.flags);
    assert_eq!(called.len(), 1);
    assert_eq!(called[0].report.verdict, Verdict::Refuse);
}

#[test]
fn unavailable_is_the_safe_default() {
    assert_eq!(
        script_ast::Unavailable.facts("x"),
        Err(AstError::NoPowerShell)
    );
    assert_eq!(
        wingetrs::install::executor::Runners::disabled()
            .ast
            .facts("x"),
        Err(AstError::NoPowerShell)
    );
}

// ---- Task 227, review: `using` the parser would resolve ----

#[test]
fn parse_time_load_cases() {
    for (text, line) in [
        ("using module x", 1),
        ("Write-Output 1\n  USING <# c #>\n assembly x.dll", 2),
        ("us`ing mod`ule x", 1),
        ("& { using module y }", 1),
        ("x\n\nusing\tmodule y", 3),
        ("using\nassembly y", 1),
        ("USING MODULE x", 1),
        ("us`ing module x", 1),
        ("using<# c #>module x", 1),
        ("#requires -Version 5\nusing module x", 2),
        ("using `\n  module x", 1),
        ("using #c\nmodule x", 1),
        ("using #<#\nmodule x", 1),
        ("using <# a #> # b\n assembly y", 1),
        ("using <# unclosed", 1),
        ("Write-Output 1\nusing", 2),
        ("using x", 1),
        // Conservative: a `using` in a string, or before any other word, refuses too.
        ("Write-Output 'Using modules'", 1),
        ("using modulex", 1),
    ] {
        assert_eq!(script_ast::parse_time_load(text), Some(line), "{text:?}");
    }
    for text in [
        "using namespace System.IO",
        "using <# a #> # b\n namespace System.IO",
        "Start-Job { $using:x }",
        "Write-Output ${using:x}",
        "$using:x module",
        "x_using module y",
        "",
    ] {
        assert_eq!(script_ast::parse_time_load(text), None, "{text:?}");
    }
}

/// A gate that counts the scripts it is asked about.
#[derive(Default)]
struct Counting(std::sync::atomic::AtomicUsize);
impl AstGate for Counting {
    fn facts(&self, _: &str) -> Result<AstFacts, AstError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(AstFacts::default())
    }
}

#[test]
fn using_and_lexer_refusals_never_reach_the_parser() {
    for text in [
        "using module .\\m.psm1\nWrite-Output 1",
        "using assembly C:\\x\\nao_existe.dll\nWrite-Output 1",
    ] {
        let gate = Counting::default();
        let (report, _) =
            script_scan::scan_gated(text, PRELUDE_FUNCTIONS, &CallRoots::default(), &gate);
        assert_eq!(report.verdict, Verdict::Refuse, "{text:?}");
        assert!(
            report
                .flags
                .iter()
                .any(|f| f.kind == FlagKind::DynamicCode && f.verdict == Verdict::Refuse),
            "{:?}",
            report.flags
        );
        assert_eq!(
            gate.0.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{text:?}"
        );
    }
    let gate = Counting::default();
    let (report, _) = script_scan::scan_gated(
        "Start-Process x -Verb RunAs",
        PRELUDE_FUNCTIONS,
        &CallRoots::default(),
        &gate,
    );
    assert_eq!(report.verdict, Verdict::Refuse);
    assert_eq!(gate.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    // Anything else is asked.
    script_scan::scan_gated(
        "using namespace System.IO",
        PRELUDE_FUNCTIONS,
        &CallRoots::default(),
        &gate,
    );
    assert_eq!(gate.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// Two reasons of one kind on one line both stay; the parser's worse one sets the verdict.
#[test]
fn merge_keeps_every_reason_of_a_line() {
    let text = "[Environment]::SetEnvironmentVariable('a', 'b')";
    let gate = fake(&[(
        text,
        Ok("MEM\t1\t1\t.\t[Environment]\tSetEnvironmentVariable\ts:a\ts:b\ts:User\n"),
    )]);
    let (lexer, _) = script_scan::scan_gated(
        text,
        PRELUDE_FUNCTIONS,
        &CallRoots::default(),
        &script_ast::LexerOnly,
    );
    let lexer_env: Vec<_> = lexer
        .flags
        .iter()
        .filter(|f| f.kind == FlagKind::EnvironmentVariable)
        .collect();
    assert_eq!(lexer_env.len(), 1, "{:?}", lexer.flags);
    assert_eq!(lexer_env[0].verdict, Verdict::ConfirmStrong);
    let (report, _) =
        script_scan::scan_gated(text, PRELUDE_FUNCTIONS, &CallRoots::default(), &gate);
    assert_eq!(report.verdict, Verdict::Refuse);
    let env: Vec<_> = report
        .flags
        .iter()
        .filter(|f| f.kind == FlagKind::EnvironmentVariable && f.line == 1)
        .map(|f| (f.verdict, f.detail.as_str()))
        .collect();
    assert!(
        env.contains(&(Verdict::ConfirmStrong, lexer_env[0].detail.as_str())),
        "{env:?}"
    );
    assert!(
        env.iter()
            .any(|(v, d)| *v == Verdict::Refuse && *d != lexer_env[0].detail),
        "{env:?}"
    );
}

/// The dead `if ($global)` body of every case: refused once `$global` is bound.
const DEAD_BODY: &str = "CMD\t9\tnone\tStringConstant\tG\tRemove-Item\ts:C:\\Windows\\x\n";

/// Each way a script binds its own `$global` refuses it, and makes the `if ($global)` body live.
#[test]
fn any_global_binding_refuses_and_revives_the_dead_branch() {
    assert!(flags(DEAD_BODY).is_empty(), "alone, the body is dead code");
    let mut bindings = vec![
        "SET\t1\t.\tglobal\n".to_string(),
        "SET\t1\tF\tglobal\n".to_string(),
        "SET\t1\t.\tscript:global\n".to_string(),
        "CMD\t1\tnone\tStringConstant\t.\tSet-Variable\tp:Name\ts:global\ts:1\n".to_string(),
        "CMD\t1\tnone\tStringConstant\t.\tNew-Variable\tp:Name\ts:global\n".to_string(),
    ];
    for p in [
        "OutVariable",
        "ov",
        "PipelineVariable",
        "pv",
        "ErrorVariable",
        "ev",
        "WarningVariable",
        "wv",
        "InformationVariable",
        "iv",
        "OutVar",
    ] {
        bindings.push(format!(
            "CMD\t1\tnone\tStringConstant\t.\tWrite-Output\ts:x\tp:{p}\ts:global\n"
        ));
    }
    bindings.push("CMD\t1\tnone\tStringConstant\t.\tWrite-Output\ts:x\tp:ov\ts:+global\n".into());
    for b in &bindings {
        let got = flags(&format!("{b}{DEAD_BODY}"));
        assert!(
            got.contains(&(FlagKind::GlobalName, Verdict::Refuse)),
            "{b:?}: {got:?}"
        );
        assert!(
            got.contains(&(FlagKind::SystemWrite, Verdict::Refuse)),
            "{b:?}: {got:?}"
        );
    }
    // Look-alikes: another variable, `global` as a plain value.
    for b in [
        "SET\t1\t.\tglobal:x\n",
        "CMD\t1\tnone\tStringConstant\t.\tWrite-Output\ts:global\n",
        "CMD\t1\tnone\tStringConstant\t.\tWrite-Output\ts:x\tp:ov\ts:other\n",
    ] {
        assert!(flags(&format!("{b}{DEAD_BODY}")).is_empty(), "{b:?}");
    }
}
