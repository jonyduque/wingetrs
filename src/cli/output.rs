//! Text rendering for CLI commands; colored through `style::paint` when color is on.
use super::style::{DIM, ERROR, HEADER, ID, NAME, NEW_VERSION, OK, SOURCE, Style, VERSION, paint};
use crate::core::{PackageDetails, PackageEntry};
use crate::installed::correlate::{Correlation, ListRow, UpdateState};
use crate::update::{Outcome, SourceReport};

/// `text` painted, then padded to `width` columns counted on the plain text.
fn pad(style: Style, text: &str, width: usize) -> String {
    let pad = " ".repeat(width.saturating_sub(text.chars().count()));
    format!("{}{pad}", paint(style, text))
}

/// Prints `value` as pretty JSON, never colored.
pub fn print_json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    use std::io::Write;
    let text = serde_json::to_string_pretty(value)?;
    writeln!(std::io::stdout().lock(), "{text}")?;
    Ok(())
}

/// `Nome  Id  Versão  Fonte` with aligned columns.
pub fn search_table(rows: &[PackageEntry]) -> String {
    let cells: Vec<[String; 4]> = rows
        .iter()
        .map(|e| {
            [
                e.name.clone(),
                e.id.clone(),
                e.version.clone(),
                e.source_name.clone(),
            ]
        })
        .collect();
    aligned(
        ["Nome", "Id", "Versão", "Fonte"],
        [NAME, ID, VERSION, SOURCE],
        &cells,
    )
}

pub fn details_text(d: &PackageDetails) -> String {
    let mut out = String::new();
    let mut field = |k: &str, v: &str| {
        out.push_str(&format!("{}: {v}\n", paint(HEADER, k)));
    };
    field("Nome", &paint(NAME, &d.name));
    field("Id", &paint(ID, &d.id));
    field("Versão", &paint(VERSION, &d.version));
    field("Fonte", &paint(SOURCE, d.source.as_str()));
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
            "{}: {} {} {} {}\n",
            paint(HEADER, "Instalador"),
            i.architecture,
            i.installer_type,
            scope,
            i.url
        ));
    }
    out
}

pub fn report_line(r: &SourceReport) -> String {
    let src = paint(SOURCE, &r.source.to_string());
    match &r.outcome {
        Outcome::Fresh => format!("{src}: {}", paint(OK, "em dia")),
        Outcome::Unchanged => format!("{src}: sem mudanças"),
        Outcome::Updated { packages, skipped } => {
            let n = paint(OK, &format!("{packages} pacotes"));
            format!("{src}: {n} ({skipped} ignorados)")
        }
        Outcome::Failed { message: m, .. } => {
            format!("{src}: {} {m}", paint(ERROR, "falhou:"))
        }
    }
}

fn aligned<const N: usize>(header: [&str; N], styles: [Style; N], cells: &[[String; N]]) -> String {
    let mut widths = header.map(|h| h.chars().count());
    for r in cells {
        for (w, c) in widths.iter_mut().zip(r) {
            *w = (*w).max(c.chars().count());
        }
    }
    let line = |r: &[&str], styles: [Style; N]| {
        let parts: Vec<String> = r
            .iter()
            .zip(widths)
            .zip(styles)
            .map(|((c, w), st)| pad(st, c, w))
            .collect();
        parts.join("  ").trim_end().to_string()
    };
    let mut out = line(&header, [HEADER; N]);
    out.push('\n');
    for r in cells {
        let refs: Vec<&str> = r.iter().map(String::as_str).collect();
        out.push_str(&line(&refs, styles));
        out.push('\n');
    }
    out
}

/// The name with `(sobra provável)` when the row's uninstaller is gone.
fn shown_name(r: &ListRow) -> String {
    if r.stale.is_some() {
        format!("{} (sobra provável)", r.name)
    } else {
        r.name.clone()
    }
}

/// `, +N registros` for a row that folded other records, else empty.
fn extra_suffix(r: &ListRow) -> String {
    if r.others.is_empty() {
        String::new()
    } else {
        format!(
            ", {}",
            crate::installed::correlate::extra_records_label(r.others.len())
        )
    }
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
                let installed = if r.others.is_empty() {
                    installed
                } else {
                    format!(
                        "{installed} {}",
                        crate::installed::correlate::extra_records_label(r.others.len())
                    )
                };
                cells.push([
                    shown_name(r),
                    package_id.clone(),
                    installed,
                    available,
                    source.clone(),
                    r.origin.clone(),
                ]);
            }
            Correlation::Ambiguous { candidates, .. } => {
                ambiguous.push(format!(
                    "{} ({installed}{}, {}): {}",
                    shown_name(r),
                    extra_suffix(r),
                    r.origin,
                    candidates.join(", ")
                ));
            }
            Correlation::Unmatched => {
                unmatched.push(format!(
                    "{} ({installed}{}, {})",
                    shown_name(r),
                    extra_suffix(r),
                    r.origin
                ));
            }
        }
    }
    let mut out = aligned(
        ["Nome", "Id", "Instalada", "Disponível", "Fonte", "Origem"],
        [NAME, ID, VERSION, NEW_VERSION, SOURCE, DIM],
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
            out.push_str(&format!("\n{} ({}):\n", paint(HEADER, title), items.len()));
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
        s.push_str(&paint(ID, c));
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
            suggestions
                .iter()
                .map(|x| paint(ID, x))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// What `uninstall` is about to do, one numbered line per action.
pub fn uninstall_plan_text(
    plan: &crate::install::plan::UninstallPlan,
    layout: &crate::install::layout::Layout,
) -> String {
    let mut s = format!(
        "{} {} {}\n",
        paint(HEADER, "Desinstalar"),
        paint(NAME, &plan.app),
        paint(VERSION, &plan.version)
    );
    let mut n = 1;
    let mut line = |text: String| {
        s.push_str(&format!("{n:>3}. {text}\n"));
        n += 1;
    };
    line(format!("Apagar {}", layout.app_dir(&plan.app).display()));
    for text in plan.artifact_lines() {
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
        s.push_str(&format!(
            "{}  {} -> {}\n",
            paint(NAME, app),
            paint(VERSION, from),
            paint(NEW_VERSION, to)
        ));
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
    let h = |t: &str| paint(HEADER, t);
    format!(
        "{}: {}\n{}: {}\n{}: {}\n{}: {}\n",
        h("Reconstruídos"),
        list(&r.rebuilt),
        h("Mantidos"),
        list(&r.kept),
        h("Registros removidos (app sumiu do disco)"),
        list(&r.removed),
        h("Pulados (sem install.json legível)"),
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
