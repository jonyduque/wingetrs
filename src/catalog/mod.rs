//! Unified package catalog backed by SQLite.
pub mod index_file;
pub mod search;

use crate::core::{PackageEntry, SourceKind};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
  source TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  source_version TEXT NOT NULL,
  fetched_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS packages (
  rowid INTEGER PRIMARY KEY,
  source TEXT NOT NULL,
  kind TEXT NOT NULL,
  id TEXT NOT NULL,
  name TEXT NOT NULL,
  moniker TEXT,
  version TEXT NOT NULL,
  publisher TEXT,
  description TEXT,
  tags TEXT NOT NULL DEFAULT '',
  commands TEXT NOT NULL DEFAULT '',
  arp_min_version TEXT,
  arp_max_version TEXT,
  winget_hash BLOB,
  raw TEXT,
  UNIQUE (source, id)
);
CREATE TABLE IF NOT EXISTS map_productcode    (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS map_upgradecode    (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS map_pfn            (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS map_norm_name      (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS map_norm_publisher (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS ix_pc  ON map_productcode(key);
CREATE INDEX IF NOT EXISTS ix_uc  ON map_upgradecode(key);
CREATE INDEX IF NOT EXISTS ix_pfn ON map_pfn(key);
CREATE INDEX IF NOT EXISTS ix_nn  ON map_norm_name(key);
CREATE INDEX IF NOT EXISTS ix_np  ON map_norm_publisher(key);
";

type MapPair = (String, String);

const MAP_TABLES: [&str; 5] = [
    "map_productcode",
    "map_upgradecode",
    "map_pfn",
    "map_norm_name",
    "map_norm_publisher",
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewPackage {
    pub id: String,
    pub name: String,
    pub moniker: Option<String>,
    pub version: String,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub commands: Vec<String>,
    pub arp_min_version: Option<String>,
    pub arp_max_version: Option<String>,
    pub winget_hash: Option<Vec<u8>>,
    pub raw: Option<String>,
}

/// (key, package id) pairs; ids refer to NewPackage.id of the same import.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WingetMaps {
    pub productcodes: Vec<(String, String)>,
    pub upgradecodes: Vec<(String, String)>,
    pub pfns: Vec<(String, String)>,
    pub norm_names: Vec<(String, String)>,
    pub norm_publishers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapTable {
    ProductCode,
    UpgradeCode,
    Pfn,
    NormName,
    NormPublisher,
}

impl MapTable {
    fn table(self) -> &'static str {
        match self {
            MapTable::ProductCode => "map_productcode",
            MapTable::UpgradeCode => "map_upgradecode",
            MapTable::Pfn => "map_pfn",
            MapTable::NormName => "map_norm_name",
            MapTable::NormPublisher => "map_norm_publisher",
        }
    }

    /// Codes and package family names are stored lowercase; normalized names are exact.
    fn lowercases(self) -> bool {
        matches!(
            self,
            MapTable::ProductCode | MapTable::UpgradeCode | MapTable::Pfn
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceMeta {
    pub source: String,
    pub kind: SourceKind,
    pub source_version: String,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredPackage {
    pub entry: PackageEntry,
    pub arp_min_version: Option<String>,
    pub arp_max_version: Option<String>,
    pub winget_hash: Option<Vec<u8>>,
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportStats {
    pub packages: usize,
    pub map_rows: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("erro no catálogo local: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("mapeamento aponta para pacote inexistente: {0}")]
    DanglingMap(String),
}

/// Read-only connection with the settings used by every reader.
pub(crate) fn open_worker_connection(path: &Path) -> Result<Connection, CatalogError> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let _ = conn.pragma_update(None, "mmap_size", 268_435_456i64);
    Ok(conn)
}

/// Splits `lo..=hi` into up to 8 contiguous rowid ranges (one when the span is small).
pub(crate) fn rowid_ranges(lo: i64, hi: i64) -> Vec<(i64, i64)> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 8) as i64;
    if workers <= 1 || hi - lo <= 1024 {
        return vec![(lo, hi)];
    }
    let step = (hi - lo) / workers + 1;
    (0..workers)
        .map(|i| (lo + i * step, (lo + (i + 1) * step - 1).min(hi)))
        .collect()
}

pub struct Catalog {
    conn: Connection,
    /// File path when opened from disk; lets `load_entries` read with several connections.
    path: Option<std::path::PathBuf>,
}

const PKG_COLS: &str = "source, kind, id, name, moniker, version, publisher, description, tags, commands, \
                        arp_min_version, arp_max_version, winget_hash, raw";

fn split_words(s: String) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

fn kind_from_db(s: &str) -> SourceKind {
    s.parse().unwrap_or(SourceKind::Winget)
}

/// Columns of `PackageEntry` only (first ten of `PKG_COLS`): skips `raw` and `winget_hash`.
const ENTRY_COLS: &str =
    "source, kind, id, name, moniker, version, publisher, description, tags, commands";

fn entry_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PackageEntry> {
    let kind: String = r.get(1)?;
    Ok(PackageEntry {
        source: kind_from_db(&kind),
        source_name: r.get(0)?,
        id: r.get(2)?,
        name: r.get(3)?,
        moniker: r.get(4)?,
        version: r.get(5)?,
        publisher: r.get(6)?,
        description: r.get(7)?,
        tags: split_words(r.get(8)?),
        commands: split_words(r.get(9)?),
    })
}

fn stored_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredPackage> {
    Ok(StoredPackage {
        entry: entry_from_row(r)?,
        arp_min_version: r.get(10)?,
        arp_max_version: r.get(11)?,
        winget_hash: r.get(12)?,
        raw: r.get(13)?,
    })
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Catalog, CatalogError> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Reads through the OS mapping instead of pread copies (best effort).
        let _ = conn.pragma_update(None, "mmap_size", 268_435_456i64);
        conn.execute_batch(SCHEMA)?;
        Ok(Catalog {
            conn,
            path: Some(path.to_path_buf()),
        })
    }

    /// Read-only open for an existing catalog: no schema DDL, no journal_mode write.
    /// Never creates the file; fails when it does not exist.
    pub fn open_read_only(path: &Path) -> Result<Catalog, CatalogError> {
        let conn = open_worker_connection(path)?;
        Ok(Catalog {
            conn,
            path: Some(path.to_path_buf()),
        })
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn rowid_bounds(&self) -> Result<Option<(i64, i64)>, CatalogError> {
        let (lo, hi): (Option<i64>, Option<i64>) =
            self.conn
                .query_row("SELECT min(rowid), max(rowid) FROM packages", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        Ok(lo.zip(hi))
    }

    /// Entries for the given rowids, keyed by rowid.
    pub(crate) fn entries_by_rowid(
        &self,
        ids: &[i64],
    ) -> Result<HashMap<i64, PackageEntry>, CatalogError> {
        let mut out = HashMap::with_capacity(ids.len());
        if ids.is_empty() {
            return Ok(out);
        }
        let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let mut st = self.conn.prepare(&format!(
            "SELECT {ENTRY_COLS}, rowid FROM packages WHERE rowid IN ({list})"
        ))?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(10)?, entry_from_row(r)?)))?;
        for r in rows {
            let (id, e) = r?;
            out.insert(id, e);
        }
        Ok(out)
    }

    pub fn open_in_memory() -> Result<Catalog, CatalogError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Catalog { conn, path: None })
    }

    pub fn replace_source(
        &mut self,
        source: &str,
        kind: SourceKind,
        source_version: &str,
        fetched_at: i64,
        packages: &[NewPackage],
        maps: &WingetMaps,
    ) -> Result<ImportStats, CatalogError> {
        // Dropping `tx` on an early return rolls the whole import back.
        let tx = self.conn.transaction()?;
        for t in MAP_TABLES {
            tx.execute(
                &format!("DELETE FROM {t} WHERE package IN (SELECT rowid FROM packages WHERE source = ?1)"),
                params![source],
            )?;
        }
        tx.execute("DELETE FROM packages WHERE source = ?1", params![source])?;
        let mut rowids: HashMap<&str, i64> = HashMap::with_capacity(packages.len());
        {
            let mut ins = tx.prepare(&format!(
                "INSERT INTO packages ({PKG_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)"
            ))?;
            for p in packages {
                ins.execute(params![
                    source,
                    kind.as_str(),
                    p.id,
                    p.name,
                    p.moniker,
                    p.version,
                    p.publisher,
                    p.description,
                    p.tags.join(" "),
                    p.commands.join(" "),
                    p.arp_min_version,
                    p.arp_max_version,
                    p.winget_hash,
                    p.raw,
                ])?;
                rowids.insert(p.id.as_str(), tx.last_insert_rowid());
            }
        }
        let mut map_rows = 0usize;
        let groups: [(&str, &Vec<MapPair>, bool); 5] = [
            ("map_productcode", &maps.productcodes, true),
            ("map_upgradecode", &maps.upgradecodes, true),
            ("map_pfn", &maps.pfns, true),
            ("map_norm_name", &maps.norm_names, false),
            ("map_norm_publisher", &maps.norm_publishers, false),
        ];
        for (table, pairs, lower) in groups {
            let mut ins = tx.prepare(&format!(
                "INSERT INTO {table} (key, package) VALUES (?1, ?2)"
            ))?;
            for (key, id) in pairs {
                let Some(&rowid) = rowids.get(id.as_str()) else {
                    return Err(CatalogError::DanglingMap(id.clone()));
                };
                let key = if lower {
                    key.to_lowercase()
                } else {
                    key.clone()
                };
                ins.execute(params![key, rowid])?;
                map_rows += 1;
            }
        }
        tx.execute(
            "INSERT INTO meta (source, kind, source_version, fetched_at) VALUES (?1,?2,?3,?4)
             ON CONFLICT(source) DO UPDATE SET kind=excluded.kind,
               source_version=excluded.source_version, fetched_at=excluded.fetched_at",
            params![source, kind.as_str(), source_version, fetched_at],
        )?;
        tx.commit()?;
        Ok(ImportStats {
            packages: packages.len(),
            map_rows,
        })
    }

    pub fn sources(&self) -> Result<Vec<SourceMeta>, CatalogError> {
        let mut st = self
            .conn
            .prepare("SELECT source, kind, source_version, fetched_at FROM meta ORDER BY source")?;
        let rows = st.query_map([], |r| {
            let kind: String = r.get(1)?;
            Ok(SourceMeta {
                source: r.get(0)?,
                kind: kind_from_db(&kind),
                source_version: r.get(2)?,
                fetched_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Loads every package as a search entry (no `raw`/`winget_hash`), in rowid order.
    /// On-disk catalogs are decoded by several read connections, one rowid range each.
    pub fn load_entries(&self) -> Result<Vec<PackageEntry>, CatalogError> {
        // A writer committing mid-load would give workers different snapshots: retry once.
        let version = self.data_version()?;
        let out = self.load_entries_once()?;
        if self.data_version()? == version {
            return Ok(out);
        }
        self.load_entries_once()
    }

    pub(crate) fn data_version(&self) -> Result<i64, CatalogError> {
        Ok(self
            .conn
            .query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }

    fn load_entries_once(&self) -> Result<Vec<PackageEntry>, CatalogError> {
        let Some((lo, hi)) = self.rowid_bounds()? else {
            return Ok(Vec::new());
        };
        let ranges = rowid_ranges(lo, hi);
        let Some(path) = self.path.as_deref().filter(|_| ranges.len() > 1) else {
            return Self::load_range(&self.conn, lo, hi);
        };
        let parts: Vec<Result<Vec<PackageEntry>, CatalogError>> = std::thread::scope(|s| {
            let handles: Vec<_> = ranges
                .iter()
                .map(|&(from, to)| {
                    s.spawn(move || {
                        let conn = open_worker_connection(path)?;
                        Self::load_range(&conn, from, to)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                .collect()
        });
        let parts = parts.into_iter().collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(parts.iter().map(Vec::len).sum());
        for p in parts {
            out.extend(p);
        }
        Ok(out)
    }

    fn load_range(
        conn: &Connection,
        from: i64,
        to: i64,
    ) -> Result<Vec<PackageEntry>, CatalogError> {
        let mut st = conn.prepare(&format!(
            "SELECT {ENTRY_COLS} FROM packages WHERE rowid BETWEEN ?1 AND ?2 ORDER BY rowid"
        ))?;
        let rows = st.query_map(params![from, to], entry_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn get(&self, source: &str, id: &str) -> Result<Option<StoredPackage>, CatalogError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {PKG_COLS} FROM packages WHERE source = ?1 AND id = ?2"),
                params![source, id],
                stored_from_row,
            )
            .optional()?)
    }

    /// Finds by id across sources (case-insensitive); used by `show <id>` without --source.
    pub fn find_by_id(&self, id: &str) -> Result<Vec<StoredPackage>, CatalogError> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {PKG_COLS} FROM packages WHERE lower(id) = lower(?1) ORDER BY rowid"
        ))?;
        let rows = st.query_map(params![id], stored_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Package ids (winget source) whose map key equals `key` (case rules above).
    pub fn lookup(&self, table: MapTable, key: &str) -> Result<Vec<String>, CatalogError> {
        let key = if table.lowercases() {
            key.to_lowercase()
        } else {
            key.to_owned()
        };
        let mut st = self.conn.prepare(&format!(
            "SELECT p.id FROM {} m JOIN packages p ON p.rowid = m.package WHERE m.key = ?1 ORDER BY p.rowid",
            table.table()
        ))?;
        let rows = st.query_map(params![key], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn is_empty(&self) -> Result<bool, CatalogError> {
        let n: i64 = self
            .conn
            .query_row("SELECT EXISTS(SELECT 1 FROM packages)", [], |r| r.get(0))?;
        Ok(n == 0)
    }
}
