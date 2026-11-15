//! Vim-style key sequences and counts, mapped to actions per context.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::Tab;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyContext {
    Sidebar,
    Pane,
    Transfers,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
    /// Number of files (Space tab only).
    Count,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Down,
    Up,
    Parent,
    Open,
    Top,
    Bottom,
    HalfDown,
    HalfUp,
    ToggleFocus,
    Filter,
    NextMatch,
    PrevMatch,
    ToggleMark,
    Visual,
    Sort(SortKey),
    ReverseSort,
    ToggleHidden,
    Back,
    Yank,
    Cut,
    Paste,
    Delete,
    Rename,
    Mkdir,
    Refresh,
    Transfers,
    Help,
    Quit,
    Escape,
    Stop,
    Resume,
    ClearFinished,
    /// Drop the selected transfer from the list (not running or starting).
    RemoveTransfer,
    /// Drop every transfer that is not running or starting.
    ClearInactive,
    NextTab,
    PrevTab,
    GoTab(Tab),
    /// Open the Space tab.
    Space,
    TogglePanel,
    Rescan,
    OpenInFiles,
    /// Leave the tab without cancelling anything.
    CloseTab,
    Settings,
    AddAccount,
    EditAccount,
    ReauthAccount,
    /// Open or close the second file pane.
    ToggleSplit,
    /// Move focus the other way round (`Shift-Tab`).
    FocusBack,
    /// Copy the selection into the other pane's folder.
    SendCopy,
    /// Move the selection into the other pane's folder.
    SendMove,
    /// Show the next account in the list (wraps round), without the sidebar.
    NextAccount,
    /// Show the previous account in the list (wraps round).
    PrevAccount,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// An action and its count (1 when none was typed).
    Action(Action, u32),
    /// Waiting for more keys.
    Pending,
    Unbound,
}

/// Help group, key label, description, action.
pub type Binding = (&'static str, &'static str, &'static str, Action);

/// The single source of truth for keys: the parser and the help screen both read it.
/// Key labels are token sequences: plain characters or `<Name>` / `<C-x>`.
pub fn bindings(ctx: KeyContext) -> Vec<Binding> {
    use Action::*;
    let nav = "Navigation";
    let files = "Files";
    let tr = "Transfers";
    let app = "App";
    let tabs = "Tabs";
    let accounts = "Accounts";
    let mut list = match ctx {
        KeyContext::Pane => vec![
            (nav, "j", "down", Down),
            (nav, "<Down>", "down", Down),
            (nav, "k", "up", Up),
            (nav, "<Up>", "up", Up),
            (nav, "h", "parent folder", Parent),
            (nav, "<Left>", "parent folder", Parent),
            (nav, "<BS>", "parent folder", Parent),
            (nav, "l", "open", Open),
            (nav, "<Right>", "open", Open),
            (nav, "<CR>", "open", Open),
            (nav, "gg", "first entry", Top),
            (nav, "G", "last entry", Bottom),
            (nav, "<C-d>", "half page down", HalfDown),
            (nav, "<C-u>", "half page up", HalfUp),
            (nav, "-", "previous folder", Back),
            (nav, "<Tab>", "next pane / accounts", ToggleFocus),
            (nav, "<S-Tab>", "focus back", FocusBack),
            (nav, "|", "two panes", ToggleSplit),
            (nav, "/", "filter", Filter),
            (nav, "n", "next match", NextMatch),
            (nav, "N", "previous match", PrevMatch),
            (nav, "sn", "sort by name", Sort(SortKey::Name)),
            (nav, "ss", "sort by size", Sort(SortKey::Size)),
            (nav, "sm", "sort by modified", Sort(SortKey::Modified)),
            (nav, "sr", "reverse sort", ReverseSort),
            (nav, ".", "show hidden", ToggleHidden),
            (nav, "R", "refresh", Refresh),
            (files, "<Space>", "mark", ToggleMark),
            (files, "v", "visual select", Visual),
            (files, "V", "visual select", Visual),
            (files, "y", "copy", Yank),
            (files, "x", "cut", Cut),
            (files, "p", "paste", Paste),
            (files, "dd", "delete", Delete),
            (files, "D", "delete", Delete),
            (files, "r", "rename", Rename),
            (files, "cw", "rename", Rename),
            (files, "o", "new folder", Mkdir),
            (files, "a", "new folder", Mkdir),
            (files, ">", "copy to other pane", SendCopy),
            (files, "<F5>", "copy to other pane", SendCopy),
            (files, "<F6>", "move to other pane", SendMove),
            (tr, "T", "transfers", Transfers),
            (app, "?", "help", Help),
            (app, "q", "quit", Quit),
            (app, "<Esc>", "clear selection", Escape),
        ],
        KeyContext::Sidebar => vec![
            (nav, "j", "down", Down),
            (nav, "<Down>", "down", Down),
            (nav, "k", "up", Up),
            (nav, "<Up>", "up", Up),
            (nav, "gg", "first account", Top),
            (nav, "G", "last account", Bottom),
            (nav, "l", "open account", Open),
            (nav, "<Right>", "open account", Open),
            (nav, "<CR>", "open account", Open),
            (nav, "<Tab>", "switch to files", ToggleFocus),
            (nav, "<S-Tab>", "focus back", FocusBack),
            (accounts, "r", "refresh usage", Refresh),
            (accounts, "R", "refresh usage", Refresh),
            (accounts, "a", "add account", AddAccount),
            (accounts, "e", "edit account", EditAccount),
            (accounts, "A", "re-authorise", ReauthAccount),
            (accounts, "dd", "remove account", Delete),
            (accounts, "D", "remove account", Delete),
            (tr, "T", "transfers", Transfers),
            (app, "?", "help", Help),
            (app, "q", "quit", Quit),
            (app, "<Esc>", "clear", Escape),
        ],
        KeyContext::Transfers => vec![
            (nav, "j", "down", Down),
            (nav, "<Down>", "down", Down),
            (nav, "k", "up", Up),
            (nav, "<Up>", "up", Up),
            (nav, "gg", "first", Top),
            (nav, "G", "last", Bottom),
            (tr, "s", "stop", Stop),
            (tr, "x", "stop", Stop),
            (tr, "<CR>", "resume", Resume),
            (tr, "r", "resume", Resume),
            (tr, "c", "clear finished", ClearFinished),
            (tr, "dd", "remove from list", RemoveTransfer),
            (tr, "D", "remove from list", RemoveTransfer),
            (tr, "C", "clear all but running", ClearInactive),
            (app, "T", "close", Escape),
            (app, "q", "close", Escape),
            (app, "<Esc>", "close", Escape),
            (app, "?", "help", Help),
        ],
        KeyContext::Space => vec![
            (nav, "j", "down", Down),
            (nav, "<Down>", "down", Down),
            (nav, "k", "up", Up),
            (nav, "<Up>", "up", Up),
            (nav, "h", "parent folder", Parent),
            (nav, "<Left>", "parent folder", Parent),
            (nav, "-", "parent folder", Parent),
            (nav, "l", "open folder", Open),
            (nav, "<Right>", "open folder", Open),
            (nav, "<CR>", "open folder / scan", Open),
            (nav, "gg", "first", Top),
            (nav, "G", "last", Bottom),
            (nav, "<C-d>", "half page down", HalfDown),
            (nav, "<C-u>", "half page up", HalfUp),
            (nav, "/", "filter", Filter),
            (nav, "ss", "sort by size", Sort(SortKey::Size)),
            (nav, "sn", "sort by name", Sort(SortKey::Name)),
            (nav, "sc", "sort by file count", Sort(SortKey::Count)),
            (nav, "<Tab>", "switch to accounts", ToggleFocus),
            (nav, "gf", "open in Files", OpenInFiles),
            ("Space", "R", "rescan", Rescan),
            (tr, "T", "transfers", Transfers),
            (app, "?", "help", Help),
            (app, "q", "back to files", CloseTab),
            (app, "<Esc>", "cancel scan / back", Escape),
        ],
    };
    list.extend([
        (tabs, "gt", "next tab", NextTab),
        (tabs, "gT", "previous tab", PrevTab),
        (tabs, "<A-1>", "Files tab", GoTab(Tab::Files)),
        (tabs, "<A-2>", "Space tab", GoTab(Tab::Space)),
        (tabs, "<A-3>", "Transfers tab", GoTab(Tab::Transfers)),
        (tabs, "i", "info panel", TogglePanel),
        (app, ",", "settings", Settings),
        (app, "<C-c>", "quit", Quit),
    ]);
    if matches!(
        ctx,
        KeyContext::Pane | KeyContext::Sidebar | KeyContext::Transfers
    ) {
        list.push((tabs, "U", "space usage", Space));
    }
    if ctx == KeyContext::Transfers {
        list.push((nav, "<Tab>", "switch to accounts", ToggleFocus));
    }
    if matches!(ctx, KeyContext::Pane | KeyContext::Space) {
        list.push((nav, "<C-n>", "next account", NextAccount));
        list.push((nav, "<C-p>", "previous account", PrevAccount));
    }
    list
}

pub struct KeyParser {
    count: Option<u32>,
    pending: Vec<String>,
}

impl Default for KeyParser {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyParser {
    pub fn new() -> Self {
        KeyParser {
            count: None,
            pending: Vec::new(),
        }
    }

    pub fn feed(&mut self, key: KeyEvent, ctx: KeyContext) -> Parsed {
        let Some(token) = token(&key) else {
            self.reset();
            return Parsed::Unbound;
        };
        // Esc on a count or a half-typed sequence only takes that back.
        if token == "<Esc>" && (self.count.is_some() || !self.pending.is_empty()) {
            self.reset();
            return Parsed::Unbound;
        }
        if self.pending.is_empty()
            && let Some(d) = token.chars().next().and_then(|c| c.to_digit(10))
            && token.len() == 1
            && (d != 0 || self.count.is_some())
        {
            self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d));
            return Parsed::Pending;
        }
        self.pending.push(token);
        let mut prefix = false;
        for (_, keys, _, action) in bindings(ctx) {
            let seq = tokenize(keys);
            if seq == self.pending {
                let count = self.count.unwrap_or(1).max(1);
                self.reset();
                return Parsed::Action(action, count);
            }
            if seq.starts_with(&self.pending) {
                prefix = true;
            }
        }
        if prefix {
            Parsed::Pending
        } else {
            self.reset();
            Parsed::Unbound
        }
    }

    /// What has been typed so far, e.g. `"3d"`; empty when idle.
    pub fn pending_keys(&self) -> String {
        let mut s = self.count.map(|c| c.to_string()).unwrap_or_default();
        s.extend(self.pending.iter().map(String::as_str));
        s
    }

    fn reset(&mut self) {
        self.count = None;
        self.pending.clear();
    }
}

/// Split a key label like `"gg"` or `"<C-d>"` into tokens.
pub fn tokenize(label: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' && chars.peek().is_some() && label.contains('>') {
            let mut t = String::from('<');
            for n in chars.by_ref() {
                t.push(n);
                if n == '>' {
                    break;
                }
            }
            out.push(t);
        } else {
            out.push(c.to_string());
        }
    }
    out
}

/// The token for one key press, or `None` for keys we never bind.
fn token(key: &KeyEvent) -> Option<String> {
    let t = match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => format!("<A-{c}>"),
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            format!("<C-{}>", c.to_ascii_lowercase())
        }
        KeyCode::Char(' ') => "<Space>".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "<CR>".into(),
        KeyCode::Esc => "<Esc>".into(),
        KeyCode::Tab => "<Tab>".into(),
        KeyCode::BackTab => "<S-Tab>".into(),
        KeyCode::Backspace => "<BS>".into(),
        KeyCode::Up => "<Up>".into(),
        KeyCode::Down => "<Down>".into(),
        KeyCode::Left => "<Left>".into(),
        KeyCode::Right => "<Right>".into(),
        KeyCode::F(n) => format!("<F{n}>"),
        _ => return None,
    };
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn feed_str(p: &mut KeyParser, s: &str, ctx: KeyContext) -> Parsed {
        let mut last = Parsed::Unbound;
        for c in s.chars() {
            last = p.feed(k(c), ctx);
        }
        last
    }

    use KeyContext::*;

    #[test]
    fn single_keys_and_counts() {
        let mut p = KeyParser::new();
        assert_eq!(p.feed(k('j'), Pane), Parsed::Action(Action::Down, 1));
        assert_eq!(feed_str(&mut p, "5j", Pane), Parsed::Action(Action::Down, 5));
        assert_eq!(feed_str(&mut p, "12k", Pane), Parsed::Action(Action::Up, 12));
        assert_eq!(
            p.feed(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), Pane),
            Parsed::Action(Action::Down, 1)
        );
    }

    #[test]
    fn sequences() {
        let mut p = KeyParser::new();
        assert_eq!(p.feed(k('g'), Pane), Parsed::Pending);
        assert_eq!(p.feed(k('g'), Pane), Parsed::Action(Action::Top, 1));
        assert_eq!(feed_str(&mut p, "dd", Pane), Parsed::Action(Action::Delete, 1));
        assert_eq!(feed_str(&mut p, "cw", Pane), Parsed::Action(Action::Rename, 1));
        assert_eq!(
            feed_str(&mut p, "sm", Pane),
            Parsed::Action(Action::Sort(SortKey::Modified), 1)
        );
    }

    #[test]
    fn bad_sequence_resets() {
        let mut p = KeyParser::new();
        assert_eq!(p.feed(k('d'), Pane), Parsed::Pending);
        assert_eq!(p.feed(k('j'), Pane), Parsed::Unbound);
        assert_eq!(p.pending_keys(), "");
        assert_eq!(p.feed(k('j'), Pane), Parsed::Action(Action::Down, 1));
    }

    #[test]
    fn ctrl_and_shift() {
        let mut p = KeyParser::new();
        let cd = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(p.feed(cd, Pane), Parsed::Action(Action::HalfDown, 1));
        let g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(p.feed(g, Pane), Parsed::Action(Action::Bottom, 1));
    }

    #[test]
    fn context_matters() {
        let mut p = KeyParser::new();
        assert_eq!(p.feed(k('x'), Transfers), Parsed::Action(Action::Stop, 1));
        assert_eq!(p.feed(k('x'), Pane), Parsed::Action(Action::Cut, 1));
        assert_eq!(p.feed(k('r'), Sidebar), Parsed::Action(Action::Refresh, 1));
    }

    #[test]
    fn pending_keys_shows_count_and_prefix() {
        let mut p = KeyParser::new();
        feed_str(&mut p, "3d", Pane);
        assert_eq!(p.pending_keys(), "3d");
    }

    #[test]
    fn esc_takes_back_a_count_without_acting() {
        let mut p = KeyParser::new();
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(p.feed(k('3'), Pane), Parsed::Pending);
        assert_eq!(p.feed(esc, Pane), Parsed::Unbound, "not the Escape action");
        assert_eq!(p.pending_keys(), "");
        assert_eq!(p.feed(k('d'), Pane), Parsed::Pending);
        assert_eq!(p.feed(esc, Pane), Parsed::Unbound);
        assert_eq!(p.feed(esc, Pane), Parsed::Action(Action::Escape, 1));
    }

    #[test]
    fn ctrl_c_quits_everywhere() {
        let mut p = KeyParser::new();
        let cc = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        for ctx in [Pane, Sidebar, Transfers, Space] {
            assert_eq!(p.feed(cc, ctx), Parsed::Action(Action::Quit, 1));
        }
    }

    #[test]
    fn zero_alone_is_not_a_count() {
        let mut p = KeyParser::new();
        assert_eq!(p.feed(k('0'), Pane), Parsed::Unbound);
        assert_eq!(feed_str(&mut p, "10j", Pane), Parsed::Action(Action::Down, 10));
    }

    #[test]
    fn split_keys() {
        let mut p = KeyParser::new();
        let f = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert_eq!(token(&f(KeyCode::F(5))).as_deref(), Some("<F5>"));
        assert_eq!(p.feed(k('|'), Pane), Parsed::Action(Action::ToggleSplit, 1));
        assert_eq!(p.feed(k('>'), Pane), Parsed::Action(Action::SendCopy, 1));
        assert_eq!(
            p.feed(f(KeyCode::F(5)), Pane),
            Parsed::Action(Action::SendCopy, 1)
        );
        assert_eq!(
            p.feed(f(KeyCode::F(6)), Pane),
            Parsed::Action(Action::SendMove, 1)
        );
        assert_eq!(
            p.feed(f(KeyCode::BackTab), Pane),
            Parsed::Action(Action::FocusBack, 1)
        );
        assert_eq!(
            p.feed(f(KeyCode::BackTab), Sidebar),
            Parsed::Action(Action::FocusBack, 1)
        );
    }

    #[test]
    fn tab_keys() {
        use crate::app::Tab;
        let mut p = KeyParser::new();
        for ctx in [Pane, Sidebar, Transfers, Space] {
            assert_eq!(feed_str(&mut p, "gt", ctx), Parsed::Action(Action::NextTab, 1));
            assert_eq!(feed_str(&mut p, "gT", ctx), Parsed::Action(Action::PrevTab, 1));
            let a2 = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT);
            assert_eq!(p.feed(a2, ctx), Parsed::Action(Action::GoTab(Tab::Space), 1));
            assert_eq!(feed_str(&mut p, "i", ctx), Parsed::Action(Action::TogglePanel, 1));
        }
        assert_eq!(feed_str(&mut p, "U", Pane), Parsed::Action(Action::Space, 1));
        // Bare digits are still counts.
        assert_eq!(feed_str(&mut p, "2j", Pane), Parsed::Action(Action::Down, 2));
    }

    #[test]
    fn space_has_no_match_keys() {
        let mut p = KeyParser::new();
        assert_eq!(p.feed(k('n'), Space), Parsed::Unbound);
        assert_eq!(p.feed(k('N'), Space), Parsed::Unbound);
    }

    #[test]
    fn no_binding_is_a_prefix_of_another() {
        for ctx in [Pane, Sidebar, Transfers, Space] {
            let keys: Vec<&str> = bindings(ctx).iter().map(|b| b.1).collect();
            for a in &keys {
                for b in &keys {
                    assert!(a == b || !b.starts_with(a), "{a} is a prefix of {b} in {ctx:?}");
                }
            }
        }
    }
}
