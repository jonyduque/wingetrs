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

/// `main/x` for a catalog source name (`scoop:main`) and id.
pub fn bucket_id(source_name: &str, id: &str) -> String {
    let bucket = source_name.strip_prefix("scoop:").unwrap_or(source_name);
    format!("{bucket}/{id}")
}

/// Error text for an ambiguous target: one `bucket/id` per line.
pub fn ambiguous_text(target: &str, candidates: &[String]) -> String {
    let mut s = format!("'{target}' existe em mais de um lugar; use bucket/id:");
    for c in candidates {
        s.push_str("\n  ");
        s.push_str(c);
    }
    s
}

/// Error text for a target that is not in the catalog.
pub fn not_found_text(query: &str, suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        format!("nenhum pacote encontrado para '{query}'")
    } else {
        format!(
            "nenhum pacote encontrado para '{query}'; você quis dizer: {}?",
            suggestions.join(", ")
        )
    }
}

fn artifact_kind(k: crate::install::state::ArtifactKind) -> &'static str {
    use crate::install::state::ArtifactKind as K;
    match k {
        K::Shim => "atalho de linha de comando",
        K::Shortcut => "atalho do Menu Iniciar",
        K::Persist => "dados persistentes",
        K::Path => "entrada no PATH",
        K::Env => "variável de ambiente",
    }
}

/// What `uninstall` is about to do, one numbered line per action.
pub fn uninstall_plan_text(
    plan: &crate::install::plan::UninstallPlan,
    layout: &crate::install::layout::Layout,
) -> String {
    let mut s = format!("Desinstalar {} {}\n", plan.app, plan.version);
    let mut n = 1;
    let mut line = |text: String| {
        s.push_str(&format!("{n:>3}. {text}\n"));
        n += 1;
    };
    line(format!("Apagar {}", layout.app_dir(&plan.app).display()));
    for a in &plan.artifacts {
        let text = match a.kind {
            crate::install::state::ArtifactKind::Persist if plan.purge => {
                format!("Apagar {} ({})", a.target, artifact_kind(a.kind))
            }
            crate::install::state::ArtifactKind::Persist => {
                format!(
                    "Manter {} ({}; use --purge)",
                    a.target,
                    artifact_kind(a.kind)
                )
            }
            _ => format!("Desfazer {}: {}", artifact_kind(a.kind), a.target),
        };
        line(text);
    }
    s
}

/// One line per residue for the picker and the listings.
pub fn residue_label(r: &crate::install::residue::Residue) -> String {
    use crate::install::residue::ResidueKind as K;
    match &r.kind {
        K::RegValue { key, value_name } => {
            format!(
                "Registro: HKCU\\{key} [{}]",
                value_name.as_deref().unwrap_or("")
            )
        }
        K::RegKey { key } => format!("Registro: HKCU\\{key}"),
        K::Folder { path } => format!("Pasta: {}", path.display()),
        K::Shortcut { path } => format!("Atalho: {}", path.display()),
    }
}

/// `app  de -> para` lines for the upgrade listing.
pub fn upgrades_text(rows: &[(String, String, String)]) -> String {
    let mut s = String::new();
    for (app, from, to) in rows {
        s.push_str(&format!("{app}  {from} -> {to}\n"));
    }
    s
}

pub fn repair_text(r: &crate::install::ops::RepairReport) -> String {
    let list = |v: &[String]| {
        if v.is_empty() {
            "nenhum".to_string()
        } else {
            v.join(", ")
        }
    };
    format!(
        "Reconstruídos: {}\nMantidos: {}\nRegistros removidos (app sumiu do disco): {}\nPulados (sem install.json legível): {}\n",
        list(&r.rebuilt),
        list(&r.kept),
        list(&r.removed),
        list(&r.skipped)
    )
}

fn mib(n: u64) -> String {
    format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0))
}

/// Single-line download progress, padded so a shorter line fully overwrites a longer one.
pub fn progress_line(done: u64, total: Option<u64>) -> String {
    let text = match total {
        Some(t) if t > 0 => format!(
            "Baixando: {} / {} ({}%)",
            mib(done),
            mib(t),
            (done.min(t) * 100) / t
        ),
        _ => format!("Baixando: {}", mib(done)),
    };
    format!("{text:<60}")
}

#[cfg(test)]
mod install_output_tests {
    use super::*;

    #[test]
    fn progress_line_shows_percent_and_pads() {
        let l = progress_line(512 * 1024, Some(1024 * 1024));
        assert!(l.starts_with("Baixando: 0.5 MiB / 1.0 MiB (50%)"), "{l}");
        assert_eq!(l.chars().count(), 60);
        assert!(progress_line(10, None).starts_with("Baixando: 0.0 MiB"));
    }

    #[test]
    fn ambiguous_text_lists_bucket_ids() {
        let t = ambiguous_text("x", &["main/x".into(), "extras/x".into()]);
        assert!(
            t.contains("\n  main/x") && t.contains("\n  extras/x"),
            "{t}"
        );
        assert_eq!(bucket_id("scoop:main", "x"), "main/x");
    }
}
