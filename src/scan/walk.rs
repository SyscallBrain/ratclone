//! The folder walker: lists folders breadth-first with a bounded number of
//! requests in flight and reports each listing as an [`AppEvent::Scan`].

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Semaphore;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::{JoinHandle, JoinSet};

use crate::app::AppEvent;
use crate::backend::{Backend, BackendError, Entry};
use crate::location::Location;

#[derive(Debug)]
pub enum ScanMsg {
    /// The listing of folder `rel` (relative to the scan root, `""` for the root).
    Listed {
        rel: String,
        result: Result<Vec<Entry>, BackendError>,
    },
    /// Sent exactly once, last.
    Finished { cancelled: bool },
}

/// A running walk. Dropping it does not stop it; call [`ScanHandle::cancel`].
pub struct ScanHandle {
    pub id: u64,
    cancelled: Arc<AtomicBool>,
    task: JoinHandle<()>,
    tx: UnboundedSender<AppEvent>,
}

impl ScanHandle {
    /// Stop listing: requests in flight are dropped and `Finished { cancelled: true }` is sent.
    pub fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::SeqCst) {
            self.task.abort();
            let _ = self.tx.send(AppEvent::Scan {
                id: self.id,
                msg: ScanMsg::Finished { cancelled: true },
            });
        }
    }
}

impl std::fmt::Debug for ScanHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanHandle").field("id", &self.id).finish()
    }
}

fn location_of(root: &Location, rel: &str) -> Location {
    if rel.is_empty() {
        root.clone()
    } else {
        root.join(rel)
    }
}

pub fn start_walk(
    backend: Arc<dyn Backend>,
    root: Location,
    concurrency: usize,
    id: u64,
    tx: UnboundedSender<AppEvent>,
) -> ScanHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let out = tx.clone();
    let task = tokio::spawn(async move {
        let permits = Arc::new(Semaphore::new(concurrency.max(1)));
        let mut queue: VecDeque<String> = VecDeque::from([String::new()]);
        let mut running: JoinSet<(String, Result<Vec<Entry>, BackendError>)> = JoinSet::new();
        // Which folder each task lists, so a task that panics is still accounted for.
        let mut names: HashMap<tokio::task::Id, String> = HashMap::new();
        type Done =
            Result<(tokio::task::Id, (String, Result<Vec<Entry>, BackendError>)), tokio::task::JoinError>;
        let settle = |done: Done, names: &mut HashMap<tokio::task::Id, String>| match done {
            Ok((id, (rel, result))) => {
                names.remove(&id);
                Some((rel, result))
            }
            Err(e) if e.is_panic() => names
                .remove(&e.id())
                .map(|rel| (rel, Err(BackendError::Other("listing crashed".into())))),
            Err(e) => {
                names.remove(&e.id());
                None
            }
        };
        loop {
            while let Some(rel) = queue.pop_front() {
                let Ok(permit) = permits.clone().acquire_owned().await else {
                    return;
                };
                let backend = backend.clone();
                let loc = location_of(&root, &rel);
                let name = rel.clone();
                let handle = running.spawn(async move {
                    let result = backend.list(&loc).await;
                    drop(permit);
                    (rel, result)
                });
                names.insert(handle.id(), name);
                // Take in finished listings so their folders join the queue early.
                while let Some(done) = running.try_join_next_with_id() {
                    if let Some((rel, result)) = settle(done, &mut names) {
                        enqueue(&mut queue, &rel, &result);
                        if !send(&out, &flag, id, rel, result) {
                            return;
                        }
                    }
                }
            }
            match running.join_next_with_id().await {
                Some(done) => {
                    if let Some((rel, result)) = settle(done, &mut names) {
                        enqueue(&mut queue, &rel, &result);
                        if !send(&out, &flag, id, rel, result) {
                            return;
                        }
                    }
                }
                None => break,
            }
        }
        if !flag.swap(true, Ordering::SeqCst) {
            let _ = out.send(AppEvent::Scan {
                id,
                msg: ScanMsg::Finished { cancelled: false },
            });
        }
    });
    ScanHandle {
        id,
        cancelled,
        task,
        tx,
    }
}

fn enqueue(queue: &mut VecDeque<String>, rel: &str, result: &Result<Vec<Entry>, BackendError>) {
    if let Ok(entries) = result {
        for e in entries.iter().filter(|e| e.is_dir) {
            queue.push_back(if rel.is_empty() {
                e.name.clone()
            } else {
                format!("{rel}/{}", e.name)
            });
        }
    }
}

/// Report one listing unless the walk was cancelled; `false` means stop.
fn send(
    tx: &UnboundedSender<AppEvent>,
    cancelled: &AtomicBool,
    id: u64,
    rel: String,
    result: Result<Vec<Entry>, BackendError>,
) -> bool {
    if cancelled.load(Ordering::SeqCst) {
        return false;
    }
    tx.send(AppEvent::Scan {
        id,
        msg: ScanMsg::Listed { rel, result },
    })
    .is_ok()
}
