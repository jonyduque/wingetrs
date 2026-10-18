//! Installed-state database (`state.db`) and `install.json` of wingetrs-managed apps.
use crate::install::layout::Layout;
use crate::installed::{InstallSource, InstalledApp};
use rusqlite::{Connection, params};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("erro no banco de estado: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("tipo de artefato desconhecido no banco de estado: {0}")]
    UnknownKind(String),
    #[error("erro de E/S no estado: {0}")]
    Io(#[from] std::io::Error),
    #[error("o manifesto gravado de {0} não confere com o registro")]
    ManifestMismatch(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ArtifactKind {
    Shim,
    Shortcut,
    Persist,
    Path,
    Env,
}

impl ArtifactKind {
    fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Shim => "shim",
            ArtifactKind::Shortcut => "shortcut",
            ArtifactKind::Persist => "persist",
            ArtifactKind::Path => "path",
            ArtifactKind::Env => "env",
        }
    }
    fn parse(s: &str) -> Result<ArtifactKind, StateError> {
        Ok(match s {
            "shim" => ArtifactKind::Shim,
            "shortcut" => ArtifactKind::Shortcut,
            "persist" => ArtifactKind::Persist,
            "path" => ArtifactKind::Path,
            "env" => ArtifactKind::Env,
            other => return Err(StateError::UnknownKind(other.to_string())),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub kind: ArtifactKind,
    pub target: String,
    pub previous: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRecord {
    pub app: String,
    pub bucket: String,
    pub version: String,
    pub arch: Option<String>,
    pub installed_at: i64,
    pub manifest_sha256: String,
}

pub struct StateDb {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS apps(
    app TEXT PRIMARY KEY,
    bucket TEXT NOT NULL,
    version TEXT NOT NULL,
    arch TEXT,
    installed_at INTEGER NOT NULL,
    manifest_sha256 TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS artifacts(
    app TEXT NOT NULL,
    kind TEXT NOT NULL,
    target TEXT NOT NULL,
    previous TEXT
);
CREATE INDEX IF NOT EXISTS artifacts_app ON artifacts(app);
";

impl StateDb {
    pub fn open(path: &Path) -> Result<StateDb, StateError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(StateDb { conn })
    }

    /// Opens an existing `state.db` without creating it, its folder or its schema.
    /// `None` when the file is absent (nothing installed yet).
    pub fn open_existing(path: &Path) -> Result<Option<StateDb>, StateError> {
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        Ok(Some(StateDb { conn }))
    }

    /// Empty database in memory (stands in for an absent `state.db` when only reading).
    pub fn empty_in_memory() -> Result<StateDb, StateError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(StateDb { conn })
    }

    /// Replaces the app's record and artifacts in one transaction.
    pub fn record(&mut self, app: &AppRecord, artifacts: &[Artifact]) -> Result<(), StateError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM artifacts WHERE app = ?1", params![app.app])?;
        tx.execute(
            "INSERT OR REPLACE INTO apps(app, bucket, version, arch, installed_at, manifest_sha256) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                app.app,
                app.bucket,
                app.version,
                app.arch,
                app.installed_at,
                app.manifest_sha256
            ],
        )?;
        for a in artifacts {
            tx.execute(
                "INSERT INTO artifacts(app, kind, target, previous) VALUES (?1, ?2, ?3, ?4)",
                params![app.app, a.kind.as_str(), a.target, a.previous],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get(&self, app: &str) -> Result<Option<(AppRecord, Vec<Artifact>)>, StateError> {
        let mut stmt = self.conn.prepare(
            "SELECT app, bucket, version, arch, installed_at, manifest_sha256 FROM apps WHERE app = ?1",
        )?;
        let mut rows = stmt.query(params![app])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let rec = row_to_record(row)?;
        let mut stmt = self.conn.prepare(
            "SELECT kind, target, previous FROM artifacts WHERE app = ?1 ORDER BY rowid",
        )?;
        let raw = stmt
            .query_map(params![app], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut arts = Vec::with_capacity(raw.len());
        for (kind, target, previous) in raw {
            arts.push(Artifact {
                kind: ArtifactKind::parse(&kind)?,
                target,
                previous,
            });
        }
        Ok(Some((rec, arts)))
    }

    pub fn remove(&mut self, app: &str) -> Result<(), StateError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM artifacts WHERE app = ?1", params![app])?;
        tx.execute("DELETE FROM apps WHERE app = ?1", params![app])?;
        tx.commit()?;
        Ok(())
    }

    pub fn apps(&self) -> Result<Vec<AppRecord>, StateError> {
        let mut stmt = self.conn.prepare(
            "SELECT app, bucket, version, arch, installed_at, manifest_sha256 FROM apps ORDER BY app",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(row_to_record(row)?);
        }
        Ok(out)
    }

    /// Apps (other than `app`) whose recorded artifacts include a PATH entry equal to `entry`.
    pub fn path_users(&self, app: &str, entry: &str) -> Result<Vec<String>, StateError> {
        use crate::install::path_edit::norm;
        let want = norm(entry);
        let mut stmt = self.conn.prepare(
            "SELECT app, target FROM artifacts WHERE kind = 'path' AND app <> ?1 ORDER BY app",
        )?;
        let rows = stmt
            .query_map(params![app], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out: Vec<String> = Vec::new();
        for (a, t) in rows {
            if norm(&t) == want && !out.contains(&a) {
                out.push(a);
            }
        }
        Ok(out)
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<AppRecord> {
    Ok(AppRecord {
        app: row.get(0)?,
        bucket: row.get(1)?,
        version: row.get(2)?,
        arch: row.get(3)?,
        installed_at: row.get(4)?,
        manifest_sha256: row.get(5)?,
    })
}

/// Writes `<version_dir>\install.json` in the scoop format plus a `wingetrs` marker.
pub fn write_install_json(
    version_dir: &Path,
    bucket: &str,
    arch: Option<&str>,
    version: &str,
) -> std::io::Result<()> {
    let mut v = serde_json::json!({ "bucket": bucket, "version": version, "wingetrs": 1 });
    if let (Some(a), Some(obj)) = (arch, v.as_object_mut()) {
        obj.insert("architecture".into(), a.into());
    }
    let bytes = serde_json::to_vec_pretty(&v).map_err(std::io::Error::other)?;
    let tmp = version_dir.join("install.json.tmp");
    std::fs::write(&tmp, bytes)?;
    let dst = version_dir.join("install.json");
    crate::fs_retry::retry("renomear", &tmp, || std::fs::rename(&tmp, &dst))
}

/// Reads `<version_dir>\manifest.json` written at install time; `None` for apps installed
/// before Phase 2c. Bytes that are not UTF-8 or whose SHA-256 differs from the record are refused.
pub fn installed_manifest(
    layout: &Layout,
    app: &str,
    version: &str,
    expected_sha256: &str,
) -> Result<Option<String>, StateError> {
    let path = layout.version_dir(app, version).join("manifest.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if crate::cache::sha256_hex(&bytes) != expected_sha256 {
        return Err(StateError::ManifestMismatch(app.to_string()));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| StateError::ManifestMismatch(app.to_string()))
}

fn read_install_json(app_dir: &Path) -> Option<serde_json::Value> {
    let bytes = std::fs::read(app_dir.join("current").join("install.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Apps installed by wingetrs: `apps\<app>\current\install.json`, version from `state.db` when present.
/// An app whose `current` is broken is still listed, with `version: None`.
pub fn scan_wingetrs(layout: &Layout) -> Vec<InstalledApp> {
    let Ok(rd) = std::fs::read_dir(layout.apps()) else {
        return Vec::new();
    };
    let db = layout
        .state_db()
        .exists()
        .then(|| StateDb::open(&layout.state_db()).ok())
        .flatten();
    let mut apps = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(key) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let install = read_install_json(&path);
        let (bucket, version) = match &install {
            None => (None, None),
            Some(j) => {
                let from_db = db
                    .as_ref()
                    .and_then(|d| d.get(&key).ok().flatten())
                    .map(|(r, _)| r.version);
                let from_json = j
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                (
                    j.get("bucket").and_then(|v| v.as_str()).map(str::to_string),
                    from_db.or(from_json),
                )
            }
        };
        apps.push(InstalledApp {
            source: InstallSource::Wingetrs,
            name: key.clone(),
            key,
            publisher: None,
            version,
            product_code: None,
            upgrade_code: None,
            family_name: None,
            scoop_bucket: bucket,
            stale: None,
        });
    }
    apps.sort_by(|a, b| a.key.cmp(&b.key));
    apps
}
