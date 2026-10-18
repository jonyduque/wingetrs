//! installer.file/args substitution (Task 151).
use std::path::Path;
use std::sync::OnceLock;

use fancy_regex::Regex;

use crate::install::planner::PlanError;

/// The only variables scoop substitutes in `installer.args` (`Invoke-Installer`).
const INSTALLER_VARS: [&str; 3] = ["$dir", "$global", "$version"];

/// Scoop `lib/core.ps1` `Invoke-ExternalCommand`: the split pattern has literal spaces.
const SPLIT: &str = r"(?<=(?<![:\w])[/-]\w+) | (?=[/-])";

/// Substitutes `$dir`, `$global` (always `False`) and `$version` (case-sensitive, one pass per
/// variable), then splits like scoop's `Invoke-ExternalCommand`: quotes dropped, each argument
/// cut at spaces after an option word or before `/`/`-`, empties discarded.
pub fn installer_args(
    args: &[String],
    dir: &Path,
    version: &str,
) -> Result<Vec<String>, PlanError> {
    let dir = dir.display().to_string();
    let values = [dir.as_str(), "False", version];
    let mut out = Vec::new();
    for arg in args {
        let mut text = arg.clone();
        for (var, value) in INSTALLER_VARS.iter().zip(values) {
            text = text.replace(var, value);
        }
        if text.chars().any(char::is_control) {
            return Err(PlanError::UnsafeArgs(format!("{text:?}")));
        }
        out.extend(split(&text.replace('"', ""))?);
    }
    Ok(out)
}

fn split(text: &str) -> Result<Vec<String>, PlanError> {
    static RE: OnceLock<Result<Regex, String>> = OnceLock::new();
    let re = RE
        .get_or_init(|| Regex::new(SPLIT).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| PlanError::UnsafeArgs(e.clone()))?;
    let mut parts = Vec::new();
    let mut last = 0;
    for m in re.find_iter(text) {
        let m = m.map_err(|e| PlanError::UnsafeArgs(e.to_string()))?;
        parts.push(&text[last..m.start()]);
        last = m.end();
    }
    parts.push(&text[last..]);
    Ok(parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}
