//! In-memory backend for tests and `--demo`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::{
    About, Backend, BackendError, ConfigParams, ConfigStep, Entry, Provider, ProviderOption, RemoteInfo,
    StartError,
};
use crate::location::{Location, Remote};
use crate::transfer::spec::{FileProgress, JobsState, TransferKind, TransferProgress, TransferSpec};

pub struct FakeBackend {
    state: Mutex<State>,
}

#[derive(Clone, Debug)]
struct Node {
    is_dir: bool,
    size: u64,
    mod_time: jiff::Timestamp,
    children: Vec<(String, Node)>,
}

impl Node {
    fn dir() -> Node {
        Node {
            is_dir: true,
            size: 0,
            mod_time: demo_time(),
            children: Vec::new(),
        }
    }
    fn file(size: u64) -> Node {
        Node {
            is_dir: false,
            size,
            mod_time: demo_time(),
            children: Vec::new(),
        }
    }
    fn total_size(&self) -> u64 {
        if self.is_dir {
            self.children.iter().map(|(_, c)| c.total_size()).sum()
        } else {
            self.size
        }
    }
    fn get(&self, path: &str) -> Option<&Node> {
        if path.is_empty() {
            return Some(self);
        }
        let (head, rest) = path.split_once('/').unwrap_or((path, ""));
        self.children
            .iter()
            .find(|(n, _)| n == head)
            .and_then(|(_, c)| c.get(rest))
    }
    fn get_mut(&mut self, path: &str) -> Option<&mut Node> {
        if path.is_empty() {
            return Some(self);
        }
        let (head, rest) = path.split_once('/').unwrap_or((path, ""));
        self.children
            .iter_mut()
            .find(|(n, _)| n == head)
            .and_then(|(_, c)| c.get_mut(rest))
    }
    /// Insert `node` at `path`, creating parent directories; replaces an existing entry.
    fn insert(&mut self, path: &str, node: Node) {
        let (head, rest) = path.split_once('/').unwrap_or((path, ""));
        if rest.is_empty() {
            if let Some(slot) = self.children.iter_mut().find(|(n, _)| n == head) {
                slot.1 = node;
            } else {
                self.children.push((head.to_string(), node));
            }
            return;
        }
        if !self.children.iter().any(|(n, _)| n == head) {
            self.children.push((head.to_string(), Node::dir()));
        }
        let child = &mut self.children.iter_mut().find(|(n, _)| n == head).unwrap().1;
        child.insert(rest, node);
    }
    fn remove(&mut self, path: &str) -> Option<Node> {
        let (parent, name) = match path.rsplit_once('/') {
            Some((p, n)) => (p, n),
            None => ("", path),
        };
        let p = self.get_mut(parent)?;
        let i = p.children.iter().position(|(n, _)| n == name)?;
        Some(p.children.remove(i).1)
    }
}

/// A plausible mime type from the file extension, as rclone would report it.
fn mime_of(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "tiff" => "image/tiff",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        _ => "application/octet-stream",
    }
}

fn demo_time() -> jiff::Timestamp {
    "2025-08-01T10:00:00Z".parse().unwrap()
}

struct FakeTransfer {
    spec: TransferSpec,
    /// Only the first `started` items run (a partly failed start).
    started: usize,
    /// One job per started item.
    ids: Vec<u64>,
    /// Jobs whose item arrived.
    done: Vec<u64>,
    total: u64,
    percent: u64,
    state: JobsState,
}

#[derive(Default)]
struct State {
    remotes: Vec<RemoteInfo>,
    trees: HashMap<Remote, Node>,
    abouts: HashMap<Remote, About>,
    transfers: HashMap<String, FakeTransfer>,
    next_job: u64,
    fail_next: HashMap<&'static str, BackendError>,
    list_delay: Option<Duration>,
    progress_error: Option<BackendError>,
    fail_start_after: Option<(usize, BackendError)>,
    /// With `fail_start_after`: the failing job did start, only its answer was lost.
    lose_start_reply: bool,
    /// Pause between two jobs of one start.
    start_delay: Option<Duration>,
    disconnected: bool,
    calls: Vec<String>,
    /// Folders whose listing always fails.
    fail_list_at: HashMap<Location, BackendError>,
    /// Folders whose listing panics (a bug in a backend task).
    panic_list_at: Vec<Location>,
    lists_in_flight: usize,
    bwlimit: Option<String>,
    max_lists_in_flight: usize,
    /// Saved account sections (`type` plus options), kept in step with `remotes`.
    configs: BTreeMap<String, BTreeMap<String, String>>,
    config_jobs: HashMap<u64, ConfigJob>,
    auth_delay: Duration,
    /// `(method, keys sent, obscure)` of every config call.
    sent: Vec<(String, Vec<String>, bool)>,
}

/// A browser login running as a job: it takes effect when it finishes, unless stopped.
struct ConfigJob {
    name: String,
    answer: String,
    state: String,
    ready_at: Instant,
    stopped: bool,
}

const FAKE_TOKEN: &str = r#"{"access_token":"fake","token_type":"bearer"}"#;

fn is_oauth_kind(kind: &str) -> bool {
    matches!(kind, "drive" | "dropbox" | "onedrive")
}

fn question(state: &str, option: ProviderOption, error: Option<&str>) -> ConfigStep {
    ConfigStep::Question {
        state: state.to_string(),
        option,
        error: error.map(str::to_string),
    }
}

fn bool_opt(name: &str, help: &str) -> ProviderOption {
    ProviderOption {
        name: name.into(),
        help: help.into(),
        default: "false".into(),
        examples: vec![("true".into(), "Yes".into()), ("false".into(), "No".into())],
        exclusive: true,
        kind: "bool".into(),
        ..Default::default()
    }
}

fn text_opt(name: &str, help: &str) -> ProviderOption {
    ProviderOption {
        name: name.into(),
        help: help.into(),
        kind: "string".into(),
        ..Default::default()
    }
}

fn q_is_local() -> ConfigStep {
    question(
        "oauth-islocal",
        bool_opt(
            "config_is_local",
            "Use web browser to automatically authenticate rclone with remote?",
        ),
        None,
    )
}

/// As rclone asks it: the command to run elsewhere names the type, and a client of
/// one's own travels as one more (encoded) argument.
fn q_token(kind: &str, client_id: Option<&str>, error: Option<&str>) -> ConfigStep {
    let extra = client_id.map(|c| format!(" \"enc-{c}\"")).unwrap_or_default();
    let help = format!(
        "For this to work, you will need rclone available on a machine that has\na web browser available.\n\n\
         Execute the following on the machine with the web browser (same rclone\nversion recommended):\n\n\
         \trclone authorize \"{kind}\"{extra}\n\nThen paste the result.\n"
    );
    question("oauth-token", text_opt("config_token", &help), error)
}

impl State {
    fn kind_of(&self, name: &str) -> String {
        self.configs
            .get(name)
            .and_then(|c| c.get("type"))
            .cloned()
            .unwrap_or_default()
    }

    /// Store the values as rclone would: `config_*` answers are not saved, passwords are obscured.
    fn write(&mut self, name: &str, params: &ConfigParams) {
        let section = self.configs.entry(name.to_string()).or_default();
        for (k, v) in &params.values {
            if k.starts_with("config_") {
                continue;
            }
            let v = if params.obscure && matches!(k.as_str(), "pass" | "password" | "password2") {
                format!("obscured-{}", v.len())
            } else {
                v.clone()
            };
            section.insert(k.clone(), v);
        }
    }

    /// An answer is recorded under the name of the question it answers.
    fn record_answer(&mut self, method: &str, state: &str, obscure: bool) {
        let key = match state {
            "client-warning" => "config_shared_client_id",
            "oauth-confirm" => "config_refresh_token",
            "oauth-islocal" => "config_is_local",
            "oauth-token" => "config_token",
            "teamdrive" => "config_team_drive",
            other => other,
        };
        self.sent
            .push((method.to_string(), vec![key.to_string()], obscure));
    }

    fn record(&mut self, method: &str, params: &ConfigParams) {
        self.sent.push((
            method.to_string(),
            params.values.keys().cloned().collect(),
            params.obscure,
        ));
    }

    fn after_oauth(&mut self, name: &str) -> ConfigStep {
        self.configs
            .entry(name.to_string())
            .or_default()
            .insert("token".into(), FAKE_TOKEN.into());
        if self.kind_of(name) == "drive" {
            question(
                "teamdrive",
                bool_opt(
                    "config_team_drive",
                    "Configure this as a Shared Drive (Team Drive)?",
                ),
                None,
            )
        } else {
            ConfigStep::Done
        }
    }

    /// One step of the scripted config state machine.
    fn advance(&mut self, name: &str, answer: &str, state: &str) -> ConfigStep {
        match state {
            "client-warning" if answer == "true" => q_is_local(),
            "client-warning" => ConfigStep::Done,
            "oauth-confirm" if answer == "true" => q_is_local(),
            "oauth-confirm" => ConfigStep::Done,
            "oauth-islocal" if answer == "true" => self.after_oauth(name),
            "oauth-islocal" => {
                let client = self.configs.get(name).and_then(|c| c.get("client_id")).cloned();
                q_token(
                    &self.kind_of(name),
                    client.as_deref().filter(|c| !c.is_empty()),
                    None,
                )
            }
            "oauth-token" if answer.trim_start().starts_with('{') => self.after_oauth(name),
            "oauth-token" if answer.is_empty() => ConfigStep::Done,
            "oauth-token" => ConfigStep::Rejected {
                state: state.into(),
                error: "Couldn't decode response - try again".into(),
            },
            _ => ConfigStep::Done,
        }
    }
}

impl State {
    fn tree(&mut self, remote: &Remote) -> &mut Node {
        self.trees.entry(remote.clone()).or_insert_with(Node::dir)
    }

    /// Record the call and apply injected failures.
    fn enter(&mut self, method: &'static str) -> Result<(), BackendError> {
        self.calls.push(method.to_string());
        if self.disconnected {
            return Err(BackendError::Disconnected);
        }
        match self.fail_next.remove(method) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn apply(&mut self, group: &str) {
        let Some(t) = self.transfers.get(group) else {
            return;
        };
        if t.state != JobsState::Running {
            return;
        }
        let spec = t.spec.clone();
        let mut result = JobsState::Succeeded;
        let started = t.started;
        let ids = t.ids.clone();
        let mut done = Vec::new();
        for (item, id) in spec.items.iter().take(started).zip(ids) {
            let src = spec.src_dir.join(&item.name);
            let dst = spec.dst_dir.join(&item.dst_name);
            let node = match spec.kind {
                TransferKind::Copy => self.tree(&src.remote).get(&src.path).cloned(),
                TransferKind::Move => self.tree(&src.remote).remove(&src.path),
            };
            match node {
                Some(n) => {
                    self.tree(&dst.remote).insert(&dst.path, n);
                    done.push(id);
                }
                None => result = JobsState::Failed(format!("{}: object not found", item.name)),
            }
        }
        let t = self.transfers.get_mut(group).unwrap();
        t.done = done;
        t.percent = 100;
        t.state = result;
    }
}

fn not_found(loc: &Location) -> BackendError {
    BackendError::Rclone(format!("directory not found: {}", loc.display()))
}

fn entry(name: &str, n: &Node) -> Entry {
    Entry {
        name: name.to_string(),
        size: (!n.is_dir).then_some(n.size),
        mod_time: Some(n.mod_time),
        is_dir: n.is_dir,
        mime: if n.is_dir {
            "inode/directory".into()
        } else {
            mime_of(name).into()
        },
    }
}

impl FakeBackend {
    pub fn empty() -> FakeBackend {
        let state = State {
            remotes: vec![RemoteInfo {
                remote: Remote::Local,
                kind: "local".into(),
            }],
            ..Default::default()
        };
        FakeBackend {
            state: Mutex::new(state),
        }
    }

    /// Plausible accounts and files for `--demo` and UI snapshots.
    pub fn demo() -> FakeBackend {
        let f = FakeBackend::empty();
        let gb = 1024 * 1024 * 1024u64;
        let named = |n: &str, p: &str| Location::new(Remote::Named(n.into()), p);
        for (name, kind, used, total) in [
            ("personal", "drive", 9.3, 15.0),
            ("work", "drive", 2.7, 15.0),
            ("media", "mega", 8.0, 20.0),
            ("archive", "mega", 18.2, 20.0),
            ("shared", "dropbox", 0.08, 2.0),
        ] {
            f.add_remote(name, kind);
            let used = (used * gb as f64) as u64;
            let total = (total * gb as f64) as u64;
            f.set_about(
                &Remote::Named(name.into()),
                About {
                    total: Some(total),
                    used: Some(used),
                    free: Some(total - used),
                    ..Default::default()
                },
            );
        }
        let home = |p: &str| Location::new(Remote::Local, &format!("home/demo/{p}"));
        f.add_dir(&home("Documents"));
        f.add_dir(&home("Downloads/iso"));
        f.add_file(&home("Downloads/summary.pdf"), 2_200_000);
        f.add_file(&home("Downloads/video.mkv"), 1_500_000_000);
        f.add_file(&home("Downloads/日本語のファイル名.txt"), 1_234);
        f.add_file(
            &home("Downloads/a very long file name that will certainly not fit in the column.tar.gz"),
            98_765_432,
        );
        f.add_file(&home("Downloads/it's \"quoted\" & spaced.txt"), 42);
        f.add_file(&home(".bashrc"), 3_500);
        f.add_dir(&named("personal", "Photos/2025/Holidays"));
        f.add_dir(&named("personal", "Photos/2025/Christmas"));
        for i in 10..30 {
            f.add_file(
                &named("personal", &format!("Photos/2025/IMG_00{i}.jpg")),
                3_900_000 + i * 10_000,
            );
        }
        let mb = 1_000_000u64;
        for (path, size) in [
            ("Videos/beach.mp4", 1_200 * mb),
            ("Videos/birthday.mkv", 350 * mb),
            ("Photos/2023/drone_flight.mp4", 1_400 * mb),
            ("Photos/2024/panorama.tiff", 210 * mb),
            ("Photos/2024/IMG_4410.jpg", 4 * mb),
            ("Backups/2024/archive.zip", 600 * mb),
            ("Backups/2024/phone.tar.gz", 180 * mb),
            ("Documents/taxes-2024.pdf", 3 * mb),
            ("Documents/manual.pdf", 12 * mb),
            ("Documents/notes.md", 20_000),
            ("Music/album/01 intro.mp3", 9 * mb),
            ("Music/album/02 song.mp3", 11 * mb),
        ] {
            f.add_file(&named("personal", path), size);
        }
        f.add_dir(&named("personal", "Work"));
        f.add_file(&named("personal", "Work/cv.pdf"), 180_000);
        f.add_dir(&named("work", "Backups"));
        f.add_file(&named("work", "notes.md"), 4_096);
        f.add_dir(&named("media", "Music"));
        f.add_dir(&named("archive", "Movies"));
        f.add_file(&named("archive", "Movies/movie.mkv"), 4_500_000_000);
        f.add_file(&named("shared", "Getting Started.pdf"), 1_100_000);
        f
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    pub fn add_remote(&self, name: &str, kind: &str) {
        let mut s = self.lock();
        s.remotes.push(RemoteInfo {
            remote: Remote::Named(name.into()),
            kind: kind.into(),
        });
        s.tree(&Remote::Named(name.into()));
        let mut section = BTreeMap::from([("type".to_string(), kind.to_string())]);
        if is_oauth_kind(kind) {
            section.insert("token".into(), FAKE_TOKEN.into());
        }
        s.configs.insert(name.into(), section);
    }

    /// Set one saved value of an account (test control).
    pub fn set_config_value(&self, name: &str, key: &str, value: &str) {
        self.lock()
            .configs
            .entry(name.into())
            .or_default()
            .insert(key.into(), value.into());
    }

    /// The saved section of an account, `None` when it does not exist.
    pub fn config_of(&self, name: &str) -> Option<BTreeMap<String, String>> {
        self.lock().configs.get(name).cloned()
    }

    /// How long a browser login takes when run as a job.
    pub fn set_auth_delay(&self, d: Duration) {
        self.lock().auth_delay = d;
    }

    /// `(method, keys sent, obscure)` of every config call so far.
    pub fn sent_params(&self) -> Vec<(String, Vec<String>, bool)> {
        self.lock().sent.clone()
    }

    pub fn add_dir(&self, loc: &Location) {
        self.lock().tree(&loc.remote).insert(&loc.path, Node::dir());
    }

    pub fn add_file(&self, loc: &Location, size: u64) {
        self.lock().tree(&loc.remote).insert(&loc.path, Node::file(size));
    }

    pub fn exists(&self, loc: &Location) -> bool {
        self.lock().tree(&loc.remote).get(&loc.path).is_some()
    }

    pub fn set_about(&self, remote: &Remote, about: About) {
        self.lock().abouts.insert(remote.clone(), about);
    }

    pub fn fail_next(&self, method: &'static str, err: BackendError) {
        self.lock().fail_next.insert(method, err);
    }

    /// Listing `loc` fails with `err` every time.
    pub fn fail_list_at(&self, loc: &Location, err: BackendError) {
        self.lock().fail_list_at.insert(loc.clone(), err);
    }

    /// The bandwidth limit last set (`None` = off or never set).
    pub fn bwlimit(&self) -> Option<String> {
        self.lock().bwlimit.clone()
    }

    /// Listing `loc` panics.
    pub fn panic_list_at(&self, loc: &Location) {
        self.lock().panic_list_at.push(loc.clone());
    }

    /// Most `list` calls that were running at the same time.
    pub fn max_concurrent_lists(&self) -> usize {
        self.lock().max_lists_in_flight
    }

    pub fn set_list_delay(&self, d: Duration) {
        self.lock().list_delay = Some(d);
    }

    /// The next start launches only `n` items, then fails with `err` (test control).
    /// The answer to job `n + 1` of the next transfer is lost: rclone runs it, the
    /// app only hears the first `n` and a [`BackendError::Disconnected`].
    pub fn lose_start_reply_after(&self, n: usize) {
        let mut s = self.lock();
        s.fail_start_after = Some((n, BackendError::Disconnected));
        s.lose_start_reply = true;
    }

    pub fn fail_start_after(&self, n: usize, err: BackendError) {
        self.lock().fail_start_after = Some((n, err));
    }

    /// Wait `d` before starting each job after the first (test control).
    pub fn set_start_delay(&self, d: Duration) {
        self.lock().start_delay = Some(d);
    }

    /// Every progress query fails with `err` until cleared (test control).
    pub fn set_progress_error(&self, err: Option<BackendError>) {
        self.lock().progress_error = err;
    }

    pub fn set_disconnected(&self, on: bool) {
        self.lock().disconnected = on;
    }

    /// Finish every running transfer now, applying it to the tree.
    pub fn complete_transfers(&self) {
        let mut s = self.lock();
        let groups: Vec<String> = s.transfers.keys().cloned().collect();
        for g in groups {
            s.apply(&g);
        }
    }

    /// Remove an entry behind the app's back (test control).
    pub fn delete_now(&self, loc: &Location) {
        self.lock().tree(&loc.remote).remove(&loc.path);
    }

    /// Simulate a restarted rcd: every transfer it knew about is gone.
    pub fn forget_transfers(&self) {
        self.lock().transfers.clear();
    }

    pub fn calls(&self) -> Vec<String> {
        self.lock().calls.clone()
    }
}

#[async_trait]
impl Backend for FakeBackend {
    async fn remotes(&self) -> Result<Vec<RemoteInfo>, BackendError> {
        let mut s = self.lock();
        s.enter("remotes")?;
        Ok(s.remotes.clone())
    }

    async fn list(&self, dir: &Location) -> Result<Vec<Entry>, BackendError> {
        let delay = {
            let mut s = self.lock();
            s.enter("list")?;
            s.lists_in_flight += 1;
            s.max_lists_in_flight = s.max_lists_in_flight.max(s.lists_in_flight);
            s.list_delay
        };
        // Counted down even when the caller drops this future mid-sleep.
        struct InFlight<'a>(&'a FakeBackend);
        impl Drop for InFlight<'_> {
            fn drop(&mut self) {
                self.0.lock().lists_in_flight -= 1;
            }
        }
        let _guard = InFlight(self);
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
        let mut s = self.lock();
        if let Some(e) = s.fail_list_at.get(dir) {
            return Err(e.clone());
        }
        if s.panic_list_at.contains(dir) {
            drop(s);
            panic!("fake listing of {} panics", dir.display());
        }
        match s.tree(&dir.remote).get(&dir.path) {
            Some(n) if n.is_dir => Ok(n.children.iter().map(|(name, c)| entry(name, c)).collect()),
            _ => Err(not_found(dir)),
        }
    }

    async fn stat(&self, loc: &Location) -> Result<Option<Entry>, BackendError> {
        let mut s = self.lock();
        s.enter("stat")?;
        Ok(s.tree(&loc.remote).get(&loc.path).map(|n| entry(loc.name(), n)))
    }

    async fn about(&self, remote: &Remote) -> Result<About, BackendError> {
        let mut s = self.lock();
        s.enter("about")?;
        Ok(s.abouts.get(remote).cloned().unwrap_or_default())
    }

    async fn mkdir(&self, loc: &Location) -> Result<(), BackendError> {
        let mut s = self.lock();
        s.enter("mkdir")?;
        s.tree(&loc.remote).insert(&loc.path, Node::dir());
        Ok(())
    }

    async fn rename(&self, from: &Location, new_name: &str, _is_dir: bool) -> Result<(), BackendError> {
        let mut s = self.lock();
        s.enter("rename")?;
        let to = from.parent().unwrap_or_else(|| from.clone()).join(new_name);
        // As the real backend: a rename never lands on another entry.
        if to != *from && s.tree(&to.remote).get(&to.path).is_some() {
            return Err(BackendError::Other(format!("“{new_name}” already exists here")));
        }
        let node = s
            .tree(&from.remote)
            .remove(&from.path)
            .ok_or_else(|| not_found(from))?;
        s.tree(&to.remote).insert(&to.path, node);
        Ok(())
    }

    async fn delete(&self, loc: &Location, _is_dir: bool) -> Result<(), BackendError> {
        let mut s = self.lock();
        s.enter("delete")?;
        s.tree(&loc.remote)
            .remove(&loc.path)
            .ok_or_else(|| not_found(loc))?;
        Ok(())
    }

    async fn start_transfer(
        &self,
        spec: &TransferSpec,
        on_job: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<Vec<u64>, StartError> {
        let group = spec.group();
        let (partial, started, delay) = {
            let mut s = self.lock();
            s.enter("start_transfer")?;
            let total = spec
                .items
                .iter()
                .filter_map(|i| {
                    s.trees
                        .get(&spec.src_dir.remote)?
                        .get(&spec.src_dir.join(&i.name).path)
                })
                .map(Node::total_size)
                .sum();
            let partial = s.fail_start_after.take();
            let lost = partial.is_some() && std::mem::take(&mut s.lose_start_reply);
            let started = partial.as_ref().map_or(spec.items.len(), |(n, _)| {
                (*n + usize::from(lost)).min(spec.items.len())
            });
            if started > 0 {
                s.transfers.insert(
                    group.clone(),
                    FakeTransfer {
                        spec: spec.clone(),
                        started: 0,
                        ids: Vec::new(),
                        done: Vec::new(),
                        total,
                        percent: 0,
                        state: JobsState::Running,
                    },
                );
            }
            (partial.map(|p| (p, lost)), started, s.start_delay)
        };
        let mut ids = Vec::with_capacity(started);
        for k in 0..started {
            if k > 0
                && let Some(d) = delay
            {
                tokio::time::sleep(d).await;
            }
            let id = {
                let mut s = self.lock();
                s.next_job += 1;
                let id = s.next_job;
                if let Some(t) = s.transfers.get_mut(&group) {
                    t.ids.push(id);
                    t.started += 1;
                }
                id
            };
            if partial.as_ref().is_some_and(|&(_, lost)| lost) && k + 1 == started {
                break;
            }
            on_job(id);
            ids.push(id);
        }
        match partial {
            Some(((_, error), uncertain)) => Err(StartError {
                started: ids,
                error,
                uncertain,
            }),
            None => Ok(ids),
        }
    }

    /// One job per item.
    fn job_units(&self, spec: &TransferSpec) -> Vec<Vec<usize>> {
        (0..spec.items.len()).map(|i| vec![i]).collect()
    }

    async fn group_jobs(&self, group: &str) -> Result<Vec<u64>, BackendError> {
        let mut s = self.lock();
        s.enter("group_jobs")?;
        Ok(s.transfers.get(group).map(|t| t.ids.clone()).unwrap_or_default())
    }

    async fn transfer_progress(
        &self,
        group: &str,
        _jobids: &[u64],
    ) -> Result<TransferProgress, BackendError> {
        let mut s = self.lock();
        s.enter("transfer_progress")?;
        if let Some(e) = s.progress_error.clone() {
            return Err(e);
        }
        let advance = match s.transfers.get_mut(group) {
            None => {
                return Ok(TransferProgress {
                    state: JobsState::NotFound,
                    bytes: 0,
                    total_bytes: 0,
                    speed: 0.0,
                    eta: None,
                    errors: 0,
                    files: Vec::new(),
                    done_jobs: Vec::new(),
                });
            }
            Some(t) if t.state == JobsState::Running => {
                t.percent += 25;
                t.percent >= 100
            }
            Some(_) => false,
        };
        if advance {
            s.apply(group);
        }
        let t = &s.transfers[group];
        let bytes = t.total * t.percent.min(100) / 100;
        let running = t.state == JobsState::Running;
        Ok(TransferProgress {
            state: t.state.clone(),
            bytes,
            total_bytes: t.total,
            speed: if running { 12.0 * 1024.0 * 1024.0 } else { 0.0 },
            eta: running.then_some(41),
            errors: matches!(t.state, JobsState::Failed(_)) as u64,
            files: if running {
                vec![FileProgress {
                    name: t.spec.items[0].name.clone(),
                    bytes,
                    size: Some(t.total),
                    percentage: t.percent as u8,
                }]
            } else {
                Vec::new()
            },
            done_jobs: t.done.clone(),
        })
    }

    async fn stop_transfer(&self, group: &str) -> Result<(), BackendError> {
        let mut s = self.lock();
        s.enter("stop_transfer")?;
        if let Some(t) = s.transfers.get_mut(group)
            && t.state == JobsState::Running
        {
            t.state = JobsState::Failed("context canceled".into());
        }
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), BackendError> {
        self.lock().enter("shutdown")
    }

    async fn providers(&self) -> Result<Vec<Provider>, BackendError> {
        self.lock().enter("providers")?;
        Ok(fake_providers())
    }

    async fn remote_config(&self, name: &str) -> Result<BTreeMap<String, String>, BackendError> {
        let mut s = self.lock();
        s.enter("remote_config")?;
        // rclone answers an unknown name with an empty object.
        Ok(s.configs.get(name).cloned().unwrap_or_default())
    }

    async fn config_create(
        &self,
        name: &str,
        kind: &str,
        params: ConfigParams,
    ) -> Result<ConfigStep, BackendError> {
        let mut s = self.lock();
        s.enter("config_create")?;
        s.record("config_create", &params);
        s.configs.insert(
            name.into(),
            BTreeMap::from([("type".to_string(), kind.to_string())]),
        );
        s.write(name, &params);
        s.remotes.push(RemoteInfo {
            remote: Remote::Named(name.into()),
            kind: kind.into(),
        });
        s.tree(&Remote::Named(name.into()));
        let has_client = params.values.get("client_id").is_some_and(|v| !v.is_empty());
        Ok(match kind {
            "drive" if !has_client => question(
                "client-warning",
                bool_opt(
                    "config_shared_client_id",
                    "rclone's shared Google Drive client_id is being retired and will stop working during 2026.\nCreate your own to avoid interruption: https://rclone.org/drive/#making-your-own-client-id\n\nContinue using the shared client_id anyway?",
                ),
                None,
            ),
            "drive" | "dropbox" => q_is_local(),
            _ => ConfigStep::Done,
        })
    }

    async fn config_update(&self, name: &str, params: ConfigParams) -> Result<ConfigStep, BackendError> {
        let mut s = self.lock();
        s.enter("config_update")?;
        s.record("config_update", &params);
        if !s.configs.contains_key(name) {
            return Err(BackendError::Rclone(format!(
                "couldn't find type of fs for \"{name}\""
            )));
        }
        s.write(name, &params);
        let has_token = s.configs[name].contains_key("token");
        let refresh = params.values.get("config_refresh_token").map(String::as_str);
        Ok(match (has_token, refresh) {
            (_, Some("true")) if is_oauth_kind(&s.kind_of(name)) => q_is_local(),
            (true, _) => question(
                "oauth-confirm",
                bool_opt("config_refresh_token", "Token already configured - replace it?"),
                None,
            ),
            _ => ConfigStep::Done,
        })
    }

    async fn config_continue(
        &self,
        name: &str,
        state: &str,
        answer: &str,
        obscure: bool,
    ) -> Result<ConfigStep, BackendError> {
        let mut s = self.lock();
        s.enter("config_continue")?;
        s.record_answer("config_continue", state, obscure);
        Ok(s.advance(name, answer, state))
    }

    async fn config_continue_job(
        &self,
        name: &str,
        state: &str,
        answer: &str,
        obscure: bool,
    ) -> Result<u64, BackendError> {
        let mut s = self.lock();
        s.enter("config_continue_job")?;
        s.record_answer("config_continue_job", state, obscure);
        s.next_job += 1;
        let id = s.next_job;
        let ready_at = Instant::now() + s.auth_delay;
        s.config_jobs.insert(
            id,
            ConfigJob {
                name: name.into(),
                answer: answer.into(),
                state: state.into(),
                ready_at,
                stopped: false,
            },
        );
        Ok(id)
    }

    async fn config_job(&self, jobid: u64) -> Result<Option<Result<ConfigStep, BackendError>>, BackendError> {
        let mut s = self.lock();
        s.enter("config_job")?;
        let Some(job) = s.config_jobs.get(&jobid) else {
            return Err(BackendError::Rclone("job not found".into()));
        };
        if job.stopped {
            return Ok(Some(Err(BackendError::Rclone("job stopped".into()))));
        }
        if Instant::now() < job.ready_at {
            return Ok(None);
        }
        let job = s.config_jobs.remove(&jobid).unwrap();
        Ok(Some(Ok(s.advance(&job.name, &job.answer, &job.state))))
    }

    async fn stop_job(&self, jobid: u64) -> Result<(), BackendError> {
        let mut s = self.lock();
        s.enter("stop_job")?;
        match s.config_jobs.get_mut(&jobid) {
            Some(j) => {
                j.stopped = true;
                Ok(())
            }
            None => Err(BackendError::Rclone("job not found".into())),
        }
    }

    async fn config_delete(&self, name: &str) -> Result<(), BackendError> {
        let mut s = self.lock();
        s.enter("config_delete")?;
        s.configs.remove(name);
        let r = Remote::Named(name.into());
        s.remotes.retain(|x| x.remote != r);
        s.trees.remove(&r);
        Ok(())
    }

    async fn get_bwlimit(&self) -> Result<String, BackendError> {
        Ok(self.lock().bwlimit.clone().unwrap_or_else(|| "off".into()))
    }

    async fn set_bwlimit(&self, rate: &str) -> Result<String, BackendError> {
        let mut s = self.lock();
        s.calls.push(format!("bwlimit {rate}"));
        if !crate::app::settings::valid_bwlimit(rate) {
            return Err(BackendError::Rclone(format!("invalid bandwidth \"{rate}\"")));
        }
        s.bwlimit = (rate != "off").then(|| rate.to_string());
        Ok(rate.to_string())
    }
}

/// A few realistic backend types (options trimmed to what the wizard needs).
fn fake_providers() -> Vec<Provider> {
    let pw = |name: &str, help: &str, required: bool| ProviderOption {
        is_password: true,
        required,
        ..text_opt(name, help)
    };
    let req = |o: ProviderOption| ProviderOption { required: true, ..o };
    let adv = |o: ProviderOption| ProviderOption { advanced: true, ..o };
    let provider = |name: &str, description: &str, options: Vec<ProviderOption>| Provider {
        name: name.into(),
        description: description.into(),
        hide: false,
        options,
    };
    let examples = |o: ProviderOption, ex: &[(&str, &str)], exclusive: bool| ProviderOption {
        examples: ex.iter().map(|(v, h)| (v.to_string(), h.to_string())).collect(),
        exclusive,
        ..o
    };
    vec![
        provider(
            "alias",
            "Alias for an existing remote",
            vec![req(text_opt(
                "remote",
                "Remote or path to alias.\n\nCan be \"myremote:path/to/dir\", \"myremote:bucket\", \"myremote:\" or \"/local/path\".",
            ))],
        ),
        provider(
            "crypt",
            "Encrypt/Decrypt a remote",
            vec![
                req(text_opt("remote", "Remote to encrypt/decrypt.")),
                examples(
                    ProviderOption {
                        default: "standard".into(),
                        ..text_opt("filename_encryption", "How to encrypt the filenames.")
                    },
                    &[
                        ("standard", "Encrypt the filenames."),
                        ("obfuscate", "Very simple filename obfuscation."),
                        ("off", "Don't encrypt the file names."),
                    ],
                    true,
                ),
                pw("password", "Password or pass phrase for encryption.", true),
            ],
        ),
        provider(
            "drive",
            "Google Drive",
            vec![
                text_opt(
                    "client_id",
                    "Google Application Client Id\nSetting your own is recommended.",
                ),
                ProviderOption {
                    sensitive: true,
                    ..text_opt("client_secret", "OAuth Client Secret.")
                },
                examples(
                    text_opt(
                        "scope",
                        "Comma separated list of scopes that rclone should use when requesting access from drive.\n\nChoose a number from below, or type in your own value.",
                    ),
                    &[
                        ("drive", "Full access all files"),
                        ("drive.readonly", "Read-only access"),
                        ("drive.file", "Access to files created by rclone only"),
                        ("drive.appfolder", "Application Data folder"),
                        ("drive.metadata.readonly", "Metadata only"),
                    ],
                    false,
                ),
                ProviderOption {
                    sensitive: true,
                    ..text_opt("token", "OAuth Access Token as a JSON blob.")
                },
                adv(text_opt("team_drive", "ID of the Shared Drive (Team Drive).")),
            ],
        ),
        provider(
            "dropbox",
            "Dropbox",
            vec![
                text_opt("client_id", "OAuth Client Id."),
                ProviderOption {
                    sensitive: true,
                    ..text_opt("client_secret", "OAuth Client Secret.")
                },
                ProviderOption {
                    sensitive: true,
                    ..text_opt("token", "OAuth Access Token as a JSON blob.")
                },
            ],
        ),
        provider(
            "mega",
            "Mega",
            vec![
                req(text_opt("user", "User name.")),
                pw("pass", "Password.", true),
                text_opt(
                    "2fa",
                    "The 2FA code of your MEGA account if the account is set up with one",
                ),
                adv(bool_opt("debug", "Output more debug from Mega.")),
            ],
        ),
        provider(
            "onedrive",
            "Microsoft OneDrive",
            vec![
                text_opt("client_id", "OAuth Client Id."),
                ProviderOption {
                    sensitive: true,
                    ..text_opt("token", "OAuth Access Token as a JSON blob.")
                },
            ],
        ),
        Provider {
            hide: true,
            ..provider("tardigrade", "Storj Decentralized Cloud Storage", vec![])
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::spec::TransferItem;

    fn g(p: &str) -> Location {
        Location::new(Remote::Named("g".into()), p)
    }
    fn t(p: &str) -> Location {
        Location::new(Remote::Named("t".into()), p)
    }
    fn kv(pairs: &[(&str, &str)]) -> ConfigParams {
        ConfigParams {
            values: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            obscure: false,
        }
    }

    fn asks(step: &ConfigStep) -> (String, String) {
        match step {
            ConfigStep::Question { state, option, .. } => (option.name.clone(), state.clone()),
            ConfigStep::Done => ("DONE".into(), String::new()),
            ConfigStep::Rejected { state, .. } => ("REJECTED".into(), state.clone()),
        }
    }

    #[tokio::test]
    async fn dropbox_remote_path_flow() {
        let f = FakeBackend::empty();
        let step = f.config_create("d", "dropbox", kv(&[])).await.unwrap();
        let (q, st) = asks(&step);
        assert_eq!(q, "config_is_local");
        let step = f.config_continue("d", &st, "false", false).await.unwrap();
        let (q, st) = asks(&step);
        assert_eq!(q, "config_token");
        let step = f.config_continue("d", &st, "{}", false).await.unwrap();
        assert_eq!(step, ConfigStep::Done);
        assert!(f.config_of("d").unwrap().contains_key("token"));
        assert!(
            f.remotes()
                .await
                .unwrap()
                .iter()
                .any(|r| r.remote == Remote::Named("d".into()))
        );
    }

    #[tokio::test]
    async fn drive_warning_then_oauth_then_team_drive() {
        let f = FakeBackend::empty();
        let (q, st) = asks(&f.config_create("g", "drive", kv(&[])).await.unwrap());
        assert_eq!(q, "config_shared_client_id");
        let (q, st) = asks(&f.config_continue("g", &st, "true", false).await.unwrap());
        assert_eq!(q, "config_is_local");
        let (q, st) = asks(&f.config_continue("g", &st, "true", false).await.unwrap());
        assert_eq!(q, "config_team_drive");
        let step = f.config_continue("g", &st, "false", false).await.unwrap();
        assert_eq!(step, ConfigStep::Done);
        // With its own client the warning is skipped.
        let (q, _) = asks(
            &f.config_create("h", "drive", kv(&[("client_id", "x")]))
                .await
                .unwrap(),
        );
        assert_eq!(q, "config_is_local");
    }

    #[tokio::test]
    async fn refresh_token_false_is_done() {
        let f = FakeBackend::empty();
        f.add_remote("d", "dropbox");
        let (q, st) = asks(&f.config_update("d", kv(&[("client_id", "abc")])).await.unwrap());
        assert_eq!(q, "config_refresh_token");
        assert_eq!(f.config_of("d").unwrap()["client_id"], "abc");
        let step = f.config_continue("d", &st, "false", false).await.unwrap();
        assert_eq!(step, ConfigStep::Done);
        assert!(!f.config_of("d").unwrap().contains_key("config_refresh_token"));
        // Asking for a new token goes straight to the browser question.
        let (q, _) = asks(
            &f.config_update("d", kv(&[("config_refresh_token", "true")]))
                .await
                .unwrap(),
        );
        assert_eq!(q, "config_is_local");
    }

    #[tokio::test]
    async fn stopped_job_writes_nothing() {
        let f = FakeBackend::empty();
        f.set_auth_delay(Duration::from_secs(60));
        let (_, st) = asks(&f.config_create("d", "dropbox", kv(&[])).await.unwrap());
        let job = f.config_continue_job("d", &st, "true", false).await.unwrap();
        assert_eq!(f.config_job(job).await.unwrap(), None);
        f.stop_job(job).await.unwrap();
        let r = f.config_job(job).await.unwrap().unwrap();
        assert!(r.is_err());
        assert!(!f.config_of("d").unwrap().contains_key("token"));
        // Without a delay the job finishes with the login done.
        f.set_auth_delay(Duration::ZERO);
        let job = f.config_continue_job("d", &st, "true", false).await.unwrap();
        assert_eq!(f.config_job(job).await.unwrap(), Some(Ok(ConfigStep::Done)));
        assert!(f.config_of("d").unwrap().contains_key("token"));
    }

    #[tokio::test]
    async fn invalid_token_is_rejected() {
        let f = FakeBackend::empty();
        let (_, st) = asks(&f.config_create("d", "dropbox", kv(&[])).await.unwrap());
        let (_, st) = asks(&f.config_continue("d", &st, "false", false).await.unwrap());
        let step = f.config_continue("d", &st, "notjson", false).await.unwrap();
        let ConfigStep::Rejected { state, error } = step else {
            panic!("{step:?}")
        };
        assert_eq!(state, st);
        assert!(error.starts_with("Couldn't decode response"));
        // Like rclone, an empty answer ends the flow without a token.
        assert_eq!(
            f.config_continue("d", &st, "", false).await.unwrap(),
            ConfigStep::Done
        );
        assert!(!f.config_of("d").unwrap().contains_key("token"));
    }

    #[tokio::test]
    async fn passwords_obscured_and_params_recorded() {
        let f = FakeBackend::empty();
        let p = ConfigParams {
            obscure: true,
            ..kv(&[("user", "x"), ("pass", "hunter2")])
        };
        assert_eq!(f.config_create("m", "mega", p).await.unwrap(), ConfigStep::Done);
        assert_ne!(f.config_of("m").unwrap()["pass"], "hunter2");
        assert_eq!(
            f.sent_params(),
            vec![("config_create".into(), vec!["pass".into(), "user".into()], true)]
        );
        f.config_delete("m").await.unwrap();
        assert!(f.config_of("m").is_none());
        assert!(fake_providers().iter().any(|p| p.hide));
    }

    fn fake() -> FakeBackend {
        let f = FakeBackend::empty();
        f.add_remote("g", "drive");
        f.add_remote("t", "mega");
        f.add_dir(&g("Photos"));
        f.add_file(&g("Photos/b.jpg"), 10);
        f.add_file(&g("Photos/a.jpg"), 20);
        f.add_dir(&g("Photos/sub"));
        f.add_file(&g("Photos/sub/x.txt"), 3);
        f
    }

    #[tokio::test]
    async fn remotes_start_with_local() {
        let r = fake().remotes().await.unwrap();
        assert_eq!(r[0].remote, Remote::Local);
        assert_eq!(
            r[1],
            RemoteInfo {
                remote: Remote::Named("g".into()),
                kind: "drive".into()
            }
        );
        assert_eq!(r.len(), 3);
    }

    #[tokio::test]
    async fn list_in_insertion_order() {
        let names: Vec<String> = fake()
            .list(&g("Photos"))
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["b.jpg", "a.jpg", "sub"]);
    }

    #[tokio::test]
    async fn list_missing_dir_is_error() {
        assert!(matches!(
            fake().list(&g("nope")).await,
            Err(BackendError::Rclone(_))
        ));
    }

    #[tokio::test]
    async fn file_ops_change_tree() {
        let f = fake();
        f.mkdir(&g("New")).await.unwrap();
        assert!(f.exists(&g("New")));
        f.rename(&g("Photos/a.jpg"), "c.jpg", false).await.unwrap();
        assert!(f.exists(&g("Photos/c.jpg")) && !f.exists(&g("Photos/a.jpg")));
        f.rename(&g("Photos/sub"), "sub2", true).await.unwrap();
        assert!(f.exists(&g("Photos/sub2/x.txt")));
        f.delete(&g("Photos/sub2"), true).await.unwrap();
        assert!(!f.exists(&g("Photos/sub2")));
        assert_eq!(f.stat(&g("Photos/b.jpg")).await.unwrap().unwrap().size, Some(10));
        assert_eq!(f.stat(&g("Photos/zzz")).await.unwrap(), None);
    }

    fn spec(kind: TransferKind) -> TransferSpec {
        TransferSpec {
            id: 1,
            kind,
            src_dir: g("Photos"),
            dst_dir: t(""),
            items: vec![
                TransferItem {
                    replace: false,
                    name: "sub".into(),
                    is_dir: true,
                    dst_name: "sub".into(),
                    size: None,
                },
                TransferItem {
                    replace: false,
                    name: "b.jpg".into(),
                    is_dir: false,
                    dst_name: "b (1).jpg".into(),
                    size: None,
                },
            ],
        }
    }

    #[tokio::test]
    async fn copy_dir_recursively_on_complete() {
        let f = fake();
        let s = spec(TransferKind::Copy);
        let ids = f.start_transfer(&s, &|_| {}).await.unwrap();
        assert_eq!(
            f.transfer_progress(&s.group(), &ids).await.unwrap().state,
            JobsState::Running
        );
        f.complete_transfers();
        assert_eq!(
            f.transfer_progress(&s.group(), &ids).await.unwrap().state,
            JobsState::Succeeded
        );
        assert!(f.exists(&t("sub/x.txt")) && f.exists(&t("b (1).jpg")));
        assert!(f.exists(&g("Photos/sub/x.txt")));
    }

    #[tokio::test]
    async fn progress_advances_then_completes() {
        let f = fake();
        let s = spec(TransferKind::Move);
        let ids = f.start_transfer(&s, &|_| {}).await.unwrap();
        let mut last = 0;
        for _ in 0..3 {
            let p = f.transfer_progress(&s.group(), &ids).await.unwrap();
            assert_eq!(p.state, JobsState::Running);
            assert!(p.bytes > last && p.total_bytes == 13);
            last = p.bytes;
        }
        let p = f.transfer_progress(&s.group(), &ids).await.unwrap();
        assert_eq!(p.state, JobsState::Succeeded);
        assert!(f.exists(&t("sub/x.txt")) && !f.exists(&g("Photos/sub")) && !f.exists(&g("Photos/b.jpg")));
    }

    #[tokio::test]
    async fn unknown_group_is_not_found() {
        let p = fake().transfer_progress("rtui/9", &[9]).await.unwrap();
        assert_eq!(p.state, JobsState::NotFound);
    }

    #[tokio::test]
    async fn fail_next_affects_one_call() {
        let f = fake();
        f.fail_next("list", BackendError::Rclone("token expired".into()));
        assert_eq!(
            f.list(&g("Photos")).await,
            Err(BackendError::Rclone("token expired".into()))
        );
        assert!(f.list(&g("Photos")).await.is_ok());
    }

    #[tokio::test]
    async fn disconnected_fails_everything() {
        let f = fake();
        f.set_disconnected(true);
        assert_eq!(f.list(&g("Photos")).await, Err(BackendError::Disconnected));
        assert_eq!(f.remotes().await, Err(BackendError::Disconnected));
        f.set_disconnected(false);
        assert!(f.remotes().await.is_ok());
    }

    #[tokio::test]
    async fn stop_marks_failed_and_records_calls() {
        let f = fake();
        let s = spec(TransferKind::Copy);
        let ids = f.start_transfer(&s, &|_| {}).await.unwrap();
        f.stop_transfer(&s.group()).await.unwrap();
        assert!(matches!(
            f.transfer_progress(&s.group(), &ids).await.unwrap().state,
            JobsState::Failed(_)
        ));
        assert!(!f.exists(&t("sub")));
        assert_eq!(
            f.calls(),
            ["start_transfer", "stop_transfer", "transfer_progress"]
        );
    }

    #[tokio::test]
    async fn demo_has_remotes_and_files() {
        let f = FakeBackend::demo();
        let r = f.remotes().await.unwrap();
        assert!(r.len() >= 5);
        assert!(
            !f.list(&Location::new(Remote::Named("personal".into()), "Photos/2025"))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            f.about(&Remote::Named("personal".into()))
                .await
                .unwrap()
                .used_ratio()
                .is_some()
        );
    }
}
