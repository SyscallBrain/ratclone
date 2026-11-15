//! Locations on a remote or the local disk.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Remote {
    Local,
    Named(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Location {
    pub remote: Remote,
    pub path: String,
}

impl Location {
    /// Build a location, normalising away leading and trailing slashes.
    pub fn new(remote: Remote, path: &str) -> Self {
        let path = path.trim_matches('/').to_string();
        Location { remote, path }
    }

    /// The rclone fs string for the remote root: `"personal:"` or `"/"`.
    pub fn fs(&self) -> String {
        match &self.remote {
            Remote::Local => "/".to_string(),
            Remote::Named(n) => format!("{n}:"),
        }
    }

    /// The full rclone path: `"personal:Photos/2025"` or `"/home/demo"`.
    pub fn fs_path(&self) -> String {
        format!("{}{}", self.fs(), self.path)
    }

    pub fn join(&self, name: &str) -> Location {
        let name = name.trim_matches('/');
        let path = if self.path.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", self.path, name)
        };
        Location {
            remote: self.remote.clone(),
            path,
        }
    }

    pub fn parent(&self) -> Option<Location> {
        if self.path.is_empty() {
            return None;
        }
        let path = match self.path.rfind('/') {
            Some(i) => self.path[..i].to_string(),
            None => String::new(),
        };
        Some(Location {
            remote: self.remote.clone(),
            path,
        })
    }

    /// Last path component; empty at the root.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or("")
    }

    /// True when `self` is `other` or lies somewhere below it.
    pub fn is_within(&self, other: &Location) -> bool {
        if self.remote != other.remote {
            return false;
        }
        other.path.is_empty()
            || self.path == other.path
            || self
                .path
                .strip_prefix(&other.path)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    /// Human form shown in the UI: `"personal:/Photos"`, `"local:/home/demo"`.
    pub fn display(&self) -> String {
        let name = match &self.remote {
            Remote::Local => "local",
            Remote::Named(n) => n.as_str(),
        };
        format!("{name}:/{}", self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(n: &str, p: &str) -> Location {
        Location::new(Remote::Named(n.into()), p)
    }

    #[test]
    fn fs_strings() {
        let g = named("personal", "/Photos/2025/");
        assert_eq!(g.path, "Photos/2025");
        assert_eq!(g.fs(), "personal:");
        assert_eq!(g.fs_path(), "personal:Photos/2025");
        assert_eq!(named("personal", "").fs_path(), "personal:");
        let l = Location::new(Remote::Local, "/home/demo");
        assert_eq!(l.fs(), "/");
        assert_eq!(l.fs_path(), "/home/demo");
        assert_eq!(Location::new(Remote::Local, "").fs_path(), "/");
    }

    #[test]
    fn join_parent_name() {
        let g = named("g", "a");
        assert_eq!(g.join("b c").path, "a/b c");
        assert_eq!(g.parent().unwrap().path, "");
        assert_eq!(named("g", "").parent(), None);
        assert_eq!(g.join("x").name(), "x");
        assert_eq!(named("g", "").join("x").path, "x");
        assert_eq!(named("g", "a/b").parent().unwrap().path, "a");
    }

    #[test]
    fn within() {
        let a = named("g", "Photos");
        assert!(named("g", "Photos").is_within(&a));
        assert!(named("g", "Photos/2025").is_within(&a));
        assert!(!named("g", "Photos2").is_within(&a));
        assert!(!Location::new(Remote::Local, "Photos").is_within(&a));
        assert!(named("g", "x").is_within(&named("g", "")));
    }

    #[test]
    fn display() {
        assert_eq!(named("g", "a/b").display(), "g:/a/b");
        assert_eq!(Location::new(Remote::Local, "home/t").display(), "local:/home/t");
    }
}
