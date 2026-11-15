//! Persisted history of transfers, so they survive TUI restarts.

use std::path::PathBuf;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::spec::{JobsState, TransferProgress, TransferSpec};

/// Records kept on disk; the oldest finished ones are dropped first.
pub const MAX_RECORDS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordState {
    /// Saved before the launch: its jobs are being handed to rclone (`jobids` grows as
    /// rclone accepts them). Left over after an exit, it is reconciled with rclone.
    Starting,
    Running,
    Done,
    Failed,
    Stopped,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransferRecord {
    pub spec: TransferSpec,
    pub jobids: Vec<u64>,
    pub state: RecordState,
    pub started: Timestamp,
    pub finished: Option<Timestamp>,
    pub bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
    /// Set when only some items could be started; the record ends as Failed.
    #[serde(default)]
    pub start_error: Option<String>,
    /// Jobs known to have finished successfully: a resume skips their items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub done_jobs: Vec<u64>,
    /// A resume: the record this one replaces once one of its jobs is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<u64>,
}

impl TransferRecord {
    /// For each item, whether its job is known to have succeeded (`units` maps each
    /// job, in start order, to its items; see [`TransferSpec::job_units`]).
    pub fn done_items(&self, units: &[Vec<usize>]) -> Vec<bool> {
        let mut done = vec![false; self.spec.items.len()];
        let mut jobids = self.jobids.clone();
        jobids.sort_unstable();
        for (unit, job) in units.iter().zip(&jobids) {
            if self.done_jobs.contains(job) {
                for &i in unit {
                    if let Some(d) = done.get_mut(i) {
                        *d = true;
                    }
                }
            }
        }
        done
    }

    /// Running in rclone, or being handed to it: never dropped from the history.
    pub fn is_live(&self) -> bool {
        matches!(self.state, RecordState::Running | RecordState::Starting)
    }
}

#[derive(Debug)]
pub struct TransferStore {
    path: PathBuf,
    pub records: Vec<TransferRecord>,
    next_id: u64,
}

#[derive(Deserialize)]
struct OnDisk {
    next_id: u64,
    records: Vec<TransferRecord>,
}

/// What [`TransferStore::save`] writes: the same shape as [`OnDisk`], borrowed.
#[derive(Serialize)]
struct OnDiskRef<'a> {
    next_id: u64,
    records: &'a [TransferRecord],
}

impl TransferStore {
    /// Missing file → empty store. Unreadable JSON is renamed to `*.bad`.
    pub fn load(path: PathBuf) -> color_eyre::Result<Self> {
        let disk = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<OnDisk>(&bytes) {
                Ok(d) => d,
                Err(_) => {
                    let mut bad = path.clone().into_os_string();
                    bad.push(".bad");
                    std::fs::rename(&path, bad)?;
                    OnDisk {
                        next_id: 1,
                        records: Vec::new(),
                    }
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => OnDisk {
                next_id: 1,
                records: Vec::new(),
            },
            Err(e) => return Err(e.into()),
        };
        Ok(TransferStore {
            path,
            records: disk.records,
            next_id: disk.next_id.max(1),
        })
    }

    /// Atomic write: temporary file in the same directory, then rename.
    pub fn save(&self) -> color_eyre::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let disk = OnDiskRef {
            next_id: self.next_id,
            records: &self.records,
        };
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        std::fs::write(&tmp, serde_json::to_vec(&disk)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn push(&mut self, spec: TransferSpec, jobids: Vec<u64>, now: Timestamp) {
        self.push_state(spec, jobids, RecordState::Running, now);
    }

    /// Record a transfer about to be launched (see [`RecordState::Starting`]).
    pub fn push_starting(&mut self, spec: TransferSpec, replaces: Option<u64>, now: Timestamp) {
        self.push_state(spec, Vec::new(), RecordState::Starting, now);
        if let Some(r) = self.records.last_mut() {
            r.replaces = replaces;
        }
    }

    fn push_state(&mut self, spec: TransferSpec, jobids: Vec<u64>, state: RecordState, now: Timestamp) {
        self.records.push(TransferRecord {
            spec,
            jobids,
            state,
            started: now,
            finished: None,
            bytes: 0,
            total_bytes: 0,
            error: None,
            start_error: None,
            done_jobs: Vec::new(),
            replaces: None,
        });
        // Over the cap, finished records go first; running ones are never dropped.
        while self.records.len() > MAX_RECORDS {
            let victim = self
                .records
                .iter()
                .position(|r| r.state == RecordState::Done)
                .or_else(|| self.records.iter().position(|r| !r.is_live()));
            match victim {
                Some(i) => {
                    self.records.remove(i);
                }
                None => break,
            }
        }
    }

    pub fn get(&self, id: u64) -> Option<&TransferRecord> {
        self.records.iter().find(|r| r.spec.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut TransferRecord> {
        self.records.iter_mut().find(|r| r.spec.id == id)
    }

    /// Fold live progress into a running record; finished records are left alone.
    pub fn apply(&mut self, id: u64, p: &TransferProgress, now: Timestamp) {
        let Some(r) = self.get_mut(id) else { return };
        // Even a late answer tells which jobs are done: a resume must not repeat them.
        for j in &p.done_jobs {
            if r.jobids.contains(j) && !r.done_jobs.contains(j) {
                r.done_jobs.push(*j);
            }
        }
        if r.state != RecordState::Running {
            return;
        }
        r.bytes = p.bytes;
        r.total_bytes = p.total_bytes;
        let finished = |r: &mut TransferRecord, state| {
            r.state = state;
            r.finished = Some(now);
        };
        match &p.state {
            JobsState::Running => {}
            JobsState::Succeeded => match r.start_error.clone() {
                Some(e) => {
                    finished(r, RecordState::Failed);
                    r.error = Some(format!("some items were not started ({e}); resume to finish"));
                }
                None => finished(r, RecordState::Done),
            },
            JobsState::Failed(msg) => {
                finished(r, RecordState::Failed);
                r.error = Some(msg.clone());
            }
            JobsState::NotFound => finished(r, RecordState::Interrupted),
        }
    }

    /// Some items of this transfer never started; it can't end as Done.
    pub fn set_start_error(&mut self, id: u64, msg: String) {
        if let Some(r) = self.get_mut(id) {
            r.start_error = Some(msg);
        }
    }

    /// Only a running transfer can become stopped (one that ended first keeps its end).
    pub fn mark_stopped(&mut self, id: u64, now: Timestamp) {
        if let Some(r) = self.get_mut(id)
            && r.state == RecordState::Running
        {
            r.state = RecordState::Stopped;
            r.finished = Some(now);
        }
    }

    /// rclone accepted one more job of a starting transfer.
    pub fn add_job(&mut self, id: u64, jobid: u64) {
        if let Some(r) = self.get_mut(id)
            && !r.jobids.contains(&jobid)
        {
            r.jobids.push(jobid);
        }
    }

    /// The launch is over (or was reconciled): follow `jobids` from now on. Items of
    /// the `units` with no job never started: the transfer can't end as Done.
    pub fn launched(&mut self, id: u64, jobids: Vec<u64>, units: usize) {
        let Some(r) = self.get_mut(id) else { return };
        for j in jobids {
            if !r.jobids.contains(&j) {
                r.jobids.push(j);
            }
        }
        r.jobids.sort_unstable();
        r.state = RecordState::Running;
        if r.jobids.len() < units && r.start_error.is_none() {
            r.start_error = Some(format!("{} of {units} jobs started", r.jobids.len()));
        }
    }

    /// A starting transfer of which nothing reached rclone.
    pub fn never_started(&mut self, id: u64, why: String, now: Timestamp) {
        if let Some(r) = self.get_mut(id) {
            r.state = RecordState::Interrupted;
            r.finished = Some(now);
            r.error = Some(why);
        }
    }

    /// Used when a fresh rcd was started: nothing it knew about survives.
    pub fn interrupt_all_running(&mut self) {
        for r in &mut self.records {
            if r.state == RecordState::Running {
                r.state = RecordState::Interrupted;
            }
        }
    }

    /// Transfers left in [`RecordState::Starting`].
    pub fn starting(&self) -> impl Iterator<Item = &TransferRecord> {
        self.records.iter().filter(|r| r.state == RecordState::Starting)
    }

    pub fn running(&self) -> impl Iterator<Item = &TransferRecord> {
        self.records.iter().filter(|r| r.state == RecordState::Running)
    }

    /// Drop completed transfers; failed, stopped and interrupted ones stay.
    pub fn clear_finished(&mut self) {
        self.records.retain(|r| r.state != RecordState::Done);
    }

    /// The record's unfinished items under a fresh id, ready to relaunch: items of
    /// jobs known to have succeeded are left out (`units` maps each job, in order, to
    /// its items; items past the last job never started). The old record stays until
    /// the relaunch has started (see [`TransferStore::remove`]).
    pub fn resume_spec(&mut self, id: u64, units: &[Vec<usize>]) -> Option<TransferSpec> {
        let r = self.records.iter().find(|r| r.spec.id == id)?;
        let mut spec = r.spec.clone();
        let skip = r.done_items(units);
        let mut n = 0;
        spec.items.retain(|_| {
            n += 1;
            !skip[n - 1]
        });
        spec.id = self.next_id();
        Some(spec)
    }

    /// Mark a record done once nothing is left to resume.
    pub fn mark_done(&mut self, id: u64, now: Timestamp) {
        if let Some(r) = self.get_mut(id) {
            r.state = RecordState::Done;
            r.finished = Some(now);
            r.error = None;
        }
    }

    /// Whether a record can be dropped from the list: not running or starting, and
    /// not the record a resume being started will replace.
    pub fn removable(&self, id: u64) -> bool {
        let Some(r) = self.get(id) else {
            return false;
        };
        !r.is_live() && !self.records.iter().any(|o| o.is_live() && o.replaces == Some(id))
    }

    /// Ids of every record [`TransferStore::removable`] would drop.
    pub fn inactive_ids(&self) -> Vec<u64> {
        self.records
            .iter()
            .map(|r| r.spec.id)
            .filter(|&id| self.removable(id))
            .collect()
    }

    pub fn remove(&mut self, id: u64) {
        self.records.retain(|r| r.spec.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{Location, Remote};
    use crate::transfer::spec::{TransferItem, TransferKind};

    fn now() -> Timestamp {
        "2026-11-14T12:00:00Z".parse().unwrap()
    }

    fn spec(id: u64) -> TransferSpec {
        TransferSpec {
            id,
            kind: TransferKind::Copy,
            src_dir: Location::new(Remote::Local, "home/t"),
            dst_dir: Location::new(Remote::Named("g".into()), "Docs"),
            items: vec![TransferItem {
                replace: false,
                name: "a".into(),
                is_dir: false,
                dst_name: "a".into(),
                size: None,
            }],
        }
    }

    fn progress(state: JobsState, bytes: u64) -> TransferProgress {
        TransferProgress {
            state,
            bytes,
            total_bytes: 100,
            speed: 0.0,
            eta: None,
            errors: 0,
            files: vec![],
            done_jobs: vec![],
        }
    }

    fn store() -> (tempfile::TempDir, TransferStore) {
        let d = tempfile::tempdir().unwrap();
        let s = TransferStore::load(d.path().join("transfers.json")).unwrap();
        (d, s)
    }

    #[test]
    fn roundtrip_and_ids_persist() {
        let (d, mut s) = store();
        let id = s.next_id();
        assert_eq!(s.next_id(), id + 1);
        s.push(spec(id), vec![3, 4], now());
        s.save().unwrap();
        let mut again = TransferStore::load(d.path().join("transfers.json")).unwrap();
        assert_eq!(again.records, s.records);
        assert_eq!(again.next_id(), id + 2);
    }

    #[test]
    fn apply_each_state() {
        let (_d, mut s) = store();
        for id in 1..=4 {
            s.push(spec(id), vec![id], now());
        }
        s.apply(1, &progress(JobsState::Running, 40), now());
        s.apply(2, &progress(JobsState::Succeeded, 100), now());
        s.apply(3, &progress(JobsState::Failed("boom".into()), 10), now());
        s.apply(4, &progress(JobsState::NotFound, 0), now());
        let st: Vec<RecordState> = s.records.iter().map(|r| r.state).collect();
        assert_eq!(
            st,
            [
                RecordState::Running,
                RecordState::Done,
                RecordState::Failed,
                RecordState::Interrupted
            ]
        );
        assert_eq!(s.records[0].bytes, 40);
        assert_eq!(s.records[0].total_bytes, 100);
        assert_eq!(s.records[1].finished, Some(now()));
        assert_eq!(s.records[2].error.as_deref(), Some("boom"));
        assert!(s.records[0].finished.is_none());
    }

    #[test]
    fn stopped_record_ignores_late_progress() {
        let (_d, mut s) = store();
        s.push(spec(1), vec![1], now());
        s.mark_stopped(1, now());
        s.apply(
            1,
            &progress(JobsState::Failed("context canceled".into()), 5),
            now(),
        );
        assert_eq!(s.records[0].state, RecordState::Stopped);
    }

    #[test]
    fn only_records_at_rest_are_removable() {
        let (_d, mut s) = store();
        s.push(spec(1), vec![1], now());
        s.push(spec(2), vec![2], now());
        s.mark_stopped(2, now());
        s.push(spec(3), vec![3], now());
        s.apply(3, &progress(JobsState::Succeeded, 100), now());
        // A resume of 4 being handed to rclone: 4 must stay until it runs.
        s.push(spec(4), vec![4], now());
        s.mark_stopped(4, now());
        s.push_starting(spec(5), Some(4), now());
        assert!(!s.removable(1), "running");
        assert!(s.removable(2), "stopped");
        assert!(s.removable(3), "done");
        assert!(!s.removable(4), "replaced by a starting resume");
        assert!(!s.removable(5), "starting");
        assert!(!s.removable(99), "unknown");
        assert_eq!(s.inactive_ids(), [2, 3]);
    }

    #[test]
    fn interrupt_only_running() {
        let (_d, mut s) = store();
        s.push(spec(1), vec![1], now());
        s.push(spec(2), vec![2], now());
        s.apply(2, &progress(JobsState::Succeeded, 100), now());
        s.interrupt_all_running();
        assert_eq!(s.records[0].state, RecordState::Interrupted);
        assert_eq!(s.records[1].state, RecordState::Done);
        assert_eq!(s.running().count(), 0);
    }

    #[test]
    fn resume_gives_new_id_and_drops_old() {
        let (_d, mut s) = store();
        let id = s.next_id();
        s.push(spec(id), vec![1], now());
        s.mark_stopped(id, now());
        let r = s.resume_spec(id, &[vec![0]]).unwrap();
        assert_ne!(r.id, id);
        assert_eq!(r.items, spec(id).items);
        assert_eq!(r.dst_dir, spec(id).dst_dir);
        assert_eq!(s.records.len(), 1, "kept until the relaunch starts");
        s.remove(id);
        assert!(s.records.is_empty());
        assert!(s.resume_spec(12345, &[]).is_none());
    }

    #[test]
    fn clear_finished_keeps_problems() {
        let (_d, mut s) = store();
        for id in 1..=3 {
            s.push(spec(id), vec![id], now());
        }
        s.apply(1, &progress(JobsState::Succeeded, 100), now());
        s.apply(2, &progress(JobsState::Failed("x".into()), 0), now());
        s.clear_finished();
        let ids: Vec<u64> = s.records.iter().map(|r| r.spec.id).collect();
        assert_eq!(ids, [2, 3]);
    }

    #[test]
    fn corrupt_file_is_set_aside() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("transfers.json");
        std::fs::write(&p, "{not json").unwrap();
        let s = TransferStore::load(p.clone()).unwrap();
        assert!(s.records.is_empty());
        assert!(d.path().join("transfers.json.bad").exists());
    }

    #[test]
    fn cap_never_drops_running_transfers() {
        let (_d, mut s) = store();
        for id in 1..=(MAX_RECORDS as u64 + 3) {
            s.push(spec(id), vec![id], now());
        }
        assert_eq!(s.running().count(), MAX_RECORDS + 3);
    }

    #[test]
    fn history_is_capped() {
        let (_d, mut s) = store();
        for id in 1..=(MAX_RECORDS as u64 + 10) {
            s.push(spec(id), vec![id], now());
            if id <= 20 {
                s.apply(id, &progress(JobsState::Succeeded, 100), now());
            }
        }
        assert_eq!(s.records.len(), MAX_RECORDS);
        // The ten oldest Done records went first.
        assert_eq!(s.records[0].spec.id, 11);
    }

    fn spec_of(id: u64, names: &[&str]) -> TransferSpec {
        TransferSpec {
            items: names
                .iter()
                .map(|n| TransferItem {
                    replace: false,
                    name: n.to_string(),
                    is_dir: false,
                    dst_name: n.to_string(),
                    size: None,
                })
                .collect(),
            ..spec(id)
        }
    }

    #[test]
    fn resume_leaves_out_items_of_jobs_that_succeeded() {
        let (_d, mut s) = store();
        s.push(spec_of(1, &["a", "b", "c", "d"]), vec![10, 11, 12], now());
        let mut p = progress(JobsState::Failed("boom".into()), 0);
        p.done_jobs = vec![10, 12, 99];
        s.apply(1, &p, now());
        assert_eq!(s.records[0].done_jobs, [10, 12], "only jobs of this transfer");
        // Job 10 carried a and c (a batch), 11 carried b, 12 carried d.
        let units = [vec![0, 2], vec![1], vec![3]];
        let r = s.resume_spec(1, &units).unwrap();
        let names: Vec<&str> = r.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["b"]);
        // Items whose job never started are resumed too.
        s.push(spec_of(2, &["a", "b"]), vec![20], now());
        s.records[1].done_jobs = vec![20];
        let r = s.resume_spec(2, &[vec![0], vec![1]]).unwrap();
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].name, "b");
    }

    #[test]
    fn late_progress_still_records_finished_jobs() {
        let (_d, mut s) = store();
        s.push(spec_of(1, &["a", "b"]), vec![1, 2], now());
        s.mark_stopped(1, now());
        let mut p = progress(JobsState::Failed("context canceled".into()), 0);
        p.done_jobs = vec![1];
        s.apply(1, &p, now());
        assert_eq!(s.records[0].state, RecordState::Stopped);
        assert_eq!(s.records[0].done_jobs, [1]);
    }

    #[test]
    fn a_starting_record_grows_then_runs() {
        let (d, mut s) = store();
        s.push_starting(spec_of(1, &["a", "b", "c"]), Some(7), now());
        s.add_job(1, 5);
        s.save().unwrap();
        let mut again = TransferStore::load(d.path().join("transfers.json")).unwrap();
        let r = &again.records[0];
        assert_eq!(
            (r.state, r.jobids.clone(), r.replaces),
            (RecordState::Starting, vec![5], Some(7))
        );
        assert_eq!(again.starting().count(), 1);
        assert_eq!(again.running().count(), 0);
        again.launched(1, vec![5, 6], 3);
        let r = &again.records[0];
        assert_eq!((r.state, r.jobids.clone()), (RecordState::Running, vec![5, 6]));
        assert!(r.start_error.is_some(), "the third item never started");
    }

    #[test]
    fn stopping_an_ended_transfer_keeps_its_end() {
        let (_d, mut s) = store();
        s.push(spec(1), vec![1], now());
        s.apply(1, &progress(JobsState::Succeeded, 100), now());
        s.mark_stopped(1, now());
        assert_eq!(s.records[0].state, RecordState::Done);
    }
}
