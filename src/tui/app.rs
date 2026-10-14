//! Elm-style TUI state: `update` is pure and performs no I/O; side effects are returned as `Cmd`.
use crate::catalog::search::{SearchHit, SearchIndex};
use crate::config::{Behavior, Config, ConfirmMode, ResidueMode};
use crate::core::{PackageDetails, PackageEntry, SourceKind};
use crate::install::plan::{InstallPlan, UninstallPlan};
use crate::install::residue::{Confidence, Residue, ResidueKind};
use crate::install::target::Candidate;
use crate::installed::correlate::{Correlation, ListRow, UpdateState};
use crate::tui::icons::{IconCache, IconProtocol, choose_from_env};
use crate::tui::layout::{
    DETAIL_ICON_COLS, DETAIL_ICON_ROWS, FILTER_GAP, ITEM_GAP, LIST_ICON_COLS,
};
use crate::tui::picker::{PickOutcome, Picker, PickerItem};
use crate::tui::settings::{SettingId, SettingsState, glyphs_name};
use crate::tui::theme::GlyphSet;
use crate::update::{Outcome, SourceReport};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Search,
    Installed,
    Updates,
    Settings,
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
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
    /// A toast deadline passed: prune and redraw.
    Clock,
    CatalogLoaded(Vec<PackageEntry>),
    /// Reloading the catalog failed; the current index is kept.
    CatalogFailed(String),
    /// Startup load finished: the catalog (or why it could not be read), its age and
    /// whether the startup refresh is due.
    Booted {
        catalog: Result<Vec<PackageEntry>, String>,
        age: Option<i64>,
        needs_refresh: bool,
    },
    InstalledLoaded(Vec<ListRow>),
    /// Reading or correlating the installed apps failed (a worker panic is re-raised instead).
    InstalledFailed(String),
    DetailsLoaded {
        key: String,
        result: Result<PackageDetails, String>,
    },
    /// Raw icon bytes of `key`, sent after its details and only when the icon was obtained.
    IconLoaded {
        key: String,
        bytes: Vec<u8>,
    },
    /// The `icons.json` map (`<source>/<id>` -> SHA-256 of the cached icon), read at startup.
    IconMapLoaded(BTreeMap<String, String>),
    /// An icon was just cached: `key` now has its blob under `sha`.
    IconMapped {
        key: String,
        sha: String,
    },
    /// The list's icon fetch of `key` ended (with or without an icon); it is not asked again.
    PrefetchDone {
        key: String,
    },
    /// The icon of `key` encoded for `size` cells (columns, rows), or why it could not be.
    IconReady {
        /// `App::icon_gen` when the encode was requested; older results are dropped.
        generation: u64,
        key: String,
        size: (u16, u16),
        proto: Result<ratatui_image::protocol::Protocol, String>,
    },
    RefreshStarted,
    RefreshFinished(Vec<SourceReport>),
    /// Another process holds the update lock; not a failure and not offline.
    RefreshBusy,
    /// Result of writing the settings to `config.toml`.
    SettingsSaved(Result<(), String>),
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
    /// `BUSY_RETRY` passed since another process held the lock: try the queue again.
    RetryQueue,
    /// Residue cleanup ended; one message per item that failed.
    ResiduesCleaned(Vec<String>),
    /// Hits of the `Cmd::Search` with this generation; stale generations are dropped.
    SearchDone {
        generation: u64,
        hits: Vec<SearchHit>,
    },
    /// The search of this generation was cancelled without a result; the list keeps its hits.
    SearchCancelled {
        generation: u64,
    },
    /// A worker thread panicked; the event loop re-raises it before `update` sees it.
    Panicked(Box<dyn std::any::Any + Send>),
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

/// The operation in progress (an action, or the cleanup of residues).
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub id: u64,
    pub label: String,
    pub progress: Option<RunProgress>,
    /// The action itself, put back in line when another process holds the lock;
    /// `None` for a residue cleanup.
    pub action: Option<Action>,
}

/// A confirmed action waiting for the one in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct Queued {
    pub id: u64,
    pub label: String,
    pub action: Action,
}

/// Wait before trying the queue again after another process held `install.lock`.
pub const BUSY_RETRY: Duration = Duration::from_secs(2);

/// The dialog on top of the screen; it takes every key until it closes.
pub enum Modal {
    Confirm {
        plan_text: String,
        /// `None`: the quit confirmation (Enter drops the queue and quits once the
        /// operation in progress ends).
        action: Option<Action>,
    },
    Picking {
        picker: Picker,
        candidates: Vec<Candidate>,
        /// `keep` of the plan that asked (install: false; upgrade: setting at key time).
        keep: bool,
    },
    /// Actions of an installed app: [upgrade, remove].
    Actions {
        picker: Picker,
        app: String,
        /// `(source_name, id)` when the app can be upgraded.
        upgrade: Option<(String, String)>,
        /// Name to remove when wingetrs installed the app.
        remove: Option<String>,
    },
    Residues {
        picker: Picker,
        app: String,
        residues: Vec<Residue>,
    },
    /// The operation queue; `selected` is the id of the row under the cursor.
    Queue { selected: u64 },
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
    /// `keep`: keep the old version when this is an upgrade (setting at key time).
    PlanInstall {
        source_name: String,
        id: String,
        keep: bool,
    },
    /// `purge`: also delete the app's persistent data (setting at key time).
    PlanRemove {
        app: String,
        purge: bool,
    },
    /// Plan upgrading every app with a newer catalog version.
    PlanUpgradeAll {
        keep: bool,
    },
    Execute(Action),
    /// Answer with `Msg::RetryQueue` after `BUSY_RETRY`.
    RetryQueue,
    CleanResidues {
        app: String,
        residues: Vec<Residue>,
    },
    /// Write the settings to `config.toml` (`glyphs`: nerd|unicode|ascii, `theme`: dark|light).
    SaveSettings {
        behavior: Behavior,
        /// `None` leaves `glyphs` in the file as it is (flag in force, or Glifos never changed).
        glyphs: Option<String>,
        /// `None` leaves `icons` in the file as it is (Ícones never changed).
        icons: Option<String>,
        /// `None` leaves `list_icons` in the file as it is (Ícones da lista never changed).
        list_icons: Option<String>,
        theme: String,
    },
    /// Turn the terminal mouse capture on or off (`App.mouse_enabled` is already set).
    SetMouseCapture(bool),
    /// Decode and encode an icon off the UI thread and answer with `Msg::IconReady`.
    EncodeIcon {
        generation: u64,
        key: String,
        bytes: Vec<u8>,
        size: (u16, u16),
    },
    /// Read a cached icon blob and encode it, off the UI thread; answers with `Msg::IconReady`.
    /// Never touches the network.
    EncodeCachedIcon {
        generation: u64,
        source: String,
        id: String,
        sha: String,
        size: (u16, u16),
    },
    /// Fetch the icons of these visible winget rows (details, then icon) off the UI thread;
    /// each one obtained answers with `Msg::IconMapped`. A newer generation supersedes it.
    PrefetchIcons {
        generation: u64,
        keys: Vec<(String, String)>,
    },
    /// Run `run_search` off the UI thread and answer with `Msg::SearchDone`.
    Search {
        generation: u64,
        query: String,
        kind: Option<SourceKind>,
    },
}

/// What kind of event a toast reports; decides its color, glyph and lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Info,
    Warning,
    Error,
}

impl ToastKind {
    /// Time on screen; `None` stays until `Esc`.
    fn lifetime(self) -> Option<Duration> {
        match self {
            ToastKind::Success => Some(Duration::from_secs(3)),
            ToastKind::Info => Some(Duration::from_secs(4)),
            ToastKind::Warning => Some(Duration::from_secs(6)),
            ToastKind::Error => None,
        }
    }
}

/// A notice for something that happened; the status bar holds what is true now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub kind: ToastKind,
    pub text: String,
    pub expires: Option<Instant>,
    pub count: u32,
}

/// Most icon encodings in flight at once.
pub const MAX_ICON_JOBS: usize = 8;

/// Most toasts on screen at once.
pub const MAX_TOASTS: usize = 3;

/// Order of the Installed and Updates lists; lives for the session only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Source,
    State,
}

pub struct App {
    pub sort_installed: SortKey,
    pub sort_updates: SortKey,
    pub tab: Tab,
    pub filter: SourceFilter,
    pub query: String,
    pub selected: usize,
    pub scroll: usize,
    pub search: Arc<SearchIndex>,
    pub hits: Vec<SearchHit>,
    /// Generation of the newest `Cmd::Search`.
    pub search_generation: u64,
    /// The one search running off the UI thread: generation, query and filter it was asked for.
    /// Keystrokes meanwhile only change `query`; the latest is searched when this one ends.
    search_in_flight: Option<(u64, String, Option<SourceKind>)>,
    pub installed: Vec<ListRow>,
    pub details: HashMap<String, Result<PackageDetails, String>>,
    /// Raw icon bytes, keyed like `details`.
    pub icon_bytes: HashMap<String, Vec<u8>>,
    /// Protocol icons are drawn with; `Off` decodes and draws nothing.
    pub icon_protocol: IconProtocol,
    /// Encoded icons ready to draw.
    pub icons: IconCache,
    /// Keys whose icon could not be decoded; never retried in this session.
    pub icon_none: HashSet<String>,
    /// Bumped whenever the icon protocol changes; encodes of an older one are discarded.
    pub icon_gen: u64,
    /// `(key, size)` of the icons being encoded.
    icon_in_flight: HashSet<(String, (u16, u16))>,
    /// Cached icons known to the list: `<source>/<id>` -> SHA-256 of the blob.
    pub icon_map: BTreeMap<String, String>,
    /// `<source>/<id>` of the list rows whose icon fetch ended in this session.
    icon_tried: HashSet<String>,
    /// Keys of the newest `Cmd::PrefetchIcons`.
    last_prefetch: HashSet<String>,
    /// Generation of the newest `Cmd::PrefetchIcons`.
    pub list_fetch_gen: u64,
    /// Fetch the icons of the visible list rows.
    pub list_icons_visible: bool,
    /// `"{source}/{id}"` of the selected package while its details are being loaded.
    pub details_pending: Option<String>,
    pub refreshing: bool,
    pub offline: bool,
    pub index_age_secs: Option<i64>,
    pub spinner_frame: usize,
    pub show_help: bool,
    pub status: Option<String>,
    pub glyphs: GlyphSet,
    pub size: (u16, u16),
    pub modal: Option<Modal>,
    /// A plan is being computed; further plan keys are ignored until it arrives.
    pub planning: bool,
    /// `keep` of the plan in flight, carried to the candidate picker if one opens.
    pub planning_keep: bool,
    /// `confirm` setting when the plan key was pressed (a later change does not skip the dialog);
    /// `None` = no plan key pending, the current setting applies.
    pub planning_confirm: Option<ConfirmMode>,
    /// Clock for toasts; the loop sets it before each batch, tests set it by hand.
    pub now: Instant,
    /// Toasts on screen, newest first.
    pub toasts: Vec<Toast>,
    /// The refresh in flight was asked for with Ctrl+R (its success is announced).
    pub refresh_forced: bool,
    /// State of the Settings tab.
    pub settings: SettingsState,
    /// Dark palette in use (the loop rebuilds the theme when it changes).
    pub dark: bool,
    /// Bucket names for the read-only sources section.
    pub buckets_summary: String,
    /// The values (behavior, glyphs, dark) of the one save in flight. Further changes only
    /// update the state; when the save ends, one more save carries the newest values.
    save_in_flight: Option<(Behavior, GlyphSet, bool)>,
    /// The `icons` value of the save in flight (goes with `save_in_flight`).
    icons_in_flight: Option<String>,
    list_icons_in_flight: Option<String>,
    /// Whether terminal mouse events are acted on (the capture itself is `run`'s).
    pub mouse_enabled: bool,
    /// Time, tab and index of the last left click, for the double click.
    last_click: Option<(Instant, Tab, usize)>,
    /// The operation in progress; the screen stays usable meanwhile.
    pub running: Option<Running>,
    /// Confirmed actions waiting for `running`, in order.
    pub queue: VecDeque<Queued>,
    pub next_op_id: u64,
    /// Another process held the lock: the queue waits for `Msg::RetryQueue`.
    pub waiting_lock: bool,
    /// Quit was confirmed with an operation in progress: quit when it ends.
    pub quit_after_running: bool,
    /// Dialogs that arrived while another was open; shown, in order, as it closes.
    pending_modals: VecDeque<Modal>,
}

const MAX_HITS: usize = 1000;

/// The one search the TUI runs, on the UI thread at load and on a worker per keystroke.
pub fn run_search(index: &SearchIndex, query: &str, kind: Option<SourceKind>) -> Vec<SearchHit> {
    index.search(query, kind, MAX_HITS)
}
const PAGE: usize = 10;
/// Screen lines per list item: title and description.
pub const ITEM_ROWS: usize = 2;
/// Terminal height from which the help bar shows under the status bar.
pub const HELP_BAR_MIN_HEIGHT: u16 = 20;
/// Rows of screen taken by tabs (3), search line, filter gap, bottom border, status bar and,
/// on tall terminals, the help bar.
/// Items that fit in `rows` list lines (at least one).
pub fn items_in_rows(rows: usize) -> usize {
    ((rows + usize::from(ITEM_GAP)) / (ITEM_ROWS + usize::from(ITEM_GAP))).max(1)
}

/// Items the list shows on a screen `screen_height` lines tall; the view sizes its window
/// with `items_in_rows` on the area it really has, so both agree.
pub fn list_page_rows(screen_height: u16) -> usize {
    let rows = if screen_height < crate::tui::view::MIN_HEIGHT_FOR_CHROME {
        usize::from(screen_height)
    } else {
        usize::from(screen_height).saturating_sub(chrome_rows(screen_height))
    };
    items_in_rows(rows)
}

pub fn chrome_rows(height: u16) -> usize {
    let help = usize::from(height >= HELP_BAR_MIN_HEIGHT);
    3 + 1 + FILTER_GAP as usize + 1 + 1 + help
}

impl App {
    pub fn new(entries: Vec<PackageEntry>, glyphs: GlyphSet) -> App {
        let mut app = App {
            sort_installed: SortKey::Name,
            sort_updates: SortKey::Name,
            tab: Tab::Search,
            filter: SourceFilter::All,
            query: String::new(),
            selected: 0,
            scroll: 0,
            search: Arc::new(SearchIndex::new(entries)),
            hits: Vec::new(),
            search_generation: 0,
            search_in_flight: None,
            installed: Vec::new(),
            details: HashMap::new(),
            icon_bytes: HashMap::new(),
            icon_protocol: IconProtocol::Off,
            icons: IconCache::default(),
            icon_none: HashSet::new(),
            icon_gen: 0,
            icon_in_flight: HashSet::new(),
            icon_map: BTreeMap::new(),
            icon_tried: HashSet::new(),
            last_prefetch: HashSet::new(),
            list_fetch_gen: 0,
            list_icons_visible: true,
            details_pending: None,
            refreshing: false,
            offline: false,
            index_age_secs: None,
            spinner_frame: 0,
            show_help: false,
            status: None,
            glyphs,
            size: (80, 24),
            modal: None,
            planning: false,
            planning_keep: false,
            planning_confirm: None,
            now: Instant::now(),
            toasts: Vec::new(),
            refresh_forced: false,
            settings: SettingsState::new(Config::default().behavior, glyphs, true, false),
            dark: true,
            buckets_summary: String::new(),
            save_in_flight: None,
            icons_in_flight: None,
            list_icons_in_flight: None,
            mouse_enabled: true,
            last_click: None,
            running: None,
            queue: VecDeque::new(),
            next_op_id: 0,
            waiting_lock: false,
            quit_after_running: false,
            pending_modals: VecDeque::new(),
        };
        app.research();
        app
    }

    /// Shows a toast; a repeat of one already shown moves it to the top, counts it and
    /// restarts its timer. Past `MAX_TOASTS`, the oldest timed toast goes (or the oldest).
    pub fn push_toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        let expires = kind.lifetime().map(|d| self.now + d);
        if let Some(i) = self
            .toasts
            .iter()
            .position(|t| t.kind == kind && t.text == text)
        {
            let mut t = self.toasts.remove(i);
            t.count += 1;
            t.expires = expires;
            self.toasts.insert(0, t);
            return;
        }
        self.toasts.insert(
            0,
            Toast {
                kind,
                text,
                expires,
                count: 1,
            },
        );
        if self.toasts.len() > MAX_TOASTS {
            let victim = self
                .toasts
                .iter()
                .rposition(|t| t.expires.is_some())
                .unwrap_or(self.toasts.len() - 1);
            self.toasts.remove(victim);
        }
    }

    /// Drops the toasts whose time is up.
    pub fn prune_toasts(&mut self) {
        let now = self.now;
        self.toasts.retain(|t| t.expires.is_none_or(|e| e > now));
    }

    /// When the loop must wake to drop a toast: the earliest deadline.
    pub fn next_toast_deadline(&self) -> Option<Instant> {
        self.toasts.iter().filter_map(|t| t.expires).min()
    }

    /// Order in force for the current tab.
    pub fn sort_key(&self) -> SortKey {
        if self.tab == Tab::Updates {
            self.sort_updates
        } else {
            self.sort_installed
        }
    }

    /// Installed rows visible in the Installed/Updates tabs after query and source filter,
    /// in the tab's sort order (the one place that orders them).
    pub fn installed_view(&self) -> Vec<&ListRow> {
        let q = self.query.to_lowercase();
        let mut rows: Vec<&ListRow> = self
            .installed
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
            .collect();
        // Stable sort: ties fall to the lowercase name, the final tie-break.
        match self.sort_key() {
            SortKey::Name => {
                rows.sort_by_cached_key(|r| (0u8, String::new(), r.name.to_lowercase()))
            }
            SortKey::Source => rows.sort_by_cached_key(|r| match &r.correlation {
                Correlation::Matched { source, .. } => (0u8, source.clone(), r.name.to_lowercase()),
                _ => (1u8, String::new(), r.name.to_lowercase()),
            }),
            SortKey::State => rows.sort_by_cached_key(|r| {
                let rank = match r.state {
                    UpdateState::Available { .. } => 0u8,
                    UpdateState::Broken => 1,
                    UpdateState::Unknown => 2,
                    UpdateState::UpToDate => 3,
                };
                (rank, String::new(), r.name.to_lowercase())
            }),
        }
        rows
    }

    /// Next order in the cycle (`o`); the same item stays selected, or 0 if it vanished.
    fn cycle_sort(&mut self) {
        let kept = self.selected_ident();
        if self.tab == Tab::Updates {
            self.sort_updates = match self.sort_updates {
                SortKey::Name => SortKey::Source,
                _ => SortKey::Name,
            };
        } else {
            self.sort_installed = match self.sort_installed {
                SortKey::Name => SortKey::Source,
                SortKey::Source => SortKey::State,
                SortKey::State => SortKey::Name,
            };
        }
        self.selected = self.position_of(kept).unwrap_or(0);
        self.clamp();
    }

    /// `name` + `origin` of the selected Installed/Updates row.
    fn selected_ident(&self) -> Option<(String, String)> {
        if !matches!(self.tab, Tab::Installed | Tab::Updates) {
            return None;
        }
        self.installed_view()
            .get(self.selected)
            .map(|r| (r.name.clone(), r.origin.clone()))
    }

    /// Where the row identified by `ident` sits in the current view.
    fn position_of(&self, ident: Option<(String, String)>) -> Option<usize> {
        let (n, o) = ident?;
        self.installed_view()
            .iter()
            .position(|r| r.name == n && r.origin == o)
    }

    /// Rows visible in the current tab after filters.
    pub fn visible_len(&self) -> usize {
        match self.tab {
            Tab::Search => self.hits.len(),
            Tab::Installed | Tab::Updates => self.installed_view().len(),
            Tab::Settings => 0,
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
            Tab::Settings => None,
        }
    }

    fn search_kind(&self) -> Option<SourceKind> {
        match self.filter {
            SourceFilter::All => None,
            SourceFilter::Winget => Some(SourceKind::Winget),
            SourceFilter::Scoop => Some(SourceKind::Scoop),
        }
    }

    /// Synchronous search, used only when the index itself is (re)built.
    fn research(&mut self) {
        // A search still in flight refers to the previous index: its result is dropped.
        self.search_in_flight = None;
        self.hits = run_search(&self.search, &self.query, self.search_kind());
    }

    /// Recomputes the row set after query/filter/tab changes and resets the selection.
    /// On the Search tab the hits come later through `Msg::SearchDone`.
    fn refresh_view(&mut self) -> Vec<Cmd> {
        self.selected = 0;
        self.scroll = 0;
        if self.tab != Tab::Search || self.search_in_flight.is_some() {
            return Vec::new();
        }
        self.start_search()
    }

    fn start_search(&mut self) -> Vec<Cmd> {
        self.search_generation += 1;
        let (query, kind) = (self.query.clone(), self.search_kind());
        self.search_in_flight = Some((self.search_generation, query.clone(), kind));
        vec![Cmd::Search {
            generation: self.search_generation,
            query,
            kind,
        }]
    }

    /// Result of the search in flight: dropped if stale or off the Search tab; if the query
    /// changed meanwhile, the current one is searched instead of showing the old result.
    fn search_done(&mut self, generation: u64, hits: Vec<SearchHit>) -> Vec<Cmd> {
        let Some((_, query, kind)) = self.search_in_flight.take_if(|f| f.0 == generation) else {
            return Vec::new();
        };
        if self.tab != Tab::Search {
            return Vec::new();
        }
        if query != self.query || kind != self.search_kind() {
            return self.start_search();
        }
        self.hits = hits;
        self.selected = 0;
        self.scroll = 0;
        self.details_cmd()
    }

    /// `refresh_view` plus the detail request; on the Search tab that waits for the hits.
    fn view_changed(&mut self) -> Vec<Cmd> {
        let mut cmds = self.refresh_view();
        if self.tab != Tab::Search {
            cmds.extend(self.details_cmd());
        }
        cmds
    }

    /// Items that fit in the list at the current terminal height.
    fn page(&self) -> usize {
        list_page_rows(self.size.1)
    }

    fn clamp(&mut self) {
        let len = self.visible_len();
        self.selected = self.selected.min(len.saturating_sub(1));
        let page = self.page();
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + page {
            self.scroll = self.selected + 1 - page;
        }
    }

    /// True while something on screen animates the spinner.
    pub fn animating(&self) -> bool {
        self.refreshing || self.details_pending.is_some() || self.running.is_some()
    }

    /// Requests encoding the selected package's icon when its bytes are here and it is
    /// neither encoded, being encoded, nor known to be undecodable.
    fn icon_cmd(&mut self) -> Vec<Cmd> {
        if self.icon_protocol == IconProtocol::Off {
            return Vec::new();
        }
        let Some((source, id)) = self.selected_key() else {
            return Vec::new();
        };
        let key = format!("{source}/{id}");
        let size = (DETAIL_ICON_COLS, DETAIL_ICON_ROWS);
        if self.icon_none.contains(&key)
            || self.icon_in_flight.contains(&(key.clone(), size))
            || self.icons.get(&key, size).is_some()
        {
            return Vec::new();
        }
        let Some(bytes) = self.icon_bytes.get(&key) else {
            return Vec::new();
        };
        let bytes = bytes.clone();
        self.icon_in_flight.insert((key.clone(), size));
        vec![Cmd::EncodeIcon {
            generation: self.icon_gen,
            key,
            bytes,
            size,
        }]
    }

    /// `(source, id)` of each list row in `window`, in order; `None` for rows with no
    /// catalog package.
    pub fn list_keys(&self, window: std::ops::Range<usize>) -> Vec<Option<(String, String)>> {
        let key = |source: &str, id: &str| Some((source.to_owned(), id.to_owned()));
        match self.tab {
            Tab::Search => self
                .hits
                .iter()
                .skip(window.start)
                .take(window.len())
                .map(|h| {
                    let e = self.search.entries().get(h.index)?;
                    key(&e.source_name, &e.id)
                })
                .collect(),
            Tab::Installed | Tab::Updates => self
                .installed_view()
                .into_iter()
                .skip(window.start)
                .take(window.len())
                .map(|r| match &r.correlation {
                    Correlation::Matched {
                        source, package_id, ..
                    } => key(source, package_id),
                    _ => None,
                })
                .collect(),
            Tab::Settings => Vec::new(),
        }
    }

    /// Rows of the list on screen.
    fn list_window(&self) -> std::ops::Range<usize> {
        crate::tui::view::list_window(
            self.selected.min(self.visible_len().saturating_sub(1)),
            self.scroll,
            self.visible_len(),
            self.page(),
        )
    }

    /// Requests fetching the icons of the visible winget rows that are neither cached nor
    /// done, in screen order; at most one command. Only when some row is not in the previous
    /// request: the new one covers every visible row, so superseding the old loses nothing,
    /// and rows ending never ask again.
    fn list_fetch_cmds(&mut self) -> Vec<Cmd> {
        if !self.list_icons_visible
            || self.tab == Tab::Settings
            || !crate::tui::view::icon_visible(self)
        {
            return Vec::new();
        }
        let (keys, names): (Vec<(String, String)>, HashSet<String>) = self
            .list_keys(self.list_window())
            .into_iter()
            .flatten()
            .filter(|(source, _)| source == "winget")
            .map(|(source, id)| {
                let key = format!("{source}/{id}");
                ((source, id), key)
            })
            .filter(|(_, key)| !self.icon_map.contains_key(key) && !self.icon_tried.contains(key))
            .unzip();
        if names.iter().all(|k| self.last_prefetch.contains(k)) {
            return Vec::new();
        }
        self.last_prefetch = names;
        self.list_fetch_gen += 1;
        vec![Cmd::PrefetchIcons {
            generation: self.list_fetch_gen,
            keys,
        }]
    }

    /// Requests encoding of the visible rows' cached icons that are neither encoded, being
    /// encoded, nor known to be undecodable; at most `MAX_ICON_JOBS` in flight. The bytes
    /// come from the blob store (never the network) on a worker.
    fn list_icon_cmds(&mut self) -> Vec<Cmd> {
        if self.icon_map.is_empty()
            || self.tab == Tab::Settings
            || !crate::tui::view::icon_visible(self)
        {
            return Vec::new();
        }
        let size = (LIST_ICON_COLS, ITEM_ROWS as u16);
        let mut cmds = Vec::new();
        for (source, id) in self.list_keys(self.list_window()).into_iter().flatten() {
            if self.icon_in_flight.len() >= MAX_ICON_JOBS {
                break;
            }
            let key = format!("{source}/{id}");
            let Some(sha) = self.icon_map.get(&key) else {
                continue;
            };
            let flight = (key, size);
            if self.icon_none.contains(&flight.0)
                || self.icon_in_flight.contains(&flight)
                || self.icons.get(&flight.0, size).is_some()
            {
                continue;
            }
            cmds.push(Cmd::EncodeCachedIcon {
                generation: self.icon_gen,
                source,
                id,
                sha: sha.clone(),
                size,
            });
            self.icon_in_flight.insert(flight);
        }
        cmds
    }

    /// Requests the selected package's details unless cached (and its icon's encoding);
    /// records what is pending.
    fn details_cmd(&mut self) -> Vec<Cmd> {
        let mut cmds = self.details_only_cmd();
        cmds.extend(self.icon_cmd());
        cmds
    }

    fn details_only_cmd(&mut self) -> Vec<Cmd> {
        match self.selected_key() {
            Some((source, id)) => {
                let key = format!("{source}/{id}");
                if self.details.contains_key(&key) {
                    self.details_pending = None;
                    Vec::new()
                } else {
                    self.details_pending = Some(key);
                    vec![Cmd::LoadDetails { source, id }]
                }
            }
            None => {
                self.details_pending = None;
                Vec::new()
            }
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
        let mut cmds = self.refresh_view();
        if tab != Tab::Search {
            if self.installed.is_empty() && tab != Tab::Settings {
                cmds.push(Cmd::ScanInstalled);
            }
            cmds.extend(self.details_cmd());
        }
        cmds
    }
}

/// Applies `msg`; the result also asks for the visible list rows' cached icons and for
/// fetching those not tried yet.
pub fn update(app: &mut App, msg: Msg) -> Vec<Cmd> {
    let mut cmds = apply(app, msg);
    cmds.extend(app.list_icon_cmds());
    cmds.extend(app.list_fetch_cmds());
    cmds
}

fn apply(app: &mut App, msg: Msg) -> Vec<Cmd> {
    app.prune_toasts();
    match msg {
        Msg::Key(k) => on_key(app, k),
        Msg::Mouse(m) => on_mouse(app, m),
        Msg::Clock => Vec::new(),
        Msg::Resize(w, h) => {
            app.size = (w, h);
            app.clamp();
            Vec::new()
        }
        Msg::Tick => {
            if app.animating() {
                app.spinner_frame = app.spinner_frame.wrapping_add(1);
            }
            Vec::new()
        }
        Msg::CatalogLoaded(entries) => {
            app.search = Arc::new(SearchIndex::new(entries));
            app.research();
            app.clamp();
            app.details_cmd()
        }
        Msg::CatalogFailed(e) => {
            app.push_toast(ToastKind::Error, format!("falha ao ler o catálogo: {e}"));
            Vec::new()
        }
        Msg::Booted {
            catalog,
            age,
            needs_refresh,
        } => {
            app.status = None;
            app.index_age_secs = age;
            let mut cmds = vec![Cmd::ScanInstalled];
            let refresh = match catalog {
                Ok(entries) => {
                    let empty = entries.is_empty();
                    app.search = Arc::new(SearchIndex::new(entries));
                    app.research();
                    app.clamp();
                    cmds.extend(app.details_cmd());
                    empty || needs_refresh
                }
                Err(e) => {
                    app.push_toast(ToastKind::Error, format!("falha ao ler o catálogo: {e}"));
                    true
                }
            };
            if refresh {
                cmds.push(Cmd::Refresh { force: false });
            }
            cmds
        }
        Msg::InstalledLoaded(rows) => {
            let kept = app.selected_ident();
            app.installed = rows;
            if let Some(i) = app.position_of(kept) {
                app.selected = i;
            }
            app.clamp();
            app.details_cmd()
        }
        Msg::InstalledFailed(reason) => {
            app.push_toast(
                ToastKind::Error,
                format!("falha ao ler instalados: {reason}"),
            );
            Vec::new()
        }
        Msg::IconLoaded { key, bytes } => {
            // Stored even if the selection moved on: the bytes are keyed and the cache is bounded.
            app.icon_bytes.insert(key, bytes);
            app.icon_cmd()
        }
        Msg::IconMapLoaded(loaded) => {
            // Entries written meanwhile are newer than the file read at startup.
            for (k, sha) in loaded {
                app.icon_map.entry(k).or_insert(sha);
            }
            Vec::new()
        }
        Msg::IconMapped { key, sha } => {
            app.icon_map.insert(key, sha);
            Vec::new()
        }
        Msg::PrefetchDone { key } => {
            app.icon_tried.insert(key);
            Vec::new()
        }
        Msg::IconReady {
            generation,
            key,
            size,
            proto,
        } => {
            if generation != app.icon_gen {
                // Encoded for a protocol no longer in use; its slot was already cleared.
                return Vec::new();
            }
            app.icon_in_flight.remove(&(key.clone(), size));
            match proto {
                _ if app.icon_protocol == IconProtocol::Off => {}
                Ok(p) => app.icons.insert(key, size, p),
                Err(_) => {
                    app.icon_none.insert(key);
                }
            }
            Vec::new()
        }
        Msg::DetailsLoaded { key, result } => {
            if app.details_pending.as_deref() == Some(key.as_str()) {
                app.details_pending = None;
            }
            app.details.insert(key, result);
            Vec::new()
        }
        Msg::RefreshStarted => {
            app.refreshing = true;
            Vec::new()
        }
        Msg::RefreshBusy => {
            app.refreshing = false;
            app.refresh_forced = false;
            app.push_toast(
                ToastKind::Info,
                "Aguardando: outra atualização em andamento",
            );
            Vec::new()
        }
        Msg::SearchDone { generation, hits } => app.search_done(generation, hits),
        Msg::SearchCancelled { generation } => {
            app.search_in_flight.take_if(|f| f.0 == generation);
            Vec::new()
        }
        Msg::SettingsSaved(result) => {
            let sent = app.save_in_flight.take();
            let current = (
                app.settings.behavior,
                app.settings.glyphs,
                app.settings.dark,
            );
            // A newer change was made while saving: it gets its own save and the outcome
            // (toast, and on failure the revert) is decided by that one. A failure with a
            // newer change pending therefore does not revert; the retry carries the newest.
            let sent_icons = app.icons_in_flight.take();
            let sent_list_icons = app.list_icons_in_flight.take();
            let newer = sent.is_some_and(|s| s != current)
                || sent_icons
                    .as_ref()
                    .is_some_and(|i| *i != app.settings.icons)
                || sent_list_icons
                    .as_ref()
                    .is_some_and(|l| *l != app.settings.list_icons);
            if let (Ok(()), Some(sent)) = (&result, sent) {
                app.settings.saved = sent;
                if let Some(i) = sent_icons {
                    app.settings.saved_icons = i;
                }
                if let Some(l) = sent_list_icons {
                    app.settings.saved_list_icons = l;
                }
            }
            if newer {
                return request_save(app);
            }
            app.status = None;
            match result {
                Ok(()) => {
                    app.push_toast(ToastKind::Success, "ajustes salvos");
                }
                Err(e) => {
                    let mouse_before = app.mouse_enabled;
                    let (behavior, glyphs, dark) = app.settings.saved;
                    app.settings.behavior = behavior;
                    app.settings.glyphs = glyphs;
                    app.settings.dark = dark;
                    app.glyphs = glyphs;
                    app.dark = dark;
                    if app.settings.icons != app.settings.saved_icons {
                        app.settings.icons = app.settings.saved_icons.clone();
                        apply_icons(app);
                    }
                    if app.settings.list_icons != app.settings.saved_list_icons {
                        app.settings.list_icons = app.settings.saved_list_icons.clone();
                        app.list_icons_visible =
                            app.settings.list_icons == crate::config::LIST_ICONS[0];
                    }
                    app.push_toast(ToastKind::Error, format!("falha ao salvar: {e}"));
                    app.mouse_enabled = behavior.mouse;
                    if app.mouse_enabled != mouse_before {
                        return vec![Cmd::SetMouseCapture(app.mouse_enabled)];
                    }
                }
            }
            Vec::new()
        }
        Msg::Planned(result) => on_planned(app, result),
        Msg::Panicked(payload) => std::panic::resume_unwind(payload),
        Msg::InstallProgress { label, done, total } => {
            if let Some(r) = &mut app.running {
                r.progress = Some(RunProgress { label, done, total });
            }
            Vec::new()
        }
        Msg::InstallFinished(result) => on_finished(app, result),
        Msg::RetryQueue => {
            app.waiting_lock = false;
            pump(app)
        }
        Msg::ResiduesCleaned(errors) => {
            app.running = None;
            if errors.is_empty() {
                app.push_toast(ToastKind::Success, "Resíduos removidos");
            } else {
                app.push_toast(
                    ToastKind::Error,
                    format!("Falha ao limpar resíduos: {}", errors.join("; ")),
                );
            }
            pump(app)
        }
        Msg::RefreshFinished(reports) => {
            app.refreshing = false;
            let forced = std::mem::take(&mut app.refresh_forced);
            let mut failed: Vec<String> = Vec::new();
            let mut any_offline = false;
            let mut all_offline = true;
            for r in &reports {
                if let Outcome::Failed { message, offline } = &r.outcome {
                    failed.push(format!("{}: {message}", r.source));
                    any_offline |= *offline;
                    all_offline &= *offline;
                }
            }
            app.offline = any_offline;
            if failed.is_empty() {
                app.index_age_secs = Some(0);
                if forced {
                    app.push_toast(ToastKind::Success, "catálogo atualizado");
                }
            } else {
                let kind = if all_offline {
                    ToastKind::Warning
                } else {
                    ToastKind::Error
                };
                app.push_toast(kind, format!("Falha ao atualizar ({})", failed.join("; ")));
            }
            Vec::new()
        }
    }
}

/// Two left clicks on the same item within this time act as `Enter`.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Items a wheel notch moves the selection.
const WHEEL_ITEMS: usize = 3;

/// Mouse: left click on a tab, list item or setting row; double click is `Enter`; the wheel
/// moves the list. Ignored under a modal or the help, and when `mouse_enabled` is off.
fn on_mouse(app: &mut App, ev: MouseEvent) -> Vec<Cmd> {
    if !app.mouse_enabled || app.modal.is_some() || app.show_help {
        return Vec::new();
    }
    let area = ratatui::layout::Rect::new(0, 0, app.size.0, app.size.1);
    let r = crate::tui::regions::regions(app, area);
    let inside = |rect: ratatui::layout::Rect| rect.contains((ev.column, ev.row).into());
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some((_, tab)) = r.tabs.iter().find(|(rect, _)| inside(*rect)) {
                app.last_click = None;
                return app.set_tab(*tab);
            }
            let hit = if app.tab == Tab::Settings {
                r.setting_rows.iter().position(|rect| inside(*rect))
            } else {
                crate::tui::regions::item_at(app, r.list, ev.column, ev.row)
            };
            let Some(index) = hit else {
                app.last_click = None;
                return Vec::new();
            };
            let double = matches!(app.last_click,
                Some((at, tab, i)) if tab == app.tab && i == index
                    && app.now.saturating_duration_since(at) <= DOUBLE_CLICK);
            if app.tab == Tab::Settings {
                app.settings.selected = index;
            }
            let mut cmds = if app.tab == Tab::Settings {
                Vec::new()
            } else {
                app.move_to(index)
            };
            if double {
                app.last_click = None;
                let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
                cmds.extend(on_key(app, enter));
            } else {
                app.last_click = Some((app.now, app.tab, index));
            }
            cmds
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if app.tab != Tab::Settings => {
            if !inside(r.list) {
                return Vec::new();
            }
            if ev.kind == MouseEventKind::ScrollDown {
                app.move_to(app.selected.saturating_add(WHEEL_ITEMS))
            } else {
                app.move_to(app.selected.saturating_sub(WHEEL_ITEMS))
            }
        }
        _ => Vec::new(),
    }
}

fn on_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }
    // A second Ctrl+C once quitting was asked for (dialog open or confirmed) quits at once:
    // the way out if the job in progress hangs.
    let ctrl_c = key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
    let quit_asked = matches!(app.modal, Some(Modal::Confirm { action: None, .. }));
    if ctrl_c && (quit_asked || app.quit_after_running) {
        return vec![Cmd::Quit];
    }
    if app.modal.is_some() {
        return on_modal_key(app, key);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => request_quit(app),
            KeyCode::Char('r') => {
                app.refresh_forced = true;
                vec![Cmd::Refresh { force: true }]
            }
            KeyCode::Char('s') => {
                app.filter = match app.filter {
                    SourceFilter::All => SourceFilter::Winget,
                    SourceFilter::Winget => SourceFilter::Scoop,
                    SourceFilter::Scoop => SourceFilter::All,
                };
                app.view_changed()
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
    if app.tab == Tab::Settings
        && let Some(cmds) = settings_key(app, key)
    {
        return cmds;
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
        KeyCode::Tab | KeyCode::Right => app.set_tab(next_tab(app.tab)),
        KeyCode::BackTab | KeyCode::Left => app.set_tab(prev_tab(app.tab)),
        KeyCode::Enter => {
            if app.visible_len() == 0 {
                return Vec::new();
            }
            match app.tab {
                Tab::Search => install_selected(app),
                Tab::Updates => upgrade_selected(app),
                Tab::Installed => open_actions(app),
                Tab::Settings => Vec::new(),
            }
        }
        KeyCode::Esc => {
            // An error toast stays until dismissed: Esc closes the newest and does nothing else.
            if let Some(i) = app.toasts.iter().position(|t| t.kind == ToastKind::Error) {
                app.toasts.remove(i);
                Vec::new()
            } else if !empty {
                app.query.clear();
                app.view_changed()
            } else {
                request_quit(app)
            }
        }
        KeyCode::Char('i') if empty && app.tab == Tab::Search => install_selected(app),
        KeyCode::Char('u') if empty && app.tab == Tab::Installed => remove_selected(app),
        KeyCode::Char('U') if empty && app.tab == Tab::Updates => upgrade_selected(app),
        KeyCode::Char('o') if empty && matches!(app.tab, Tab::Installed | Tab::Updates) => {
            app.cycle_sort();
            app.details_cmd()
        }
        KeyCode::Char('q') if empty => request_quit(app),
        KeyCode::Char('f') if empty => open_queue(app),
        KeyCode::Char('?') if empty => {
            app.show_help = true;
            Vec::new()
        }
        KeyCode::Backspace => {
            app.query.pop();
            app.view_changed()
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::ALT) => {
            app.query.push(c);
            app.view_changed()
        }
        _ => Vec::new(),
    }
}

/// Keys of the Settings tab; `None` leaves the key to the common handling (tabs, Esc).
fn settings_key(app: &mut App, key: KeyEvent) -> Option<Vec<Cmd>> {
    let last = crate::tui::settings::ORDER.len() - 1;
    Some(match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            app.settings.selected = (app.settings.selected + 1).min(last);
            Vec::new()
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.settings.selected = app.settings.selected.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Enter | KeyCode::Char(' ' | 'l') => change_setting(app, true),
        KeyCode::Char('h') => change_setting(app, false),
        KeyCode::Char('q') => request_quit(app),
        KeyCode::Char('f') => open_queue(app),
        KeyCode::Char('?') => {
            app.show_help = true;
            Vec::new()
        }
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right | KeyCode::Esc => {
            return None;
        }
        _ => Vec::new(),
    })
}

/// Cycles the selected setting, applies it at once and asks for it to be saved.
fn change_setting(app: &mut App, forward: bool) -> Vec<Cmd> {
    if !app.settings.cycle(forward) {
        return Vec::new();
    }
    app.glyphs = app.settings.glyphs;
    app.dark = app.settings.dark;
    let mut cmds = Vec::new();
    match app.settings.current_id() {
        SettingId::Glyphs => app.settings.glyphs_touched = true,
        SettingId::ListIcons => {
            app.settings.list_icons_touched = true;
            // `update` asks for the visible rows' icons right after this.
            app.list_icons_visible = app.settings.list_icons == crate::config::LIST_ICONS[0];
        }
        SettingId::Mouse => {
            app.mouse_enabled = app.settings.behavior.mouse;
            cmds.push(Cmd::SetMouseCapture(app.mouse_enabled));
        }
        SettingId::Icons => {
            app.settings.icons_touched = true;
            apply_icons(app);
            // Under the new protocol the selected icon is encoded again.
            cmds.extend(app.icon_cmd());
        }
        _ => {}
    }
    cmds.extend(request_save(app));
    cmds
}

/// Makes the protocol of `settings.icons` the live one and forgets everything encoded or
/// decided under the old one, so icons are requested again.
fn apply_icons(app: &mut App) {
    app.icon_protocol = choose_from_env(&app.settings.icons);
    app.icons = IconCache::default();
    app.icon_gen += 1;
    app.icon_in_flight.clear();
    app.icon_none.clear();
}

/// Starts a save unless one is in flight (then its end sends the newest values).
fn request_save(app: &mut App) -> Vec<Cmd> {
    if app.save_in_flight.is_some() {
        return Vec::new();
    }
    let s = &app.settings;
    app.save_in_flight = Some((s.behavior, s.glyphs, s.dark));
    app.icons_in_flight = Some(s.icons.clone());
    app.list_icons_in_flight = Some(s.list_icons.clone());
    app.status = Some("salvando…".to_string());
    vec![Cmd::SaveSettings {
        behavior: s.behavior,
        glyphs: (s.glyphs_touched && !s.glyphs_forced).then(|| glyphs_name(s.glyphs).to_string()),
        icons: s.icons_touched.then(|| s.icons.clone()),
        list_icons: s.list_icons_touched.then(|| s.list_icons.clone()),
        theme: if s.dark { "dark" } else { "light" }.to_string(),
    }]
}

const NOT_OURS_NOTE: &str = "instalado fora do wingetrs";
const NOT_OURS: &str = "só apps instalados pelo wingetrs podem ser removidos ou atualizados aqui";

/// Marks a plan as in flight; `None` when one already is.
fn start_planning(app: &mut App, cmd: Cmd) -> Vec<Cmd> {
    if app.planning {
        return Vec::new();
    }
    app.planning = true;
    app.planning_keep = match cmd {
        Cmd::PlanInstall { keep, .. } | Cmd::PlanUpgradeAll { keep } => keep,
        _ => false,
    };
    app.planning_confirm = Some(app.settings.behavior.confirm);
    app.status = Some("planejando…".to_string());
    vec![cmd]
}

/// `i`: plan installing the selected package (scoop only).
fn install_selected(app: &mut App) -> Vec<Cmd> {
    let Some(hit) = app.hits.get(app.selected) else {
        return Vec::new();
    };
    let Some(e) = app.search.entries().get(hit.index) else {
        return Vec::new();
    };
    if e.source != SourceKind::Scoop {
        app.push_toast(ToastKind::Warning, "instalação na fase 2d");
        return Vec::new();
    }
    // A new package has no old version to keep.
    let cmd = Cmd::PlanInstall {
        source_name: e.source_name.clone(),
        id: e.id.clone(),
        keep: false,
    };
    start_planning(app, cmd)
}

/// `u`: plan removing the selected app, which must be one wingetrs installed.
fn remove_selected(app: &mut App) -> Vec<Cmd> {
    let name = match app.installed_view().get(app.selected) {
        None => return Vec::new(),
        Some(r) if r.origin == "wingetrs" => r.name.clone(),
        Some(_) => {
            app.push_toast(ToastKind::Warning, NOT_OURS);
            return Vec::new();
        }
    };
    plan_remove(app, name)
}

/// The one rule for who can be upgraded: ours, matched to a catalog package, with a newer version.
/// `Err` carries the reason, as the note of a disabled menu item.
fn upgrade_target(r: &ListRow) -> Result<(String, String), &'static str> {
    if r.origin != "wingetrs" {
        return Err(NOT_OURS_NOTE);
    }
    match (&r.correlation, &r.state) {
        (
            Correlation::Matched {
                source, package_id, ..
            },
            UpdateState::Available { .. },
        ) => Ok((source.clone(), package_id.clone())),
        _ => Err("sem atualização"),
    }
}

fn plan_upgrade(app: &mut App, source_name: String, id: String) -> Vec<Cmd> {
    let keep = app.settings.behavior.keep_old;
    start_planning(
        app,
        Cmd::PlanInstall {
            source_name,
            id,
            keep,
        },
    )
}

fn plan_remove(app: &mut App, name: String) -> Vec<Cmd> {
    let purge = app.settings.behavior.purge;
    start_planning(app, Cmd::PlanRemove { app: name, purge })
}

/// `U`: plan upgrading the selected app; with nothing selected, every upgradable app.
fn upgrade_selected(app: &mut App) -> Vec<Cmd> {
    let keep = app.settings.behavior.keep_old;
    let target = match app.installed_view().get(app.selected) {
        None => return start_planning(app, Cmd::PlanUpgradeAll { keep }),
        Some(r) => upgrade_target(r),
    };
    match target {
        Ok((source_name, id)) => plan_upgrade(app, source_name, id),
        Err(_) => {
            app.push_toast(ToastKind::Warning, NOT_OURS);
            Vec::new()
        }
    }
}

/// `Enter` in Installed: the actions menu of the selected app.
fn open_actions(app: &mut App) -> Vec<Cmd> {
    let Some(r) = app.installed_view().get(app.selected).copied() else {
        return Vec::new();
    };
    let upgrade = upgrade_target(r);
    let remove = (r.origin == "wingetrs").then(|| r.name.clone());
    let items = vec![
        PickerItem {
            label: "Atualizar".to_string(),
            detail: String::new(),
            selectable: upgrade.is_ok(),
            note: upgrade.as_ref().err().map(|n| (*n).to_string()),
        },
        PickerItem {
            label: "Remover".to_string(),
            detail: String::new(),
            selectable: remove.is_some(),
            note: remove.is_none().then(|| NOT_OURS.to_string()),
        },
    ];
    let name = r.name.clone();
    app.modal = Some(Modal::Actions {
        picker: Picker::single_at(items, 0),
        app: name,
        upgrade: upgrade.ok(),
        remove,
    });
    Vec::new()
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

/// Whether a ready plan asks before executing, per the `confirm` setting.
pub fn needs_confirm(mode: ConfirmMode, action: &Action) -> bool {
    match mode {
        ConfirmMode::Always => true,
        ConfirmMode::Removals => matches!(action, Action::Remove(_)),
        ConfirmMode::Never => false,
    }
}

impl App {
    fn next_id(&mut self) -> u64 {
        self.next_op_id += 1;
        self.next_op_id
    }
}

/// Shows `modal`, or keeps it for when the dialog on screen closes.
fn open_modal(app: &mut App, modal: Modal) {
    if app.modal.is_some() {
        app.pending_modals.push_back(modal);
    } else {
        app.modal = Some(modal);
    }
}

/// A residues dialog is open or waiting to be: the queue does not go on until it closes.
fn residues_pending(app: &App) -> bool {
    matches!(app.modal, Some(Modal::Residues { .. }))
        || app
            .pending_modals
            .iter()
            .any(|m| matches!(m, Modal::Residues { .. }))
}

fn can_start(app: &App) -> bool {
    app.running.is_none() && !app.waiting_lock && !residues_pending(app)
}

fn start(app: &mut App, q: Queued) -> Vec<Cmd> {
    app.running = Some(Running {
        id: q.id,
        label: q.label,
        progress: None,
        action: Some(q.action.clone()),
    });
    vec![Cmd::Execute(q.action)]
}

/// A confirmed action (Enter on the dialog, or no confirmation needed): starts now when
/// nothing is in the way, else waits at the end of the queue.
fn enqueue(app: &mut App, action: Action) -> Vec<Cmd> {
    let q = Queued {
        id: app.next_id(),
        label: action.label(),
        action,
    };
    if app.quit_after_running {
        app.push_toast(ToastKind::Warning, "saindo: operação não enfileirada");
        return Vec::new();
    }
    if can_start(app) && app.queue.is_empty() {
        return start(app, q);
    }
    let label = q.label.clone();
    app.queue.push_back(q);
    let n = app.queue.len();
    app.push_toast(ToastKind::Info, format!("na fila: {label} (posição {n})"));
    Vec::new()
}

/// Next step once the way may be clear: quit if asked to, else start the next in line.
fn pump(app: &mut App) -> Vec<Cmd> {
    if app.quit_after_running && app.running.is_none() {
        return vec![Cmd::Quit];
    }
    if !can_start(app) {
        return Vec::new();
    }
    match app.queue.pop_front() {
        Some(q) => start(app, q),
        None => Vec::new(),
    }
}

/// `q`, `Esc` at the root or Ctrl+C: quit now, or ask first when work is in progress or queued.
fn request_quit(app: &mut App) -> Vec<Cmd> {
    if app.running.is_none() && app.queue.is_empty() {
        return vec![Cmd::Quit];
    }
    let n = app.queue.len();
    let mut plan_text = match (&app.running, n) {
        (Some(_), 0) => "Há uma operação em andamento; ela termina antes de fechar.".to_string(),
        (Some(_), _) => format!(
            "Há {n} operações na fila e uma em andamento. Sair cancela as pendentes; a em andamento termina antes de fechar."
        ),
        (None, _) => format!("Há {n} operações na fila. Sair cancela as pendentes."),
    };
    // Quitting right after a removal skips its residues dialog.
    let removing = matches!(
        app.running,
        Some(Running {
            action: Some(Action::Remove(_)),
            ..
        })
    );
    if removing && app.settings.behavior.residues == ResidueMode::Ask {
        plan_text.push_str(" Resíduos dela não serão limpos.");
    }
    app.modal = Some(Modal::Confirm {
        plan_text,
        action: None,
    });
    Vec::new()
}

/// Rows of the queue dialog with their op ids: the operation in progress (not selectable),
/// then the queue in order.
pub fn queue_rows(app: &App) -> Vec<(u64, PickerItem)> {
    let row = |id: u64, label: String, selectable: bool| {
        let item = PickerItem {
            label,
            detail: String::new(),
            selectable,
            note: None,
        };
        (id, item)
    };
    let running = app
        .running
        .iter()
        .map(|r| row(r.id, format!("em andamento: {}", r.label), false));
    let queued = app.queue.iter().map(|q| row(q.id, q.label.clone(), true));
    running.chain(queued).collect()
}

/// `f`: the queue dialog, cursor on the first row.
fn open_queue(app: &mut App) -> Vec<Cmd> {
    match queue_rows(app).first() {
        Some((id, _)) => app.modal = Some(Modal::Queue { selected: *id }),
        None => app.push_toast(ToastKind::Info, "fila vazia"),
    }
    Vec::new()
}

/// Keys of the queue dialog; it stays open until `Esc` or `f`.
fn queue_key(app: &mut App, selected: u64, key: KeyEvent) -> Vec<Cmd> {
    let rows = queue_rows(app);
    let at = rows.iter().position(|(id, _)| *id == selected).unwrap_or(0);
    let (ids, items): (Vec<u64>, Vec<PickerItem>) = rows.into_iter().unzip();
    let selected = match key.code {
        KeyCode::Char('f') => return Vec::new(),
        KeyCode::Delete | KeyCode::Char('x') => {
            // Only pending rows are in the queue: the one in progress is never removed.
            let Some(q) = app
                .queue
                .iter()
                .position(|q| q.id == selected)
                .and_then(|i| app.queue.remove(i))
            else {
                app.modal = Some(Modal::Queue { selected });
                return Vec::new();
            };
            app.push_toast(ToastKind::Info, format!("removido da fila: {}", q.label));
            // The cursor stays on the same line, now the next row.
            let ids: Vec<u64> = queue_rows(app).into_iter().map(|(id, _)| id).collect();
            match ids.get(at.min(ids.len().saturating_sub(1))) {
                Some(&id) => id,
                None => return Vec::new(),
            }
        }
        _ => {
            let mut picker = Picker::single_at(items, at);
            if picker.key(key) == PickOutcome::Cancelled {
                return Vec::new();
            }
            ids.get(picker.selected()).copied().unwrap_or(selected)
        }
    };
    app.modal = Some(Modal::Queue { selected });
    Vec::new()
}

fn on_planned(app: &mut App, result: Result<Planned, String>) -> Vec<Cmd> {
    app.planning = false;
    app.status = None;
    let confirm = app
        .planning_confirm
        .take()
        .unwrap_or(app.settings.behavior.confirm);
    match result {
        Ok(Planned::Ready { text, action }) => {
            if !needs_confirm(confirm, &action) {
                return enqueue(app, action);
            }
            open_modal(
                app,
                Modal::Confirm {
                    plan_text: text,
                    action: Some(action),
                },
            );
        }
        Ok(Planned::Choose(candidates)) => {
            let items = candidates.iter().map(candidate_item).collect();
            let keep = app.planning_keep;
            open_modal(
                app,
                Modal::Picking {
                    picker: Picker::single(items),
                    candidates,
                    keep,
                },
            );
        }
        Err(message) => app.push_toast(ToastKind::Error, message),
    }
    Vec::new()
}

fn on_finished(app: &mut App, result: Result<Done, InstallFail>) -> Vec<Cmd> {
    let finished = app.running.take();
    // Success or not, the disk may have changed.
    let mut cmds = vec![Cmd::ScanInstalled];
    match result {
        Ok(done) => {
            app.push_toast(ToastKind::Success, done.summary);
            if let Some((name, residues)) = done.residues
                && !residues.is_empty()
            {
                cmds.extend(handle_residues(app, name, residues));
            }
        }
        Err(InstallFail::Busy) => {
            app.push_toast(ToastKind::Warning, "outra instalação em andamento");
            // It never ran: first in line again, tried once the other process may be done.
            if let Some(Running {
                id,
                label,
                action: Some(action),
                ..
            }) = finished
            {
                app.queue.push_front(Queued { id, label, action });
            }
            app.waiting_lock = true;
            cmds.push(Cmd::RetryQueue);
        }
        Err(InstallFail::Other(message)) => app.push_toast(ToastKind::Error, message),
    }
    cmds.extend(pump(app));
    cmds
}

/// Runs the cleanup of `residues` as the operation in progress.
fn start_cleanup(app: &mut App, name: String, residues: Vec<Residue>) -> Vec<Cmd> {
    app.running = Some(Running {
        id: app.next_id(),
        label: format!("Limpando resíduos de {name}"),
        progress: None,
        action: None,
    });
    vec![Cmd::CleanResidues {
        app: name,
        residues,
    }]
}

/// Leftovers after a removal, per the `residues` setting.
fn handle_residues(app: &mut App, name: String, residues: Vec<Residue>) -> Vec<Cmd> {
    let mode = app.settings.behavior.residues;
    if mode == ResidueMode::Ask {
        let items: Vec<PickerItem> = residues.iter().map(residue_item).collect();
        let initial: Vec<bool> = residues.iter().map(|r| r.preselected).collect();
        open_modal(
            app,
            Modal::Residues {
                picker: Picker::multi(items, &initial),
                app: name,
                residues,
            },
        );
        return Vec::new();
    }
    let n = residues.len();
    let chosen: Vec<Residue> = match mode {
        ResidueMode::High => crate::install::residue::auto_clean(&residues),
        _ => Vec::new(),
    };
    if chosen.is_empty() {
        app.push_toast(ToastKind::Info, format!("resíduos mantidos: {n} itens"));
        return Vec::new();
    }
    start_cleanup(app, name, chosen)
}

fn on_modal_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    let cmds = modal_key(app, key);
    if app.modal.is_none() {
        app.modal = app.pending_modals.pop_front();
    }
    cmds
}

fn modal_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    let Some(modal) = app.modal.take() else {
        return Vec::new();
    };
    match modal {
        Modal::Confirm { plan_text, action } => match key.code {
            KeyCode::Enter => match action {
                Some(action) => enqueue(app, action),
                None => {
                    // Quit: the pending ones are dropped, the one in progress is waited for.
                    app.queue.clear();
                    app.quit_after_running = true;
                    pump(app)
                }
            },
            KeyCode::Esc => Vec::new(),
            _ => {
                app.modal = Some(Modal::Confirm { plan_text, action });
                Vec::new()
            }
        },
        Modal::Picking {
            mut picker,
            candidates,
            keep,
        } => match picker.key(key) {
            PickOutcome::Pending => {
                app.modal = Some(Modal::Picking {
                    picker,
                    candidates,
                    keep,
                });
                Vec::new()
            }
            PickOutcome::Cancelled => Vec::new(),
            PickOutcome::Picked(chosen) => match chosen.first().and_then(|&i| candidates.get(i)) {
                Some(c) if c.selectable => {
                    let cmd = Cmd::PlanInstall {
                        source_name: c.entry.source_name.clone(),
                        id: c.entry.id.clone(),
                        keep,
                    };
                    start_planning(app, cmd)
                }
                _ => Vec::new(),
            },
        },
        Modal::Actions {
            mut picker,
            app: name,
            upgrade,
            remove,
        } => match picker.key(key) {
            PickOutcome::Pending => {
                app.modal = Some(Modal::Actions {
                    picker,
                    app: name,
                    upgrade,
                    remove,
                });
                Vec::new()
            }
            PickOutcome::Cancelled => Vec::new(),
            PickOutcome::Picked(chosen) => match (chosen.first(), upgrade, remove) {
                (Some(0), Some((source_name, id)), _) => plan_upgrade(app, source_name, id),
                (Some(1), _, Some(target)) => plan_remove(app, target),
                _ => Vec::new(),
            },
        },
        Modal::Queue { selected } => queue_key(app, selected, key),
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
                app.push_toast(ToastKind::Info, "resíduos mantidos");
                pump(app)
            }
            PickOutcome::Picked(chosen) => {
                let selected: Vec<Residue> = chosen
                    .iter()
                    .filter_map(|&i| residues.get(i).cloned())
                    .collect();
                if selected.is_empty() {
                    app.push_toast(ToastKind::Info, "resíduos mantidos");
                    return pump(app);
                }
                start_cleanup(app, name, selected)
            }
        },
    }
}

/// Next tab in the cycle (single place to extend when a tab is added).
pub fn next_tab(tab: Tab) -> Tab {
    match tab {
        Tab::Search => Tab::Installed,
        Tab::Installed => Tab::Updates,
        Tab::Updates => Tab::Settings,
        Tab::Settings => Tab::Search,
    }
}

/// Previous tab in the cycle.
pub fn prev_tab(tab: Tab) -> Tab {
    match tab {
        Tab::Search => Tab::Settings,
        Tab::Installed => Tab::Search,
        Tab::Updates => Tab::Installed,
        Tab::Settings => Tab::Updates,
    }
}
