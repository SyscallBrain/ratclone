//! The `rclone` command line equivalent to each operation, for display.

use crate::location::Location;
use crate::transfer::spec::{TransferKind, TransferSpec};

pub enum Op<'a> {
    Mkdir(&'a Location),
    Rename { from: &'a Location, new_name: &'a str },
    Delete { loc: &'a Location, is_dir: bool },
    Transfer(&'a TransferSpec),
}

fn q(loc: &Location) -> String {
    shell_quote(&loc.fs_path())
}

/// One command line per rclone invocation the operation corresponds to.
pub fn equivalent(op: &Op) -> Vec<String> {
    match op {
        Op::Mkdir(loc) => vec![format!("rclone mkdir {}", q(loc))],
        Op::Rename { from, new_name } => {
            let to = from.parent().unwrap_or_else(|| (*from).clone()).join(new_name);
            vec![format!("rclone moveto {} {}", q(from), q(&to))]
        }
        Op::Delete { loc, is_dir } => {
            let cmd = if *is_dir { "purge" } else { "deletefile" };
            vec![format!("rclone {cmd} {}", q(loc))]
        }
        Op::Transfer(spec) => spec
            .items
            .iter()
            .map(|item| {
                let src = q(&spec.src_dir.join(&item.name));
                let dst = q(&spec.dst_dir.join(&item.dst_name));
                let cmd = match (item.is_dir, spec.kind) {
                    (false, TransferKind::Copy) => format!("rclone copyto {src} {dst}"),
                    (false, TransferKind::Move) => format!("rclone moveto {src} {dst}"),
                    (true, TransferKind::Copy) => format!("rclone copy {src} {dst} --create-empty-src-dirs"),
                    (true, TransferKind::Move) => {
                        format!("rclone move {src} {dst} --create-empty-src-dirs --delete-empty-src-dirs")
                    }
                };
                if item.replace {
                    format!("{cmd} --ignore-times")
                } else {
                    cmd
                }
            })
            .collect(),
    }
}

/// POSIX shell quoting: bare when only safe characters, otherwise single quotes.
pub fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./:@%+=-".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Remote;
    use crate::transfer::spec::TransferItem;

    fn g(p: &str) -> Location {
        Location::new(Remote::Named("personal".into()), p)
    }

    #[test]
    fn simple_ops() {
        assert_eq!(
            equivalent(&Op::Mkdir(&g("Photos/New"))),
            ["rclone mkdir personal:Photos/New"]
        );
        assert_eq!(
            equivalent(&Op::Rename {
                from: &g("a b.txt"),
                new_name: "c.txt"
            }),
            ["rclone moveto 'personal:a b.txt' personal:c.txt"]
        );
        assert_eq!(
            equivalent(&Op::Delete {
                loc: &g("Old"),
                is_dir: true
            }),
            ["rclone purge personal:Old"]
        );
        assert_eq!(
            equivalent(&Op::Delete {
                loc: &g("x.txt"),
                is_dir: false
            }),
            ["rclone deletefile personal:x.txt"]
        );
    }

    #[test]
    fn transfers() {
        let mut spec = TransferSpec {
            id: 1,
            kind: TransferKind::Copy,
            src_dir: Location::new(Remote::Local, "home/t"),
            dst_dir: g(""),
            items: vec![
                TransferItem {
                    replace: false,
                    name: "a.txt".into(),
                    is_dir: false,
                    dst_name: "a.txt".into(),
                    size: None,
                },
                TransferItem {
                    replace: false,
                    name: "Docs".into(),
                    is_dir: true,
                    dst_name: "Docs (1)".into(),
                    size: None,
                },
            ],
        };
        assert_eq!(
            equivalent(&Op::Transfer(&spec)),
            [
                "rclone copyto /home/t/a.txt personal:a.txt",
                "rclone copy /home/t/Docs 'personal:Docs (1)' --create-empty-src-dirs",
            ]
        );
        spec.kind = TransferKind::Move;
        assert_eq!(
            equivalent(&Op::Transfer(&spec)),
            [
                "rclone moveto /home/t/a.txt personal:a.txt",
                "rclone move /home/t/Docs 'personal:Docs (1)' --create-empty-src-dirs --delete-empty-src-dirs",
            ]
        );
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("日本.txt"), "'日本.txt'");
        assert_eq!(shell_quote("personal:a/b-c_d.e"), "personal:a/b-c_d.e");
        assert_eq!(shell_quote(""), "''");
    }
}
