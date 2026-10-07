//! Plain-text rendering for CLI commands (no colors).
use crate::core::{PackageDetails, PackageEntry};
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
        Outcome::Failed(m) => format!("{}: falhou: {m}", r.source),
    }
}
