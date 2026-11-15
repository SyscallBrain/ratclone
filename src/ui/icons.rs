//! File and account icons: Nerd Font glyphs (Font Awesome range) or plain fallbacks.

use crate::backend::Entry;
use crate::config::IconStyle;

pub fn icon(entry: &Entry, style: IconStyle) -> &'static str {
    if style == IconStyle::Ascii {
        return if entry.is_dir { "▸" } else { "·" };
    }
    if entry.is_dir {
        return "\u{f07b}";
    }
    let ext = entry
        .name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "svg" | "bmp" | "tiff" | "raw" => "\u{f1c5}",
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v" => "\u{f1c8}",
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "opus" => "\u{f1c7}",
        "zip" | "gz" | "xz" | "bz2" | "zst" | "7z" | "rar" | "tar" | "iso" => "\u{f1c6}",
        "pdf" => "\u{f1c1}",
        "rs" | "py" | "js" | "ts" | "go" | "c" | "h" | "cpp" | "java" | "sh" | "json" | "toml" | "yaml"
        | "yml" | "html" | "css" => "\u{f1c9}",
        "txt" | "md" | "doc" | "docx" | "odt" | "rtf" => "\u{f0f6}",
        "xls" | "xlsx" | "ods" | "csv" => "\u{f1c3}",
        "ppt" | "pptx" | "odp" => "\u{f1c4}",
        _ => "\u{f016}",
    }
}

/// Icon for an account by rclone backend type.
pub fn remote_icon(kind: &str, style: IconStyle) -> &'static str {
    if style == IconStyle::Ascii {
        return if kind == "local" { "~" } else { "●" };
    }
    match kind {
        "local" => "\u{f0a0}",
        "drive" => "\u{f1a0}",
        "dropbox" => "\u{f16b}",
        "crypt" => "\u{f023}",
        "s3" | "b2" | "swift" | "azureblob" => "\u{f1c0}",
        _ => "\u{f0c2}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            size: None,
            mod_time: None,
            is_dir,
            mime: String::new(),
        }
    }

    #[test]
    fn by_extension() {
        assert_eq!(icon(&e("a.JPG", false), IconStyle::Nerd), "\u{f1c5}");
        assert_eq!(icon(&e("Photos", true), IconStyle::Nerd), "\u{f07b}");
        assert_eq!(icon(&e("noext", false), IconStyle::Nerd), "\u{f016}");
        assert_eq!(icon(&e("Photos", true), IconStyle::Ascii), "▸");
        assert_eq!(remote_icon("dropbox", IconStyle::Nerd), "\u{f16b}");
        assert_eq!(remote_icon("mega", IconStyle::Ascii), "●");
    }
}
