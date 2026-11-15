//! Space scanning: walk an account folder by folder and add up where the bytes go.

pub mod cache;
pub mod tree;
pub mod walk;

/// Folders listed at once: the configured value, else 2 on Mega (it rate-limits) and 4 elsewhere.
pub fn concurrency_for(kind: &str, configured: Option<usize>) -> usize {
    configured
        .unwrap_or(if kind == "mega" { 2 } else { 4 })
        .clamp(1, 16)
}
