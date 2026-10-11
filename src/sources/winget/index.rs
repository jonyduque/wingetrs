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

/// Fills `buf` unless the reader ends first; returns the bytes read.
fn read_block<R: Read>(r: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// Block hashes and the optional whole-file hash of Public\index.db, from AppxBlockMap.xml.
fn read_blockmap<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> Result<(Vec<String>, Option<String>), IndexError> {
    let map = read_entry(zip, BLOCKMAP_ENTRY)?;
    let xml = std::str::from_utf8(&map).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let doc = roxmltree::Document::parse(xml).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let file = doc
        .descendants()
        .find(|n| n.has_tag_name("File") && n.attribute("Name") == Some(INDEX_BLOCKMAP_NAME))
        .ok_or_else(|| IndexError::BlockMap("entrada Public\\index.db ausente".into()))?;
    let hashes = file
        .children()
        .filter(|n| n.has_tag_name("Block"))
        .filter_map(|n| n.attribute("Hash"))
        .map(str::to_owned)
        .collect();
    // b4:FileHash is a child element of <File> carrying the base64 sha256 in `Hash`.
    let file_hash = file
        .children()
        .find(|n| n.has_tag_name("FileHash"))
        .and_then(|n| n.attribute("Hash"))
        .map(str::to_owned);
    Ok((hashes, file_hash))
}

/// Cap for the up-front buffer: the declared size comes from the download.
const PREALLOC_MAX: usize = 64 << 20;

/// Returns the validated bytes of Public/index.db.
/// Inflate, per-block hashes and the whole-file hash run as a pipeline (this thread plus two
/// scoped workers); serially they cost ~235 ms under load on a CPU without SHA-NI.
/// Errors keep the serial precedence: Zip, missing index, index read error, missing/invalid
/// blockmap, block count, first bad block, file hash.
pub fn validate_msix(msix: &[u8]) -> Result<Vec<u8>, IndexError> {
    use std::sync::{Arc, mpsc};
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(msix))
        .map_err(|e| IndexError::Zip(e.to_string()))?;
    if zip.index_for_name(INDEX_ENTRY).is_none() {
        return Err(IndexError::MissingEntry(INDEX_ENTRY));
    }
    let (hashes, file_hash) = match read_blockmap(&mut zip) {
        Ok(m) => m,
        Err(e) => {
            // index.db used to be inflated first, so its read error wins.
            read_entry(&mut zip, INDEX_ENTRY)?;
            return Err(e);
        }
    };
    let mut entry = zip
        .by_name(INDEX_ENTRY)
        .map_err(|e| IndexError::Zip(e.to_string()))?;
    let cap = usize::try_from(entry.size()).map_or(PREALLOC_MAX, |n| n.min(PREALLOC_MAX));
    let (hashes, file_hash) = (&hashes, file_hash.as_deref());
    std::thread::scope(|s| {
        let (block_tx, block_rx) = mpsc::sync_channel::<Arc<Vec<u8>>>(32);
        let (file_tx, file_rx) = mpsc::sync_channel::<Arc<Vec<u8>>>(32);
        let spawn_err = |e: std::io::Error| IndexError::Io(e.to_string());
        let blocks = std::thread::Builder::new().spawn_scoped(s, move || {
            let mut first_bad = None;
            for (i, block) in block_rx.into_iter().enumerate() {
                if first_bad.is_none()
                    && hashes
                        .get(i)
                        .is_some_and(|h| STANDARD.encode(Sha256::digest(&block[..])) != *h)
                {
                    first_bad = Some(i);
                }
            }
            first_bad
        });
        // block_rx moved into the failed closure is dropped with it; nothing to join.
        let blocks = blocks.map_err(spawn_err)?;
        let whole = std::thread::Builder::new().spawn_scoped(s, move || {
            let mut h = Sha256::new();
            for block in file_rx {
                h.update(&block[..]);
            }
            file_hash.is_none_or(|expected| STANDARD.encode(h.finalize()) == expected)
        });
        let whole = match whole {
            Ok(w) => w,
            Err(e) => {
                // Close the block channel so the first worker ends, and join it before returning.
                drop(block_tx);
                blocks
                    .join()
                    .unwrap_or_else(|p| std::panic::resume_unwind(p));
                return Err(spawn_err(e));
            }
        };
        let mut db = Vec::with_capacity(cap);
        let produced = (|| -> Result<(), IndexError> {
            loop {
                let mut buf = vec![0u8; BLOCK_SIZE];
                let n =
                    read_block(&mut entry, &mut buf).map_err(|e| IndexError::Zip(e.to_string()))?;
                if n == 0 {
                    return Ok(());
                }
                buf.truncate(n);
                db.extend_from_slice(&buf);
                let buf = Arc::new(buf);
                // A closed receiver means that worker panicked; the join below re-raises it.
                if block_tx.send(Arc::clone(&buf)).is_err() || file_tx.send(buf).is_err() {
                    return Ok(());
                }
            }
        })();
        // Closing both channels ends the workers' loops; join both before any `?`.
        drop(block_tx);
        drop(file_tx);
        let first_bad = blocks
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p));
        let file_ok = whole
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p));
        produced?;
        if hashes.len() != db.len().div_ceil(BLOCK_SIZE) {
            return Err(IndexError::BlockMap(format!(
                "esperados {} blocos, encontrados {}",
                db.len().div_ceil(BLOCK_SIZE),
                hashes.len()
            )));
        }
        if let Some(block) = first_bad {
            return Err(IndexError::Integrity { block });
        }
        if !file_ok {
            return Err(IndexError::FileHash);
        }
        Ok(db)
    })
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
