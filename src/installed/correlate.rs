//! Installed <-> catalog correlation and update state (spec §5.4).
use std::collections::BTreeMap;

use crate::catalog::{Catalog, CatalogError, MapTable};
use crate::core::version::Version;
use crate::installed::normalize::NameNormalizer;
use crate::installed::overrides::{self, Choice};
use crate::installed::tiebreak::{Candidate, Tiebreak, break_tie};
use crate::installed::winget_tracking::Tracking;
use crate::installed::{InstallSource, InstalledApp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Rule {
    ProductCode,
    UpgradeCode,
    PackageFamilyName,
    NameAndPublisher,
    ScoopManifest,
    /// Chosen by the user (`correlations.json`).
    User,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum Correlation {
    Matched {
        source: String,
        package_id: String,
        rule: Rule,
    },
    Ambiguous {
        /// Catalog source name of every candidate.
        source: String,
        candidates: Vec<String>,
        rule: Rule,
    },
    Unmatched,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum UpdateState {
    UpToDate,
    Available {
        latest: String,
    },
    Unknown,
    /// Installed by wingetrs, but `current` or its `install.json` is missing (run `repair`).
    Broken,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ListRow {
    pub name: String,
    /// Stable identity of the installed app (`overrides::app_key`).
    pub app_key: String,
    /// The user said this app is not in the catalog (`Choice::NotInCatalog`).
    pub not_in_catalog: bool,
    /// Who installed it: `arp`, `msix`, `scoop` or `wingetrs`.
    pub origin: String,
    pub installed_version: Option<String>,
    pub correlation: Correlation,
    pub latest: Option<String>,
    pub state: UpdateState,
    /// Tie-break that turned an ambiguous correlation into `Matched` (debugging aid).
    pub tiebreak: Option<Tiebreak>,
}

const WINGET_SOURCE: &str = "winget";

/// Decides whether `latest` is an upgrade over `installed`. With an ARP version
/// range, the installed version is compared against its upper bound instead
/// (catalog versions and ARP display versions often use different schemes).
pub fn update_state(
    installed: Option<&str>,
    latest: &str,
    _arp_min: Option<&str>,
    arp_max: Option<&str>,
) -> UpdateState {
    let Some(raw) = installed.map(str::trim).filter(|s| !s.is_empty()) else {
        return UpdateState::Unknown;
    };
    let inst = Version::parse(raw);
    if inst.is_unknown() {
        return UpdateState::Unknown;
    }
    let available = UpdateState::Available {
        latest: latest.to_owned(),
    };
    match arp_max.map(str::trim).filter(|s| !s.is_empty()) {
        Some(max) => {
            if inst >= Version::parse(max) {
                UpdateState::UpToDate
            } else {
                available
            }
        }
        None => {
            if inst < Version::parse(latest) {
                available
            } else {
                UpdateState::UpToDate
            }
        }
    }
}

/// Candidates found by one rule: (source, ids), deduplicated in catalog order.
type Hit = (String, Vec<String>, Rule);

fn dedup(ids: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(ids.len());
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

fn by_key(
    catalog: &Catalog,
    table: MapTable,
    key: Option<&str>,
    rule: Rule,
) -> Result<Option<Hit>, CatalogError> {
    let Some(key) = key.map(str::trim).filter(|k| !k.is_empty()) else {
        return Ok(None);
    };
    let ids = dedup(catalog.lookup(table, key)?);
    Ok((!ids.is_empty()).then(|| (WINGET_SOURCE.to_owned(), ids, rule)))
}

fn by_name_and_publisher(
    app: &InstalledApp,
    catalog: &Catalog,
    normalizer: &NameNormalizer,
) -> Result<Option<Hit>, CatalogError> {
    // winget correlates MSIX only by family name; sparse packages share names with ARP installs.
    if !matches!(app.source, InstallSource::Arp(_)) {
        return Ok(None);
    }
    let keys = normalizer.index_keys(&app.name, app.publisher.as_deref().unwrap_or(""));
    if keys.name.is_empty() || keys.publisher.is_empty() {
        return Ok(None);
    }
    let mut names = Vec::new();
    if let Some(with_arch) = keys.name_with_arch.as_deref() {
        names = catalog.lookup(MapTable::NormName, with_arch)?;
    }
    if names.is_empty() {
        names = catalog.lookup(MapTable::NormName, &keys.name)?;
    }
    if names.is_empty() {
        return Ok(None);
    }
    let publishers = catalog.lookup(MapTable::NormPublisher, &keys.publisher)?;
    let ids = dedup(
        names
            .into_iter()
            .filter(|id| publishers.contains(id))
            .collect(),
    );
    Ok((!ids.is_empty()).then(|| (WINGET_SOURCE.to_owned(), ids, Rule::NameAndPublisher)))
}

/// Scoop and wingetrs installs record their bucket; both correlate by `scoop:<bucket>` + app.
fn by_scoop(app: &InstalledApp, catalog: &Catalog) -> Result<Option<Hit>, CatalogError> {
    let (InstallSource::Scoop | InstallSource::Wingetrs, Some(bucket)) =
        (&app.source, app.scoop_bucket.as_deref())
    else {
        return Ok(None);
    };
    let source = crate::sources::scoop::source_name(bucket);
    Ok(catalog
        .get(&source, &app.key)?
        .map(|p| (source, vec![p.entry.id], Rule::ScoopManifest)))
}

/// First rule that finds candidates decides (spec §5.4 order).
fn find(
    app: &InstalledApp,
    catalog: &Catalog,
    normalizer: &NameNormalizer,
) -> Result<Option<Hit>, CatalogError> {
    let keyed = [
        (
            MapTable::ProductCode,
            app.product_code.as_deref(),
            Rule::ProductCode,
        ),
        (
            MapTable::UpgradeCode,
            app.upgrade_code.as_deref(),
            Rule::UpgradeCode,
        ),
        (
            MapTable::Pfn,
            app.family_name.as_deref(),
            Rule::PackageFamilyName,
        ),
    ];
    for (table, key, rule) in keyed {
        if let Some(hit) = by_key(catalog, table, key, rule)? {
            return Ok(Some(hit));
        }
    }
    if let Some(hit) = by_name_and_publisher(app, catalog, normalizer)? {
        return Ok(Some(hit));
    }
    by_scoop(app, catalog)
}

pub fn origin(source: &InstallSource) -> &'static str {
    match source {
        InstallSource::Arp(_) => "arp",
        InstallSource::Msix => "msix",
        InstallSource::Scoop => "scoop",
        InstallSource::Wingetrs => "wingetrs",
    }
}

fn row(
    app: &InstalledApp,
    correlation: Correlation,
    latest: Option<String>,
    state: UpdateState,
) -> ListRow {
    // `scan_wingetrs` lists an app without version when its install is broken.
    let state = match (&app.source, &app.version) {
        (InstallSource::Wingetrs, None) => UpdateState::Broken,
        _ => state,
    };
    ListRow {
        name: app.name.clone(),
        app_key: overrides::app_key(app),
        not_in_catalog: false,
        origin: origin(&app.source).to_string(),
        installed_version: app.version.clone(),
        correlation,
        latest,
        state,
        tiebreak: None,
    }
}

/// Tries the objective tie-breaks on tied winget candidates; `None` keeps them ambiguous.
fn resolve_tie(
    app: &InstalledApp,
    source: &str,
    candidates: &[String],
    catalog: &Catalog,
    normalizer: &NameNormalizer,
    tracking: &Tracking,
) -> Result<Option<(String, Tiebreak)>, CatalogError> {
    let mut tied = Vec::with_capacity(candidates.len());
    for id in candidates {
        let pkg = catalog.get(source, id)?;
        tied.push(Candidate {
            id: id.clone(),
            arp_min_version: pkg.as_ref().and_then(|p| p.arp_min_version.clone()),
            arp_max_version: pkg.and_then(|p| p.arp_max_version),
        });
    }
    let tracked = if source == WINGET_SOURCE {
        tracking.ids_for(app, normalizer)
    } else {
        Vec::new()
    };
    Ok(break_tie(
        &app.name,
        app.version.as_deref(),
        &tied,
        &tracked,
    ))
}

fn matched_row(
    app: &InstalledApp,
    catalog: &Catalog,
    source: String,
    package_id: String,
    rule: Rule,
) -> Result<ListRow, CatalogError> {
    let pkg = catalog.get(&source, &package_id)?;
    let correlation = Correlation::Matched {
        source,
        package_id,
        rule,
    };
    Ok(match pkg {
        Some(p) => {
            let state = update_state(
                app.version.as_deref(),
                &p.entry.version,
                p.arp_min_version.as_deref(),
                p.arp_max_version.as_deref(),
            );
            row(app, correlation, Some(p.entry.version), state)
        }
        None => row(app, correlation, None, UpdateState::Unknown),
    })
}

/// The row the user's choice for `app` dictates; `None` when there is none, or when it points
/// at a package no longer in the catalog (the automatic correlation then stands).
fn user_choice(
    app: &InstalledApp,
    catalog: &Catalog,
    overrides: &BTreeMap<String, Choice>,
) -> Result<Option<ListRow>, CatalogError> {
    let key = overrides::app_key(app);
    match overrides.get(&key) {
        None => Ok(None),
        Some(Choice::NotInCatalog) => Ok(Some(ListRow {
            not_in_catalog: true,
            ..row(app, Correlation::Unmatched, None, UpdateState::Unknown)
        })),
        Some(Choice::Package { source, package_id }) => {
            if catalog.get(source, package_id)?.is_none() {
                tracing::warn!("escolha de {key} ignorada: {source}/{package_id} saiu do catálogo");
                return Ok(None);
            }
            matched_row(app, catalog, source.clone(), package_id.clone(), Rule::User).map(Some)
        }
    }
}

pub fn correlate(
    apps: &[InstalledApp],
    catalog: &Catalog,
    normalizer: &NameNormalizer,
    tracking: &Tracking,
    overrides: &BTreeMap<String, Choice>,
) -> Result<Vec<ListRow>, CatalogError> {
    let mut rows = Vec::with_capacity(apps.len());
    for app in apps {
        if let Some(r) = user_choice(app, catalog, overrides)? {
            rows.push(r);
            continue;
        }
        let r = match find(app, catalog, normalizer)? {
            None => row(app, Correlation::Unmatched, None, UpdateState::Unknown),
            Some((source, candidates, rule)) if candidates.len() > 1 => {
                match resolve_tie(app, &source, &candidates, catalog, normalizer, tracking)? {
                    Some((package_id, tiebreak)) => ListRow {
                        tiebreak: Some(tiebreak),
                        ..matched_row(app, catalog, source, package_id, rule)?
                    },
                    None => row(
                        app,
                        Correlation::Ambiguous {
                            source,
                            candidates,
                            rule,
                        },
                        None,
                        UpdateState::Unknown,
                    ),
                }
            }
            Some((source, mut ids, rule)) => {
                let package_id = ids.pop().unwrap_or_default();
                matched_row(app, catalog, source, package_id, rule)?
            }
        };
        rows.push(r);
    }
    Ok(rows)
}
