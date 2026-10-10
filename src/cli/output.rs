//! Plain-text rendering for CLI commands (no colors).
use crate::core::{PackageDetails, PackageEntry};
use crate::installed::correlate::{Correlation, ListRow, UpdateState};
use crate::update::{Outcome, SourceReport};

fn pad(s: &str, width: usize) -> String {
    let n = s.chars().count();
    format!("{s}{}", " ".repeat(width.saturating_sub(n)))
}

/// `Nome  Id  Versão  Fonte` with aligned columns.
pub fn search_table(rows: &[PackageEntry]) -> String {
    let header = ["Nome", "Id", "Versão", "Fonte"];
    let cells: Vec<[&str; 4]> = rows
        .iter()
        .map(|e| {
            [
                e.name.as_str(),
                e.id.as_str(),
                e.version.as_str(),
                e.source_name.as_str(),
            ]
        })
        .collect();
    let mut widths = header.map(|h| h.chars().count());
    for r in &cells {
        for (w, c) in widths.iter_mut().zip(r) {
            *w = (*w).max(c.chars().count());
        }
    }
    let line = |r: &[&str; 4]| {
        let parts: Vec<String> = r.iter().zip(widths).map(|(c, w)| pad(c, w)).collect();
        parts.join("  ").trim_end().to_string()
    };
    let mut out = String::new();
    out.push_str(&line(&header));
    out.push('\n');
    for r in &cells {
        out.push_str(&line(r));
        out.push('\n');
    }
    out
}

pub fn details_text(d: &PackageDetails) -> String {
    let mut out = String::new();
    let mut field = |k: &str, v: &str| {
        out.push_str(&format!("{k}: {v}\n"));
    };
    field("Nome", &d.name);
    field("Id", &d.id);
    field("Versão", &d.version);
    field("Fonte", d.source.as_str());
    if let Some(v) = &d.publisher {
        field("Publicador", v);
    }
    if let Some(v) = &d.description {
        field("Descrição", v);
    }
    if let Some(v) = &d.homepage {
        field("Página", v);
    }
    if let Some(v) = &d.license {
        field("Licença", v);
    }
    if !d.tags.is_empty() {
        field("Tags", &d.tags.join(", "));
    }
    if !d.available_versions.is_empty() {
        field("Versões", &d.available_versions.join(", "));
    }
    for i in &d.installers {
        let scope = i.scope.as_deref().unwrap_or("-");
        out.push_str(&format!(
            "Instalador: {} {} {} {}\n",
            i.architecture, i.installer_type, scope, i.url
        ));
    }
    out
}

pub fn report_line(r: &SourceReport) -> String {
    match &r.outcome {
        Outcome::Fresh => format!("{}: em dia", r.source),
        Outcome::Unchanged => format!("{}: sem mudanças", r.source),
        Outcome::Updated { packages, skipped } => {
            format!("{}: {packages} pacotes ({skipped} ignorados)", r.source)
        }
        Outcome::Failed { message: m, .. } => format!("{}: falhou: {m}", r.source),
    }
}

fn aligned<const N: usize>(header: [&str; N], cells: &[[String; N]]) -> String {
    let mut widths = header.map(|h| h.chars().count());
    for r in cells {
        for (w, c) in widths.iter_mut().zip(r) {
            *w = (*w).max(c.chars().count());
        }
    }
    let line = |r: &[&str]| {
        let parts: Vec<String> = r.iter().zip(widths).map(|(c, w)| pad(c, w)).collect();
        parts.join("  ").trim_end().to_string()
    };
    let mut out = line(&header);
    out.push('\n');
    for r in cells {
        let refs: Vec<&str> = r.iter().map(String::as_str).collect();
        out.push_str(&line(&refs));
        out.push('\n');
    }
    out
}

/// `Nome  Id  Instalada  Disponível  Fonte  Origem` for correlated rows, followed by the
/// ambiguous and unmatched sections unless `upgradable_only`. A broken wingetrs install
/// shows "quebrado" instead of its version.
pub fn list_text(rows: &[ListRow], upgradable_only: bool) -> String {
    let mut cells = Vec::new();
    let mut ambiguous = Vec::new();
    let mut unmatched = Vec::new();
    for r in rows {
        let installed = match r.state {
            UpdateState::Broken => "quebrado".to_string(),
            _ => r.installed_version.clone().unwrap_or_default(),
        };
        match &r.correlation {
            Correlation::Matched {
                source, package_id, ..
            } => {
                let available = match &r.state {
                    UpdateState::Available { latest } => latest.clone(),
                    _ => String::new(),
                };
                if upgradable_only && available.is_empty() {
                    continue;
                }
                cells.push([
                    r.name.clone(),
                    package_id.clone(),
                    installed,
                    available,
                    source.clone(),
                    r.origin.clone(),
                ]);
            }
            Correlation::Ambiguous { candidates, .. } => {
                ambiguous.push(format!(
                    "{} ({installed}, {}): {}",
                    r.name,
                    r.origin,
                    candidates.join(", ")
                ));
            }
            Correlation::Unmatched => {
                unmatched.push(format!("{} ({installed}, {})", r.name, r.origin));
            }
        }
    }
    let mut out = aligned(
        ["Nome", "Id", "Instalada", "Disponível", "Fonte", "Origem"],
        &cells,
    );
    if !upgradable_only {
        for (title, items) in [
            ("Ambíguos", &ambiguous),
            ("Não correlacionados", &unmatched),
        ] {
            if items.is_empty() {
                continue;
            }
            out.push_str(&format!("\n{title} ({}):\n", items.len()));
            for i in items {
                out.push_str(&format!("  {i}\n"));
            }
        }
        out.push_str(&format!(
            "\n{} instalados: {} correlacionados, {} ambíguos, {} não correlacionados\n",
            rows.len(),
            cells.len(),
            ambiguous.len(),
            unmatched.len()
        ));
    }
    out
}
