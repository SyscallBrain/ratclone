//! ratclone: a keyboard-driven terminal UI for rclone.

pub mod app;
pub mod backend;
pub mod cli;
pub mod clipboard;
pub mod command;
pub mod config;
pub mod config_write;
pub mod daemon;
pub mod format;
pub mod input;
pub mod location;
pub mod paths;
pub mod rc;
pub mod reaper;
pub mod scan;
pub mod theme;
pub mod transfer;
pub mod ui;

/// Application name used for XDG directories and the rcd socket.
pub const APP_NAME: &str = "ratclone";
