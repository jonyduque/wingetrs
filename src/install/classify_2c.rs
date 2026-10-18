//! Phase 2c classification of Scoop manifests (Task 142): which sub-cases a manifest
//! needs (psmodule, innosetup, installer file, `.ps1` installer, script hooks) and the
//! command names its scripts call. Pure; used by the `oracle_2c` example to measure,
//! never to decide whether a script is safe.
use std::collections::BTreeSet;

use serde_json::{Map, Value};

/// Phase 2c sub-case, ordered by increasing risk (`Ord` follows the declaration order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SubCase {
    /// `psmodule`.
    Psm,
    /// `innosetup: true`.
    Inno,
    /// `installer`/`uninstaller` with `file` (not `.ps1`) or only `args`.
    File,
    /// `installer.file`/`uninstaller.file` ending in `.ps1`.
    Ps1,
    /// Script hooks: `pre_install`, `post_install`, `pre_uninstall`, `post_uninstall`,
    /// `installer.script`, `uninstaller.script`.
    Hook,
}

impl SubCase {
    pub const ALL: [SubCase; 5] = [
        SubCase::Psm,
        SubCase::Inno,
        SubCase::File,
        SubCase::Ps1,
        SubCase::Hook,
    ];

    /// Short label used in reports.
    pub fn label(self) -> &'static str {
        match self {
            SubCase::Psm => "S-psm",
            SubCase::Inno => "S-inno",
            SubCase::File => "S-file",
            SubCase::Ps1 => "S-ps1",
            SubCase::Hook => "S-hook",
        }
    }
}

/// Sub-cases present at the root or in any `architecture` variant, and the riskiest one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub cases: BTreeSet<SubCase>,
    pub worst: SubCase,
}

const HOOK_KEYS: [&str; 4] = [
    "pre_install",
    "post_install",
    "pre_uninstall",
    "post_uninstall",
];

fn present(v: Option<&Value>) -> bool {
    v.is_some_and(|v| !v.is_null())
}

/// Sub-cases of an installer/uninstaller object. Scoop runs `script` and then the
/// program (`file`, defaulting to the downloaded file, when `file` or `args` is set).
fn installer_cases(v: Option<&Value>, out: &mut BTreeSet<SubCase>) {
    let Some(obj) = v.and_then(Value::as_object) else {
        return;
    };
    if present(obj.get("script")) {
        out.insert(SubCase::Hook);
    }
    match obj.get("file").and_then(Value::as_str) {
        Some(f) if f.trim().to_ascii_lowercase().ends_with(".ps1") => {
            out.insert(SubCase::Ps1);
        }
        Some(_) => {
            out.insert(SubCase::File);
        }
        None if present(obj.get("args")) => {
            out.insert(SubCase::File);
        }
        None => {}
    }
}

fn object_cases(o: &Map<String, Value>, out: &mut BTreeSet<SubCase>) {
    if present(o.get("psmodule")) {
        out.insert(SubCase::Psm);
    }
    if o.get("innosetup")
        .is_some_and(|v| !v.is_null() && *v != Value::Bool(false))
    {
        out.insert(SubCase::Inno);
    }
    installer_cases(o.get("installer"), out);
    installer_cases(o.get("uninstaller"), out);
    if HOOK_KEYS.iter().any(|k| present(o.get(*k))) {
        out.insert(SubCase::Hook);
    }
}

/// The root object followed by every object under `architecture`.
fn objects(root: &Value) -> Vec<&Map<String, Value>> {
    let mut out = Vec::new();
    if let Some(r) = root.as_object() {
        out.push(r);
        if let Some(arch) = r.get("architecture").and_then(Value::as_object) {
            out.extend(arch.values().filter_map(Value::as_object));
        }
    }
    out
}

/// Sub-cases of a Scoop manifest. `Ok(None)` when it needs none (not phase 2c).
pub fn classify(raw_json: &str) -> Result<Option<Classification>, serde_json::Error> {
    let root: Value = serde_json::from_str(raw_json)?;
    let mut cases = BTreeSet::new();
    for o in objects(&root) {
        object_cases(o, &mut cases);
    }
    Ok(cases
        .last()
        .copied()
        .map(|worst| Classification { cases, worst }))
}

/// Script text of a hook value (string or array of strings, joined by newlines).
fn script_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

/// Command names called by every script of the manifest (hooks, `installer.script`,
/// `uninstaller.script`; root then each `architecture` variant), deduplicated
/// case-insensitively in order of first appearance, minus the names the manifest's own
/// scripts define with `function`/`filter`. Invalid JSON ⇒ empty.
pub fn script_commands(raw_json: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<Value>(raw_json) else {
        return Vec::new();
    };
    let mut lex = Lexer::default();
    for o in objects(&root) {
        let sub = |k: &str| o.get(k).and_then(|i| i.get("script"));
        let scripts = [
            o.get("pre_install"),
            sub("installer"),
            o.get("post_install"),
            o.get("pre_uninstall"),
            sub("uninstaller"),
            o.get("post_uninstall"),
        ];
        for text in scripts.into_iter().filter_map(script_text) {
            lex.script(&text);
        }
    }
    lex.finish()
}

/// Command names in one PowerShell script (see [`Lexer`] for the rules), deduplicated
/// case-insensitively, minus the functions it defines.
pub fn commands_in(script: &str) -> Vec<String> {
    let mut lex = Lexer::default();
    lex.script(script);
    lex.finish()
}

/// PowerShell keywords: never reported as commands.
const KEYWORDS: [&str; 27] = [
    "begin",
    "break",
    "catch",
    "continue",
    "data",
    "do",
    "dynamicparam",
    "else",
    "elseif",
    "end",
    "exit",
    "filter",
    "finally",
    "for",
    "foreach",
    "function",
    "if",
    "in",
    "param",
    "process",
    "return",
    "switch",
    "throw",
    "trap",
    "try",
    "until",
    "while",
];

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '\\' | '/' | ':')
}

/// A deliberately simple PowerShell lexer that collects command names:
/// - a command is the first word at a command position: start of line or after `|`,
///   `;`, `{`, `}`, `(`, `&` (so `&&`, `||` too) or `=`;
/// - a word must start with a letter or `_` (`$var`, `.\x.exe`, `-flag`, numbers and
///   `[type]` and `${braced}` variables are not commands), must not be a keyword, and must not be followed by
///   `=` (hashtable key or assignment target);
/// - the name after `function`/`filter` is a definition (scope prefix such as `global:`
///   dropped); defined names are removed from the result by [`Lexer::finish`];
/// - skipped: `#` comments to end of line (a `#` glued to a word, as in `a#b`, is part of it), `<# #>` block comments, `'...'` strings,
///   here-strings `@'`/`@"` up to a line starting with `'@`/`"@`, and the character
///   after a backtick (line continuation);
/// - `"..."` strings and `@" "@` here-strings are skipped except for `$( ... )`
///   subexpressions, which are lexed recursively up to the matching `)` (as are
///   `$( ... )` outside strings, through the `(` rule), up to [`MAX_NEST`] levels.
#[derive(Default)]
struct Lexer {
    calls: Vec<String>,
    defined: BTreeSet<String>,
    /// Current `$( ... )`-in-string nesting; capped at [`MAX_NEST`] (no unbounded recursion).
    nest: usize,
}

/// Deeper `$(` inside strings are skipped as plain text.
const MAX_NEST: usize = 32;

impl Lexer {
    fn script(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        self.code(&chars, 0, false);
    }

    /// Calls in order of first appearance (case-insensitive), minus defined names.
    fn finish(self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        self.calls
            .into_iter()
            .filter(|c| {
                let lower = c.to_ascii_lowercase();
                !self.defined.contains(&lower) && seen.insert(lower)
            })
            .collect()
    }

    /// Lexes code from `i`. With `sub`, stops after the `)` that closes the `$(` already
    /// consumed by the caller. Returns the index after the last consumed char.
    fn code(&mut self, chars: &[char], mut i: usize, sub: bool) -> usize {
        let n = chars.len();
        let mut at_start = true;
        let mut skip_next_word = false;
        let mut depth = 0usize;
        while i < n {
            let c = chars[i];
            match c {
                ')' if sub && depth == 0 => return i + 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    at_start = false;
                    i += 1;
                }
                '(' => {
                    depth += 1;
                    at_start = true;
                    i += 1;
                }
                '\n' | '\r' | '|' | ';' | '{' | '}' | '&' | '=' => {
                    at_start = true;
                    i += 1;
                }
                ' ' | '\t' => i += 1,
                '$' if chars.get(i + 1) == Some(&'{') => {
                    // `${name}` variable: never a command position.
                    while i < n && chars[i] != '}' {
                        i += 1;
                    }
                    i += 1;
                    at_start = false;
                }
                '`' => {
                    // Escaped char / line continuation: keep the command position as is.
                    i += 2;
                }
                '<' if chars.get(i + 1) == Some(&'#') => {
                    i += 2;
                    while i < n && !(chars[i] == '#' && chars.get(i + 1) == Some(&'>')) {
                        i += 1;
                    }
                    i += 2;
                }
                // As in pwsh, `#` right after a word char continues the word (`a#b`).
                '#' if i > 0 && is_word_char(chars[i - 1]) => {
                    at_start = false;
                    i += 1;
                }
                '#' => {
                    while i < n && chars[i] != '\n' {
                        i += 1;
                    }
                }
                '@' if here_string_start(chars, i) => {
                    let quote = chars[i + 1];
                    i = self.here_string(chars, i + 2, quote);
                    at_start = false;
                }
                '\'' => {
                    i += 1;
                    while i < n {
                        if chars[i] == '\'' {
                            if chars.get(i + 1) == Some(&'\'') {
                                i += 2;
                                continue;
                            }
                            break;
                        }
                        i += 1;
                    }
                    i += 1;
                    at_start = false;
                }
                '"' => {
                    i += 1;
                    while i < n && chars[i] != '"' {
                        i = self.expandable_char(chars, i);
                    }
                    i += 1;
                    at_start = false;
                }
                _ if is_word_char(c) => {
                    let start = i;
                    while i < n && is_word_char(chars[i]) {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    if at_start || skip_next_word {
                        let lower = word.to_ascii_lowercase();
                        if skip_next_word {
                            skip_next_word = false;
                            let name = lower.rsplit(':').next().unwrap_or_default();
                            self.defined.insert(name.to_string());
                        } else if KEYWORDS.contains(&lower.as_str()) {
                            skip_next_word = lower == "function" || lower == "filter";
                            // Command position continues after a keyword (`else Foo`).
                            continue;
                        } else {
                            let mut j = i;
                            while j < n && matches!(chars[j], ' ' | '\t') {
                                j += 1;
                            }
                            let assigned = chars.get(j) == Some(&'=');
                            let first = word.chars().next().unwrap_or('0');
                            if !assigned && (first.is_alphabetic() || first == '_') {
                                self.calls.push(word);
                            }
                        }
                    }
                    at_start = false;
                }
                _ => {
                    at_start = false;
                    i += 1;
                }
            }
        }
        n
    }

    /// One step inside an expandable string at `i`: skips an escaped char, lexes a
    /// `$( ... )` subexpression, or moves one char. Returns the next index.
    fn expandable_char(&mut self, chars: &[char], i: usize) -> usize {
        match chars[i] {
            '`' => i + 2,
            '$' if chars.get(i + 1) == Some(&'(') && self.nest < MAX_NEST => {
                self.nest += 1;
                let next = self.code(chars, i + 2, true);
                self.nest -= 1;
                next
            }
            _ => i + 1,
        }
    }

    /// Index just after the closing `'@`/`"@` (a line starting, after blanks, with it),
    /// or the end of the script when it is unterminated. `@"` bodies have their
    /// `$( ... )` subexpressions lexed.
    fn here_string(&mut self, chars: &[char], mut i: usize, quote: char) -> usize {
        let n = chars.len();
        while i < n {
            if chars[i] == '\n' {
                let mut j = i + 1;
                while j < n && matches!(chars[j], ' ' | '\t') {
                    j += 1;
                }
                if chars.get(j) == Some(&quote) && chars.get(j + 1) == Some(&'@') {
                    return j + 2;
                }
            }
            i = if quote == '"' {
                self.expandable_char(chars, i)
            } else {
                i + 1
            };
        }
        n
    }
}

/// `@'` or `@"` followed only by blanks up to the end of the line.
fn here_string_start(chars: &[char], i: usize) -> bool {
    if !matches!(chars.get(i + 1), Some('\'' | '"')) {
        return false;
    }
    let mut j = i + 2;
    while j < chars.len() && matches!(chars[j], ' ' | '\t' | '\r') {
        j += 1;
    }
    j >= chars.len() || chars[j] == '\n'
}
