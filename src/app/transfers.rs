//! Launching transfers, polling their progress, reconnecting, and the quit flow.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};

use super::ops::plural;
use super::overlay::{Overlay, PendingOp};
use super::{App, AppEvent, MessageKind};
use crate::backend::{BackendError, StartError};
use crate::input::keymap::Action;
use crate::transfer::spec::{JobsState, TransferKind, TransferProgress, TransferSpec};
use crate::transfer::store::{RecordState, TransferRecord};

/// Minimum time between two reconnection attempts.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

impl App {
    /// Hand `spec` to rclone. The record is saved first and every job id as soon as
    /// rclone accepts it, so an exit at any point leaves a record that can be reconciled.
    pub fn launch_transfer(&mut self, spec: TransferSpec) {
        let replaces = self.resuming.get(&spec.id).copied();
        self.store
            .push_starting(spec.clone(), replaces, jiff::Timestamp::now());
        self.save_store();
        self.launching += 1;
        self.launching_ids.insert(spec.id);
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let id = spec.id;
        self.spawn(async move {
            let on_job = move |jobid| {
                let _ = tx.send(AppEvent::TransferJob { id, jobid });
            };
            let result = backend.start_transfer(&spec, &on_job).await;
            AppEvent::TransferStarted { spec, result }
        });
    }

    /// rclone accepted one job of a transfer still being launched.
    pub(super) fn transfer_job(&mut self, id: u64, jobid: u64) {
        self.store.add_job(id, jobid);
        // Something of the resume runs: the record it replaces goes now, not later,
        // so the two can never be resumed side by side.
        if let Some(old) = self.store.get(id).and_then(|r| r.replaces) {
            self.store.remove(old);
            if let Some(r) = self.store.records.iter_mut().find(|r| r.spec.id == id) {
                r.replaces = None;
            }
        }
        self.save_store();
    }

    pub(super) fn transfer_started(&mut self, spec: TransferSpec, result: Result<Vec<u64>, StartError>) {
        self.launching = self.launching.saturating_sub(1);
        self.launching_ids.remove(&spec.id);
        self.resuming.remove(&spec.id);
        self.forget_listings(&spec);
        let title = spec.title();
        let id = spec.id;
        let (ids, error, uncertain) = match result {
            Ok(ids) => (ids, None, false),
            Err(StartError {
                started,
                error,
                uncertain,
            }) => (started, Some(error), uncertain),
        };
        if uncertain {
            // The answer was lost, not the request: rclone may be running more than we
            // heard of. The record stays Starting and is reconciled by its group (never
            // sent again: that could run it twice).
            for jobid in ids {
                self.store.add_job(id, jobid);
            }
            self.save_store();
            let error = error.unwrap_or(BackendError::Disconnected);
            self.info(
                format!("Not sure {title} started ({error}) — asking rclone"),
                MessageKind::Error,
            );
            if error == BackendError::Disconnected {
                self.start_reconnect();
            }
            self.check_quit_when_launched();
            return;
        }
        let confirmed = self.store.get(id).is_some_and(|r| !r.jobids.is_empty());
        if ids.is_empty() && !confirmed {
            // Nothing reached rclone: no record (a resumed one stays as it was).
            self.store.remove(id);
            self.save_store();
            let error = error.unwrap_or(BackendError::Other("nothing was started".into()));
            self.info(format!("Could not start {title}: {error}"), MessageKind::Error);
            if error == BackendError::Disconnected {
                self.start_reconnect();
            }
            self.check_quit_when_launched();
            self.check_quit_when_idle();
            return;
        }
        // Whatever started is running in rclone: track it, even if the rest failed.
        if let Some(old) = self.store.get(id).and_then(|r| r.replaces) {
            self.store.remove(old);
        }
        let units = self.backend.job_units(&spec).len();
        self.store.launched(id, ids, units);
        match &error {
            None => self.info(format!("Started {title}"), MessageKind::Info),
            Some(e) => {
                self.store.set_start_error(id, e.to_string());
                self.info(
                    format!("Only part of {title} started: {e} — resume it from T"),
                    MessageKind::Error,
                );
            }
        }
        if self.cancel_new {
            self.stop_transfer_id(id);
        }
        self.save_store();
        if error == Some(BackendError::Disconnected) {
            self.start_reconnect();
        }
        self.check_quit_when_launched();
    }

    /// Transfers left starting by an earlier run (the app exited mid-launch): ask
    /// rclone which of their jobs it accepted.
    fn reconcile_starting(&mut self) {
        let left: Vec<(u64, String)> = self
            .store
            .starting()
            .filter(|r| !self.launching_ids.contains(&r.spec.id) && !self.reconciling.contains(&r.spec.id))
            .map(|r| (r.spec.id, r.spec.group()))
            .collect();
        for (id, group) in left {
            self.reconciling.insert(id);
            let backend = self.backend.clone();
            self.spawn(async move {
                let result = backend.group_jobs(&group).await;
                AppEvent::Reconciled { id, result }
            });
        }
    }

    pub(super) fn reconciled(&mut self, id: u64, result: Result<Vec<u64>, BackendError>) {
        self.reconciling.remove(&id);
        let Some(rec) = self.store.get(id).filter(|r| r.state == RecordState::Starting) else {
            return;
        };
        let found = match result {
            Ok(found) => found,
            Err(BackendError::Disconnected) => return self.start_reconnect(),
            Err(e) => {
                // Tried again on the next tick; said once.
                if self.progress_errors.insert(id) {
                    self.info(
                        format!("Could not check an unfinished start: {e}"),
                        MessageKind::Error,
                    );
                }
                return;
            }
        };
        self.progress_errors.remove(&id);
        let spec = rec.spec.clone();
        let replaces = rec.replaces;
        let known = rec.jobids.len();
        if known == 0 && found.is_empty() {
            match replaces {
                // A resume that never got going: the record it would replace is still there.
                Some(_) => self.store.remove(id),
                None => self.store.never_started(
                    id,
                    "this transfer never reached rclone".into(),
                    jiff::Timestamp::now(),
                ),
            }
        } else {
            if let Some(old) = replaces {
                self.store.remove(old);
            }
            let units = self.backend.job_units(&spec).len();
            self.store.launched(id, found, units);
        }
        self.save_store();
        self.poll_transfers();
    }

    pub(super) fn save_store(&mut self) {
        if let Err(e) = self.store.save() {
            self.info(
                format!("Could not save transfer history: {e}"),
                MessageKind::Error,
            );
        }
    }

    /// Called once a second: ask for the progress of every running transfer.
    pub(super) fn poll_transfers(&mut self) {
        if self.reconnecting {
            return;
        }
        self.reconcile_starting();
        let running: Vec<(u64, String, Vec<u64>)> = self
            .store
            .running()
            .filter(|r| !self.polling.contains(&r.spec.id))
            .map(|r| (r.spec.id, r.spec.group(), r.jobids.clone()))
            .collect();
        for (id, group, jobids) in running {
            self.polling.insert(id);
            let backend = self.backend.clone();
            self.spawn(async move {
                let result = backend.transfer_progress(&group, &jobids).await;
                AppEvent::Progress { id, result }
            });
        }
    }

    pub(super) fn on_progress(&mut self, id: u64, result: Result<TransferProgress, BackendError>) {
        self.polling.remove(&id);
        let mut p = match result {
            Ok(p) => {
                self.progress_errors.remove(&id);
                p
            }
            Err(BackendError::Disconnected) => return self.start_reconnect(),
            Err(e) => {
                // Say it once per transfer, not every second.
                if self.progress_errors.insert(id) {
                    self.info(format!("Transfer status unavailable: {e}"), MessageKind::Error);
                }
                return;
            }
        };
        let Some(before) = self
            .store
            .records
            .iter()
            .find(|r| r.spec.id == id)
            .map(|r| r.state)
        else {
            return;
        };
        // Ending in a failure (or gone) while being stopped: the stop did it.
        let stopped =
            self.stopping.contains(&id) && matches!(p.state, JobsState::Failed(_) | JobsState::NotFound);
        if stopped {
            p.state = JobsState::Running;
        }
        self.store.apply(id, &p, jiff::Timestamp::now());
        if stopped {
            self.store.mark_stopped(id, jiff::Timestamp::now());
        }
        self.progress.insert(id, p);
        // Most answers change only the numbers: skip the copy below for those.
        let Some(rec) = self.store.records.iter().find(|r| r.spec.id == id) else {
            return;
        };
        if rec.state == before {
            return;
        }
        let rec = rec.clone();
        self.stopping.remove(&id);
        self.progress.remove(&id);
        self.save_store();
        self.forget_listings(&rec.spec);
        let title = rec.spec.title();
        match rec.state {
            RecordState::Done => self.info(format!("Done: {title}"), MessageKind::Success),
            RecordState::Failed => {
                let err = rec.error.unwrap_or_default();
                self.info(format!("Failed: {title}: {err}"), MessageKind::Error)
            }
            RecordState::Interrupted => self.info(format!("Interrupted: {title}"), MessageKind::Error),
            RecordState::Stopped => self.info(format!("Stopped: {title}"), MessageKind::Success),
            _ => {}
        }
        let mut dirs = vec![&rec.spec.dst_dir];
        if rec.spec.kind == TransferKind::Move {
            dirs.push(&rec.spec.src_dir);
        }
        self.refresh_showing(&dirs);
        // However it ended, it changed how much those accounts hold.
        for dir in dirs {
            self.refresh_about(&dir.remote);
        }
        self.check_quit_when_idle();
    }

    // ---- reconnect --------------------------------------------------------

    pub(super) fn start_reconnect(&mut self) {
        if self.reconnecting
            || self
                .last_reconnect
                .is_some_and(|t| t.elapsed() < RECONNECT_BACKOFF)
        {
            return;
        }
        let Some(f) = self.reconnect.clone() else {
            return self.info("Lost connection to rclone", MessageKind::Error);
        };
        self.reconnecting = true;
        self.last_reconnect = Some(Instant::now());
        self.info("Lost connection to rclone — reconnecting…", MessageKind::Error);
        self.spawn(async move { AppEvent::Reconnected(f().await) });
    }

    pub(super) fn reconnected(&mut self, result: Result<bool, String>) {
        self.reconnecting = false;
        self.polling.clear();
        // A stop sent to an rcd that went away may not have reached it: stop again.
        self.stopping.clear();
        match result {
            Ok(restarted) => {
                self.retry_pending_deletes();
                if restarted {
                    self.store.interrupt_all_running();
                    self.progress.clear();
                    self.save_store();
                    self.info(
                        "rclone was restarted — running transfers were interrupted (T to resume)",
                        MessageKind::Error,
                    );
                } else {
                    self.info("Reconnected to rclone", MessageKind::Info);
                }
                // Running jobs the new rcd does not know come back as NotFound → Interrupted.
                self.poll_transfers();
                self.refresh_pane();
                self.refresh_other();
            }
            Err(e) => self.info(format!("Could not restart rclone: {e}"), MessageKind::Error),
        }
    }

    // ---- quit -------------------------------------------------------------

    /// Transfers running or about to start.
    pub fn active_transfers(&self) -> usize {
        let live = self.store.records.iter().filter(|r| r.is_live()).count();
        // Launches have a Starting record already; the rest are pastes being checked.
        let checks = self.launching.saturating_sub(self.launching_ids.len());
        live + checks + self.ops_in_flight
    }

    /// Leave rclone running, but only once it has every transfer being launched and
    /// the app's own file operations are over: a delete of several items is sent one
    /// by one, and quitting earlier would drop what was not handed over yet.
    fn quit_in_background(&mut self) {
        self.quit_after(false);
    }

    /// Quit (stopping rclone when `shutdown`) once nothing the app itself drives is left.
    fn quit_after(&mut self, shutdown: bool) {
        self.quit_shutdown = shutdown;
        if self.waiting_on_app() {
            self.quit_when_launched = true;
            let what = if self.ops_in_flight > 0 {
                "the operations in progress end"
            } else {
                "rclone has every transfer"
            };
            self.info(format!("Quitting once {what} (Esc to stay)"), MessageKind::Info);
        } else {
            self.should_quit = true;
            self.shutdown_on_exit = shutdown;
        }
    }

    /// Work that ends if the app exits: launches being handed over (not when rclone is
    /// stopped anyway) and the app's own file operations.
    fn waiting_on_app(&self) -> bool {
        (self.launching > 0 && !self.quit_shutdown) || self.ops_in_flight > 0
    }

    pub(super) fn check_quit_when_launched(&mut self) {
        if self.quit_when_launched && !self.waiting_on_app() {
            self.quit_when_launched = false;
            self.should_quit = true;
            self.shutdown_on_exit = self.quit_shutdown;
        }
    }

    pub(super) fn request_quit(&mut self) {
        let running = self.active_transfers();
        if running == 0 {
            self.should_quit = true;
            self.shutdown_on_exit = true;
        } else {
            match self.on_quit {
                crate::config::OnQuit::Ask => {
                    self.overlay = Some(Overlay::Quit {
                        running,
                        ops: self.ops_in_flight,
                    })
                }
                crate::config::OnQuit::Background => self.quit_in_background(),
                crate::config::OnQuit::Wait => {
                    self.quit_when_idle = true;
                    self.info(
                        "Will quit when the transfers finish (Esc to stay)",
                        MessageKind::Info,
                    );
                }
            }
        }
    }

    pub(super) fn quit_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('w') => {
                self.overlay = None;
                self.quit_when_idle = true;
                self.info(
                    "Will quit when the transfers finish (Esc to stay)",
                    MessageKind::Info,
                );
            }
            KeyCode::Char('c') => {
                self.overlay = None;
                self.cancel_new = true;
                self.stop_all_running();
                // The stops, and any delete still going, are answered first.
                self.quit_after(true);
            }
            KeyCode::Char('b') => {
                self.overlay = None;
                self.quit_in_background();
            }
            KeyCode::Esc | KeyCode::Char('q') => self.overlay = None,
            _ => {}
        }
    }

    pub(super) fn check_quit_when_idle(&mut self) {
        // Never in the middle of making an account: it would stay half-made.
        let wizard_open = matches!(self.overlay, Some(super::overlay::Overlay::Account(_)));
        if self.quit_when_idle && self.active_transfers() == 0 && !wizard_open {
            self.should_quit = true;
            self.shutdown_on_exit = true;
        }
    }

    /// Ask rclone to stop a transfer. It stays running (and followed) until rclone
    /// says the stop worked: a refused or lost request changes nothing.
    pub(super) fn stop_transfer_id(&mut self, id: u64) {
        let Some(group) = self
            .store
            .running()
            .find(|r| r.spec.id == id)
            .map(|r| r.spec.group())
        else {
            return;
        };
        if !self.stopping.insert(id) {
            return;
        }
        let backend = self.backend.clone();
        self.spawn_op(async move {
            let result = backend.stop_transfer(&group).await;
            AppEvent::StopDone { id, result }
        });
    }

    pub(super) fn stop_done(&mut self, id: u64, result: Result<(), BackendError>) {
        self.ops_in_flight = self.ops_in_flight.saturating_sub(1);
        let running = self.store.running().any(|r| r.spec.id == id);
        match result {
            // rclone took the stop: what the jobs report next says how they ended
            // (a job that finished first ends as done, not stopped).
            Ok(()) if running => self.poll_transfers(),
            Ok(()) => {
                self.stopping.remove(&id);
            }
            Err(e) => {
                self.stopping.remove(&id);
                let title = self.store.get(id).map(|r| r.spec.title()).unwrap_or_default();
                self.info(
                    format!("Could not stop {title}: {e} — it is still running"),
                    MessageKind::Error,
                );
                if e == BackendError::Disconnected {
                    self.start_reconnect();
                }
            }
        }
        self.check_quit_when_idle();
    }

    /// Being stopped: the stop request has not answered yet.
    pub fn is_stopping(&self, id: u64) -> bool {
        self.stopping.contains(&id)
    }

    fn stop_all_running(&mut self) {
        let ids: Vec<u64> = self.store.running().map(|r| r.spec.id).collect();
        for id in ids {
            self.stop_transfer_id(id);
        }
        self.save_store();
    }

    // ---- transfers view ---------------------------------------------------

    /// Record ids in display order: running first, then newest first.
    pub fn transfer_order(&self) -> Vec<u64> {
        self.transfer_records().into_iter().map(|r| r.spec.id).collect()
    }

    /// The records themselves, in [`App::transfer_order`].
    pub fn transfer_records(&self) -> Vec<&TransferRecord> {
        let mut recs: Vec<_> = self.store.records.iter().collect();
        recs.sort_by(|a, b| {
            (b.state == RecordState::Running)
                .cmp(&(a.state == RecordState::Running))
                .then(b.started.cmp(&a.started))
                .then(b.spec.id.cmp(&a.spec.id))
        });
        recs
    }

    pub(super) fn transfers_action(&mut self, action: Action, count: u32) {
        let order = self.transfer_order();
        if matches!(
            action,
            Action::NextTab
                | Action::PrevTab
                | Action::GoTab(_)
                | Action::Space
                | Action::TogglePanel
                | Action::Settings
                | Action::Quit
        ) {
            self.common_action(&action);
            return;
        }
        let view = &mut self.transfers;
        let last = order.len().saturating_sub(1);
        let n = count as usize;
        let selected = order.get(view.cursor.min(last)).copied();
        match action {
            Action::Down => view.cursor = (view.cursor + n).min(last),
            Action::Up => view.cursor = view.cursor.saturating_sub(n),
            Action::Top => view.cursor = 0,
            Action::Bottom => view.cursor = last,
            Action::Escape => self.set_tab(super::Tab::Files),
            Action::ToggleFocus => self.focus = super::Focus::Sidebar,
            Action::Help => self.overlay = Some(Overlay::Help { scroll: 0 }),
            Action::Stop => match selected {
                Some(id) if self.store.running().any(|r| r.spec.id == id) => {
                    self.stop_transfer_id(id);
                    self.save_store();
                }
                _ => self.info("Only running transfers can be stopped", MessageKind::Info),
            },
            Action::Resume if selected.is_some_and(|id| self.resuming.values().any(|old| *old == id)) => {
                self.info("Already resuming this transfer", MessageKind::Info);
            }
            Action::Resume => {
                let resumable = selected.and_then(|id| {
                    self.store.records.iter().find(|r| r.spec.id == id).map(|r| {
                        matches!(
                            r.state,
                            RecordState::Stopped | RecordState::Failed | RecordState::Interrupted
                        )
                    })
                });
                match (selected, resumable) {
                    (Some(id), Some(true)) => self.resume(id),
                    _ => self.info(
                        "Only stopped, failed or interrupted transfers can be resumed",
                        MessageKind::Info,
                    ),
                }
            }
            Action::ClearFinished => {
                self.store.clear_finished();
                self.save_store();
                let len = self.store.records.len();
                self.transfers.cursor = self.transfers.cursor.min(len.saturating_sub(1));
            }
            Action::RemoveTransfer => match selected {
                Some(id) if self.transfer_removable(id) => {
                    let title = self.store.get(id).map(|r| r.spec.title()).unwrap_or_default();
                    self.overlay = Some(Overlay::Confirm {
                        title: "Remove from the list".into(),
                        lines: vec![
                            format!("Remove {title} from the list?"),
                            "It can no longer be resumed; files already copied stay where they are.".into(),
                        ],
                        commands: Vec::new(),
                        danger: false,
                        on_yes: PendingOp::RemoveTransfers(vec![id]),
                    });
                }
                Some(_) => self.info(
                    "Running transfers cannot be removed — stop it first (s)",
                    MessageKind::Info,
                ),
                None => {}
            },
            Action::ClearInactive => {
                let ids: Vec<u64> = self
                    .store
                    .inactive_ids()
                    .into_iter()
                    .filter(|&id| self.transfer_removable(id))
                    .collect();
                if ids.is_empty() {
                    return self.info("Nothing to clear: every transfer is running", MessageKind::Info);
                }
                let problems = ids
                    .iter()
                    .filter(|&&id| self.store.get(id).is_some_and(|r| r.state != RecordState::Done))
                    .count();
                let mut lines = vec![format!("Remove {} from the list?", plural(ids.len(), "transfer"))];
                if problems > 0 {
                    lines.push(format!(
                        "{} stopped, failed or interrupted can no longer be resumed.",
                        plural(problems, "transfer")
                    ));
                }
                lines.push("Running transfers stay; files already copied stay where they are.".into());
                self.overlay = Some(Overlay::Confirm {
                    title: "Clear transfers".into(),
                    lines,
                    commands: Vec::new(),
                    danger: false,
                    on_yes: PendingOp::RemoveTransfers(ids),
                });
            }
            _ => {}
        }
    }

    /// Not running, starting, being stopped or being resumed.
    fn transfer_removable(&self, id: u64) -> bool {
        self.store.removable(id)
            && !self.stopping.contains(&id)
            && !self.resuming.values().any(|old| *old == id)
    }

    /// Drop records from the list, re-checking each: one may have been resumed since
    /// the question was asked.
    pub(super) fn remove_transfers(&mut self, ids: &[u64]) {
        let ids: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|&id| self.transfer_removable(id))
            .collect();
        for &id in &ids {
            self.store.remove(id);
            self.progress.remove(&id);
            self.progress_errors.remove(&id);
        }
        self.save_store();
        let len = self.store.records.len();
        self.transfers.cursor = self.transfers.cursor.min(len.saturating_sub(1));
        if !ids.is_empty() {
            let what = format!("Removed {} from the list", plural(ids.len(), "transfer"));
            self.info(what, MessageKind::Success);
        }
    }

    /// Relaunch what a stopped, failed or interrupted transfer did not finish.
    fn resume(&mut self, id: u64) {
        let Some(units) = self.store.get(id).map(|r| self.backend.job_units(&r.spec)) else {
            return;
        };
        let Some(spec) = self.store.resume_spec(id, &units) else {
            return;
        };
        if spec.items.is_empty() {
            return self.nothing_left(id);
        }
        self.resuming.insert(spec.id, id);
        if spec.kind == TransferKind::Copy {
            self.info("Resuming — files already copied are skipped", MessageKind::Info);
            return self.launch_transfer(spec);
        }
        // A move may have finished items whose job was not seen to end (rclone was
        // restarted, say): their source is gone. Look before relaunching them.
        self.launching += 1;
        let backend = self.backend.clone();
        self.spawn(async move {
            let result = crate::backend::moved_already(backend.as_ref(), &spec).await;
            AppEvent::ResumeChecked {
                old: id,
                spec,
                result,
            }
        });
    }

    fn nothing_left(&mut self, id: u64) {
        self.store.mark_done(id, jiff::Timestamp::now());
        self.save_store();
        self.info(
            "Nothing left to resume: every item had arrived",
            MessageKind::Success,
        );
    }

    pub(super) fn resume_checked(
        &mut self,
        old: u64,
        mut spec: TransferSpec,
        result: Result<Vec<bool>, BackendError>,
    ) {
        self.launching = self.launching.saturating_sub(1);
        match result {
            Ok(arrived) => {
                let mut arrived = arrived.into_iter();
                spec.items.retain(|_| !arrived.next().unwrap_or(false));
                if spec.items.is_empty() {
                    self.resuming.remove(&spec.id);
                    self.nothing_left(old);
                } else {
                    self.info("Resuming — items already moved are skipped", MessageKind::Info);
                    self.launch_transfer(spec);
                }
            }
            Err(e) => {
                self.resuming.remove(&spec.id);
                self.backend_error(e);
            }
        }
        self.check_quit_when_launched();
        self.check_quit_when_idle();
    }
}
