//! The helper left behind when the app quits with transfers still running
//! (`ratclone --reap-when-idle`): it writes down how each transfer ended, and
//! stops the rcd once nothing needs it.

use std::path::Path;
use std::time::Duration;

use serde_json::json;

use crate::backend::Backend;
use crate::backend::rc::RcBackend;
use crate::rc::types::RcJobStatus;
use crate::rc::{RcError, UnixRc};
use crate::transfer::store::{RecordState, TransferStore};

/// How often the helper looks at the rcd.
pub const INTERVAL: Duration = Duration::from_secs(30);

/// The transfer history, in the app's state directory.
pub const STORE_FILE: &str = "transfers.json";

/// Follow the rcd until nothing needs it, then stop it. Each round, with no app
/// open (the instance lock in `state_dir` is free), the transfers the app left
/// running are checked and their end is saved to the history: the rcd forgets a
/// finished job after a few minutes, and everything once it is stopped, so an app
/// opened later could only call them interrupted. The rcd is stopped when no
/// transfer of the app is running and nothing is mounted. Returns when the rcd is
/// gone, or at once when another helper is already on duty.
pub async fn reap_when_idle(rc: &UnixRc, state_dir: &Path, interval: Duration) {
    // One helper is enough: each quit with transfers running would add another.
    let Ok(_duty) = crate::paths::acquire_lock(state_dir, "reaper.lock") else {
        return;
    };
    let backend = RcBackend::new(rc.clone());
    loop {
        tokio::time::sleep(interval).await;
        if let Err(RcError::Connect(_)) = rc.call("core/version", json!({})).await {
            return;
        }
        // Holding the lock also keeps a starting app out until we decide.
        let Ok(_lock) = crate::paths::acquire_instance_lock(state_dir) else {
            continue;
        };
        record_outcomes(&backend, &state_dir.join(STORE_FILE)).await;
        if let Ok(false) = busy(rc).await {
            let _ = crate::daemon::shutdown(rc).await;
            return;
        }
    }
}

/// Save how the transfers in the history at `path` stand now: jobs rclone accepted
/// for a transfer still marked as starting, and the end of each running one.
/// Call only while holding the instance lock: the app owns the file otherwise.
pub async fn record_outcomes(backend: &dyn Backend, path: &Path) {
    if !path.exists() {
        return;
    }
    let Ok(mut store) = TransferStore::load(path.to_path_buf()) else {
        return;
    };
    let mut changed = false;
    // Left starting by an app that went away mid-launch (a closed terminal).
    let starting: Vec<_> = store.starting().map(|r| (r.spec.clone(), r.replaces)).collect();
    for (spec, replaces) in starting {
        if let Ok(found) = backend.group_jobs(&spec.group()).await
            && !found.is_empty()
        {
            // A resume that got going takes the place of the record it resumed.
            if let Some(old) = replaces {
                store.remove(old);
            }
            store.launched(spec.id, found, backend.job_units(&spec).len());
            changed = true;
        }
    }
    let running: Vec<_> = store
        .running()
        .map(|r| (r.spec.id, r.spec.group(), r.jobids.clone()))
        .collect();
    for (id, group, jobids) in running {
        let Ok(progress) = backend.transfer_progress(&group, &jobids).await else {
            continue;
        };
        let before = store.get(id).map(|r| r.done_jobs.len());
        store.apply(id, &progress, jiff::Timestamp::now());
        // Bytes alone are not worth a write every round; an ended transfer or job is.
        changed |= store
            .get(id)
            .is_some_and(|r| r.state != RecordState::Running || Some(r.done_jobs.len()) != before);
    }
    if changed {
        let _ = store.save();
    }
}

#[derive(serde::Deserialize)]
struct JobList {
    #[serde(rename = "runningIds", default)]
    running: Option<Vec<u64>>,
    jobids: Vec<u64>,
}

impl JobList {
    fn candidates(self) -> Vec<u64> {
        // Older supported rclones return only jobids. Check their status below
        // instead of treating a missing runningIds field as an idle daemon.
        self.running.unwrap_or(self.jobids)
    }
}

#[derive(serde::Deserialize)]
struct Mounts {
    #[serde(rename = "mountPoints", default)]
    mount_points: Vec<serde_json::Value>,
}

/// Any app transfer still running, or any mount active?
async fn busy(rc: &UnixRc) -> Result<bool, RcError> {
    let jobs: JobList = rc.call_typed("job/list", json!({})).await?;
    for id in jobs.candidates() {
        if let Ok(s) = rc
            .call_typed::<RcJobStatus>("job/status", json!({ "jobid": id }))
            .await
            && !s.finished
            && s.group.starts_with("rtui/")
        {
            return Ok(true);
        }
    }
    let mounts: Mounts = rc.call_typed("mount/listmounts", json!({})).await?;
    Ok(!mounts.mount_points.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_job_list_keeps_candidates_for_status_checks() {
        // rclone 1.68 returns jobids but has no runningIds/finishedIds fields.
        let jobs: JobList = serde_json::from_value(json!({
            "executeId": "legacy", "jobids": [1, 2, 3]
        }))
        .unwrap();
        assert_eq!(jobs.candidates(), vec![1, 2, 3]);
    }

    #[test]
    fn current_job_list_limits_status_checks_to_running_jobs() {
        let jobs: JobList = serde_json::from_value(json!({
            "executeId": "current", "jobids": [1, 2, 3],
            "finishedIds": [1, 3], "runningIds": [2]
        }))
        .unwrap();
        assert_eq!(jobs.candidates(), vec![2]);
    }
}
