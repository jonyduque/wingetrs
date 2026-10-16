//! Typed scoop manifest: parses the raw JSON stored in the catalog, resolves the
//! architecture variant and checks whether phase 2a supports the manifest.
use serde::Serialize;
use serde_json::{Map, Value};

use crate::install::archive::ArchiveKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Arch {
    X64,
    X86,
    Arm64,
}

impl Arch {
    /// Key used by scoop manifests.
    pub fn key(&self) -> &'static str {
        match self {
            Arch::X64 => "64bit",
            Arch::X86 => "32bit",
            Arch::Arm64 => "arm64",
        }
    }

    pub fn host() -> Arch {
        match std::env::consts::ARCH {
            "aarch64" => Arch::Arm64,
            "x86" => Arch::X86,
            _ => Arch::X64,
        }
    }

    /// Variants this host can run, most preferred first.
    fn preference(self) -> &'static [Arch] {
        match self {
            Arch::X64 => &[Arch::X64, Arch::X86],
            Arch::X86 => &[Arch::X86],
            Arch::Arm64 => &[Arch::Arm64, Arch::X64, Arch::X86],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo {
    Sha256,
    Sha1,
    Sha512,
    Md5,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashSpec {
    pub algo: HashAlgo,
    pub hex: String,
}

impl HashSpec {
    /// Parses `hex` (sha256) or `sha1:`/`sha512:`/`md5:`/`sha256:` prefixed hex.
    pub fn parse(s: &str) -> Option<HashSpec> {
        let s = s.trim();
        let (algo, hex) = match s.split_once(':') {
            Some((p, h)) => {
                let algo = match p.to_ascii_lowercase().as_str() {
                    "sha256" => HashAlgo::Sha256,
                    "sha1" => HashAlgo::Sha1,
                    "sha512" => HashAlgo::Sha512,
                    "md5" => HashAlgo::Md5,
                    _ => return None,
                };
                (algo, h)
            }
            None => (HashAlgo::Sha256, s),
        };
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(HashSpec {
            algo,
            hex: hex.to_ascii_lowercase(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub file_name: String,
    pub hash: HashSpec,
    pub extract_dir: Option<String>,
    pub extract_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinEntry {
    pub path: String,
    pub alias: Option<String>,
    pub args: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutEntry {
    pub target: String,
    pub name: String,
    pub args: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistEntry {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub version: String,
    pub arch: Option<Arch>,
    pub sources: Vec<Source>,
    pub bin: Vec<BinEntry>,
    pub shortcuts: Vec<ShortcutEntry>,
    pub persist: Vec<PersistEntry>,
    pub env_add_path: Vec<String>,
    pub env_set: Vec<(String, String)>,
    pub depends: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ManifestError {
    #[error("manifesto inválido: {0}")]
    Invalid(String),
    #[error("não suportado: {reason}")]
    Unsupported { reason: String },
    #[error("sem variante para a arquitetura desta máquina")]
    NoArch,
}

type Obj = Map<String, Value>;

fn invalid<T>(msg: impl Into<String>) -> Result<T, ManifestError> {
    Err(ManifestError::Invalid(msg.into()))
}

fn unsupported<T>(reason: impl Into<String>) -> Result<T, ManifestError> {
    Err(ManifestError::Unsupported {
        reason: reason.into(),
    })
}

/// Field of the architecture variant if present, else of the root.
fn field<'a>(root: &'a Obj, variant: Option<&'a Obj>, key: &str) -> Option<&'a Value> {
    variant
        .and_then(|v| v.get(key))
        .or_else(|| root.get(key))
        .filter(|v| !v.is_null())
}

fn str_of(v: &Value, what: &str) -> Result<String, ManifestError> {
    match v.as_str() {
        Some(s) => Ok(s.to_string()),
        None => invalid(format!("{what} deve ser texto")),
    }
}

/// string | list of string.
fn str_list(v: Option<&Value>, what: &str) -> Result<Vec<String>, ManifestError> {
    match v {
        None => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(a)) => a.iter().map(|x| str_of(x, what)).collect(),
        Some(_) => invalid(format!("{what} deve ser texto ou lista")),
    }
}

/// Items that are either a string or an array of strings with `1..=max` elements.
fn tuple_items(
    v: Option<&Value>,
    what: &str,
    max: usize,
) -> Result<Vec<Vec<String>>, ManifestError> {
    let items: Vec<&Value> = match v {
        None => return Ok(Vec::new()),
        Some(Value::Array(a)) => a.iter().collect(),
        Some(x @ Value::String(_)) => vec![x],
        Some(_) => return invalid(format!("{what} deve ser texto ou lista")),
    };
    let mut out = Vec::new();
    for item in items {
        match item {
            Value::String(s) => out.push(vec![s.clone()]),
            Value::Array(a) => {
                if a.is_empty() || a.len() > max {
                    return invalid(format!("{what}: item com {} elementos", a.len()));
                }
                out.push(
                    a.iter()
                        .map(|x| str_of(x, what))
                        .collect::<Result<_, _>>()?,
                );
            }
            _ => return invalid(format!("{what}: item inesperado")),
        }
    }
    Ok(out)
}

const SCRIPT_KEYS: [&str; 7] = [
    "pre_install",
    "post_install",
    "pre_uninstall",
    "post_uninstall",
    "installer",
    "uninstaller",
    "psmodule",
];

fn check_keys(root: &Obj) -> Result<(), ManifestError> {
    for k in SCRIPT_KEYS {
        if root.get(k).is_some_and(|v| !v.is_null()) {
            return unsupported(format!(
                "usa {k} (PowerShell/instalador próprio): suportado na fase 2c"
            ));
        }
    }
    if root
        .get("innosetup")
        .is_some_and(|v| !v.is_null() && *v != Value::Bool(false))
    {
        return unsupported("usa innosetup: suportado na fase 2c");
    }
    Ok(())
}

/// Returns the request URL (without fragment) and the file name to save as (`#/name` renames).
fn split_fragment(url: &str) -> (String, String) {
    let (base, frag) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let rename = frag
        .and_then(|f| f.strip_prefix('/'))
        .filter(|n| !n.is_empty());
    let name = match rename {
        Some(n) => n.to_string(),
        None => {
            let path = base.split('?').next().unwrap_or(base);
            path.rsplit('/').next().unwrap_or(path).to_string()
        }
    };
    (base.to_string(), name)
}

fn build_sources(root: &Obj, variant: Option<&Obj>) -> Result<Vec<Source>, ManifestError> {
    let urls = str_list(field(root, variant, "url"), "url")?;
    let hashes = str_list(field(root, variant, "hash"), "hash")?;
    let dirs = str_list(field(root, variant, "extract_dir"), "extract_dir")?;
    let tos = str_list(field(root, variant, "extract_to"), "extract_to")?;
    // More hashes than URLs is malformed (checked first); fewer means some URL has no hash (Unsupported).
    if hashes.len() > urls.len() {
        return invalid(format!("{} hashes para {} URLs", hashes.len(), urls.len()));
    }
    let mut sources = Vec::new();
    for (i, raw_url) in urls.iter().enumerate() {
        let (url, file_name) = split_fragment(raw_url);
        match ArchiveKind::of(&file_name) {
            Err(ext) => return unsupported(format!("download {ext}: formato não suportado")),
            Ok(k) if !k.supported() => {
                return unsupported(format!("download {}: suportado na fase 2b", k.ext()));
            }
            Ok(_) => {}
        }
        let Some(raw_hash) = hashes.get(i) else {
            return unsupported("sem hash para verificar o download");
        };
        let Some(hash) = HashSpec::parse(raw_hash) else {
            return invalid(format!("hash ilegível: {raw_hash}"));
        };
        let pick = |v: &[String]| v.get(i).filter(|s| !s.is_empty()).cloned();
        sources.push(Source {
            url,
            file_name,
            hash,
            extract_dir: pick(&dirs),
            extract_to: pick(&tos),
        });
    }
    Ok(sources)
}

/// Resolves the manifest for `host`; the first unsupported feature found becomes the error.
pub fn resolve(raw_json: &str, host: Arch) -> Result<Resolved, ManifestError> {
    let value: Value =
        serde_json::from_str(raw_json).map_err(|e| ManifestError::Invalid(e.to_string()))?;
    let Some(root) = value.as_object() else {
        return invalid("a raiz deve ser um objeto");
    };
    let version = match root.get("version") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => return invalid("version ausente ou não é texto"),
    };
    check_keys(root)?;

    let (arch, variant) = match root.get("architecture") {
        None | Some(Value::Null) => (None, None),
        Some(Value::Object(archs)) => {
            let found = host
                .preference()
                .iter()
                .find_map(|a| match archs.get(a.key()) {
                    Some(Value::Object(o)) => Some((*a, o)),
                    _ => None,
                });
            match found {
                Some((a, o)) => (Some(a), Some(o)),
                None => return Err(ManifestError::NoArch),
            }
        }
        Some(_) => return invalid("architecture deve ser um objeto"),
    };
    if let Some(v) = variant {
        check_keys(v)?;
    }

    let sources = build_sources(root, variant)?;
    let bin = tuple_items(field(root, variant, "bin"), "bin", usize::MAX)?
        .into_iter()
        .map(|t| {
            let mut it = t.into_iter();
            let path = it.next().unwrap_or_default();
            let alias = it.next();
            // scoop interpolates the remaining elements joined by `$OFS` (one space).
            let rest: Vec<String> = it.collect();
            let args = (!rest.is_empty()).then(|| rest.join(" "));
            BinEntry { path, alias, args }
        })
        .collect();
    let mut shortcuts = Vec::new();
    for t in tuple_items(field(root, variant, "shortcuts"), "shortcuts", 4)? {
        let mut it = t.into_iter();
        let (Some(target), Some(name)) = (it.next(), it.next()) else {
            return invalid("shortcuts: item precisa de alvo e nome");
        };
        shortcuts.push(ShortcutEntry {
            target,
            name,
            args: it.next(),
            icon: it.next(),
        });
    }
    let persist = tuple_items(root.get("persist"), "persist", 2)?
        .into_iter()
        .map(|t| {
            let mut it = t.into_iter();
            let source = it.next().unwrap_or_default();
            let target = it
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| source.clone());
            PersistEntry { source, target }
        })
        .collect();
    let env_set = match field(root, variant, "env_set") {
        None => Vec::new(),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| Ok((k.clone(), str_of(v, "env_set")?)))
            .collect::<Result<_, ManifestError>>()?,
        Some(_) => return invalid("env_set deve ser um objeto"),
    };
    Ok(Resolved {
        version,
        arch,
        sources,
        bin,
        shortcuts,
        persist,
        env_add_path: str_list(field(root, variant, "env_add_path"), "env_add_path")?,
        env_set,
        depends: str_list(root.get("depends"), "depends")?,
    })
}
