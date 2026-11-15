//! The operations the app needs from rclone, behind a trait.

pub mod fake;
pub mod rc;

use async_trait::async_trait;

use crate::location::{Location, Remote};
use crate::rc::RcError;
use crate::transfer::spec::{TransferProgress, TransferSpec};

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// `None` for directories or when the backend does not know.
    pub size: Option<u64>,
    pub mod_time: Option<jiff::Timestamp>,
    pub is_dir: bool,
    pub mime: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct About {
    pub total: Option<u64>,
    pub used: Option<u64>,
    pub free: Option<u64>,
    pub trashed: Option<u64>,
    pub other: Option<u64>,
    pub objects: Option<u64>,
}

impl About {
    /// Fraction of the quota in use, when both numbers are known.
    pub fn used_ratio(&self) -> Option<f64> {
        match (self.used, self.total) {
            (Some(u), Some(t)) if t > 0 => Some(u as f64 / t as f64),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteInfo {
    pub remote: Remote,
    pub kind: String,
}

/// An rclone backend type from `config/providers`.
#[derive(Clone, Debug, PartialEq)]
pub struct Provider {
    /// The type name (`Prefix`), e.g. `drive`.
    pub name: String,
    pub description: String,
    pub hide: bool,
    pub options: Vec<ProviderOption>,
}

/// One option of a provider, or a question from the config state machine.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ProviderOption {
    pub name: String,
    pub help: String,
    /// rclone's `DefaultStr`.
    pub default: String,
    /// `(value, help)` pairs.
    pub examples: Vec<(String, String)>,
    pub exclusive: bool,
    pub required: bool,
    pub is_password: bool,
    pub advanced: bool,
    pub sensitive: bool,
    /// rclone's `Type` (`string`, `bool`, `int`, `SizeSuffix`…).
    pub kind: String,
    /// rclone's visibility bits (`4` = hidden from the configurator).
    pub hide: u8,
    /// rclone's `Provider`: the values of the `provider` option this one applies to
    /// (`a,b`, or `!a,b` for all others); empty for all.
    pub provider: String,
}

/// Values sent to `config/create` / `config/update`.
#[derive(Clone, PartialEq, Default)]
pub struct ConfigParams {
    pub values: std::collections::BTreeMap<String, String>,
    /// Ask rclone to obscure password values.
    pub obscure: bool,
}

impl std::fmt::Debug for ConfigParams {
    // Values may be passwords or tokens: only the keys are shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigParams")
            .field("keys", &self.values.keys().collect::<Vec<_>>())
            .field("obscure", &self.obscure)
            .finish()
    }
}

/// What the config state machine answered.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigStep {
    Done,
    /// rclone refused the last answer (e.g. a token it can't decode) and asks the same
    /// question again; it does not repeat the question itself.
    Rejected {
        state: String,
        error: String,
    },
    Question {
        state: String,
        option: ProviderOption,
        error: Option<String>,
    },
}

#[derive(thiserror::Error, Debug, Clone, PartialEq)]
pub enum BackendError {
    #[error("{0}")]
    Rclone(String),
    #[error("lost connection to rclone")]
    Disconnected,
    #[error("{0}")]
    Other(String),
}

impl From<RcError> for BackendError {
    fn from(e: RcError) -> Self {
        match e {
            RcError::Connect(_) => BackendError::Disconnected,
            RcError::Api { message, .. } => BackendError::Rclone(message),
            other => BackendError::Other(other.to_string()),
        }
    }
}

/// Starting a transfer failed part-way: the jobs in `started` are running anyway.
#[derive(Debug, Clone, PartialEq)]
pub struct StartError {
    pub started: Vec<u64>,
    pub error: BackendError,
    /// The request that failed may have reached rclone (its answer was lost): more
    /// jobs than `started` may be running. Ask rclone (by group) before concluding.
    pub uncertain: bool,
}

impl StartError {
    /// A failure known to have started nothing past `started`.
    pub fn new(started: Vec<u64>, error: BackendError) -> Self {
        StartError {
            started,
            error,
            uncertain: false,
        }
    }
}

impl From<BackendError> for StartError {
    fn from(error: BackendError) -> Self {
        StartError::new(Vec::new(), error)
    }
}

impl From<RcError> for StartError {
    fn from(e: RcError) -> Self {
        BackendError::from(e).into()
    }
}

#[async_trait]
pub trait Backend: Send + Sync {
    async fn remotes(&self) -> Result<Vec<RemoteInfo>, BackendError>;
    async fn list(&self, dir: &Location) -> Result<Vec<Entry>, BackendError>;
    async fn stat(&self, loc: &Location) -> Result<Option<Entry>, BackendError>;
    async fn about(&self, remote: &Remote) -> Result<About, BackendError>;
    async fn mkdir(&self, loc: &Location) -> Result<(), BackendError>;
    async fn rename(&self, from: &Location, new_name: &str, is_dir: bool) -> Result<(), BackendError>;
    async fn delete(&self, loc: &Location, is_dir: bool) -> Result<(), BackendError>;
    /// Launch the async rc jobs of `spec` (one per [`Backend::job_units`] entry, in that
    /// order), all in `spec.group()`. `on_job` hears each id as soon as rclone accepts
    /// it, so it can be recorded before the rest have started. Returns the job ids.
    async fn start_transfer(
        &self,
        spec: &TransferSpec,
        on_job: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<Vec<u64>, StartError>;
    /// The items (indices) each job of `spec` covers, in the order they are started.
    fn job_units(&self, spec: &TransferSpec) -> Vec<Vec<usize>> {
        spec.job_units()
    }
    /// Ids of the jobs rclone knows in `group`, oldest first (finished ones included).
    async fn group_jobs(&self, group: &str) -> Result<Vec<u64>, BackendError>;
    async fn transfer_progress(&self, group: &str, jobids: &[u64]) -> Result<TransferProgress, BackendError>;
    async fn stop_transfer(&self, group: &str) -> Result<(), BackendError>;
    async fn shutdown(&self) -> Result<(), BackendError>;
    /// Limit transfer speed (rclone syntax, `off` for none); returns the rate rclone reports.
    async fn set_bwlimit(&self, rate: &str) -> Result<String, BackendError>;
    /// The current limit (`off` when none).
    async fn get_bwlimit(&self) -> Result<String, BackendError>;

    /// Every backend type rclone knows (`config/providers`).
    async fn providers(&self) -> Result<Vec<Provider>, BackendError>;
    /// The saved values of one account (`config/get`).
    async fn remote_config(
        &self,
        name: &str,
    ) -> Result<std::collections::BTreeMap<String, String>, BackendError>;
    async fn config_create(
        &self,
        name: &str,
        kind: &str,
        params: ConfigParams,
    ) -> Result<ConfigStep, BackendError>;
    async fn config_update(&self, name: &str, params: ConfigParams) -> Result<ConfigStep, BackendError>;
    /// Answer the question asked with `state` (`config/update` with `continue`; the
    /// answer travels as `opt.result`). `obscure`: the answer is a password.
    async fn config_continue(
        &self,
        name: &str,
        state: &str,
        answer: &str,
        obscure: bool,
    ) -> Result<ConfigStep, BackendError>;
    /// Like [`Backend::config_continue`], as an rc job (for answers that wait on a browser).
    async fn config_continue_job(
        &self,
        name: &str,
        state: &str,
        answer: &str,
        obscure: bool,
    ) -> Result<u64, BackendError>;
    /// `None` while the job runs.
    async fn config_job(&self, jobid: u64) -> Result<Option<Result<ConfigStep, BackendError>>, BackendError>;
    async fn stop_job(&self, jobid: u64) -> Result<(), BackendError>;
    async fn config_delete(&self, name: &str) -> Result<(), BackendError>;
}

/// For each item of a move: has it arrived already? Only when its source is gone and
/// the destination holds the same kind of entry, with the size it had (files). A
/// source gone without such a destination stays in the move, which then fails on it.
pub async fn moved_already(backend: &dyn Backend, spec: &TransferSpec) -> Result<Vec<bool>, BackendError> {
    let mut out = Vec::with_capacity(spec.items.len());
    for item in &spec.items {
        if backend.stat(&spec.src_dir.join(&item.name)).await?.is_some() {
            out.push(false);
            continue;
        }
        let dst = backend.stat(&spec.dst_dir.join(&item.dst_name)).await?;
        out.push(dst.is_some_and(|d| {
            d.is_dir == item.is_dir && (item.is_dir || (item.size.is_some() && d.size == item.size))
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio() {
        let a = About {
            total: Some(200),
            used: Some(50),
            ..Default::default()
        };
        assert_eq!(a.used_ratio(), Some(0.25));
        assert_eq!(
            About {
                total: Some(0),
                used: Some(0),
                ..Default::default()
            }
            .used_ratio(),
            None
        );
        assert_eq!(About::default().used_ratio(), None);
    }
}
