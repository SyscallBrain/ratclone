//! Modal dialogs drawn over the main screen.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::paste::Resolution;
use super::register::Register;
use crate::backend::Entry;
use crate::location::Location;

/// A single-line text field with a cursor (in characters).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LineInput {
    pub value: String,
    pub cursor: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputOutcome {
    Changed,
    Submit,
    Cancel,
    Ignored,
}

impl LineInput {
    pub fn new(value: &str, cursor: usize) -> Self {
        LineInput {
            value: value.to_string(),
            cursor: cursor.min(value.chars().count()),
        }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.value.len())
    }

    pub fn handle(&mut self, key: KeyEvent) -> InputOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => InputOutcome::Submit,
            KeyCode::Esc => InputOutcome::Cancel,
            KeyCode::Char('u') if ctrl => {
                let b = self.byte_at(self.cursor);
                self.value.replace_range(..b, "");
                self.cursor = 0;
                InputOutcome::Changed
            }
            KeyCode::Char('w') if ctrl => {
                let chars: Vec<char> = self.value.chars().collect();
                let mut start = self.cursor;
                while start > 0 && chars[start - 1] == ' ' {
                    start -= 1;
                }
                while start > 0 && chars[start - 1] != ' ' {
                    start -= 1;
                }
                let (a, b) = (self.byte_at(start), self.byte_at(self.cursor));
                self.value.replace_range(a..b, "");
                self.cursor = start;
                InputOutcome::Changed
            }
            KeyCode::Char('a') if ctrl => {
                self.cursor = 0;
                InputOutcome::Ignored
            }
            KeyCode::Char('e') if ctrl => {
                self.cursor = self.value.chars().count();
                InputOutcome::Ignored
            }
            KeyCode::Char(c) if !ctrl => {
                let b = self.byte_at(self.cursor);
                self.value.insert(b, c);
                self.cursor += 1;
                InputOutcome::Changed
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let (a, b) = (self.byte_at(self.cursor - 1), self.byte_at(self.cursor));
                self.value.replace_range(a..b, "");
                self.cursor -= 1;
                InputOutcome::Changed
            }
            KeyCode::Delete if self.cursor < self.value.chars().count() => {
                let (a, b) = (self.byte_at(self.cursor), self.byte_at(self.cursor + 1));
                self.value.replace_range(a..b, "");
                InputOutcome::Changed
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                InputOutcome::Ignored
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.value.chars().count());
                InputOutcome::Ignored
            }
            KeyCode::Home => {
                self.cursor = 0;
                InputOutcome::Ignored
            }
            KeyCode::End => {
                self.cursor = self.value.chars().count();
                InputOutcome::Ignored
            }
            _ => InputOutcome::Ignored,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputPurpose {
    Filter,
    SpaceFilter,
    Rename(Entry),
    Mkdir,
}

/// What a confirmed dialog does.
#[derive(Clone, Debug, PartialEq)]
pub enum PendingOp {
    /// `(location, is_dir)` of each entry to delete.
    Delete(Vec<(Location, bool)>),
    /// Scan this root again.
    Rescan(Location),
    /// Cancel the running scan and scan this root.
    ReplaceScan(Location),
    /// Delete an account from rclone's config.
    RemoveAccount { name: String },
    /// Drop these transfers from the list (their files are left alone).
    RemoveTransfers(Vec<u64>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingPaste {
    pub register: Register,
    pub dst_dir: Location,
    pub dst_entries: Vec<Entry>,
    /// Lower-cased names of `dst_entries`, for the dialog's suggested new names.
    pub taken_lower: std::collections::HashSet<String>,
    /// Sent straight to the other pane: the register is left alone.
    pub direct: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    Input {
        title: String,
        input: LineInput,
        purpose: InputPurpose,
    },
    Confirm {
        title: String,
        lines: Vec<String>,
        commands: Vec<String>,
        danger: bool,
        on_yes: PendingOp,
    },
    /// One question per clashing name, unless "apply to all" is on.
    Conflict {
        names: Vec<String>,
        index: usize,
        apply_all: bool,
        default: Resolution,
        chosen: HashMap<String, Resolution>,
        pending: PendingPaste,
    },
    Quit {
        /// Transfers and file operations still going.
        running: usize,
        /// Of `running`: deletes, renames… done by the app itself, which must stay
        /// open until they end.
        ops: usize,
    },
    /// The key list, scrolled down `scroll` lines.
    Help {
        scroll: usize,
    },
    Settings(Box<super::settings::SettingsDialog>),
    Account(Box<super::account::AccountWizard>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn editing_unicode() {
        let mut i = LineInput::new("日本.txt", 2);
        assert_eq!(i.handle(k(KeyCode::Char('語'))), InputOutcome::Changed);
        assert_eq!(i.value, "日本語.txt");
        i.handle(k(KeyCode::Backspace));
        i.handle(k(KeyCode::Backspace));
        assert_eq!(i.value, "日.txt");
        assert_eq!(i.cursor, 1);
        i.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(i.value, ".txt");
        i.handle(k(KeyCode::End));
        i.handle(k(KeyCode::Char(' ')));
        i.handle(k(KeyCode::Char('b')));
        i.handle(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(i.value, ".txt ");
        assert_eq!(i.handle(k(KeyCode::Enter)), InputOutcome::Submit);
        assert_eq!(i.handle(k(KeyCode::Esc)), InputOutcome::Cancel);
    }
}
