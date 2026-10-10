//! Windows user environment store.
//!
//! Values are read and written as raw UTF-16 so `%VAR%` references survive verbatim
//! (no `ExpandEnvironmentStrings`, no implicit expansion on read).
use crate::install::traits::{EffectError, EnvStore, EnvValue};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, RegType};
use winreg::{RegKey, RegValue};

const TEST_KEY_VAR: &str = "WINGETRS_TEST_ENV_KEY";
const USER_ENV_KEY: &str = "Environment";

pub struct RegEnvStore {
    key: String,
    broadcast: bool,
}

impl Default for RegEnvStore {
    fn default() -> Self {
        RegEnvStore::new()
    }
}

impl RegEnvStore {
    /// `HKCU\Environment`, or `HKCU\<WINGETRS_TEST_ENV_KEY>` when that variable is set.
    pub fn new() -> RegEnvStore {
        let test = std::env::var(TEST_KEY_VAR).ok().filter(|s| !s.is_empty());
        RegEnvStore {
            key: RegEnvStore::key_path(test.as_deref()),
            broadcast: RegEnvStore::should_broadcast(test.as_deref()),
        }
    }

    /// Store rooted at an explicit HKCU-relative test key; never broadcasts.
    pub fn for_test_key(key: &str) -> RegEnvStore {
        RegEnvStore {
            key: key.to_string(),
            broadcast: false,
        }
    }

    /// HKCU-relative key used for the given test override.
    pub fn key_path(test_key: Option<&str>) -> String {
        test_key.unwrap_or(USER_ENV_KEY).to_string()
    }

    /// Only the real `HKCU\Environment` notifies other processes.
    pub fn should_broadcast(test_key: Option<&str>) -> bool {
        test_key.is_none()
    }

    pub fn broadcasts(&self) -> bool {
        self.broadcast
    }

    fn open(&self, access: u32) -> Result<Option<RegKey>, EffectError> {
        match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(&self.key, access) {
            Ok(k) => Ok(Some(k)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(err("abrir", &self.key, e)),
        }
    }
}

fn err(what: &str, key: &str, e: std::io::Error) -> EffectError {
    EffectError(format!("falha ao {what} HKCU\\{key}: {e}"))
}

/// Raw registry bytes (UTF-16LE, possibly NUL-terminated) to text, unexpanded.
fn decode(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn encode(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

impl EnvStore for RegEnvStore {
    fn get(&self, name: &str) -> Result<Option<EnvValue>, EffectError> {
        let Some(key) = self.open(KEY_READ)? else {
            return Ok(None);
        };
        let raw = match key.get_raw_value(name) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err("ler", &self.key, e)),
        };
        let expand = match raw.vtype {
            RegType::REG_EXPAND_SZ => true,
            RegType::REG_SZ => false,
            other => {
                return Err(EffectError(format!(
                    "HKCU\\{}\\{name} tem tipo {other:?}, esperado texto",
                    self.key
                )));
            }
        };
        Ok(Some(EnvValue {
            value: decode(&raw.bytes),
            expand,
        }))
    }

    fn set(&self, name: &str, value: &EnvValue) -> Result<(), EffectError> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags(&self.key, KEY_READ | KEY_SET_VALUE)
            .map_err(|e| err("abrir", &self.key, e))?;
        // The existing value's type wins, so a REG_EXPAND_SZ Path never degrades.
        let vtype = match key.get_raw_value(name) {
            Ok(old) if old.vtype == RegType::REG_EXPAND_SZ => RegType::REG_EXPAND_SZ,
            Ok(old) if old.vtype == RegType::REG_SZ => RegType::REG_SZ,
            _ if value.expand || name.eq_ignore_ascii_case("Path") => RegType::REG_EXPAND_SZ,
            _ => RegType::REG_SZ,
        };
        let raw = RegValue {
            bytes: encode(&value.value).into(),
            vtype,
        };
        key.set_raw_value(name, &raw)
            .map_err(|e| err("gravar", &self.key, e))
    }

    fn remove(&self, name: &str) -> Result<(), EffectError> {
        let Some(key) = self.open(KEY_READ | KEY_SET_VALUE)? else {
            return Ok(());
        };
        match key.delete_value(name) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(err("apagar valor em", &self.key, e)),
        }
    }

    fn broadcast(&self) {
        if !self.broadcast {
            return;
        }
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        };
        let area = windows::core::w!("Environment");
        // SAFETY: `area` is a static NUL-terminated wide string; the result pointer is optional.
        let r = unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(area.as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                2000,
                None,
            )
        };
        if r.0 == 0 {
            tracing::warn!("WM_SETTINGCHANGE broadcast timed out or failed");
        }
    }
}
