//! Matches a Scoop app to its winget twin by name and homepage, to borrow the twin's icon.

/// Lowercase letters and digits only.
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Host without `www.`; for code forges also the owner (first path segment). All lowercase.
pub fn site(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.is_empty() || !host.contains('.') {
        return None;
    }
    if matches!(host.as_str(), "github.com" | "gitlab.com" | "codeberg.org") {
        let owner = path
            .trim_start_matches('/')
            .split(['/', '?', '#'])
            .next()
            .filter(|s| !s.is_empty())?;
        return Some(format!("{host}/{}", owner.to_ascii_lowercase()));
    }
    Some(host)
}

/// The id of the only candidate on the same site as the Scoop homepage; `None` when no
/// candidate or more than one matches.
pub fn pick(scoop_homepage: &str, candidates: &[(String, Option<String>)]) -> Option<String> {
    let want = site(scoop_homepage)?;
    let mut same = candidates
        .iter()
        .filter(|(_, home)| home.as_deref().and_then(site).as_deref() == Some(want.as_str()));
    let first = same.next()?;
    if same.next().is_some() {
        return None;
    }
    Some(first.0.clone())
}
