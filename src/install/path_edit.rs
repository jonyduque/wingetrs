//! Pure editing of the user `Path` value.

fn norm(s: &str) -> String {
    s.strip_suffix('\\').unwrap_or(s).to_lowercase()
}

/// Appends `entry` unless an equivalent entry exists (case-insensitive, ignoring one trailing '\').
/// Never touches other entries: the result is `existing` + (";" if existing non-empty and not ending in ';') + entry.
pub fn path_add(existing: &str, entry: &str) -> String {
    if path_contains(existing, entry) {
        return existing.to_string();
    }
    if existing.is_empty() || existing.ends_with(';') {
        format!("{existing}{entry}")
    } else {
        format!("{existing};{entry}")
    }
}

/// Removes every entry equivalent to `entry`; all other bytes (including empty segments, %VARS%) stay.
pub fn path_remove(existing: &str, entry: &str) -> String {
    let target = norm(entry);
    existing
        .split(';')
        .filter(|seg| norm(seg) != target)
        .collect::<Vec<_>>()
        .join(";")
}

pub fn path_contains(existing: &str, entry: &str) -> bool {
    let target = norm(entry);
    existing.split(';').any(|seg| norm(seg) == target)
}
