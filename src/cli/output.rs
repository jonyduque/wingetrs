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

/// `  N. text` with the number in bold.
fn numbered(n: usize, text: &str) -> String {
    format!("  {}. {text}\n", paint(NAME, &n.to_string()))
}

/// File name without directory and `.lnk`.
fn shortcut_name(link: &str) -> &str {
    let file = link.rsplit(['\\', '/']).next().unwrap_or(link);
    file.strip_suffix(".lnk").unwrap_or(file)
}

/// What `uninstall` is about to do, one numbered line per action.
pub fn uninstall_plan_text(
    plan: &crate::install::plan::UninstallPlan,
    _layout: &crate::install::layout::Layout,
    glyphs: &crate::tui::theme::Glyphs,
) -> String {
    use crate::install::state::ArtifactKind as K;
    let mut s = format!(
        "{} {} {}\n",
        paint(HEADER, "Desinstalar"),
        paint(NAME, &plan.app),
        paint(VERSION, &plan.version)
    );
    let mut lines: Vec<String> = vec!["Apaga a pasta do app".into()];
    let of = |k: K| plan.artifacts.iter().filter(move |a| a.kind == k);
    let shims: Vec<&str> = of(K::Shim).map(|a| a.target.as_str()).collect();
    if !shims.is_empty() {
        lines.push(format!("Comandos: {}", shims.join(", ")));
    }
    for a in of(K::Shortcut) {
        lines.push(format!(
            "Atalho no Menu Iniciar: {}",
            shortcut_name(&a.target)
        ));
    }
    for a in of(K::Path) {
        match plan.path_users.iter().find(|(t, _)| *t == a.target) {
            Some((_, users)) => lines.push(format!(
                "PATH do usuário mantém {} (ainda usado por: {})",
                a.target,
                users.join(", ")
            )),
            None if plan.remove_path_entry => lines.push(format!(
                "PATH do usuário {} remove {}",
                glyphs.arrow, a.target
            )),
            None => {}
        }
    }
    for a in of(K::Env) {
        lines.push(format!(
            "Variável de usuário {} volta ao valor anterior",
            a.target
        ));
    }
    for a in of(K::Persist) {
        lines.push(if plan.purge {
            format!("Apaga os dados: {}", a.target)
        } else {
            format!("Dados mantidos: {} (use --purge para apagar)", a.target)
        });
    }
    for (i, text) in lines.iter().enumerate() {
        s.push_str(&numbered(i + 1, text));
    }
    s
}

/// The short plan `install`/`upgrade` show before asking: only what touches the world
/// outside the app folder, numbered in plan order.
pub fn plan_summary(
    plan: &crate::install::plan::InstallPlan,
    layout: &crate::install::layout::Layout,
    glyphs: &crate::tui::theme::Glyphs,
    user_path: Option<&str>,
    allow_msiexec: bool,
) -> String {
    use crate::install::archive::ArchiveKind;
    use crate::install::plan::Step;
    let arch = plan.arch.map_or("sem arquitetura", |a| a.key());
    let mut s = match &plan.upgrade_from {
        Some(old) => format!(
            "{} {} {} {} {}",
            paint(HEADER, "Atualizar"),
            paint(NAME, &plan.app),
            paint(VERSION, old),
            glyphs.arrow,
            paint(NEW_VERSION, &plan.version)
        ),
        None => format!(
            "{} {} {}",
            paint(HEADER, "Instalar"),
            paint(NAME, &plan.app),
            paint(VERSION, &plan.version)
        ),
    };
    s.push_str(&paint(DIM, &format!("  {} · {arch}", plan.bucket)));
    s.push('\n');
    let shims: Vec<&str> = plan
        .steps
        .iter()
        .filter_map(|st| match st {
            Step::Shim(sp) => Some(sp.name.as_str()),
            _ => None,
        })
        .collect();
    let persists: Vec<String> = plan
        .steps
        .iter()
        .filter_map(|st| match st {
            Step::Persist(p) => Some(p.in_version.display().to_string()),
            _ => None,
        })
        .collect();
    // Folders the user PATH already has need no line.
    let on_path =
        |dir: &str| user_path.is_some_and(|p| crate::install::path_edit::path_contains(p, dir));
    let (mut shims_done, mut persists_done) = (false, false);
    let mut lines: Vec<String> = Vec::new();
    for step in &plan.steps {
        match step {
            Step::Shim(_) if !shims_done => {
                shims_done = true;
                lines.push(format!("Comandos: {}", shims.join(", ")));
            }
            Step::Persist(_) if !persists_done => {
                persists_done = true;
                lines.push(format!(
                    "Dados mantidos entre versões: {}",
                    persists.join(", ")
                ));
            }
            Step::Shortcut(sc) => lines.push(format!(
                "Atalho no Menu Iniciar: {}",
                shortcut_name(&sc.link.display().to_string())
            )),
            Step::EnvSet { name, value } => {
                lines.push(format!("Variável de usuário {name} = {value}"));
            }
            Step::EnvAddPath { entry } if !on_path(&entry.display().to_string()) => {
                lines.push(format!(
                    "PATH do usuário {} {}",
                    glyphs.arrow,
                    entry.display()
                ));
            }
            Step::EnsureShimsOnPath if !on_path(&layout.shims().display().to_string()) => {
                lines.push(format!(
                    "PATH do usuário {} {}",
                    glyphs.arrow,
                    layout.shims().display()
                ));
            }
            Step::RemoveOldVersion { version } => {
                lines.push(format!("Remove a versão {version}"));
            }
            // Shim/Persist already grouped; the rest is internal bookkeeping.
            _ => {}
        }
    }
    for (i, text) in lines.iter().enumerate() {
        s.push_str(&numbered(i + 1, text));
    }
    let has_msi = plan.steps.iter().any(|st| {
        matches!(st, Step::Download { source, .. }
            if matches!(ArchiveKind::of(&source.file_name), Ok(ArchiveKind::Msi)))
    });
    if has_msi {
        s.push_str(&paint(
            DIM,
            if allow_msiexec {
                "  Se o extrator recusar este MSI, usa o Windows Installer (msiexec /a)\n"
            } else {
                "  Se o extrator não suportar este MSI, pede --allow-msiexec\n"
            },
        ));
    }
    s
}

/// After a successful install: the command names to use once a new terminal sees the PATH
/// folder the plan added, or `None` when the running process already has every folder.
pub fn new_terminal_hint(
    plan: &crate::install::plan::InstallPlan,
    layout: &crate::install::layout::Layout,
    process_path: Option<&std::ffi::OsStr>,
) -> Option<String> {
    use crate::install::plan::Step;
    let norm = |p: &std::path::Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    let have: Vec<String> = process_path
        .map(|p| std::env::split_paths(p).map(|d| norm(&d)).collect())
        .unwrap_or_default();
    let folders = plan.steps.iter().filter_map(|st| match st {
        Step::EnsureShimsOnPath => Some(layout.shims()),
        Step::EnvAddPath { entry } => Some(entry.clone()),
        _ => None,
    });
    let missing = folders.into_iter().any(|f| !have.contains(&norm(&f)));
    let names: Vec<&str> = plan
        .steps
        .iter()
        .filter_map(|st| match st {
            Step::Shim(sp) => Some(sp.name.as_str()),
            _ => None,
        })
        .collect();
    if !missing {
        return None;
    }
    Some(if names.is_empty() {
        paint(DIM, "Abra um terminal novo para o PATH novo valer.")
    } else {
        format!(
            "{} {}",
            paint(DIM, "Abra um terminal novo para usar:"),
            names.join(", ")
        )
    })
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

/// Phase of an install as the CLI shows it on one stderr line: downloads (summed over all
/// URLs), the Windows Installer, then "Instalando…". Pure state; the caller draws.
#[derive(Default)]
pub struct Phases {
    per_url: std::collections::BTreeMap<String, (u64, Option<u64>)>,
    installer_since: Option<std::time::Instant>,
}

impl Phases {
    pub fn new() -> Phases {
        Phases::default()
    }

    /// One progress event: `label` is a download URL, except `INSTALLER_RUNNING`.
    pub fn update(&mut self, label: &str, done: u64, total: Option<u64>) {
        if label == crate::install::executor::INSTALLER_RUNNING {
            // Each launch restarts the clock; it is never a download.
            self.installer_since = Some(std::time::Instant::now());
        } else {
            self.installer_since = None;
            self.per_url.insert(label.to_string(), (done, total));
        }
    }

    /// The line for spinner frame `frame`, or `None` while there is nothing to show.
    pub fn line(&self, frame: usize, glyphs: &crate::tui::theme::Glyphs) -> Option<String> {
        let spin = glyphs.spinner.get(frame % glyphs.spinner.len().max(1))?;
        if let Some(since) = self.installer_since {
            let secs = since.elapsed().as_secs();
            return Some(format!(
                "{spin} Windows Installer (msiexec /a) · não pode ser cancelado · {:02}:{:02}",
                secs / 60,
                secs % 60
            ));
        }
        if self.per_url.is_empty() {
            return None;
        }
        let done: u64 = self.per_url.values().map(|(d, _)| d).sum();
        let known = self.per_url.values().all(|(_, t)| t.is_some());
        let total: u64 = self.per_url.values().filter_map(|(_, t)| *t).sum();
        let finished = self
            .per_url
            .values()
            .all(|(d, t)| t.is_some_and(|t| *d >= t));
        if finished {
            return Some(format!("{spin} Instalando…"));
        }
        Some(match known.then_some(total).filter(|t| *t > 0) {
            Some(t) => {
                let pct = done.min(t) * 100 / t;
                let filled = usize::try_from(pct / 5).unwrap_or(20).min(20);
                format!(
                    "{spin} Baixando [{}{}] {pct}%  {:.1} / {}",
                    "#".repeat(filled),
                    "-".repeat(20 - filled),
                    done as f64 / (1024.0 * 1024.0),
                    mib(t)
                )
            }
            None => format!("{spin} Baixando {}", mib(done)),
        })
    }
}
#[cfg(test)]
mod install_output_tests {
    use super::*;

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
