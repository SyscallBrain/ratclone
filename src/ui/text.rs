//! Fixed strings shown by the renderer, kept together for easy rewording.

pub const ACCOUNTS: &str = "Accounts";
pub const COL_NAME: &str = "Name";
pub const COL_SIZE: &str = "Size";
pub const COL_MODIFIED: &str = "Modified";
pub const LOADING: &str = "Loading…";
pub const EMPTY: &str = "This folder is empty";
pub const NO_MATCH: &str = "Nothing matches the filter";
pub const RETRY: &str = "press R to try again";
pub const HIDDEN_ON: &str = "hidden";
pub const FILTER_HINT: &str = "Enter keep · Esc clear";

/// Key hints in the status line, per focus: `(key, label)`.
pub const PANE_HINTS: &[(&str, &str)] = &[
    ("y", "copy"),
    ("x", "cut"),
    ("p", "paste"),
    ("r", "rename"),
    ("dd", "delete"),
    ("o", "new folder"),
    ("T", "transfers"),
    ("?", "help"),
];
pub const SIDEBAR_HINTS: &[(&str, &str)] = &[
    ("l", "open"),
    ("a", "add account"),
    ("r", "refresh usage"),
    ("Tab", "files"),
    ("T", "transfers"),
    ("?", "help"),
    ("q", "quit"),
];
pub const TRANSFERS_HINTS: &[(&str, &str)] = &[
    ("j/k", "move"),
    ("s", "stop"),
    ("Enter", "resume"),
    ("c", "clear finished"),
    ("dd", "remove"),
    ("T", "close"),
    ("?", "help"),
];

/// Human name of an rclone backend type (`drive` → `Google Drive`).
pub fn remote_kind_label(kind: &str) -> &str {
    match kind {
        "drive" => "Google Drive",
        "mega" => "Mega",
        "dropbox" => "Dropbox",
        "onedrive" => "OneDrive",
        "s3" => "S3",
        "crypt" => "Encrypted",
        "local" => "Local disk",
        other => other,
    }
}

pub const SPACE_HINTS: &[(&str, &str)] = &[
    ("j/k", "move"),
    ("l", "open"),
    ("h", "up"),
    ("R", "rescan"),
    ("gf", "open in Files"),
    ("/", "filter"),
    ("s", "sort"),
    ("?", "help"),
];

pub const SPACE_EMPTY_HINTS: &[(&str, &str)] = &[("Enter", "scan"), ("Tab", "accounts"), ("?", "help")];
