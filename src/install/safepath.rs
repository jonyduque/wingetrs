//! Validation of relative paths that come from untrusted data (archives, manifests).

/// Splits a relative path on `/` and `\` into safe components, dropping empty and `.`
/// components. Returns `None` for `..`, absolute paths, drive prefixes, UNC and `\\?\`
/// forms, alternate data streams (`:`), NUL and Windows-reserved names.
pub fn safe_components(raw: &str) -> Option<Vec<String>> {
    let norm = raw.replace('\\', "/");
    if norm.starts_with('/') {
        return None;
    }
    let mut out = Vec::new();
    for c in norm.split('/') {
        match c {
            "" | "." => {}
            ".." => return None,
            c if c.contains(':') || c.contains('\0') => return None,
            c if is_windows_unfriendly(c) => return None,
            c => out.push(c.to_string()),
        }
    }
    Some(out)
}

/// Windows reserved device names (with or without extension) and names ending in `.` or space.
pub fn is_windows_unfriendly(c: &str) -> bool {
    if c.ends_with('.') || c.ends_with(' ') {
        return true;
    }
    let stem = c
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|p| {
            stem.strip_prefix(p)
                .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
        })
}
