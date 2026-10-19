//! Text rendering for CLI commands; colored through `style::paint` when color is on.
use super::style::{
    self, DIM, ERROR, HEADER, ID, NAME, NEW_VERSION, OK, SOURCE, Style, VERSION, WARN, paint,
};
use crate::core::{PackageDetails, PackageEntry};
use crate::install::layout::Layout;
use crate::install::manifest::{self, Arch, HookKind, Resolved};
use crate::install::msiexec_log::LogTail;
use crate::install::plan::{InstallPlan, Step};
use crate::install::planner;
use crate::install::prelude::PRELUDE_FUNCTIONS;
use crate::install::progress::{Phase, ProgressEvent, phase_status, phase_text, phase_title};
use crate::install::script_scan::{self, CallRoots, ScriptReport, Verdict, printable};
use crate::installed::correlate::{Correlation, ListRow, UpdateState};
use crate::update::{Outcome, SourceReport};
use std::collections::VecDeque;
use std::path::Path;
use std::time::{Duration, Instant};

/// Terminal columns of `text`: wide (CJK, emoji) count two, combining marks none.
pub fn columns(text: &str) -> usize {
    ratatui::text::Span::raw(text).width()
}

/// `text` cut to `width` terminal columns, ending in `…` when cut.
pub fn truncate(text: &str, width: usize) -> String {
    if columns(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let mut buf = [0u8; 4];
        let w = columns(c.encode_utf8(&mut buf));
        if used + w + 1 > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// `text` painted, then padded to `width` terminal columns counted on the plain text.
fn pad(style: Style, text: &str, width: usize) -> String {
    let pad = " ".repeat(width.saturating_sub(columns(text)));
    format!("{}{pad}", paint(style, text))
}

/// A success line: `✓ Instalado: rest` (glyph and label in the success color).
pub fn success(label: &str, rest: &str) -> String {
    let head = paint(OK, &format!("{} {label}", style::glyphs().toast_ok));
    if rest.is_empty() {
        head
    } else {
        format!("{head} {rest}")
    }
}

/// A warning line: `⚠ aviso: text`.
pub fn warning(text: &str) -> String {
    let head = paint(WARN, &format!("{} aviso:", style::glyphs().toast_warn));
    format!("{head} {text}")
}

/// Text in the warning color, without glyph (notices said in full sentences).
pub fn warn_text(text: &str) -> String {
    paint(WARN, text)
}

/// One item of a batch that failed: `✗ Erro: text`.
pub fn failed_item(text: &str) -> String {
    let head = paint(ERROR, &format!("{} Erro:", style::glyphs().toast_err));
    format!("{head} {text}")
}

/// `install` of a winget package through the bridge: `<name version> via winget`.
pub fn via_winget(name_version: &str) -> String {
    format!("{} {}", paint(NAME, name_version), paint(DIM, "via winget"))
}

/// The line of an operation the user stopped or declined: `⚠ Cancelado.`
pub fn cancelled() -> String {
    paint(WARN, &format!("{} Cancelado.", style::glyphs().toast_warn))
}

/// The last line of a `--dry-run`.
pub const DRY_RUN_DONE: &str = "Simulação: nada foi alterado.";

/// What the user can do next: `→ text` (the arrow dimmed; the text keeps its own styles).
pub fn action(text: &str) -> String {
    format!("{} {text}", paint(DIM, style::glyphs().arrow))
}

/// A count in bold.
fn bold_count(n: usize) -> String {
    paint(NAME, &n.to_string())
}

/// A package, app or bucket name in bold.
pub fn name(text: &str) -> String {
    paint(NAME, text)
}

/// `bucket list`: `name  owner/repo`, one per line.
pub fn buckets_text(rows: &[(String, String)]) -> String {
    let cells: Vec<[String; 2]> = rows.iter().map(|(n, r)| [n.clone(), r.clone()]).collect();
    table(None, [NAME, ID], &cells)
}

/// A path of the disk, in its own style.
pub fn path(p: &Path) -> String {
    paint(style::PATH, &p.display().to_string())
}

/// A failure the user can act on: [`error_report`] shows `what` and then `→ todo` on a line of
/// its own. `Display` keeps the one-line form `what; todo` (logs, job log, tests).
#[derive(Debug, thiserror::Error)]
#[error("{what}; {todo}")]
pub struct Advice {
    pub what: String,
    pub todo: String,
}

impl Advice {
    pub fn new(what: impl Into<String>, todo: impl Into<String>) -> Advice {
        Advice {
            what: what.into(),
            todo: todo.into(),
        }
    }
}

/// The error of a failed command, for stderr: `✗ <what failed>`, then `→ <what to do>` when
/// there is an action ([`Advice`]). The causes of the `anyhow` chain only with `-v`; without
/// it a chain says how to see them.
pub fn error_report(e: &anyhow::Error, verbose: bool) -> String {
    let g = style::glyphs();
    // Only the outermost error: an `Advice` under a `.context()` is a cause.
    let top = e.chain().next().and_then(|c| c.downcast_ref::<Advice>());
    let (what, todo) = match top {
        Some(a) => (a.what.clone(), Some(a.todo.as_str())),
        None => (e.to_string(), None),
    };
    let mut s = format!("{} {what}\n", paint(ERROR, g.toast_err));
    if let Some(todo) = todo {
        s.push_str(&action(todo));
        s.push('\n');
    }
    let causes: Vec<String> = e.chain().skip(1).map(ToString::to_string).collect();
    if verbose {
        for c in &causes {
            s.push_str(&paint(DIM, &format!("  causa: {c}")));
            s.push('\n');
        }
    } else if !causes.is_empty() {
        s.push_str(&action("rode de novo com -v para ver a causa"));
        s.push('\n');
    }
    s
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
    let g = style::glyphs();
    let src = paint(SOURCE, &r.source.to_string());
    let ok = paint(OK, g.toast_ok);
    match &r.outcome {
        Outcome::Fresh => format!("{ok} {src}: {}", paint(OK, "em dia")),
        Outcome::Unchanged => format!("{ok} {src}: sem mudanças"),
        Outcome::Updated { packages, skipped } => {
            let n = paint(NAME, &packages.to_string());
            format!(
                "{ok} {src}: {n} {} ({} ignorados)",
                paint(OK, "pacotes"),
                paint(NAME, &skipped.to_string())
            )
        }
        Outcome::Failed { message: m, .. } => {
            let (x, failed) = (paint(ERROR, g.toast_err), paint(ERROR, "falhou:"));
            format!("{x} {src}: {failed} {m}")
        }
    }
}

fn aligned<const N: usize>(header: [&str; N], styles: [Style; N], cells: &[[String; N]]) -> String {
    table(Some(header), styles, cells)
}

/// Columns padded to the widest cell (terminal columns, so wide chars and accents align),
/// joined by two spaces; the header, when given, in `HEADER`.
fn table<const N: usize>(
    header: Option<[&str; N]>,
    styles: [Style; N],
    cells: &[[String; N]],
) -> String {
    let mut widths = header.map_or([0; N], |h| h.map(columns));
    for r in cells {
        for (w, c) in widths.iter_mut().zip(r) {
            *w = (*w).max(columns(c));
        }
    }
    // On a terminal the first column (the names) gives up what the line lacks, so no line
    // wraps (one column less than the terminal); never below the header nor 8 columns.
    if let Some(cols) = style::table_columns() {
        let total = widths.iter().sum::<usize>() + 2 * N.saturating_sub(1);
        let room = cols.saturating_sub(1);
        if total > room {
            let floor = header.map_or(0, |h| columns(h[0])).max(8);
            widths[0] = widths[0].saturating_sub(total - room).max(floor);
        }
    }
    let first = widths[0];
    let line = |r: &[&str], styles: [Style; N]| {
        let parts: Vec<String> = r
            .iter()
            .zip(widths)
            .zip(styles)
            .enumerate()
            .map(|(i, ((c, w), st))| {
                if i == 0 {
                    pad(st, &truncate(c, first), w)
                } else {
                    pad(st, c, w)
                }
            })
            .collect();
        parts.join("  ").trim_end().to_string()
    };
    let mut out = String::new();
    if let Some(header) = header {
        out.push_str(&line(&header, [HEADER; N]));
        out.push('\n');
    }
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
            out.push_str(&format!(
                "\n{} ({}):\n",
                paint(HEADER, title),
                bold_count(items.len())
            ));
            for i in items {
                out.push_str(&format!("  {i}\n"));
            }
        }
        out.push_str(&format!(
            "\n{} instalados: {} correlacionados, {} ambíguos, {} não correlacionados\n",
            bold_count(rows.len()),
            bold_count(cells.len()),
            bold_count(ambiguous.len()),
            bold_count(unmatched.len())
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

/// The error of a target that is not in the catalog, with the close names as the action.
pub fn not_found(query: &str, suggestions: &[String]) -> Advice {
    let what = format!("nenhum pacote encontrado para '{query}'");
    if suggestions.is_empty() {
        Advice::new(what, "procure o nome com `wingetrs search <termo>`")
    } else {
        let names: Vec<String> = suggestions.iter().map(|x| paint(ID, x)).collect();
        Advice::new(what, format!("você quis dizer: {}?", names.join(", ")))
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
                paint(style::PATH, &a.target),
                users.join(", ")
            )),
            None if plan.remove_path_entry => lines.push(format!(
                "PATH do usuário {} remove {}",
                glyphs.arrow,
                paint(style::PATH, &a.target)
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

/// `Instalar app 1.0  bucket · arch` (or `Atualizar app 0.9 → 1.0 …`): the first line of the
/// short and the full plan.
fn plan_header(
    plan: &crate::install::plan::InstallPlan,
    glyphs: &crate::tui::theme::Glyphs,
) -> String {
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
    s
}

/// The full plan of `--dry-run` (the only place with URLs, hashes and the staging folder):
/// the header, every step numbered (number in bold, right-aligned), the planner's notes as
/// warnings.
pub fn install_plan_text(
    plan: &crate::install::plan::InstallPlan,
    glyphs: &crate::tui::theme::Glyphs,
) -> String {
    let mut s = plan_header(plan, glyphs);
    let lines = plan.step_lines();
    let width = lines.len().to_string().len();
    for (i, line) in lines.iter().enumerate() {
        let n = format!("{:>width$}", i + 1);
        s.push_str(&format!("  {}. {line}\n", paint(NAME, &n)));
    }
    for note in &plan.notes {
        s.push_str(&warning(note));
        s.push('\n');
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
    use crate::install::plan::Step;
    let mut s = plan_header(plan, glyphs);
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
                lines.push(format!("PATH do usuário {} {}", glyphs.arrow, path(entry)));
            }
            Step::EnsureShimsOnPath if !on_path(&layout.shims().display().to_string()) => {
                lines.push(format!(
                    "PATH do usuário {} {}",
                    glyphs.arrow,
                    path(&layout.shims())
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
    if let Some(hint) = msi_hint(plan, allow_msiexec) {
        s.push_str(&paint(DIM, &format!("  {hint}\n")));
    }
    s
}

/// What happens when the own extractor refuses an MSI of `plan` (`None`: it downloads no
/// MSI); shown by the summary and by `--dry-run`.
pub fn msi_hint(plan: &InstallPlan, allow_msiexec: bool) -> Option<&'static str> {
    use crate::install::archive::ArchiveKind;
    let has_msi = plan.steps.iter().any(|st| {
        matches!(st, Step::Download { source, .. }
            if matches!(ArchiveKind::of(&source.file_name), Ok(ArchiveKind::Msi)))
    });
    has_msi.then_some(if allow_msiexec {
        "Se o extrator recusar este MSI, usa o Windows Installer (msiexec /a)"
    } else {
        "Se o extrator não suportar este MSI, pede --allow-msiexec"
    })
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
    let arrow = style::glyphs().arrow;
    let cells: Vec<[String; 4]> = rows
        .iter()
        .map(|(app, from, to)| [app.clone(), from.clone(), arrow.to_string(), to.clone()])
        .collect();
    table(None, [NAME, VERSION, DIM, NEW_VERSION], &cells)
}

/// `upgrade` without `--all` and without an app: what has a newer version, the pinned apps,
/// and how to go on.
pub fn upgrades_listing(rows: &[(String, String, String)], pinned: &[String]) -> String {
    let mut s = upgrades_text(rows);
    for app in pinned {
        s.push_str(&format!(
            "{} {}\n",
            paint(NAME, app),
            paint(DIM, "(fixado)")
        ));
    }
    if rows.is_empty() {
        s.push_str(NOTHING_TO_UPGRADE);
    } else {
        s.push_str(&action(
            "use `wingetrs upgrade --all` ou `wingetrs upgrade <app>`",
        ));
    }
    s.push('\n');
    s
}

/// Nothing has a newer version.
pub const NOTHING_TO_UPGRADE: &str = "Nada para atualizar.";

/// `upgrade --all` skips a pinned app.
pub fn pinned_skipped(app: &str) -> String {
    paint(DIM, &format!("fixado, pulado: {app}"))
}

/// `upgrade <app>` of an app already at the newest version.
pub fn up_to_date(app: &str) -> String {
    format!(
        "{} {} já está na versão mais recente.",
        paint(OK, style::glyphs().toast_ok),
        paint(NAME, app)
    )
}

/// `app version`, the name in bold.
pub fn app_version(app: &str, version: &str) -> String {
    format!("{} {}", paint(NAME, app), paint(VERSION, version))
}

/// Dimmed text (hints that need no action).
pub fn dim(text: &str) -> String {
    paint(DIM, text)
}

/// Residues found after a removal without a terminal and without `--yes`.
pub fn residues_left(residues: &[crate::install::residue::Residue]) -> String {
    let mut s = warning("resíduos encontrados (nada foi removido)");
    s.push('\n');
    for r in residues {
        s.push_str(&format!("  {}\n", residue_label(r)));
    }
    s.push_str(&action("use --yes para remover os de confiança alta"));
    s.push('\n');
    s
}

/// The animated progress line: `main`, then its warning (`sem dados há N s`) in yellow.
pub fn progress_line(main: &str, warn: &str) -> String {
    format!("{main}{}", paint(WARN, warn))
}

/// `Label: value` with the label in header style (the `--dry-run` details and the license
/// lines are built this way by their plans).
fn labeled(line: &str) -> String {
    match line.split_once(": ") {
        Some((label, value)) => format!("{}: {value}", paint(HEADER, label)),
        None => line.to_string(),
    }
}

/// A license line of a winget package (`Licença: EULA — url`, `Licença aceita: …`).
pub fn license(line: &str) -> String {
    labeled(line)
}

/// The steps that ask for the administrator's permission (they get the warning glyph).
fn asks_admin(step: &str) -> bool {
    use crate::winget_install::plan::{ADMIN_PROMPT, ADMIN_PROMPT_MSI, SELF_ADMIN_PROMPT};
    use crate::winget_install::uninstall::{UNINSTALL_ADMIN_PROMPT, WINGET_ADMIN_PROMPT};
    [
        ADMIN_PROMPT,
        ADMIN_PROMPT_MSI,
        SELF_ADMIN_PROMPT,
        UNINSTALL_ADMIN_PROMPT,
        WINGET_ADMIN_PROMPT,
    ]
    .contains(&step)
}

/// A plan in the shape of the Scoop one: the title, every step numbered (number in bold, the
/// UAC steps with the warning glyph, licenses labeled) and, only for `--dry-run`, the details
/// (URL, command line, hash). `warnings` follow the steps as their own block, each with the
/// warning glyph and unnumbered (Task 203).
fn numbered_plan(
    title: String,
    steps: &[String],
    warnings: &[String],
    details: &[String],
) -> String {
    let mut s = title;
    s.push('\n');
    let width = steps.len().to_string().len();
    for (i, step) in steps.iter().enumerate() {
        let n = paint(NAME, &format!("{:>width$}", i + 1));
        let text = if asks_admin(step) {
            format!("{} {step}", paint(WARN, style::glyphs().toast_warn))
        } else if step.starts_with("Licença") {
            license(step)
        } else {
            step.clone()
        };
        s.push_str(&format!("  {n}. {text}\n"));
    }
    for line in warnings {
        s.push_str(&format!(
            "  {} {}\n",
            paint(WARN, style::glyphs().toast_warn),
            paint(WARN, line)
        ));
    }
    for line in details {
        s.push_str(&format!("  {}\n", labeled(line)));
    }
    s
}

/// The native winget install/upgrade plan: `7-Zip 26.04` in bold and its `(tipo, escopo)`
/// dimmed, then [`numbered_plan`] of `WingetPlan::steps` (the TUI lists the same steps).
pub fn winget_plan_text(plan: &crate::winget_install::plan::WingetPlan, dry_run: bool) -> String {
    let title = plan.title();
    let title = match title.rfind(" (") {
        Some(at) => format!("{}{}", paint(NAME, &title[..at]), paint(DIM, &title[at..])),
        None => paint(NAME, &title),
    };
    let details = if dry_run {
        plan.dry_run_details()
    } else {
        Vec::new()
    };
    numbered_plan(title, &plan.steps(), &[], &details)
}

/// The install of a winget package through winget, in the shape of the native plan (Task
/// 203): `<name version> via winget`, then the numbered `steps` (`Instala pelo winget (zip)`,
/// licenses) and, only for `--dry-run`, `details` (the command line).
pub fn bridge_plan_text(name_version: &str, steps: &[String], details: &[String]) -> String {
    numbered_plan(via_winget(name_version), steps, &[], details)
}

/// The removal of an app of the Windows list (ARP/MSIX): `Desinstalar` as a header and the
/// name in bold, then [`numbered_plan`] of `UninstallPlan::steps` (the TUI lists the same).
pub fn winget_uninstall_text(
    plan: &crate::winget_install::uninstall::UninstallPlan,
    dry_run: bool,
) -> String {
    let title = plan.title();
    let title = match title.split_once(' ') {
        Some((verb, rest)) => format!("{} {}", paint(HEADER, verb), paint(NAME, rest)),
        None => paint(NAME, &title),
    };
    let details = if dry_run {
        plan.dry_run_details()
    } else {
        Vec::new()
    };
    numbered_plan(title, &plan.steps(), &plan.warnings(), &details)
}

/// The engine chose `winget` for this package: `Instalando pelo winget: <why>`.
pub fn through_winget(upgrade: bool, why: &str) -> String {
    let verb = if upgrade {
        "Atualizando pelo winget:"
    } else {
        "Instalando pelo winget:"
    };
    format!("{} {why}", paint(DIM, verb))
}

/// After a removal that copied the shared folders first.
pub fn folders_copied(dest: &Path) -> String {
    format!(
        "{} Pastas copiadas para {}",
        paint(OK, style::glyphs().toast_ok),
        path(dest)
    )
}

/// The installer asked for a restart (`initiated`: it is restarting now).
pub fn reboot(initiated: bool) -> String {
    warn_text(if initiated {
        "O instalador está reiniciando o computador."
    } else {
        "Reinicie o computador para concluir."
    })
}

/// `cleanup --orphans`: one record whose uninstaller is gone.
pub fn orphan_line(name: &str, hive: &str, missing: &str) -> String {
    format!(
        "{} {} — {} — falta {}",
        paint(WARN, "Órfão:"),
        paint(NAME, name),
        paint(DIM, hive),
        paint(style::PATH, missing)
    )
}

/// The registry-only removal of an app: `✓ Removido: registro de <name> (cópia em <file>)`.
pub fn record_removed(name: &str, file: &Path) -> String {
    success(
        "Removido:",
        &format!(
            "registro de {} (cópia em {})",
            paint(NAME, name),
            path(file)
        ),
    )
}

/// `cleanup --orphans` removed one record: `✓ Removido: <name> (backup <file>)`.
pub fn orphan_removed(name: &str, file: &Path) -> String {
    success(
        "Removido:",
        &format!("{} (backup {})", paint(NAME, name), path(file)),
    )
}

/// `cleanup --orphans` could not remove one record: `✗ Erro: <name>: <error>`.
pub fn orphan_failed(name: &str, error: &str) -> String {
    failed_item(&format!("{}: {error}", paint(NAME, name)))
}

/// `cleanup --orphans` found nothing.
pub const NO_ORPHANS: &str = "Nenhum registro órfão.";

/// Why a listed orphan may not be one.
pub fn orphans_may_have_moved() -> String {
    paint(
        DIM,
        "O app pode ter mudado de lugar (ex.: Steam em outra pasta).",
    )
}

/// Where `cleanup --orphans` exports each record before deleting it.
pub fn orphans_backup(dir: &Path) -> String {
    format!(
        "Antes de apagar, cada registro é exportado para um .reg em {}",
        path(dir)
    )
}

/// `cleanup --orphans` without a terminal and without `--yes`.
pub fn orphans_not_removed() -> String {
    format!(
        "Nada foi removido: sem terminal.\n{}",
        action("use --yes para remover todos os listados")
    )
}

/// The user picked no record.
pub const NONE_REMOVED: &str = "Nenhum registro removido.";

/// One piece of third-party code an operation runs, as the confirmation shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThirdParty {
    /// A manifest script; `hook` names it (`post_install`, `pre_uninstall da versão instalada`).
    Script {
        hook: String,
        script: String,
        report: ScriptReport,
    },
    /// An `installer.file`/`uninstaller.file` run directly (no PowerShell).
    Program {
        uninstaller: bool,
        file: String,
        args: Vec<String>,
    },
    /// A `.ps1` installer/uninstaller file: its content is not in the manifest.
    Ps1 {
        uninstaller: bool,
        file: String,
        args: Vec<String>,
        report: ScriptReport,
    },
    /// The plan says its manifest has uninstall code, but the manifest could not be read back.
    Unknown { what: String },
}

impl ThirdParty {
    /// Short name for the refusal message (`script post_install`, `instalador setup.exe`).
    pub fn label(&self) -> String {
        match self {
            ThirdParty::Script { hook, .. } => format!("script {}", printable(hook)),
            ThirdParty::Program {
                uninstaller, file, ..
            }
            | ThirdParty::Ps1 {
                uninstaller, file, ..
            } => {
                let kind = if *uninstaller {
                    "desinstalador"
                } else {
                    "instalador"
                };
                format!("{kind} {}", printable(file))
            }
            ThirdParty::Unknown { what } => what.clone(),
        }
    }
}

/// A manifest script, then each file it runs that `roots` can read (Task 192): the user
/// sees what runs, and the script's report already carries their flags.
fn script_items(hook: String, script: &str, roots: &CallRoots) -> Vec<ThirdParty> {
    let (report, called) = script_scan::scan_called(script, PRELUDE_FUNCTIONS, roots);
    let files = called.into_iter().map(|c| ThirdParty::Script {
        hook: format!("arquivo chamado por {hook}: {}", c.target),
        script: c.script,
        report: c.report,
    });
    let first = ThirdParty::Script {
        hook: hook.clone(),
        script: script.to_string(),
        report,
    };
    std::iter::once(first).chain(files).collect()
}

fn ps1_item(uninstaller: bool, file: &str, args: &[String]) -> ThirdParty {
    ThirdParty::Ps1 {
        uninstaller,
        file: file.to_string(),
        args: args.to_vec(),
        report: script_scan::ps1_installer_report(file),
    }
}

fn program_item(uninstaller: bool, file: &str, args: &[String]) -> ThirdParty {
    ThirdParty::Program {
        uninstaller,
        file: file.to_string(),
        args: args.to_vec(),
    }
}

/// An `uninstaller.file`: the executor runs a `.ps1` through the script host and anything
/// else directly (for installers the planner's step kind already decided).
fn uninstaller_item(file: &str, args: &[String]) -> ThirdParty {
    if file.to_lowercase().ends_with(".ps1") {
        ps1_item(true, file, args)
    } else {
        program_item(true, file, args)
    }
}

/// The third-party code removing an app of manifest `m` (installed in `vdir`) runs, in the
/// executor's order (`pre_uninstall`, the uninstaller file, `uninstaller.script`,
/// `post_uninstall`); the uninstaller comes from `planner::uninstaller_command`, the same call
/// the executor makes (an `args`-only uninstaller runs the first download). `suffix` is
/// appended to each script name (`, na remoção`). `roots`: where the files the scripts run
/// can be read (`CallRoots::new(root, Some(vdir))`).
pub fn removal_third_party(
    m: &Resolved,
    vdir: &Path,
    suffix: &str,
    roots: &CallRoots,
) -> Vec<ThirdParty> {
    let name = |k: HookKind| format!("{}{suffix}", k.key());
    let mut out = Vec::new();
    if let Some(s) = &m.hooks.pre_uninstall {
        out.extend(script_items(name(HookKind::PreUninstall), s, roots));
    }
    if let Some(u) = &m.uninstaller {
        match planner::uninstaller_command(m, u, vdir) {
            Ok(Some((file, args))) => out.push(uninstaller_item(&file, &args)),
            Ok(None) => {}
            // The removal refuses it before any step; the user still sees it is there.
            Err(e) => out.push(ThirdParty::Unknown {
                what: format!("desinstalador do manifesto ({e})"),
            }),
        }
        if let Some(s) = &u.script {
            out.extend(script_items(name(HookKind::UninstallerScript), s, roots));
        }
    }
    if let Some(s) = &m.hooks.post_uninstall {
        out.extend(script_items(name(HookKind::PostUninstall), s, roots));
    }
    out
}

/// The third-party code `plan` runs now, then the one its manifest runs at removal time
/// (installing it already needs the consent: Tasks 151 and 159).
pub fn plan_third_party(plan: &InstallPlan) -> Vec<ThirdParty> {
    let mut out = Vec::new();
    let layout = Layout::new(&plan.root);
    // The same roots the planner scanned with: nothing of the new version is extracted yet,
    // the old one (an update) is.
    let roots = CallRoots::new(&plan.root, None);
    let old_dir = plan
        .upgrade_from
        .as_deref()
        .map(|v| layout.version_dir(&plan.app, v));
    let old_roots = CallRoots::new(&plan.root, old_dir.as_deref());
    for step in &plan.steps {
        match step {
            Step::Hook { kind, script } => {
                out.extend(script_items(kind.key().to_string(), script, &roots));
            }
            Step::UpgradeUninstallHook { kind, script } => {
                let hook = format!("{} da versão instalada", kind.key());
                out.extend(script_items(hook, script, &old_roots));
            }
            Step::RunInstaller { file, args, .. } => out.push(program_item(false, file, args)),
            Step::RunPs1Installer { file, args } => out.push(ps1_item(false, file, args)),
            Step::UpgradeRunUninstaller { file, args } => out.push(uninstaller_item(file, args)),
            _ => {}
        }
    }
    if plan.third_party_uninstaller || plan.has_uninstall_scripts {
        // The same text and architecture `ops::plan_for` resolved.
        let later = plan
            .steps
            .iter()
            .find_map(|s| match s {
                Step::WriteManifest { json } => manifest::resolve(json, Arch::host()).ok(),
                _ => None,
            })
            .map(|m| {
                let vdir = layout.version_dir(&plan.app, &plan.version);
                let roots = CallRoots::new(&plan.root, Some(&vdir));
                removal_third_party(&m, &vdir, ", na remoção", &roots)
            })
            .unwrap_or_default();
        if later.is_empty() {
            out.push(ThirdParty::Unknown {
                what: "desinstalador do manifesto".into(),
            });
        }
        out.extend(later);
    }
    out
}

const PRIVILEGES: &str = "roda com os seus privilégios e pode alterar qualquer arquivo seu";

fn verdict_text(v: Verdict) -> &'static str {
    match v {
        Verdict::Confirm => "pede confirmação",
        Verdict::ConfirmStrong => "pede atenção redobrada; revise antes de permitir",
        Verdict::Refuse => "recusado",
    }
}

fn push_report(s: &mut String, r: &ScriptReport) {
    s.push_str(&format!("  Análise: {}\n", verdict_text(r.verdict)));
    for f in &r.flags {
        let flag = format!("  ! {} (linha {})", printable(&f.detail), f.line);
        s.push_str(&paint(WARN, &flag));
        s.push('\n');
    }
}

fn push_title(s: &mut String, title: &str) {
    s.push_str(&paint(HEADER, title));
    s.push('\n');
}

fn command_line(file: &str, args: &[String]) -> String {
    std::iter::once(file)
        .chain(args.iter().map(String::as_str))
        .map(printable)
        .collect::<Vec<_>>()
        .join(" ")
}

fn program_title(uninstaller: bool) -> &'static str {
    if uninstaller {
        "Desinstalador de terceiro"
    } else {
        "Instalador de terceiro"
    }
}

/// What the user permits with `--allow-scripts`: per script the header, the gate's verdict
/// and flags, and the script numbered; per installer its command line.
pub fn third_party_block(items: &[ThirdParty]) -> String {
    let mut s = String::new();
    for item in items {
        match item {
            ThirdParty::Script {
                hook,
                script,
                report,
            } => {
                let title = format!("Script de terceiro ({}), {PRIVILEGES}", printable(hook));
                push_title(&mut s, &title);
                push_report(&mut s, report);
                let width = script.lines().count().max(1).to_string().len();
                for (i, line) in script.lines().enumerate() {
                    let n = format!("{:>width$}", i + 1);
                    s.push_str(&format!("  {} | {}\n", paint(DIM, &n), printable(line)));
                }
            }
            ThirdParty::Program {
                uninstaller,
                file,
                args,
            } => {
                let title = format!(
                    "{}: {}",
                    program_title(*uninstaller),
                    command_line(file, args)
                );
                push_title(&mut s, &title);
                s.push_str(&format!("  {PRIVILEGES}\n"));
            }
            ThirdParty::Ps1 {
                uninstaller,
                file,
                args,
                report,
            } => {
                let title = format!(
                    "{}: {}",
                    program_title(*uninstaller),
                    command_line(file, args)
                );
                push_title(&mut s, &title);
                s.push_str(&format!("  {PRIVILEGES}\n"));
                push_report(&mut s, report);
            }
            ThirdParty::Unknown { what } => {
                push_title(&mut s, &format!("Código de terceiro: {what}"));
                s.push_str(&format!("  {PRIVILEGES}\n"));
            }
        }
    }
    s
}

/// The refusal without `--allow-scripts` (even with `--yes`).
pub fn third_party_refusal<'a>(
    apps: &[&str],
    items: impl Iterator<Item = &'a ThirdParty>,
) -> Advice {
    let what: Vec<String> = items.map(ThirdParty::label).collect();
    Advice::new(
        format!(
            "{} roda código de terceiros ({})",
            apps.join(", "),
            what.join(", ")
        ),
        "revise com --dry-run e rode de novo com --allow-scripts",
    )
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
        "{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n",
        h("Reconstruídos"),
        list(&r.rebuilt),
        h("Mantidos"),
        list(&r.kept),
        h("Registros removidos (app sumiu do disco)"),
        list(&r.removed),
        h("Pulados (sem install.json legível)"),
        list(&r.skipped),
        h("Pastas apagadas (só sobras de instalação interrompida)"),
        list(&r.cleaned)
    )
}

fn mib(n: u64) -> String {
    format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0))
}

/// Window of the average download speed.
const SPEED_WINDOW: Duration = Duration::from_secs(3);
/// Download silence after which the line says so.
pub const STALL_AFTER: Duration = Duration::from_secs(10);
/// Log silence of the Windows Installer after which the line warns.
pub const INSTALLER_IDLE: Duration = Duration::from_secs(60);

/// Time after which a native installer or uninstaller (no log to follow) is said to maybe wait
/// for an answer in a window.
pub const NATIVE_QUIET: Duration = Duration::from_secs(300);
/// Windows Installer time after which [`INSTALLER_NOTICE`] is printed once.
pub const INSTALLER_NOTICE_AFTER: Duration = Duration::from_secs(30);
/// Fixed line printed above the animation once the Windows Installer passes 30 s.
pub const INSTALLER_NOTICE: &str =
    "O Windows Installer pode levar vários minutos em pacotes grandes; não feche o terminal.";
/// Columns the progress line is made to fit (the current file is cut from the left).
pub const LINE_COLUMNS: usize = 100;

/// What the Windows Installer log said at the last look.
struct LogWatch {
    size: u64,
    /// When `size` last changed (or was first seen).
    changed_at: Instant,
    files: u64,
    estimated: bool,
    last_file: Option<String>,
}

/// `18.304`: thousands separated by dots.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// `text` cut from the left to `room` chars, with `…` in front when cut.
fn fit_left(text: &str, room: usize) -> String {
    let n = text.chars().count();
    if n <= room {
        return text.to_string();
    }
    let keep = room.saturating_sub(1);
    let tail: String = text.chars().skip(n - keep).collect();
    format!("…{tail}")
}

/// `~6 s` or `~3 min`.
fn eta(secs: f64) -> String {
    let s = secs.round() as u64;
    if s < 60 {
        format!("~{s} s")
    } else {
        format!("~{} min", s.div_ceil(60))
    }
}

/// Phase of an install as the CLI shows it on one stderr line: the phase with its clock,
/// the download bar (summed over all URLs) with speed and estimate, or the Windows Installer
/// with what its log says. Pure state with an injected clock; the caller draws.
#[derive(Default)]
pub struct Phases {
    per_url: std::collections::BTreeMap<String, (u64, Option<u64>)>,
    /// The phase and when it started; `None` while downloads are the latest news.
    phase: Option<(Phase, Instant)>,
    /// `(when, summed bytes)` of recent download reports, plus one anchor before the window.
    samples: VecDeque<(Instant, u64)>,
    /// When the summed bytes last grew (or the first report).
    last_data: Option<Instant>,
    log: Option<LogWatch>,
    notice_shown: bool,
}

impl Phases {
    pub fn new() -> Phases {
        Phases::default()
    }

    /// [`Phases::update_at`] now.
    pub fn update(&mut self, ev: &ProgressEvent) {
        self.update_at(ev, Instant::now());
    }

    /// One progress event at `now`. The same phase again keeps its clock.
    pub fn update_at(&mut self, ev: &ProgressEvent, now: Instant) {
        match ev {
            ProgressEvent::Download { url, done, total } => {
                self.phase = None;
                self.per_url.insert(url.clone(), (*done, *total));
                let sum = self
                    .per_url
                    .values()
                    .fold(0u64, |a, (d, _)| a.saturating_add(*d));
                if self.samples.back().is_none_or(|(_, last)| sum > *last) {
                    self.last_data = Some(now);
                }
                self.samples.push_back((now, sum));
                while self.samples.len() >= 2 && self.samples[1].0 + SPEED_WINDOW <= now {
                    self.samples.pop_front();
                }
            }
            ProgressEvent::Phase(p) => {
                if self.phase.as_ref().is_some_and(|(cur, _)| cur == p) {
                    return;
                }
                self.phase = Some((p.clone(), now));
                self.log = None;
            }
        }
    }

    /// The Windows Installer log to look at and the size seen last time (`0` the first time),
    /// or `None` outside that phase.
    pub fn log_to_read(&self) -> Option<(std::path::PathBuf, u64)> {
        match &self.phase {
            Some((Phase::Installer { log }, _)) => {
                Some((log.clone(), self.log.as_ref().map_or(0, |l| l.size)))
            }
            Some((
                Phase::Preparing
                | Phase::WaitingInstaller
                | Phase::Extracting { .. }
                | Phase::Placing
                | Phase::Linking
                | Phase::Recording
                | Phase::Removing { .. }
                | Phase::Scanning
                | Phase::WaitingElevation
                | Phase::RunningInstaller { .. }
                | Phase::Verifying
                | Phase::CopyingFolders
                | Phase::Uninstalling { .. }
                | Phase::CheckingRemoval { .. }
                | Phase::SelfElevating { .. }
                | Phase::Bridge { .. }
                | Phase::RunningScript { .. }
                | Phase::RunningPackageInstaller { .. },
                _,
            ))
            | None => None,
        }
    }

    /// A look at `log` at `now`; `None` (a read error) leaves only the phase and the clock.
    /// Ignored when the phase moved on to another log meanwhile (a relaunch).
    pub fn log_read_at(&mut self, log: &std::path::Path, tail: Option<LogTail>, now: Instant) {
        if self.log_to_read().is_none_or(|(cur, _)| cur != log) {
            return;
        }
        let Some(t) = tail else {
            self.log = None;
            return;
        };
        match &mut self.log {
            Some(w) => {
                if t.size != w.size {
                    w.changed_at = now;
                    w.size = t.size;
                }
                w.files = w.files.saturating_add(t.new_files);
                w.estimated |= t.estimated;
                if t.last_file.is_some() {
                    w.last_file = t.last_file;
                }
            }
            None => {
                self.log = Some(LogWatch {
                    size: t.size,
                    changed_at: now,
                    files: t.new_files,
                    estimated: t.estimated,
                    last_file: t.last_file,
                });
            }
        }
    }

    /// [`INSTALLER_NOTICE`] the first time the Windows Installer passes 30 s, else `None`.
    pub fn take_notice_at(&mut self, now: Instant) -> Option<&'static str> {
        let since = match &self.phase {
            Some((Phase::Installer { .. }, since)) => *since,
            Some((
                Phase::Preparing
                | Phase::WaitingInstaller
                | Phase::Extracting { .. }
                | Phase::Placing
                | Phase::Linking
                | Phase::Recording
                | Phase::Removing { .. }
                | Phase::Scanning
                | Phase::WaitingElevation
                | Phase::RunningInstaller { .. }
                | Phase::Verifying
                | Phase::CopyingFolders
                | Phase::Uninstalling { .. }
                | Phase::CheckingRemoval { .. }
                | Phase::SelfElevating { .. }
                | Phase::Bridge { .. }
                | Phase::RunningScript { .. }
                | Phase::RunningPackageInstaller { .. },
                _,
            ))
            | None => return None,
        };
        if self.notice_shown || now.saturating_duration_since(since) < INSTALLER_NOTICE_AFTER {
            return None;
        }
        self.notice_shown = true;
        Some(INSTALLER_NOTICE)
    }

    /// [`Phases::line_at`] now.
    pub fn line(&self, frame: usize, glyphs: &crate::tui::theme::Glyphs) -> Option<String> {
        self.line_at(frame, glyphs, Instant::now())
    }

    /// The line for spinner frame `frame` at `now`, or `None` while there is nothing to show.
    pub fn line_at(
        &self,
        frame: usize,
        glyphs: &crate::tui::theme::Glyphs,
        now: Instant,
    ) -> Option<String> {
        self.parts_at(frame, glyphs, now)
            .map(|(main, warn)| main + &warn)
    }

    /// The line split in its plain part and its warning (painted by the caller; may be empty).
    pub fn parts_at(
        &self,
        frame: usize,
        glyphs: &crate::tui::theme::Glyphs,
        now: Instant,
    ) -> Option<(String, String)> {
        let spin = glyphs.spinner.get(frame % glyphs.spinner.len().max(1))?;
        if let Some((phase, since)) = &self.phase {
            let elapsed = now.saturating_duration_since(*since);
            let mut main = format!("{spin} {}", phase_text(phase, elapsed));
            let mut warn = String::new();
            match phase {
                Phase::Installer { .. } => match &self.log {
                    None => main = format!("{spin} {}", phase_status(phase, elapsed)),
                    Some(w) => {
                        if w.files > 0 {
                            let approx = if w.estimated { "~" } else { "" };
                            main.push_str(&format!(" · {approx}{} arquivos", thousands(w.files)));
                        }
                        let idle = now.saturating_duration_since(w.changed_at);
                        if idle >= INSTALLER_IDLE {
                            warn = format!(
                                " · sem atividade há {} min; o Windows Installer pode estar \
                                 esperando outro instalador",
                                idle.as_secs() / 60
                            );
                        } else if let Some(f) = &w.last_file {
                            let room = LINE_COLUMNS.saturating_sub(main.chars().count() + 3);
                            main.push_str(&format!(" · {}", fit_left(f, room)));
                        }
                    }
                },
                Phase::Preparing
                | Phase::WaitingInstaller
                | Phase::Extracting { .. }
                | Phase::Placing
                | Phase::Linking
                | Phase::Recording
                | Phase::Removing { .. }
                | Phase::Scanning
                | Phase::Verifying
                | Phase::CopyingFolders => {}
                // The native installer and uninstaller cannot be cancelled either; they have
                // no log to follow: past `NATIVE_QUIET` the line says a window may be waiting.
                Phase::RunningInstaller { .. }
                | Phase::WaitingElevation
                | Phase::Uninstalling { .. }
                | Phase::CheckingRemoval { .. }
                | Phase::SelfElevating { .. }
                | Phase::Bridge { .. }
                | Phase::RunningScript { .. }
                | Phase::RunningPackageInstaller { .. } => {
                    main = format!("{spin} {}", phase_status(phase, elapsed));
                    if elapsed >= NATIVE_QUIET && !matches!(phase, Phase::CheckingRemoval { .. }) {
                        warn = format!(
                            " · rodando há {} min; pode haver uma janela esperando resposta",
                            elapsed.as_secs() / 60
                        );
                    }
                }
            }
            return Some((main, warn));
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
            return Some((format!("{spin} Instalando…"), String::new()));
        }
        let total = known.then_some(total).filter(|t| *t > 0);
        let mut main = match total {
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
        };
        if let Some(rate) = self.speed_at(now) {
            main.push_str(&format!(" · {}/s", mib(rate as u64)));
            if let Some(t) = total {
                main.push_str(&format!(" · {}", eta(t.saturating_sub(done) as f64 / rate)));
            }
        }
        let mut warn = String::new();
        if let Some(at) = self.last_data {
            let quiet = now.saturating_duration_since(at);
            if quiet >= STALL_AFTER {
                warn = format!(" · sem dados há {} s", quiet.as_secs());
            }
        }
        Some((main, warn))
    }

    /// Average bytes per second over the last ~3 s, or `None` without a measurable rise.
    fn speed_at(&self, now: Instant) -> Option<f64> {
        let (_, latest) = *self.samples.back()?;
        let anchor = self
            .samples
            .iter()
            .rev()
            .find(|(t, _)| *t + SPEED_WINDOW <= now)
            .or(self.samples.front())?;
        let secs = now.saturating_duration_since(anchor.0).as_secs_f64();
        let rate = latest.saturating_sub(anchor.1) as f64 / secs;
        (secs >= 0.5 && rate > 0.0).then_some(rate)
    }
}

/// Runs a bridge job (`run` gets the tool's line sink) with the CLI's progress: `Bridge { tool }`
/// from the first instant (stderr), then each output line through `progress::bridge_line`:
/// the elevation notice becomes the phase, with its clock; the tool's own lines are the
/// command's result and go to `output` (stdout). The caller ends the progress line afterwards.
pub fn bridge_progress<R>(
    progress: &dyn Fn(&ProgressEvent),
    output: &dyn Fn(&str),
    tool: &str,
    run: impl FnOnce(&mut dyn FnMut(&str)) -> R,
) -> R {
    let mut current = Phase::Bridge {
        tool: tool.to_string(),
    };
    progress(&ProgressEvent::Phase(current.clone()));
    run(&mut |line: &str| {
        let (next, show) = crate::install::progress::bridge_line(line, Some(&current), tool);
        if let Some(p) = next {
            current = p;
            progress(&ProgressEvent::Phase(current.clone()));
        }
        if show {
            output(&format!("  {line}"));
        }
    })
}

/// Progress for a stderr that is not a terminal: one plain line per phase change (no `\r`,
/// no color), so a redirected log shows how far the install got.
#[derive(Default)]
pub struct PhaseLog {
    /// Kind of the last line: `Some(None)` = downloading.
    last: Option<Option<Phase>>,
}

impl PhaseLog {
    pub fn new() -> PhaseLog {
        PhaseLog::default()
    }

    /// The line to print for `ev`, or `None` while it is still the same phase.
    pub fn line(&mut self, ev: &ProgressEvent) -> Option<String> {
        let (kind, text) = match ev {
            ProgressEvent::Download { .. } => (None, "Baixando".to_string()),
            ProgressEvent::Phase(p) => (Some(p.clone()), phase_title(p)),
        };
        if self.last.as_ref() == Some(&kind) {
            return None;
        }
        self.last = Some(kind);
        Some(format!("{text}…"))
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
