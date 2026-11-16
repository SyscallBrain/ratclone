//! The account wizard (`a` add, `e` edit, `A` re-authorise in the accounts sidebar).
//!
//! The form comes from `config/providers`; after it is sent, rclone's own questions
//! (OAuth, warnings…) arrive one at a time from the non-interactive config state
//! machine. A cancelled or failed add never leaves the account behind.

pub mod form;

use std::collections::BTreeMap;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::overlay::{InputOutcome, LineInput, Overlay};
use super::{App, AppEvent, MessageKind};
use crate::backend::{BackendError, ConfigParams, ConfigStep, Provider};
use crate::location::Remote;
use form::{AutoAnswer, Field, Widget};

pub const LOCAL_REFUSED: &str = "The local disk is not an rclone account";

/// What a wizard's background request answered. Each carries the id of the wizard
/// that asked, so a late answer never lands in another one.
pub enum WizardEvent {
    Providers {
        result: Result<Vec<Provider>, BackendError>,
        /// Account names asked afresh (accounts may have been made outside the app).
        names: Result<Vec<String>, BackendError>,
    },
    /// The types and the saved values of the account being edited.
    RemoteConfig(Result<(Vec<Provider>, BTreeMap<String, String>), BackendError>),
    Step(Result<ConfigStep, BackendError>),
    JobStarted(Result<u64, BackendError>),
    Job {
        jobid: u64,
        result: Result<Option<Result<ConfigStep, BackendError>>, BackendError>,
    },
    /// By the time the account was to be created, the name was taken: nothing was sent.
    NameTaken,
    /// Whether the name is free could not be checked: nothing was sent.
    NotSent(BackendError),
}

impl std::fmt::Debug for WizardEvent {
    // Saved values may hold tokens and passwords.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            WizardEvent::Providers { .. } => "Providers",
            WizardEvent::RemoteConfig(_) => "RemoteConfig",
            WizardEvent::Step(_) => "Step",
            WizardEvent::JobStarted(_) => "JobStarted",
            WizardEvent::Job { .. } => "Job",
            WizardEvent::NameTaken => "NameTaken",
            WizardEvent::NotSent(_) => "NotSent",
        };
        f.write_str(name)
    }
}

/// A text input whose value never shows in `Debug` (a pasted token).
#[derive(Clone, PartialEq, Default)]
pub struct SecretInput(pub LineInput);

impl std::fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} characters>", self.0.value.chars().count())
    }
}

impl std::ops::Deref for SecretInput {
    type Target = LineInput;
    fn deref(&self) -> &LineInput {
        &self.0
    }
}

impl std::ops::DerefMut for SecretInput {
    fn deref_mut(&mut self) -> &mut LineInput {
        &mut self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WizardMode {
    Add,
    Edit { name: String, kind: String },
    Reauth { name: String, kind: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Loading,
    Type {
        query: LineInput,
        cursor: usize,
    },
    /// The fields; `cursor` indexes [`AccountWizard::rows`].
    Form {
        cursor: usize,
        show_advanced: bool,
        help_open: bool,
    },
    Sending,
    Question {
        state: String,
        field: Box<Field>,
        error: Option<String>,
    },
    WaitingBrowser {
        jobid: Option<u64>,
        since: Instant,
    },
    TokenPaste {
        state: String,
        command: String,
        input: SecretInput,
        error: Option<String>,
    },
    DriveWarning {
        state: String,
        help: String,
    },
    ConfirmCancel {
        back: Box<Step>,
    },
    Failed {
        message: String,
        back: Box<Step>,
    },
}

/// A row of the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Name,
    Field(usize),
    /// "⋯ n advanced options (Tab to show)".
    Advanced,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountWizard {
    /// Tells this wizard's answers from those of a wizard closed earlier.
    pub id: u64,
    pub mode: WizardMode,
    pub step: Step,
    pub name: LineInput,
    pub name_error: Option<String>,
    /// A required field left empty, or another reason the form was not sent.
    pub form_error: Option<String>,
    pub provider: Option<Provider>,
    pub providers: Vec<Provider>,
    pub fields: Vec<Field>,
    /// Add: `config/create` was sent (the account may exist in rclone now).
    pub created: bool,
    pub existed_before: bool,
    /// Account names when the wizard opened.
    pub existing: Vec<String>,
    pub over_ssh: bool,
    /// The user flipped the browser/paste guess with `L`.
    pub local_flipped: bool,
    pub auto_note: Option<String>,
    pub transfers_warning: Option<String>,
    /// Edit: at least one change already reached rclone.
    pub sent_any: bool,
    /// Where the form cursor was when it was sent.
    pub last_form: (usize, bool),
    /// The state of rclone's "use this machine's browser?" question, to answer it again.
    pub local_state: Option<String>,
    /// The question last answered by the user, shown again if rclone rejects the answer.
    pub asked: Option<Step>,
    /// First line shown of the full help (F1).
    pub help_scroll: u16,
    /// Names of unfinished accounts still being removed.
    pub busy: Vec<String>,
}

impl AccountWizard {
    fn new(id: u64, mode: WizardMode, existing: Vec<String>, over_ssh: bool) -> AccountWizard {
        AccountWizard {
            id,
            mode,
            step: Step::Loading,
            name: LineInput::default(),
            name_error: None,
            form_error: None,
            provider: None,
            providers: Vec::new(),
            fields: Vec::new(),
            created: false,
            existed_before: false,
            existing,
            over_ssh,
            local_flipped: false,
            auto_note: None,
            transfers_warning: None,
            sent_any: false,
            last_form: (0, false),
            local_state: None,
            asked: None,
            help_scroll: 0,
            busy: Vec::new(),
        }
    }

    pub fn editing(&self) -> bool {
        !matches!(self.mode, WizardMode::Add)
    }

    /// The account being made or changed.
    pub fn account_name(&self) -> String {
        match &self.mode {
            WizardMode::Add => self.name.value.clone(),
            WizardMode::Edit { name, .. } | WizardMode::Reauth { name, .. } => name.clone(),
        }
    }

    pub fn kind(&self) -> String {
        match &self.mode {
            WizardMode::Add => self.provider.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
            WizardMode::Edit { kind, .. } | WizardMode::Reauth { kind, .. } => kind.clone(),
        }
    }

    pub fn description(&self) -> String {
        let kind = self.kind();
        self.provider
            .as_ref()
            .map(|p| p.description.clone())
            .or_else(|| {
                self.providers
                    .iter()
                    .find(|p| p.name == kind)
                    .map(|p| p.description.clone())
            })
            .unwrap_or(kind)
    }

    pub fn title(&self) -> String {
        match &self.mode {
            WizardMode::Add if self.provider.is_some() => format!("New account · {}", self.description()),
            WizardMode::Add => "New account".into(),
            WizardMode::Edit { name, .. } => format!("Edit account · {name} ({})", self.description()),
            WizardMode::Reauth { name, .. } => format!("Re-authorise · {name}"),
        }
    }

    /// The types matching the query of the Type step.
    pub fn filtered(&self, query: &str) -> Vec<&Provider> {
        self.providers
            .iter()
            .filter(|p| form::provider_matches(p, query))
            .collect()
    }

    /// The fields that apply to the variant chosen now (by index).
    fn shown_fields(&self) -> impl Iterator<Item = (usize, &Field)> {
        let applies = form::applicable(&self.fields);
        self.fields.iter().enumerate().filter(move |(i, _)| applies[*i])
    }

    pub fn advanced_count(&self) -> usize {
        self.shown_fields().filter(|(_, f)| f.option.advanced).count()
    }

    pub fn rows(&self, show_advanced: bool) -> Vec<Row> {
        let mut rows = Vec::new();
        if !self.editing() {
            rows.push(Row::Name);
        }
        for (i, f) in self.shown_fields() {
            if !f.option.advanced {
                rows.push(Row::Field(i));
            }
        }
        if show_advanced {
            for (i, f) in self.shown_fields() {
                if f.option.advanced {
                    rows.push(Row::Field(i));
                }
            }
        } else if self.advanced_count() > 0 {
            rows.push(Row::Advanced);
        }
        rows
    }

    /// Whether a text field has the keys: characters typed (or pasted) are text.
    pub fn editing_text(&self) -> bool {
        let text = |w: &Widget| !matches!(w, Widget::Toggle | Widget::Choice(_));
        match &self.step {
            Step::Type { .. } | Step::TokenPaste { .. } => true,
            Step::Question { field, .. } => text(&field.widget),
            Step::Form {
                cursor,
                show_advanced,
                help_open: false,
            } => {
                let rows = self.rows(*show_advanced);
                match rows.get((*cursor).min(rows.len().saturating_sub(1))) {
                    Some(Row::Name) => true,
                    Some(Row::Field(i)) => text(&self.fields[*i].widget),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// The step shown, looking through a pending "Cancel?".
    pub fn effective_step(&self) -> &Step {
        match &self.step {
            Step::ConfirmCancel { back } => back,
            s => s,
        }
    }

    fn validate_name(&mut self) {
        self.name_error = form::validate_name_busy(&self.name.value, &self.existing, &self.busy).err();
    }

    fn form_step(&self) -> Step {
        Step::Form {
            cursor: self.last_form.0,
            show_advanced: self.last_form.1,
            help_open: false,
        }
    }
}

/// `kind`, or `kind2`, `kind3`… whichever is free.
pub fn suggest_name(kind: &str, existing: &[String]) -> String {
    let taken = |n: &str| existing.iter().any(|e| e == n);
    if !taken(kind) {
        return kind.to_string();
    }
    (2..).map(|i| format!("{kind}{i}")).find(|n| !taken(n)).unwrap()
}

/// The account names in a list of remotes.
fn named(remotes: &[crate::backend::RemoteInfo]) -> Vec<String> {
    remotes
        .iter()
        .filter_map(|r| match &r.remote {
            Remote::Named(n) => Some(n.clone()),
            Remote::Local => None,
        })
        .collect()
}

/// `name`, quoted for a shell when it needs it.
fn shell_quote(name: &str) -> String {
    if name.chars().all(|c| c.is_alphanumeric() || "_-.+@".contains(c)) {
        name.to_string()
    } else {
        format!("\"{name}\"")
    }
}

fn one(key: &str, value: &str, obscure: bool) -> ConfigParams {
    ConfigParams {
        values: BTreeMap::from([(key.to_string(), value.to_string())]),
        obscure,
    }
}

fn plain(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

fn ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

impl App {
    fn wizard(&mut self) -> Option<&mut AccountWizard> {
        match self.overlay.as_mut() {
            Some(Overlay::Account(w)) => Some(w),
            _ => None,
        }
    }

    fn account_names(&self) -> Vec<String> {
        self.sidebar
            .remotes
            .iter()
            .filter_map(|r| match &r.remote {
                Remote::Named(n) => Some(n.clone()),
                Remote::Local => None,
            })
            .collect()
    }

    /// The named account under the sidebar cursor, or a message for the local disk.
    fn selected_account(&mut self) -> Option<(String, String)> {
        let r = self.sidebar.current()?.clone();
        match r.remote {
            Remote::Named(n) => Some((n, r.kind)),
            Remote::Local => {
                self.info(LOCAL_REFUSED, MessageKind::Error);
                None
            }
        }
    }

    fn transfers_warning(&self, name: &str) -> Option<String> {
        let r = Remote::Named(name.to_string());
        let n = self
            .store
            .running()
            .filter(|t| t.spec.src_dir.remote == r || t.spec.dst_dir.remote == r)
            .count();
        match n {
            0 => None,
            1 => Some(format!(
                "1 transfer is using {name}; changing it may make them fail."
            )),
            n => Some(format!(
                "{n} transfers are using {name}; changing it may make them fail."
            )),
        }
    }

    /// A new wizard over the app, with its own id.
    fn open_wizard(&mut self, mode: WizardMode) -> Option<u64> {
        if self.wizard().is_some() {
            return None;
        }
        let name = match &mode {
            WizardMode::Add => String::new(),
            WizardMode::Edit { name, .. } | WizardMode::Reauth { name, .. } => name.clone(),
        };
        self.next_wizard += 1;
        let id = self.next_wizard;
        let mut w = AccountWizard::new(id, mode, self.account_names(), self.over_ssh);
        w.busy = self.cleaning.iter().cloned().collect();
        w.busy.sort();
        if !name.is_empty() {
            w.transfers_warning = self.transfers_warning(&name);
        }
        self.overlay = Some(Overlay::Account(Box::new(w)));
        Some(id)
    }

    /// Run `fut` for the open wizard; its answer comes back tagged with the wizard's id.
    fn spawn_wizard<F>(&mut self, fut: F)
    where
        F: std::future::Future<Output = WizardEvent> + Send + 'static,
    {
        let Some(id) = self.wizard().map(|w| w.id) else {
            return;
        };
        self.spawn(async move { AppEvent::Wizard { id, ev: fut.await } });
    }

    pub(crate) fn open_add_account(&mut self) {
        if self.open_wizard(WizardMode::Add).is_none() {
            return;
        }
        let backend = self.backend.clone();
        self.spawn_wizard(async move {
            let result = backend.providers().await;
            let names = backend.remotes().await.map(|r| named(&r));
            WizardEvent::Providers { result, names }
        });
    }

    pub(crate) fn open_edit_account(&mut self) {
        if self.wizard().is_some() {
            return;
        }
        let Some((name, kind)) = self.selected_account() else {
            return;
        };
        self.open_wizard(WizardMode::Edit {
            name: name.clone(),
            kind,
        });
        let backend = self.backend.clone();
        self.spawn_wizard(async move {
            let result = match backend.providers().await {
                Ok(p) => backend.remote_config(&name).await.map(|c| (p, c)),
                Err(e) => Err(e),
            };
            WizardEvent::RemoteConfig(result)
        });
    }

    pub(crate) fn open_reauth(&mut self) {
        if self.wizard().is_some() {
            return;
        }
        let Some((name, kind)) = self.selected_account() else {
            return;
        };
        self.open_wizard(WizardMode::Reauth { name, kind });
        let backend = self.backend.clone();
        self.spawn_wizard(async move {
            WizardEvent::Providers {
                result: backend.providers().await,
                names: Ok(Vec::new()),
            }
        });
    }

    // ---- events -------------------------------------------------------------

    pub(super) fn account_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Wizard { id, ev } => self.wizard_event(id, ev),
            AppEvent::AccountDeleted { name, result, quiet } => self.account_deleted(name, result, quiet),
            AppEvent::RemoveChecked { name, kind, result } => self.remove_checked(name, kind, result),
            _ => {}
        }
    }

    fn wizard_event(&mut self, id: u64, ev: WizardEvent) {
        // The answer to the login poll in flight, whichever wizard is open now.
        if let WizardEvent::Job { jobid, .. } = &ev
            && self.polling_job == Some((id, *jobid))
        {
            self.polling_job = None;
        }
        if self.wizard().map(|w| w.id) != Some(id) {
            return self.stale_event(id, ev);
        }
        // A "Cancel?" on screen: the answer still lands on the step it was meant for,
        // and the question stays.
        let confirming = match self.wizard() {
            Some(w) => match std::mem::replace(&mut w.step, Step::Loading) {
                Step::ConfirmCancel { back } => {
                    w.step = *back;
                    true
                }
                other => {
                    w.step = other;
                    false
                }
            },
            None => false,
        };
        match ev {
            WizardEvent::Providers { result, names } => self.providers_loaded(result, names),
            WizardEvent::RemoteConfig(result) => self.edit_loaded(result),
            WizardEvent::Step(result) => {
                if self.wizard().is_some_and(|w| w.step == Step::Sending) {
                    match result {
                        Ok(step) => self.on_step(step),
                        Err(e) => self.wizard_failed(e),
                    }
                }
            }
            WizardEvent::JobStarted(result) => self.job_started(result),
            WizardEvent::Job { jobid, result } => self.job_polled(jobid, result),
            WizardEvent::NameTaken => self.name_taken(),
            WizardEvent::NotSent(e) => {
                if let Some(w) = self.wizard()
                    && w.step == Step::Sending
                {
                    // Nothing reached rclone: there is nothing of ours to remove.
                    w.created = false;
                    let e = match e {
                        BackendError::Disconnected => e,
                        e => BackendError::Other(format!("Could not check that the name is free: {e}")),
                    };
                    self.wizard_failed(e);
                }
            }
        }
        if confirming
            && let Some(w) = self.wizard()
            && w.id == id
            && !matches!(w.step, Step::ConfirmCancel { .. })
        {
            w.step = Step::ConfirmCancel {
                back: Box::new(w.step.clone()),
            };
        }
    }

    /// An answer for a wizard closed meanwhile. If it was an unfinished add, rclone may
    /// just have written the account again: remove it (unless nothing was created).
    fn stale_event(&mut self, id: u64, ev: WizardEvent) {
        let orphan = self.orphans.remove(&id);
        match ev {
            WizardEvent::JobStarted(Ok(job)) => {
                self.stop_job_quietly(job);
                if let Some(name) = orphan {
                    self.delete_account_quietly(name);
                }
            }
            WizardEvent::Step(_) | WizardEvent::JobStarted(Err(_)) => {
                if let Some(name) = orphan {
                    self.delete_account_quietly(name);
                }
            }
            // Nothing was created: nothing to remove.
            WizardEvent::NameTaken | WizardEvent::NotSent(_) => {
                if let Some(name) = orphan {
                    self.cleaned(&name);
                }
            }
            // Not an answer that can write anything: keep waiting for the one that can.
            _ => {
                if let Some(name) = orphan {
                    self.orphans.insert(id, name);
                }
            }
        }
    }

    fn name_taken(&mut self) {
        let Some(w) = self.wizard() else { return };
        w.created = false;
        w.existing.push(w.name.value.clone());
        w.validate_name();
        w.last_form.0 = 0;
        w.step = w.form_step();
    }

    fn providers_loaded(
        &mut self,
        result: Result<Vec<Provider>, BackendError>,
        names: Result<Vec<String>, BackendError>,
    ) {
        let Some(w) = self.wizard() else { return };
        if w.step != Step::Loading {
            return;
        }
        if let Ok(names) = names
            && w.mode == WizardMode::Add
        {
            w.existing = names;
        }
        match result {
            Err(e) => {
                self.overlay = None;
                self.info(format!("Could not load account types: {e}"), MessageKind::Error);
                if e == BackendError::Disconnected {
                    self.start_reconnect();
                }
            }
            Ok(all) => {
                w.providers = form::provider_list(&all);
                if let WizardMode::Reauth { name, kind } = w.mode.clone() {
                    w.provider = all.iter().find(|p| p.name == kind).cloned();
                    if !w.provider.as_ref().is_some_and(form::is_oauth) {
                        self.overlay = None;
                        return self.info(format!("{name} doesn't use a browser login"), MessageKind::Error);
                    }
                    w.step = Step::Sending;
                    let backend = self.backend.clone();
                    self.spawn_wizard(async move {
                        WizardEvent::Step(
                            backend
                                .config_update(&name, one("config_refresh_token", "true", false))
                                .await,
                        )
                    });
                } else {
                    w.step = Step::Type {
                        query: LineInput::default(),
                        cursor: 0,
                    };
                }
            }
        }
    }

    fn edit_loaded(&mut self, result: Result<(Vec<Provider>, BTreeMap<String, String>), BackendError>) {
        let Some(w) = self.wizard() else { return };
        if w.step != Step::Loading {
            return;
        }
        let (all, config) = match result {
            Ok(r) => r,
            Err(e) => {
                self.overlay = None;
                return self.backend_error(e);
            }
        };
        let kind = w.kind();
        let Some(p) = all.iter().find(|p| p.name == kind).cloned() else {
            self.overlay = None;
            return self.info(format!("Unknown account type {kind}"), MessageKind::Error);
        };
        w.providers = form::provider_list(&all);
        w.fields = form::fields_for(&p, Some(&config));
        w.provider = Some(p);
        w.step = Step::Form {
            cursor: 0,
            show_advanced: false,
            help_open: false,
        };
    }

    /// rclone answered: finish, ask the user, or answer by ourselves.
    fn on_step(&mut self, step: ConfigStep) {
        let Some(w) = self.wizard() else { return };
        if w.editing() {
            w.sent_any = true;
        }
        let (state, option, error) = match step {
            ConfigStep::Done => return self.wizard_done(),
            ConfigStep::Rejected { state, error } => return self.answer_rejected(state, error),
            ConfigStep::Question { state, option, error } => (state, option, error),
        };
        let auto = form::auto_answer(
            &option.name,
            w.editing(),
            w.over_ssh,
            form::client_changed(&w.fields),
        );
        match auto {
            Some(AutoAnswer::Local(local)) => {
                let local = local != w.local_flipped;
                w.local_state = Some(state.clone());
                w.auto_note = Some(if local {
                    "Using this machine's browser".into()
                } else {
                    "Over SSH: paste a token from another machine".into()
                });
                self.answer_local(state, local);
            }
            Some(AutoAnswer::RefreshToken(refresh)) => self.send_answer(
                &state,
                "config_refresh_token",
                if refresh { "true" } else { "false" },
                false,
            ),
            None if option.name == "config_token" => {
                let command = form::authorize_command(&option.help, &w.kind());
                w.step = Step::TokenPaste {
                    state,
                    command,
                    input: SecretInput::default(),
                    error,
                };
            }
            None if option.name == "config_shared_client_id" => {
                w.step = Step::DriveWarning {
                    state,
                    help: option.help,
                };
            }
            None => {
                w.step = Step::Question {
                    state,
                    field: Box::new(Field::new(option, None)),
                    error,
                };
            }
        }
    }

    /// Show the question again, with rclone's reason and the answer kept to fix it.
    fn answer_rejected(&mut self, new_state: String, reason: String) {
        let Some(w) = self.wizard() else { return };
        let Some(mut asked) = w.asked.take() else {
            return self.wizard_failed(BackendError::Rclone(reason));
        };
        match &mut asked {
            Step::Question { state, error, .. } | Step::TokenPaste { state, error, .. } => {
                *state = new_state;
                *error = Some(reason);
            }
            _ => return self.wizard_failed(BackendError::Rclone(reason)),
        }
        w.step = asked;
    }

    /// `config_is_local`: through this machine's browser (an rc job that waits), or not.
    fn answer_local(&mut self, state: String, local: bool) {
        let Some(w) = self.wizard() else { return };
        let name = w.account_name();
        if !local {
            return self.send_answer(&state, "config_is_local", "false", false);
        }
        w.step = Step::WaitingBrowser {
            jobid: None,
            since: Instant::now(),
        };
        let backend = self.backend.clone();
        self.spawn_wizard(async move {
            WizardEvent::JobStarted(backend.config_continue_job(&name, &state, "true", false).await)
        });
    }

    fn job_started(&mut self, result: Result<u64, BackendError>) {
        let Some(w) = self.wizard() else { return };
        match (&mut w.step, result) {
            (Step::WaitingBrowser { jobid, .. }, Ok(id)) => *jobid = Some(id),
            (Step::WaitingBrowser { .. }, Err(BackendError::LocalOAuthUnsupported)) => {
                let Some(state) = w.local_state.clone() else {
                    return self.wizard_failed(BackendError::LocalOAuthUnsupported);
                };
                w.local_flipped = !w.over_ssh;
                w.auto_note = Some("Browser login needs rclone 1.75+: paste an authorization token".into());
                self.answer_local(state, false);
            }
            (_, Ok(id)) => self.stop_job_quietly(id),
            (_, Err(e)) => self.wizard_failed(e),
        }
    }

    fn job_polled(
        &mut self,
        jobid: u64,
        result: Result<Option<Result<ConfigStep, BackendError>>, BackendError>,
    ) {
        let waiting = self
            .wizard()
            .is_some_and(|w| matches!(w.step, Step::WaitingBrowser { jobid: Some(j), .. } if j == jobid));
        if !waiting {
            return;
        }
        match result {
            Ok(None) => {}
            Ok(Some(Ok(step))) => {
                if let Some(w) = self.wizard() {
                    w.step = Step::Sending;
                }
                self.on_step(step);
            }
            Ok(Some(Err(e))) | Err(e) => self.wizard_failed(e),
        }
    }

    /// Ask rclone how a browser login is going (from the tick, at most one at a time).
    pub(super) fn poll_login(&mut self) {
        let Some(w) = self.wizard() else { return };
        let wizard = w.id;
        let Step::WaitingBrowser {
            jobid: Some(id),
            since,
        } = *w.effective_step()
        else {
            return;
        };
        if since.elapsed() > LOGIN_TIMEOUT {
            self.stop_job_quietly(id);
            return self.wizard_failed(BackendError::Other("Timed out waiting for the browser".into()));
        }
        // A poll of a closed wizard (still unanswered) does not hold this one back.
        let busy = self.polling_job == Some((wizard, id));
        let recent = self
            .last_job_poll
            .is_some_and(|(w, t)| w == wizard && t.elapsed() < JOB_POLL_EVERY);
        if busy || recent {
            return;
        }
        self.polling_job = Some((wizard, id));
        self.last_job_poll = Some((wizard, Instant::now()));
        let backend = self.backend.clone();
        self.spawn_wizard(async move {
            WizardEvent::Job {
                jobid: id,
                result: backend.config_job(id).await,
            }
        });
    }

    fn send_answer(&mut self, state: &str, _question: &str, answer: &str, obscure: bool) {
        let Some(w) = self.wizard() else { return };
        let name = w.account_name();
        w.asked = Some(std::mem::replace(&mut w.step, Step::Sending));
        let backend = self.backend.clone();
        let (state, answer) = (state.to_string(), answer.to_string());
        self.spawn_wizard(async move {
            WizardEvent::Step(backend.config_continue(&name, &state, &answer, obscure).await)
        });
    }

    /// A request failed: say so, and leave nothing half-made behind.
    fn wizard_failed(&mut self, e: BackendError) {
        let disconnected = e == BackendError::Disconnected;
        let Some(w) = self.wizard() else { return };
        let mut message = e.to_string();
        if w.editing() && w.sent_any {
            message.push_str(". Some changes were saved before the error");
        }
        let orphan = (!w.editing() && w.created && !w.existed_before).then(|| w.account_name());
        w.created = false;
        let back = match w.mode {
            WizardMode::Reauth { .. } => Step::Loading,
            _ => w.form_step(),
        };
        w.step = Step::Failed {
            message,
            back: Box::new(back),
        };
        if let Some(name) = orphan {
            if disconnected {
                // rclone is not there to ask: clean up once it is back.
                self.pending_deletes.push(name);
            } else {
                self.delete_account_quietly(name);
            }
        }
        if disconnected {
            self.start_reconnect();
        }
    }

    fn wizard_done(&mut self) {
        let Some(Overlay::Account(w)) = self.overlay.take() else {
            return;
        };
        self.check_quit_when_idle();
        let name = w.account_name();
        let text = match w.mode {
            WizardMode::Add => format!("Account {name} added"),
            WizardMode::Edit { .. } => format!("{name} updated"),
            WizardMode::Reauth { .. } => format!("{name} re-authorised"),
        };
        let remote = Remote::Named(name);
        self.sidebar.about.remove(&remote);
        self.sidebar.about_failed.remove(&remote);
        self.sidebar.about_pending.remove(&remote);
        self.pending_select = Some(remote);
        let backend = self.backend.clone();
        self.spawn(async move { AppEvent::Remotes(backend.remotes().await) });
        self.info(text, MessageKind::Success);
    }

    fn delete_account_quietly(&mut self, name: String) {
        self.cleaning.insert(name.clone());
        let backend = self.backend.clone();
        self.spawn(async move {
            let result = backend.config_delete(&name).await;
            AppEvent::AccountDeleted {
                name,
                result,
                quiet: true,
            }
        });
    }

    fn stop_job_quietly(&mut self, id: u64) {
        let backend = self.backend.clone();
        self.spawn(async move {
            let _ = backend.stop_job(id).await;
            AppEvent::JobStopped
        });
    }

    fn account_deleted(&mut self, name: String, result: Result<(), BackendError>, quiet: bool) {
        match (result, quiet) {
            (Ok(()), true) => self.cleaned(&name),
            (Err(BackendError::Disconnected), true) => self.pending_deletes.push(name),
            (Err(e), true) => {
                self.cleaned(&name);
                self.info(
                    format!("Could not remove the unfinished account {name}: {e}"),
                    MessageKind::Error,
                );
            }
            (result, false) => self.account_removed(name, result),
        }
    }

    // ---- remove -------------------------------------------------------------

    /// `dd` in the sidebar: find what depends on the account, then confirm.
    pub(crate) fn ask_remove_account(&mut self) {
        let Some((name, kind)) = self.selected_account() else {
            return;
        };
        let backend = self.backend.clone();
        self.spawn(async move {
            // Asked afresh: accounts may have been made outside the app.
            let names: Vec<String> = match backend.remotes().await {
                Ok(r) => r
                    .into_iter()
                    .filter_map(|r| match r.remote {
                        Remote::Named(n) => Some(n),
                        Remote::Local => None,
                    })
                    .collect(),
                Err(e) => {
                    return AppEvent::RemoveChecked {
                        name,
                        kind,
                        result: Err(e),
                    };
                }
            };
            let mut configs = Vec::with_capacity(names.len());
            let mut result = Ok(());
            for n in names {
                match backend.remote_config(&n).await {
                    Ok(c) => configs.push((n, c)),
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            let result = result.map(|()| form::dependents(&name, &configs));
            AppEvent::RemoveChecked { name, kind, result }
        });
    }

    fn remove_checked(&mut self, name: String, kind: String, result: Result<Vec<String>, BackendError>) {
        let deps = match result {
            Ok(d) => d,
            Err(e) => return self.backend_error(e),
        };
        // Something else opened meanwhile (another dialog, a wizard): leave it be.
        if self.overlay.is_some() {
            return;
        }
        let used: Vec<Remote> = std::iter::once(&name)
            .chain(&deps)
            .map(|n| Remote::Named(n.clone()))
            .collect();
        let busy = self
            .store
            .running()
            .any(|t| used.contains(&t.spec.src_dir.remote) || used.contains(&t.spec.dst_dir.remote));
        if busy {
            return self.info(
                format!("{name} is in use by running transfers"),
                MessageKind::Error,
            );
        }
        let description = crate::ui::text::remote_kind_label(&kind);
        let mut lines = vec![
            format!("Remove account {name} ({description})?"),
            "This only deletes it from rclone's config; the files in the account are untouched.".into(),
        ];
        if !deps.is_empty() {
            lines.push(format!(
                "Also used by: {} — they will stop working.",
                deps.join(", ")
            ));
        }
        self.overlay = Some(Overlay::Confirm {
            title: "Remove account".into(),
            lines,
            commands: vec![format!("rclone config delete {}", shell_quote(&name))],
            danger: true,
            on_yes: super::overlay::PendingOp::RemoveAccount { name },
        });
    }

    pub(super) fn remove_account(&mut self, name: String) {
        let backend = self.backend.clone();
        self.spawn_op(async move {
            let result = backend.config_delete(&name).await;
            AppEvent::AccountDeleted {
                name,
                result,
                quiet: false,
            }
        });
    }

    pub(super) fn account_removed(&mut self, name: String, result: Result<(), BackendError>) {
        self.ops_in_flight = self.ops_in_flight.saturating_sub(1);
        self.check_quit_when_idle();
        if let Err(e) = result {
            return self.backend_error(e);
        }
        let remote = Remote::Named(name.clone());
        self.sidebar.remotes.retain(|r| r.remote != remote);
        self.sidebar.about.remove(&remote);
        self.sidebar.about_failed.remove(&remote);
        self.sidebar.about_pending.remove(&remote);
        self.sidebar.move_by(0);
        self.listings.forget_remote(&remote);
        self.scans.retain(|root, _| root.remote != remote);
        if self.scan.as_ref().is_some_and(|s| s.tree.root.remote == remote) {
            self.cancel_scan();
        }
        if self.pane.loc.remote == remote {
            let home = self.home.clone();
            self.navigate(home, None);
        }
        if self.other.as_ref().is_some_and(|o| o.loc.remote == remote) {
            let home = self.home.clone();
            self.with_other(|a| a.request_listing(home, None));
        }
        if self.right_loc.as_ref().is_some_and(|l| l.remote == remote) {
            self.right_loc = None;
        }
        if self.tab == super::Tab::Space {
            self.sync_space();
        }
        self.info(format!("Account {name} removed"), MessageKind::Success);
    }

    /// A clean-up ended: the name is free again (even in a wizard that read the list
    /// of accounts before it ended).
    fn cleaned(&mut self, name: &str) {
        if self.orphans.values().any(|n| n == name) || self.pending_deletes.iter().any(|n| n == name) {
            return;
        }
        self.cleaning.remove(name);
        if let Some(w) = self.wizard() {
            w.busy.retain(|b| b != name);
            if w.mode == WizardMode::Add {
                w.existing.retain(|e| e != name);
                if !w.name.value.is_empty() {
                    w.validate_name();
                }
            }
        }
    }

    /// Unfinished adds left while rclone was unreachable.
    pub(super) fn retry_pending_deletes(&mut self) {
        for name in std::mem::take(&mut self.pending_deletes) {
            self.delete_account_quietly(name);
        }
    }

    /// Close the wizard; an unfinished add is removed from rclone.
    fn cancel_wizard(&mut self) {
        let Some(Overlay::Account(w)) = self.overlay.take() else {
            return;
        };
        self.check_quit_when_idle();
        let back = w.effective_step().clone();
        if let Step::WaitingBrowser { jobid: Some(id), .. } = back {
            // A stopped login writes nothing more (checked against rclone v1.76).
            self.stop_job_quietly(id);
        }
        if w.editing() || !w.created || w.existed_before {
            return;
        }
        let name = w.account_name();
        match back {
            // The create may not have happened (or the name was taken): delete only
            // once its answer says what rclone did.
            Step::Sending => {
                self.cleaning.insert(name.clone());
                self.orphans.insert(w.id, name);
            }
            // The login job is starting: stop it when it does, and delete again then.
            Step::WaitingBrowser { jobid: None, .. } => {
                self.orphans.insert(w.id, name.clone());
                self.delete_account_quietly(name);
            }
            _ => self.delete_account_quietly(name),
        }
    }

    // ---- keys ---------------------------------------------------------------

    pub(super) fn account_key(&mut self, key: KeyEvent) {
        let Some(w) = self.wizard() else { return };
        match w.step.clone() {
            Step::Loading | Step::Sending if key.code == KeyCode::Esc => {
                if w.created || w.sent_any {
                    w.step = Step::ConfirmCancel {
                        back: Box::new(w.step.clone()),
                    };
                } else {
                    self.cancel_wizard();
                }
            }
            Step::Loading | Step::Sending => {}
            Step::Type { query, cursor } => self.type_key(key, query, cursor),
            Step::Form {
                cursor,
                show_advanced,
                help_open,
            } => self.form_key(key, cursor, show_advanced, help_open),
            Step::Question { state, field, error } => self.question_key(key, state, field, error),
            Step::WaitingBrowser { .. } => self.waiting_key(key),
            Step::TokenPaste {
                state,
                command,
                input,
                error,
            } => self.token_key(key, state, command, input, error),
            Step::DriveWarning { state, .. } => self.drive_warning_key(key, state),
            Step::ConfirmCancel { back } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => self.cancel_wizard(),
                KeyCode::Char('n') | KeyCode::Esc => w.step = *back,
                _ => {}
            },
            Step::Failed { back, .. } => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                    if *back == Step::Loading {
                        self.overlay = None;
                    } else {
                        w.step = *back;
                    }
                }
            }
        }
    }

    /// Esc on a step where rclone has already been told something.
    fn ask_cancel(&mut self) {
        if let Some(w) = self.wizard() {
            w.step = Step::ConfirmCancel {
                back: Box::new(w.step.clone()),
            };
        }
    }

    fn type_key(&mut self, key: KeyEvent, mut query: LineInput, mut cursor: usize) {
        let Some(w) = self.wizard() else { return };
        let n = w.filtered(&query.value).len();
        match key.code {
            KeyCode::Esc => return self.cancel_wizard(),
            KeyCode::Down => cursor = (cursor + 1).min(n.saturating_sub(1)),
            KeyCode::Char('n') if ctrl(&key, 'n') => cursor = (cursor + 1).min(n.saturating_sub(1)),
            KeyCode::Up => cursor = cursor.saturating_sub(1),
            KeyCode::Char('p') if ctrl(&key, 'p') => cursor = cursor.saturating_sub(1),
            KeyCode::Enter => {
                let Some(p) = w.filtered(&query.value).get(cursor).map(|p| (*p).clone()) else {
                    return;
                };
                let same = w.provider.as_ref().is_some_and(|old| old.name == p.name);
                if !same {
                    w.fields = form::fields_for(&p, None);
                }
                if w.name.value.is_empty() || !same {
                    let name = suggest_name(&p.name, &w.existing);
                    w.name = LineInput::new(&name, name.chars().count());
                }
                w.existed_before = w.existing.contains(&w.name.value);
                w.provider = Some(p);
                w.validate_name();
                w.last_form = (0, false);
                w.step = w.form_step();
                return;
            }
            _ => {
                if query.handle(key) == InputOutcome::Changed {
                    cursor = 0;
                }
            }
        }
        w.step = Step::Type { query, cursor };
    }

    fn form_key(&mut self, key: KeyEvent, mut cursor: usize, mut show_advanced: bool, mut help_open: bool) {
        let Some(w) = self.wizard() else { return };
        let rows = w.rows(show_advanced);
        let last = rows.len().saturating_sub(1);
        cursor = cursor.min(last);
        let row = rows.get(cursor).copied();
        let text_row = match row {
            Some(Row::Name) => true,
            Some(Row::Field(i)) => !matches!(w.fields[i].widget, Widget::Toggle | Widget::Choice(_)),
            _ => false,
        };
        if help_open {
            match key.code {
                KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => {
                    help_open = false;
                    w.help_scroll = 0;
                }
                KeyCode::Down | KeyCode::Char('j') => w.help_scroll = w.help_scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => w.help_scroll = w.help_scroll.saturating_sub(1),
                _ => {}
            }
            w.step = Step::Form {
                cursor,
                show_advanced,
                help_open,
            };
            return;
        }
        match key.code {
            KeyCode::Esc => {
                if w.editing() {
                    return self.cancel_wizard();
                }
                let query = w.provider.as_ref().map(|p| p.name.clone()).unwrap_or_default();
                w.step = Step::Type {
                    query: LineInput::new(&query, query.chars().count()),
                    cursor: 0,
                };
                return;
            }
            _ if ctrl(&key, 's') => return self.submit_form(cursor, show_advanced),
            // Enter on the last field (or on the advanced-options row after it) saves.
            KeyCode::Enter if cursor >= rows.iter().rposition(|r| *r != Row::Advanced).unwrap_or(0) => {
                return self.submit_form(cursor, show_advanced);
            }
            KeyCode::Enter | KeyCode::Down => cursor = (cursor + 1).min(last),
            _ if ctrl(&key, 'n') => cursor = (cursor + 1).min(last),
            KeyCode::Up => cursor = cursor.saturating_sub(1),
            _ if ctrl(&key, 'p') => cursor = cursor.saturating_sub(1),
            KeyCode::Tab => {
                show_advanced = !show_advanced;
                cursor = cursor.min(w.rows(show_advanced).len().saturating_sub(1));
            }
            KeyCode::F(1) => help_open = true,
            KeyCode::Char('j') if !text_row && plain(&key) => cursor = (cursor + 1).min(last),
            KeyCode::Char('k') if !text_row && plain(&key) => cursor = cursor.saturating_sub(1),
            KeyCode::Char('?') if !text_row => help_open = true,
            _ => match row {
                Some(Row::Advanced) => {
                    if matches!(key.code, KeyCode::Char(' ') | KeyCode::Char('l') | KeyCode::Right) {
                        show_advanced = true;
                    }
                }
                Some(Row::Name) => {
                    if w.name.handle(key) == InputOutcome::Changed {
                        w.validate_name();
                        w.existed_before = w.existing.contains(&w.name.value);
                    }
                }
                Some(Row::Field(i)) => {
                    let names = w.existing.clone();
                    if edit_field(&mut w.fields[i], key, &names) {
                        w.form_error = None;
                    }
                }
                None => {}
            },
        }
        w.step = Step::Form {
            cursor,
            show_advanced,
            help_open,
        };
    }

    fn submit_form(&mut self, cursor: usize, show_advanced: bool) {
        let Some(w) = self.wizard() else { return };
        w.last_form = (cursor, show_advanced);
        let editing = w.editing();
        if !editing {
            w.validate_name();
            if w.name_error.is_some() {
                w.last_form.0 = 0;
                w.step = w.form_step();
                return;
            }
        }
        let applies = form::applicable(&w.fields);
        if let Some(i) = (0..w.fields.len()).find(|&i| applies[i] && w.fields[i].missing_required(editing)) {
            let advanced = w.fields[i].option.advanced || show_advanced;
            let at = w
                .rows(advanced)
                .iter()
                .position(|r| *r == Row::Field(i))
                .unwrap_or(0);
            w.form_error = Some(format!("{} is required", w.fields[i].option.name));
            w.last_form = (at, advanced);
            w.step = w.form_step();
            return;
        }
        w.form_error = None;
        let params = form::params_for(&w.fields, editing);
        let name = w.account_name();
        let kind = w.kind();
        if editing && params.values.is_empty() {
            self.overlay = None;
            return self.info(format!("{name}: nothing changed"), MessageKind::Info);
        }
        w.step = Step::Sending;
        if !editing {
            w.created = true;
        }
        let backend = self.backend.clone();
        self.spawn_wizard(async move {
            if editing {
                return WizardEvent::Step(backend.config_update(&name, params).await);
            }
            // rclone's create replaces an account of the same name: check right before,
            // and send nothing when the check itself fails.
            let target = Remote::Named(name.clone());
            match backend.remotes().await {
                Ok(r) if r.iter().any(|r| r.remote == target) => return WizardEvent::NameTaken,
                Ok(_) => {}
                Err(e) => return WizardEvent::NotSent(e),
            }
            WizardEvent::Step(backend.config_create(&name, &kind, params).await)
        });
    }

    fn question_key(&mut self, key: KeyEvent, state: String, mut field: Box<Field>, error: Option<String>) {
        if key.code == KeyCode::Esc {
            return self.ask_cancel();
        }
        if key.code == KeyCode::Enter {
            let value = field.current();
            let password = field.widget == Widget::Password;
            return self.send_answer(&state, &field.option.name.clone(), &value, password);
        }
        let Some(w) = self.wizard() else { return };
        let names = w.existing.clone();
        let error = if edit_field(&mut field, key, &names) {
            None
        } else {
            error
        };
        w.step = Step::Question { state, field, error };
    }

    fn waiting_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.ask_cancel(),
            KeyCode::Char('L') => self.flip_local(),
            _ => {}
        }
    }

    /// The browser/paste guess was wrong: ask rclone the other way.
    fn flip_local(&mut self) {
        let Some(w) = self.wizard() else { return };
        let Some(state) = w.local_state.clone() else {
            return;
        };
        w.local_flipped = !w.local_flipped;
        if let Step::WaitingBrowser { jobid: Some(id), .. } = w.step {
            self.stop_job_quietly(id);
        }
        let Some(w) = self.wizard() else { return };
        let local = w.over_ssh == w.local_flipped;
        w.auto_note = Some(match (local, w.over_ssh) {
            (true, _) => "Using this machine's browser".into(),
            (false, true) => "Over SSH: paste a token from another machine".into(),
            (false, false) => "Paste a token from another machine".into(),
        });
        self.answer_local(state, local);
    }

    fn token_key(
        &mut self,
        key: KeyEvent,
        state: String,
        command: String,
        mut input: SecretInput,
        error: Option<String>,
    ) {
        // `L` and `y` are commands only while nothing is typed, and never when
        // they are part of a pasted token.
        let empty = input.value.is_empty() && !self.pasting;
        match key.code {
            KeyCode::Esc => return self.ask_cancel(),
            // rclone takes an empty answer as "done", leaving the account without a token.
            KeyCode::Enter if input.value.trim().is_empty() => {
                if let Some(w) = self.wizard() {
                    w.step = Step::TokenPaste {
                        state,
                        command,
                        input,
                        error: Some("Paste the token first".into()),
                    };
                }
                return;
            }
            KeyCode::Enter => return self.send_answer(&state, "config_token", input.value.trim(), false),
            KeyCode::Char('L') if empty => return self.flip_local(),
            KeyCode::Char('y') if empty && plain(&key) => return self.copy_command(command),
            _ if ctrl(&key, 'y') => return self.copy_command(command),
            _ => {}
        }
        let Some(w) = self.wizard() else { return };
        let error = if input.handle(key) == InputOutcome::Changed {
            None
        } else {
            error
        };
        w.step = Step::TokenPaste {
            state,
            command,
            input,
            error,
        };
    }

    fn copy_command(&mut self, command: String) {
        self.clipboard_out = Some(command);
        self.info("Command copied", MessageKind::Info);
    }

    fn drive_warning_key(&mut self, key: KeyEvent, state: String) {
        let Some(w) = self.wizard() else { return };
        match key.code {
            KeyCode::Esc => self.ask_cancel(),
            KeyCode::Char('y') => self.send_answer(&state, "config_shared_client_id", "true", false),
            KeyCode::Char('n') if w.editing() => {
                self.send_answer(&state, "config_shared_client_id", "false", false)
            }
            KeyCode::Char('n') => {
                // Back to the form to fill in an own client; the account made so far goes.
                let name = w.account_name();
                let orphan = w.created && !w.existed_before;
                w.created = false;
                let cursor = w
                    .rows(false)
                    .iter()
                    .position(|r| matches!(r, Row::Field(i) if w.fields[*i].option.name == "client_id"));
                w.last_form = (cursor.unwrap_or(0), false);
                w.step = w.form_step();
                if orphan {
                    self.delete_account_quietly(name);
                }
            }
            _ => {}
        }
    }
}

/// Polls of a browser login are at least this far apart.
const JOB_POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(1);
/// The app stops waiting for a browser login after this long.
pub const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// `value` with its account (the last one, for `upstreams`) moved to the next of
/// `accounts`, then to the local disk (no `name:`), then round again. The path stays.
fn next_remote(value: &str, option: &str, accounts: &[String]) -> String {
    let (keep, last) = if option == "upstreams" {
        match value.rsplit_once(' ') {
            Some((head, last)) => (format!("{head} "), last),
            None => (String::new(), value),
        }
    } else {
        (String::new(), value)
    };
    let (account, path) = match last.split_once(':') {
        Some((a, p)) => (Some(a), p),
        None => (None, last),
    };
    // Positions: 0..n are the accounts, n is the local disk.
    let n = accounts.len();
    let at = match account {
        Some(a) => accounts.iter().position(|x| x == a).unwrap_or(n),
        None => n,
    };
    let next = (at + 1) % (n + 1);
    let one = if next == n {
        path.to_string()
    } else {
        format!("{}:{path}", accounts[next])
    };
    format!("{keep}{one}")
}

/// Apply a key to a field. Returns whether its value changed.
fn edit_field(f: &mut Field, key: KeyEvent, accounts: &[String]) -> bool {
    if ctrl(&key, 'd') {
        f.reset();
        return true;
    }
    match f.widget.clone() {
        Widget::Toggle | Widget::Choice(_) => match key.code {
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Char(' ') if plain(&key) => {
                f.cycle(1);
                true
            }
            KeyCode::Char('h') | KeyCode::Left if plain(&key) => {
                f.cycle(-1);
                true
            }
            _ => false,
        },
        widget => {
            if ctrl(&key, 'r') && widget == Widget::Password {
                f.reveal = !f.reveal;
                return false;
            }
            if ctrl(&key, 'o') && widget == Widget::RemotePath && !accounts.is_empty() {
                let v = next_remote(&f.input.value, &f.option.name, accounts);
                f.set_text(&v);
                return true;
            }
            if f.input.handle(key) == InputOutcome::Changed {
                let v = f.input.value.clone();
                f.set_text(&v);
                return true;
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_field(name: &str, value: &str) -> Field {
        let opt = crate::backend::ProviderOption {
            name: name.into(),
            kind: "string".into(),
            ..Default::default()
        };
        let mut f = Field::new(opt, None);
        f.set_text(value);
        f
    }

    fn ctrl_o() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)
    }

    #[test]
    fn ctrl_o_keeps_the_path_and_includes_local() {
        let accounts = vec!["personal".to_string(), "archive".to_string()];
        let mut f = remote_field("remote", "Secret");
        edit_field(&mut f, ctrl_o(), &accounts);
        assert_eq!(f.input.value, "personal:Secret", "typed path kept");
        edit_field(&mut f, ctrl_o(), &accounts);
        assert_eq!(f.input.value, "archive:Secret");
        edit_field(&mut f, ctrl_o(), &accounts);
        assert_eq!(f.input.value, "Secret", "then the local disk");
    }

    #[test]
    fn ctrl_o_on_upstreams_changes_the_last_one() {
        let accounts = vec!["personal".to_string(), "archive".to_string()];
        let mut f = remote_field("upstreams", "personal: archive:x");
        edit_field(&mut f, ctrl_o(), &accounts);
        assert_eq!(f.input.value, "personal: x");
    }

    #[test]
    fn suggested_names() {
        let e = vec!["mega".to_string(), "mega2".to_string()];
        assert_eq!(suggest_name("drive", &e), "drive");
        assert_eq!(suggest_name("mega", &e), "mega3");
    }
}
