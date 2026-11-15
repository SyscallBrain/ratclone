//! A transfer request and its live progress.

use serde::{Deserialize, Serialize};

use crate::format::{truncate_middle, truncate_width};
use crate::location::Location;
use crate::rc::types::RcJobStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferKind {
    Copy,
    Move,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransferItem {
    pub name: String,
    pub is_dir: bool,
    pub dst_name: String,
    /// The source's size when the transfer was planned (files only): evidence, on resume,
    /// that a destination with this name is the item that was moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// The user chose to replace what has this name at the destination: it must be
    /// written even when rclone would think it equal (same size and time).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace: bool,
}

impl TransferItem {
    /// The name at the destination, with `/` after a folder.
    pub fn label(&self) -> String {
        if self.is_dir {
            format!("{}/", self.dst_name)
        } else {
            self.dst_name.clone()
        }
    }

    /// Can travel in a batched (`--files-from-raw`) job: a file keeping its name.
    /// The list is read a line at a time and a line's final `\r` is dropped, so a
    /// name holding either character could not be told apart: it goes on its own.
    pub fn batchable(&self) -> bool {
        !self.is_dir && self.name == self.dst_name && !self.name.contains(['\n', '\r'])
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransferSpec {
    pub id: u64,
    pub kind: TransferKind,
    pub src_dir: Location,
    pub dst_dir: Location,
    pub items: Vec<TransferItem>,
}

impl TransferSpec {
    /// The rc stats group shared by all jobs of this transfer.
    pub fn group(&self) -> String {
        format!("rtui/{}", self.id)
    }

    /// How [`crate::backend::rc::RcBackend`] splits the items into rc jobs, in the order
    /// it starts them: files keeping their name share one job (when there are two or
    /// more), every other item gets its own. Each entry lists item indices.
    pub fn job_units(&self) -> Vec<Vec<usize>> {
        let batch: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.batchable())
            .map(|(n, _)| n)
            .collect();
        let batched = batch.len() >= 2;
        let mut units = Vec::with_capacity(self.items.len());
        if batched {
            units.push(batch);
        }
        for (n, item) in self.items.iter().enumerate() {
            if !(batched && item.batchable()) {
                units.push(vec![n]);
            }
        }
        units
    }

    /// Short label: `"video.mkv → work:/"` or `"a.pdf +2 → work:/Docs"`.
    pub fn title(&self) -> String {
        format!(
            "{}{} → {}",
            self.first_name(),
            self.more(),
            self.dst_dir.display()
        )
    }

    /// [`Self::title`] in at most `width` columns: the first name is cut in the middle
    /// so the count and the destination stay.
    pub fn title_fit(&self, width: usize) -> String {
        use unicode_width::UnicodeWidthStr;
        let rest = format!("{} → {}", self.more(), self.dst_dir.display());
        let name_w = width.saturating_sub(rest.width()).max(8);
        let title = format!("{}{rest}", truncate_middle(&self.first_name(), name_w));
        truncate_width(&title, width)
    }

    /// The first item's name at the destination (`/` after a folder).
    pub fn first_name(&self) -> String {
        self.items.first().map(TransferItem::label).unwrap_or_default()
    }

    /// `" +2"` when there are other items, else empty.
    pub fn more(&self) -> String {
        match self.items.len() {
            0 | 1 => String::new(),
            n => format!(" +{}", n - 1),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobsState {
    Running,
    Succeeded,
    Failed(String),
    NotFound,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileProgress {
    pub name: String,
    pub bytes: u64,
    pub size: Option<u64>,
    pub percentage: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransferProgress {
    pub state: JobsState,
    pub bytes: u64,
    pub total_bytes: u64,
    pub speed: f64,
    pub eta: Option<u64>,
    pub errors: u64,
    pub files: Vec<FileProgress>,
    /// Jobs of this transfer known to have finished successfully.
    pub done_jobs: Vec<u64>,
}

/// Combine the status of every rc job of one transfer (`None` = job unknown to the rcd).
pub fn aggregate(statuses: &[Option<RcJobStatus>]) -> JobsState {
    if statuses.iter().any(Option::is_none) {
        return JobsState::NotFound;
    }
    let statuses: Vec<&RcJobStatus> = statuses.iter().flatten().collect();
    if statuses.iter().any(|s| !s.finished) {
        return JobsState::Running;
    }
    if statuses.iter().all(|s| s.success) {
        return JobsState::Succeeded;
    }
    let msg = statuses
        .iter()
        .find(|s| !s.error.is_empty())
        .map(|s| s.error.clone())
        .unwrap_or_else(|| "transfer failed".to_string());
    JobsState::Failed(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Remote;

    fn st(finished: bool, success: bool, error: &str) -> Option<RcJobStatus> {
        Some(RcJobStatus {
            id: 1,
            finished,
            success,
            error: error.into(),
            group: String::new(),
        })
    }

    #[test]
    fn aggregate_states() {
        assert_eq!(
            aggregate(&[st(true, true, ""), st(false, false, "")]),
            JobsState::Running
        );
        assert_eq!(
            aggregate(&[st(true, true, ""), st(true, true, "")]),
            JobsState::Succeeded
        );
        assert_eq!(
            aggregate(&[st(true, true, ""), st(true, false, "boom")]),
            JobsState::Failed("boom".into())
        );
        assert_eq!(aggregate(&[st(false, false, ""), None]), JobsState::NotFound);
        assert_eq!(
            aggregate(&[st(true, false, "")]),
            JobsState::Failed("transfer failed".into())
        );
    }

    #[test]
    fn group_and_title() {
        let item = |n: &str| TransferItem {
            replace: false,
            name: n.into(),
            is_dir: false,
            dst_name: n.into(),
            size: None,
        };
        let mut s = TransferSpec {
            id: 7,
            kind: TransferKind::Copy,
            src_dir: Location::new(Remote::Local, "home/t"),
            dst_dir: Location::new(Remote::Named("work".into()), ""),
            items: vec![item("video.mkv")],
        };
        assert_eq!(s.group(), "rtui/7");
        assert_eq!(s.title(), "video.mkv → work:/");
        s.items = vec![item("a"), item("b"), item("c")];
        s.dst_dir = Location::new(Remote::Named("work".into()), "Docs");
        assert_eq!(s.title(), "a +2 → work:/Docs");
        s.items[0].is_dir = true;
        assert_eq!(s.title(), "a/ +2 → work:/Docs");
        s.items[0] = item("a very long file name.tar.gz");
        assert_eq!(s.title_fit(26), "a very….gz +2 → work:/Docs");
        assert_eq!(s.title_fit(26).chars().count(), 26);
    }

    #[test]
    fn job_units_batch_files_that_keep_their_name() {
        let item = |n: &str, is_dir: bool, dst: &str| TransferItem {
            replace: false,
            name: n.into(),
            is_dir,
            dst_name: dst.into(),
            size: None,
        };
        let mut s = TransferSpec {
            id: 1,
            kind: TransferKind::Move,
            src_dir: Location::new(Remote::Local, "home/t"),
            dst_dir: Location::new(Remote::Named("g".into()), ""),
            items: vec![
                item("a", false, "a"),
                item("D", true, "D"),
                item("b", false, "b (1)"),
                item("c", false, "c"),
            ],
        };
        assert_eq!(s.job_units(), [vec![0, 3], vec![1], vec![2]]);
        s.items.truncate(2);
        assert_eq!(s.job_units(), [vec![0], vec![1]], "one file is not batched");
    }
}
