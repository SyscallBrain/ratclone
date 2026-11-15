//! Application state and event handling.
//!
//! Keys become [`Action`]s; anything that talks to rclone runs in a spawned task
//! whose result comes back as an [`AppEvent`]. Listings carry a request id so an
//! answer for a folder the user already left is dropped.

pub mod account;
pub mod cache;
mod ops;
pub mod overlay;
pub mod pane;
pub mod panel;
pub mod paste;
pub mod register;
pub mod settings;
pub mod sidebar;
pub mod space;
mod transfers;

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::future::BoxFuture;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::backend::{About, Backend, BackendError, Entry, RemoteInfo, StartError};
use crate::config::{Config, IconStyle};
use crate::input::keymap::{Action, KeyContext, KeyParser, Parsed};
use crate::location::{Location, Remote};
use crate::theme::Theme;
use crate::transfer::spec::{TransferKind, TransferProgress, TransferSpec};
use crate::transfer::store::TransferStore;
use cache::ListingCache;
use overlay::{InputPurpose, LineInput, Overlay};
use pane::{Listing, PaneState};
use register::Register;
use sidebar::SidebarState;

/// How long a status message stays visible.
pub const MESSAGE_TTL: Duration = Duration::from_secs(4);
/// How long an error stays: it is longer to read, and there is no other record of it.
pub const ERROR_TTL: Duration = Duration::from_secs(10);

/// Relaunches the rcd after the connection was lost.
/// The result says whether a fresh rcd had to be started (its jobs are all gone).
pub type ReconnectFn = Arc<dyn Fn() -> BoxFuture<'static, Result<bool, String>> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Pane,
}

/// Which side of the screen the focused pane is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// The views in the top bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Files,
    Space,
    Transfers,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Files, Tab::Space, Tab::Transfers];

    pub fn next(self) -> Tab {
        match self {
            Tab::Files => Tab::Space,
            Tab::Space => Tab::Transfers,
            Tab::Transfers => Tab::Files,
        }
    }

    pub fn prev(self) -> Tab {
        self.next().next()
    }
}

/// The full-screen transfers list opened with `T`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransfersView {
    pub cursor: usize,
    /// First row on screen (set while drawing, so the list scrolls only when needed).
    pub offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Success,
    Error,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub text: String,
    pub kind: MessageKind,
    pub at: Instant,
}

impl Message {
    /// Shown for long enough: time to clear it from the status line.
    pub fn expired(&self) -> bool {
        let ttl = match self.kind {
            MessageKind::Error => ERROR_TTL,
            MessageKind::Info | MessageKind::Success => MESSAGE_TTL,
        };
        self.at.elapsed() > ttl
    }
}

#[derive(Debug)]
pub enum AppEvent {
    Remotes(Result<Vec<RemoteInfo>, BackendError>),
    Listed {
        req: u64,
        loc: Location,
        result: Result<Vec<Entry>, BackendError>,
    },
    About {
        req: u64,
        remote: Remote,
        result: Result<About, BackendError>,
    },
    OpDone {
        what: String,
        result: Result<(), BackendError>,
        refresh: Vec<Location>,
        cursor: Option<String>,
    },
    TransferStarted {
        spec: TransferSpec,
        result: Result<Vec<u64>, StartError>,
    },
    /// rclone accepted one job of a transfer being launched (before `TransferStarted`).
    TransferJob {
        id: u64,
        jobid: u64,
    },
    /// The jobs rclone knows of a transfer left starting by an earlier run.
    Reconciled {
        id: u64,
        result: Result<Vec<u64>, BackendError>,
    },
    StopDone {
        id: u64,
        result: Result<(), BackendError>,
    },
    /// Which items of a move to resume have arrived already.
    ResumeChecked {
        old: u64,
        spec: TransferSpec,
        result: Result<Vec<bool>, BackendError>,
    },
    Progress {
        id: u64,
        result: Result<TransferProgress, BackendError>,
    },
    PasteChecked {
        register: Register,
        dst_dir: Location,
        result: Result<Vec<Entry>, BackendError>,
        /// Sent to the other pane rather than pasted from the register.
        direct: bool,
    },
    Reconnected(Result<bool, String>),
    Scan {
        id: u64,
        msg: crate::scan::walk::ScanMsg,
    },
    /// A scan read back from the cache (`None`: nothing usable there).
    ScanLoaded {
        root: Location,
        tree: Option<Box<crate::scan::tree::ScanTree>>,
    },
    ScanSaved(Result<(), String>),
    BwLimit(Result<String, BackendError>),
    /// An answer for the account wizard with this id.
    Wizard {
        id: u64,
        ev: account::WizardEvent,
    },
    /// `quiet`: cleaning up an unfinished add, not a removal the user asked for.
    AccountDeleted {
        name: String,
        result: Result<(), BackendError>,
        quiet: bool,
    },
    /// Which accounts are built on the one about to be removed.
    RemoveChecked {
        name: String,
        kind: String,
        result: Result<Vec<String>, BackendError>,
    },
    /// A browser login was told to stop (nothing more to do).
    JobStopped,
    Tick,
}

pub struct App {
    pub backend: Arc<dyn Backend>,
    pub focus: Focus,
    pub sidebar: SidebarState,
    /// The focused file pane.
    pub pane: PaneState,
    /// The second file pane, when two are open (the one without focus).
    pub other: Option<PaneState>,
    /// Where `pane` is drawn when two panes are open.
    pub side: Side,
    /// Where the right pane was when it was last closed (this session only).
    right_loc: Option<Location>,
    /// The info panel's state before the second pane hid it.
    panel_before_split: Option<bool>,
    pub store: TransferStore,
    pub progress: HashMap<u64, TransferProgress>,
    pub overlay: Option<Overlay>,
    /// Questions that came up while another dialog was open, shown one at a time
    /// once it closes (see [`App::ask`]).
    deferred: VecDeque<Overlay>,
    /// Pasted text is being fed to a text field: its characters are text only,
    /// never the command keys some fields take while empty.
    pasting: bool,
    pub tab: Tab,
    /// `user@host` shown in the top bar.
    pub host: String,
    /// The time zone dates are shown in: the system's, looked up once.
    pub tz: jiff::tz::TimeZone,
    /// The info panel is on (the `i` key); the terminal size may still hide it.
    pub panel_visible: bool,
    /// Total transfer speed over the last minute.
    pub speed: panel::SpeedHistory,
    /// Where `UiState` is saved; `None` keeps it in memory (tests, demo).
    pub ui_state_path: Option<std::path::PathBuf>,
    pub space: space::SpaceState,
    /// Finished (or cancelled) scans by root.
    pub scans: HashMap<Location, crate::scan::tree::ScanTree>,
    pub scan: Option<space::ActiveScan>,
    /// Saved scans being read from the cache.
    scans_loading: HashSet<Location>,
    /// Where scans are saved; `None` keeps them in memory (tests, demo).
    pub scan_cache_dir: Option<std::path::PathBuf>,
    /// Accounts the app changed after their last scan.
    /// When the app last changed each account (copies, moves, renames, deletes…).
    pub changed_since_scan: HashMap<Remote, Instant>,
    /// Rows the Space list showed last frame.
    pub space_viewport: usize,
    /// How far the help can scroll and its page height, as last drawn.
    pub help_extent: (usize, usize),
    pub(crate) scan_concurrency: Option<usize>,
    next_scan: u64,
    /// Where settings are saved; `None` keeps them in memory (tests, demo).
    pub config_path: Option<std::path::PathBuf>,
    /// Custom theme files (`<config dir>/themes`).
    pub themes_dir: std::path::PathBuf,
    /// The settings as saved in `config.toml`.
    pub settings: crate::config::SettingValues,
    /// Show the key hints on the bottom line.
    pub key_hints: bool,
    pub on_quit: crate::config::OnQuit,
    /// Where to go once the accounts are known (the "start in" setting).
    pub pending_start: Option<Location>,
    /// The bandwidth limit rclone confirmed.
    pub bwlimit_shown: Option<String>,
    /// Cursor of the Transfers tab (kept while other tabs are shown).
    pub transfers: TransfersView,
    pub message: Option<Message>,
    pub register: Option<Register>,
    pub keys: KeyParser,
    pub should_quit: bool,
    /// Stop the rcd when the app exits.
    pub shutdown_on_exit: bool,
    pub theme: Theme,
    pub icons: IconStyle,
    /// Animation frame, advanced on every tick.
    pub spinner: usize,
    /// Where the local disk opens.
    pub home: Location,
    tx: UnboundedSender<AppEvent>,
    rx: UnboundedReceiver<AppEvent>,
    in_flight: usize,
    next_req: u64,
    reconnect: Option<ReconnectFn>,
    reconnecting: bool,
    last_reconnect: Option<Instant>,
    /// Transfers whose progress request is in flight.
    polling: HashSet<u64>,
    /// The user chose "wait" in the quit dialog.
    pub quit_when_idle: bool,
    /// Pastes and transfer launches not yet registered in the store.
    launching: usize,
    /// Renames, deletes, new folders and stop requests still running.
    ops_in_flight: usize,
    /// The user chose "cancel all": stop transfers that start from now on.
    cancel_new: bool,
    /// Relaunched transfer id → the record it replaces once it has started.
    resuming: HashMap<u64, u64>,
    /// Transfers being launched by this run (their record is `Starting`).
    launching_ids: HashSet<u64>,
    /// Starting records left by an earlier run, being checked with rclone.
    reconciling: HashSet<u64>,
    /// Transfers whose stop request has not answered yet.
    stopping: HashSet<u64>,
    /// The user chose to quit while transfers were being launched or file operations
    /// were running: quit as soon as they are over.
    pub quit_when_launched: bool,
    /// With `quit_when_launched`: stop rclone on the way out ("cancel and quit").
    quit_shutdown: bool,
    /// Transfers whose status error was already shown.
    progress_errors: HashSet<u64>,
    /// Folders listed recently, shown at once when visited again.
    listings: ListingCache,
    /// Inside an SSH session (OAuth logins then use a pasted token).
    pub over_ssh: bool,
    /// Select this account once the list of accounts arrives.
    pub pending_select: Option<Remote>,
    /// An add cancelled while a request was in flight: delete it again when that answers.
    /// Unfinished adds of closed wizards (by wizard id): deleted again when their
    /// last request answers.
    orphans: HashMap<u64, String>,
    /// Unfinished adds to remove once rclone is reachable again.
    pending_deletes: Vec<String>,
    /// Names of unfinished adds whose removal is not done yet.
    cleaning: HashSet<String>,
    next_wizard: u64,
    /// The login poll in flight: (wizard id, job id). Only its own answer frees it.
    polling_job: Option<(u64, u64)>,
    /// When the wizard (by id) last asked about its login.
    last_job_poll: Option<(u64, Instant)>,
    /// Text for the terminal's clipboard, written (OSC 52) by the main loop.
    pub clipboard_out: Option<String>,
}

fn load_theme(name: Option<&str>) -> Theme {
    let name = name.unwrap_or("tokyo-night");
    let builtin = crate::theme::builtin::by_name(name);
    if builtin.name == name {
        builtin
    } else {
        crate::theme::load::load_custom(name).unwrap_or(builtin)
    }
}

impl App {
    pub fn new(backend: Arc<dyn Backend>, store: TransferStore, config: &Config, start: Location) -> App {
        let (tx, rx) = unbounded_channel();
        let mut app = App {
            backend,
            focus: Focus::Pane,
            sidebar: SidebarState::default(),
            pane: PaneState::new(start.clone(), config.show_hidden),
            other: None,
            side: Side::Left,
            right_loc: None,
            panel_before_split: None,
            store,
            progress: HashMap::new(),
            overlay: None,
            deferred: VecDeque::new(),
            pasting: false,
            tab: Tab::Files,
            host: String::new(),
            tz: jiff::tz::TimeZone::system(),
            panel_visible: true,
            speed: panel::SpeedHistory::default(),
            ui_state_path: None,
            space: space::SpaceState::default(),
            scans: HashMap::new(),
            scan: None,
            scans_loading: HashSet::new(),
            scan_cache_dir: None,
            changed_since_scan: HashMap::new(),
            space_viewport: 1,
            help_extent: (usize::MAX, 10),
            scan_concurrency: config.scan_concurrency,
            next_scan: 0,
            config_path: None,
            themes_dir: crate::paths::config_file()
                .parent()
                .map(|p| p.join("themes"))
                .unwrap_or_default(),
            settings: crate::config::SettingValues::from_config(config),
            key_hints: true,
            on_quit: crate::config::OnQuit::Ask,
            pending_start: None,
            bwlimit_shown: None,
            transfers: TransfersView::default(),
            message: None,
            register: None,
            keys: KeyParser::new(),
            should_quit: false,
            shutdown_on_exit: false,
            theme: load_theme(config.theme.as_deref()),
            icons: config.icons.unwrap_or_default(),
            spinner: 0,
            home: start.clone(),
            tx,
            rx,
            in_flight: 0,
            next_req: 0,
            reconnect: None,
            reconnecting: false,
            last_reconnect: None,
            polling: HashSet::new(),
            quit_when_idle: false,
            launching: 0,
            ops_in_flight: 0,
            resuming: HashMap::new(),
            launching_ids: HashSet::new(),
            reconciling: HashSet::new(),
            stopping: HashSet::new(),
            quit_when_launched: false,
            quit_shutdown: false,
            progress_errors: HashSet::new(),
            cancel_new: false,
            listings: ListingCache::default(),
            over_ssh: crate::paths::over_ssh(),
            pending_select: None,
            orphans: HashMap::new(),
            pending_deletes: Vec::new(),
            cleaning: HashSet::new(),
            next_wizard: 0,
            polling_job: None,
            last_job_poll: None,
            clipboard_out: None,
        };
        let settings = app.settings.clone();
        app.apply_settings(&settings);
        let backend = app.backend.clone();
        app.spawn(async move { AppEvent::Remotes(backend.remotes().await) });
        app.request_listing(start, None);
        app
    }

    pub fn set_reconnect(&mut self, f: ReconnectFn) {
        self.reconnect = Some(f);
    }

    /// Run `fut` in the background; its event arrives through [`App::next_event`].
    pub fn spawn<F>(&mut self, fut: F)
    where
        F: Future<Output = AppEvent> + Send + 'static,
    {
        self.in_flight += 1;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(fut.await);
        });
    }

    /// Like [`App::spawn`] for an operation that ends with `OpDone`; quitting waits for it.
    pub fn spawn_op<F>(&mut self, fut: F)
    where
        F: Future<Output = AppEvent> + Send + 'static,
    {
        self.ops_in_flight += 1;
        self.spawn(fut);
    }

    pub async fn next_event(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }

    /// An event that has already arrived, without waiting.
    pub fn try_next_event(&mut self) -> Option<AppEvent> {
        self.rx.try_recv().ok()
    }

    /// Process events until no background task is left (tests and demo only).
    pub async fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.in_flight > 0 && Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(50), self.rx.recv()).await {
                Ok(Some(ev)) => self.handle_event(ev),
                Ok(None) => break,
                Err(_) => {}
            }
        }
    }

    /// On the way out: let the file operations and stop requests already sent be
    /// answered, for two seconds at most. Whatever is only being read (a listing, a
    /// scan, a usage figure) is dropped instead: nobody is left to look at it, and
    /// waiting for it would hold the shell back after every quit made mid-scan.
    pub async fn finish_ops(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.ops_in_flight > 0 && Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(50), self.rx.recv()).await {
                Ok(Some(ev)) => self.handle_event(ev),
                Ok(None) => break,
                Err(_) => {}
            }
        }
    }

    /// Something on screen moves: a spinner or a progress bar.
    pub fn is_animating(&self) -> bool {
        std::iter::once(&self.pane)
            .chain(self.other.as_ref())
            .any(|p| p.listing == Listing::Loading || p.refreshing)
            || self.scan.is_some()
            || self.store.running().next().is_some()
            || matches!(
                &self.overlay,
                Some(Overlay::Account(w)) if matches!(
                    w.step,
                    account::Step::Loading | account::Step::Sending | account::Step::WaitingBrowser { .. }
                )
            )
            || self.sidebar.remotes.iter().any(|r| {
                !self.sidebar.about.contains_key(&r.remote)
                    && !self.sidebar.about_failed.contains_key(&r.remote)
            })
    }

    pub fn info(&mut self, text: impl Into<String>, kind: MessageKind) {
        self.message = Some(Message {
            text: text.into(),
            kind,
            at: Instant::now(),
        });
    }

    /// Show a question that the user did not just ask for (it answers something
    /// started earlier, like a paste whose destination had to be listed first).
    /// With another dialog open it waits its turn: put over it, the keys being
    /// typed there would answer this one instead, and what was open would be lost.
    pub(crate) fn ask(&mut self, question: Overlay) {
        if self.overlay.is_some() {
            self.deferred.push_back(question);
        } else {
            self.overlay = Some(question);
        }
    }

    /// The next waiting question, once no dialog is open.
    fn show_deferred(&mut self) {
        if self.overlay.is_none() {
            self.overlay = self.deferred.pop_front();
        }
    }

    /// Leave because the process was told to (a closed terminal, a dropped SSH
    /// session, `kill`): transfers keep running in rclone, as with "quit and keep
    /// them running"; with none, rclone is stopped on the way out.
    pub fn quit_on_signal(&mut self) {
        self.should_quit = true;
        self.shutdown_on_exit = self.active_transfers() == 0;
    }

    // ---- navigation -------------------------------------------------------

    fn request_listing(&mut self, loc: Location, cursor: Option<String>) {
        self.next_req += 1;
        let req = self.next_req;
        self.pane.req = req;
        self.pane.refreshing = false;
        self.pane.pending_cursor = cursor;
        if self.pane.loc != loc {
            // Never let rows of the previous folder be acted on under the new path.
            self.pane.clear_contents();
            self.pane.loc = loc.clone();
            self.pane.listing = Listing::Loading;
            // Seen recently: show those rows now; the fresh listing replaces them
            // and, until then, nothing can be done to them but look and move.
            if let Some(cached) = self.listings.get(&loc) {
                let cursor = self.pane.pending_cursor.take();
                self.pane.set_sorted_entries(cached, cursor.as_deref());
                self.pane.listing = Listing::Ready;
                self.pane.refreshing = true;
            }
        } else {
            self.pane.listing = Listing::Loading;
        }
        let backend = self.backend.clone();
        self.spawn(async move {
            let mut result = backend.list(&loc).await;
            if let Ok(entries) = &mut result {
                pane::sort_by_name(entries);
            }
            AppEvent::Listed { req, loc, result }
        });
    }

    /// Go to `loc`, remembering where we were for `-`.
    pub fn navigate(&mut self, loc: Location, cursor: Option<String>) {
        // The user went somewhere: the start-up jump no longer applies.
        self.pending_start = None;
        let here = self.pane.current().map(|e| e.name.clone()).unwrap_or_default();
        self.pane.history.push((self.pane.loc.clone(), here));
        if self.pane.history.len() > 100 {
            self.pane.history.remove(0);
        }
        self.sidebar.select(&loc.remote);
        self.request_listing(loc, cursor);
    }

    /// Re-list the current folder, keeping the cursor where it is.
    pub fn refresh_pane(&mut self) {
        let cursor = self.pane.current().map(|e| e.name.clone());
        let loc = self.pane.loc.clone();
        self.request_listing(loc, cursor);
    }

    /// Ask for `remote`'s usage again: something changed what it holds, or the
    /// user asked. An answer still on its way predates that and is dropped. The
    /// figure on screen stays until the new one arrives.
    pub(crate) fn refresh_about(&mut self, remote: &Remote) {
        if !self.sidebar.remotes.iter().any(|r| &r.remote == remote) {
            return;
        }
        self.sidebar.about_pending.remove(remote);
        self.request_about(remote.clone());
    }

    /// Ask for `remote`'s usage, unless that question is already on its way.
    fn request_about(&mut self, remote: Remote) {
        if self.sidebar.about_pending.contains_key(&remote) {
            return;
        }
        self.next_req += 1;
        let req = self.next_req;
        self.sidebar.about_pending.insert(remote.clone(), req);
        let backend = self.backend.clone();
        self.spawn(async move {
            let result = backend.about(&remote).await;
            AppEvent::About { req, remote, result }
        });
    }

    /// Show the account `delta` places away in the list, wrapping round. Files
    /// counts from the account the pane shows; Space from the sidebar cursor.
    fn cycle_account(&mut self, delta: isize) {
        let len = self.sidebar.remotes.len();
        if len == 0 {
            return;
        }
        let from = match self.tab {
            Tab::Files => self
                .sidebar
                .remotes
                .iter()
                .position(|r| r.remote == self.pane.loc.remote)
                .unwrap_or(self.sidebar.cursor),
            _ => self.sidebar.cursor,
        };
        let to = (from as isize + delta).rem_euclid(len as isize) as usize;
        self.sidebar.cursor = to;
        let remote = self.sidebar.remotes[to].remote.clone();
        if self.sidebar.needs_about(&remote, Instant::now()) {
            self.request_about(remote.clone());
        }
        match self.tab {
            Tab::Files => {
                let loc = self.root_of(&remote);
                if loc != self.pane.loc {
                    self.navigate(loc, None);
                }
            }
            Tab::Space => self.sync_space(),
            Tab::Transfers => {}
        }
    }

    fn root_of(&self, remote: &Remote) -> Location {
        match remote {
            Remote::Local => self.home.clone(),
            other => Location::new(other.clone(), ""),
        }
    }

    // ---- two panes --------------------------------------------------------

    pub fn is_split(&self) -> bool {
        self.other.is_some()
    }

    pub fn left_pane(&self) -> &PaneState {
        match (&self.other, self.side) {
            (Some(other), Side::Right) => other,
            _ => &self.pane,
        }
    }

    pub fn right_pane(&self) -> Option<&PaneState> {
        match self.side {
            Side::Right => self.other.as_ref().map(|_| &self.pane),
            Side::Left => self.other.as_ref(),
        }
    }

    /// Open the second pane beside the first, or close it again.
    pub fn toggle_split(&mut self) {
        match self.other.take() {
            None => {
                let loc = self.right_loc.clone().unwrap_or_else(|| {
                    let here = &self.pane.loc.remote;
                    match self.sidebar.remotes.iter().find(|r| &r.remote != here) {
                        Some(r) => self.root_of(&r.remote.clone()),
                        None => self.pane.loc.clone(),
                    }
                });
                let new = PaneState::new(loc.clone(), self.settings.show_hidden);
                self.other = Some(std::mem::replace(&mut self.pane, new));
                // The user is working: the start-up jump no longer applies.
                self.pending_start = None;
                self.side = Side::Right;
                self.focus = Focus::Pane;
                if self.settings.split_info == crate::config::SplitInfo::Hidden {
                    self.panel_before_split = Some(self.panel_visible);
                    self.panel_visible = false;
                }
                self.sidebar.select(&loc.remote);
                // A fresh pane: force a listing (its location already matches).
                self.request_listing(loc, None);
            }
            Some(other) => {
                if self.side == Side::Right {
                    let right = std::mem::replace(&mut self.pane, other);
                    self.right_loc = Some(right.loc);
                } else {
                    self.right_loc = Some(other.loc);
                }
                self.side = Side::Left;
                if let Some(v) = self.panel_before_split.take() {
                    self.panel_visible = v;
                }
                self.sidebar.select(&self.pane.loc.remote.clone());
            }
        }
    }

    /// Both file panes, focused first.
    pub(crate) fn panes_mut(&mut self) -> impl Iterator<Item = &mut PaneState> {
        std::iter::once(&mut self.pane).chain(self.other.as_mut())
    }

    /// Give the focus to the other pane.
    fn swap_panes(&mut self) {
        if let Some(other) = &mut self.other {
            std::mem::swap(&mut self.pane, other);
            self.side = match self.side {
                Side::Left => Side::Right,
                Side::Right => Side::Left,
            };
            self.sidebar.select(&self.pane.loc.remote.clone());
        }
    }

    /// Run `f` with the unfocused pane standing in as `self.pane`.
    /// `f` may only touch `self.pane`, the listing cache and spawned work, never
    /// the focus or the sidebar.
    pub(crate) fn with_other<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> Option<R> {
        let mut other = self.other.take()?;
        std::mem::swap(&mut self.pane, &mut other);
        self.other = Some(other);
        let r = f(self);
        let other = self.other.as_mut().expect("with_other keeps the pane");
        std::mem::swap(&mut self.pane, other);
        Some(r)
    }

    /// Re-list each pane showing one of `dirs` or a folder inside one of them
    /// (which a move may have taken away), keeping its cursor.
    pub(crate) fn refresh_showing(&mut self, dirs: &[&Location]) {
        if dirs.iter().any(|d| self.pane.loc.is_within(d)) {
            self.refresh_pane();
        }
        if self
            .other
            .as_ref()
            .is_some_and(|o| dirs.iter().any(|d| o.loc.is_within(d)))
        {
            self.refresh_other();
        }
    }

    /// Re-list the unfocused pane's folder, keeping its cursor.
    pub(crate) fn refresh_other(&mut self) {
        self.with_other(|a| a.refresh_pane());
    }

    // ---- input ------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.dispatch_key(key);
        self.show_deferred();
    }

    /// Whether a text field is being edited: typed characters are text, not commands.
    pub fn editing_text(&self) -> bool {
        match &self.overlay {
            Some(Overlay::Input { .. }) => true,
            Some(Overlay::Settings(dlg)) => dlg.input.is_some(),
            Some(Overlay::Account(w)) => w.editing_text(),
            _ => false,
        }
    }

    /// Text pasted into the terminal. It goes into the text field being edited, as
    /// one line; anywhere else it is dropped, since its characters would be obeyed
    /// as keys: a `D` and, further on, a `y` would delete the file under the cursor.
    pub fn handle_paste(&mut self, text: &str) {
        if !self.editing_text() {
            return self.info("Pasted text ignored: no text field is open", MessageKind::Info);
        }
        self.pasting = true;
        for c in text.chars().filter(|c| !c.is_control()) {
            if !self.editing_text() {
                break;
            }
            self.dispatch_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        self.pasting = false;
        self.show_deferred();
    }

    fn dispatch_key(&mut self, key: KeyEvent) {
        if self.overlay.is_some() {
            self.handle_overlay_key(key);
            return;
        }
        let ctx = match (self.focus, self.tab) {
            (Focus::Sidebar, _) => KeyContext::Sidebar,
            (Focus::Pane, Tab::Files) => KeyContext::Pane,
            (Focus::Pane, Tab::Space) => KeyContext::Space,
            (Focus::Pane, Tab::Transfers) => KeyContext::Transfers,
        };
        if let Parsed::Action(action, count) = self.keys.feed(key, ctx) {
            match ctx {
                KeyContext::Sidebar => self.sidebar_action(action, count),
                KeyContext::Pane => self.pane_action(action, count),
                KeyContext::Space => self.space_action(action, count),
                KeyContext::Transfers => self.transfers_action(action, count),
            }
        }
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) {
        match self.overlay {
            Some(Overlay::Help { .. }) => self.help_key(key),
            Some(Overlay::Settings(_)) => self.settings_key(key),
            Some(Overlay::Account(_)) => self.account_key(key),
            Some(Overlay::Input { .. }) => self.input_key(key),
            Some(Overlay::Confirm { .. }) => self.confirm_key(key),
            Some(Overlay::Conflict { .. }) => self.conflict_key(key),
            Some(Overlay::Quit { .. }) => self.quit_key(key),
            None => {}
        }
    }

    /// Keys of the help: move through it, or close it (Esc, q, ?, Enter).
    fn help_key(&mut self, key: KeyEvent) {
        let Some(Overlay::Help { scroll }) = &mut self.overlay else {
            return;
        };
        let (max, page) = self.help_extent;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half = (page / 2).max(1);
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::F(1) => {
                self.overlay = None
            }
            KeyCode::Char('d') if ctrl => *scroll = (*scroll + half).min(max),
            KeyCode::Char('u') if ctrl => *scroll = scroll.saturating_sub(half),
            KeyCode::Char('j') | KeyCode::Down => *scroll = (*scroll + 1).min(max),
            KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => *scroll = (*scroll + page).min(max),
            KeyCode::PageUp => *scroll = scroll.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => *scroll = 0,
            KeyCode::Char('G') | KeyCode::End => *scroll = max,
            _ => {}
        }
    }

    fn common_action(&mut self, action: &Action) -> bool {
        match action {
            Action::Help => self.overlay = Some(Overlay::Help { scroll: 0 }),
            Action::Quit => self.request_quit(),
            Action::Transfers => self.set_tab(Tab::Transfers),
            Action::Space => self.set_tab(Tab::Space),
            Action::Settings => self.open_settings(),
            Action::NextTab => self.set_tab(self.tab.next()),
            Action::PrevTab => self.set_tab(self.tab.prev()),
            Action::GoTab(tab) => self.set_tab(*tab),
            // Session only: the saved default lives in the settings dialog.
            Action::TogglePanel => self.panel_visible = !self.panel_visible,
            _ => return false,
        }
        true
    }

    /// Show `tab`; the focus moves to its main area.
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.focus = Focus::Pane;
        if tab == Tab::Space {
            self.sync_space();
        }
    }

    fn sidebar_action(&mut self, action: Action, count: u32) {
        if self.common_action(&action) {
            return;
        }
        let n = count as isize;
        match action {
            Action::Down => self.sidebar.move_by(n),
            Action::Up => self.sidebar.move_by(-n),
            Action::Top => self.sidebar.cursor = 0,
            Action::Bottom => self.sidebar.move_by(isize::MAX / 2),
            Action::ToggleFocus => {
                self.focus = Focus::Pane;
                if self.tab == Tab::Files && self.side == Side::Right {
                    self.swap_panes();
                }
            }
            Action::FocusBack => {
                self.focus = Focus::Pane;
                if self.tab == Tab::Files && self.is_split() && self.side == Side::Left {
                    self.swap_panes();
                }
            }
            Action::Escape => self.focus = Focus::Pane,
            Action::Open if self.tab == Tab::Space => self.focus = Focus::Pane,
            Action::Open => {
                if let Some(r) = self.sidebar.current().map(|r| r.remote.clone()) {
                    let loc = self.root_of(&r);
                    self.set_tab(Tab::Files);
                    if loc != self.pane.loc {
                        self.navigate(loc, None);
                    }
                }
            }
            Action::AddAccount => self.open_add_account(),
            Action::EditAccount => self.open_edit_account(),
            Action::ReauthAccount => self.open_reauth(),
            Action::Delete => self.ask_remove_account(),
            Action::Refresh => {
                if let Some(r) = self.sidebar.current().map(|r| r.remote.clone()) {
                    self.sidebar.about_failed.remove(&r);
                    self.request_about(r);
                }
            }
            _ => {}
        }
        if let Some(r) = self.sidebar.current().map(|r| r.remote.clone())
            && self.sidebar.needs_about(&r, Instant::now())
        {
            self.request_about(r);
        }
        if self.tab == Tab::Space {
            self.sync_space();
        }
    }

    fn pane_action(&mut self, action: Action, count: u32) {
        if self.common_action(&action) {
            return;
        }
        let acts_on_selection = matches!(
            action,
            Action::Yank
                | Action::Cut
                | Action::Paste
                | Action::Delete
                | Action::Rename
                | Action::Mkdir
                | Action::ToggleMark
                | Action::Visual
                | Action::SendCopy
                | Action::SendMove
        );
        if acts_on_selection && (self.pane.listing != Listing::Ready || self.pane.refreshing) {
            return self.info("Wait for the folder to load", MessageKind::Info);
        }
        let n = count as isize;
        let half = (self.pane.viewport_height / 2).max(1) as isize;
        match action {
            Action::Down => self.pane.move_by(n),
            Action::Up => self.pane.move_by(-n),
            Action::Top => self.pane.top(),
            Action::Bottom => self.pane.bottom(),
            Action::HalfDown => self.pane.move_by(half * n),
            Action::HalfUp => self.pane.move_by(-half * n),
            Action::ToggleFocus if self.is_split() && self.side == Side::Left => self.swap_panes(),
            Action::ToggleFocus => self.focus = Focus::Sidebar,
            Action::FocusBack if self.is_split() && self.side == Side::Right => self.swap_panes(),
            Action::FocusBack => self.focus = Focus::Sidebar,
            Action::ToggleSplit => self.toggle_split(),
            Action::NextAccount => self.cycle_account(n),
            Action::PrevAccount => self.cycle_account(-n),
            Action::Open => {
                if let Some(e) = self.pane.current().filter(|e| e.is_dir) {
                    let loc = self.pane.loc.join(&e.name);
                    self.navigate(loc, None);
                }
            }
            Action::Parent => {
                if let Some(parent) = self.pane.loc.parent() {
                    let child = self.pane.loc.name().to_string();
                    self.navigate(parent, Some(child));
                }
            }
            Action::Back => {
                if let Some((loc, name)) = self.pane.history.pop() {
                    self.sidebar.select(&loc.remote);
                    self.request_listing(loc, Some(name));
                }
            }
            // Everything on screen about this folder: its rows and its account's usage.
            Action::Refresh => {
                self.refresh_pane();
                let remote = self.pane.loc.remote.clone();
                // Asked for: an account whose usage could not be read is tried again.
                self.sidebar.about_failed.remove(&remote);
                self.refresh_about(&remote);
            }
            Action::Filter => {
                let input = LineInput::new(&self.pane.filter, usize::MAX);
                self.overlay = Some(Overlay::Input {
                    title: "Filter".into(),
                    input,
                    purpose: InputPurpose::Filter,
                });
            }
            Action::NextMatch => self.pane.next_match(true),
            Action::PrevMatch => self.pane.next_match(false),
            Action::Sort(key) => self.pane.set_sort(key),
            Action::ReverseSort => self.pane.toggle_reverse(),
            Action::ToggleHidden => self.pane.toggle_hidden(),
            Action::ToggleMark => self.pane.toggle_mark(),
            Action::Visual => {
                self.pane.visual_anchor = match self.pane.visual_anchor {
                    Some(_) => None,
                    None => Some(self.pane.cursor),
                };
            }
            Action::Yank => self.yank(TransferKind::Copy),
            Action::Cut => self.yank(TransferKind::Move),
            Action::Paste => self.paste(),
            Action::SendCopy => self.send_to_other(TransferKind::Copy),
            Action::SendMove => self.send_to_other(TransferKind::Move),
            Action::Delete => self.start_delete(),
            Action::Rename => self.start_rename(),
            Action::Mkdir => self.start_mkdir(),
            // Each Esc undoes one layer: selection, then filter, then clipboard.
            Action::Escape => {
                if self.quit_when_idle || self.quit_when_launched {
                    self.quit_when_idle = false;
                    self.quit_when_launched = false;
                    self.info("Staying open", MessageKind::Info);
                } else if !self.pane.marks.is_empty() || self.pane.visual_anchor.is_some() {
                    self.pane.clear_selection();
                } else if !self.pane.filter.is_empty() {
                    self.pane.set_filter("");
                } else if self.register.take().is_some() {
                    self.info("Clipboard cleared", MessageKind::Info);
                }
            }
            _ => {}
        }
    }

    // ---- events -----------------------------------------------------------

    pub fn handle_event(&mut self, ev: AppEvent) {
        // A scan counts once for all its events (released by `Finished`).
        // A launch counts once too, for all its `TransferJob`s (released by `TransferStarted`).
        if !matches!(
            ev,
            AppEvent::Tick | AppEvent::Scan { .. } | AppEvent::TransferJob { .. }
        ) {
            self.in_flight = self.in_flight.saturating_sub(1);
        }
        self.dispatch(ev);
        // Any answer may have been the last operation a waiting quit needed.
        self.check_quit_when_launched();
        self.show_deferred();
    }

    fn dispatch(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Remotes(Ok(remotes)) => {
                self.sidebar.remotes = remotes;
                if let Some(target) = self.pending_start.take() {
                    let names: Vec<String> = self
                        .sidebar
                        .remotes
                        .iter()
                        .filter_map(|r| match &r.remote {
                            Remote::Named(n) => Some(n.clone()),
                            Remote::Local => None,
                        })
                        .collect();
                    if let Some(loc) = settings::resolve_start("last", Some(&target), &names)
                        && loc != self.pane.loc
                    {
                        self.navigate(loc, None);
                    }
                }
                self.sidebar.select(&self.pane.loc.remote.clone());
                if let Some(r) = self.pending_select.take() {
                    self.sidebar.select(&r);
                }
                let all: Vec<Remote> = self.sidebar.remotes.iter().map(|r| r.remote.clone()).collect();
                for r in all {
                    self.request_about(r);
                }
            }
            AppEvent::Remotes(Err(e)) => {
                // Too late to jump anywhere once the user has had the app for a while.
                self.pending_start = None;
                self.pending_select = None;
                self.backend_error(e)
            }
            AppEvent::Listed { req, loc, result } => {
                // Even an answer for a folder already left is worth remembering.
                let entries = match result {
                    Ok(entries) => {
                        let entries = Arc::new(entries);
                        self.listings.insert(loc, entries.clone());
                        Ok(entries)
                    }
                    Err(e) => {
                        self.listings.remove(&loc);
                        Err(e)
                    }
                };
                // Each pane only takes the answer it is waiting for.
                let lost = if req == self.pane.req {
                    self.apply_listing(entries)
                } else if self.other.as_ref().is_some_and(|o| o.req == req) {
                    self.with_other(|a| a.apply_listing(entries)) == Some(true)
                } else {
                    false
                };
                if lost {
                    self.start_reconnect();
                }
            }
            AppEvent::About { req, remote, result } => {
                // Asked again since (the account changed): wait for that answer.
                if self.sidebar.about_pending.get(&remote) != Some(&req) {
                    return;
                }
                self.sidebar.about_pending.remove(&remote);
                match result {
                    Ok(a) => {
                        self.sidebar.about_failed.remove(&remote);
                        self.sidebar.about.insert(remote, (a, Instant::now()));
                    }
                    Err(e) => {
                        self.sidebar.about_failed.insert(remote, e.to_string());
                    }
                }
            }
            AppEvent::Tick => {
                self.spinner = self.spinner.wrapping_add(1);
                if self.message.as_ref().is_some_and(Message::expired) {
                    self.message = None;
                }
                let speed: f64 = self
                    .store
                    .running()
                    .filter_map(|r| self.progress.get(&r.spec.id))
                    .map(|p| p.speed)
                    .sum();
                self.speed.push(speed);
                if self.scan.is_some() && self.tab == Tab::Space {
                    self.recompute_space_rows(None);
                }
                self.poll_transfers();
                self.poll_login();
            }
            AppEvent::OpDone {
                what,
                result,
                refresh,
                cursor,
            } => {
                self.ops_in_flight = self.ops_in_flight.saturating_sub(1);
                self.check_quit_when_idle();
                for loc in &refresh {
                    self.listings.invalidate_within(loc);
                }
                match result {
                    Ok(()) => {
                        for loc in &refresh {
                            self.note_change(&loc.remote);
                            self.refresh_about(&loc.remote);
                        }
                        self.info(what, MessageKind::Success)
                    }
                    Err(e) => self.backend_error(e),
                }
                if refresh.contains(&self.pane.loc) {
                    let loc = self.pane.loc.clone();
                    let cursor = cursor.or_else(|| self.pane.current().map(|e| e.name.clone()));
                    self.request_listing(loc, cursor);
                }
                // The other pane may sit inside a folder that was just renamed or deleted.
                if self
                    .other
                    .as_ref()
                    .is_some_and(|o| refresh.iter().any(|d| o.loc.is_within(d)))
                {
                    self.refresh_other();
                }
            }
            AppEvent::TransferStarted { spec, result } => {
                if result.as_ref().is_ok_and(|ids| !ids.is_empty()) {
                    self.note_change(&spec.dst_dir.remote);
                    if spec.kind == TransferKind::Move {
                        self.note_change(&spec.src_dir.remote);
                    }
                }
                self.transfer_started(spec, result)
            }
            AppEvent::TransferJob { id, jobid } => self.transfer_job(id, jobid),
            AppEvent::Reconciled { id, result } => self.reconciled(id, result),
            AppEvent::StopDone { id, result } => self.stop_done(id, result),
            AppEvent::ResumeChecked { old, spec, result } => self.resume_checked(old, spec, result),
            AppEvent::Progress { id, result } => self.on_progress(id, result),
            AppEvent::PasteChecked {
                register,
                dst_dir,
                result,
                direct,
            } => {
                self.launching = self.launching.saturating_sub(1);
                match result {
                    Ok(entries) => self.paste_checked(register, dst_dir, entries, direct),
                    Err(e) => self.backend_error(e),
                }
                self.check_quit_when_launched();
                self.check_quit_when_idle();
            }
            AppEvent::Reconnected(result) => self.reconnected(result),
            AppEvent::Scan { id, msg } => self.on_scan(id, msg),
            AppEvent::ScanLoaded { root, tree } => {
                self.scans_loading.remove(&root);
                if let Some(tree) = tree {
                    self.scans.entry(root).or_insert(*tree);
                    self.recompute_space_rows(None);
                }
            }
            AppEvent::ScanSaved(Err(e)) => {
                self.info(format!("Could not save the scan: {e}"), MessageKind::Error)
            }
            AppEvent::ScanSaved(Ok(())) | AppEvent::JobStopped => {}
            AppEvent::BwLimit(Ok(rate)) => self.bwlimit_shown = (rate != "off").then_some(rate),
            AppEvent::BwLimit(Err(e)) => {
                self.info(format!("Bandwidth limit not applied: {e}"), MessageKind::Error)
            }
            ev @ (AppEvent::Wizard { .. }
            | AppEvent::AccountDeleted { .. }
            | AppEvent::RemoveChecked { .. }) => self.account_event(ev),
        }
    }

    /// Put a listing answer into `self.pane`.
    /// Returns whether the connection was lost (the reconnect is started here).
    fn apply_listing(&mut self, entries: Result<Arc<Vec<Entry>>, BackendError>) -> bool {
        self.pane.refreshing = false;
        match entries {
            Ok(entries) => {
                let cursor = self.pane.pending_cursor.take();
                self.pane.set_sorted_entries(entries, cursor.as_deref());
                self.pane.listing = Listing::Ready;
                false
            }
            Err(e) => {
                self.pane.set_entries(Vec::new(), None);
                self.pane.listing = Listing::Failed(e.to_string());
                e == BackendError::Disconnected
            }
        }
    }

    /// A transfer changed (or is about to change) these folders: cached rows are stale.
    pub(crate) fn forget_listings(&mut self, spec: &TransferSpec) {
        self.listings.invalidate_within(&spec.dst_dir);
        if spec.kind == TransferKind::Move {
            for item in &spec.items {
                self.listings.invalidate_within(&spec.src_dir.join(&item.name));
            }
            self.listings.remove(&spec.src_dir);
        }
    }

    fn backend_error(&mut self, e: BackendError) {
        let disconnected = e == BackendError::Disconnected;
        self.info(e.to_string(), MessageKind::Error);
        if disconnected {
            self.start_reconnect();
        }
    }
}
