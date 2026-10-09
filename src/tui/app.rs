//! Elm-style TUI state: `update` is pure and performs no I/O; side effects are returned as `Cmd`.
use crate::catalog::search::{SearchHit, SearchIndex};
use crate::core::{PackageDetails, PackageEntry, SourceKind};
use crate::installed::correlate::{Correlation, ListRow, UpdateState};
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Quit,
    Refresh { force: bool },
    LoadDetails { source: String, id: String },
    ScanInstalled,
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
