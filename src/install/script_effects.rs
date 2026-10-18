//! effects.jsonl written by the prelude (Task 153).
//!
//! Manifest scripts never touch the registry, the PATH or the Start Menu: the prelude's
//! intercepted functions append one JSON line per effect, and the executor applies the
//! parsed effects through the phase 2a path (with `Undo`).

use serde::Deserialize;

/// Maximum number of lines in `effects.jsonl` (checked before decoding).
const MAX_LINES: usize = 1000;
/// Maximum length of one line in bytes (checked before decoding).
const MAX_LINE_BYTES: usize = 64 * 1024;

/// One global effect requested by a manifest script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptEffect {
    SetEnv {
        name: String,
        value: Option<String>,
    },
    AddPath {
        var: String,
        entries: Vec<String>,
        prepend: bool,
        /// Move the entries to the front even if already present (`add_first_in_path`, `-Force`).
        force: bool,
    },
    RemovePath {
        var: String,
        entries: Vec<String>,
    },
    Shim {
        path: String,
        name: String,
        args: Option<String>,
    },
    RmShim {
        name: String,
    },
    Shortcut {
        target: String,
        name: String,
        args: Option<String>,
        icon: Option<String>,
    },
}

/// Why `effects.jsonl` was refused (line numbers are 1-based).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EffectsError {
    #[error("effects.jsonl: linha {0} inválida")]
    Invalid(usize),
    #[error("effects.jsonl: operação desconhecida na linha {0}")]
    UnknownOp(usize),
    #[error("effects.jsonl: mais de {MAX_LINES} linhas")]
    TooMany,
    #[error("effects.jsonl: linha {0} passa de {MAX_LINE_BYTES} bytes")]
    TooLong(usize),
}

#[derive(Deserialize)]
struct OpOnly {
    op: String,
}

/// Required key whose value may be `null` (a plain `Option` field would accept a missing key).
fn nullable<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

// One struct per op: every key is required and unknown keys are refused. `op` is
// declared only so `deny_unknown_fields` accepts it; its value was matched already.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetEnvLine {
    #[serde(rename = "op")]
    _op: String,
    name: String,
    #[serde(deserialize_with = "nullable")]
    value: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddPathLine {
    #[serde(rename = "op")]
    _op: String,
    var: String,
    entries: Vec<String>,
    prepend: bool,
    force: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemovePathLine {
    #[serde(rename = "op")]
    _op: String,
    var: String,
    entries: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShimLine {
    #[serde(rename = "op")]
    _op: String,
    path: String,
    name: String,
    #[serde(deserialize_with = "nullable")]
    args: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RmShimLine {
    #[serde(rename = "op")]
    _op: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShortcutLine {
    #[serde(rename = "op")]
    _op: String,
    target: String,
    name: String,
    #[serde(deserialize_with = "nullable")]
    args: Option<String>,
    #[serde(deserialize_with = "nullable")]
    icon: Option<String>,
}

fn decode<'a, T: Deserialize<'a>>(line: &'a str, n: usize) -> Result<T, EffectsError> {
    serde_json::from_str(line).map_err(|_| EffectsError::Invalid(n))
}

/// Parses `effects.jsonl`: one JSON object per line, empty lines skipped, leading BOM ignored.
pub fn parse_effects(text: &str) -> Result<Vec<ScriptEffect>, EffectsError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    // limits before any decoding
    if text.lines().count() > MAX_LINES {
        return Err(EffectsError::TooMany);
    }
    if let Some((i, _)) = text
        .lines()
        .enumerate()
        .find(|(_, l)| l.len() > MAX_LINE_BYTES)
    {
        return Err(EffectsError::TooLong(i + 1));
    }
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        let op: OpOnly = decode(line, n)?;
        // `other` names the refused op for readers; the error carries only the line number
        #[allow(unused_variables)]
        let effect = match op.op.as_str() {
            "set_env" => {
                let l: SetEnvLine = decode(line, n)?;
                ScriptEffect::SetEnv {
                    name: l.name,
                    value: l.value,
                }
            }
            "add_path" => {
                let l: AddPathLine = decode(line, n)?;
                ScriptEffect::AddPath {
                    var: l.var,
                    entries: l.entries,
                    prepend: l.prepend,
                    force: l.force,
                }
            }
            "remove_path" => {
                let l: RemovePathLine = decode(line, n)?;
                ScriptEffect::RemovePath {
                    var: l.var,
                    entries: l.entries,
                }
            }
            "shim" => {
                let l: ShimLine = decode(line, n)?;
                ScriptEffect::Shim {
                    path: l.path,
                    name: l.name,
                    args: l.args,
                }
            }
            "rm_shim" => {
                let l: RmShimLine = decode(line, n)?;
                ScriptEffect::RmShim { name: l.name }
            }
            "shortcut" => {
                let l: ShortcutLine = decode(line, n)?;
                ScriptEffect::Shortcut {
                    target: l.target,
                    name: l.name,
                    args: l.args,
                    icon: l.icon,
                }
            }
            other => return Err(EffectsError::UnknownOp(n)),
        };
        out.push(effect);
    }
    Ok(out)
}
