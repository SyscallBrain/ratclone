//! JSON client for `rclone rcd` over a unix socket.

pub mod types;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::UnixStream;

/// Default time limit for a single rc call.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The stats group of every call that does not name one. Left alone, rclone gives
/// each call a group of its own (`job/<id>`) and keeps only the newest 1000: a
/// thousand folder listings later, the group of a transfer still running is thrown
/// away and its progress reads as zero.
pub const DEFAULT_GROUP: &str = "ratclone/ui";

#[derive(thiserror::Error, Debug, Clone)]
pub enum RcError {
    #[error("cannot reach rclone: {0}")]
    Connect(String),
    #[error("{message}")]
    Api {
        status: u16,
        message: String,
        path: String,
    },
    #[error("bad response from rclone: {0}")]
    Decode(String),
    #[error("rclone took too long")]
    Timeout,
}

/// Idle connections kept open for the next calls.
const MAX_IDLE: usize = 8;

type Sender = http1::SendRequest<Full<Bytes>>;

/// Client for one rcd socket. Clones share a small pool of keep-alive connections,
/// so a call usually skips the connect and HTTP handshake.
#[derive(Clone)]
pub struct UnixRc {
    socket: PathBuf,
    idle: Arc<Mutex<Vec<Sender>>>,
}

impl std::fmt::Debug for UnixRc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnixRc").field("socket", &self.socket).finish()
    }
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

/// `params` with `_group` set to [`DEFAULT_GROUP`] unless the caller chose one.
fn with_default_group(mut params: Value) -> Value {
    if let Some(map) = params.as_object_mut()
        && !map.contains_key("_group")
    {
        map.insert("_group".into(), Value::String(DEFAULT_GROUP.into()));
    }
    params
}

impl UnixRc {
    pub fn new(socket: PathBuf) -> Self {
        UnixRc {
            socket,
            idle: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RcError> {
        self.call_with_timeout(method, params, DEFAULT_TIMEOUT).await
    }

    /// `POST /<method>` with a JSON body.
    pub async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RcError> {
        self.call_typed_with_timeout(method, params, timeout).await
    }

    pub async fn call_typed<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, RcError> {
        self.call_typed_with_timeout(method, params, DEFAULT_TIMEOUT)
            .await
    }

    /// Like [`UnixRc::call_with_timeout`], decoding the answer straight into `T`.
    pub async fn call_typed_with_timeout<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<T, RcError> {
        let params = with_default_group(params);
        let body = Bytes::from(serde_json::to_vec(&params).map_err(|e| RcError::Decode(e.to_string()))?);
        let (status, bytes) = tokio::time::timeout(timeout, self.post(method, body))
            .await
            .map_err(|_| RcError::Timeout)??;
        if status != 200 {
            let e: ApiError = serde_json::from_slice(&bytes).map_err(|e| RcError::Decode(e.to_string()))?;
            return Err(RcError::Api {
                status,
                message: e.error.unwrap_or_else(|| "unknown rclone error".into()),
                path: e.path.unwrap_or_else(|| method.to_string()),
            });
        }
        serde_json::from_slice(&bytes).map_err(|e| RcError::Decode(e.to_string()))
    }

    async fn connect(&self) -> Result<Sender, RcError> {
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|e| RcError::Connect(e.to_string()))?;
        let (sender, conn) = http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| RcError::Connect(e.to_string()))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        Ok(sender)
    }

    fn take_idle(&self) -> Option<Sender> {
        let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        while let Some(s) = idle.pop() {
            if !s.is_closed() {
                return Some(s);
            }
        }
        None
    }

    fn put_idle(&self, sender: Sender) {
        if sender.is_closed() {
            return;
        }
        let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        if idle.len() < MAX_IDLE {
            idle.push(sender);
        }
    }

    /// Send one request, on an idle connection when there is one. A pooled
    /// connection the rcd already closed is replaced by a fresh one, but only
    /// when the request provably never left: rc calls are not idempotent.
    async fn post(&self, method: &str, body: Bytes) -> Result<(u16, Bytes), RcError> {
        let mut pooled = self.take_idle();
        loop {
            let reused = pooled.is_some();
            let mut sender = match pooled.take() {
                Some(s) => s,
                None => self.connect().await?,
            };
            if let Err(e) = sender.ready().await {
                if reused {
                    continue;
                }
                return Err(RcError::Connect(e.to_string()));
            }
            let req = Request::builder()
                .method(hyper::Method::POST)
                .uri(format!("/{method}"))
                .header(hyper::header::HOST, "localhost")
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .header(hyper::header::CONTENT_LENGTH, body.len())
                .body(Full::new(body.clone()))
                .map_err(|e| RcError::Decode(e.to_string()))?;
            let resp = match sender.send_request(req).await {
                Ok(r) => r,
                Err(e) if reused && e.is_canceled() => continue,
                Err(e) => return Err(RcError::Connect(e.to_string())),
            };
            let status = resp.status().as_u16();
            let bytes = resp
                .into_body()
                .collect()
                .await
                .map_err(|e| RcError::Connect(e.to_string()))?
                .to_bytes();
            self.put_idle(sender);
            return Ok((status, bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn calls_share_one_stats_group_unless_they_name_their_own() {
        assert_eq!(
            with_default_group(json!({ "fs": "/" })),
            json!({ "fs": "/", "_group": DEFAULT_GROUP })
        );
        assert_eq!(
            with_default_group(json!({ "_group": "rtui/7" })),
            json!({ "_group": "rtui/7" })
        );
        // Not an object: nothing to add to (rclone would refuse it anyway).
        assert_eq!(with_default_group(json!(null)), json!(null));
    }
}
