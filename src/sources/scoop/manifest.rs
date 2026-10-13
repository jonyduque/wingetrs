//! Scoop manifest parsing.
use super::ScoopError;
use crate::catalog::NewPackage;
use crate::core::{InstallerSummary, PackageDetails, SourceKind};
use serde_json::Value;

const ARCHS: [(&str, &str); 3] = [("64bit", "x64"), ("32bit", "x86"), ("arm64", "arm64")];

fn strip_bom(b: &[u8]) -> &[u8] {
    b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(b)
}

fn stem(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match file.rfind('.') {
        Some(i) if i > 0 => file[..i].to_lowercase(),
        _ => file.to_lowercase(),
    }
}

fn push_bin(bin: &Value, out: &mut Vec<String>) {
    let mut add = |c: String| {
        if !c.is_empty() && !out.contains(&c) {
            out.push(c);
        }
    };
    match bin {
        Value::String(s) => add(stem(s)),
        Value::Array(items) => {
            for it in items {
                match it {
                    Value::String(s) => add(stem(s)),
                    Value::Array(parts) => {
                        let alias = parts.get(1).and_then(Value::as_str);
                        let path = parts.first().and_then(Value::as_str);
                        if let Some(a) = alias {
                            add(a.to_lowercase());
                        } else if let Some(p) = path {
                            add(stem(p));
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn first_url(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.first().and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

fn parse_value(json: &[u8]) -> Result<Value, ScoopError> {
    serde_json::from_slice(strip_bom(json)).map_err(|e| ScoopError::Json(e.to_string()))
}

/// Parses one scoop manifest into a catalog package; `raw` keeps the original JSON.
pub fn parse(app: &str, json: &[u8]) -> Result<NewPackage, ScoopError> {
    let json = strip_bom(json);
    let v = parse_value(json)?;
    let version = v
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| ScoopError::Json(format!("{app}: sem version")))?
        .to_string();
    let mut commands = Vec::new();
    if let Some(b) = v.get("bin") {
        push_bin(b, &mut commands);
    }
    for (key, _) in ARCHS {
        if let Some(b) = v
            .get("architecture")
            .and_then(|a| a.get(key))
            .and_then(|a| a.get("bin"))
        {
            push_bin(b, &mut commands);
        }
    }
    Ok(NewPackage {
        id: app.to_string(),
        name: app.to_string(),
        version,
        description: v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        publisher: None,
        tags: Vec::new(),
        commands,
        raw: Some(String::from_utf8_lossy(json).into_owned()),
        ..NewPackage::default()
    })
}

/// Builds package details from the stored manifest JSON.
pub fn details_from_raw(app: &str, raw: &str) -> Result<PackageDetails, ScoopError> {
    let v = parse_value(raw.as_bytes())?;
    let version = v
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let license = match v.get("license") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(o) => o
            .get("identifier")
            .and_then(Value::as_str)
            .map(str::to_string),
        None => None,
    };
    let mut installers = Vec::new();
    if let Some(arch) = v.get("architecture").and_then(Value::as_object) {
        for (key, name) in ARCHS {
            if let Some(url) = arch.get(key).and_then(|a| a.get("url")).and_then(first_url) {
                installers.push(InstallerSummary {
                    architecture: name.to_string(),
                    installer_type: "scoop".to_string(),
                    scope: Some("user".to_string()),
                    url,
                });
            }
        }
    }
    if installers.is_empty()
        && let Some(url) = v.get("url").and_then(first_url)
    {
        installers.push(InstallerSummary {
            architecture: "neutral".to_string(),
            installer_type: "scoop".to_string(),
            scope: Some("user".to_string()),
            url,
        });
    }
    Ok(PackageDetails {
        source: SourceKind::Scoop,
        id: app.to_string(),
        name: app.to_string(),
        version: version.clone(),
        publisher: None,
        description: v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        homepage: v
            .get("homepage")
            .and_then(Value::as_str)
            .map(str::to_string),
        license,
        tags: Vec::new(),
        installers,
        available_versions: vec![version],
        icon: None,
    })
}
