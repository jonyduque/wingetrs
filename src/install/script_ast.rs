//! Facts of PowerShell's own parser about a manifest script (Task 227), and the gate rules over
//! them. The lexer of [`script_scan`] stays the first gate; these flags only add to it
//! ([`script_scan::scan_gated`]): a script the parser cannot read, or that the parser shows doing
//! what the lexer missed (`&("i" + "ex")`, a block run by `Start-Job`), never gets a milder
//! verdict.
//!
//! The parser runs in a PowerShell process ([`HOST_SCRIPT`], `script_ast_win`); this module
//! only decodes its wire format and applies the rules. Wire format, one record per line, fields
//! split by TAB, free text escaped only in `%` (`%25`), TAB (`%09`), CR (`%0D`) and LF (`%0A`):
//!
//! - `ERR <line> <ErrorId>`: a parse error;
//! - `USE <line> <module|assembly|namespace|type|command>`: a `using` statement;
//! - `CMD <line> <none|amp|dot> <head> <chain> <name> <arg>...`: a command; `head` = type of the
//!   first element without `ExpressionAst`/`Ast`; `name` = `GetCommandName()` or empty; each arg
//!   is `p:<parameter>`, `s:<string constant>`, `e:<expandable string, unexpanded>` or
//!   `?:<type>`;
//! - `MEM <line> <0|1 static> <chain> <target> <member> <arg>...`: a member access or call;
//!   `target` = `[<type name>]` or the expression's text; args as in `CMD`, with the raw text
//!   after `?:`;
//! - `SET <line> <chain> <variable>`: an assignment to a variable;
//! - `FN <line> <chain> <name>`: a function definition;
//! - `END <id>`: the end of one answer (read by the process side, ignored by [`decode`]).
//!
//! `chain`: one letter per ancestor from the root: `B` script block expression, `F` function,
//! `G` body of the first clause of `if ($global)`/`if ($global -and $x)`; `.` when none.
use super::prelude::PRELUDE_FUNCTIONS;
use super::script_scan::{
    D_ALIAS, D_COMPUTED, D_DOWNLOAD, D_DYNAMIC, D_ELEVATION, D_ENV_PERSISTENT, D_ENV_PROCESS,
    D_GLOBAL_NAME, D_HIDDEN, D_HKLM, D_LAUNCHED, D_MSIEXEC, D_PRELUDE_INTERNAL, D_SCOOP, D_SYSTEM,
    D_TASK, DOWNLOAD_WORDS, Flag, FlagKind, SCOOP_FUNCTIONS, SYSTEM_DIRS, TASK_WORDS, Verdict,
    WRITE_CMDLETS,
};

/// The PowerShell host that prints the facts (run with `-EncodedCommand`).
pub const HOST_SCRIPT: &str = include_str!("ast_facts.ps1");

/// Why the parser gave no facts. Each one refuses the script (`FlagKind::ParserFailed`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AstError {
    #[error("PowerShell não encontrado: o script não pode ser analisado")]
    NoPowerShell,
    #[error("o wingetrs não analisa scripts com o processo elevado")]
    Elevated,
    /// The group policy text of `script_policy::gpo_blocks`.
    #[error("{0}")]
    Policy(String),
    /// Seconds of the deadline that passed.
    #[error("a análise do PowerShell passou de {0} s")]
    Timeout(u64),
    /// What went wrong (logged; the user sees the fixed text).
    #[error("a análise do PowerShell falhou")]
    Protocol(String),
}

/// The parser's facts about one script, for the gate.
pub trait AstGate: Send + Sync {
    fn facts(&self, script: &str) -> Result<AstFacts, AstError>;
}

/// No parser: every script is refused (`Runners::disabled()`, non-Windows).
pub struct Unavailable;

impl AstGate for Unavailable {
    fn facts(&self, _script: &str) -> Result<AstFacts, AstError> {
        Err(AstError::NoPowerShell)
    }
}

/// Tests only: no facts, so the verdict is the lexer's alone. `check_rules` refuses the name
/// anywhere else in `src/`.
#[doc(hidden)]
pub struct LexerOnly;

impl AstGate for LexerOnly {
    fn facts(&self, _script: &str) -> Result<AstFacts, AstError> {
        Ok(AstFacts::default())
    }
}

/// The gate of a run under `WINGETRS_TEST_ENV_KEY` (`prod_win`): no script runs there
/// (`Runners::disabled`), and the binary's confirmations and dry runs stay testable without
/// PowerShell: the lexer's verdict alone.
pub fn under_test_key() -> std::sync::Arc<dyn AstGate> {
    std::sync::Arc::new(LexerOnly)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    /// `-Name` (without the dash).
    Param(String),
    /// A string constant (bare word or quoted), escapes resolved.
    Str(String),
    /// An expandable string as written, variables not expanded.
    Expandable(String),
    /// Anything else: its type (`CMD`) or raw text (`MEM`).
    Other(String),
}

impl Arg {
    /// The text of a string argument (`s:`/`e:`).
    fn text(&self) -> Option<&str> {
        match self {
            Arg::Str(s) | Arg::Expandable(s) => Some(s),
            _ => None,
        }
    }

    /// Every argument's text, parameters and types included (for `__wr`).
    fn any_text(&self) -> &str {
        match self {
            Arg::Param(s) | Arg::Str(s) | Arg::Expandable(s) | Arg::Other(s) => s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invocation {
    None,
    /// `&`
    Amp,
    /// `.`
    Dot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFact {
    pub line: usize,
    pub op: Invocation,
    pub head: String,
    pub chain: String,
    pub name: Option<String>,
    pub args: Vec<Arg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberFact {
    pub line: usize,
    pub is_static: bool,
    pub chain: String,
    pub target: String,
    pub member: String,
    pub args: Vec<Arg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedFact {
    pub line: usize,
    pub chain: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseErrorFact {
    pub line: usize,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsingFact {
    pub line: usize,
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AstFacts {
    pub errors: Vec<ParseErrorFact>,
    pub usings: Vec<UsingFact>,
    pub commands: Vec<CommandFact>,
    pub members: Vec<MemberFact>,
    /// `SET`: the variable, as `VariablePath.UserPath` (`global:x`).
    pub assignments: Vec<NamedFact>,
    /// `FN`: the function name.
    pub functions: Vec<NamedFact>,
}

/// Undoes the wire escapes; any other `%` is an error.
fn unescape(field: &str) -> Result<String, String> {
    if !field.contains('%') {
        return Ok(field.to_string());
    }
    let mut out = String::with_capacity(field.len());
    let mut rest = field;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let code = rest.get(at + 1..at + 3).ok_or("escape incompleto")?;
        out.push(match code {
            "25" => '%',
            "09" => '\t',
            "0D" => '\r',
            "0A" => '\n',
            _ => return Err(format!("escape desconhecido %{code}")),
        });
        rest = &rest[at + 3..];
    }
    out.push_str(rest);
    Ok(out)
}

fn arg(field: &str) -> Result<Arg, String> {
    let (tag, value) = field
        .split_once(':')
        .ok_or_else(|| format!("argumento sem tipo: {field:?}"))?;
    let value = value.to_string();
    Ok(match tag {
        "p" => Arg::Param(value),
        "s" => Arg::Str(value),
        "e" => Arg::Expandable(value),
        "?" => Arg::Other(value),
        _ => return Err(format!("argumento de tipo desconhecido: {field:?}")),
    })
}

/// The 1-based line of the first `using module`/`using assembly` of `script`, read as text:
/// PowerShell's parser (`ParseInput`, and `PSParser.Tokenize` too) resolves these while parsing,
/// so such a script never reaches it. Backticks are dropped first, a comment `<# #>` or any
/// whitespace (line breaks included) may sit between the words, and the position is not
/// checked (a misplaced `using` is resolved as well). `using namespace` passes: it only names
/// where types are looked up and loads nothing.
pub fn parse_time_load(script: &str) -> Option<usize> {
    let text: String = script.chars().filter(|c| *c != '`').collect();
    let lower = text.to_lowercase();
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut from = 0;
    while let Some(pos) = lower[from..].find("using") {
        let at = from + pos;
        from = at + "using".len();
        if word(lower[..at].chars().next_back()) {
            continue;
        }
        let mut rest = &lower[from..];
        loop {
            let trimmed = rest.trim_start();
            if let Some(comment) = trimmed.strip_prefix("<#") {
                rest = comment.split_once("#>").map_or("", |(_, after)| after);
            } else {
                rest = trimmed;
                break;
            }
        }
        let loads = ["module", "assembly"].iter().any(|k| {
            rest.strip_prefix(k)
                .is_some_and(|after| !word(after.chars().next()))
        });
        if loads {
            return Some(lower[..at].matches('\n').count() + 1);
        }
    }
    None
}

/// Facts of a script [`parse_time_load`] keeps away from the parser: its `using` refuses it.
pub fn parse_time_load_facts(line: usize) -> AstFacts {
    AstFacts {
        usings: vec![UsingFact {
            line,
            kind: "module".into(),
        }],
        ..AstFacts::default()
    }
}

/// Decodes the records of one answer (`END` lines are skipped). Any malformed line is an
/// error: a fact the gate cannot read must not vanish.
pub fn decode(wire: &str) -> Result<AstFacts, AstError> {
    decode_lines(wire.lines()).map_err(AstError::Protocol)
}

pub(crate) fn decode_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<AstFacts, String> {
    let mut facts = AstFacts::default();
    for raw in lines {
        if raw.is_empty() {
            continue;
        }
        let fields = raw
            .split('\t')
            .map(unescape)
            .collect::<Result<Vec<String>, String>>()?;
        let bad = || format!("registro inválido: {raw:?}");
        let field = |i: usize| fields.get(i).ok_or_else(bad);
        let line =
            |i: usize| -> Result<usize, String> { field(i)?.parse::<usize>().map_err(|_| bad()) };
        let exact = |n: usize| -> Result<(), String> {
            if fields.len() == n {
                Ok(())
            } else {
                Err(bad())
            }
        };
        match fields[0].as_str() {
            "END" => {}
            "ERR" => {
                exact(3)?;
                facts.errors.push(ParseErrorFact {
                    line: line(1)?,
                    id: field(2)?.clone(),
                });
            }
            "USE" => {
                exact(3)?;
                facts.usings.push(UsingFact {
                    line: line(1)?,
                    kind: field(2)?.to_ascii_lowercase(),
                });
            }
            "CMD" => {
                let op = match field(2)?.as_str() {
                    "none" => Invocation::None,
                    "amp" => Invocation::Amp,
                    "dot" => Invocation::Dot,
                    _ => return Err(bad()),
                };
                let name = field(5)?;
                facts.commands.push(CommandFact {
                    line: line(1)?,
                    op,
                    head: field(3)?.clone(),
                    chain: field(4)?.clone(),
                    name: (!name.is_empty()).then(|| name.clone()),
                    args: fields[6..]
                        .iter()
                        .map(|f| arg(f))
                        .collect::<Result<_, _>>()?,
                });
            }
            "MEM" => {
                let is_static = match field(2)?.as_str() {
                    "0" => false,
                    "1" => true,
                    _ => return Err(bad()),
                };
                facts.members.push(MemberFact {
                    line: line(1)?,
                    is_static,
                    chain: field(3)?.clone(),
                    target: field(4)?.clone(),
                    member: field(5)?.clone(),
                    args: fields[6..]
                        .iter()
                        .map(|f| arg(f))
                        .collect::<Result<_, _>>()?,
                });
            }
            "SET" | "FN" => {
                exact(4)?;
                let fact = NamedFact {
                    line: line(1)?,
                    chain: field(2)?.clone(),
                    name: field(3)?.clone(),
                };
                if fields[0] == "SET" {
                    facts.assignments.push(fact);
                } else {
                    facts.functions.push(fact);
                }
            }
            _ => return Err(bad()),
        }
    }
    Ok(facts)
}

/// Code inside `if ($global) { }` (a `G` with no `B` in the chain) never runs in the wingetrs
/// runner; the lexer already shows it as "only in a global install". A script block anywhere
/// on the chain may run elsewhere (`Start-Job`), so it is analysed.
fn dead(chain: &str) -> bool {
    chain.contains('G') && !chain.contains('B')
}

/// The command's file name in lowercase (`C:\x\Msiexec.EXE` ⇒ `msiexec.exe`).
fn file_name(name: &str) -> String {
    name.rsplit(['\\', '/'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}

/// `name` (lowercase file name) is in `set`, with or without `.exe`.
fn named(name: &str, set: &[&str]) -> bool {
    let bare = name.strip_suffix(".exe").unwrap_or(name);
    set.iter().any(|s| *s == name || *s == bare)
}

/// A type as compared: lowercase, no brackets, no `system.` prefix.
fn type_key(t: &str) -> String {
    let t = t.trim().trim_start_matches('[').trim_end_matches(']');
    let t = t.to_ascii_lowercase();
    t.strip_prefix("system.").map(str::to_string).unwrap_or(t)
}

/// `target` (a [`type_key`]) names the type `short`, written in full or not
/// (`management.automation.powershell` is `powershell`).
fn is_type(target: &str, short: &str) -> bool {
    target == short || target.ends_with(&format!(".{short}"))
}

/// A variable or function name without its scope prefix (`global:x` ⇒ `x`).
fn unscoped(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn contains_any(text: &str, set: &[&str]) -> bool {
    let t = text.to_ascii_lowercase();
    set.iter().any(|s| t.contains(s))
}

/// The gate flags of `facts` (same kinds, verdicts and texts as the lexer). `supported`: the
/// Scoop functions the prelude implements.
pub fn ast_flags(facts: &AstFacts, supported: &[&str]) -> Vec<Flag> {
    let mut out: Vec<Flag> = Vec::new();
    let mut push = |kind: FlagKind, verdict: Verdict, detail: String, line: usize| {
        let f = Flag {
            kind,
            verdict,
            detail,
            line,
        };
        if !out.contains(&f) {
            out.push(f);
        }
    };
    if let Some(e) = facts.errors.first() {
        push(
            FlagKind::SyntaxError,
            Verdict::Refuse,
            format!(
                "o PowerShell não entende o script (erro de sintaxe na linha {})",
                e.line
            ),
            e.line,
        );
    }
    for u in &facts.usings {
        if u.kind == "module" || u.kind == "assembly" {
            push(
                FlagKind::DynamicCode,
                Verdict::Refuse,
                "carrega módulo ou assembly (using)".into(),
                u.line,
            );
        }
    }
    for c in facts.commands.iter().filter(|c| !dead(&c.chain)) {
        command_flags(c, supported, &mut push);
    }
    for m in facts.members.iter().filter(|m| !dead(&m.chain)) {
        member_flags(m, &mut push);
    }
    for s in facts.assignments.iter().filter(|s| !dead(&s.chain)) {
        if unscoped(&s.name).eq_ignore_ascii_case("global") {
            push(
                FlagKind::GlobalName,
                Verdict::Refuse,
                D_GLOBAL_NAME.into(),
                s.line,
            );
        }
        if s.name.to_ascii_lowercase().contains("__wr") {
            push(
                FlagKind::PreludeInternal,
                Verdict::Refuse,
                D_PRELUDE_INTERNAL.into(),
                s.line,
            );
        }
    }
    for f in facts.functions.iter().filter(|f| !dead(&f.chain)) {
        if let Some(name) = PRELUDE_FUNCTIONS
            .iter()
            .find(|p| p.eq_ignore_ascii_case(unscoped(&f.name)))
        {
            push(
                FlagKind::PreludeRedefinition,
                Verdict::Refuse,
                format!("redefine a função do prelúdio do wingetrs: {name}"),
                f.line,
            );
        }
        if f.name.to_ascii_lowercase().contains("__wr") {
            push(
                FlagKind::PreludeInternal,
                Verdict::Refuse,
                D_PRELUDE_INTERNAL.into(),
                f.line,
            );
        }
    }
    out.sort_by_key(|f| f.line);
    out
}

fn command_flags(
    c: &CommandFact,
    supported: &[&str],
    push: &mut impl FnMut(FlagKind, Verdict, String, usize),
) {
    let line = c.line;
    let raw_name = c.name.as_deref().unwrap_or("");
    let name = file_name(raw_name);
    let is = |set: &[&str]| named(&name, set);
    let texts: Vec<String> = c
        .args
        .iter()
        .filter_map(Arg::text)
        .map(str::to_ascii_lowercase)
        .collect();
    let params: Vec<String> = c
        .args
        .iter()
        .filter_map(|a| match a {
            Arg::Param(p) => Some(p.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let any_text = |pred: &dyn Fn(&str) -> bool| texts.iter().any(|t| pred(t));
    let mut flag = |kind: FlagKind, verdict: Verdict, detail: &str| {
        push(kind, verdict, detail.to_string(), line);
    };

    if is(&["iex", "invoke-expression"]) {
        flag(FlagKind::DynamicCode, Verdict::ConfirmStrong, D_DYNAMIC);
    }
    if c.op != Invocation::None
        && c.name.is_none()
        && !matches!(
            c.head.as_str(),
            "StringConstant" | "ExpandableString" | "ScriptBlock"
        )
    {
        flag(FlagKind::CalledFile, Verdict::ConfirmStrong, D_COMPUTED);
    }
    if any_text(&|t| t == "runas") || is(&["sudo", "gsudo"]) {
        flag(FlagKind::Elevation, Verdict::Refuse, D_ELEVATION);
    }
    if any_text(&|t| {
        t.contains("hklm:") || t.contains("hklm\\") || t.contains("hkey_local_machine")
    }) {
        flag(FlagKind::Hklm, Verdict::Refuse, D_HKLM);
    }
    if is(WRITE_CMDLETS) && any_text(&|t| SYSTEM_DIRS.iter().any(|d| t.contains(d))) {
        flag(FlagKind::SystemWrite, Verdict::Refuse, D_SYSTEM);
    }
    let launcher = is(&["start-process", "saps", "start"]);
    let msiexec_arg = any_text(&|t| named(&file_name(t), &["msiexec"]));
    if is(&["msiexec"]) || (launcher && msiexec_arg) {
        flag(FlagKind::Msiexec, Verdict::Refuse, D_MSIEXEC);
    }
    if is(&["scoop", "scoop.cmd", "scoop.ps1"]) {
        flag(FlagKind::ScoopCommand, Verdict::Refuse, D_SCOOP);
    }
    if let Some(canon) = SCOOP_FUNCTIONS
        .iter()
        .find(|f| f.eq_ignore_ascii_case(raw_name))
        && !supported.iter().any(|s| s.eq_ignore_ascii_case(raw_name))
    {
        let detail = format!("função do Scoop ainda não suportada: {canon}");
        flag(FlagKind::UnknownScoopFunction, Verdict::Refuse, &detail);
    }
    if is(DOWNLOAD_WORDS) {
        flag(FlagKind::Download, Verdict::ConfirmStrong, D_DOWNLOAD);
    }
    if is(TASK_WORDS) {
        flag(
            FlagKind::ScheduledTaskOrService,
            Verdict::ConfirmStrong,
            D_TASK,
        );
    }
    if is(&["set-alias", "new-alias", "sal", "nal"]) {
        flag(FlagKind::Alias, Verdict::ConfirmStrong, D_ALIAS);
    }
    let shell = is(&["powershell", "pwsh"]);
    let encoded = shell
        && params.iter().any(|p| {
            p == "e" || p == "ec" || (p.len() >= 2 && "encodedcommand".starts_with(p.as_str()))
        });
    if encoded {
        flag(FlagKind::HiddenCode, Verdict::Refuse, D_HIDDEN);
    }
    // Any PowerShell child runs code the user does not see (`-Command`, `-File`, or the first
    // positional argument, which is either depending on the version).
    let cmd_c = is(&["cmd"]) && any_text(&|t| t == "/c" || t == "/k");
    if cmd_c || (shell && !encoded) || (launcher && !msiexec_arg) {
        flag(FlagKind::CalledFile, Verdict::ConfirmStrong, D_LAUNCHED);
    }
    if is(&["set-variable", "new-variable", "sv", "nv"]) && any_text(&|t| unscoped(t) == "global") {
        flag(FlagKind::GlobalName, Verdict::Refuse, D_GLOBAL_NAME);
    }
    if raw_name.to_ascii_lowercase().contains("__wr")
        || c.args
            .iter()
            .any(|a| a.any_text().to_ascii_lowercase().contains("__wr"))
    {
        flag(
            FlagKind::PreludeInternal,
            Verdict::Refuse,
            D_PRELUDE_INTERNAL,
        );
    }
}

fn member_flags(m: &MemberFact, push: &mut impl FnMut(FlagKind, Verdict, String, usize)) {
    let line = m.line;
    let target = type_key(&m.target);
    // `$ExecutionContext . InvokeCommand` as the target's text: spacing and backticks dropped.
    let target_text: String = m
        .target
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '`')
        .collect::<String>()
        .to_ascii_lowercase();
    let member = m.member.to_ascii_lowercase();
    let mut flag = |kind: FlagKind, verdict: Verdict, detail: &str| {
        push(kind, verdict, detail.to_string(), line);
    };

    let any_arg = |word: &str| {
        m.args
            .iter()
            .any(|a| a.any_text().to_ascii_lowercase().contains(word))
    };
    let hklm = ((is_type(&target, "registry") || is_type(&target, "registryhive"))
        && member == "localmachine")
        || (is_type(&target, "registrykey") && member == "openbasekey" && any_arg("localmachine"));
    if hklm {
        flag(FlagKind::Hklm, Verdict::Refuse, D_HKLM);
    }
    let writes = member.starts_with("write")
        || member.starts_with("append")
        || matches!(member.as_str(), "copy" | "move" | "delete");
    if is_type(&target, "file")
        && writes
        && m.args
            .iter()
            .any(|a| contains_any(a.any_text(), SYSTEM_DIRS))
    {
        flag(FlagKind::SystemWrite, Verdict::Refuse, D_SYSTEM);
    }
    if member == "downloadfile" || member == "downloadstring" {
        flag(FlagKind::Download, Verdict::ConfirmStrong, D_DOWNLOAD);
    }
    if is_type(&target, "process") && member == "start" {
        flag(FlagKind::CalledFile, Verdict::ConfirmStrong, D_LAUNCHED);
    }
    let invoke_command = target_text.starts_with("$executioncontext.invokecommand")
        || (target_text == "$executioncontext" && member == "invokecommand");
    if ((is_type(&target, "scriptblock") || is_type(&target, "powershell")) && member == "create")
        || is_type(&target, "runspacefactory")
        || invoke_command
    {
        flag(FlagKind::DynamicCode, Verdict::ConfirmStrong, D_DYNAMIC);
    }
    if (is_type(&target, "assembly") && member.starts_with("load"))
        || (is_type(&target, "activator") && member == "createinstance")
    {
        flag(
            FlagKind::DynamicCode,
            Verdict::Refuse,
            "carrega código .NET na hora",
        );
    }
    if is_type(&target, "environment") && member == "setenvironmentvariable" {
        let persistent = m.args.get(2).is_some_and(|a| match a {
            Arg::Str(s) => s.eq_ignore_ascii_case("user") || s.eq_ignore_ascii_case("machine"),
            Arg::Expandable(s) | Arg::Other(s) => {
                let s = s.to_ascii_lowercase();
                s.contains("user") || s.contains("machine")
            }
            Arg::Param(_) => false,
        });
        if persistent {
            flag(
                FlagKind::EnvironmentVariable,
                Verdict::Refuse,
                D_ENV_PERSISTENT,
            );
        } else {
            flag(
                FlagKind::EnvironmentVariable,
                Verdict::ConfirmStrong,
                D_ENV_PROCESS,
            );
        }
    }
    if [&m.target, &m.member]
        .iter()
        .any(|t| t.to_ascii_lowercase().contains("__wr"))
        || m.args
            .iter()
            .any(|a| a.any_text().to_ascii_lowercase().contains("__wr"))
    {
        flag(
            FlagKind::PreludeInternal,
            Verdict::Refuse,
            D_PRELUDE_INTERNAL,
        );
    }
}
