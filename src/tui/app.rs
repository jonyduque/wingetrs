//! Elm-style TUI state: `update` is pure and performs no I/O; side effects are returned as `Cmd`.
use crate::bridge::{self, BridgeJob, BridgeOp, NOT_OURS_NOTE, PINNED_NOTE, Tool, WingetFit};
use crate::catalog::search::{SearchHit, SearchIndex};
use crate::cli::install_cmd::{admin_code_refusal, plans_admin_refusal};
use crate::cli::output::{ThirdParty, plan_third_party};
use crate::cli::winget_cmd::winget_admin_refusal;
use crate::config::{Behavior, Config, ConfirmMode, ResidueMode, WingetEngine};
use crate::core::{PackageDetails, PackageEntry, SourceKind};
use crate::install::plan::{InstallPlan, UninstallPlan};
use crate::install::residue::{Confidence, Residue, ResidueKind};
use crate::install::script_ast::AstGate;
use crate::install::script_policy::elevated_or_unknown;
use crate::install::script_scan::{ScriptReport, Verdict, printable};
use crate::install::target::Candidate;
use crate::installed::correlate::{Correlation, InstallScope, ListRow, Rule, UpdateState};
use crate::installed::overrides::Choice;
use crate::tui::icons::{
    APP_ICON_KEY, APP_ICON_PNG, APP_ICON_SIZE, IconCache, IconProtocol, choose_from_env,
};
use crate::tui::layout::{
    DETAIL_ICON_COLS, DETAIL_ICON_ROWS, FILTER_GAP, ITEM_GAP, LIST_ICON_COLS,
};
use crate::tui::picker::{PickOutcome, Picker, PickerItem};
use crate::tui::settings::{SettingId, SettingsState, glyphs_name};
use crate::tui::theme::GlyphSet;
use crate::update::{Outcome, SourceReport};
use crate::winget_install::engine::{Engine, choose_engine};
use crate::winget_install::orphans::Orphan;
use crate::winget_install::plan::{PlanOrBridge, WingetPlan};
use crate::winget_install::select::Refusal;
use crate::winget_install::uninstall::{
    UninstallPlan as WingetUninstallPlan, UninstallTarget, target_of,
};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// Which Installed rows to show by how well they matched the catalog (`ctrl+e`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShowFilter {
    #[default]
    All,
    Unmatched,
    Duplicates,
}

/// How one attempt to get a package's icon ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconOutcome {
    /// The icon was obtained.
    Found,
    /// The package has no icon to get (none declared, refused, or gone): not asked again.
    Absent,
    /// Failed for a reason that may pass (offline, server error, cache): asked again later.
    Retry,
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
    /// Whether winget would take the upgrade of `app_key` to `latest`; `Err` (network, bad
    /// manifest) leaves the row as it is without a verdict.
    ApplicabilityLoaded {
        app_key: String,
        latest: String,
        result: Result<WingetFit, String>,
    },
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
    /// Result of writing the user's catalog match; a rescan follows on success.
    OverrideSaved {
        choice: Option<Choice>,
        result: Result<(), String>,
    },
    /// Result of writing `key` (un)pinned; the `App` already applied it and reverts on `Err`.
    PinSaved {
        key: String,
        pinned: bool,
        result: Result<(), String>,
    },
    /// An icon was just cached: `key` now has its blob under `sha`.
    IconMapped {
        key: String,
        sha: String,
    },
    /// An icon fetch of `key` ended (list prefetch or the selected package's details);
    /// `outcome` says whether it is asked again.
    PrefetchDone {
        key: String,
        outcome: IconOutcome,
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
    /// Result of `Cmd::PlanPortable`, with one note per license agreement of the package.
    PortablePlanned {
        result: Result<Planned, String>,
        agreements: Vec<String>,
    },
    /// Result of `Cmd::PlanWingetInstall`: the bridge job and one note per license agreement
    /// (`Licença: <label> — <url>`).
    WingetPlanned(Result<(BridgeJob, Vec<String>), String>),
    /// Result of `Cmd::PlanWingetInstall`: the bridge job (with its license notes) and the
    /// native planner's answer; the Settings engine chooses between them (`choose_engine`).
    WingetInstallPlanned {
        bridge: Result<(BridgeJob, Vec<String>), String>,
        native: Result<PlanOrBridge, String>,
    },
    /// Result of `Cmd::PlanWingetUninstall`.
    WingetUninstallPlanned(Result<WingetUninstallPlan, String>),
    /// The native upgrade of `app_key` to `latest`, sent after its applicability when the
    /// native planner would run it.
    NativeUpgradeLoaded {
        app_key: String,
        latest: String,
        plan: WingetPlan,
    },
    /// Results of a `Cmd::PlanBatch`, in the order of its items.
    BatchPlanned(Vec<Result<Planned, String>>),
    /// The background check found a release newer than this build (`tag` is `vX.Y.Z`; older or
    /// equal ones are dropped).
    SelfUpdateAvailable {
        tag: String,
    },
    /// Download/extract progress of the running operation.
    InstallProgress {
        label: String,
        done: u64,
        total: Option<u64>,
    },
    /// The running operation launched `msiexec /a` (writing its log at the path): from now on
    /// it cannot be cancelled.
    InstallerRunning(std::path::PathBuf),
    /// The running operation entered a slow step (the installer launch is `InstallerRunning`).
    InstallPhase(crate::install::progress::Phase),
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
    /// Startup check of the bridge tools: `winget --version` runs, `scoop.ps1` exists.
    ToolsDetected {
        winget: bool,
        scoop: bool,
    },
    /// Newest output line of the bridge upgrade in progress.
    BridgeLine(String),
    /// Answer of `Cmd::LoadBuckets`: the configuration read from disk and its buckets.
    BucketsLoaded(Result<(Config, Vec<BucketInfo>), String>),
}

/// What the user is asked to confirm, or to choose from.
// Matched once when the plan arrives; boxing the action would only change its shape.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Planned {
    Ready {
        text: String,
        action: Action,
    },
    /// A removal whose app runs third-party code (read from its recorded manifest).
    ReadyScripts {
        text: String,
        action: Action,
        code: Vec<ThirdParty>,
        /// The record's hash of the manifest the code was read from.
        manifest_sha256: String,
    },
    /// The text matched more than one package.
    Choose(Vec<Candidate>),
}

/// A confirmed operation, executed by the runtime under `install.lock`.
// Built once per confirmation and moved along; boxing would only change the brief's shape.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// One plan list per app (dependencies first), applied in order.
    Apply(Vec<Vec<InstallPlan>>),
    Remove(UninstallPlan),
    /// Upgrade through the tool that installed the app (winget or scoop).
    Bridge(BridgeJob),
    /// Like `Apply`, after the user allowed `msiexec /a` for `app` (and only for it).
    ApplyMsiexec {
        groups: Vec<Vec<InstallPlan>>,
        app: String,
        /// The groups came from a confirmed `ApplyScripts`: the consent to their code goes along.
        scripts: bool,
    },
    /// Like `Apply`, after the user read the third-party code of the plans in the
    /// confirmation and allowed it. Built only by that confirmation.
    ApplyScripts {
        groups: Vec<Vec<InstallPlan>>,
    },
    /// Like `Remove`, after the user allowed the app's uninstall code (hooks, uninstaller).
    /// `manifest_sha256` is the record's hash of the manifest whose code the user read; the
    /// removal is refused if the record changed since.
    RemoveScripts {
        plan: UninstallPlan,
        manifest_sha256: String,
    },
    /// Native install of a winget package.
    WingetInstall(WingetPlan),
    /// Native uninstall of an ARP or MSIX record (with the folder backup when
    /// `shared.folders` is not empty).
    WingetUninstall(WingetUninstallPlan),
    /// Option C of the shared-data case: removes only the registry record (no uninstaller).
    WingetRegistryOnly(WingetUninstallPlan),
    /// Native upgrade of the installed record `row_key` (`overrides::app_key`).
    WingetUpgrade {
        plan: WingetPlan,
        row_key: String,
    },
    /// Exports and deletes an ARP record whose uninstaller is gone.
    RemoveOrphan(Orphan),
    /// Replaces the running wingetrs by the release `tag` (the same update as `self update`).
    SelfUpdate {
        tag: String,
    },
    /// Adds the scoop bucket `name` from `spec` and downloads it (`wingetrs bucket add`);
    /// `confirmed`: the user accepted a bucket wingetrs does not know.
    BucketAdd {
        name: String,
        spec: String,
        confirmed: bool,
    },
    /// Removes the scoop bucket `name` (`wingetrs bucket rm`); `confirmed`: the user accepted
    /// removing a default bucket.
    BucketRemove {
        name: String,
        confirmed: bool,
    },
}

impl Action {
    pub fn label(&self) -> String {
        match self {
            Action::Apply(groups) => {
                let apps: Vec<&str> = groups.iter().flatten().map(|p| p.app.as_str()).collect();
                format!("Instalando {}", apps.join(", "))
            }
            Action::ApplyMsiexec { groups, .. } => {
                let apps: Vec<&str> = groups.iter().flatten().map(|p| p.app.as_str()).collect();
                format!("Instalando {} (Windows Installer)", apps.join(", "))
            }
            Action::ApplyScripts { groups } => {
                let apps: Vec<&str> = groups.iter().flatten().map(|p| p.app.as_str()).collect();
                format!("Instalando {}", apps.join(", "))
            }
            Action::Remove(p) | Action::RemoveScripts { plan: p, .. } => {
                format!("Removendo {}", p.app)
            }
            Action::Bridge(job) => match job.op {
                BridgeOp::Install { .. } => {
                    format!("Instalando {} via {}", job.name, job.tool.name())
                }
                BridgeOp::Upgrade => {
                    format!("Atualizando {} via {}", job.name, job.tool.name())
                }
            },
            Action::WingetInstall(p) => format!("Instalando {}", p.name),
            Action::WingetUninstall(p) => format!("Removendo {}", p.name),
            Action::WingetRegistryOnly(p) => format!("Removendo o registro de {}", p.name),
            Action::WingetUpgrade { plan, .. } => format!("Atualizando {}", plan.name),
            Action::RemoveOrphan(o) => format!("Removendo registro órfão de {}", o.name),
            Action::SelfUpdate { .. } => "Atualizando o wingetrs".to_string(),
            Action::BucketAdd { name, .. } => format!("Adicionando bucket {name}"),
            Action::BucketRemove { name, .. } => format!("Removendo bucket {name}"),
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
    /// An MSI of `app` needs `msiexec /a`; `applied` were installed before it (and stay).
    NeedsMsiexec {
        app: String,
        reason: String,
        applied: Vec<String>,
    },
    Other(String),
}

/// Note of the confirmation that authorizes `msiexec /a` for one item.
pub const MSIEXEC_NOTE: &str = "Usa o Windows Installer (msiexec /a). Não pode ser cancelado depois de começar; grava arquivos em C:\\Windows\\Installer.";

/// Text of the quit dialog while a job that may run `msiexec /a` is in progress.
pub const MSIEXEC_QUIT: &str =
    "O Windows Installer ainda está trabalhando; o wingetrs vai esperar ele terminar.";

/// Text of the quit dialog once a native winget installer or uninstaller was launched.
pub const NATIVE_QUIT: &str =
    "O instalador ou desinstalador já foi iniciado; o wingetrs vai esperar ele terminar.";

/// Download/extract progress shown while an operation runs.
#[derive(Debug, Clone, PartialEq)]
pub struct RunProgress {
    pub label: String,
    /// The step under way and when it started (`App::now`); `None` while downloading.
    pub phase: Option<(crate::install::progress::Phase, Instant)>,
    /// The downloads since the last phase, with the samples their speed comes from.
    pub meter: crate::install::progress::DownloadMeter,
}

/// Progress of a step that started at `now` (a new phase starts a new meter).
fn phase_progress(phase: crate::install::progress::Phase, now: Instant) -> RunProgress {
    RunProgress {
        label: String::new(),
        phase: Some((phase, now)),
        meter: Default::default(),
    }
}

/// Key of the TUI's download meter: the throttle already sums the downloads of an action.
const METER_KEY: &str = "";

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

/// Silence of a bridge upgrade after which the progress says it may be waiting for the user.
pub const BRIDGE_QUIET: Duration = Duration::from_secs(600);

/// Progress text of a bridge upgrade silent for `BRIDGE_QUIET`.
pub const BRIDGE_WAITING: &str =
    "ainda aguardando o instalador (pode haver uma janela ou pedido do UAC aberto)";

/// How an installed app is upgraded: by wingetrs (`(source_name, id)` to plan) or by the tool
/// that installed it.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Ours(String, String),
    /// Boxed: the job carries two version strings and `Modal` would grow.
    Bridge(Box<BridgeJob>),
    /// Native winget upgrade of the record `row_key` (engine `wingetrs`, native plan known).
    Native {
        plan: Box<WingetPlan>,
        row_key: String,
    },
}

/// Whether the bridge tools exist; `None` until the startup check answers (enabled meanwhile:
/// a missing tool then fails with "não encontrado").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tools {
    pub winget: Option<bool>,
    pub scoop: Option<bool>,
}

/// One package of a confirmation: a group of an `Apply`, a removal or a bridge upgrade.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmItem {
    pub name: String,
    /// `(source, id)` of its icon.
    pub key: Option<(String, String)>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// `"wingetrs"`, `"winget"` or `"scoop"`.
    pub manager: &'static str,
    pub checked: bool,
}

/// The two buttons under the confirmation list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmButton {
    Continue,
    Cancel,
}

/// Labels of the confirmation buttons, in drawing order.
pub const CONFIRM_BUTTONS: [(&str, ConfirmButton); 2] = [
    ("Continuar (enter)", ConfirmButton::Continue),
    ("Cancelar (esc)", ConfirmButton::Cancel),
];

/// How the view paints a line of the scripts section (theme tokens only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptLineKind {
    /// The app the code below belongs to.
    App,
    Header,
    Info,
    Flag,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLine {
    pub kind: ScriptLineKind,
    pub text: String,
}

/// The scrollable third-party scripts of a confirmation, built once when it opens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptSection {
    pub lines: Vec<ScriptLine>,
    /// First line on screen.
    pub scroll: usize,
    /// Lowercase apps that run third-party code.
    pub apps: Vec<String>,
    /// Lowercase apps with a script the gate refuses (never run).
    pub refused: Vec<String>,
    /// `(lowercase app, manifest hash)` of the removals shown.
    pub shas: Vec<(String, String)>,
}

/// One bucket of the buckets dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct BucketInfo {
    pub row: crate::buckets::BucketRow,
    /// Seconds since its last download when the list was read.
    pub age: Option<i64>,
    /// Apps installed from it (`state.db`); `Err` when that could not be read, which refuses
    /// the removal like the CLI.
    pub installed: Result<Vec<String>, String>,
}

impl BucketInfo {
    /// The repo is a well-known bucket's: adding it needs no confirmation.
    pub fn known(&self) -> bool {
        crate::buckets::is_known_repo(&self.row.repo)
    }
}

/// Free-form item of the add picker.
pub const OTHER_REPO: &str = "Outro repositório do GitHub…";

/// Warning of a bucket wingetrs does not know.
pub const THIRD_PARTY_BUCKET: &str = "bucket de terceiro: código e binários de terceiros";

/// What the buckets dialog is showing.
pub enum BucketStage {
    List,
    /// `a`: the well-known buckets not configured (`names`), then `OTHER_REPO`.
    Add {
        picker: Picker,
        names: Vec<String>,
    },
    /// `owner/repo` or GitHub URL being typed.
    Typing {
        text: String,
    },
    /// A bucket wingetrs does not know: Enter adds it anyway.
    ThirdParty {
        name: String,
        repo: String,
    },
    Remove {
        name: String,
    },
    /// Second question for a default bucket (`main`, `extras`).
    RemoveDefault {
        name: String,
    },
    /// The removal is refused: apps from the bucket are installed (or `state.db` is unreadable,
    /// then `apps` holds the reason).
    InUse {
        name: String,
        apps: Vec<String>,
    },
}

/// The buckets dialog (`b` in Settings).
pub struct BucketsDialog {
    /// `config.toml` as read with the list; the add rule (`buckets::add`) runs on a copy.
    pub cfg: Config,
    pub rows: Vec<BucketInfo>,
    pub cursor: usize,
    /// The list is being read.
    pub loading: bool,
    pub stage: BucketStage,
}

/// The dialog on top of the screen; it takes every key until it closes.
pub enum Modal {
    Confirm {
        /// One per package, in the order of `actions` (an `Apply` has one per group).
        items: Vec<ConfirmItem>,
        /// Bridge warnings and what the batch left out.
        notes: Vec<String>,
        /// First item on screen.
        scroll: usize,
        cursor: usize,
        /// Queued in order on Enter, without the unchecked items.
        actions: Vec<Action>,
        /// Tab whose marks are cleared once the actions are queued.
        clear_marks: Option<Tab>,
        /// The third-party code of the plans, shown once; empty when there is none.
        scripts: ScriptSection,
    },
    /// Quitting with work in progress or queued: Enter drops the queue and quits once the
    /// operation in progress ends.
    ConfirmQuit {
        text: String,
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
        /// How the app is upgraded, when it can be.
        upgrade: Option<Target>,
        /// Name to remove when wingetrs installed the app.
        remove: Option<String>,
        /// `(pin key, pin it?)` of the third item; `None` when the menu acts on marked rows.
        pin: Option<(String, bool)>,
        /// The tab has marked rows: the items act on them, not on `app`.
        marked: bool,
        /// Correlation item of the row.
        choice: Option<RowChoice>,
        /// What each picker item does (empty when `marked`).
        ids: Vec<ActionId>,
        /// The row the menu acts on (winget removal and orphan records); `None` when `marked`
        /// or in Search.
        row: Option<Box<ListRow>>,
    },
    Residues {
        picker: Picker,
        app: String,
        residues: Vec<Residue>,
    },
    /// Candidates of an ambiguous installed app, to pick its catalog match; the last item is
    /// "none of these".
    Choosing {
        picker: Picker,
        app_key: String,
        source: String,
        candidates: Vec<String>,
    },
    /// The operation queue; `selected` is the id of the row under the cursor.
    Queue {
        selected: u64,
    },
    Buckets(Box<BucketsDialog>),
}

/// What an item of the single-row actions menu does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionId {
    Install,
    Upgrade,
    Remove,
    Pin,
    Choose,
    UndoChoice,
    ToggleMark,
    /// Remove the record `n` of the row (0: the row's own, `n`: `others[n - 1]`), which is
    /// not the principal one when `n > 0`.
    RemoveRecord(usize),
    /// Remove the orphan record `n` of the row (numbered as `RemoveRecord`).
    RemoveOrphan(usize),
}

/// Extra item of the actions menu that acts on the correlation of the selected installed row.
#[derive(Debug, Clone, PartialEq)]
pub enum RowChoice {
    /// First item: choose among the candidates of an ambiguous row.
    Choose {
        app_key: String,
        source: String,
        candidates: Vec<String>,
    },
    /// Last item: drop the user's choice for this row.
    Undo { app_key: String },
}

/// One row whose winget upgrade is checked (`Cmd::FetchApplicability`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicabilityQuery {
    pub app_key: String,
    pub package_id: String,
    /// Catalog version whose manifest decides.
    pub latest: String,
    pub scope: Option<InstallScope>,
    /// Installed version (the native upgrade plan starts from it).
    pub installed: Option<String>,
}

// Executed right away by the event loop; boxing `Execute` would only change its shape.
#[allow(clippy::large_enum_variant)]
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
    /// Pin or unpin `key` (`<origin>/<name>`) in `pins.json`; answers with `Msg::PinSaved`.
    SetPin {
        key: String,
        pinned: bool,
    },
    /// Store (or, with `None`, drop) the user's catalog match of `app_key` in `correlations.json`;
    /// answers with `Msg::OverrideSaved`.
    SetOverride {
        app_key: String,
        choice: Option<Choice>,
    },
    /// Plan installing (or upgrading) one catalog package.
    /// `keep`: keep the old version when this is an upgrade (setting at key time).
    PlanInstall {
        source_name: String,
        id: String,
        keep: bool,
    },
    /// Read the manifest of winget package `id` and build its install through the bridge;
    /// answers with `Msg::WingetPlanned`.
    PlanWingetInstall {
        id: String,
    },
    /// Plan the winget portable package `spec` as a wingetrs app (Task 189); answers with
    /// `Msg::PortablePlanned`.
    PlanPortable {
        spec: crate::winget_install::portable::PortableSpec,
        keep: bool,
    },
    /// Plan the native uninstall of the installed record `app_key` (`name` for a record that
    /// cannot be read); `others` are the other records of the same app, `is_main` whether this
    /// is the principal one. Answers with `Msg::WingetUninstallPlanned`.
    /// The same request as the CLI's (`RemovalRequest::of_row` for the principal record).
    PlanWingetUninstall(crate::winget_install::previous::RemovalRequest),
    /// `purge`: also delete the app's persistent data (setting at key time).
    PlanRemove {
        app: String,
        purge: bool,
    },
    /// Plan each of these `PlanInstall`/`PlanRemove` in turn, in one job; answers with
    /// `Msg::BatchPlanned`.
    PlanBatch {
        items: Vec<Cmd>,
    },
    /// Plan upgrading every app with a newer catalog version.
    PlanUpgradeAll {
        keep: bool,
    },
    Execute(Action),
    /// Cancel the native winget job in progress through its `LaunchGate` (refused once its
    /// installer launched).
    CancelNative,
    /// Answer with `Msg::RetryQueue` after `BUSY_RETRY`.
    RetryQueue,
    CleanResidues {
        app: String,
        residues: Vec<Residue>,
    },
    /// Fetch the manifest of each row's catalog version and answer, per row, with
    /// `Msg::ApplicabilityLoaded`.
    FetchApplicability {
        rows: Vec<ApplicabilityQuery>,
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
    /// Fetch the icons of these visible winget and scoop rows (details, then icon) off the UI thread;
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
    /// Read `config.toml`, the buckets and their installed apps; answers with
    /// `Msg::BucketsLoaded`.
    LoadBuckets,
    /// Read the catalog into the search index again (`Msg::CatalogLoaded`).
    ReloadCatalog,
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
    pub show: ShowFilter,
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
    /// winget's verdict on upgrading each row (by `app_key`), and the catalog version it is for.
    pub applicability: HashMap<String, (String, WingetFit)>,
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
    /// Pinned apps (`<origin>/<name>`): never upgraded.
    pub pins: std::collections::BTreeSet<String>,
    /// `<source>/<id>` of the list rows whose icon fetch ended in this session.
    icon_tried: HashSet<String>,
    /// `<source>/<id>` known to have no icon in this session: drawn as the placeholder.
    pub icon_absent: HashSet<String>,
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
    /// Version of this build (`X.Y.Z`), what a newer release is compared with.
    pub version: String,
    /// Tag of the newer wingetrs release the background check found (the status pill, Ctrl+U).
    pub self_update: Option<String>,
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
    /// Third-party code of the removals planned and not yet confirmed, by lowercase app.
    /// `(manifest hash, code)`.
    pub removal_code: HashMap<String, (String, Vec<ThirdParty>)>,
    /// Whether this process is elevated (Task 218): third-party code and user-scope winget
    /// removals are refused in the plan. `App::new`: never; `tui::run`: the token's (an error
    /// counts as elevated).
    pub elevated: fn() -> std::io::Result<bool>,
    /// PowerShell's parser for the scripts shown in a confirmation (Task 227): the one the plans
    /// were made with. `App::new`: none (every script refused); `tui::run`: the production one.
    pub ast: Arc<dyn AstGate>,
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
    /// Bucket names for the sources section of Settings (`b` manages them).
    pub buckets_summary: String,
    /// The values (behavior, glyphs, dark) of the one save in flight. Further changes only
    /// update the state; when the save ends, one more save carries the newest values.
    save_in_flight: Option<(Behavior, GlyphSet, bool)>,
    /// The `icons` value of the save in flight (goes with `save_in_flight`).
    icons_in_flight: Option<String>,
    list_icons_in_flight: Option<String>,
    /// Whether terminal mouse events are acted on (the capture itself is `run`'s).
    pub mouse_enabled: bool,
    /// The operation in progress; the screen stays usable meanwhile.
    pub running: Option<Running>,
    /// Confirmed actions waiting for `running`, in order.
    pub queue: VecDeque<Queued>,
    pub next_op_id: u64,
    /// Another process held the lock: the queue waits for `Msg::RetryQueue`.
    pub waiting_lock: bool,
    /// Quit was confirmed with an operation in progress: quit when it ends.
    pub quit_after_running: bool,
    /// The operation in progress launched `msiexec /a` (`Msg::InstallerRunning`).
    pub installer_running: bool,
    /// Dialogs that arrived while another was open; shown, in order, as it closes.
    pending_modals: VecDeque<Modal>,
    /// Rows marked with Space per tab, by identity (see `RowKey`), not by position.
    pub marks: HashMap<Tab, HashSet<RowKey>>,
    /// What the batch plan in flight was made of.
    batch: Option<Batch>,
    /// Which bridge tools exist (`Msg::ToolsDetected`).
    pub tools: Tools,
    /// Scoop root and PowerShell program of the bridge command lines (shown in the confirmation).
    pub scoop_root: PathBuf,
    pub pwsh: String,
    /// When the bridge upgrade in progress started or last printed a line.
    pub bridge_last_line: Option<Instant>,
    /// Native upgrade plans by `app_key`, with the catalog version each is for.
    pub native_upgrades: HashMap<String, (String, WingetPlan)>,
    /// Where `.reg` backups go (`install::layout::Layout::backups`), shown in confirmations.
    pub backups_dir: PathBuf,
}

/// Identity of a marked row: `(origin, name)` in Installed and Updates,
/// `(source_name, id)` in Search.
pub type RowKey = (String, String);

/// A batch plan in flight: the app behind each item, the tab it came from and what was
/// left out when it was built (reported when the plan arrives).
struct Batch {
    tab: Tab,
    labels: Vec<String>,
    skipped: Vec<String>,
    hidden: usize,
    /// Upgrades of the marked rows that need no plan (bridge or native), queued after the
    /// planned actions.
    bridges: Vec<Action>,
    /// Marked rows the action does not apply to.
    left_out: usize,
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
            show: ShowFilter::All,
            query: String::new(),
            selected: 0,
            scroll: 0,
            search: Arc::new(SearchIndex::new(entries)),
            hits: Vec::new(),
            search_generation: 0,
            search_in_flight: None,
            installed: Vec::new(),
            details: HashMap::new(),
            applicability: HashMap::new(),
            icon_bytes: HashMap::new(),
            icon_protocol: IconProtocol::Off,
            icons: IconCache::default(),
            icon_none: HashSet::new(),
            icon_gen: 0,
            icon_in_flight: HashSet::new(),
            icon_map: BTreeMap::new(),
            pins: std::collections::BTreeSet::new(),
            icon_tried: HashSet::new(),
            icon_absent: HashSet::new(),
            last_prefetch: HashSet::new(),
            list_fetch_gen: 0,
            list_icons_visible: true,
            details_pending: None,
            refreshing: false,
            offline: false,
            version: env!("CARGO_PKG_VERSION").to_string(),
            self_update: None,
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
            removal_code: HashMap::new(),
            elevated: crate::install::script_policy::not_elevated,
            ast: Arc::new(crate::install::script_ast::Unavailable),
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
            running: None,
            queue: VecDeque::new(),
            next_op_id: 0,
            waiting_lock: false,
            quit_after_running: false,
            installer_running: false,
            pending_modals: VecDeque::new(),
            marks: HashMap::new(),
            batch: None,
            tools: Tools::default(),
            scoop_root: PathBuf::new(),
            pwsh: "pwsh".to_string(),
            bridge_last_line: None,
            native_upgrades: HashMap::new(),
            backups_dir: PathBuf::new(),
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
            .filter(|r| match (self.tab, self.show) {
                (Tab::Installed, ShowFilter::Unmatched) => matches!(
                    r.correlation,
                    Correlation::Unmatched | Correlation::Ambiguous { .. }
                ),
                (Tab::Installed, ShowFilter::Duplicates) => !r.others.is_empty(),
                _ => true,
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

    /// Next show filter (`ctrl+e`, Installed only); the same item stays selected, or 0 if it vanished.
    fn cycle_show(&mut self) {
        let kept = self.selected_ident();
        self.show = match self.show {
            ShowFilter::All => ShowFilter::Unmatched,
            ShowFilter::Unmatched => ShowFilter::Duplicates,
            ShowFilter::Duplicates => ShowFilter::All,
        };
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

    /// Identity of every row of the current tab's view, in view order.
    fn view_keys(&self) -> Vec<RowKey> {
        match self.tab {
            Tab::Search => self
                .hits
                .iter()
                .filter_map(|h| self.search.entries().get(h.index))
                .map(|e| (e.source_name.clone(), e.id.clone()))
                .collect(),
            Tab::Installed | Tab::Updates => self
                .installed_view()
                .into_iter()
                .map(|r| (r.origin.clone(), r.name.clone()))
                .collect(),
            Tab::Settings => Vec::new(),
        }
    }

    /// Rows marked in the current tab (visible or not).
    pub fn marked_count(&self) -> usize {
        self.marks.get(&self.tab).map_or(0, HashSet::len)
    }

    /// A marked row is among the rows the tab shows now (marks hidden by the query or the
    /// filter do not count).
    pub fn has_visible_mark(&self) -> bool {
        self.marked_count() > 0 && self.view_keys().iter().any(|k| self.is_marked(k))
    }

    /// winget's verdict on upgrading `r` to its current catalog version, once it arrived.
    pub fn winget_fit(&self, r: &ListRow) -> Option<WingetFit> {
        let UpdateState::Available { latest } = &r.state else {
            return None;
        };
        self.applicability
            .get(&r.app_key)
            .filter(|(v, _)| v == latest)
            .map(|(_, f)| *f)
    }

    /// The upgrade of `r` is one winget would refuse (`bridge::Applicability`).
    pub fn upgrade_refused(&self, r: &ListRow) -> bool {
        bridge::bridge_target(r, &self.pins).is_ok_and(|j| j.tool == Tool::Winget)
            && self
                .winget_fit(r)
                .is_some_and(|f| bridge::applicability_note(f.applicability).is_some())
    }

    /// Whether the row `key` of the current tab is marked.
    pub fn is_marked(&self, key: &RowKey) -> bool {
        self.marks.get(&self.tab).is_some_and(|m| m.contains(key))
    }

    /// Space: marks or unmarks the selected row.
    fn toggle_mark(&mut self) {
        let Some(key) = self.view_keys().into_iter().nth(self.selected) else {
            return;
        };
        let marks = self.marks.entry(self.tab).or_default();
        if !marks.remove(&key) {
            marks.insert(key);
        }
    }

    /// Drops the marks of rows that no longer exist (after a new scan or catalog).
    fn prune_marks(&mut self) {
        let installed: HashSet<RowKey> = self
            .installed
            .iter()
            .map(|r| (r.origin.clone(), r.name.clone()))
            .collect();
        let upgradable: HashSet<RowKey> = self
            .installed
            .iter()
            .filter(|r| matches!(r.state, UpdateState::Available { .. }))
            .map(|r| (r.origin.clone(), r.name.clone()))
            .collect();
        let catalog: HashSet<RowKey> = self
            .search
            .entries()
            .iter()
            .map(|e| (e.source_name.clone(), e.id.clone()))
            .collect();
        for (tab, keys) in &mut self.marks {
            match tab {
                Tab::Search => keys.retain(|k| catalog.contains(k)),
                Tab::Installed => keys.retain(|k| installed.contains(k)),
                Tab::Updates => keys.retain(|k| upgradable.contains(k)),
                Tab::Settings => keys.clear(),
            }
        }
        self.marks.retain(|_, keys| !keys.is_empty());
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

    /// Requests encoding the app icon (once per protocol; `apply_icons` forgets it).
    /// Asked by the event loop, not `update`, after it points the picker at the live protocol.
    pub fn app_icon_cmd(&mut self) -> Vec<Cmd> {
        let size = APP_ICON_SIZE;
        if self.icon_protocol == IconProtocol::Off
            || self.icon_none.contains(APP_ICON_KEY)
            || self
                .icon_in_flight
                .contains(&(APP_ICON_KEY.to_string(), size))
            || self.icons.get(APP_ICON_KEY, size).is_some()
        {
            return Vec::new();
        }
        self.icon_in_flight.insert((APP_ICON_KEY.to_string(), size));
        vec![Cmd::EncodeIcon {
            generation: self.icon_gen,
            key: APP_ICON_KEY.to_string(),
            bytes: APP_ICON_PNG.to_vec(),
            size,
        }]
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

    /// Whether `key` is known to have no icon: none to get, or one that does not decode.
    pub fn icon_known_absent(&self, key: &str) -> bool {
        self.icon_absent.contains(key) || self.icon_none.contains(key)
    }

    /// Whether a row with this correlation shows the no-icon placeholder: unmatched rows
    /// have no package to take one from; ambiguous ones are not known yet.
    fn row_placeholder(&self, c: &Correlation) -> bool {
        match c {
            Correlation::Matched {
                source, package_id, ..
            } => self.icon_known_absent(&format!("{source}/{package_id}")),
            Correlation::Ambiguous { .. } => false,
            Correlation::Unmatched => true,
        }
    }

    /// For each row of `window` (aligned with `list_keys`), whether its icon slot shows the
    /// no-icon placeholder when it has no encoded icon.
    pub fn list_placeholders(&self, window: std::ops::Range<usize>) -> Vec<bool> {
        match self.tab {
            Tab::Search => self
                .hits
                .iter()
                .skip(window.start)
                .take(window.len())
                .map(|h| {
                    self.search.entries().get(h.index).is_some_and(|e| {
                        self.icon_known_absent(&format!("{}/{}", e.source_name, e.id))
                    })
                })
                .collect(),
            Tab::Installed | Tab::Updates => self
                .installed_view()
                .into_iter()
                .skip(window.start)
                .take(window.len())
                .map(|r| self.row_placeholder(&r.correlation))
                .collect(),
            Tab::Settings => Vec::new(),
        }
    }

    /// Whether the selected row's icon is the no-icon placeholder (same rule as the list).
    pub fn selected_placeholder(&self) -> bool {
        match self.tab {
            Tab::Search => self
                .selected_key()
                .is_some_and(|(s, id)| self.icon_known_absent(&format!("{s}/{id}"))),
            Tab::Installed | Tab::Updates => self
                .installed_view()
                .get(self.selected)
                .is_some_and(|r| self.row_placeholder(&r.correlation)),
            Tab::Settings => false,
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

    /// Requests fetching the icons of the visible rows that are neither cached nor
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
            if app
                .bridge_last_line
                .is_some_and(|t| app.now.saturating_duration_since(t) >= BRIDGE_QUIET)
            {
                set_bridge_line(app, BRIDGE_WAITING);
            }
            Vec::new()
        }
        Msg::CatalogLoaded(entries) => {
            app.search = Arc::new(SearchIndex::new(entries));
            app.prune_marks();
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
                    app.prune_marks();
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
            app.prune_marks();
            if let Some(i) = app.position_of(kept) {
                app.selected = i;
            }
            app.clamp();
            let mut cmds = app.details_cmd();
            cmds.extend(applicability_cmd(app));
            cmds
        }
        Msg::ApplicabilityLoaded {
            app_key,
            latest,
            result,
        } => {
            // A native plan, if any, follows this verdict.
            app.native_upgrades.remove(&app_key);
            match result {
                Ok(fit) => {
                    app.applicability.insert(app_key, (latest, fit));
                }
                Err(e) => {
                    tracing::debug!(%app_key, error = %e, "winget applicability not known");
                    app.applicability.remove(&app_key);
                }
            }
            Vec::new()
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
        Msg::OverrideSaved { choice, result } => match result {
            Ok(()) => {
                let text = match choice {
                    Some(Choice::Package { package_id, .. }) => {
                        format!("correspondência salva: {package_id}")
                    }
                    Some(Choice::NotInCatalog) => "marcado como fora do catálogo".to_string(),
                    None => "escolha de correspondência desfeita".to_string(),
                };
                app.push_toast(ToastKind::Success, text);
                vec![Cmd::ScanInstalled]
            }
            Err(e) => {
                app.push_toast(
                    ToastKind::Error,
                    format!("falha ao gravar a correspondência: {e}"),
                );
                Vec::new()
            }
        },
        Msg::PinSaved {
            key,
            pinned,
            result,
        } => {
            let name = key
                .split_once('/')
                .map_or(key.as_str(), |(_, n)| n)
                .to_owned();
            match result {
                Ok(()) => {
                    let text = if pinned {
                        format!("versão fixada: {name}")
                    } else {
                        format!("desafixado: {name}")
                    };
                    app.push_toast(ToastKind::Success, text);
                }
                Err(e) => {
                    if pinned {
                        app.pins.remove(&key);
                    } else {
                        app.pins.insert(key);
                    }
                    app.push_toast(
                        ToastKind::Error,
                        format!("falha ao gravar a fixação de {name}: {e}"),
                    );
                }
            }
            Vec::new()
        }
        Msg::IconMapped { key, sha } => {
            app.icon_map.insert(key, sha);
            Vec::new()
        }
        Msg::PrefetchDone { key, outcome } => {
            match outcome {
                IconOutcome::Found => {
                    app.icon_tried.insert(key);
                }
                IconOutcome::Absent => {
                    app.icon_tried.insert(key.clone());
                    app.icon_absent.insert(key);
                }
                // Not marked: the next request for the screen asks again.
                IconOutcome::Retry => {}
            }
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
        Msg::PortablePlanned { result, agreements } => on_portable_planned(app, result, agreements),
        Msg::WingetPlanned(result) => on_winget_planned(app, result),
        Msg::WingetInstallPlanned { bridge, native } => {
            on_winget_install_planned(app, bridge, native)
        }
        Msg::WingetUninstallPlanned(result) => on_winget_uninstall_planned(app, result),
        Msg::NativeUpgradeLoaded {
            app_key,
            latest,
            plan,
        } => {
            app.native_upgrades.insert(app_key, (latest, plan));
            refresh_open_actions(app)
        }
        Msg::BatchPlanned(results) => on_batch_planned(app, results),
        Msg::Panicked(payload) => std::panic::resume_unwind(payload),
        Msg::InstallProgress { label, done, total } => {
            if let Some(r) = &mut app.running {
                // Recorded into the running meter: the speed needs the earlier samples.
                let mut meter = r.progress.take().map(|p| p.meter).unwrap_or_default();
                meter.record_at(METER_KEY, done, total, app.now);
                r.progress = Some(RunProgress {
                    label,
                    phase: None,
                    meter,
                });
            }
            Vec::new()
        }
        Msg::InstallerRunning(log) => {
            if let Some(r) = &mut app.running {
                app.installer_running = true;
                // Each launch restarts the clock.
                let installer = crate::install::progress::Phase::Installer { log };
                r.progress = Some(phase_progress(installer, app.now));
            }
            Vec::new()
        }
        Msg::InstallPhase(phase) => {
            if let Some(r) = &mut app.running {
                // A native winget installer, like `msiexec /a`, cannot be stopped once running;
                // `WaitingElevation` comes after the launch gate (the elevated one may run).
                // A native removal announces `Removing` right before its uninstaller (or the
                // registry deletion) starts: from then on it is not stopped either.
                let native_removal = matches!(
                    r.action,
                    Some(
                        Action::WingetUninstall(_)
                            | Action::WingetRegistryOnly(_)
                            | Action::RemoveOrphan(_)
                    )
                );
                if matches!(
                    phase,
                    crate::install::progress::Phase::RunningInstaller { .. }
                        | crate::install::progress::Phase::HelperPreparing { .. }
                        | crate::install::progress::Phase::WaitingElevation
                        | crate::install::progress::Phase::Uninstalling { .. }
                        | crate::install::progress::Phase::CheckingRemoval { .. }
                        | crate::install::progress::Phase::SelfElevating { .. }
                        | crate::install::progress::Phase::RunningScript { .. }
                        | crate::install::progress::Phase::RunningPackageInstaller { .. }
                        | crate::install::progress::Phase::ReleaseInstaller { .. }
                ) || (native_removal
                    && matches!(phase, crate::install::progress::Phase::Removing { .. }))
                {
                    app.installer_running = true;
                }
                let same = r
                    .progress
                    .as_ref()
                    .and_then(|p| p.phase.as_ref())
                    .is_some_and(|(cur, _)| *cur == phase);
                if !same {
                    r.progress = Some(phase_progress(phase, app.now));
                }
            }
            Vec::new()
        }
        Msg::InstallFinished(result) => on_finished(app, result),
        Msg::SelfUpdateAvailable { tag } => {
            app.self_update = crate::selfupdate::newer(&app.version, &tag).map(|_| tag);
            Vec::new()
        }
        Msg::ToolsDetected { winget, scoop } => {
            app.tools = Tools {
                winget: Some(winget),
                scoop: Some(scoop),
            };
            refresh_open_actions(app)
        }
        Msg::BridgeLine(line) => {
            if app.bridge_last_line.is_some() {
                app.bridge_last_line = Some(app.now);
                on_bridge_line(app, &line);
            }
            Vec::new()
        }
        Msg::RetryQueue => {
            app.waiting_lock = false;
            pump(app)
        }
        Msg::BucketsLoaded(result) => on_buckets_loaded(app, result),
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

/// Items a wheel notch moves the selection.
const WHEEL_ITEMS: usize = 3;

/// Mouse: left click on a tab, list item (selects and toggles its mark) or setting row; right
/// click on an item opens its actions menu; the wheel moves the list. Of the dialogs only the
/// confirmation takes the mouse; ignored under the help, and when `mouse_enabled` is off.
fn on_mouse(app: &mut App, ev: MouseEvent) -> Vec<Cmd> {
    if !app.mouse_enabled {
        return Vec::new();
    }
    if matches!(app.modal, Some(Modal::Confirm { .. })) {
        let cmds = confirm_mouse(app, ev);
        if app.modal.is_none() {
            app.modal = app.pending_modals.pop_front();
        }
        return cmds;
    }
    if app.modal.is_some() || app.show_help {
        return Vec::new();
    }
    let area = ratatui::layout::Rect::new(0, 0, app.size.0, app.size.1);
    let r = crate::tui::regions::regions(app, area);
    let inside = |rect: ratatui::layout::Rect| rect.contains((ev.column, ev.row).into());
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some((_, tab)) = r.tabs.iter().find(|(rect, _)| inside(*rect)) {
                return app.set_tab(*tab);
            }
            if app.tab == Tab::Settings {
                if let Some(index) = r.setting_rows.iter().position(|rect| inside(*rect)) {
                    app.settings.selected = index;
                }
                return Vec::new();
            }
            if let Some((_, id)) = r.buttons.iter().find(|(rect, _)| inside(*rect)) {
                return run_button(app, *id);
            }
            let Some(index) = crate::tui::regions::item_at(app, r.list, ev.column, ev.row) else {
                return Vec::new();
            };
            let cmds = app.move_to(index);
            app.toggle_mark();
            cmds
        }
        MouseEventKind::Down(MouseButton::Right) if app.tab != Tab::Settings => {
            let Some(index) = crate::tui::regions::item_at(app, r.list, ev.column, ev.row) else {
                return Vec::new();
            };
            let mut cmds = app.move_to(index);
            cmds.extend(open_item_menu(app));
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
    let quit_asked = matches!(app.modal, Some(Modal::ConfirmQuit { .. }));
    if ctrl_c && (quit_asked || app.quit_after_running) && !job_uncancellable(app) {
        let mut cmds = cancel_native(app);
        cmds.push(Cmd::Quit);
        return cmds;
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
            KeyCode::Char(' ') if app.tab != Tab::Settings => {
                app.toggle_mark();
                Vec::new()
            }
            KeyCode::Char('s') => {
                app.filter = match app.filter {
                    SourceFilter::All => SourceFilter::Winget,
                    SourceFilter::Winget => SourceFilter::Scoop,
                    SourceFilter::Scoop => SourceFilter::All,
                };
                app.view_changed()
            }
            KeyCode::Char('e') if app.tab == Tab::Installed => {
                app.cycle_show();
                Vec::new()
            }
            KeyCode::Char('u') => ask_self_update(app),
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
            } else if app.marked_count() > 0 {
                app.marks.remove(&app.tab);
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
        KeyCode::Insert if app.tab != Tab::Settings => {
            app.toggle_mark();
            Vec::new()
        }
        KeyCode::Char(' ') if empty && app.tab != Tab::Settings => {
            app.toggle_mark();
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
        KeyCode::Char('b') => open_buckets(app),
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

/// `i`: plan installing the selected package (scoop by the own installer, winget through the
/// bridge).
fn install_selected(app: &mut App) -> Vec<Cmd> {
    if marked_rows(app).is_some() {
        return run_button(app, ActionId::Install);
    }
    install_one(app)
}

/// Plans installing the selected package alone, whatever the marks.
fn install_one(app: &mut App) -> Vec<Cmd> {
    let Some(hit) = app.hits.get(app.selected) else {
        return Vec::new();
    };
    let Some(e) = app.search.entries().get(hit.index) else {
        return Vec::new();
    };
    if e.source != SourceKind::Scoop {
        let cmd = Cmd::PlanWingetInstall { id: e.id.clone() };
        return start_planning(app, cmd);
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
    if marked_rows(app).is_some() {
        return run_button(app, ActionId::Remove);
    }
    remove_one(app)
}

/// Plans removing the selected app alone, whatever the marks.
fn remove_one(app: &mut App) -> Vec<Cmd> {
    let Some(r) = app.installed_view().get(app.selected).map(|r| (*r).clone()) else {
        return Vec::new();
    };
    match r.origin.as_str() {
        "wingetrs" => plan_remove(app, r.name),
        "arp" | "msix" => plan_winget_remove(app, &r, 0),
        _ => {
            app.push_toast(ToastKind::Warning, NOT_OURS);
            Vec::new()
        }
    }
}

/// Whether wingetrs removes a record of this origin natively (ARP and MSIX).
fn winget_removable(origin: &str) -> bool {
    matches!(origin, "arp" | "msix")
}

/// Plans the native removal of record `n` of `r` (0: its own, `n`: `others[n - 1]`); the
/// other ARP records of the app go along for the shared-data check.
fn plan_winget_remove(app: &mut App, r: &ListRow, n: usize) -> Vec<Cmd> {
    type Record<'a> = (&'a str, &'a str, Option<&'a String>);
    let records: Vec<Record<'_>> = std::iter::once((
        r.app_key.as_str(),
        r.origin.as_str(),
        r.installed_version.as_ref(),
    ))
    .chain(r.others.iter().map(|o| {
        (
            o.app_key.as_str(),
            o.origin.as_str(),
            o.installed_version.as_ref(),
        )
    }))
    .collect();
    let Some(&(app_key, _, version)) = records.get(n) else {
        return Vec::new();
    };
    let others = records
        .iter()
        .enumerate()
        .filter(|(i, (_, origin, _))| *i != n && *origin == "arp")
        .map(|(_, (k, _, _))| k.to_string())
        .collect();
    let cmd = Cmd::PlanWingetUninstall(crate::winget_install::previous::RemovalRequest {
        app_key: app_key.to_string(),
        name: r.name.clone(),
        version: version.cloned(),
        package: crate::winget_install::previous::winget_package(r).map(str::to_string),
        others,
        is_main: n == 0,
    });
    start_planning(app, cmd)
}

/// The orphan records of `r` (numbered as `ActionId::RemoveOrphan`): its own and the others'
/// ARP records whose uninstaller is gone.
fn row_orphans(r: &ListRow) -> Vec<(usize, Orphan)> {
    let own = std::iter::once((r.app_key.as_str(), &r.stale));
    let others = r.others.iter().map(|o| (o.app_key.as_str(), &o.stale));
    own.chain(others)
        .enumerate()
        .filter_map(|(i, (app_key, stale))| {
            let missing = stale.clone()?;
            match target_of(app_key)? {
                UninstallTarget::Arp { hive, key } => Some((
                    i,
                    Orphan {
                        hive,
                        key,
                        name: r.name.clone(),
                        missing,
                    },
                )),
                UninstallTarget::Msix { .. } => None,
            }
        })
        .collect()
}

/// Asks before removing an orphan record, whatever the `confirm` setting: its one item starts
/// unchecked.
fn ask_orphan(app: &mut App, o: Orphan) -> Vec<Cmd> {
    let modal = confirm_modal(app, vec![Action::RemoveOrphan(o)], Vec::new(), None);
    open_modal(app, modal);
    Vec::new()
}

/// The one rule for who can be upgraded (actions menu, `U`, `Enter` in Updates, marked
/// batch): ours, matched to a catalog package, with a newer version; else, an app winget or
/// scoop installed, through that tool (`bridge::bridge_target`) when the tool is there.
/// `Err` carries the reason, as the note of a disabled menu item.
fn upgrade_target(app: &App, r: &ListRow) -> Result<Target, Cow<'static, str>> {
    if r.origin == "wingetrs" {
        if crate::pins::is_pinned(&app.pins, &r.origin, &r.name) {
            return Err(PINNED_NOTE.into());
        }
        return match (&r.correlation, &r.state) {
            (
                Correlation::Matched {
                    source, package_id, ..
                },
                UpdateState::Available { .. },
            ) => Ok(Target::Ours(source.clone(), package_id.clone())),
            _ => Err("sem atualização".into()),
        };
    }
    let job = bridge::bridge_target(r, &app.pins)?;
    if job.tool == Tool::Winget
        && app.settings.behavior.winget_engine == WingetEngine::Wingetrs
        && let UpdateState::Available { latest } = &r.state
        && let Some((version, plan)) = app.native_upgrades.get(&r.app_key)
        && version == latest
    {
        return Ok(Target::Native {
            plan: Box::new(plan.clone()),
            row_key: r.app_key.clone(),
        });
    }
    if job.tool == Tool::Winget
        && let Some(note) = app
            .winget_fit(r)
            .and_then(|f| bridge::applicability_note(f.applicability))
    {
        return Err(note.into());
    }
    let present = match job.tool {
        Tool::Winget => app.tools.winget,
        Tool::Scoop => app.tools.scoop,
    };
    match (present, job.tool) {
        (Some(false), Tool::Winget) => Err("winget não está instalado".into()),
        (Some(false), Tool::Scoop) => Err("scoop não está instalado".into()),
        _ => Ok(Target::Bridge(Box::new(job))),
    }
}

/// The rows whose winget upgrade is worth checking: bridged through winget, with a newer
/// catalog version.
fn applicability_cmd(app: &App) -> Option<Cmd> {
    let rows: Vec<ApplicabilityQuery> = app
        .installed
        .iter()
        .filter_map(|r| {
            let job = bridge::bridge_target(r, &app.pins).ok()?;
            (job.tool == Tool::Winget).then_some(())?;
            Some(ApplicabilityQuery {
                app_key: r.app_key.clone(),
                package_id: job.target,
                latest: job.available?,
                scope: r.scope,
                installed: r.installed_version.clone(),
            })
        })
        .collect();
    (!rows.is_empty()).then_some(Cmd::FetchApplicability { rows })
}

/// What may happen meanwhile when `tool` runs an upgrade.
fn bridge_note(tool: Tool) -> &'static str {
    match tool {
        Tool::Winget => "Pode aparecer um pedido do UAC, às vezes só piscando na barra de tarefas.",
        Tool::Scoop => "O scoop pode atualizar os buckets (git pull) antes.",
    }
}

/// The note of a batch whose action does not apply to every marked row.
fn left_out_note(left_out: usize) -> String {
    // One message for the buttons and the keys; the count is not inflected on purpose.
    format!("{left_out} itens ficaram de fora (ação não se aplica)")
}

/// Toasts what `left_out_note` says when no confirmation shows it.
fn toast_left_out(app: &mut App, left_out: usize) {
    if left_out > 0 {
        app.push_toast(ToastKind::Info, left_out_note(left_out));
    }
}

/// `(source, id)` of the catalog package of a row.
fn row_key(r: &ListRow) -> Option<(String, String)> {
    match &r.correlation {
        Correlation::Matched {
            source, package_id, ..
        } => Some((source.clone(), package_id.clone())),
        _ => None,
    }
}

/// One item per package of `actions` (built once, when the dialog opens).
fn confirm_items(app: &App, actions: &[Action]) -> Vec<ConfirmItem> {
    let ours = |name: &str| {
        app.installed
            .iter()
            .find(|r| r.name == name && r.origin == "wingetrs")
    };
    // The row `bridge_target` built the job from: same name, origin and target.
    let bridged = |job: &BridgeJob| {
        app.installed.iter().find(|r| {
            r.name == job.name
                && match job.tool {
                    Tool::Scoop => r.origin == "scoop",
                    Tool::Winget => {
                        matches!(r.origin.as_str(), "arp" | "msix")
                            && row_key(r).is_some_and(|(_, id)| id == job.target)
                    }
                }
        })
    };
    let mut items = Vec::new();
    for action in actions {
        match action {
            Action::Apply(groups) | Action::ApplyScripts { groups } => {
                // The target of a group is its last plan; the others are its dependencies.
                items.extend(groups.iter().filter_map(|g| g.last()).map(|p| ConfirmItem {
                    name: p.app.clone(),
                    key: Some((format!("scoop:{}", p.bucket), p.app.clone())),
                    from: p.upgrade_from.clone(),
                    to: Some(p.version.clone()),
                    manager: "wingetrs",
                    checked: true,
                }));
            }
            Action::ApplyMsiexec { groups, app, .. } => {
                let plan = groups
                    .iter()
                    .flatten()
                    .find(|p| p.app.eq_ignore_ascii_case(app));
                items.push(ConfirmItem {
                    name: app.clone(),
                    key: plan.map(|p| (format!("scoop:{}", p.bucket), p.app.clone())),
                    from: plan.and_then(|p| p.upgrade_from.clone()),
                    to: plan.map(|p| p.version.clone()),
                    manager: "wingetrs",
                    checked: true,
                });
            }
            Action::Remove(p) | Action::RemoveScripts { plan: p, .. } => items.push(ConfirmItem {
                name: p.app.clone(),
                key: ours(&p.app).and_then(row_key),
                from: Some(p.version.clone()),
                to: None,
                manager: "wingetrs",
                checked: true,
            }),
            Action::Bridge(job) if matches!(job.op, BridgeOp::Install { .. }) => {
                items.push(ConfirmItem {
                    name: job.name.clone(),
                    key: Some((job.tool.name().to_string(), job.target.clone())),
                    from: None,
                    to: job.available.clone(),
                    manager: job.tool.name(),
                    checked: true,
                });
            }
            Action::Bridge(job) => {
                let r = bridged(job);
                items.push(ConfirmItem {
                    name: job.name.clone(),
                    key: r.and_then(row_key),
                    from: r.and_then(|r| r.installed_version.clone()),
                    to: r.and_then(|r| r.latest.clone()),
                    manager: job.tool.name(),
                    checked: true,
                });
            }
            Action::WingetInstall(p) | Action::WingetUpgrade { plan: p, .. } => {
                items.push(ConfirmItem {
                    name: p.name.clone(),
                    key: Some(("winget".to_string(), p.id.clone())),
                    from: p.upgrade_from.clone(),
                    to: Some(p.version.clone()),
                    manager: "wingetrs",
                    checked: true,
                });
            }
            Action::WingetUninstall(p) | Action::WingetRegistryOnly(p) => {
                items.push(ConfirmItem {
                    name: p.name.clone(),
                    key: None,
                    from: p.version.clone(),
                    to: None,
                    manager: "wingetrs",
                    checked: true,
                });
                let (backup, registry_only) = match action {
                    Action::WingetUninstall(p) => uninstall_options(p),
                    _ => (false, false),
                };
                if backup {
                    items.push(option_item(BACKUP_ITEM, true));
                }
                if registry_only {
                    items.push(option_item(REGISTRY_ONLY_ITEM, false));
                }
            }
            // Confirmed in the buckets dialog, never through this one.
            Action::BucketAdd { name, .. } | Action::BucketRemove { name, .. } => {
                items.push(ConfirmItem {
                    name: format!("bucket {name}"),
                    key: None,
                    from: None,
                    to: None,
                    manager: "wingetrs",
                    checked: true,
                });
            }
            Action::SelfUpdate { tag } => items.push(ConfirmItem {
                name: "wingetrs".to_string(),
                key: None,
                from: Some(app.version.clone()),
                to: crate::selfupdate::parse_tag(tag).map(|v| v.to_string()),
                manager: "wingetrs",
                checked: true,
            }),
            Action::RemoveOrphan(o) => items.push(ConfirmItem {
                name: format!("Remover registro órfão de {}", o.name),
                key: None,
                from: None,
                to: None,
                manager: "",
                // Never pre-selected: the user marks it.
                checked: false,
            }),
        }
    }
    items
}

/// Item of a removal that copies the app's data folders first.
pub const BACKUP_ITEM: &str = crate::winget_install::removal::BACKUP_THEN_REMOVE_LABEL;
/// Item of a removal that deletes only the registry record (option C).
pub const REGISTRY_ONLY_ITEM: &str = crate::winget_install::removal::REGISTRY_ONLY_LABEL;

/// An option of the item above it (no icon, versions or manager).
fn option_item(name: &str, checked: bool) -> ConfirmItem {
    ConfirmItem {
        name: name.to_string(),
        key: None,
        from: None,
        to: None,
        manager: "",
        checked,
    }
}

/// The options a native removal offers: copy the data folders first (when another
/// installation shares them), and remove only the registry record (when allowed).
fn uninstall_options(p: &WingetUninstallPlan) -> (bool, bool) {
    match &p.shared {
        Some(s) => (!s.folders.is_empty(), s.registry_only_allowed),
        None => (false, false),
    }
}

/// How many confirmation items `action` has (an `Apply` has one per non-empty group).
fn item_count(action: &Action) -> usize {
    match action {
        Action::Apply(groups) | Action::ApplyScripts { groups } => {
            groups.iter().filter(|g| !g.is_empty()).count()
        }
        Action::WingetUninstall(p) => {
            let (backup, registry_only) = uninstall_options(p);
            1 + usize::from(backup) + usize::from(registry_only)
        }
        _ => 1,
    }
}

/// The notes of `action` in its confirmation (several lines for a native plan).
fn action_notes(app: &App, action: &Action) -> Vec<String> {
    let lines = |text: String| -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    };
    match action {
        Action::Bridge(job) => vec![bridge_note(job.tool).to_string()],
        Action::ApplyMsiexec { .. } => vec![MSIEXEC_NOTE.to_string()],
        Action::Remove(p) | Action::RemoveScripts { plan: p, .. } if p.purge => vec![format!(
            "Os dados persistentes de {} serão APAGADOS (purge).",
            p.app
        )],
        Action::WingetInstall(p) | Action::WingetUpgrade { plan: p, .. } => lines(p.to_string()),
        Action::WingetUninstall(p) => lines(p.to_string()),
        Action::RemoveOrphan(o) => vec![
            format!("O desinstalador de {} não existe: {}", o.name, o.missing),
            "O app pode ter mudado de lugar (ex.: Steam em outra pasta).".to_string(),
            format!(
                "O registro é exportado antes para um .reg em {}",
                app.backups_dir.display()
            ),
        ],
        _ => Vec::new(),
    }
}

/// The confirmation of `actions`: one item per package, the bridge and purge warnings, then
/// `notes`.
fn confirm_modal(
    app: &App,
    actions: Vec<Action>,
    notes: Vec<String>,
    clear_marks: Option<Tab>,
) -> Modal {
    let mut all: Vec<String> = Vec::new();
    for note in actions.iter().flat_map(|a| action_notes(app, a)) {
        if !all.contains(&note) {
            all.push(note);
        }
    }
    all.extend(notes);
    let mut scripts = script_section(&actions_code(app, &actions));
    for a in &actions {
        if let Action::Remove(p) = a
            && let Some((sha, _)) = app.removal_code.get(&p.app.to_lowercase())
        {
            scripts.shas.push((p.app.to_lowercase(), sha.clone()));
        }
    }
    Modal::Confirm {
        items: confirm_items(app, &actions),
        notes: all,
        scroll: 0,
        cursor: 0,
        actions,
        clear_marks,
        scripts,
    }
}

/// The third-party code each `Apply`/`Remove` of `actions` runs, by app (one entry per app).
fn actions_code(app: &App, actions: &[Action]) -> Vec<(String, Vec<ThirdParty>)> {
    let mut out: Vec<(String, Vec<ThirdParty>)> = Vec::new();
    let mut add = |name: &str, code: Vec<ThirdParty>| {
        if !code.is_empty() && !out.iter().any(|(a, _)| a.eq_ignore_ascii_case(name)) {
            out.push((name.to_string(), code));
        }
    };
    for action in actions {
        match action {
            Action::Apply(groups) => {
                for p in groups.iter().flatten() {
                    add(&p.app, plan_third_party(p, &*app.ast));
                }
            }
            Action::Remove(p) => {
                let code = app.removal_code.get(&p.app.to_lowercase());
                add(&p.app, code.map(|(_, c)| c.clone()).unwrap_or_default());
            }
            _ => {}
        }
    }
    out
}

/// Whether confirming `actions` would let third-party code run: the dialog then opens whatever
/// the `confirm` setting says.
fn has_third_party_code(app: &App, actions: &[Action]) -> bool {
    !actions_code(app, actions).is_empty()
}

const PRIVILEGES: &str = "roda com os seus privilégios e pode alterar qualquer arquivo seu";

fn verdict_text(v: Verdict) -> &'static str {
    match v {
        Verdict::Confirm => "pede confirmação",
        Verdict::ConfirmStrong => "pede atenção redobrada; revise antes de permitir",
        Verdict::Refuse => "recusado: não roda",
    }
}

fn push_report(lines: &mut Vec<ScriptLine>, r: &ScriptReport) {
    let line = |kind, text: String| ScriptLine { kind, text };
    lines.push(line(
        ScriptLineKind::Info,
        format!("  Análise: {}", verdict_text(r.verdict)),
    ));
    for f in &r.flags {
        lines.push(line(
            ScriptLineKind::Flag,
            format!("  ! {} (linha {})", printable(&f.detail), f.line),
        ));
    }
}

fn command_line(file: &str, args: &[String]) -> String {
    std::iter::once(file)
        .chain(args.iter().map(String::as_str))
        .map(printable)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The scripts section of a confirmation: per piece of code its header, the gate's verdict and
/// flags, and the script's lines numbered. Built once, when the dialog opens.
fn script_section(code: &[(String, Vec<ThirdParty>)]) -> ScriptSection {
    let mut sec = ScriptSection::default();
    let line = |kind, text: String| ScriptLine { kind, text };
    for (name, items) in code {
        sec.apps.push(name.to_lowercase());
        if code.len() > 1 {
            let text = format!("{}:", printable(name));
            sec.lines.push(line(ScriptLineKind::App, text));
        }
        for item in items {
            let report = match item {
                ThirdParty::Script { report, .. } | ThirdParty::Ps1 { report, .. } => Some(report),
                _ => None,
            };
            if report.is_some_and(|r| r.verdict == Verdict::Refuse) {
                sec.refused.push(name.to_lowercase());
            }
            let program = |uninstaller: &bool, file: &str, args: &[String]| {
                let what = if *uninstaller {
                    "Desinstalador"
                } else {
                    "Instalador"
                };
                format!("{what} de terceiro: {}", command_line(file, args))
            };
            match item {
                ThirdParty::Script {
                    hook,
                    script,
                    report,
                } => {
                    let title = format!("Script de terceiro ({}), {PRIVILEGES}", printable(hook));
                    sec.lines.push(line(ScriptLineKind::Header, title));
                    push_report(&mut sec.lines, report);
                    let width = script.lines().count().max(1).to_string().len();
                    for (i, l) in script.lines().enumerate() {
                        let text = format!("  {:>width$} | {}", i + 1, printable(l));
                        sec.lines.push(line(ScriptLineKind::Code, text));
                    }
                }
                ThirdParty::Program {
                    uninstaller,
                    file,
                    args,
                } => {
                    let title = program(uninstaller, file, args);
                    sec.lines.push(line(ScriptLineKind::Header, title));
                    sec.lines
                        .push(line(ScriptLineKind::Info, format!("  {PRIVILEGES}")));
                }
                ThirdParty::Ps1 {
                    uninstaller,
                    file,
                    args,
                    report,
                } => {
                    let title = program(uninstaller, file, args);
                    sec.lines.push(line(ScriptLineKind::Header, title));
                    sec.lines
                        .push(line(ScriptLineKind::Info, format!("  {PRIVILEGES}")));
                    push_report(&mut sec.lines, report);
                }
                ThirdParty::Unknown { what } => {
                    let title = format!("Código de terceiro: {}", printable(what));
                    sec.lines.push(line(ScriptLineKind::Header, title));
                    sec.lines
                        .push(line(ScriptLineKind::Info, format!("  {PRIVILEGES}")));
                }
            }
        }
    }
    sec
}

/// `actions` as the user allowed them: an `Apply`/`Remove` whose app runs third-party code
/// (shown in the dialog) becomes `ApplyScripts`/`RemoveScripts`. `Err(app)` when one of them
/// has a script the gate refuses: that never runs.
fn authorize_scripts(sec: &ScriptSection, actions: Vec<Action>) -> Result<Vec<Action>, String> {
    let code = |name: &str| sec.apps.iter().any(|a| a.eq_ignore_ascii_case(name));
    let mut out = Vec::new();
    for action in actions {
        let (names, converted): (Vec<String>, Action) = match action {
            Action::Apply(groups) => {
                let names: Vec<String> = groups.iter().flatten().map(|p| p.app.clone()).collect();
                if names.iter().any(|n| code(n)) {
                    (names, Action::ApplyScripts { groups })
                } else {
                    out.push(Action::Apply(groups));
                    continue;
                }
            }
            Action::Remove(p) if code(&p.app) => {
                let sha = sec
                    .shas
                    .iter()
                    .find(|(a, _)| a.eq_ignore_ascii_case(&p.app))
                    .map(|(_, s)| s.clone())
                    .unwrap_or_default();
                let names = vec![p.app.clone()];
                (
                    names,
                    Action::RemoveScripts {
                        plan: p,
                        manifest_sha256: sha,
                    },
                )
            }
            other => {
                out.push(other);
                continue;
            }
        };
        if let Some(n) = names
            .iter()
            .find(|n| sec.refused.iter().any(|r| r.eq_ignore_ascii_case(n)))
        {
            return Err(n.clone());
        }
        out.push(converted);
    }
    Ok(out)
}

/// Middle of the note of an unchecked package that a checked one still installs.
const KEPT_DEP_NOTE: &str = " será instalado mesmo assim: é dependência de ";

/// `(checked, group)` of each `Apply` group, aligned with `items` as `confirm_items` built it.
fn apply_groups<'a>(
    items: &[ConfirmItem],
    actions: &'a [Action],
) -> Vec<(bool, &'a [InstallPlan])> {
    let mut checked = items.iter().map(|i| i.checked);
    let mut out = Vec::new();
    for action in actions {
        match action {
            Action::Apply(groups) => {
                for g in groups.iter().filter(|g| !g.is_empty()) {
                    out.push((checked.next().unwrap_or(false), g.as_slice()));
                }
            }
            other => {
                for _ in 0..item_count(other) {
                    checked.next();
                }
            }
        }
    }
    out
}

/// Replaces the notes of unchecked packages that a checked group installs anyway (its
/// dependency): called whenever an item is toggled.
fn refresh_kept_deps(items: &[ConfirmItem], actions: &[Action], notes: &mut Vec<String>) {
    notes.retain(|n| !n.contains(KEPT_DEP_NOTE));
    let groups = apply_groups(items, actions);
    for (_, g) in groups.iter().filter(|(checked, _)| !checked) {
        let Some(target) = g.last() else { continue };
        let owner = groups
            .iter()
            .filter(|(checked, _)| *checked)
            .find_map(|(_, other)| {
                let last = other.last()?;
                other[..other.len() - 1]
                    .iter()
                    .any(|p| p.app.eq_ignore_ascii_case(&target.app))
                    .then_some(last)
            });
        if let Some(owner) = owner {
            notes.push(format!("{}{KEPT_DEP_NOTE}{}", target.app, owner.app));
        }
    }
}

/// Toggles item `index` of a confirmation and refreshes the dependency notes.
fn toggle_confirm_item(
    items: &mut [ConfirmItem],
    actions: &[Action],
    notes: &mut Vec<String>,
    index: usize,
) {
    if let Some(item) = items.get_mut(index) {
        item.checked = !item.checked;
        refresh_kept_deps(items, actions, notes);
    }
}

/// `actions` without the unchecked items; `items` is aligned as `confirm_items` built it.
fn checked_actions(items: &[ConfirmItem], actions: Vec<Action>) -> Vec<Action> {
    let mut checked = items.iter().map(|i| i.checked);
    let mut out = Vec::new();
    for action in actions {
        match action {
            Action::Apply(groups) => {
                // A kept group keeps its own dependencies (merged at execution).
                let kept: Vec<Vec<InstallPlan>> = groups
                    .into_iter()
                    .filter(|g| !g.is_empty() && checked.next().unwrap_or(false))
                    .collect();
                if !kept.is_empty() {
                    out.push(Action::Apply(kept));
                }
            }
            Action::WingetUninstall(mut p) => {
                let (backup, registry_only) = uninstall_options(&p);
                let main = checked.next().unwrap_or(false);
                let keep_backup = backup && checked.next().unwrap_or(false);
                let only_record = registry_only && checked.next().unwrap_or(false);
                if !main {
                    continue;
                }
                if only_record {
                    out.push(Action::WingetRegistryOnly(p));
                    continue;
                }
                if !keep_backup && let Some(s) = &mut p.shared {
                    // Unchecked copy: nothing is copied before the uninstaller runs.
                    s.folders.clear();
                }
                out.push(Action::WingetUninstall(p));
            }
            other => {
                if checked.next().unwrap_or(false) {
                    out.push(other);
                }
            }
        }
    }
    out
}

/// Upgrades one app through its own tool: confirmed per the `confirm` setting, then queued.
fn start_bridge(app: &mut App, job: BridgeJob) -> Vec<Cmd> {
    let action = Action::Bridge(job);
    if !needs_confirm(app.settings.behavior.confirm, &action) {
        return enqueue(app, action);
    }
    let modal = confirm_modal(app, vec![action], Vec::new(), None);
    open_modal(app, modal);
    Vec::new()
}

/// A native winget action: confirmed per `confirm`, and always when the user has something
/// to read or choose (license agreements, `notes`, the shared-data options); else queued.
fn start_ready(
    app: &mut App,
    action: Action,
    confirm: ConfirmMode,
    notes: Vec<String>,
) -> Vec<Cmd> {
    let must_show = match &action {
        Action::WingetInstall(p) | Action::WingetUpgrade { plan: p, .. } => {
            !p.agreements.is_empty()
        }
        Action::WingetUninstall(p) => p.shared.is_some(),
        _ => false,
    };
    if notes.is_empty() && !must_show && !needs_confirm(confirm, &action) {
        return enqueue(app, action);
    }
    let modal = confirm_modal(app, vec![action], notes, None);
    open_modal(app, modal);
    Vec::new()
}

/// Shows `line` as the progress of the bridge upgrade in progress.
/// A bridge output line, as the CLI reads it (`progress::bridge_line`): the UAC notice
/// becomes the phase with its clock; other lines go to the label.
fn on_bridge_line(app: &mut App, line: &str) {
    let Some(r) = &mut app.running else {
        return;
    };
    let Some(Action::Bridge(job)) = &r.action else {
        return;
    };
    let current = r
        .progress
        .as_ref()
        .and_then(|p| p.phase.as_ref())
        .map(|(p, _)| p);
    let (next, show) = crate::install::progress::bridge_line(line, current, job.tool.name());
    if let Some(p) = next {
        r.progress = Some(phase_progress(p, app.now));
    }
    if show {
        set_bridge_line(app, line);
    }
}

fn set_bridge_line(app: &mut App, line: &str) {
    if let Some(r) = &mut app.running
        && let Some(action @ Action::Bridge(_)) = &r.action
    {
        let shown =
            crate::bridge::winget_out::tui_label(&crate::bridge::winget_out::classify(line))
                .unwrap_or_else(|| line.to_string());
        r.label = format!("{} · {shown}", action.label());
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
    if marked_rows(app).is_some() {
        return run_button(app, ActionId::Upgrade);
    }
    let keep = app.settings.behavior.keep_old;
    if app.installed_view().get(app.selected).is_none() {
        return start_planning(app, Cmd::PlanUpgradeAll { keep });
    }
    upgrade_one(app)
}

/// Plans upgrading the selected app alone, whatever the marks.
fn upgrade_one(app: &mut App) -> Vec<Cmd> {
    let Some(target) = app
        .installed_view()
        .get(app.selected)
        .map(|r| upgrade_target(app, r))
    else {
        return Vec::new();
    };
    match target {
        Ok(Target::Ours(source_name, id)) => plan_upgrade(app, source_name, id),
        Ok(Target::Bridge(job)) => start_bridge(app, *job),
        Ok(Target::Native { plan, row_key }) => {
            let confirm = app.settings.behavior.confirm;
            start_ready(
                app,
                Action::WingetUpgrade {
                    plan: *plan,
                    row_key,
                },
                confirm,
                Vec::new(),
            )
        }
        Err(note) if note == PINNED_NOTE => {
            app.push_toast(ToastKind::Warning, "versão fixada (desafixe no menu)");
            Vec::new()
        }
        Err(note) if note == NOT_OURS_NOTE => {
            app.push_toast(ToastKind::Warning, NOT_OURS);
            Vec::new()
        }
        Err(note) => {
            app.push_toast(ToastKind::Warning, note);
            Vec::new()
        }
    }
}

/// View positions of the marked rows that are visible, and how many marked rows are out of
/// view (hidden by the query or the source filter); `None` when the tab has no marks.
fn marked_rows(app: &App) -> Option<(Vec<usize>, usize)> {
    let marks = app.marks.get(&app.tab).filter(|m| !m.is_empty())?;
    let rows: Vec<usize> = app
        .view_keys()
        .iter()
        .enumerate()
        .filter(|(_, k)| marks.contains(*k))
        .map(|(i, _)| i)
        .collect();
    let hidden = marks.len() - rows.len();
    Some((rows, hidden))
}

/// Toasts what a batch leaves out: rows that cannot take the action and marked rows out of view.
fn report_left_out(app: &mut App, skipped: &[String], hidden: usize) {
    if !skipped.is_empty() {
        app.push_toast(
            ToastKind::Warning,
            format!("pulados: {}", skipped.join(", ")),
        );
    }
    if hidden > 0 {
        app.push_toast(
            ToastKind::Warning,
            format!("{hidden} marcados fora da vista não incluídos"),
        );
    }
}

/// Plans `items` (label, command) as one batch, or only reports what was left out when there
/// is nothing to plan. `left_out` marked rows do not take the action.
fn start_batch(
    app: &mut App,
    items: Vec<(String, Cmd)>,
    bridges: Vec<Action>,
    skipped: Vec<String>,
    hidden: usize,
    left_out: usize,
) -> Vec<Cmd> {
    if items.is_empty() {
        report_left_out(app, &skipped, hidden);
        // Bridge upgrades need no plan: straight to the one confirmation.
        let tab = app.tab;
        let confirm = app.settings.behavior.confirm;
        return confirm_batch(app, Vec::new(), bridges, confirm, tab, left_out);
    }
    if app.planning {
        if !bridges.is_empty() {
            app.push_toast(
                ToastKind::Warning,
                "outro plano em andamento: lote não iniciado, tente de novo",
            );
        }
        toast_left_out(app, left_out);
        return Vec::new();
    }
    let (labels, items): (Vec<String>, Vec<Cmd>) = items.into_iter().unzip();
    app.batch = Some(Batch {
        tab: app.tab,
        labels,
        skipped,
        hidden,
        bridges,
        left_out,
    });
    start_planning(app, Cmd::PlanBatch { items })
}

/// The actions of a marked batch, then one bridge upgrade per job: one confirmation covers
/// them all (when any needs it), else they are queued; the tab's marks are cleared either way.
/// The `left_out` count goes into the confirmation, or a toast without one.
fn confirm_batch(
    app: &mut App,
    mut actions: Vec<Action>,
    bridges: Vec<Action>,
    confirm: ConfirmMode,
    tab: Tab,
    left_out: usize,
) -> Vec<Cmd> {
    actions.extend(bridges);
    if actions.is_empty() {
        toast_left_out(app, left_out);
        return Vec::new();
    }
    if has_third_party_code(app, &actions) || actions.iter().any(|a| needs_confirm(confirm, a)) {
        let notes = (left_out > 0).then(|| left_out_note(left_out));
        let modal = confirm_modal(app, actions, notes.into_iter().collect(), Some(tab));
        open_modal(app, modal);
        return Vec::new();
    }
    let mut cmds = Vec::new();
    for action in actions {
        cmds.extend(enqueue(app, action));
    }
    app.marks.remove(&tab);
    toast_left_out(app, left_out);
    cmds
}

/// Marked rows of Search: plan installing each Scoop one; winget ones are skipped.
fn install_marked(app: &mut App, rows: Vec<usize>, hidden: usize, left_out: usize) -> Vec<Cmd> {
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for i in rows {
        let Some(e) = app
            .hits
            .get(i)
            .and_then(|h| app.search.entries().get(h.index))
        else {
            continue;
        };
        if e.source == SourceKind::Scoop {
            items.push((
                e.name.clone(),
                Cmd::PlanInstall {
                    source_name: e.source_name.clone(),
                    id: e.id.clone(),
                    keep: false,
                },
            ));
        } else {
            skipped.push(e.name.clone());
        }
    }
    start_batch(app, items, Vec::new(), skipped, hidden, left_out)
}

/// Marked rows of Installed/Updates: plan upgrading those `upgrade_target` accepts.
fn upgrade_marked(app: &mut App, rows: &[usize], hidden: usize, left_out: usize) -> Vec<Cmd> {
    let keep = app.settings.behavior.keep_old;
    let mut items = Vec::new();
    let mut bridges = Vec::new();
    let mut skipped = Vec::new();
    let view = app.installed_view();
    for r in rows.iter().filter_map(|&i| view.get(i)) {
        match upgrade_target(app, r) {
            Ok(Target::Ours(source_name, id)) => items.push((
                r.name.clone(),
                Cmd::PlanInstall {
                    source_name,
                    id,
                    keep,
                },
            )),
            Ok(Target::Bridge(job)) => bridges.push(Action::Bridge(*job)),
            Ok(Target::Native { plan, row_key }) => bridges.push(Action::WingetUpgrade {
                plan: *plan,
                row_key,
            }),
            Err(_) => skipped.push(r.name.clone()),
        }
    }
    start_batch(app, items, bridges, skipped, hidden, left_out)
}

/// Marked rows of Installed: plan removing those wingetrs installed.
fn remove_marked(app: &mut App, rows: &[usize], hidden: usize, left_out: usize) -> Vec<Cmd> {
    let purge = app.settings.behavior.purge;
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    let view = app.installed_view();
    for r in rows.iter().filter_map(|&i| view.get(i)) {
        if r.origin == "wingetrs" {
            items.push((
                r.name.clone(),
                Cmd::PlanRemove {
                    app: r.name.clone(),
                    purge,
                },
            ));
        } else {
            skipped.push(r.name.clone());
        }
    }
    start_batch(app, items, Vec::new(), skipped, hidden, left_out)
}

/// `Enter` in Installed: the actions menu of the selected app, or of the marked ones.
fn open_actions(app: &mut App) -> Vec<Cmd> {
    if let Some((rows, _)) = marked_rows(app) {
        let view = app.installed_view();
        let marked: Vec<&ListRow> = rows.iter().filter_map(|&i| view.get(i).copied()).collect();
        let upgradable = marked
            .iter()
            .filter(|r| upgrade_target(app, r).is_ok())
            .count();
        let removable = marked.iter().filter(|r| r.origin == "wingetrs").count();
        let item = |label: &str, n: usize| PickerItem {
            label: format!("{label} marcados ({n})"),
            detail: String::new(),
            selectable: n > 0,
            note: (n == 0).then(|| "nenhum marcado aceita".to_string()),
        };
        let items = vec![item("Atualizar", upgradable), item("Remover", removable)];
        let title = format!("{} marcados", marked.len());
        app.modal = Some(Modal::Actions {
            picker: Picker::single_at(items, 0),
            app: title,
            upgrade: None,
            remove: None,
            pin: None,
            marked: true,
            choice: None,
            ids: Vec::new(),
            row: None,
        });
        return Vec::new();
    }
    open_item_menu(app)
}

/// The one rule for the actions of a row: label, the reason it is disabled (if it is) and what
/// it does. `row` is the visible index in the current tab.
fn actions_for(app: &App, row: usize) -> Vec<(String, Option<String>, ActionId)> {
    let marked = app.view_keys().get(row).is_some_and(|k| app.is_marked(k));
    if app.tab == Tab::Search {
        return search_actions(app, row, marked);
    }
    match app.installed_view().get(row).copied() {
        Some(r) => installed_actions(app, r, marked),
        None => Vec::new(),
    }
}

fn mark_item(marked: bool) -> (String, Option<String>, ActionId) {
    let label = if marked { "Desmarcar" } else { "Marcar" };
    (label.to_string(), None, ActionId::ToggleMark)
}

/// `actions_for` of a Search row.
fn search_actions(app: &App, row: usize, marked: bool) -> Vec<(String, Option<String>, ActionId)> {
    // Scoop packages install through the own installer, winget ones through the bridge.
    if app
        .hits
        .get(row)
        .and_then(|h| app.search.entries().get(h.index))
        .is_none()
    {
        return Vec::new();
    }
    vec![
        ("Instalar".to_string(), None, ActionId::Install),
        mark_item(marked),
    ]
}

/// `actions_for` of an Installed/Updates row.
fn installed_actions(
    app: &App,
    r: &ListRow,
    marked: bool,
) -> Vec<(String, Option<String>, ActionId)> {
    let upgrade = upgrade_target(app, r);
    let pinned = crate::pins::is_pinned(&app.pins, &r.origin, &r.name);
    let mut items = vec![
        (
            match &upgrade {
                Ok(Target::Bridge(job)) => format!("Atualizar (via {})", job.tool.name()),
                _ => "Atualizar".to_string(),
            },
            upgrade.as_ref().err().map(|n| n.to_string()),
            ActionId::Upgrade,
        ),
        (
            "Remover".to_string(),
            (r.origin != "wingetrs" && !winget_removable(&r.origin)).then(|| NOT_OURS.to_string()),
            ActionId::Remove,
        ),
    ];
    // The other records of the same app, each removable on its own.
    for (i, o) in r.others.iter().enumerate() {
        if winget_removable(&o.origin) {
            items.push((
                format!("Remover o registro {}", o.location),
                None,
                ActionId::RemoveRecord(i + 1),
            ));
        }
    }
    for (i, _) in row_orphans(r) {
        let label = match i {
            0 => "Remover registro órfão".to_string(),
            _ => format!(
                "Remover registro órfão ({})",
                r.others.get(i - 1).map_or("", |o| o.location.as_str())
            ),
        };
        items.push((label, None, ActionId::RemoveOrphan(i)));
    }
    items.push((
        if pinned { "Desafixar" } else { "Fixar versão" }.to_string(),
        None,
        ActionId::Pin,
    ));
    match row_choice(r) {
        Some(RowChoice::Choose { .. }) => items.insert(
            0,
            (
                "Escolher correspondência".to_string(),
                None,
                ActionId::Choose,
            ),
        ),
        Some(RowChoice::Undo { .. }) => items.push((
            "Desfazer escolha de correspondência".to_string(),
            None,
            ActionId::UndoChoice,
        )),
        None => {}
    }
    items.push(mark_item(marked));
    items
}

/// The correlation item the row offers, if any.
fn row_choice(r: &ListRow) -> Option<RowChoice> {
    match &r.correlation {
        Correlation::Ambiguous {
            source, candidates, ..
        } => Some(RowChoice::Choose {
            app_key: r.app_key.clone(),
            source: source.clone(),
            candidates: candidates.clone(),
        }),
        Correlation::Matched {
            rule: Rule::User, ..
        } => Some(RowChoice::Undo {
            app_key: r.app_key.clone(),
        }),
        Correlation::Unmatched if r.not_in_catalog => Some(RowChoice::Undo {
            app_key: r.app_key.clone(),
        }),
        _ => None,
    }
}

/// What the buttons act on: the rows of the current view (Installed/Updates), the target rows
/// (the visible marked ones, else the selected one) and how many marks are out of view.
struct ButtonTarget<'a> {
    view: Vec<&'a ListRow>,
    rows: Vec<usize>,
    marked: bool,
    hidden: usize,
}

impl<'a> ButtonTarget<'a> {
    /// Built once per call: `installed_view` sorts and filters the whole tab.
    fn new(app: &'a App) -> Self {
        let view = if app.tab == Tab::Search {
            Vec::new()
        } else {
            app.installed_view()
        };
        match marked_rows(app) {
            Some((rows, hidden)) => Self {
                view,
                rows,
                marked: true,
                hidden,
            },
            None => Self {
                view,
                rows: vec![app.selected],
                marked: false,
                hidden: 0,
            },
        }
    }

    /// Whether the action `id` applies to `row`, by the one rule of `actions_for`.
    fn possible(&self, app: &App, row: usize, id: ActionId) -> bool {
        // Pin and the correspondence items are never disabled: skip building the whole menu.
        if matches!(id, ActionId::Pin | ActionId::Choose | ActionId::UndoChoice) {
            return self.view.get(row).is_some_and(|r| {
                matches!(
                    (id, row_choice(r)),
                    (ActionId::Pin, _)
                        | (ActionId::Choose, Some(RowChoice::Choose { .. }))
                        | (ActionId::UndoChoice, Some(RowChoice::Undo { .. }))
                )
            });
        }
        let acts = if app.tab == Tab::Search {
            search_actions(app, row, false)
        } else {
            self.view
                .get(row)
                .map_or_else(Vec::new, |r| installed_actions(app, r, false))
        };
        acts.iter().any(|(_, note, i)| *i == id && note.is_none())
    }
}

fn is_pinned_row(app: &App, r: &ListRow) -> bool {
    crate::pins::is_pinned(&app.pins, &r.origin, &r.name)
}

/// The buttons under the details: label and action, only for actions that apply to at least
/// one row of the target. Keys are the ones the tab already has.
pub fn buttons(app: &App) -> Vec<(String, ActionId)> {
    if app.tab == Tab::Settings || app.visible_len() == 0 {
        return Vec::new();
    }
    let target = ButtonTarget::new(app);
    let ids: &[ActionId] = match app.tab {
        Tab::Search => &[ActionId::Install],
        Tab::Updates => &[ActionId::Upgrade],
        _ => &[
            ActionId::Upgrade,
            ActionId::Remove,
            ActionId::Pin,
            ActionId::Choose,
            ActionId::UndoChoice,
        ],
    };
    let mut out = Vec::new();
    for &id in ids {
        // The correspondence picker is for one app at a time.
        if id == ActionId::Choose && target.rows.len() != 1 {
            continue;
        }
        let Some(&first) = target.rows.iter().find(|&&r| target.possible(app, r, id)) else {
            continue;
        };
        let pinned = target
            .view
            .get(first)
            .is_some_and(|r| is_pinned_row(app, r));
        let base = match id {
            ActionId::Install => "Instalar",
            ActionId::Upgrade => "Atualizar",
            ActionId::Remove => "Remover",
            ActionId::Pin if pinned => "Desafixar",
            ActionId::Pin => "Fixar",
            ActionId::Choose => "Correspondência",
            ActionId::UndoChoice => "Desfazer correspondência",
            ActionId::ToggleMark | ActionId::RemoveRecord(_) | ActionId::RemoveOrphan(_) => {
                continue;
            }
        };
        let key = match (app.tab, id) {
            (Tab::Search, ActionId::Install) | (Tab::Updates, ActionId::Upgrade) => Some("enter"),
            (Tab::Installed, ActionId::Remove) => Some("u"),
            _ => None,
        };
        out.push((
            match key {
                Some(k) => format!("{base} ({k})"),
                None => base.to_string(),
            },
            id,
        ));
    }
    out
}

/// What `run_button` does to the rows where the action applies.
enum ButtonJob {
    Plan,
    Pin { keys: Vec<String>, pinned: bool },
    Choose(Option<RowChoice>),
    Undo(Vec<Cmd>),
}

/// Runs the action `id` over the rows of the target where it applies (the buttons and the
/// keys of a marked batch share this), and says how many were left out.
fn run_button(app: &mut App, id: ActionId) -> Vec<Cmd> {
    let (possible, left_out, marked, hidden, job) = {
        let target = ButtonTarget::new(app);
        let possible: Vec<usize> = target
            .rows
            .iter()
            .copied()
            .filter(|&r| target.possible(app, r, id))
            .collect();
        let chosen = || possible.iter().filter_map(|&i| target.view.get(i).copied());
        let job = match id {
            ActionId::Pin => ButtonJob::Pin {
                // Every row goes to the state opposite to the first one's.
                pinned: !chosen().next().is_some_and(|r| is_pinned_row(app, r)),
                keys: chosen()
                    .map(|r| crate::pins::key(&r.origin, &r.name))
                    .collect(),
            },
            ActionId::Choose => ButtonJob::Choose(chosen().next().and_then(row_choice)),
            ActionId::UndoChoice => ButtonJob::Undo(
                chosen()
                    .filter_map(|r| match row_choice(r) {
                        Some(RowChoice::Undo { app_key }) => Some(Cmd::SetOverride {
                            app_key,
                            choice: None,
                        }),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => ButtonJob::Plan,
        };
        let left_out = target.rows.len() - possible.len();
        (possible, left_out, target.marked, target.hidden, job)
    };
    if possible.is_empty() && !marked {
        return Vec::new();
    }
    // A marked batch plan says it in its confirmation (or toasts it without one).
    let batch = marked && matches!(job, ButtonJob::Plan);
    let cmds = match job {
        ButtonJob::Pin { keys, pinned } => keys
            .into_iter()
            .map(|key| {
                // Applied now; `Msg::PinSaved` reverts it if the write fails.
                if pinned {
                    app.pins.insert(key.clone());
                } else {
                    app.pins.remove(&key);
                }
                Cmd::SetPin { key, pinned }
            })
            .collect(),
        ButtonJob::Choose(choice) => open_choosing(app, choice),
        ButtonJob::Undo(cmds) => cmds,
        ButtonJob::Plan => match (id, marked) {
            (ActionId::Install, true) => install_marked(app, possible, hidden, left_out),
            (ActionId::Install, false) => install_one(app),
            (ActionId::Upgrade, true) => upgrade_marked(app, &possible, hidden, left_out),
            (ActionId::Upgrade, false) => upgrade_one(app),
            (ActionId::Remove, true) => remove_marked(app, &possible, hidden, left_out),
            (ActionId::Remove, false) => remove_one(app),
            _ => Vec::new(),
        },
    };
    if !batch {
        toast_left_out(app, left_out);
    }
    cmds
}

/// Rebuilds an open actions menu with the constructor that opened it, so it reflects the tools
/// just detected; the cursor stays on its item when that item is still enabled.
fn refresh_open_actions(app: &mut App) -> Vec<Cmd> {
    let Some(Modal::Actions { marked, picker, .. }) = &app.modal else {
        return Vec::new();
    };
    let (marked, at) = (*marked, picker.selected());
    let cmds = if marked {
        open_actions(app)
    } else {
        open_item_menu(app)
    };
    if let Some(Modal::Actions { picker, .. }) = &mut app.modal
        && picker.items().get(at).is_some_and(|i| i.selectable)
    {
        *picker = Picker::single_at(picker.items().to_vec(), at);
    }
    cmds
}

/// Opens the actions menu of the selected row (any list tab), whatever the marks. The cursor
/// starts on the first enabled item that is not `Remover`.
fn open_item_menu(app: &mut App) -> Vec<Cmd> {
    let acts = actions_for(app, app.selected);
    if acts.is_empty() {
        return Vec::new();
    }
    let at = acts
        .iter()
        .position(|(_, note, id)| note.is_none() && *id != ActionId::Remove)
        .unwrap_or(0);
    let items: Vec<PickerItem> = acts
        .iter()
        .map(|(label, note, _)| PickerItem {
            label: label.clone(),
            detail: String::new(),
            selectable: note.is_none(),
            note: note.clone(),
        })
        .collect();
    let ids: Vec<ActionId> = acts.iter().map(|(_, _, id)| *id).collect();
    let mut modal = Modal::Actions {
        picker: Picker::single_at(items, at),
        app: String::new(),
        upgrade: None,
        remove: None,
        pin: None,
        marked: false,
        choice: None,
        ids,
        row: None,
    };
    if app.tab == Tab::Search
        && let Some(e) = app
            .hits
            .get(app.selected)
            .and_then(|h| app.search.entries().get(h.index))
        && let Modal::Actions { app: name, .. } = &mut modal
    {
        *name = if e.name.is_empty() {
            e.id.clone()
        } else {
            e.name.clone()
        };
    }
    if app.tab != Tab::Search
        && let Some(r) = app.installed_view().get(app.selected).copied()
    {
        let pinned = crate::pins::is_pinned(&app.pins, &r.origin, &r.name);
        if let Modal::Actions {
            app: name,
            upgrade,
            remove,
            pin,
            choice,
            row,
            ..
        } = &mut modal
        {
            *row = Some(Box::new(r.clone()));
            *name = r.name.clone();
            *upgrade = upgrade_target(app, r).ok();
            *remove = (r.origin == "wingetrs").then(|| r.name.clone());
            *pin = Some((crate::pins::key(&r.origin, &r.name), !pinned));
            *choice = row_choice(r);
        }
    }
    app.modal = Some(modal);
    Vec::new()
}

/// Runs the item `id` of the actions menu of the selected row.
fn run_action(
    app: &mut App,
    id: ActionId,
    upgrade: Option<Target>,
    remove: Option<String>,
    pin: Option<(String, bool)>,
    choice: Option<RowChoice>,
    row: Option<Box<ListRow>>,
) -> Vec<Cmd> {
    match id {
        ActionId::Install => install_one(app),
        ActionId::Upgrade => match upgrade {
            Some(Target::Ours(source_name, id)) => plan_upgrade(app, source_name, id),
            Some(Target::Bridge(job)) => start_bridge(app, *job),
            Some(Target::Native { plan, row_key }) => {
                let confirm = app.settings.behavior.confirm;
                let action = Action::WingetUpgrade {
                    plan: *plan,
                    row_key,
                };
                start_ready(app, action, confirm, Vec::new())
            }
            None => Vec::new(),
        },
        ActionId::Remove => match (remove, row) {
            (Some(target), _) => plan_remove(app, target),
            (None, Some(r)) if winget_removable(&r.origin) => plan_winget_remove(app, &r, 0),
            _ => Vec::new(),
        },
        ActionId::RemoveRecord(n) => row.map_or_else(Vec::new, |r| plan_winget_remove(app, &r, n)),
        ActionId::RemoveOrphan(n) => {
            let orphan = row.and_then(|r| {
                row_orphans(&r)
                    .into_iter()
                    .find_map(|(i, o)| (i == n).then_some(o))
            });
            orphan.map_or_else(Vec::new, |o| ask_orphan(app, o))
        }
        ActionId::Pin => match pin {
            Some((key, pinned)) => {
                // Applied now; `Msg::PinSaved` reverts it if the write fails.
                if pinned {
                    app.pins.insert(key.clone());
                } else {
                    app.pins.remove(&key);
                }
                vec![Cmd::SetPin { key, pinned }]
            }
            None => Vec::new(),
        },
        ActionId::Choose => open_choosing(app, choice),
        ActionId::UndoChoice => match choice {
            Some(RowChoice::Undo { app_key }) => vec![Cmd::SetOverride {
                app_key,
                choice: None,
            }],
            _ => Vec::new(),
        },
        ActionId::ToggleMark => {
            app.toggle_mark();
            Vec::new()
        }
    }
}

/// Opens the picker of the catalog candidates of an ambiguous installed row.
fn open_choosing(app: &mut App, choice: Option<RowChoice>) -> Vec<Cmd> {
    let Some(RowChoice::Choose {
        app_key,
        source,
        candidates,
    }) = choice
    else {
        return Vec::new();
    };
    let mut items: Vec<PickerItem> = candidates
        .iter()
        .map(|id| {
            let detail = app
                .search
                .entries()
                .iter()
                .find(|e| e.source_name == source && e.id == *id)
                .map(|e| match &e.publisher {
                    Some(p) => format!("{} · {p}", e.name),
                    None => e.name.clone(),
                })
                .unwrap_or_default();
            PickerItem {
                label: id.clone(),
                detail,
                selectable: true,
                note: None,
            }
        })
        .collect();
    items.push(PickerItem {
        label: "Nenhum destes (não está no catálogo)".to_string(),
        detail: String::new(),
        selectable: true,
        note: None,
    });
    app.modal = Some(Modal::Choosing {
        picker: Picker::single_at(items, 0),
        app_key,
        source,
        candidates,
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
        ConfirmMode::Removals => matches!(
            action,
            Action::Remove(_) | Action::WingetUninstall(_) | Action::WingetRegistryOnly(_)
        ),
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
    app.bridge_last_line = matches!(q.action, Action::Bridge(_)).then_some(app.now);
    use crate::install::progress::Phase;
    // A bridge job reports only lines: its phase (spinner and clock) starts now. So does a
    // bucket job, before its first download event.
    let progress = match &q.action {
        Action::Bridge(job) => Some(phase_progress(
            Phase::Bridge {
                tool: job.tool.name().to_string(),
            },
            app.now,
        )),
        Action::BucketAdd { .. } => Some(phase_progress(
            Phase::FetchingCatalog { sources: 1 },
            app.now,
        )),
        Action::BucketRemove { name, .. } => Some(phase_progress(
            Phase::Removing {
                app: format!("bucket {name}"),
            },
            app.now,
        )),
        _ => None,
    };
    app.running = Some(Running {
        id: q.id,
        label: q.label,
        progress,
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
    if job_uncancellable(app) {
        let native = app
            .running
            .as_ref()
            .and_then(|r| r.action.as_ref())
            .is_some_and(crate::tui::jobs::is_winget_action);
        let mut text = if native {
            NATIVE_QUIT.to_string()
        } else {
            MSIEXEC_QUIT.to_string()
        };
        if n > 0 {
            text.push_str(&format!(" Sair cancela as {n} operações na fila."));
        }
        app.modal = Some(Modal::ConfirmQuit { text });
        return Vec::new();
    }
    let mut text = match (&app.running, n) {
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
        text.push_str(" Resíduos dela não serão limpos.");
    }
    app.modal = Some(Modal::ConfirmQuit { text });
    Vec::new()
}

/// `Cmd::CancelNative` when the job in progress is a native winget one whose installer has
/// not launched (its `LaunchGate` then refuses the launch).
fn cancel_native(app: &App) -> Vec<Cmd> {
    let native = app
        .running
        .as_ref()
        .and_then(|r| r.action.as_ref())
        .is_some_and(crate::tui::jobs::is_winget_action);
    if native && !job_uncancellable(app) {
        vec![Cmd::CancelNative]
    } else {
        Vec::new()
    }
}

/// The job in progress launched `msiexec /a`, which cannot be cancelled: quitting waits for
/// it, even on a second Ctrl+C. Before the launch (download, locks, installer queue) it can.
pub fn job_uncancellable(app: &App) -> bool {
    app.installer_running && app.running.is_some()
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

/// Task 218: the administrator-terminal refusal of `action` when this process is elevated
/// (the same text as the CLI's).
fn admin_refusal(app: &App, action: &Action, elevated: bool) -> Option<String> {
    match action {
        Action::Apply(groups) => groups.iter().find_map(|g| plans_admin_refusal(elevated, g)),
        Action::Remove(p) => {
            let code = app.removal_code.get(&p.app.to_lowercase());
            let code = code.map(|(_, c)| c.as_slice()).unwrap_or_default();
            admin_code_refusal(elevated, code, false)
        }
        Action::WingetUninstall(p) => winget_admin_refusal(p, elevated),
        _ => None,
    }
}

/// Task 218: what of `action` an administrator terminal lets go on, before any confirmation.
/// A refused item is an error toast; in an upgrade of several apps each refused group is
/// skipped with a warning and the others go on. `None`: nothing is left.
fn admin_filter(app: &mut App, action: Action) -> Option<Action> {
    let elevated = elevated_or_unknown(app.elevated);
    if !elevated {
        return Some(action);
    }
    match action {
        Action::Apply(groups) if groups.len() > 1 => {
            let mut kept = Vec::new();
            for g in groups {
                match plans_admin_refusal(elevated, &g) {
                    Some(r) => {
                        let apps: Vec<&str> = g.iter().map(|p| p.app.as_str()).collect();
                        app.push_toast(
                            ToastKind::Warning,
                            format!("{} pulado: {r}", apps.join(", ")),
                        );
                    }
                    None => kept.push(g),
                }
            }
            (!kept.is_empty()).then_some(Action::Apply(kept))
        }
        other => match admin_refusal(app, &other, elevated) {
            Some(r) => {
                if let Action::Remove(p) = &other {
                    app.removal_code.remove(&p.app.to_lowercase());
                }
                app.push_toast(ToastKind::Error, r);
                None
            }
            None => Some(other),
        },
    }
}

/// [`admin_filter`] of one item of a batch: a refused item is skipped with the reason.
fn admin_batch_item(app: &mut App, label: &str, action: Action) -> Option<Action> {
    let elevated = elevated_or_unknown(app.elevated);
    let Some(r) = admin_refusal(app, &action, elevated) else {
        return Some(action);
    };
    if let Action::Remove(p) = &action {
        app.removal_code.remove(&p.app.to_lowercase());
    }
    app.push_toast(ToastKind::Warning, format!("{label} pulado: {r}"));
    None
}

fn on_planned(app: &mut App, result: Result<Planned, String>) -> Vec<Cmd> {
    app.planning = false;
    app.status = None;
    let confirm = app
        .planning_confirm
        .take()
        .unwrap_or(app.settings.behavior.confirm);
    match result {
        Ok(Planned::Ready { action, .. }) => {
            if let Action::Remove(p) = &action {
                app.removal_code.remove(&p.app.to_lowercase());
            }
            let Some(action) = admin_filter(app, action) else {
                return Vec::new();
            };
            return ready_one(app, action, confirm);
        }
        Ok(Planned::ReadyScripts {
            action,
            code,
            manifest_sha256,
            ..
        }) => {
            if let Action::Remove(p) = &action {
                app.removal_code
                    .insert(p.app.to_lowercase(), (manifest_sha256, code));
            }
            let Some(action) = admin_filter(app, action) else {
                return Vec::new();
            };
            return ready_one(app, action, confirm);
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

/// A ready plan: confirmed per the `confirm` setting, and always when it runs third-party code.
fn ready_one(app: &mut App, action: Action, confirm: ConfirmMode) -> Vec<Cmd> {
    let actions = vec![action];
    if !needs_confirm(confirm, &actions[0]) && !has_third_party_code(app, &actions) {
        return actions.into_iter().flat_map(|a| enqueue(app, a)).collect();
    }
    let modal = confirm_modal(app, actions, Vec::new(), None);
    open_modal(app, modal);
    Vec::new()
}

/// A portable package planned as a wingetrs app: like any 2a plan, except that license
/// agreements always ask for the confirmation that shows them.
fn on_portable_planned(
    app: &mut App,
    result: Result<Planned, String>,
    agreements: Vec<String>,
) -> Vec<Cmd> {
    match result {
        Ok(Planned::Ready { action, .. }) if !agreements.is_empty() => {
            app.planning = false;
            app.status = None;
            app.planning_confirm = None;
            let Some(action) = admin_filter(app, action) else {
                return Vec::new();
            };
            let modal = confirm_modal(app, vec![action], agreements, None);
            open_modal(app, modal);
            Vec::new()
        }
        other => on_planned(app, other),
    }
}

/// The winget install planned by `Cmd::PlanWingetInstall`: confirmed (always, when the package
/// has license agreements to show), then queued as a bridge job.
fn on_winget_planned(app: &mut App, result: Result<(BridgeJob, Vec<String>), String>) -> Vec<Cmd> {
    app.planning = false;
    app.status = None;
    let confirm = app
        .planning_confirm
        .take()
        .unwrap_or(app.settings.behavior.confirm);
    match result {
        Ok((job, agreements)) => {
            let action = Action::Bridge(job);
            if agreements.is_empty() && !needs_confirm(confirm, &action) {
                return enqueue(app, action);
            }
            let modal = confirm_modal(app, vec![action], agreements, None);
            open_modal(app, modal);
        }
        Err(message) => app.push_toast(ToastKind::Error, message),
    }
    Vec::new()
}

/// The winget install planned by `Cmd::PlanWingetInstall` with both engines' answers: the
/// Settings engine chooses (`choose_engine`); native ⇒ the plan's text in the confirmation,
/// bridge ⇒ the bridge job, with why it goes through winget when it was not the setting.
fn on_winget_install_planned(
    app: &mut App,
    bridge: Result<(BridgeJob, Vec<String>), String>,
    native: Result<PlanOrBridge, String>,
) -> Vec<Cmd> {
    let engine = app.settings.behavior.winget_engine;
    match choose_engine(engine, native.map_err(Refusal)) {
        Ok(Engine::Native(plan)) => {
            app.planning = false;
            app.status = None;
            let confirm = app
                .planning_confirm
                .take()
                .unwrap_or(app.settings.behavior.confirm);
            start_ready(app, Action::WingetInstall(plan), confirm, Vec::new())
        }
        // Still planning: the 2a plan needs `state.db` and the root, read on a job.
        Ok(Engine::Portable(spec)) => {
            let keep = app.settings.behavior.keep_old;
            app.planning_keep = keep;
            vec![Cmd::PlanPortable { spec, keep }]
        }
        Ok(Engine::Bridge(why)) => {
            let Some(why) = why else {
                return on_winget_planned(app, bridge);
            };
            // The reason shows only in a confirmation; it never forces one.
            app.planning = false;
            app.status = None;
            let confirm = app
                .planning_confirm
                .take()
                .unwrap_or(app.settings.behavior.confirm);
            match bridge {
                Ok((job, agreements)) => {
                    let action = Action::Bridge(job);
                    if agreements.is_empty() && !needs_confirm(confirm, &action) {
                        // No confirmation to show the reason in: a toast says it.
                        app.push_toast(
                            ToastKind::Info,
                            format!(
                                "Pelo winget: {}",
                                crate::winget_install::plan::bridge_reason(&why)
                            ),
                        );
                        return enqueue(app, action);
                    }
                    let mut notes = vec![format!(
                        "Pelo winget: {}",
                        crate::winget_install::plan::bridge_reason(&why)
                    )];
                    notes.extend(agreements);
                    let modal = confirm_modal(app, vec![action], notes, None);
                    open_modal(app, modal);
                }
                Err(message) => app.push_toast(ToastKind::Error, message),
            }
            Vec::new()
        }
        Err(Refusal(why)) => {
            app.planning = false;
            app.status = None;
            app.planning_confirm = None;
            app.push_toast(ToastKind::Error, why);
            Vec::new()
        }
    }
}

/// The native removal planned by `Cmd::PlanWingetUninstall`.
fn on_winget_uninstall_planned(
    app: &mut App,
    result: Result<WingetUninstallPlan, String>,
) -> Vec<Cmd> {
    app.planning = false;
    app.status = None;
    let confirm = app
        .planning_confirm
        .take()
        .unwrap_or(app.settings.behavior.confirm);
    // Refused by winget from here, yet the record alone may go: the confirmation offers only
    // that (no item that calls winget), with the refusal as its note; always shown.
    if let Ok(plan) = &result
        && plan
            .shared
            .as_ref()
            .is_some_and(|s| s.registry_only_allowed)
        && let Some(r) = winget_admin_refusal(plan, elevated_or_unknown(app.elevated))
    {
        let action = Action::WingetRegistryOnly(plan.clone());
        let modal = confirm_modal(app, vec![action], vec![r], None);
        open_modal(app, modal);
        return Vec::new();
    }
    match result {
        Ok(plan) => match admin_filter(app, Action::WingetUninstall(plan)) {
            Some(action) => start_ready(app, action, confirm, Vec::new()),
            None => Vec::new(),
        },
        Err(message) => {
            app.push_toast(ToastKind::Error, message);
            Vec::new()
        }
    }
}

/// Results of a batch plan: one confirmation for all the plans, then everything is queued.
fn on_batch_planned(app: &mut App, results: Vec<Result<Planned, String>>) -> Vec<Cmd> {
    app.planning = false;
    app.status = None;
    let confirm = app
        .planning_confirm
        .take()
        .unwrap_or(app.settings.behavior.confirm);
    let Some(batch) = app.batch.take() else {
        return Vec::new();
    };
    report_left_out(app, &batch.skipped, batch.hidden);
    let mut groups = Vec::new();
    let mut removals = Vec::new();
    let mut ambiguous = Vec::new();
    for (i, result) in results.into_iter().enumerate() {
        let label = batch.labels.get(i).cloned().unwrap_or_default();
        match result {
            Ok(Planned::Ready { action, .. }) => {
                if let Action::Remove(p) = &action {
                    app.removal_code.remove(&p.app.to_lowercase());
                }
                match admin_batch_item(app, &label, action) {
                    Some(Action::Apply(g)) => groups.extend(g),
                    Some(other) => removals.push(other),
                    None => {}
                }
            }
            Ok(Planned::ReadyScripts {
                action,
                code,
                manifest_sha256,
                ..
            }) => {
                if let Action::Remove(p) = &action {
                    app.removal_code
                        .insert(p.app.to_lowercase(), (manifest_sha256, code));
                }
                removals.extend(admin_batch_item(app, &label, action));
            }
            Ok(Planned::Choose(_)) => ambiguous.push(label),
            Err(message) => app.push_toast(ToastKind::Error, format!("{label}: {message}")),
        }
    }
    if !ambiguous.is_empty() {
        app.push_toast(
            ToastKind::Warning,
            format!("ambíguo, escolha individualmente: {}", ambiguous.join(", ")),
        );
    }
    let mut actions = Vec::new();
    if !groups.is_empty() {
        actions.push(Action::Apply(groups));
    }
    actions.extend(removals);
    confirm_batch(
        app,
        actions,
        batch.bridges,
        confirm,
        batch.tab,
        batch.left_out,
    )
}

fn on_finished(app: &mut App, result: Result<Done, InstallFail>) -> Vec<Cmd> {
    let finished = app.running.take();
    if result.is_ok()
        && matches!(
            finished.as_ref().and_then(|r| r.action.as_ref()),
            Some(Action::SelfUpdate { .. })
        )
    {
        app.self_update = None;
    }
    app.installer_running = false;
    app.bridge_last_line = None;
    let bucket = matches!(
        finished.as_ref().and_then(|r| r.action.as_ref()),
        Some(Action::BucketAdd { .. } | Action::BucketRemove { .. })
    );
    // Success or not, the disk may have changed.
    let mut cmds = vec![Cmd::ScanInstalled];
    match result {
        Ok(done) => {
            if bucket {
                // The summary line, the open dialog and the search see the new catalog.
                cmds.extend([Cmd::LoadBuckets, Cmd::ReloadCatalog]);
            }
            app.push_toast(ToastKind::Success, done.summary);
            if let Some((name, residues)) = done.residues
                && !residues.is_empty()
            {
                cmds.extend(handle_residues(app, name, residues));
            }
        }
        Err(InstallFail::Busy) => {
            let text = if bucket {
                "uma atualização está em andamento; o bucket espera na fila"
            } else {
                "outra instalação em andamento"
            };
            app.push_toast(ToastKind::Warning, text);
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
        Err(InstallFail::NeedsMsiexec {
            app: name,
            reason,
            applied,
        }) => {
            let (groups, scripts) = match finished.and_then(|r| r.action) {
                Some(Action::Apply(groups)) => (groups, false),
                Some(Action::ApplyScripts { groups }) => (groups, true),
                Some(Action::ApplyMsiexec {
                    groups, scripts, ..
                }) => (groups, scripts),
                _ => (Vec::new(), false),
            };
            cmds.extend(ask_msiexec(app, name, reason, &applied, groups, scripts));
        }
        Err(InstallFail::Other(message)) => app.push_toast(ToastKind::Error, message),
    }
    cmds.extend(pump(app));
    cmds
}

/// An MSI of `name` needs `msiexec /a`. The groups not applied yet that contain it wait for
/// the user to confirm that one item (`Continuar` queues them allowed for `name` only;
/// `Cancelar` ends the item as an error); the other groups not applied go back in line.
fn ask_msiexec(
    app: &mut App,
    name: String,
    reason: String,
    applied: &[String],
    groups: Vec<Vec<InstallPlan>>,
    scripts: bool,
) -> Vec<Cmd> {
    let done = |p: &InstallPlan| applied.iter().any(|a| a.eq_ignore_ascii_case(&p.app));
    let (with, rest): (Vec<_>, Vec<_>) = groups
        .into_iter()
        .filter(|g| g.last().is_some_and(|t| !done(t)))
        .partition(|g| g.iter().any(|p| p.app.eq_ignore_ascii_case(&name)));
    if with.is_empty() {
        // Not one of this action's apps: nothing to confirm, and nothing is run again.
        app.push_toast(
            ToastKind::Error,
            format!("{name}: o MSI precisa do Windows Installer ({reason})"),
        );
        return Vec::new();
    }
    let mut cmds = Vec::new();
    if !rest.is_empty() {
        // The rest keeps the consent the user gave to the whole batch.
        let again = if scripts {
            Action::ApplyScripts { groups: rest }
        } else {
            Action::Apply(rest)
        };
        cmds.extend(enqueue(app, again));
    }
    let action = Action::ApplyMsiexec {
        groups: with,
        app: name.clone(),
        scripts,
    };
    let note = format!("{name}: o extrator do wingetrs não abre este MSI ({reason}).");
    let modal = confirm_modal(app, vec![action], vec![note], None);
    open_modal(app, modal);
    cmds
}

/// Ctrl+U: with a newer release known, asks (the usual buttons) to update wingetrs; the update
/// itself is queued only after Continuar. Refused with a toast while an operation runs or
/// waits (the executable must not be swapped under an installer), and without a newer release
/// it does nothing.
fn ask_self_update(app: &mut App) -> Vec<Cmd> {
    let Some(tag) = app.self_update.clone() else {
        return Vec::new();
    };
    let Some(new) = crate::selfupdate::parse_tag(&tag) else {
        return Vec::new();
    };
    if !can_start(app) || !app.queue.is_empty() {
        app.push_toast(ToastKind::Warning, "espere a operação atual");
        return Vec::new();
    }
    let note = format!("Atualizar o wingetrs de {} para {new}?", app.version);
    let modal = confirm_modal(app, vec![Action::SelfUpdate { tag }], vec![note], None);
    open_modal(app, modal);
    Vec::new()
}

/// `Cancelar` (or `Esc`) on a confirmation; declining `msiexec /a` ends that item as an error.
fn cancel_confirm(app: &mut App) -> Vec<Cmd> {
    if let Some(Modal::Confirm { actions, .. }) = app.modal.take() {
        for action in actions {
            if let Action::ApplyMsiexec { app: name, .. } = action {
                app.push_toast(
                    ToastKind::Error,
                    format!("{name}: cancelado; o MSI precisa do Windows Installer"),
                );
            }
        }
    }
    Vec::new()
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

/// Items of the open confirmation that fit on screen (at least one).
fn confirm_rows(app: &App) -> usize {
    let Some(Modal::Confirm {
        items,
        notes,
        scripts,
        ..
    }) = &app.modal
    else {
        return 1;
    };
    let area = ratatui::layout::Rect::new(0, 0, app.size.0, app.size.1);
    let geom = crate::tui::regions::confirm_geom(items.len(), notes, &scripts.lines, area);
    (usize::from(geom.list.height) / ITEM_ROWS).max(1)
}

/// Whether the open confirmation has third-party code but too little room to show more than
/// the section title (fewer than two lines of it).
fn scripts_hidden(app: &App) -> bool {
    let Some(Modal::Confirm {
        items,
        notes,
        scripts,
        ..
    }) = &app.modal
    else {
        return false;
    };
    if scripts.lines.is_empty() {
        return false;
    }
    let area = ratatui::layout::Rect::new(0, 0, app.size.0, app.size.1);
    let geom = crate::tui::regions::confirm_geom(items.len(), notes, &scripts.lines, area);
    geom.scripts.height < 3
}

/// `(rows, width)` the scripts section draws its lines in (at least one row).
fn script_rows(app: &App) -> (usize, usize) {
    let Some(Modal::Confirm {
        items,
        notes,
        scripts,
        ..
    }) = &app.modal
    else {
        return (1, 1);
    };
    let area = ratatui::layout::Rect::new(0, 0, app.size.0, app.size.1);
    let geom = crate::tui::regions::confirm_geom(items.len(), notes, &scripts.lines, area);
    (
        usize::from(geom.scripts.height.saturating_sub(1)).max(1),
        usize::from(geom.scripts.width).max(1),
    )
}

/// Keys of the confirmation: arrows move the cursor (the list scrolls to keep it on screen),
/// `Espaço` toggles the item, `Enter` continues, `Esc` cancels.
fn confirm_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    let rows = confirm_rows(app);
    let (page_rows, page_width) = script_rows(app);
    let Some(Modal::Confirm {
        items,
        notes,
        scroll,
        cursor,
        actions,
        scripts,
        ..
    }) = &mut app.modal
    else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Enter => return continue_confirm(app),
        KeyCode::Esc => return cancel_confirm(app),
        KeyCode::PageDown => {
            use crate::tui::regions::{script_fit, script_max_scroll};
            let lines = &scripts.lines;
            let page = script_fit(lines, scripts.scroll, page_rows, page_width);
            let last = script_max_scroll(lines, page_rows, page_width);
            scripts.scroll = (scripts.scroll + page).min(last);
            return Vec::new();
        }
        KeyCode::PageUp => {
            let back = crate::tui::regions::script_fit_back(
                &scripts.lines,
                scripts.scroll,
                page_rows,
                page_width,
            );
            scripts.scroll = scripts.scroll.saturating_sub(back);
            return Vec::new();
        }
        KeyCode::Up => *cursor = cursor.saturating_sub(1),
        KeyCode::Down => *cursor = (*cursor + 1).min(items.len().saturating_sub(1)),
        KeyCode::Char(' ') => toggle_confirm_item(items, actions, notes, *cursor),
        _ => {}
    }
    *scroll = crate::tui::view::list_window(*cursor, *scroll, items.len(), rows).start;
    Vec::new()
}

/// `Continuar`: queues the checked items of the confirmation; with none checked it stays.
fn continue_confirm(app: &mut App) -> Vec<Cmd> {
    let any =
        matches!(&app.modal, Some(Modal::Confirm { items, .. }) if items.iter().any(|i| i.checked));
    if !any {
        return Vec::new();
    }
    let refused = match &app.modal {
        Some(Modal::Confirm {
            items,
            actions,
            scripts,
            ..
        }) => authorize_scripts(scripts, checked_actions(items, actions.clone())).err(),
        _ => None,
    };
    // Consent needs the code on screen: a window too small to show it blocks Continue.
    if scripts_hidden(app) {
        app.push_toast(
            ToastKind::Warning,
            "A janela é pequena demais para mostrar os scripts; aumente a janela para revisá-los antes de continuar",
        );
        return Vec::new();
    }
    if let Some(name) = refused {
        // A refused script never runs: the dialog stays so the item can be unchecked.
        app.push_toast(
            ToastKind::Error,
            format!("{name}: um script foi recusado pela análise e não roda"),
        );
        return Vec::new();
    }
    let Some(Modal::Confirm {
        items,
        actions,
        clear_marks,
        scripts,
        ..
    }) = app.modal.take()
    else {
        return Vec::new();
    };
    let checked = checked_actions(&items, actions);
    let Ok(authorized) = authorize_scripts(&scripts, checked) else {
        return Vec::new();
    };
    let mut cmds = Vec::new();
    for action in authorized {
        cmds.extend(enqueue(app, action));
    }
    if let Some(tab) = clear_marks {
        app.marks.remove(&tab);
    }
    cmds
}

/// Mouse on the confirmation: a click on a row toggles it, on a button presses it; the wheel
/// scrolls the list.
fn confirm_mouse(app: &mut App, ev: MouseEvent) -> Vec<Cmd> {
    let area = ratatui::layout::Rect::new(0, 0, app.size.0, app.size.1);
    let r = crate::tui::regions::regions(app, area);
    let inside = |rect: ratatui::layout::Rect| rect.contains((ev.column, ev.row).into());
    if let MouseEventKind::Down(MouseButton::Left) = ev.kind
        && let Some((_, button)) = r.confirm_buttons.iter().find(|(rect, _)| inside(*rect))
    {
        return match button {
            ConfirmButton::Continue => continue_confirm(app),
            ConfirmButton::Cancel => cancel_confirm(app),
        };
    }
    let rows = usize::from(r.confirm_list.height) / ITEM_ROWS;
    let script_page = (
        usize::from(r.confirm_scripts.height.saturating_sub(1)).max(1),
        usize::from(r.confirm_scripts.width).max(1),
    );
    let Some(Modal::Confirm {
        items,
        notes,
        scroll,
        cursor,
        actions,
        scripts,
        ..
    }) = &mut app.modal
    else {
        return Vec::new();
    };
    if matches!(
        ev.kind,
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
    ) && inside(r.confirm_scripts)
    {
        let last =
            crate::tui::regions::script_max_scroll(&scripts.lines, script_page.0, script_page.1);
        scripts.scroll = if ev.kind == MouseEventKind::ScrollDown {
            (scripts.scroll + WHEEL_ITEMS).min(last)
        } else {
            scripts.scroll.saturating_sub(WHEEL_ITEMS)
        };
        return Vec::new();
    }
    let shown = crate::tui::regions::confirm_window(*scroll, items.len(), rows);
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) if inside(r.confirm_list) => {
            let index = shown.start + usize::from(ev.row - r.confirm_list.y) / ITEM_ROWS;
            if index < shown.end {
                toggle_confirm_item(items, actions, notes, index);
                *cursor = index;
            }
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if inside(r.confirm) => {
            let start = if ev.kind == MouseEventKind::ScrollDown {
                (shown.start + WHEEL_ITEMS).min(items.len().saturating_sub(rows))
            } else {
                shown.start.saturating_sub(WHEEL_ITEMS)
            };
            *scroll = start;
            // The cursor stays on screen.
            let last = (start + rows.max(1)).min(items.len()).saturating_sub(1);
            *cursor = (*cursor).clamp(start.min(last), last);
        }
        _ => {}
    }
    Vec::new()
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
        modal @ Modal::Confirm { .. } => {
            app.modal = Some(modal);
            confirm_key(app, key)
        }
        Modal::ConfirmQuit { text } => match key.code {
            KeyCode::Enter => {
                // Quit: the pending ones are dropped, the one in progress is waited for (a
                // native winget job not launched yet is cancelled: it never launches).
                app.queue.clear();
                app.quit_after_running = true;
                let mut cmds = cancel_native(app);
                cmds.extend(pump(app));
                cmds
            }
            KeyCode::Esc => Vec::new(),
            _ => {
                app.modal = Some(Modal::ConfirmQuit { text });
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
                Some(c) if c.selectable && c.entry.source == SourceKind::Winget => {
                    let cmd = Cmd::PlanWingetInstall {
                        id: c.entry.id.clone(),
                    };
                    start_planning(app, cmd)
                }
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
            pin,
            marked,
            choice,
            ids,
            row,
        } => match picker.key(key) {
            PickOutcome::Pending => {
                app.modal = Some(Modal::Actions {
                    picker,
                    app: name,
                    upgrade,
                    remove,
                    pin,
                    marked,
                    choice,
                    ids,
                    row,
                });
                Vec::new()
            }
            PickOutcome::Cancelled => Vec::new(),
            PickOutcome::Picked(chosen) if marked => match (chosen.first(), marked_rows(app)) {
                (Some(0), Some((rows, hidden))) => upgrade_marked(app, &rows, hidden, 0),
                (Some(1), Some((rows, hidden))) => remove_marked(app, &rows, hidden, 0),
                _ => Vec::new(),
            },
            PickOutcome::Picked(chosen) => {
                match chosen.first().and_then(|&i| ids.get(i)).copied() {
                    Some(id) => run_action(app, id, upgrade, remove, pin, choice, row),
                    None => Vec::new(),
                }
            }
        },
        Modal::Choosing {
            mut picker,
            app_key,
            source,
            candidates,
        } => match picker.key(key) {
            PickOutcome::Pending => {
                app.modal = Some(Modal::Choosing {
                    picker,
                    app_key,
                    source,
                    candidates,
                });
                Vec::new()
            }
            PickOutcome::Cancelled => Vec::new(),
            PickOutcome::Picked(chosen) => {
                let choice = match chosen.first().and_then(|&i| candidates.get(i)) {
                    Some(id) => Choice::Package {
                        source,
                        package_id: id.clone(),
                    },
                    None => Choice::NotInCatalog,
                };
                vec![Cmd::SetOverride {
                    app_key,
                    choice: Some(choice),
                }]
            }
        },
        Modal::Queue { selected } => queue_key(app, selected, key),
        Modal::Buckets(dialog) => buckets_key(app, *dialog, key),
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
/// `b` in Settings: the buckets dialog, filled when `Msg::BucketsLoaded` arrives.
fn open_buckets(app: &mut App) -> Vec<Cmd> {
    app.modal = Some(Modal::Buckets(Box::new(BucketsDialog {
        cfg: Config::default(),
        rows: Vec::new(),
        cursor: 0,
        loading: true,
        stage: BucketStage::List,
    })));
    vec![Cmd::LoadBuckets]
}

/// New bucket list: the Settings summary line always, the dialog when it is open.
fn on_buckets_loaded(app: &mut App, result: Result<(Config, Vec<BucketInfo>), String>) -> Vec<Cmd> {
    let (cfg, rows) = match result {
        Ok(loaded) => loaded,
        Err(e) => {
            app.push_toast(ToastKind::Error, format!("falha ao ler os buckets: {e}"));
            if let Some(Modal::Buckets(d)) = &mut app.modal {
                d.loading = false;
            }
            return Vec::new();
        }
    };
    app.buckets_summary = rows
        .iter()
        .map(|b| b.row.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if let Some(Modal::Buckets(d)) = &mut app.modal {
        d.cursor = d.cursor.min(rows.len().saturating_sub(1));
        d.cfg = cfg;
        d.rows = rows;
        d.loading = false;
    }
    Vec::new()
}

/// The add picker: the well-known buckets not configured, then `OTHER_REPO`.
fn add_picker(d: &BucketsDialog) -> BucketStage {
    let missing: Vec<(&str, &str)> = crate::buckets::KNOWN
        .iter()
        .filter(|(n, _)| !d.rows.iter().any(|b| b.row.name == *n))
        .copied()
        .collect();
    let mut items: Vec<PickerItem> = missing
        .iter()
        .map(|(n, repo)| PickerItem {
            label: n.to_string(),
            detail: repo.to_string(),
            selectable: true,
            note: None,
        })
        .collect();
    items.push(PickerItem {
        label: OTHER_REPO.to_string(),
        detail: String::new(),
        selectable: true,
        note: None,
    });
    BucketStage::Add {
        picker: Picker::single(items),
        names: missing.iter().map(|(n, _)| n.to_string()).collect(),
    }
}

/// The bucket name an `owner/repo` gets when added from the free-form field.
pub fn bucket_name_of(repo: &str) -> String {
    repo.rsplit('/').next().unwrap_or(repo).to_lowercase()
}

/// Adds `name` from `spec` by the CLI's rule (`buckets::add` on a copy of the config): a
/// bucket wingetrs does not know asks first; anything else wrong is a toast.
fn try_add(app: &mut App, d: &mut BucketsDialog, name: &str, spec: &str) -> Vec<Cmd> {
    use crate::buckets::BucketError;
    let mut probe = d.cfg.clone();
    match crate::buckets::add(&mut probe, name, spec, false) {
        Ok(()) => enqueue(
            app,
            Action::BucketAdd {
                name: name.to_string(),
                spec: spec.to_string(),
                confirmed: false,
            },
        ),
        Err(BucketError::NeedsConfirmation { name, repo }) => {
            d.stage = BucketStage::ThirdParty { name, repo };
            Vec::new()
        }
        Err(e) => {
            app.push_toast(ToastKind::Error, e.to_string());
            Vec::new()
        }
    }
}

/// `d`/Delete on a bucket: refused with the installed apps (the CLI's rule), else asked.
fn ask_remove(d: &mut BucketsDialog) {
    let Some(b) = d.rows.get(d.cursor) else {
        return;
    };
    let name = b.row.name.clone();
    d.stage = match &b.installed {
        Ok(apps) if apps.is_empty() => BucketStage::Remove { name },
        Ok(apps) => BucketStage::InUse {
            name,
            apps: apps.clone(),
        },
        Err(e) => BucketStage::InUse {
            name,
            apps: vec![e.clone()],
        },
    };
}

/// Keys of the buckets dialog: `a` adds, `d`/Delete removes, `Esc` goes back or closes.
fn buckets_key(app: &mut App, mut d: BucketsDialog, key: KeyEvent) -> Vec<Cmd> {
    let stage = std::mem::replace(&mut d.stage, BucketStage::List);
    let mut cmds = Vec::new();
    match stage {
        BucketStage::List => match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'b') => return Vec::new(),
            KeyCode::Down | KeyCode::Char('j') => {
                d.cursor = (d.cursor + 1).min(d.rows.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => d.cursor = d.cursor.saturating_sub(1),
            KeyCode::Home => d.cursor = 0,
            KeyCode::End => d.cursor = d.rows.len().saturating_sub(1),
            KeyCode::Char('a') | KeyCode::Insert if !d.loading => d.stage = add_picker(&d),
            KeyCode::Char('d') | KeyCode::Delete if !d.loading => ask_remove(&mut d),
            _ => {}
        },
        BucketStage::Add { mut picker, names } => match picker.key(key) {
            PickOutcome::Pending => d.stage = BucketStage::Add { picker, names },
            PickOutcome::Cancelled => {}
            PickOutcome::Picked(chosen) => match chosen.first().and_then(|&i| names.get(i)) {
                Some(name) => {
                    let repo = crate::buckets::known_repo(name).unwrap_or_default();
                    cmds = try_add(app, &mut d, name, repo);
                }
                None => {
                    d.stage = BucketStage::Typing {
                        text: String::new(),
                    };
                }
            },
        },
        BucketStage::Typing { mut text } => match key.code {
            KeyCode::Esc => {}
            KeyCode::Enter => match crate::buckets::parse_repo(&text) {
                Ok(repo) => cmds = try_add(app, &mut d, &bucket_name_of(&repo), text.trim()),
                Err(_) => d.stage = BucketStage::Typing { text },
            },
            KeyCode::Backspace => {
                text.pop();
                d.stage = BucketStage::Typing { text };
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                text.push(c);
                d.stage = BucketStage::Typing { text };
            }
            _ => d.stage = BucketStage::Typing { text },
        },
        BucketStage::ThirdParty { name, repo } => match key.code {
            KeyCode::Enter => {
                let action = Action::BucketAdd {
                    name,
                    spec: repo,
                    confirmed: true,
                };
                cmds = enqueue(app, action);
            }
            KeyCode::Esc => {}
            _ => d.stage = BucketStage::ThirdParty { name, repo },
        },
        BucketStage::Remove { name } => match key.code {
            KeyCode::Enter if crate::buckets::is_default_bucket(&name) => {
                d.stage = BucketStage::RemoveDefault { name };
            }
            KeyCode::Enter => {
                let action = Action::BucketRemove {
                    name,
                    confirmed: false,
                };
                cmds = enqueue(app, action);
            }
            KeyCode::Esc => {}
            _ => d.stage = BucketStage::Remove { name },
        },
        BucketStage::RemoveDefault { name } => match key.code {
            KeyCode::Enter => {
                let action = Action::BucketRemove {
                    name,
                    confirmed: true,
                };
                cmds = enqueue(app, action);
            }
            KeyCode::Esc => {}
            _ => d.stage = BucketStage::RemoveDefault { name },
        },
        BucketStage::InUse { name, apps } => match key.code {
            KeyCode::Enter | KeyCode::Esc => {}
            _ => d.stage = BucketStage::InUse { name, apps },
        },
    }
    app.modal = Some(Modal::Buckets(Box::new(d)));
    cmds
}
