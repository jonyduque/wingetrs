//! `install`, `uninstall`, `upgrade` and `repair`: plan, show, confirm, execute.
//! Platform-neutral: every real effect arrives through `OpsCtx`, every prompt through `Ui`.
use super::{Command, output};
use crate::install::ops::{self, OpsCtx, Resolution};
use crate::install::residue::{self, Confidence, Residue};
use crate::install::target::Candidate;
use crate::tui::picker::PickerItem;
use anyhow::bail;
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Everything the commands ask of the user. Production: [`TtyUi`].
pub trait Ui {
    /// Both stdin and stdout are terminals.
    fn interactive(&self) -> bool;
    fn confirm(&self, question: &str) -> anyhow::Result<bool>;
    fn pick(&self, items: Vec<PickerItem>) -> anyhow::Result<Option<usize>>;
    fn pick_many(
        &self,
        items: Vec<PickerItem>,
        initial: &[bool],
    ) -> anyhow::Result<Option<Vec<usize>>>;
}

pub struct TtyUi {
    theme: crate::tui::theme::Theme,
    glyphs: &'static crate::tui::theme::Glyphs,
}

impl TtyUi {
    pub fn new(cfg: &crate::config::Config) -> TtyUi {
        use crate::tui::theme::{GlyphSet, Glyphs, Theme, detect_depth};
        let set = match cfg.glyphs.as_str() {
            "unicode" => GlyphSet::Unicode,
            "ascii" => GlyphSet::Ascii,
            _ => GlyphSet::Nerd,
        };
        TtyUi {
            theme: Theme::new(
                cfg.theme != "light",
                detect_depth(
                    std::env::var("COLORTERM").ok().as_deref(),
                    std::env::var("NO_COLOR").ok().as_deref(),
                ),
            ),
            glyphs: Glyphs::for_set(set),
        }
    }
}

/// `s`, `sim`, `y`, `yes` (any case) confirm; anything else, including empty, declines.
fn is_yes(answer: &str) -> bool {
    matches!(
        answer.trim().to_lowercase().as_str(),
        "s" | "sim" | "y" | "yes"
    )
}

impl Ui for TtyUi {
    fn interactive(&self) -> bool {
        super::picker::is_interactive()
    }

    fn confirm(&self, question: &str) -> anyhow::Result<bool> {
        print!("{question} [s/N] ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        Ok(is_yes(&line))
    }

    fn pick(&self, items: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        Ok(super::picker::pick_inline(items, &self.theme, self.glyphs)?)
    }

    fn pick_many(
        &self,
        items: Vec<PickerItem>,
        initial: &[bool],
    ) -> anyhow::Result<Option<Vec<usize>>> {
        Ok(super::picker::pick_inline_multi(
            items,
            initial,
            &self.theme,
            self.glyphs,
        )?)
    }
}

/// Aggregated one-line download progress on stderr; a no-op unless `enabled` (a TTY).
pub struct LineProgress {
    enabled: bool,
    drawn: AtomicBool,
    per_url: Mutex<BTreeMap<String, (u64, Option<u64>)>>,
}

impl LineProgress {
    pub fn new(enabled: bool) -> LineProgress {
        LineProgress {
            enabled,
            drawn: AtomicBool::new(false),
            per_url: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn update(&self, url: &str, done: u64, total: Option<u64>) {
        if !self.enabled {
            return;
        }
        let (sum, all_known, sum_total) = {
            let mut map = self.per_url.lock().unwrap_or_else(|p| p.into_inner());
            map.insert(url.to_string(), (done, total));
            let sum: u64 = map.values().map(|(d, _)| d).sum();
            let known = map.values().all(|(_, t)| t.is_some());
            let tot: u64 = map.values().filter_map(|(_, t)| *t).sum();
            (sum, known, tot)
        };
        let line = output::progress_line(sum, all_known.then_some(sum_total));
        let mut err = std::io::stderr().lock();
        // Progress is cosmetic: a broken stderr must not fail the install.
        let _ = write!(err, "\r{line}");
        let _ = err.flush();
        self.drawn.store(true, Ordering::Relaxed);
    }

    /// Erases the progress line.
    pub fn finish(&self) {
        if self.drawn.swap(false, Ordering::Relaxed) {
            let mut err = std::io::stderr().lock();
            let _ = write!(err, "\r{:<60}\r", "");
            let _ = err.flush();
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Flags {
    yes: bool,
    dry_run: bool,
}

/// Asks to proceed: `--yes` skips, no terminal without `--yes` is an error.
fn proceed(ui: &dyn Ui, f: Flags) -> anyhow::Result<bool> {
    if f.yes {
        return Ok(true);
    }
    if !ui.interactive() {
        bail!("a confirmação exige um terminal interativo; use --yes");
    }
    ui.confirm("Continuar?")
}

/// Runs one of the operation commands.
pub async fn execute(ctx: &OpsCtx<'_>, ui: &dyn Ui, cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::Install {
            target,
            bucket,
            dry_run,
            yes,
        } => install(ctx, ui, &target, bucket.as_deref(), Flags { yes, dry_run }).await,
        Command::Uninstall {
            target,
            purge,
            dry_run,
            yes,
        } => uninstall(ctx, ui, &target, purge, Flags { yes, dry_run }),
        Command::Upgrade {
            target,
            all,
            keep,
            dry_run,
            yes,
        } => {
            upgrade(
                ctx,
                ui,
                target.as_deref(),
                all,
                keep,
                Flags { yes, dry_run },
            )
            .await
        }
        Command::Repair => {
            print!("{}", output::repair_text(&ops::repair(ctx)?));
            Ok(())
        }
        _ => bail!("comando não é uma operação de instalação"),
    }
}

/// The candidate to act on: the only match, or the user's pick when a terminal allows it.
/// `Ok(None)` = the user cancelled.
fn choose(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    target: &str,
    bucket: Option<&str>,
    yes: bool,
) -> anyhow::Result<Option<Candidate>> {
    match ops::resolve_target(ctx, target, bucket)? {
        Resolution::One(c) => Ok(Some(c)),
        Resolution::NotFound { suggestions } => {
            bail!("{}", output::not_found_text(target, &suggestions))
        }
        Resolution::Many(mut found) => {
            // Never pick on the user's behalf: `--yes` and a missing terminal both fail.
            if yes || !ui.interactive() {
                let names: Vec<String> = found
                    .iter()
                    .map(|c| output::bucket_id(&c.entry.source_name, &c.entry.id))
                    .collect();
                bail!("{}", output::ambiguous_text(target, &names));
            }
            let items = found
                .iter()
                .map(|c| PickerItem {
                    label: output::bucket_id(&c.entry.source_name, &c.entry.id),
                    detail: c.entry.version.clone(),
                    selectable: c.selectable,
                    note: c.note.clone(),
                })
                .collect();
            Ok(ui
                .pick(items)?
                .and_then(|i| (i < found.len()).then(|| found.swap_remove(i))))
        }
    }
}

fn print_plans(plans: &[crate::install::plan::InstallPlan]) {
    for p in plans {
        println!("{p}");
    }
}

async fn install(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    target: &str,
    bucket: Option<&str>,
    f: Flags,
) -> anyhow::Result<()> {
    let Some(cand) = choose(ctx, ui, target, bucket, f.yes)? else {
        println!("Cancelado.");
        return Ok(());
    };
    let plans = ops::plan_for(ctx, &cand, false)?;
    print_plans(&plans);
    if f.dry_run {
        println!("Simulação: nada foi alterado.");
        return Ok(());
    }
    if !proceed(ui, f)? {
        println!("Cancelado.");
        return Ok(());
    }
    ops::apply(ctx, &plans).await?;
    for p in &plans {
        println!("Instalado: {} {}", p.app, p.version);
    }
    Ok(())
}

fn uninstall(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    target: &str,
    purge: bool,
    f: Flags,
) -> anyhow::Result<()> {
    // Installed apps are named by id only; accept `bucket/id` too.
    let app = target.rsplit('/').next().unwrap_or(target);
    let plan = ops::plan_uninstall(ctx, app, purge)?;
    print!(
        "{}",
        output::uninstall_plan_text(&plan, &ctx.data.install_layout())
    );
    if f.dry_run {
        println!("Simulação: nada foi alterado.");
        return Ok(());
    }
    if !proceed(ui, f)? {
        println!("Cancelado.");
        return Ok(());
    }
    let residues = ops::uninstall(ctx, &plan)?;
    println!("Desinstalado: {} {}", plan.app, plan.version);
    clean_residues(ctx, ui, &plan.app, &residues, f.yes)
}

fn clean_residues(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    app: &str,
    residues: &[Residue],
    yes: bool,
) -> anyhow::Result<()> {
    if residues.is_empty() {
        return Ok(());
    }
    let selected: Vec<Residue> = if yes {
        // Only what the classifier is sure about and pre-selected itself.
        residues
            .iter()
            .filter(|r| r.confidence == Confidence::High && r.preselected)
            .cloned()
            .collect()
    } else if ui.interactive() {
        let items = residues
            .iter()
            .map(|r| PickerItem {
                label: output::residue_label(r),
                detail: match r.confidence {
                    Confidence::High => "confiança alta".into(),
                    Confidence::Low => "confiança baixa".into(),
                },
                selectable: true,
                note: r.note.clone(),
            })
            .collect();
        let initial: Vec<bool> = residues.iter().map(|r| r.preselected).collect();
        match ui.pick_many(items, &initial)? {
            Some(idx) => idx
                .into_iter()
                .filter_map(|i| residues.get(i).cloned())
                .collect(),
            None => Vec::new(),
        }
    } else {
        println!("Resíduos encontrados (nada foi removido; use --yes para os de confiança alta):");
        for r in residues {
            println!("  {}", output::residue_label(r));
        }
        return Ok(());
    };
    if selected.is_empty() {
        println!("Nenhum resíduo removido.");
        return Ok(());
    }
    let backups = ctx.data.install_layout().backups();
    let results = residue::clean(&selected, app, &ctx.effects, &backups);
    for (r, res) in selected.iter().zip(results) {
        match res {
            Ok(()) => println!("Removido: {}", output::residue_label(r)),
            Err(e) => eprintln!("aviso: {e}"),
        }
    }
    Ok(())
}

async fn upgrade(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    target: Option<&str>,
    all: bool,
    keep: bool,
    f: Flags,
) -> anyhow::Result<()> {
    let list = ops::upgradable(ctx)?;
    let state = ops::read_state(&ctx.data.install_layout())?;
    let chosen: Vec<(String, String, String)> = match target {
        Some(t) => {
            let t = t.rsplit('/').next().unwrap_or(t);
            if let Some((app, reason)) =
                list.skipped.iter().find(|(a, _)| a.eq_ignore_ascii_case(t))
            {
                bail!("{app}: {reason}");
            }
            let found: Vec<_> = list
                .upgradable
                .into_iter()
                .filter(|(a, _, _)| a.eq_ignore_ascii_case(t))
                .collect();
            if found.is_empty() {
                if state.get(t)?.is_none() {
                    return Err(ops::OpsError::NotInstalled(t.to_string()).into());
                }
                println!("{t} já está na versão mais recente.");
                return Ok(());
            }
            found
        }
        None => {
            for (app, reason) in &list.skipped {
                eprintln!("aviso: {app}: {reason}");
            }
            if !all {
                print!("{}", output::upgrades_text(&list.upgradable));
                if list.upgradable.is_empty() {
                    println!("Nada para atualizar.");
                } else {
                    println!("Use `wingetrs upgrade --all` ou `wingetrs upgrade <app>`.");
                }
                return Ok(());
            }
            list.upgradable
        }
    };
    if chosen.is_empty() {
        println!("Nada para atualizar.");
        return Ok(());
    }
    let mut groups = Vec::new();
    for (app, _, _) in &chosen {
        let Some((rec, _)) = state.get(app)? else {
            bail!("{app} não está instalado pelo wingetrs");
        };
        let cand = match ops::resolve_installed(ctx, app, &rec.bucket)? {
            Resolution::One(c) => c,
            _ => bail!("{app} não foi encontrado no bucket {}", rec.bucket),
        };
        groups.push(ops::plan_for(ctx, &cand, keep)?);
    }
    for g in &groups {
        print_plans(g);
    }
    if f.dry_run {
        println!("Simulação: nada foi alterado.");
        return Ok(());
    }
    if !proceed(ui, f)? {
        println!("Cancelado.");
        return Ok(());
    }
    for p in ops::apply_groups(ctx, groups).await? {
        println!("Atualizado: {} {}", p.app, p.version);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoUi;
    impl Ui for NoUi {
        fn interactive(&self) -> bool {
            false
        }
        fn confirm(&self, _: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        fn pick(&self, _: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
            Ok(None)
        }
        fn pick_many(&self, _: Vec<PickerItem>, _: &[bool]) -> anyhow::Result<Option<Vec<usize>>> {
            Ok(None)
        }
    }

    fn no_progress(_: &str, _: u64, _: Option<u64>) {}
    fn nothing_in_use(_: &std::path::Path) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    #[test]
    fn yes_cleans_only_high_confidence_preselected_residues() {
        use crate::install::ops::ResidueScan;
        use crate::install::residue::ResidueKind;
        use crate::install::traits::Effects;
        use crate::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
        use std::path::PathBuf;
        let t = tempfile::tempdir().unwrap();
        let data = crate::cache::DataDir::new(t.path());
        let http = crate::http::Http::new().unwrap();
        let catalog = crate::catalog::Catalog::open_in_memory().unwrap();
        let (env, sc, trash, reg) = (
            FakeEnv::new(),
            FakeShortcuts::new(t.path().join("sm")),
            FakeTrash::new(),
            FakeRegistry::new(),
        );
        let ctx = OpsCtx {
            data: &data,
            http: &http,
            catalog: &catalog,
            effects: Effects {
                env: &env,
                shortcuts: &sc,
                trash: &trash,
                registry: &reg,
            },
            wait: crate::update::LockWait::NoWait,
            progress: &no_progress,
            in_use: &nothing_in_use,
            residue: ResidueScan::none(),
        };
        let res = |name: &str, confidence, preselected| Residue {
            kind: ResidueKind::Folder {
                path: PathBuf::from(name),
            },
            confidence,
            preselected,
            note: None,
        };
        let residues = [
            res("high-pre", Confidence::High, true),
            res("low-pre", Confidence::Low, true),
            res("high-not", Confidence::High, false),
        ];
        clean_residues(&ctx, &NoUi, "app", &residues, true).unwrap();
        assert_eq!(trash.dump(), vec![PathBuf::from("high-pre")]);
        // Without --yes and without a terminal nothing is removed.
        clean_residues(&ctx, &NoUi, "app", &residues, false).unwrap();
        assert_eq!(trash.dump().len(), 1);
    }

    #[test]
    fn only_explicit_yes_answers_confirm() {
        for a in ["s", "S\n", "sim", "y", "YES"] {
            assert!(is_yes(a), "{a}");
        }
        for a in ["", "\n", "n", "não", "talvez"] {
            assert!(!is_yes(a), "{a}");
        }
    }
}
