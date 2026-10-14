//! Pinned apps: `pins.json` holds the set of `<origin>/<name>` keys whose version is frozen.
//! The only place that decides "is this app pinned?" is [`is_pinned`].
use std::collections::BTreeSet;
use std::sync::atomic::AtomicU64;

use crate::cache::DataDir;

static PINS_TMP_SEQ: AtomicU64 = AtomicU64::new(0);
/// Serializes the load-modify-rename of `pins.json` within the process.
static PINS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn pins_path(data: &DataDir) -> std::path::PathBuf {
    data.root().join("pins.json")
}

/// Key of an app in the pin set: `<origin>/<name>` (the CLI uses origin `wingetrs`).
pub fn key(origin: &str, name: &str) -> String {
    format!("{origin}/{name}")
}

/// Whether `<origin>/<name>` is in `pins`.
pub fn is_pinned(pins: &BTreeSet<String>, origin: &str, name: &str) -> bool {
    pins.contains(&key(origin, name))
}

/// Missing or corrupt file yields an empty set.
pub fn load(data: &DataDir) -> BTreeSet<String> {
    let Ok(bytes) = std::fs::read(pins_path(data)) else {
        return BTreeSet::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Pins or unpins `key`, rewriting the file whole by temporary + rename.
pub fn set(data: &DataDir, key: &str, pinned: bool) -> std::io::Result<()> {
    // Poison is harmless: the guarded data is `()` and the file is always rewritten whole by rename.
    let _guard = PINS_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut pins = load(data);
    if pinned {
        pins.insert(key.to_owned());
    } else {
        pins.remove(key);
    }
    crate::fs_retry::write_json_atomic(&pins_path(data), &pins, &PINS_TMP_SEQ)
}
