//! User-chosen catalog matches: `correlations.json` maps an installed app (`<source>/<key>`) to a
//! catalog package, or to "not in the catalog". Consulted by `correlate`, the only place that
//! decides a correlation.
use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;

use crate::cache::DataDir;
use crate::installed::InstalledApp;
use crate::installed::correlate::origin;

static OVERRIDES_TMP_SEQ: AtomicU64 = AtomicU64::new(0);
/// Serializes the load-modify-rename of `correlations.json` within the process.
static OVERRIDES_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum Choice {
    Package { source: String, package_id: String },
    NotInCatalog,
}

fn overrides_path(data: &DataDir) -> std::path::PathBuf {
    data.root().join("correlations.json")
}

/// Stable identity of an installed app: `<source>/<key>` (`arp/{GUID}`, `msix/<family>`, `scoop/<name>`).
pub fn app_key(app: &InstalledApp) -> String {
    format!("{}/{}", origin(&app.source), app.key)
}

/// Missing or corrupt file yields an empty map.
pub fn load(data: &DataDir) -> BTreeMap<String, Choice> {
    let Ok(bytes) = std::fs::read(overrides_path(data)) else {
        return BTreeMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Stores `choice` for `app_key`, or removes the entry when `None`.
pub fn set(data: &DataDir, app_key: &str, choice: Option<Choice>) -> std::io::Result<()> {
    // Poison is harmless: the guarded data is `()` and the file is always rewritten whole by rename.
    let _guard = OVERRIDES_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut map = load(data);
    match choice {
        Some(c) => {
            map.insert(app_key.to_owned(), c);
        }
        None => {
            map.remove(app_key);
        }
    }
    crate::fs_retry::write_json_atomic(&overrides_path(data), &map, &OVERRIDES_TMP_SEQ)
}
