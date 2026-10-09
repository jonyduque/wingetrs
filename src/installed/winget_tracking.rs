//! Read-only view of winget's package tracking catalog (`installed.db`): the packages winget
//! itself installed from the `winget` source, with the system references recorded for them.
//! Never written to; see docs/reference/winget-cli/AppInstallerRepositoryCore_CompositeSource_SearchInstalled_3973956.cpp.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::installed::InstalledApp;
use crate::installed::normalize::NameNormalizer;

#[derive(Debug, thiserror::Error)]
pub enum TrackingError {
    #[error("catálogo de rastreamento do winget ilegível: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("catálogo de rastreamento do winget ilegível: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "catálogo de rastreamento do winget em modo WAL; ignorado para não criar arquivos na pasta do winget"
    )]
    Wal,
}

/// One tracked manifest: an id and the system references winget recorded for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackedPackage {
    pub id: String,
    pub product_codes: Vec<String>,
    pub upgrade_codes: Vec<String>,
    pub pfns: Vec<String>,
    pub norm_names: Vec<String>,
    pub norm_publishers: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Tracking {
    packages: Vec<TrackedPackage>,
}

fn eq_any(values: &[String], key: Option<&str>) -> bool {
    let Some(key) = key.map(str::trim).filter(|k| !k.is_empty()) else {
        return false;
    };
    let key = key.to_lowercase();
    values.iter().any(|v| v.to_lowercase() == key)
}

impl Tracking {
    pub fn new(packages: Vec<TrackedPackage>) -> Tracking {
        Tracking { packages }
    }

    /// Ids whose tracked system references match the installed app, as winget's
    /// "Tracking :: Search system references" step does (codes case-insensitive,
    /// normalized name and publisher from the same manifest).
    // port: CompositeSource.cpp CompositeSource::SearchInstalled (tracking search)
    pub fn ids_for(&self, app: &InstalledApp, normalizer: &NameNormalizer) -> Vec<String> {
        let keys = normalizer.index_keys(&app.name, app.publisher.as_deref().unwrap_or(""));
        let mut out: Vec<String> = Vec::new();
        for p in &self.packages {
            let by_code = eq_any(&p.product_codes, app.product_code.as_deref())
                || eq_any(&p.upgrade_codes, app.upgrade_code.as_deref())
                || eq_any(&p.pfns, app.family_name.as_deref());
            let by_name = !keys.name.is_empty()
                && !keys.publisher.is_empty()
                && p.norm_publishers.contains(&keys.publisher)
                && (p.norm_names.contains(&keys.name)
                    || keys
                        .name_with_arch
                        .as_ref()
                        .is_some_and(|n| p.norm_names.contains(n)));
            if (by_code || by_name) && !out.iter().any(|o| o.eq_ignore_ascii_case(&p.id)) {
                out.push(p.id.clone());
            }
        }
        out
    }
}

/// `(manifest rowid, value)` pairs of one V1 index map table.
fn read_map(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<Vec<(i64, String)>, TrackingError> {
    let sql = format!(
        "SELECT m.manifest, v.{column} FROM {table}_map m JOIN {table} v ON v.rowid = m.{column}"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// True when a SQLite file header (first 100 bytes) declares WAL mode (write version 2):
/// even a read-only connection may then create `-shm`/`-wal` next to the file.
pub fn header_is_wal(header: &[u8]) -> bool {
    header.get(18) == Some(&2)
}

/// Reads a tracking catalog (winget V1 SQLite index schema) opened strictly read-only.
/// WAL-mode files are refused so nothing is ever created in winget's folder.
pub fn read_tracking(path: &Path) -> Result<Tracking, TrackingError> {
    let mut header = Vec::with_capacity(100);
    std::io::Read::read_to_end(
        &mut std::io::Read::take(std::fs::File::open(path)?, 100),
        &mut header,
    )?;
    if header_is_wal(&header) {
        return Err(TrackingError::Wal);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut by_manifest: HashMap<i64, TrackedPackage> = HashMap::new();
    {
        let mut st =
            conn.prepare("SELECT m.rowid, i.id FROM manifest m JOIN ids i ON i.rowid = m.id")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (manifest, id) = row?;
            by_manifest.insert(
                manifest,
                TrackedPackage {
                    id,
                    ..Default::default()
                },
            );
        }
    }
    type Field = fn(&mut TrackedPackage) -> &mut Vec<String>;
    let maps: [(&str, &str, Field); 5] = [
        ("productcodes", "productcode", |p| &mut p.product_codes),
        ("upgradecodes", "upgradecode", |p| &mut p.upgrade_codes),
        ("pfns", "pfn", |p| &mut p.pfns),
        ("norm_names", "norm_name", |p| &mut p.norm_names),
        ("norm_publishers", "norm_publisher", |p| {
            &mut p.norm_publishers
        }),
    ];
    for (table, column, field) in maps {
        for (manifest, value) in read_map(&conn, table, column)? {
            if let Some(p) = by_manifest.get_mut(&manifest) {
                field(p).push(value);
            }
        }
    }
    let mut packages: Vec<(i64, TrackedPackage)> = by_manifest.into_iter().collect();
    packages.sort_by_key(|(m, _)| *m);
    Ok(Tracking::new(
        packages.into_iter().map(|(_, p)| p).collect(),
    ))
}

/// `%LOCALAPPDATA%\Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\
/// Microsoft.Winget.Source_8wekyb3d8bbwe\installed.db` (tracking for the `winget` source).
// port: PackageTrackingCatalog.cpp GetPackageTrackingFilePath
pub fn default_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local)
            .join("Packages")
            .join("Microsoft.DesktopAppInstaller_8wekyb3d8bbwe")
            .join("LocalState")
            .join("Microsoft.Winget.Source_8wekyb3d8bbwe")
            .join("installed.db"),
    )
}

/// Loads the default tracking catalog; absent or unreadable means "no tracking" (logged).
pub fn load_default() -> Tracking {
    let Some(path) = default_path().filter(|p| p.is_file()) else {
        return Tracking::default();
    };
    match read_tracking(&path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("{e}");
            Tracking::default()
        }
    }
}
