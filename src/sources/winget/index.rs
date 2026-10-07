//! Winget index acquisition: MSIX validation (AppxBlockMap) and v2 index.db reading.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

use crate::catalog::{NewPackage, WingetMaps};

const BLOCK_SIZE: usize = 65536;
const INDEX_ENTRY: &str = "Public/index.db";
const BLOCKMAP_ENTRY: &str = "AppxBlockMap.xml";
const INDEX_BLOCKMAP_NAME: &str = "Public\\index.db";

#[derive(Debug, Clone, PartialEq)]
pub struct WingetImport {
    pub packages: Vec<NewPackage>,
    pub maps: WingetMaps,
}

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("pacote MSIX inválido: {0}")]
    Zip(String),
    #[error("arquivo ausente no MSIX: {0}")]
    MissingEntry(&'static str),
    #[error("AppxBlockMap inválido: {0}")]
    BlockMap(String),
    #[error("integridade: bloco {block} do índice não confere")]
    Integrity { block: usize },
    #[error("integridade: hash do arquivo do índice não confere")]
    FileHash,
    #[error("formato de índice v{0} não suportado")]
    UnsupportedVersion(String),
    #[error("índice ilegível: {0}")]
    Sqlite(String),
    #[error("E/S: {0}")]
    Io(String),
}

fn sql(e: rusqlite::Error) -> IndexError {
    IndexError::Sqlite(e.to_string())
}

fn read_entry<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &'static str,
) -> Result<Vec<u8>, IndexError> {
    let mut f = zip.by_name(name).map_err(|e| match e {
        zip::result::ZipError::FileNotFound => IndexError::MissingEntry(name),
        other => IndexError::Zip(other.to_string()),
    })?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)
        .map_err(|e| IndexError::Zip(e.to_string()))?;
    Ok(buf)
}

/// Returns the validated bytes of Public/index.db.
pub fn validate_msix(msix: &[u8]) -> Result<Vec<u8>, IndexError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(msix))
        .map_err(|e| IndexError::Zip(e.to_string()))?;
    let db = read_entry(&mut zip, INDEX_ENTRY)?;
    let map = read_entry(&mut zip, BLOCKMAP_ENTRY)?;
    let xml = std::str::from_utf8(&map).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let doc = roxmltree::Document::parse(xml).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let file = doc
        .descendants()
        .find(|n| n.has_tag_name("File") && n.attribute("Name") == Some(INDEX_BLOCKMAP_NAME))
        .ok_or_else(|| IndexError::BlockMap("entrada Public\\index.db ausente".into()))?;
    let hashes: Vec<&str> = file
        .children()
        .filter(|n| n.has_tag_name("Block"))
        .filter_map(|n| n.attribute("Hash"))
        .collect();
    if hashes.len() != db.len().div_ceil(BLOCK_SIZE) {
        return Err(IndexError::BlockMap(format!(
            "esperados {} blocos, encontrados {}",
            db.len().div_ceil(BLOCK_SIZE),
            hashes.len()
        )));
    }
    for (i, (chunk, expected)) in db.chunks(BLOCK_SIZE).zip(&hashes).enumerate() {
        if STANDARD.encode(Sha256::digest(chunk)) != *expected {
            return Err(IndexError::Integrity { block: i });
        }
    }
    let file_hash = file
        .attributes()
        .find(|a| a.name() == "FileHash")
        .map(|a| a.value());
    if let Some(expected) = file_hash
        && STANDARD.encode(Sha256::digest(&db)) != expected
    {
        return Err(IndexError::FileHash);
    }
    Ok(db)
}

fn pairs(c: &rusqlite::Connection, query: &str) -> Result<Vec<(i64, String)>, IndexError> {
    let mut st = c.prepare(query).map_err(sql)?;
    let rows = st
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(sql)?;
    rows.collect::<Result<_, _>>().map_err(sql)
}

pub fn read_index(db_path: &Path) -> Result<WingetImport, IndexError> {
    let c =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(sql)?;
    let major: String = c
        .query_row(
            "select value from metadata where name='majorVersion'",
            [],
            |r| {
                r.get::<_, rusqlite::types::Value>(0).map(|v| match v {
                    rusqlite::types::Value::Text(t) => t,
                    rusqlite::types::Value::Integer(i) => i.to_string(),
                    _ => String::new(),
                })
            },
        )
        .map_err(sql)?;
    if major != "2" {
        return Err(IndexError::UnsupportedVersion(major));
    }

    let mut tags: HashMap<i64, Vec<String>> = HashMap::new();
    for (pkg, tag) in pairs(
        &c,
        "select m.package, t.tag from tags2_map m join tags2 t on t.rowid = m.tag",
    )? {
        tags.entry(pkg).or_default().push(tag);
    }
    let mut commands: HashMap<i64, Vec<String>> = HashMap::new();
    for (pkg, cmd) in pairs(
        &c,
        "select m.package, t.command from commands2_map m join commands2 t on t.rowid = m.command",
    )? {
        commands.entry(pkg).or_default().push(cmd);
    }

    let mut st = c
        .prepare(
            "select rowid, id, name, moniker, latest_version, arp_min_version, arp_max_version, hash from packages",
        )
        .map_err(sql)?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<Vec<u8>>>(7)?,
            ))
        })
        .map_err(sql)?;
    let mut by_rowid: HashMap<i64, String> = HashMap::new();
    let mut packages = Vec::new();
    for row in rows {
        let (rowid, id, name, moniker, version, arp_min, arp_max, hash) = row.map_err(sql)?;
        by_rowid.insert(rowid, id.clone());
        packages.push(NewPackage {
            id,
            name,
            moniker: moniker.filter(|m| !m.is_empty()),
            version,
            publisher: None,
            description: None,
            tags: tags.remove(&rowid).unwrap_or_default(),
            commands: commands.remove(&rowid).unwrap_or_default(),
            arp_min_version: arp_min.filter(|s| !s.is_empty()),
            arp_max_version: arp_max.filter(|s| !s.is_empty()),
            winget_hash: hash,
            raw: None,
        });
    }

    let map = |table: &str, key: &str| -> Result<Vec<(String, String)>, IndexError> {
        let mut st = c
            .prepare(&format!("select {key}, package from {table}"))
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(sql)?;
        let mut out = Vec::new();
        for row in rows {
            let (k, pkg) = row.map_err(sql)?;
            if let Some(id) = by_rowid.get(&pkg) {
                out.push((k, id.clone()));
            }
        }
        Ok(out)
    };
    let maps = WingetMaps {
        productcodes: map("productcodes2", "productcode")?,
        upgradecodes: map("upgradecodes2", "upgradecode")?,
        pfns: map("pfns2", "pfn")?,
        norm_names: map("norm_names2", "norm_name")?,
        norm_publishers: map("norm_publishers2", "norm_publisher")?,
    };
    Ok(WingetImport { packages, maps })
}
