//! The accounts list on the left.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::backend::{About, RemoteInfo};
use crate::location::Remote;

/// How long a usage figure is considered fresh.
pub const ABOUT_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Default)]
pub struct SidebarState {
    pub remotes: Vec<RemoteInfo>,
    pub cursor: usize,
    pub about: HashMap<Remote, (About, Instant)>,
    /// Remotes whose `about` failed (shown as unknown, not retried until refresh).
    pub about_failed: HashMap<Remote, String>,
    /// Remotes whose `about` was asked for and has not answered yet, with the
    /// request's number: an answer to an older request is dropped.
    pub about_pending: HashMap<Remote, u64>,
}

impl SidebarState {
    pub fn current(&self) -> Option<&RemoteInfo> {
        self.remotes.get(self.cursor)
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.remotes.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn needs_about(&self, remote: &Remote, now: Instant) -> bool {
        if self.about_pending.contains_key(remote) {
            return false;
        }
        match self.about.get(remote) {
            Some((_, at)) => now.duration_since(*at) > ABOUT_TTL,
            None => !self.about_failed.contains_key(remote),
        }
    }

    pub fn select(&mut self, remote: &Remote) {
        if let Some(i) = self.remotes.iter().position(|r| &r.remote == remote) {
            self.cursor = i;
        }
    }
}
