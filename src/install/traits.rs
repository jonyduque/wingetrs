//! Traits through which every side effect on global user state passes, plus in-memory fakes.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvValue {
    pub value: String,
    /// `REG_EXPAND_SZ` when true.
    pub expand: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EffectError(pub String);

pub trait EnvStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<EnvValue>, EffectError>;
    fn set(&self, name: &str, value: &EnvValue) -> Result<(), EffectError>;
    fn remove(&self, name: &str) -> Result<(), EffectError>;
    fn broadcast(&self);
}

pub trait Shortcuts: Send + Sync {
    /// `link` is the full .lnk path (already under the shortcut root).
    fn create(
        &self,
        link: &Path,
        target: &Path,
        args: Option<&str>,
        icon: Option<&Path>,
    ) -> Result<(), EffectError>;
    fn remove(&self, link: &Path) -> Result<(), EffectError>;
    /// Start Menu\Programs\wingetrs Apps (or test dir).
    fn root(&self) -> PathBuf;
}

pub trait Trash: Send + Sync {
    fn recycle(&self, path: &Path) -> Result<(), EffectError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegHit {
    pub key: String,
    pub value_name: Option<String>,
    pub data: String,
}

pub trait RegistryScan: Send + Sync {
    /// Values under each HKCU-relative root whose string data contains `needle` (case-insensitive).
    fn values_containing(&self, roots: &[&str], needle: &str) -> Result<Vec<RegHit>, EffectError>;
    /// Direct subkeys of `parent` (HKCU-relative) whose name equals one of `names` (case-insensitive).
    fn subkeys_named(&self, parent: &str, names: &[&str]) -> Result<Vec<String>, EffectError>;
    fn export(&self, key: &str, file: &Path) -> Result<(), EffectError>;
    fn delete_value(&self, key: &str, value_name: &str) -> Result<(), EffectError>;
    fn delete_key(&self, key: &str) -> Result<(), EffectError>;
}

pub struct Effects<'a> {
    pub env: &'a dyn EnvStore,
    pub shortcuts: &'a dyn Shortcuts,
    pub trash: &'a dyn Trash,
    pub registry: &'a dyn RegistryScan,
}

pub mod fake {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, MutexGuard};

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[derive(Default)]
    pub struct FakeEnv {
        vars: Mutex<BTreeMap<String, EnvValue>>,
        broadcasts: Mutex<u32>,
    }
    impl FakeEnv {
        pub fn new() -> FakeEnv {
            FakeEnv::default()
        }
        /// Variables plus broadcast count, comparable between states.
        pub fn dump(&self) -> (BTreeMap<String, EnvValue>, u32) {
            (lock(&self.vars).clone(), *lock(&self.broadcasts))
        }
        /// Raw value of `name`, if set.
        pub fn get_raw(&self, name: &str) -> Option<String> {
            lock(&self.vars).get(name).map(|v| v.value.clone())
        }
    }
    impl EnvStore for FakeEnv {
        fn get(&self, name: &str) -> Result<Option<EnvValue>, EffectError> {
            Ok(lock(&self.vars).get(name).cloned())
        }
        fn set(&self, name: &str, value: &EnvValue) -> Result<(), EffectError> {
            lock(&self.vars).insert(name.to_string(), value.clone());
            Ok(())
        }
        fn remove(&self, name: &str) -> Result<(), EffectError> {
            lock(&self.vars).remove(name);
            Ok(())
        }
        fn broadcast(&self) {
            *lock(&self.broadcasts) += 1;
        }
    }

    /// Shortcut record: (target, args, icon).
    pub type ShortcutRec = (PathBuf, Option<String>, Option<PathBuf>);

    pub struct FakeShortcuts {
        root: PathBuf,
        links: Mutex<BTreeMap<PathBuf, ShortcutRec>>,
    }
    impl FakeShortcuts {
        pub fn new(root: impl Into<PathBuf>) -> FakeShortcuts {
            FakeShortcuts {
                root: root.into(),
                links: Mutex::new(BTreeMap::new()),
            }
        }
        pub fn dump(&self) -> BTreeMap<PathBuf, ShortcutRec> {
            lock(&self.links).clone()
        }
    }
    impl Shortcuts for FakeShortcuts {
        fn create(
            &self,
            link: &Path,
            target: &Path,
            args: Option<&str>,
            icon: Option<&Path>,
        ) -> Result<(), EffectError> {
            lock(&self.links).insert(
                link.to_path_buf(),
                (
                    target.to_path_buf(),
                    args.map(str::to_string),
                    icon.map(Path::to_path_buf),
                ),
            );
            Ok(())
        }
        fn remove(&self, link: &Path) -> Result<(), EffectError> {
            lock(&self.links).remove(link);
            Ok(())
        }
        fn root(&self) -> PathBuf {
            self.root.clone()
        }
    }

    #[derive(Default)]
    pub struct FakeTrash {
        recycled: Mutex<Vec<PathBuf>>,
    }
    impl FakeTrash {
        pub fn new() -> FakeTrash {
            FakeTrash::default()
        }
        pub fn dump(&self) -> Vec<PathBuf> {
            lock(&self.recycled).clone()
        }
    }
    impl Trash for FakeTrash {
        fn recycle(&self, path: &Path) -> Result<(), EffectError> {
            lock(&self.recycled).push(path.to_path_buf());
            Ok(())
        }
    }

    /// key -> (value name, `None` = default value) -> data
    pub type RegState = BTreeMap<String, BTreeMap<Option<String>, String>>;

    #[derive(Default)]
    pub struct FakeRegistry {
        keys: Mutex<RegState>,
        exports: Mutex<Vec<(String, PathBuf)>>,
    }
    impl FakeRegistry {
        pub fn new() -> FakeRegistry {
            FakeRegistry::default()
        }
        pub fn insert(&self, key: &str, value_name: Option<&str>, data: &str) {
            lock(&self.keys)
                .entry(key.to_string())
                .or_default()
                .insert(value_name.map(str::to_string), data.to_string());
        }
        pub fn dump(&self) -> (RegState, Vec<(String, PathBuf)>) {
            (lock(&self.keys).clone(), lock(&self.exports).clone())
        }
    }
    fn under(key: &str, root: &str) -> bool {
        let (k, r) = (key.to_lowercase(), root.to_lowercase());
        k == r || k.starts_with(&format!("{r}\\"))
    }
    impl RegistryScan for FakeRegistry {
        fn values_containing(
            &self,
            roots: &[&str],
            needle: &str,
        ) -> Result<Vec<RegHit>, EffectError> {
            let needle = needle.to_lowercase();
            let mut hits = Vec::new();
            for (key, vals) in lock(&self.keys).iter() {
                if !roots.iter().any(|r| under(key, r)) {
                    continue;
                }
                for (name, data) in vals {
                    if data.to_lowercase().contains(&needle) {
                        hits.push(RegHit {
                            key: key.clone(),
                            value_name: name.clone(),
                            data: data.clone(),
                        });
                    }
                }
            }
            Ok(hits)
        }
        fn subkeys_named(&self, parent: &str, names: &[&str]) -> Result<Vec<String>, EffectError> {
            let prefix = format!("{}\\", parent.to_lowercase());
            let mut out = Vec::new();
            for key in lock(&self.keys).keys() {
                let lower = key.to_lowercase();
                if let Some(rest) = lower.strip_prefix(&prefix)
                    && !rest.contains('\\')
                    && names.iter().any(|n| n.to_lowercase() == rest)
                {
                    out.push(key.clone());
                }
            }
            Ok(out)
        }
        fn export(&self, key: &str, file: &Path) -> Result<(), EffectError> {
            lock(&self.exports).push((key.to_string(), file.to_path_buf()));
            Ok(())
        }
        fn delete_value(&self, key: &str, value_name: &str) -> Result<(), EffectError> {
            let mut keys = lock(&self.keys);
            if let Some(vals) = keys.get_mut(key) {
                vals.remove(&Some(value_name.to_string()));
            }
            Ok(())
        }
        fn delete_key(&self, key: &str) -> Result<(), EffectError> {
            lock(&self.keys).retain(|k, _| !under(k, key));
            Ok(())
        }
    }
}
