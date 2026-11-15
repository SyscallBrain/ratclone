//! The yank/cut register.

use std::collections::HashSet;

use crate::backend::Entry;
use crate::location::Location;
use crate::transfer::spec::TransferKind;

#[derive(Clone, Debug, PartialEq)]
pub struct Register {
    pub kind: TransferKind,
    pub src_dir: Location,
    pub items: Vec<Entry>,
    /// Names of `items`, for the per-row lookup while drawing.
    names: HashSet<String>,
}

impl Register {
    pub fn new(kind: TransferKind, src_dir: Location, items: Vec<Entry>) -> Self {
        let names = items.iter().map(|e| e.name.clone()).collect();
        Register {
            kind,
            src_dir,
            items,
            names,
        }
    }

    pub fn contains(&self, dir: &Location, name: &str) -> bool {
        &self.src_dir == dir && self.names.contains(name)
    }
}
