//! Text rendering for CLI commands; colored through `style::paint` when color is on.
use super::style::{
    DIM, ERROR, HEADER, ID, NAME, NEW_VERSION, OK, SOURCE, Style, VERSION, WARN, paint,
};
use crate::core::{PackageDetails, PackageEntry};
use crate::install::manifest::{self, Arch, HookKind, Resolved};
use crate::install::msiexec_log::LogTail;
use crate::install::plan::{InstallPlan, Step};
use crate::install::prelude::PRELUDE_FUNCTIONS;
use crate::install::progress::{Phase, ProgressEvent, phase_status, phase_text, phase_title};
use crate::install::script_scan::{self, ScriptReport, Verdict};
use crate::installed::correlate::{Correlation, ListRow, UpdateState};
use crate::update::{Outcome, SourceReport};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

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

fn script_item(hook: String, script: &str) -> ThirdParty {
    ThirdParty::Script {
        hook,
        script: script.to_string(),
        report: script_scan::scan(script, PRELUDE_FUNCTIONS),
    }
}

fn file_item(uninstaller: bool, file: &str, args: &[String]) -> ThirdParty {
    if file.to_lowercase().ends_with(".ps1") {
        ThirdParty::Ps1 {
            uninstaller,
            file: file.to_string(),
            args: args.to_vec(),
            report: script_scan::ps1_installer_report(file),
        }
    } else {
        ThirdParty::Program {
            uninstaller,
            file: file.to_string(),
            args: args.to_vec(),
        }
    }
}

/// The third-party code removing an app of manifest `m` runs, in the executor's order;
/// `suffix` is appended to each script name (`, na remoção`).
pub fn removal_third_party(m: &Resolved, suffix: &str) -> Vec<ThirdParty> {
    let name = |k: HookKind| format!("{}{suffix}", k.key());
    let mut out = Vec::new();
    if let Some(s) = &m.hooks.pre_uninstall {
        out.push(script_item(name(HookKind::PreUninstall), s));
    }
    if let Some(u) = &m.uninstaller {
        if let Some(s) = &u.script {
            out.push(script_item(name(HookKind::UninstallerScript), s));
        }
        if let Some(f) = &u.file {
            out.push(file_item(true, f, &u.args));
        }
    }
    if let Some(s) = &m.hooks.post_uninstall {
        out.push(script_item(name(HookKind::PostUninstall), s));
    }
    out
}

/// The third-party code `plan` runs now, then the one its manifest runs at removal time
/// (installing it already needs the consent: Tasks 151 and 159).
pub fn plan_third_party(plan: &InstallPlan) -> Vec<ThirdParty> {
    let mut out = Vec::new();
    for step in &plan.steps {
        match step {
            Step::Hook { kind, script } => out.push(script_item(kind.key().to_string(), script)),
            Step::UpgradeUninstallHook { kind, script } => {
                let hook = format!("{} da versão instalada", kind.key());
                out.push(script_item(hook, script));
            }
            Step::RunInstaller { file, args, .. } | Step::RunPs1Installer { file, args } => {
                out.push(file_item(false, file, args));
            }
            Step::UpgradeRunUninstaller { file, args } => out.push(file_item(true, file, args)),
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
            .map(|m| removal_third_party(&m, ", na remoção"))
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

/// Third-party text made safe for the terminal: control characters (escape sequences
/// included) are replaced; tabs are kept.
fn printable(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\t' {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
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
) -> String {
    let what: Vec<String> = items.map(ThirdParty::label).collect();
    format!(
        "{} roda código de terceiros ({}); revise com --dry-run e rode de novo com --allow-scripts",
        apps.join(", "),
        what.join(", ")
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
                | Phase::Verifying,
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
                | Phase::Verifying,
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
                | Phase::Verifying => {}
                // The native installer cannot be cancelled either; it has no log to follow.
                Phase::RunningInstaller { .. } | Phase::WaitingElevation => {
                    main = format!("{spin} {}", phase_status(phase, elapsed));
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
