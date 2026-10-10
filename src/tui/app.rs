//! Elm-style TUI state: `update` is pure and performs no I/O; side effects are returned as `Cmd`.
use crate::catalog::search::{SearchHit, SearchIndex};
use crate::core::{PackageDetails, PackageEntry, SourceKind};
use crate::install::plan::{InstallPlan, UninstallPlan};
use crate::install::residue::{Confidence, Residue, ResidueKind};
use crate::install::target::Candidate;
use crate::installed::correlate::{Correlation, ListRow, UpdateState};
use crate::tui::picker::{PickOutcome, Picker, PickerItem};
use crate::tui::theme::GlyphSet;
use crate::update::{Outcome, SourceReport};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Search,
    Installed,
    Updates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFilter {
    All,
    Winget,
    Scoop,
}

#[allow(clippy::large_enum_variant)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick,
    CatalogLoaded(Vec<PackageEntry>),
    InstalledLoaded(Vec<ListRow>),
    /// Reading or correlating the installed apps failed (error or worker panic).
    InstalledFailed(String),
    DetailsLoaded {
        key: String,
        result: Result<PackageDetails, String>,
    },
    RefreshStarted,
    RefreshFinished(Vec<SourceReport>),
    /// Another process holds the update lock; not a failure and not offline.
    RefreshBusy,
    /// Result of planning an install, upgrade or removal.
    Planned(Result<Planned, String>),
    /// Download/extract progress of the running operation.
    InstallProgress {
        label: String,
        done: u64,
        total: Option<u64>,
    },
    /// The operation ended (also on failure, so the installed list is rescanned).
    InstallFinished(Result<Done, InstallFail>),
    /// Residue cleanup ended; one message per item that failed.
    ResiduesCleaned(Vec<String>),
}

/// What the user is asked to confirm, or to choose from.
#[derive(Debug, Clone, PartialEq)]
pub enum Planned {
    Ready {
        text: String,
        action: Action,
    },
    /// The text matched more than one package.
    Choose(Vec<Candidate>),
}

/// A confirmed operation, executed by the runtime under `install.lock`.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// One plan list per app (dependencies first), applied in order.
    Apply(Vec<Vec<InstallPlan>>),
    Remove(UninstallPlan),
}

impl Action {
    fn label(&self) -> String {
        match self {
            Action::Apply(groups) => {
                let apps: Vec<&str> = groups.iter().flatten().map(|p| p.app.as_str()).collect();
                format!("Instalando {}", apps.join(", "))
            }
            Action::Remove(p) => format!("Removendo {}", p.app),
        }
    }
}

/// Successful end of an operation.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub summary: String,
    /// `(app, residues)` left behind by a removal.
    pub residues: Option<(String, Vec<Residue>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InstallFail {
    /// Another wingetrs process holds `install.lock`.
    Busy,
    Other(String),
}

/// Download/extract progress shown while an operation runs.
#[derive(Debug, Clone, PartialEq)]
pub struct RunProgress {
    pub label: String,
    pub done: u64,
    pub total: Option<u64>,
}

/// The dialog on top of the screen; it takes every key until it closes.
pub enum Modal {
    Confirm {
        plan_text: String,
        action: Action,
    },
    Picking {
        picker: Picker,
        candidates: Vec<Candidate>,
    },
    Running {
        label: String,
        progress: Option<RunProgress>,
    },
    Residues {
        picker: Picker,
        app: String,
        residues: Vec<Residue>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Quit,
    Refresh {
        force: bool,
    },
    LoadDetails {
        source: String,
        id: String,
    },
    ScanInstalled,
    /// Plan installing (or upgrading) one catalog package.
    PlanInstall {
        source_name: String,
        id: String,
    },
    PlanRemove {
        app: String,
    },
    /// Plan upgrading every app with a newer catalog version.
    PlanUpgradeAll,
    Execute(Action),
    CleanResidues {
        app: String,
        residues: Vec<Residue>,
    },
}

pub struct App {
    pub tab: Tab,
    pub focus: Focus,
    pub filter: SourceFilter,
    pub query: String,
    pub selected: usize,
    pub scroll: usize,
    pub search: SearchIndex,
    pub hits: Vec<SearchHit>,
    pub installed: Vec<ListRow>,
    pub details: HashMap<String, Result<PackageDetails, String>>,
    pub refreshing: bool,
    pub offline: bool,
    pub index_age_secs: Option<i64>,
    pub spinner_frame: usize,
    pub show_help: bool,
    pub status: Option<String>,
    pub glyphs: GlyphSet,
    pub size: (u16, u16),
    pub modal: Option<Modal>,
}

const MAX_HITS: usize = 1000;
const PAGE: usize = 10;
/// Rows of screen taken by tabs, search box, status bar and borders.
const CHROME_ROWS: usize = 6;

impl App {
    pub fn new(entries: Vec<PackageEntry>, glyphs: GlyphSet) -> App {
        let mut app = App {
            tab: Tab::Search,
            focus: Focus::List,
            filter: SourceFilter::All,
            query: String::new(),
            selected: 0,
            scroll: 0,
            search: SearchIndex::new(entries),
            hits: Vec::new(),
            installed: Vec::new(),
            details: HashMap::new(),
            refreshing: false,
            offline: false,
            index_age_secs: None,
            spinner_frame: 0,
            show_help: false,
            status: None,
            glyphs,
            size: (80, 24),
            modal: None,
        };
        app.research();
        app
    }

    /// Installed rows visible in the Installed/Updates tabs after query and source filter.
    pub fn installed_view(&self) -> Vec<&ListRow> {
        let q = self.query.to_lowercase();
        self.installed
            .iter()
            .filter(|r| {
                self.tab != Tab::Updates || matches!(r.state, UpdateState::Available { .. })
            })
            .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q))
            .filter(|r| match (self.filter, &r.correlation) {
                (SourceFilter::All, _) => true,
                (SourceFilter::Winget, Correlation::Matched { source, .. }) => source == "winget",
                (SourceFilter::Scoop, Correlation::Matched { source, .. }) => {
                    source.starts_with("scoop")
                }
                _ => false,
            })
            .collect()
    }

    /// Rows visible in the current tab after filters.
    pub fn visible_len(&self) -> usize {
        match self.tab {
            Tab::Search => self.hits.len(),
            Tab::Installed | Tab::Updates => self.installed_view().len(),
        }
    }

    /// `(source_name, id)` of the selected row, when it maps to a catalog package.
    pub fn selected_key(&self) -> Option<(String, String)> {
        match self.tab {
            Tab::Search => {
                let hit = self.hits.get(self.selected)?;
                let e = self.search.entries().get(hit.index)?;
                Some((e.source_name.clone(), e.id.clone()))
            }
            Tab::Installed | Tab::Updates => {
                let rows = self.installed_view();
                match &rows.get(self.selected)?.correlation {
                    Correlation::Matched {
                        source, package_id, ..
                    } => Some((source.clone(), package_id.clone())),
                    _ => None,
                }
            }
        }
    }

    fn research(&mut self) {
        let kind = match self.filter {
            SourceFilter::All => None,
            SourceFilter::Winget => Some(SourceKind::Winget),
            SourceFilter::Scoop => Some(SourceKind::Scoop),
        };
        self.hits = self.search.search(&self.query, kind, MAX_HITS);
    }

    /// Recomputes the row set after query/filter/tab changes and resets the selection.
    fn refresh_view(&mut self) {
        if self.tab == Tab::Search {
            self.research();
        }
        self.selected = 0;
        self.scroll = 0;
    }

    fn clamp(&mut self) {
        let len = self.visible_len();
        self.selected = self.selected.min(len.saturating_sub(1));
        let page = usize::from(self.size.1).saturating_sub(CHROME_ROWS).max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + page {
            self.scroll = self.selected + 1 - page;
        }
    }

    fn details_cmd(&self) -> Vec<Cmd> {
        match self.selected_key() {
            Some((source, id)) if !self.details.contains_key(&format!("{source}/{id}")) => {
                vec![Cmd::LoadDetails { source, id }]
            }
            _ => Vec::new(),
        }
    }

    fn move_to(&mut self, target: usize) -> Vec<Cmd> {
        let before = self.selected;
        self.selected = target;
        self.clamp();
        if self.selected == before {
            Vec::new()
        } else {
            self.details_cmd()
        }
    }

    fn set_tab(&mut self, tab: Tab) -> Vec<Cmd> {
        self.tab = tab;
        self.focus = Focus::List;
        self.refresh_view();
        if tab != Tab::Search && self.installed.is_empty() {
            vec![Cmd::ScanInstalled]
        } else {
            Vec::new()
        }
    }
}

pub fn update(app: &mut App, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Key(k) => on_key(app, k),
        Msg::Resize(w, h) => {
            app.size = (w, h);
            app.clamp();
            Vec::new()
        }
        Msg::Tick => {
            let loading_details = app
                .selected_key()
                .is_some_and(|(s, id)| !app.details.contains_key(&format!("{s}/{id}")));
            if app.refreshing || loading_details {
                app.spinner_frame = app.spinner_frame.wrapping_add(1);
            }
            Vec::new()
        }
        Msg::CatalogLoaded(entries) => {
            app.search = SearchIndex::new(entries);
            app.research();
            app.clamp();
            Vec::new()
        }
        Msg::InstalledLoaded(rows) => {
            app.installed = rows;
            app.clamp();
            Vec::new()
        }
        Msg::InstalledFailed(reason) => {
            app.status = Some(format!("falha ao ler instalados: {reason}"));
            Vec::new()
        }
        Msg::DetailsLoaded { key, result } => {
            app.details.insert(key, result);
            Vec::new()
        }
        Msg::RefreshStarted => {
            app.refreshing = true;
            Vec::new()
        }
        Msg::RefreshBusy => {
            app.refreshing = false;
            app.status = Some("Aguardando: outra atualização em andamento".to_string());
            Vec::new()
        }
        Msg::Planned(result) => on_planned(app, result),
        Msg::InstallProgress { label, done, total } => {
            if let Some(Modal::Running { progress, .. }) = &mut app.modal {
                *progress = Some(RunProgress { label, done, total });
            }
            Vec::new()
        }
        Msg::InstallFinished(result) => on_finished(app, result),
        Msg::ResiduesCleaned(errors) => {
            app.modal = None;
            app.status = Some(if errors.is_empty() {
                "Resíduos removidos".to_string()
            } else {
                format!("Falha ao limpar resíduos: {}", errors.join("; "))
            });
            Vec::new()
        }
        Msg::RefreshFinished(reports) => {
            app.refreshing = false;
            let mut failed: Vec<String> = Vec::new();
            let mut any_offline = false;
            for r in &reports {
                if let Outcome::Failed { message, offline } = &r.outcome {
                    failed.push(format!("{}: {message}", r.source));
                    any_offline |= *offline;
                }
            }
            app.offline = any_offline;
            if failed.is_empty() {
                app.index_age_secs = Some(0);
                app.status = None;
            } else {
                app.status = Some(format!("Falha ao atualizar ({})", failed.join("; ")));
            }
            Vec::new()
        }
    }
}

fn on_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }
    if app.modal.is_some() {
        return on_modal_key(app, key);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => vec![Cmd::Quit],
            KeyCode::Char('r') => vec![Cmd::Refresh { force: true }],
            KeyCode::Char('s') => {
                app.filter = match app.filter {
                    SourceFilter::All => SourceFilter::Winget,
                    SourceFilter::Winget => SourceFilter::Scoop,
                    SourceFilter::Scoop => SourceFilter::All,
                };
                app.refresh_view();
                app.details_cmd()
            }
            _ => Vec::new(),
        };
    }
    if app.show_help {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
        ) {
            app.show_help = false;
        }
        return Vec::new();
    }
    let last = app.visible_len().saturating_sub(1);
    let empty = app.query.is_empty();
    match key.code {
        KeyCode::Down => app.move_to(app.selected.saturating_add(1)),
        KeyCode::Up => app.move_to(app.selected.saturating_sub(1)),
        KeyCode::Char('j') if empty => app.move_to(app.selected.saturating_add(1)),
        KeyCode::Char('k') if empty => app.move_to(app.selected.saturating_sub(1)),
        KeyCode::PageDown => app.move_to(app.selected.saturating_add(PAGE)),
        KeyCode::PageUp => app.move_to(app.selected.saturating_sub(PAGE)),
        KeyCode::Home => app.move_to(0),
        KeyCode::End => app.move_to(last),
        KeyCode::Tab => {
            let next = match app.tab {
                Tab::Search => Tab::Installed,
                Tab::Installed => Tab::Updates,
                Tab::Updates => Tab::Search,
            };
            app.set_tab(next)
        }
        KeyCode::BackTab => {
            let prev = match app.tab {
                Tab::Search => Tab::Updates,
                Tab::Installed => Tab::Search,
                Tab::Updates => Tab::Installed,
            };
            app.set_tab(prev)
        }
        KeyCode::Enter => {
            if app.visible_len() == 0 {
                return Vec::new();
            }
            app.focus = Focus::Details;
            app.details_cmd()
        }
        KeyCode::Esc => {
            if app.focus == Focus::Details {
                app.focus = Focus::List;
                Vec::new()
            } else if !empty {
                app.query.clear();
                app.refresh_view();
                Vec::new()
            } else {
                vec![Cmd::Quit]
            }
        }
        KeyCode::Char('i') if empty && app.tab == Tab::Search => install_selected(app),
        KeyCode::Char('u') if empty && app.tab == Tab::Installed => remove_selected(app),
        KeyCode::Char('U') if empty && app.tab == Tab::Updates => upgrade_selected(app),
        KeyCode::Char('q') if empty => vec![Cmd::Quit],
        KeyCode::Char('?') if empty => {
            app.show_help = true;
            Vec::new()
        }
        KeyCode::Backspace => {
            app.query.pop();
            app.refresh_view();
            app.details_cmd()
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::ALT) => {
            app.query.push(c);
            app.focus = Focus::List;
            app.refresh_view();
            app.details_cmd()
        }
        _ => Vec::new(),
    }
}

const NOT_OURS: &str = "só apps instalados pelo wingetrs podem ser removidos ou atualizados aqui";

/// `i`: plan installing the selected package (scoop only).
fn install_selected(app: &mut App) -> Vec<Cmd> {
    let Some(hit) = app.hits.get(app.selected) else {
        return Vec::new();
    };
    let Some(e) = app.search.entries().get(hit.index) else {
        return Vec::new();
    };
    if e.source != SourceKind::Scoop {
        app.status = Some("instalação na fase 2d".to_string());
        return Vec::new();
    }
    vec![Cmd::PlanInstall {
        source_name: e.source_name.clone(),
        id: e.id.clone(),
    }]
}

/// `u`: plan removing the selected app, which must be one wingetrs installed.
fn remove_selected(app: &mut App) -> Vec<Cmd> {
    let name = match app.installed_view().get(app.selected) {
        None => return Vec::new(),
        Some(r) if r.origin == "wingetrs" => r.name.clone(),
        Some(_) => {
            app.status = Some(NOT_OURS.to_string());
            return Vec::new();
        }
    };
    vec![Cmd::PlanRemove { app: name }]
}

/// `U`: plan upgrading the selected app; with nothing selected, every upgradable app.
fn upgrade_selected(app: &mut App) -> Vec<Cmd> {
    let (ours, source) = match app.installed_view().get(app.selected) {
        None => return vec![Cmd::PlanUpgradeAll],
        Some(r) => (
            r.origin == "wingetrs",
            match &r.correlation {
                Correlation::Matched {
                    source, package_id, ..
                } => Some((source.clone(), package_id.clone())),
                _ => None,
            },
        ),
    };
    match source {
        Some((source_name, id)) if ours => vec![Cmd::PlanInstall { source_name, id }],
        _ => {
            app.status = Some(NOT_OURS.to_string());
            Vec::new()
        }
    }
}

fn candidate_item(c: &Candidate) -> PickerItem {
    PickerItem {
        label: c.entry.id.clone(),
        detail: format!("{} {}", c.entry.source_name, c.entry.version),
        selectable: c.selectable,
        note: c.note.clone(),
    }
}

fn residue_item(r: &Residue) -> PickerItem {
    let (label, detail) = match &r.kind {
        ResidueKind::RegValue { key, value_name } => (
            format!("{key}\\{}", value_name.as_deref().unwrap_or("(padrão)")),
            "valor do registro",
        ),
        ResidueKind::RegKey { key } => (key.clone(), "chave do registro"),
        ResidueKind::Folder { path } => (path.display().to_string(), "pasta"),
        ResidueKind::Shortcut { path } => (path.display().to_string(), "atalho"),
    };
    let confidence = match r.confidence {
        Confidence::High => "alta confiança",
        Confidence::Low => "baixa confiança",
    };
    PickerItem {
        label,
        detail: format!("{detail}, {confidence}"),
        selectable: true,
        note: r.note.clone(),
    }
}

fn on_planned(app: &mut App, result: Result<Planned, String>) -> Vec<Cmd> {
    match result {
        Ok(Planned::Ready { text, action }) => {
            app.modal = Some(Modal::Confirm {
                plan_text: text,
                action,
            });
        }
        Ok(Planned::Choose(candidates)) => {
            let items = candidates.iter().map(candidate_item).collect();
            app.modal = Some(Modal::Picking {
                picker: Picker::single(items),
                candidates,
            });
        }
        Err(message) => app.status = Some(message),
    }
    Vec::new()
}

fn on_finished(app: &mut App, result: Result<Done, InstallFail>) -> Vec<Cmd> {
    app.modal = None;
    match result {
        Ok(done) => {
            app.status = Some(done.summary);
            if let Some((name, residues)) = done.residues
                && !residues.is_empty()
            {
                let items: Vec<PickerItem> = residues.iter().map(residue_item).collect();
                let initial: Vec<bool> = residues.iter().map(|r| r.preselected).collect();
                app.modal = Some(Modal::Residues {
                    picker: Picker::multi(items, &initial),
                    app: name,
                    residues,
                });
            }
        }
        Err(InstallFail::Busy) => {
            app.status = Some("outra instalação em andamento".to_string());
        }
        Err(InstallFail::Other(message)) => app.status = Some(message),
    }
    // Success or not, the disk may have changed.
    vec![Cmd::ScanInstalled]
}

fn on_modal_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    let ctrl_c = key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
    let Some(modal) = app.modal.take() else {
        return Vec::new();
    };
    match modal {
        Modal::Confirm { plan_text, action } => match key.code {
            KeyCode::Enter => {
                app.modal = Some(Modal::Running {
                    label: action.label(),
                    progress: None,
                });
                vec![Cmd::Execute(action)]
            }
            KeyCode::Esc => Vec::new(),
            _ => {
                app.modal = Some(Modal::Confirm { plan_text, action });
                Vec::new()
            }
        },
        Modal::Picking {
            mut picker,
            candidates,
        } => match picker.key(key) {
            PickOutcome::Pending => {
                app.modal = Some(Modal::Picking { picker, candidates });
                Vec::new()
            }
            PickOutcome::Cancelled => Vec::new(),
            PickOutcome::Picked(chosen) => match chosen.first().and_then(|&i| candidates.get(i)) {
                Some(c) if c.selectable => vec![Cmd::PlanInstall {
                    source_name: c.entry.source_name.clone(),
                    id: c.entry.id.clone(),
                }],
                _ => Vec::new(),
            },
        },
        Modal::Running { label, progress } => {
            // The operation cannot be abandoned half-way: keys wait for it.
            if ctrl_c {
                app.status = Some("aguarde: operação em andamento".to_string());
            }
            app.modal = Some(Modal::Running { label, progress });
            Vec::new()
        }
        Modal::Residues {
            mut picker,
            app: name,
            residues,
        } => match picker.key(key) {
            PickOutcome::Pending => {
                app.modal = Some(Modal::Residues {
                    picker,
                    app: name,
                    residues,
                });
                Vec::new()
            }
            PickOutcome::Cancelled => {
                app.status = Some("resíduos mantidos".to_string());
                Vec::new()
            }
            PickOutcome::Picked(chosen) => {
                let selected: Vec<Residue> = chosen
                    .iter()
                    .filter_map(|&i| residues.get(i).cloned())
                    .collect();
                if selected.is_empty() {
                    app.status = Some("resíduos mantidos".to_string());
                    return Vec::new();
                }
                app.modal = Some(Modal::Running {
                    label: format!("Limpando resíduos de {name}"),
                    progress: None,
                });
                vec![Cmd::CleanResidues {
                    app: name,
                    residues: selected,
                }]
            }
        },
    }
}
