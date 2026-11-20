//! `Backend` implemented over the rclone rc API.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    About, Backend, BackendError, ConfigParams, ConfigStep, Entry, Provider, ProviderOption, RemoteInfo,
    StartError,
};
use crate::location::{Location, Remote};
use crate::rc::types::{
    RcAbout, RcConfigOut, RcJobId, RcJobStatus, RcList, RcListItem, RcOption, RcProviders, RcStats, RcVersion,
};
use crate::rc::{RcError, UnixRc};
use crate::transfer::spec::{FileProgress, TransferKind, TransferProgress, TransferSpec, aggregate};

/// Time limit for synchronous calls that may touch many files.
const LONG_TIMEOUT: Duration = Duration::from_secs(3600);

/// The stats group of the app's own file operations (new folder, rename, delete),
/// apart from the listings' one so that the errors of one never count for the other.
const OPS_GROUP: &str = "ratclone/ops";

pub struct RcBackend {
    rc: UnixRc,
    /// Where file lists for batched transfers are written (read by the local rcd).
    list_dir: PathBuf,
}

/// Listing a huge remote folder pages through the provider's API; stale answers are
/// dropped by request id, so a generous limit costs nothing.
const LIST_TIMEOUT: Duration = Duration::from_secs(900);

/// Independent rc calls in flight at once (each one holds a connection).
const PARALLEL_CALLS: usize = 8;

/// File lists older than this are removed when a new one is written.
const LIST_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

impl RcBackend {
    pub fn new(rc: UnixRc) -> Self {
        Self::with_list_dir(
            rc,
            std::env::temp_dir().join(format!("{}-lists", crate::APP_NAME)),
        )
    }

    pub fn with_list_dir(rc: UnixRc, list_dir: PathBuf) -> Self {
        RcBackend { rc, list_dir }
    }

    /// Whether listing `dir` finds anything. A missing folder is a 404, or an empty
    /// answer on bucket-based remotes (S3, B2), where folders are only name prefixes.
    async fn lists_anything(&self, dir: &Location) -> Result<bool, BackendError> {
        let params = json!({ "fs": dir.fs(), "remote": dir.path });
        match self
            .rc
            .call_typed_with_timeout::<RcList>("operations/list", params, LIST_TIMEOUT)
            .await
        {
            Ok(l) => Ok(!l.list.is_empty()),
            Err(RcError::Api { status: 404, .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Write `names` (one per line, relative to the source folder) for `--files-from-raw`.
    fn write_list(&self, group: &str, names: &[&str]) -> Result<PathBuf, BackendError> {
        let io = |e: std::io::Error| BackendError::Other(format!("could not write file list: {e}"));
        std::fs::create_dir_all(&self.list_dir).map_err(io)?;
        if let Ok(old) = std::fs::read_dir(&self.list_dir) {
            for f in old.flatten() {
                let stale = f
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok());
                if stale.is_some_and(|age| age > LIST_MAX_AGE) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
        let path = self.list_dir.join(format!("{}.txt", group.replace('/', "-")));
        let mut body = names.join("\n");
        body.push('\n');
        std::fs::write(&path, body).map_err(io)?;
        Ok(path)
    }

    /// Every one of `names` is a file in `dir`, or an error naming those that are not.
    async fn check_sources(&self, dir: &Location, names: &[&str]) -> Result<(), BackendError> {
        let present: std::collections::HashSet<String> = self
            .list(dir)
            .await?
            .into_iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.name)
            .collect();
        let missing: Vec<&str> = names.iter().copied().filter(|n| !present.contains(*n)).collect();
        match missing.as_slice() {
            [] => Ok(()),
            [one] => Err(BackendError::Other(format!("{one} is no longer at the source"))),
            [first, rest @ ..] => Err(BackendError::Other(format!(
                "{first} and {} more are no longer at the source",
                rest.len()
            ))),
        }
    }

    pub fn rc(&self) -> &UnixRc {
        &self.rc
    }
}

pub(crate) fn entry_from(item: RcListItem) -> Entry {
    Entry {
        size: if item.is_dir || item.size < 0 {
            None
        } else {
            Some(item.size as u64)
        },
        mod_time: item.mod_time.parse().ok(),
        is_dir: item.is_dir,
        mime: item.mime_type,
        name: item.name,
    }
}

#[derive(Deserialize)]
struct Remotes {
    #[serde(default)]
    remotes: Vec<String>,
}

#[derive(Deserialize)]
struct RemoteType {
    #[serde(rename = "type", default)]
    kind: String,
}

/// The backend type, or empty when this one remote's section can't be read.
fn kind_of(t: Result<RemoteType, RcError>) -> String {
    t.map(|t| t.kind).unwrap_or_default()
}

/// The answer of `config/providers`, as the app's types.
pub(crate) fn providers_from(p: RcProviders) -> Vec<Provider> {
    p.providers
        .into_iter()
        .map(|p| Provider {
            name: p.prefix,
            description: p.description,
            hide: p.hide,
            options: p.options.into_iter().map(option_from).collect(),
        })
        .collect()
}

fn option_from(o: RcOption) -> ProviderOption {
    ProviderOption {
        name: o.name,
        help: o.help,
        default: o.default_str,
        examples: o
            .examples
            .unwrap_or_default()
            .into_iter()
            .map(|e| (e.value, e.help))
            .collect(),
        exclusive: o.exclusive,
        required: o.required,
        is_password: o.is_password,
        advanced: o.advanced,
        sensitive: o.sensitive,
        kind: o.kind,
        hide: o.hide,
        provider: o.provider,
    }
}

pub(crate) fn step_from(out: RcConfigOut) -> ConfigStep {
    if out.option.is_none() && !out.state.is_empty() && !out.error.is_empty() {
        return ConfigStep::Rejected {
            state: out.state,
            error: out.error.trim_end().to_string(),
        };
    }
    match out.option {
        Some(option) if !out.state.is_empty() => ConfigStep::Question {
            state: out.state,
            option: option_from(option),
            error: (!out.error.is_empty()).then_some(out.error),
        },
        _ => ConfigStep::Done,
    }
}

fn opt(params: &ConfigParams, extra: Value) -> Value {
    let mut o = json!({ "nonInteractive": true, "obscure": params.obscure });
    if let (Some(o), Some(extra)) = (o.as_object_mut(), extra.as_object()) {
        o.extend(extra.clone());
    }
    o
}

/// The answer to a question goes in `opt.result`, not in `parameters`.
fn continue_body(name: &str, state: &str, answer: &str, obscure: bool) -> Value {
    json!({
        "name": name,
        "parameters": {},
        "opt": { "nonInteractive": true, "obscure": obscure, "continue": true, "state": state, "result": answer },
    })
}

/// Display a config value as text (rclone stores strings, but be lenient).
fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[derive(Deserialize)]
struct Stat {
    item: Option<RcListItem>,
}

#[async_trait]
impl Backend for RcBackend {
    async fn remotes(&self) -> Result<Vec<RemoteInfo>, BackendError> {
        let names: Remotes = self.rc.call_typed("config/listremotes", json!({})).await?;
        let mut out = vec![RemoteInfo {
            remote: Remote::Local,
            kind: "local".into(),
        }];
        // Asked all at once; answers keep the order of the names.
        // Only the `type` field is read: the section also holds OAuth tokens.
        let calls: Vec<_> = names
            .remotes
            .iter()
            .map(|name| {
                self.rc
                    .call_typed::<RemoteType>("config/get", json!({ "name": name }))
            })
            .collect();
        let types: Vec<_> = futures::stream::iter(calls)
            .buffered(PARALLEL_CALLS)
            .collect()
            .await;
        for (name, t) in names.remotes.into_iter().zip(types) {
            if let Err(RcError::Connect(e)) = &t {
                return Err(RcError::Connect(e.clone()).into());
            }
            out.push(RemoteInfo {
                remote: Remote::Named(name),
                kind: kind_of(t),
            });
        }
        Ok(out)
    }

    async fn list(&self, dir: &Location) -> Result<Vec<Entry>, BackendError> {
        let params = json!({ "fs": dir.fs(), "remote": dir.path });
        let l: RcList = self
            .rc
            .call_typed_with_timeout("operations/list", params, LIST_TIMEOUT)
            .await?;
        Ok(l.list.into_iter().map(entry_from).collect())
    }

    async fn stat(&self, loc: &Location) -> Result<Option<Entry>, BackendError> {
        let s: Stat = self
            .rc
            .call_typed("operations/stat", json!({ "fs": loc.fs(), "remote": loc.path }))
            .await?;
        Ok(s.item.map(entry_from))
    }

    async fn about(&self, remote: &Remote) -> Result<About, BackendError> {
        let fs = Location::new(remote.clone(), "").fs();
        let a: RcAbout = self
            .rc
            .call_typed("operations/about", json!({ "fs": fs }))
            .await?;
        Ok(About {
            total: a.total,
            used: a.used,
            free: a.free,
            trashed: a.trashed,
            other: a.other,
            objects: a.objects,
        })
    }

    async fn mkdir(&self, loc: &Location) -> Result<(), BackendError> {
        let params = json!({ "fs": loc.fs(), "remote": loc.path, "_group": OPS_GROUP });
        self.rc.call("operations/mkdir", params).await?;
        Ok(())
    }

    async fn rename(&self, from: &Location, new_name: &str, is_dir: bool) -> Result<(), BackendError> {
        let parent = from
            .parent()
            .ok_or_else(|| BackendError::Other("cannot rename a root".into()))?;
        let to = parent.join(new_name);
        // rclone moves onto whatever has the new name: a file is overwritten, a folder
        // merged into. The pane refuses names it shows, but its listing may be old,
        // and some services (Dropbox, OneDrive) find `photo.jpg` when asked for
        // `Photo.JPG`. Look right before moving. The entry itself answering to the
        // new name is a change of case only, which is fine.
        if let Some(taken) = self.stat(&to).await?
            && taken.name != from.name()
        {
            return Err(BackendError::Other(format!(
                "“{}” already exists here",
                taken.name
            )));
        }
        // `stat` finds a folder only by its exact name. Where the filesystem ignores
        // case (FAT, exFAT or SMB on Linux), `photos` still opens `Photos`, and moving
        // a folder there deletes every file in it as "already at the destination",
        // or merges into `B` when asked for `b`. A folder that lists nothing has no
        // file to lose, and is how a missing folder answers on bucket-based remotes.
        if is_dir && self.lists_anything(&to).await? {
            let message = if new_name.to_lowercase() == from.name().to_lowercase() {
                format!(
                    "this filesystem ignores case: “{new_name}” is “{}” itself; \
                     rename it to another name first",
                    from.name()
                )
            } else {
                format!("“{new_name}” already exists here")
            };
            return Err(BackendError::Other(message));
        }
        if is_dir {
            let params = json!({
                "srcFs": from.fs_path(),
                "dstFs": to.fs_path(),
                "deleteEmptySrcDirs": true,
                "createEmptySrcDirs": true,
                "_group": OPS_GROUP,
            });
            self.rc
                .call_with_timeout("sync/move", params, LONG_TIMEOUT)
                .await?;
        } else {
            let params = json!({
                "srcFs": from.fs(), "srcRemote": from.path,
                "dstFs": to.fs(), "dstRemote": to.path,
                "_group": OPS_GROUP,
            });
            self.rc
                .call_with_timeout("operations/movefile", params, LONG_TIMEOUT)
                .await?;
        }
        Ok(())
    }

    async fn delete(&self, loc: &Location, is_dir: bool) -> Result<(), BackendError> {
        let method = if is_dir {
            "operations/purge"
        } else {
            "operations/deletefile"
        };
        let params: Value = json!({ "fs": loc.fs(), "remote": loc.path, "_group": OPS_GROUP });
        self.rc.call_with_timeout(method, params, LONG_TIMEOUT).await?;
        Ok(())
    }

    async fn start_transfer(
        &self,
        spec: &TransferSpec,
        on_job: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<Vec<u64>, StartError> {
        let group = spec.group();
        let units = spec.job_units();
        let mut ids = Vec::with_capacity(units.len());
        // Files keeping their name travel in one job, so rclone's own --transfers
        // limit applies instead of one upload per file all at once.
        for unit in &units {
            let (method, params) = if unit.len() > 1 {
                let names: Vec<&str> = unit.iter().map(|&i| spec.items[i].name.as_str()).collect();
                // A name in the list that is not at the source is skipped without an
                // error, and the job still succeeds: look first, refuse rather than
                // report as done a transfer that left files out.
                if let Err(error) = self.check_sources(&spec.src_dir, &names).await {
                    return Err(StartError::new(ids, error));
                }
                let list = match self.write_list(&group, &names) {
                    Ok(list) => list,
                    Err(error) => return Err(StartError::new(ids, error)),
                };
                let method = if spec.kind == TransferKind::Copy {
                    "sync/copy"
                } else {
                    "sync/move"
                };
                (
                    method,
                    json!({
                        "srcFs": spec.src_dir.fs_path(),
                        "dstFs": spec.dst_dir.fs_path(),
                        "_filter": { "FilesFromRaw": [list.display().to_string()] },
                    }),
                )
            } else {
                let item = &spec.items[unit[0]];
                let src = spec.src_dir.join(&item.name);
                let dst = spec.dst_dir.join(&item.dst_name);
                match (item.is_dir, spec.kind) {
                    (false, kind) => (
                        if kind == TransferKind::Copy {
                            "operations/copyfile"
                        } else {
                            "operations/movefile"
                        },
                        json!({
                            "srcFs": src.fs(), "srcRemote": src.path,
                            "dstFs": dst.fs(), "dstRemote": dst.path,
                        }),
                    ),
                    (true, TransferKind::Copy) => (
                        "sync/copy",
                        json!({ "srcFs": src.fs_path(), "dstFs": dst.fs_path(), "createEmptySrcDirs": true }),
                    ),
                    (true, TransferKind::Move) => (
                        "sync/move",
                        json!({
                            "srcFs": src.fs_path(), "dstFs": dst.fs_path(),
                            "createEmptySrcDirs": true, "deleteEmptySrcDirs": true,
                        }),
                    ),
                }
            };
            let mut params = params;
            // rclone skips a destination of the same size and time as "already there"
            // (and a move then deletes the source): a chosen replace must write it.
            if unit.iter().any(|&i| spec.items[i].replace) {
                params["_config"] = json!({ "IgnoreTimes": true });
            }
            params["_async"] = json!(true);
            params["_group"] = json!(group);
            let id: RcJobId = match self.rc.call_typed(method, params).await {
                Ok(id) => id,
                // Jobs already started keep running: hand their ids back with the error.
                // A refusal from rclone means this job did not start; a lost or late
                // answer does not: rclone may have taken it.
                Err(e) => {
                    let uncertain = !matches!(e, RcError::Api { .. });
                    return Err(StartError {
                        started: ids,
                        error: e.into(),
                        uncertain,
                    });
                }
            };
            on_job(id.jobid);
            ids.push(id.jobid);
        }
        Ok(ids)
    }

    async fn group_jobs(&self, group: &str) -> Result<Vec<u64>, BackendError> {
        let list = self.rc.call("job/list", json!({})).await?;
        let mut ids: Vec<u64> = list
            .get("jobids")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_u64()).collect())
            .unwrap_or_default();
        ids.sort_unstable();
        let calls: Vec<_> = ids
            .iter()
            .map(|id| {
                self.rc
                    .call_typed::<RcJobStatus>("job/status", json!({ "jobid": id }))
            })
            .collect();
        let answers: Vec<_> = futures::stream::iter(calls)
            .buffered(PARALLEL_CALLS)
            .collect()
            .await;
        let mut mine = Vec::new();
        for (id, answer) in ids.into_iter().zip(answers) {
            match answer {
                Ok(s) if s.group == group => mine.push(id),
                Ok(_) => {}
                // Expired between the list and the status: not one we can follow.
                Err(RcError::Api { message, .. }) if message.contains("job not found") => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(mine)
    }

    async fn transfer_progress(&self, group: &str, jobids: &[u64]) -> Result<TransferProgress, BackendError> {
        let mut statuses = Vec::with_capacity(jobids.len());
        let calls: Vec<_> = jobids
            .iter()
            .map(|id| {
                self.rc
                    .call_typed::<RcJobStatus>("job/status", json!({ "jobid": id }))
            })
            .collect();
        let answers: Vec<_> = futures::stream::iter(calls)
            .buffered(PARALLEL_CALLS)
            .collect()
            .await;
        let mut done_jobs = Vec::new();
        for answer in answers {
            match answer {
                // A job id from another group is a reused id after an rcd restart: not ours.
                Ok(s) => {
                    let ours = s.group == group;
                    if ours && s.finished && s.success {
                        done_jobs.push(s.id);
                    }
                    statuses.push(ours.then_some(s))
                }
                Err(RcError::Api { message, .. }) if message.contains("job not found") => statuses.push(None),
                Err(e) => return Err(e.into()),
            }
        }
        // Stats are read after the job statuses so that a "finished" state never
        // comes with numbers older than the moment it finished.
        let stats: RcStats = self
            .rc
            .call_typed("core/stats", json!({ "group": group }))
            .await?;
        Ok(TransferProgress {
            state: aggregate(&statuses),
            bytes: stats.bytes,
            total_bytes: stats.total_bytes,
            speed: stats.speed,
            eta: stats.eta,
            errors: stats.errors,
            files: stats
                .transferring
                .into_iter()
                .map(|t| FileProgress {
                    name: t.name,
                    bytes: t.bytes,
                    size: (t.size >= 0).then_some(t.size as u64),
                    percentage: t.percentage,
                })
                .collect(),
            done_jobs,
        })
    }

    async fn stop_transfer(&self, group: &str) -> Result<(), BackendError> {
        self.rc.call("job/stopgroup", json!({ "group": group })).await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), BackendError> {
        crate::daemon::shutdown(&self.rc).await?;
        Ok(())
    }

    async fn get_bwlimit(&self) -> Result<String, BackendError> {
        let reply = self.rc.call("core/bwlimit", json!({})).await?;
        Ok(reply
            .get("rate")
            .and_then(|r| r.as_str())
            .unwrap_or("off")
            .to_string())
    }

    async fn providers(&self) -> Result<Vec<Provider>, BackendError> {
        let p: RcProviders = self.rc.call_typed("config/providers", json!({})).await?;
        Ok(providers_from(p))
    }

    async fn remote_config(
        &self,
        name: &str,
    ) -> Result<std::collections::BTreeMap<String, String>, BackendError> {
        let v = self.rc.call("config/get", json!({ "name": name })).await?;
        Ok(v.as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), value_text(v))).collect())
            .unwrap_or_default())
    }

    async fn config_create(
        &self,
        name: &str,
        kind: &str,
        params: ConfigParams,
    ) -> Result<ConfigStep, BackendError> {
        let body = json!({ "name": name, "type": kind, "parameters": params.values, "opt": opt(&params, json!({})) });
        let out: RcConfigOut = self.rc.call_typed("config/create", body).await?;
        Ok(step_from(out))
    }

    async fn config_update(&self, name: &str, params: ConfigParams) -> Result<ConfigStep, BackendError> {
        let body = json!({ "name": name, "parameters": params.values, "opt": opt(&params, json!({})) });
        let out: RcConfigOut = self.rc.call_typed("config/update", body).await?;
        Ok(step_from(out))
    }

    async fn config_continue(
        &self,
        name: &str,
        state: &str,
        answer: &str,
        obscure: bool,
    ) -> Result<ConfigStep, BackendError> {
        let body = continue_body(name, state, answer, obscure);
        let out: RcConfigOut = self.rc.call_typed("config/update", body).await?;
        Ok(step_from(out))
    }

    async fn config_continue_job(
        &self,
        name: &str,
        state: &str,
        answer: &str,
        obscure: bool,
    ) -> Result<u64, BackendError> {
        // Before 1.75 the OAuth callback wait ignores job cancellation, leaving
        // the job and localhost listener alive. Use the token-paste flow instead.
        if answer == "true" {
            let version: RcVersion = self.rc.call_typed("core/version", json!({})).await?;
            if version.decomposed < [1, 75, 0] {
                return Err(BackendError::LocalOAuthUnsupported);
            }
        }
        let mut body = continue_body(name, state, answer, obscure);
        body["_async"] = json!(true);
        let id: RcJobId = self.rc.call_typed("config/update", body).await?;
        Ok(id.jobid)
    }

    async fn config_job(&self, jobid: u64) -> Result<Option<Result<ConfigStep, BackendError>>, BackendError> {
        let v = self.rc.call("job/status", json!({ "jobid": jobid })).await?;
        if !v.get("finished").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(None);
        }
        if !v.get("success").and_then(Value::as_bool).unwrap_or(false) {
            let e = v.get("error").and_then(Value::as_str).unwrap_or("job failed");
            return Ok(Some(Err(BackendError::Rclone(e.to_string()))));
        }
        let out: RcConfigOut = v
            .get("output")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| BackendError::Other(format!("unexpected job output: {e}")))?
            .unwrap_or_default();
        Ok(Some(Ok(step_from(out))))
    }

    async fn stop_job(&self, jobid: u64) -> Result<(), BackendError> {
        self.rc.call("job/stop", json!({ "jobid": jobid })).await?;
        Ok(())
    }

    async fn config_delete(&self, name: &str) -> Result<(), BackendError> {
        self.rc.call("config/delete", json!({ "name": name })).await?;
        Ok(())
    }

    async fn set_bwlimit(&self, rate: &str) -> Result<String, BackendError> {
        let reply = self.rc.call("core/bwlimit", json!({ "rate": rate })).await?;
        Ok(reply
            .get("rate")
            .and_then(|r| r.as_str())
            .unwrap_or(rate)
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_unreadable_remote_does_not_hide_the_others() {
        let err = crate::rc::RcError::Api {
            status: 500,
            message: "boom".into(),
            path: "config/get".into(),
        };
        assert_eq!(kind_of(Err(err)), "");
        assert_eq!(kind_of(Ok(RemoteType { kind: "drive".into() })), "drive");
    }

    #[test]
    fn rejected_answer_is_not_done() {
        // What rclone v1.76 answers to a token it can't decode.
        let out: RcConfigOut = serde_json::from_value(json!({
            "Error": "Couldn't decode response - try again", "Option": null,
            "Result": "", "State": "*oauth-authorize,,,"
        }))
        .unwrap();
        assert_eq!(
            step_from(out),
            ConfigStep::Rejected {
                state: "*oauth-authorize,,,".into(),
                error: "Couldn't decode response - try again".into()
            }
        );
        let done: RcConfigOut =
            serde_json::from_value(json!({"Error": "", "Option": null, "State": ""})).unwrap();
        assert_eq!(step_from(done), ConfigStep::Done);
    }

    #[test]
    fn listings_may_take_minutes() {
        // Drive pages 1000 entries per request: a 50k-entry folder can take close to a minute.
        assert!(LIST_TIMEOUT >= Duration::from_secs(600));
    }
}
