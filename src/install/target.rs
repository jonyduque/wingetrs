//! Install target resolution: user text -> candidates from the catalog (never by fuzzy match).
use crate::catalog::search::SearchIndex;
use crate::catalog::{Catalog, CatalogError};
use crate::core::PackageEntry;

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub entry: PackageEntry,
    pub selectable: bool,
    pub note: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    #[error("erro no catálogo: {0}")]
    Catalog(#[from] CatalogError),
    #[error("nenhum pacote encontrado para '{query}'")]
    NotFound {
        query: String,
        suggestions: Vec<String>,
    },
}

const MAX_SUGGESTIONS: usize = 5;
const MIN_PREFIX: usize = 3;

/// `extras/vscode` -> (Some("extras"), "vscode"); winget ids like `A.B` have no slash.
fn split_bucket(query: &str) -> (Option<&str>, &str) {
    match query.split_once('/') {
        Some((b, rest)) if !b.is_empty() && !rest.is_empty() => (Some(b), rest),
        _ => (None, query),
    }
}

/// Every catalog package is selectable: scoop ones install through the own installer, winget
/// ones through the bridge (`winget install`).
pub(crate) fn candidate(entry: &PackageEntry) -> Candidate {
    Candidate {
        entry: entry.clone(),
        selectable: true,
        note: None,
    }
}

/// Resolves `query` (optionally restricted by `bucket`) to candidates:
/// exact id (case-insensitive), else exact moniker/name, else `NotFound` with fuzzy suggestions.
pub fn resolve(
    catalog: &Catalog,
    query: &str,
    bucket: Option<&str>,
) -> Result<Vec<Candidate>, TargetError> {
    let (prefix, text) = split_bucket(query.trim());
    if text.trim().is_empty() {
        return Err(TargetError::NotFound {
            query: query.to_owned(),
            suggestions: Vec::new(),
        });
    }
    let bucket = prefix.or(bucket);
    let wanted = bucket.map(|b| format!("scoop:{}", b.to_lowercase()));
    let entries: Vec<PackageEntry> = catalog
        .load_entries()?
        .into_iter()
        .filter(|e| {
            wanted
                .as_ref()
                .is_none_or(|w| e.source_name.to_lowercase() == *w)
        })
        .collect();
    let lower = text.to_lowercase();
    let by_id: Vec<Candidate> = entries
        .iter()
        .filter(|e| e.id.to_lowercase() == lower)
        .map(candidate)
        .collect();
    if !by_id.is_empty() {
        return Ok(by_id);
    }
    let by_name: Vec<Candidate> = entries
        .iter()
        .filter(|e| {
            e.name.to_lowercase() == lower
                || e.moniker
                    .as_ref()
                    .is_some_and(|m| m.to_lowercase() == lower)
        })
        .map(candidate)
        .collect();
    if !by_name.is_empty() {
        return Ok(by_name);
    }
    let index = SearchIndex::new(entries);
    // The matcher is subsequence-based, so a typo ("ripgrop") finds nothing: retry with shorter prefixes.
    let chars: Vec<char> = text.chars().collect();
    let mut hits = Vec::new();
    for len in (MIN_PREFIX.min(chars.len())..=chars.len()).rev() {
        let probe: String = chars[..len].iter().collect();
        hits = index.search(&probe, None, MAX_SUGGESTIONS);
        if !hits.is_empty() {
            break;
        }
    }
    let suggestions = hits
        .iter()
        .take(MAX_SUGGESTIONS)
        .filter_map(|h| index.entries().get(h.index))
        .map(|e| format!("{} ({})", e.id, e.source_name))
        .collect();
    Err(TargetError::NotFound {
        query: query.to_owned(),
        suggestions,
    })
}
