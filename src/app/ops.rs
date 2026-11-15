//! File operations started from the pane: rename, new folder, delete, yank/cut/paste.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};

use super::overlay::{InputOutcome, InputPurpose, LineInput, Overlay, PendingOp, PendingPaste};
use super::paste::{PasteError, Resolution, build_items, check_paste, conflicts, kind_clash};
use super::register::Register;
use super::{App, AppEvent, MessageKind};
use crate::backend::Entry;
use crate::command::{Op, equivalent};
use crate::location::Location;
use crate::transfer::spec::{TransferKind, TransferSpec};

/// Names listed in a delete confirmation before "…and N more".
const CONFIRM_LIST: usize = 10;

fn paste_error_text(e: PasteError) -> String {
    match e {
        PasteError::IntoItself(name) => format!("Can't paste “{name}” inside itself"),
        PasteError::SameFolderMove => "Those items are already in this folder".to_string(),
    }
}

pub(super) fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

impl App {
    // ---- rename / mkdir ---------------------------------------------------

    pub(super) fn start_rename(&mut self) {
        let Some(entry) = self.pane.current().cloned() else {
            return;
        };
        let stem_end = match entry.name.rfind('.') {
            Some(i) if i > 0 && !entry.is_dir => entry.name[..i].chars().count(),
            _ => entry.name.chars().count(),
        };
        self.overlay = Some(Overlay::Input {
            title: format!("Rename {}", if entry.is_dir { "folder" } else { "file" }),
            input: LineInput::new(&entry.name, stem_end),
            purpose: InputPurpose::Rename(entry),
        });
    }

    pub(super) fn start_mkdir(&mut self) {
        self.overlay = Some(Overlay::Input {
            title: "New folder".into(),
            input: LineInput::default(),
            purpose: InputPurpose::Mkdir,
        });
    }

    /// A name typed by the user, or the message explaining why it is refused.
    fn validate_name(&self, name: &str, current: Option<&str>) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." {
            return Err("Please type a name".into());
        }
        if name.contains('/') {
            return Err("A name can't contain “/”".into());
        }
        if Some(name) != current && self.pane.has_name(name) {
            return Err(format!("“{name}” already exists here"));
        }
        Ok(name.to_string())
    }

    pub(super) fn input_key(&mut self, key: KeyEvent) {
        let Some(Overlay::Input { input, purpose, .. }) = self.overlay.as_mut() else {
            return;
        };
        let outcome = input.handle(key);
        let value = input.value.clone();
        let purpose = purpose.clone();
        match outcome {
            InputOutcome::Changed if purpose == InputPurpose::Filter => self.pane.set_filter(&value),
            InputOutcome::Changed if purpose == InputPurpose::SpaceFilter => self.set_space_filter(&value),
            InputOutcome::Cancel => {
                if purpose == InputPurpose::Filter {
                    self.pane.set_filter("");
                }
                if purpose == InputPurpose::SpaceFilter {
                    self.set_space_filter("");
                }
                self.overlay = None;
            }
            InputOutcome::Submit => {
                self.overlay = None;
                match purpose {
                    InputPurpose::Filter | InputPurpose::SpaceFilter => {}
                    InputPurpose::Rename(entry) => self.submit_rename(entry, &value),
                    InputPurpose::Mkdir => self.submit_mkdir(&value),
                }
            }
            _ => {}
        }
    }

    fn submit_rename(&mut self, entry: Entry, value: &str) {
        let name = match self.validate_name(value, Some(&entry.name)) {
            Ok(n) if n == entry.name => return,
            Ok(n) => n,
            Err(msg) => return self.info(msg, MessageKind::Error),
        };
        let from = self.pane.loc.join(&entry.name);
        let dir = self.pane.loc.clone();
        let backend = self.backend.clone();
        self.spawn_op(async move {
            let result = backend.rename(&from, &name, entry.is_dir).await;
            AppEvent::OpDone {
                what: format!("Renamed to {name}"),
                result,
                refresh: vec![dir],
                cursor: Some(name),
            }
        });
    }

    fn submit_mkdir(&mut self, value: &str) {
        let name = match self.validate_name(value, None) {
            Ok(n) => n,
            Err(msg) => return self.info(msg, MessageKind::Error),
        };
        let loc = self.pane.loc.join(&name);
        let dir = self.pane.loc.clone();
        let backend = self.backend.clone();
        self.spawn_op(async move {
            let result = backend.mkdir(&loc).await;
            AppEvent::OpDone {
                what: format!("Created {name}"),
                result,
                refresh: vec![dir],
                cursor: Some(name),
            }
        });
    }

    // ---- delete -----------------------------------------------------------

    pub(super) fn start_delete(&mut self) {
        let items = self.pane.selection();
        if items.is_empty() {
            return;
        }
        let targets: Vec<(Location, bool)> = items
            .iter()
            .map(|e| (self.pane.loc.join(&e.name), e.is_dir))
            .collect();
        let mut lines: Vec<String> = items
            .iter()
            .take(CONFIRM_LIST)
            .map(|e| {
                if e.is_dir {
                    format!("{}/", e.name)
                } else {
                    e.name.clone()
                }
            })
            .collect();
        if items.len() > CONFIRM_LIST {
            lines.push(format!("…and {} more", items.len() - CONFIRM_LIST));
        }
        let commands: Vec<String> = targets
            .iter()
            .take(CONFIRM_LIST)
            .flat_map(|(loc, is_dir)| equivalent(&Op::Delete { loc, is_dir: *is_dir }))
            .collect();
        let title = match items.as_slice() {
            [one] => format!("Delete “{}”?", one.name),
            many => format!("Delete {}?", plural(many.len(), "item")),
        };
        self.overlay = Some(Overlay::Confirm {
            title,
            lines,
            commands,
            danger: true,
            on_yes: PendingOp::Delete(targets),
        });
    }

    pub(super) fn confirm_key(&mut self, key: KeyEvent) {
        let Some(Overlay::Confirm { on_yes, .. }) = self.overlay.as_ref() else {
            return;
        };
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let op = on_yes.clone();
                self.overlay = None;
                self.run_pending(op);
            }
            KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => self.overlay = None,
            _ => {}
        }
    }

    fn run_pending(&mut self, op: PendingOp) {
        match op {
            PendingOp::Rescan(_) | PendingOp::ReplaceScan(_) => self.run_scan_op(op),
            PendingOp::RemoveAccount { name } => self.remove_account(name),
            PendingOp::RemoveTransfers(ids) => self.remove_transfers(&ids),
            PendingOp::Delete(targets) => {
                self.pane.clear_selection();
                let n = targets.len();
                let dir = self.pane.loc.clone();
                let backend = self.backend.clone();
                self.spawn_op(async move {
                    let mut result = Ok(());
                    for (loc, is_dir) in &targets {
                        if let Err(e) = backend.delete(loc, *is_dir).await {
                            result = Err(e);
                            break;
                        }
                    }
                    AppEvent::OpDone {
                        what: format!("Deleted {}", plural(n, "item")),
                        result,
                        refresh: vec![dir],
                        cursor: None,
                    }
                });
            }
        }
    }

    // ---- yank / cut / paste ----------------------------------------------

    pub(super) fn yank(&mut self, kind: TransferKind) {
        let items = self.pane.selection();
        if items.is_empty() {
            return;
        }
        let n = items.len();
        self.register = Some(Register::new(kind, self.pane.loc.clone(), items));
        self.pane.clear_selection();
        let verb = if kind == TransferKind::Copy {
            "copied"
        } else {
            "cut"
        };
        self.info(
            format!("{} {verb} — go to the destination and press p", plural(n, "item")),
            MessageKind::Info,
        );
    }

    pub(super) fn paste(&mut self) {
        let Some(register) = self.register.clone() else {
            return self.info(
                "Nothing to paste — select with y (copy) or x (cut) first",
                MessageKind::Info,
            );
        };
        let dst_dir = self.pane.loc.clone();
        if let Err(e) = check_paste(&register, &dst_dir) {
            return self.info(paste_error_text(e), MessageKind::Error);
        }
        self.check_destination(register, dst_dir, false);
    }

    /// Copy or move the selection straight into the other pane's folder.
    pub(super) fn send_to_other(&mut self, kind: TransferKind) {
        let Some(dst_dir) = self.other.as_ref().map(|o| o.loc.clone()) else {
            return self.info("Open a second pane with | first", MessageKind::Info);
        };
        let items = self.pane.selection();
        if items.is_empty() {
            return;
        }
        let register = Register::new(kind, self.pane.loc.clone(), items);
        if let Err(e) = check_paste(&register, &dst_dir) {
            return self.info(paste_error_text(e), MessageKind::Error);
        }
        self.pane.clear_selection();
        self.check_destination(register, dst_dir, true);
    }

    /// List the destination to find conflicts before launching the transfer.
    fn check_destination(&mut self, register: Register, dst_dir: Location, direct: bool) {
        let backend = self.backend.clone();
        self.launching += 1;
        self.spawn(async move {
            let result = backend.list(&dst_dir).await;
            AppEvent::PasteChecked {
                register,
                dst_dir,
                result,
                direct,
            }
        });
    }

    pub(super) fn paste_checked(
        &mut self,
        register: Register,
        dst_dir: Location,
        dst_entries: Vec<Entry>,
        direct: bool,
    ) {
        let names = conflicts(&register.items, &dst_entries);
        let taken_lower = dst_entries.iter().map(|e| e.name.to_lowercase()).collect();
        let pending = PendingPaste {
            register,
            dst_dir,
            dst_entries,
            taken_lower,
            direct,
        };
        if names.is_empty() {
            return self.launch_paste(pending, HashMap::new());
        }
        if self.quit_when_launched {
            // No one is left to answer the conflict question.
            return self.info("Paste dropped: the app is quitting", MessageKind::Info);
        }
        let same_folder = pending.register.src_dir == pending.dst_dir;
        let default = if same_folder {
            Resolution::Rename
        } else {
            Resolution::Skip
        };
        self.ask(Overlay::Conflict {
            names,
            index: 0,
            apply_all: false,
            default,
            chosen: HashMap::new(),
            pending,
        });
    }

    /// Whether the name being asked about is a file against a folder (or the reverse).
    pub fn conflict_clash(&self) -> bool {
        let Some(Overlay::Conflict {
            names,
            index,
            pending,
            ..
        }) = &self.overlay
        else {
            return false;
        };
        let Some(name) = names.get(*index) else {
            return false;
        };
        pending
            .register
            .items
            .iter()
            .find(|e| &e.name == name)
            .is_some_and(|e| kind_clash(e, &pending.dst_entries))
    }

    pub(super) fn conflict_key(&mut self, key: KeyEvent) {
        let clash = self.conflict_clash();
        let Some(Overlay::Conflict {
            names,
            index,
            apply_all,
            default,
            chosen,
            ..
        }) = self.overlay.as_mut()
        else {
            return;
        };
        let choice = match key.code {
            KeyCode::Char('r') if clash => return,
            KeyCode::Char('r') => Resolution::Replace,
            KeyCode::Char('s') => Resolution::Skip,
            KeyCode::Char('n') => Resolution::Rename,
            KeyCode::Enter => *default,
            KeyCode::Char('a') => {
                *apply_all = !*apply_all;
                return;
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlay = None;
                return self.info("Paste cancelled", MessageKind::Info);
            }
            _ => return,
        };
        if *apply_all {
            for name in &names[*index..] {
                chosen.insert(name.clone(), choice);
            }
            *index = names.len();
        } else {
            chosen.insert(names[*index].clone(), choice);
            *index += 1;
        }
        if *index >= names.len()
            && let Some(Overlay::Conflict { chosen, pending, .. }) = self.overlay.take()
        {
            self.launch_paste(pending, chosen);
        }
    }

    fn launch_paste(&mut self, pending: PendingPaste, chosen: HashMap<String, Resolution>) {
        let items = build_items(&pending.register.items, &chosen, &pending.dst_entries);
        if items.is_empty() {
            return self.info("Nothing to paste — every item was skipped", MessageKind::Info);
        }
        if pending.register.kind == TransferKind::Move && !pending.direct {
            self.register = None;
        }
        let spec = TransferSpec {
            id: self.store.next_id(),
            kind: pending.register.kind,
            src_dir: pending.register.src_dir,
            dst_dir: pending.dst_dir,
            items,
        };
        self.launch_transfer(spec);
    }
}
