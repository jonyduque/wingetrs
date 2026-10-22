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
    /// The word `global` appears in the script outside `if ($global)` conditions (set by the
    /// gate from the text, [`global_named_elsewhere`]): no `if ($global)` body is dead code.
    pub global_named: bool,
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

/// A word character of the text checks, on both sides (`[A-Za-z0-9_]`, the host's class): any
/// other character ends a word, which can only make a check match more.
fn word_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Words that make PowerShell's parser load modules or engines while parsing: DSC
/// (`configuration`, `Import-DscResource`, dynamic keywords) looks the module up, even on a UNC
/// path, and `workflow` loads the workflow engine in 5.1.
const PARSE_TIME_WORDS: &[&str] = &[
    "configuration",
    "import-dscresource",
    "workflow",
    "dynamickeyword",
];

/// The dashes PowerShell reads as `-`.
const DASHES: [char; 4] = ['-', '\u{2013}', '\u{2014}', '\u{2015}'];

/// [`parse_time_load`] for `using`: the 1-based line of the first `using` not followed by
/// `namespace` (after any mix of whitespace, `<# #>` and `#` comments; an unclosed `<#` or the
/// end of the text refuses). `$using:x`/`${using:x}` is a variable scope, not the keyword.
fn using_load(lower: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(pos) = lower[from..].find("using") {
        let at = from + pos;
        from = at + "using".len();
        let before = &lower[..at];
        let variable = before.ends_with('$') || before.ends_with("${");
        if variable
            || word_char(before.chars().next_back())
            || word_char(lower[from..].chars().next())
        {
            continue;
        }
        let mut rest = &lower[from..];
        loop {
            let trimmed = rest.trim_start();
            if let Some(block) = trimmed.strip_prefix("<#") {
                rest = block.split_once("#>").map_or("", |(_, after)| after);
            } else if let Some(line) = trimmed.strip_prefix('#') {
                rest = line.split_once('\n').map_or("", |(_, after)| after);
            } else {
                rest = trimmed;
                break;
            }
        }
        let namespace = rest
            .strip_prefix("namespace")
            .is_some_and(|after| !word_char(after.chars().next()));
        if !namespace {
            return Some(before.matches('\n').count() + 1);
        }
    }
    None
}

/// [`parse_time_load`] for [`PARSE_TIME_WORDS`]: the first line with one of them as a word, in
/// any position (a string or a comment too), unless it is part of a variable, member, scope or
/// parameter name (`$configuration`, `${workflow}`, `$env:workflow`, `-Configuration`); a
/// member or type name (`$x.Configuration`, `[..Language.DynamicKeyword]`) refuses too.
fn keyword_load(lower: &str) -> Option<usize> {
    let mut first: Option<usize> = None;
    for w in PARSE_TIME_WORDS {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(w) {
            let at = from + pos;
            from = at + w.len();
            let before = &lower[..at];
            let prev = before.chars().next_back();
            let named =
                word_char(prev) || matches!(prev, Some('$' | ':' | '-')) || before.ends_with("${");
            if named || word_char(lower[from..].chars().next()) {
                continue;
            }
            let line = before.matches('\n').count() + 1;
            first = Some(first.map_or(line, |f| f.min(line)));
            break;
        }
    }
    first
}

/// The rest of a `#requires` line loads nothing: only `-Version <digits and dots>` and
/// `-RunAsAdministrator` (full names, any dash), in any number, then blanks.
fn requires_allowed(rest: &str) -> bool {
    let mut r = rest;
    loop {
        r = r.trim_start_matches([' ', '\t']);
        if r.trim_start_matches([' ', '\t', '\r']).is_empty() {
            return true;
        }
        let Some(after) = r.strip_prefix(DASHES) else {
            return false;
        };
        if let Some(v) = after.strip_prefix("version") {
            let v2 = v.trim_start_matches([' ', '\t']);
            let digits = v2.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
            if v2.len() == v.len() || digits.len() == v2.len() {
                return false;
            }
            r = digits;
        } else if let Some(a) = after.strip_prefix("runasadministrator") {
            r = a;
        } else {
            return false;
        }
    }
}

/// [`parse_time_load`] for `#requires`: `-Modules`, `-Assembly` and `-PSSnapin` load what they
/// name. The first line of a `#requires` (any whitespace, line breaks too, after the `#`) whose
/// rest is not [`requires_allowed`].
fn requires_load(lower: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(pos) = lower[from..].find('#') {
        let at = from + pos;
        from = at + 1;
        let Some(after) = lower[from..].trim_start().strip_prefix("requires") else {
            continue;
        };
        if word_char(after.chars().next()) {
            continue;
        }
        let rest = after.split('\n').next().unwrap_or("");
        if !requires_allowed(rest) {
            return Some(lower[..at].matches('\n').count() + 1);
        }
    }
    None
}

/// What [`parse_time_load`] found: the line and the `USE` kind that refuses it.
pub fn parse_time_load_kind(script: &str) -> Option<(usize, &'static str)> {
    let text: String = script.chars().filter(|c| *c != '`').collect();
    let lower = text.to_lowercase();
    [
        using_load(&lower).map(|l| (l, "module")),
        keyword_load(&lower).map(|l| (l, "dsc")),
        requires_load(&lower).map(|l| (l, "requires")),
    ]
    .into_iter()
    .flatten()
    .min_by_key(|(l, _)| *l)
}

/// The 1-based line of the first construct of `script` that PowerShell's parser (`ParseInput`,
/// and `PSParser.Tokenize` too) would resolve or load while parsing, read as text so that such a
/// script never reaches it: a `using` that is not `using namespace`, a DSC or workflow word
/// ([`PARSE_TIME_WORDS`]), or a `#requires` with more than `-Version`/`-RunAsAdministrator`.
/// Conservative: backticks are dropped first, the position is not checked (a misplaced `using`
/// is resolved too), and a match inside a string or a comment refuses as well. `using
/// namespace` passes: it only names where types are looked up and loads nothing. The host runs
/// the same checks as regexes (`$usingLoad`, `$parseLoad`, `$requiresLoad` of `ast_facts.ps1`);
/// a test keeps both equivalent.
pub fn parse_time_load(script: &str) -> Option<usize> {
    parse_time_load_kind(script).map(|(line, _)| line)
}

/// Facts of a script [`parse_time_load`] keeps away from the parser: one `USE` that refuses it.
pub fn parse_time_load_facts(line: usize, kind: &str) -> AstFacts {
    AstFacts {
        usings: vec![UsingFact {
            line,
            kind: kind.into(),
        }],
        ..AstFacts::default()
    }
}

/// The word `global` appears in `script` outside the conditions `if ($global)` and
/// `if ($global -and $x)` (backticks dropped, any case, strings and comments too). Then the
/// script may bind its own `$global` in a way the parser cannot name (`[bool]$global = ..`,
/// `variable:global`, `[ref]$global`, a splat), and no `if ($global)` body is dead code.
pub fn global_named_elsewhere(script: &str) -> bool {
    let text: String = script.chars().filter(|c| *c != '`').collect();
    let mut rest = text.to_lowercase();
    // Each `if ( $global [-and $name] )` blanked, then any `global` left counts.
    let mut out = String::with_capacity(rest.len());
    while let Some(at) = rest.find("if") {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        match global_condition_len(tail) {
            Some(n) if !word_char(head.chars().next_back()) => {
                out.push(' ');
                rest = tail[n..].to_string();
            }
            _ => {
                out.push_str("if");
                rest = tail[2..].to_string();
            }
        }
    }
    out.push_str(&rest);
    let mut from = 0;
    while let Some(pos) = out[from..].find("global") {
        let at = from + pos;
        from = at + "global".len();
        if !word_char(out[..at].chars().next_back()) && !word_char(out[from..].chars().next()) {
            return true;
        }
    }
    false
}

/// Length of `if ( $global )` or `if ( $global -and $name )` at the start of `t` (lowercase).
fn global_condition_len(t: &str) -> Option<usize> {
    let r = t.strip_prefix("if")?.trim_start();
    let r = r.strip_prefix('(')?.trim_start();
    let r = r.strip_prefix("$global")?;
    if word_char(r.chars().next()) {
        return None;
    }
    let mut r = r.trim_start();
    if let Some(and) = r.strip_prefix("-and") {
        let v = and.trim_start().strip_prefix('$')?;
        let name_end = v.find(|c: char| !word_char(Some(c))).unwrap_or(v.len());
        if name_end == 0 {
            return None;
        }
        r = v[name_end..].trim_start();
    }
    let r = r.strip_prefix(')')?;
    Some(t.len() - r.len())
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

/// Common parameters that bind a variable by name (`-OutVariable x`), and their aliases.
const VARIABLE_PARAMS: &[&str] = &[
    "outvariable",
    "pipelinevariable",
    "errorvariable",
    "warningvariable",
    "informationvariable",
];
const VARIABLE_ALIASES: &[&str] = &["ov", "pv", "ev", "wv", "iv"];

/// `-<p>` names one of [`VARIABLE_PARAMS`] (alias or any prefix of 2 letters or more).
fn variable_param(p: &str) -> bool {
    let p = p.to_lowercase();
    VARIABLE_ALIASES.contains(&p.as_str())
        || (p.len() >= 2 && VARIABLE_PARAMS.iter().any(|v| v.starts_with(&p)))
}

/// A variable name given as text is `global` (`+global` appends, `global:`/`script:` scopes).
fn names_global(arg: &Arg) -> bool {
    arg.text()
        .is_some_and(|t| unscoped(t.trim_start_matches('+')).eq_ignore_ascii_case("global"))
}

/// Lines where the script binds its own `$global` in any form, whatever the chain: an
/// assignment, `param($global)`, `foreach ($global in ..)` (`SET`), `Set-Variable`/`New-Variable`
/// with `global`, or `-OutVariable global` and the other variable parameters. The runner's
/// constant `$global` should make these fail, but not in another runspace: the gate never
/// relies on it (any binding refuses, and no `if ($global)` body counts as dead code).
fn global_bindings(facts: &AstFacts) -> Vec<usize> {
    let mut lines: Vec<usize> = facts
        .assignments
        .iter()
        .filter(|s| unscoped(&s.name).eq_ignore_ascii_case("global"))
        .map(|s| s.line)
        .collect();
    for c in &facts.commands {
        let name = file_name(c.name.as_deref().unwrap_or(""));
        // A name it cannot read (`('glo' + 'bal')`, a splat) counts as `global`.
        let set_variable = named(&name, &["set-variable", "new-variable", "sv", "nv"])
            && c.args
                .iter()
                .any(|a| names_global(a) || matches!(a, Arg::Other(_)));
        let variable_arg = c
            .args
            .windows(2)
            .any(|w| matches!(&w[0], Arg::Param(p) if variable_param(p)) && names_global(&w[1]));
        if set_variable || variable_arg {
            lines.push(c.line);
        }
    }
    // `$ExecutionContext.SessionState.PSVariable.Set(..)` with any name.
    for m in &facts.members {
        let target: String = m.target.chars().filter(|c| !c.is_whitespace()).collect();
        if target.to_ascii_lowercase().contains("psvariable") {
            lines.push(m.line);
        }
    }
    lines.sort_unstable();
    lines.dedup();
    lines
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
    // Every kind but `namespace` loads something while parsing.
    for u in facts.usings.iter().filter(|u| u.kind != "namespace") {
        let detail = match u.kind.as_str() {
            "module" | "assembly" => "carrega módulo ou assembly (using)",
            "dsc" => {
                "usa configuration, workflow ou Import-DscResource (carrega módulos na análise)"
            }
            "requires" => "#requires carrega módulo, assembly ou snap-in",
            _ => "carrega código na análise do script",
        };
        push(
            FlagKind::DynamicCode,
            Verdict::Refuse,
            detail.into(),
            u.line,
        );
    }
    let bindings = global_bindings(facts);
    for line in &bindings {
        push(
            FlagKind::GlobalName,
            Verdict::Refuse,
            D_GLOBAL_NAME.into(),
            *line,
        );
    }
    // With its own `$global`, an `if ($global)` body may run: nothing is dead code.
    let dead = |chain: &str| bindings.is_empty() && !facts.global_named && dead(chain);
    for c in facts.commands.iter().filter(|c| !dead(&c.chain)) {
        command_flags(c, supported, &mut push);
    }
    for m in facts.members.iter().filter(|m| !dead(&m.chain)) {
        member_flags(m, &mut push);
    }
    for s in facts.assignments.iter().filter(|s| !dead(&s.chain)) {
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
    // `-Verb` (or a prefix) with a value it cannot read (`('Run' + 'As')`, `$v`, `"$v"`) may
    // be `RunAs`.
    let computed_verb = launcher
        && c.args.windows(2).any(|w| {
            let verb = matches!(&w[0], Arg::Param(p) if !p.is_empty() && "verb".starts_with(p.to_ascii_lowercase().as_str()));
            let unread = match &w[1] {
                Arg::Str(_) => false,
                Arg::Expandable(e) => e.contains(['$', '(']),
                Arg::Param(_) | Arg::Other(_) => true,
            };
            verb && unread
        });
    // A splat (`Start-Process @a`) may hide `-Verb RunAs`.
    let splat = launcher
        && c.args
            .iter()
            .any(|a| matches!(a, Arg::Other(t) if t == "Splat"));
    if computed_verb || splat {
        flag(
            FlagKind::Elevation,
            Verdict::Refuse,
            "Start-Process com -Verb calculado (pode pedir elevação)",
        );
    }
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
