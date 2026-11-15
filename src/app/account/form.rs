//! The account wizard's model: providers, fields and what gets sent to rclone.
//! Nothing here touches the app or the backend.

use std::collections::BTreeMap;

use crate::app::overlay::LineInput;
use crate::backend::{ConfigParams, Provider, ProviderOption};

/// rclone's `OptionHideConfigurator` bit.
const HIDE_CONFIGURATOR: u8 = 4;

/// Types most people want, listed first.
const FIRST: [&str; 3] = ["drive", "dropbox", "mega"];

/// The types offered when adding an account, in the order shown.
pub fn provider_list(all: &[Provider]) -> Vec<Provider> {
    let mut out: Vec<Provider> = all.iter().filter(|p| !p.hide).cloned().collect();
    out.sort_by_key(|p| {
        let rank = FIRST.iter().position(|f| *f == p.name).unwrap_or(FIRST.len());
        (rank, p.name.clone())
    });
    out
}

pub fn provider_matches(p: &Provider, query: &str) -> bool {
    let q = query.to_lowercase();
    p.name.to_lowercase().contains(&q) || p.description.to_lowercase().contains(&q)
}

/// rclone's rules for remote names, plus "not taken".
pub fn validate_name(name: &str, existing: &[String]) -> Result<(), String> {
    if name.is_empty() {
        return Err("Name can't be empty".into());
    }
    if name.starts_with('-') || name.starts_with(' ') {
        return Err("Name can't start with - or a space".into());
    }
    if name.ends_with(' ') {
        return Err("Name can't end with a space".into());
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.' | '+' | '@'))
    {
        return Err("Only letters, digits, spaces and _ - . + @".into());
    }
    if existing.iter().any(|e| e == name) {
        return Err(format!("An account called {name} already exists"));
    }
    Ok(())
}

/// [`validate_name`], also refusing names whose clean-up is still running.
pub fn validate_name_busy(name: &str, existing: &[String], busy: &[String]) -> Result<(), String> {
    if busy.iter().any(|b| b == name) {
        return Err(format!("{name} is still being removed; try again in a moment"));
    }
    validate_name(name, existing)
}

/// How a field is edited.
#[derive(Clone, Debug, PartialEq)]
pub enum Widget {
    Toggle,
    Choice(Vec<String>),
    Text {
        suggestions: Vec<String>,
    },
    Password,
    /// A `remote:path` value (crypt, alias, union…).
    RemotePath,
}

pub fn widget_for(opt: &ProviderOption) -> Widget {
    let values = || opt.examples.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>();
    if opt.kind == "bool" {
        Widget::Toggle
    } else if opt.is_password {
        Widget::Password
    } else if matches!(opt.name.as_str(), "remote" | "upstreams") {
        Widget::RemotePath
    } else if opt.exclusive && !opt.examples.is_empty() {
        Widget::Choice(values())
    } else {
        Widget::Text {
            suggestions: values(),
        }
    }
}

/// What the user did to a field.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Unchanged,
    Set(String),
    /// Back to rclone's default.
    Reset,
}

#[derive(Clone, PartialEq)]
pub struct Field {
    pub option: ProviderOption,
    pub widget: Widget,
    /// The saved value (edit mode).
    pub original: Option<String>,
    pub input: LineInput,
    pub edit: Edit,
    /// Show a password as typed.
    pub reveal: bool,
}

impl std::fmt::Debug for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("Field");
        d.field("name", &self.option.name).field("widget", &self.widget);
        if self.is_secret() {
            d.field("value", &"<redacted>");
        } else {
            d.field("original", &self.original)
                .field("input", &self.input.value)
                .field("edit", &self.edit);
        }
        d.finish()
    }
}

impl Field {
    pub fn new(option: ProviderOption, original: Option<String>) -> Field {
        let widget = widget_for(&option);
        let initial = match widget {
            Widget::Password => String::new(),
            Widget::Toggle | Widget::Choice(_) => original.clone().unwrap_or_else(|| option.default.clone()),
            _ => original.clone().unwrap_or_default(),
        };
        Field {
            input: LineInput::new(&initial, initial.chars().count()),
            option,
            widget,
            original,
            edit: Edit::Unchanged,
            reveal: false,
        }
    }

    fn is_secret(&self) -> bool {
        self.widget == Widget::Password || self.option.sensitive
    }

    /// Record the text now in the field (after typing into `input`, or directly).
    pub fn set_text(&mut self, s: &str) {
        if self.input.value != s {
            self.input = LineInput::new(s, s.chars().count());
        }
        let empty_password = self.widget == Widget::Password && s.is_empty();
        self.edit = if empty_password || self.original.as_deref() == Some(s) {
            Edit::Unchanged
        } else {
            Edit::Set(s.to_string())
        };
    }

    /// Step a Toggle or Choice by `dir` (wrapping).
    pub fn cycle(&mut self, dir: i32) {
        let values: Vec<String> = match &self.widget {
            Widget::Toggle => vec!["false".into(), "true".into()],
            Widget::Choice(v) => v.clone(),
            _ => return,
        };
        if values.is_empty() {
            return;
        }
        let n = values.len() as i32;
        let at = values
            .iter()
            .position(|v| *v == self.current())
            .map(|i| i as i32)
            .unwrap_or(-1);
        let next = if at < 0 && dir < 0 {
            n - 1
        } else {
            (at + dir).rem_euclid(n)
        };
        let v = values[next as usize].clone();
        self.set_text(&v);
    }

    pub fn reset(&mut self) {
        let d = if self.widget == Widget::Password {
            String::new()
        } else {
            self.option.default.clone()
        };
        self.input = LineInput::new(&d, d.chars().count());
        self.edit = Edit::Reset;
    }

    /// The value in effect: what is typed, or rclone's default.
    pub fn current(&self) -> String {
        if self.input.value.is_empty() {
            self.option.default.clone()
        } else {
            self.input.value.clone()
        }
    }

    /// The value as drawn. Empty means "show the default as a placeholder".
    pub fn shown_value(&self) -> String {
        if self.widget != Widget::Password {
            return self.input.value.clone();
        }
        if self.input.value.is_empty() {
            if self.original.is_some() && self.edit == Edit::Unchanged {
                "••••• (unchanged)".into()
            } else {
                String::new()
            }
        } else if self.reveal {
            self.input.value.clone()
        } else {
            // Fixed width: the length of a password is not shown either.
            "•••••".into()
        }
    }

    pub fn missing_required(&self, editing: bool) -> bool {
        let saved_password = editing && self.widget == Widget::Password && self.original.is_some();
        self.option.required && self.current().is_empty() && !saved_password
    }
}

/// The option whose value picks the variant of a type (S3's vendor, Koofr's service…).
const VARIANT: &str = "provider";

/// rclone's `MatchProvider`: does an option with rule `rule` apply to `variant`?
pub fn rule_matches(rule: &str, variant: &str) -> bool {
    if rule.is_empty() || variant.is_empty() {
        return true;
    }
    let (negate, list) = match rule.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, rule),
    };
    list.split(',').any(|v| v == variant) != negate
}

/// The variant chosen in the form: the `provider` field's value. Left empty, the
/// first one rclone lists (the type's own service) is what the form is for.
pub fn variant(fields: &[Field]) -> String {
    let Some(f) = fields
        .iter()
        .find(|f| f.option.name == VARIANT && f.option.provider.is_empty())
    else {
        return String::new();
    };
    let v = f.current();
    if v.is_empty() {
        f.option
            .examples
            .first()
            .map(|(v, _)| v.clone())
            .unwrap_or_default()
    } else {
        v
    }
}

/// Which fields apply to the variant chosen now (same order as `fields`).
pub fn applicable(fields: &[Field]) -> Vec<bool> {
    let v = variant(fields);
    fields
        .iter()
        .map(|f| rule_matches(&f.option.provider, &v))
        .collect()
}

/// The fields of the form for `p`, filled from `current` when editing.
pub fn fields_for(p: &Provider, current: Option<&BTreeMap<String, String>>) -> Vec<Field> {
    let mut fields: Vec<Field> = p
        .options
        .iter()
        .filter(|o| o.hide & HIDE_CONFIGURATOR == 0 && o.name != "token" && !o.name.starts_with("config_"))
        .map(|o| {
            let original = current.and_then(|c| c.get(&o.name)).cloned();
            Field::new(o.clone(), original)
        })
        .collect();
    // A new account whose fields depend on a variant: say which one, rather than
    // leave it to rclone's guess while the form shows that variant's fields.
    let depends = fields.iter().any(|f| !f.option.provider.is_empty());
    if current.is_none()
        && depends
        && let Some(f) = fields
            .iter_mut()
            .find(|f| f.option.name == VARIANT && f.option.provider.is_empty() && f.current().is_empty())
        && let Some((first, _)) = f.option.examples.first().cloned()
    {
        f.set_text(&first);
    }
    fields
}

/// What to send. Adding sends what was filled; editing sends only what changed.
/// Fields of other variants are never sent: they share names with this one's.
pub fn params_for(fields: &[Field], editing: bool) -> ConfigParams {
    let mut p = ConfigParams::default();
    let applies = applicable(fields);
    for (f, _) in fields.iter().zip(&applies).filter(|(_, a)| **a) {
        let value = match &f.edit {
            Edit::Unchanged => None,
            Edit::Set(v) if editing || !v.is_empty() => Some(v.clone()),
            Edit::Set(_) => None,
            Edit::Reset if editing => Some(f.option.default.clone()),
            Edit::Reset => None,
        };
        if let Some(v) = value {
            p.obscure |= f.widget == Widget::Password;
            p.values.insert(f.option.name.clone(), v);
        }
    }
    p
}

/// A new OAuth client needs a new token.
/// The `rclone authorize …` line rclone's token question asks to run on a machine
/// with a browser, with every argument it gave: a client of one's own travels as one
/// more (encoded) argument, and a token got without it would not work here.
pub fn authorize_command(help: &str, kind: &str) -> String {
    help.lines()
        .map(str::trim)
        .find(|l| l.starts_with("rclone authorize "))
        .map(str::to_string)
        .unwrap_or_else(|| format!("rclone authorize \"{kind}\""))
}

pub fn client_changed(fields: &[Field]) -> bool {
    fields
        .iter()
        .any(|f| matches!(f.option.name.as_str(), "client_id" | "client_secret") && f.edit != Edit::Unchanged)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoAnswer {
    /// `config_is_local`: use this machine's browser.
    Local(bool),
    /// `config_refresh_token`: get a new token.
    RefreshToken(bool),
}

/// Questions the app answers by itself.
pub fn auto_answer(option: &str, editing: bool, over_ssh: bool, client_changed: bool) -> Option<AutoAnswer> {
    match option {
        "config_is_local" => Some(AutoAnswer::Local(!over_ssh)),
        "config_refresh_token" if editing => Some(AutoAnswer::RefreshToken(client_changed)),
        _ => None,
    }
}

/// The help up to its first blank line.
pub fn first_paragraph(help: &str) -> &str {
    let end = help.find("\n\n").unwrap_or(help.len());
    help[..end].trim()
}

/// Accounts built on `name` (crypt, alias, union, combine…), directly or through
/// others (`outer` on `middle` on `name`), in the order they are reached.
pub fn dependents(name: &str, configs: &[(String, BTreeMap<String, String>)]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut next = vec![name.to_string()];
    while let Some(base) = next.pop() {
        for dep in direct_dependents(&base, configs) {
            // A cycle (or the account itself) is not followed twice.
            if dep != name && !found.contains(&dep) {
                found.push(dep.clone());
                next.insert(0, dep);
            }
        }
    }
    found
}

/// Accounts whose `remote` or `upstreams` name `name` itself.
fn direct_dependents(name: &str, configs: &[(String, BTreeMap<String, String>)]) -> Vec<String> {
    let prefix = format!("{name}:");
    configs
        .iter()
        .filter(|(other, _)| other != name)
        .filter(|(_, c)| {
            // `remote` is one path (names may hold spaces); `upstreams` is a list.
            c.get("remote").is_some_and(|v| v.trim().starts_with(&prefix))
                || c.get("upstreams").is_some_and(|v| {
                    space_separated(v).iter().any(|t| {
                        // combine: `dir=remote:path`; union: `remote:path[:ro]`.
                        let t = match t.split_once('=') {
                            Some((dir, rest)) if !dir.contains(':') => rest,
                            _ => t.as_str(),
                        };
                        t.starts_with(&prefix)
                    })
                })
        })
        .map(|(n, _)| n.clone())
        .collect()
}

/// rclone's space separated list: `"a b:" c:` is `a b:` and `c:` (`""` is a quote
/// inside quotes).
fn space_separated(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = v.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                cur.push('"');
            }
            '"' if quoted => quoted = false,
            '"' if !started => {
                quoted = true;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// The type logs in through a browser.
pub fn is_oauth(p: &Provider) -> bool {
    p.options.iter().any(|o| o.name == "token")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Provider, ProviderOption};
    use std::collections::BTreeMap;

    fn opt(name: &str) -> ProviderOption {
        ProviderOption {
            name: name.into(),
            kind: "string".into(),
            ..Default::default()
        }
    }
    fn prov(name: &str, desc: &str, options: Vec<ProviderOption>) -> Provider {
        Provider {
            name: name.into(),
            description: desc.into(),
            hide: false,
            options,
        }
    }

    #[test]
    fn order_and_hidden() {
        let all = vec![
            prov("alias", "Alias", vec![]),
            prov("mega", "Mega", vec![]),
            Provider {
                hide: true,
                ..prov("tardigrade", "Storj", vec![])
            },
            prov("box", "Box", vec![]),
            prov("drive", "Google Drive", vec![]),
            prov("dropbox", "Dropbox", vec![]),
        ];
        let names: Vec<_> = provider_list(&all).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["drive", "dropbox", "mega", "alias", "box"]);
    }

    #[test]
    fn matches_description() {
        let p = prov("drive", "Google Drive", vec![]);
        assert!(provider_matches(&p, "goo"));
        assert!(provider_matches(&p, "DRI"));
        assert!(provider_matches(&p, ""));
        assert!(!provider_matches(&p, "mega"));
    }

    #[test]
    fn name_rules() {
        let existing = vec!["personal".to_string()];
        assert_eq!(validate_name("my drive", &existing), Ok(()));
        assert_eq!(validate_name("fotos-ção", &existing), Ok(()));
        assert_eq!(validate_name("a_b.c+d@e", &existing), Ok(()));
        assert_eq!(validate_name("", &existing).unwrap_err(), "Name can't be empty");
        assert_eq!(
            validate_name("-x", &existing).unwrap_err(),
            "Name can't start with - or a space"
        );
        assert_eq!(
            validate_name(" x", &existing).unwrap_err(),
            "Name can't start with - or a space"
        );
        assert_eq!(
            validate_name("x ", &existing).unwrap_err(),
            "Name can't end with a space"
        );
        assert_eq!(
            validate_name("a/b", &existing).unwrap_err(),
            "Only letters, digits, spaces and _ - . + @"
        );
        assert_eq!(
            validate_name("a:b", &existing).unwrap_err(),
            "Only letters, digits, spaces and _ - . + @"
        );
        assert_eq!(
            validate_name("personal", &existing).unwrap_err(),
            "An account called personal already exists"
        );
    }

    #[test]
    fn widgets_by_shape() {
        let b = ProviderOption {
            kind: "bool".into(),
            ..opt("x")
        };
        assert_eq!(widget_for(&b), Widget::Toggle);
        let p = ProviderOption {
            is_password: true,
            ..opt("pass")
        };
        assert_eq!(widget_for(&p), Widget::Password);
        assert_eq!(widget_for(&opt("remote")), Widget::RemotePath);
        assert_eq!(widget_for(&opt("upstreams")), Widget::RemotePath);
        let ex = vec![
            ("a".to_string(), "A".to_string()),
            ("b".to_string(), "B".to_string()),
        ];
        let c = ProviderOption {
            examples: ex.clone(),
            exclusive: true,
            ..opt("c")
        };
        assert_eq!(widget_for(&c), Widget::Choice(vec!["a".into(), "b".into()]));
        let t = ProviderOption {
            examples: ex,
            exclusive: false,
            ..opt("t")
        };
        assert_eq!(
            widget_for(&t),
            Widget::Text {
                suggestions: vec!["a".into(), "b".into()]
            }
        );
        assert_eq!(widget_for(&opt("plain")), Widget::Text { suggestions: vec![] });
    }

    #[test]
    fn hidden_options_skipped() {
        let p = prov(
            "x",
            "X",
            vec![
                ProviderOption {
                    hide: 4,
                    ..opt("gone")
                },
                ProviderOption {
                    hide: 3,
                    ..opt("shown")
                },
                opt("token"),
            ],
        );
        let names: Vec<_> = fields_for(&p, None).into_iter().map(|f| f.option.name).collect();
        assert_eq!(names, ["shown"], "token is never a field");
    }

    fn three() -> (Provider, BTreeMap<String, String>) {
        let p = prov(
            "x",
            "X",
            vec![
                opt("a"),
                opt("b"),
                ProviderOption {
                    default: "dflt".into(),
                    ..opt("c")
                },
            ],
        );
        let cur = BTreeMap::from([
            ("type".into(), "x".into()),
            ("a".into(), "1".into()),
            ("b".into(), "2".into()),
            ("c".into(), "3".into()),
        ]);
        (p, cur)
    }

    #[test]
    fn edit_diff_sends_only_changed() {
        let (p, cur) = three();
        let mut f = fields_for(&p, Some(&cur));
        assert_eq!(f[1].input.value, "2");
        f[1].set_text("22");
        f[0].set_text("1"); // same as before: unchanged
        let params = params_for(&f, true);
        assert_eq!(params.values.keys().collect::<Vec<_>>(), ["b"]);
        assert_eq!(params.values["b"], "22");
    }

    #[test]
    fn reset_sends_default() {
        let (p, cur) = three();
        let mut f = fields_for(&p, Some(&cur));
        f[2].reset();
        let params = params_for(&f, true);
        assert_eq!(params.values.get("c").map(String::as_str), Some("dflt"));
        f[0].set_text("");
        assert_eq!(params_for(&f, true).values.get("a").map(String::as_str), Some(""));
    }

    #[test]
    fn password_unchanged_not_sent() {
        let p = prov(
            "mega",
            "Mega",
            vec![
                opt("user"),
                ProviderOption {
                    is_password: true,
                    ..opt("pass")
                },
            ],
        );
        let cur = BTreeMap::from([("user".into(), "x".into()), ("pass".into(), "OBSCURED".into())]);
        let f = fields_for(&p, Some(&cur));
        assert_eq!(f[1].input.value, "");
        assert_eq!(f[1].shown_value(), "••••• (unchanged)");
        let params = params_for(&f, true);
        assert!(params.values.is_empty());
        assert!(!params.obscure);
    }

    #[test]
    fn add_skips_empty() {
        let p = prov("mega", "Mega", vec![opt("user"), opt("2fa")]);
        let mut f = fields_for(&p, None);
        f[0].set_text("me");
        let params = params_for(&f, false);
        assert_eq!(params.values.keys().collect::<Vec<_>>(), ["user"]);
    }

    #[test]
    fn obscure_when_password_sent() {
        let p = prov(
            "mega",
            "Mega",
            vec![
                opt("user"),
                ProviderOption {
                    is_password: true,
                    ..opt("pass")
                },
            ],
        );
        let mut f = fields_for(&p, None);
        f[0].set_text("me");
        assert!(!params_for(&f, false).obscure);
        f[1].set_text("pw");
        assert!(params_for(&f, false).obscure);
        assert_eq!(f[1].shown_value(), "•••••");
    }

    #[test]
    fn toggle_and_choice_cycle() {
        let p = prov(
            "x",
            "X",
            vec![
                ProviderOption {
                    kind: "bool".into(),
                    default: "false".into(),
                    ..opt("b")
                },
                ProviderOption {
                    examples: vec![
                        ("s".into(), "".into()),
                        ("o".into(), "".into()),
                        ("f".into(), "".into()),
                    ],
                    exclusive: true,
                    default: "s".into(),
                    ..opt("c")
                },
            ],
        );
        let mut f = fields_for(&p, None);
        assert_eq!(f[0].current(), "false");
        f[0].cycle(1);
        assert_eq!(f[0].current(), "true");
        assert_eq!(f[1].current(), "s");
        f[1].cycle(-1);
        assert_eq!(f[1].current(), "f");
        let params = params_for(&f, false);
        assert_eq!(params.values["b"], "true");
        assert_eq!(params.values["c"], "f");
    }

    #[test]
    fn required_missing() {
        let p = prov(
            "mega",
            "Mega",
            vec![
                ProviderOption {
                    required: true,
                    ..opt("user")
                },
                ProviderOption {
                    required: true,
                    is_password: true,
                    ..opt("pass")
                },
            ],
        );
        let mut f = fields_for(&p, None);
        assert!(f[0].missing_required(false));
        f[0].set_text("me");
        assert!(!f[0].missing_required(false));
        let cur = BTreeMap::from([("user".into(), "x".into()), ("pass".into(), "O".into())]);
        let f = fields_for(&p, Some(&cur));
        assert!(!f[1].missing_required(true), "a saved password counts");
    }

    #[test]
    fn auto_answers() {
        assert_eq!(
            auto_answer("config_is_local", false, false, false),
            Some(AutoAnswer::Local(true))
        );
        assert_eq!(
            auto_answer("config_is_local", true, true, false),
            Some(AutoAnswer::Local(false))
        );
        assert_eq!(
            auto_answer("config_refresh_token", true, false, true),
            Some(AutoAnswer::RefreshToken(true))
        );
        assert_eq!(
            auto_answer("config_refresh_token", true, false, false),
            Some(AutoAnswer::RefreshToken(false))
        );
        assert_eq!(auto_answer("config_refresh_token", false, false, false), None);
        assert_eq!(auto_answer("config_team_drive", false, false, false), None);
    }

    #[test]
    fn client_change_detected() {
        let p = prov("drive", "Drive", vec![opt("client_id"), opt("scope")]);
        let mut f = fields_for(&p, Some(&BTreeMap::new()));
        f[1].set_text("drive");
        assert!(!client_changed(&f));
        f[0].set_text("abc");
        assert!(client_changed(&f));
    }

    #[test]
    fn first_paragraph_of_help() {
        assert_eq!(
            first_paragraph("Line one\nstill one\n\nSecond"),
            "Line one\nstill one"
        );
        assert_eq!(first_paragraph("  only  "), "only");
    }

    #[test]
    fn oauth_detected() {
        assert!(is_oauth(&prov(
            "dropbox",
            "D",
            vec![opt("client_id"), opt("token")]
        )));
        assert!(!is_oauth(&prov("mega", "M", vec![opt("user")])));
        assert!(
            !is_oauth(&prov("azureblob", "A", vec![opt("client_id")])),
            "only a token means a browser login"
        );
    }

    #[test]
    fn dependents_found() {
        let c = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>()
        };
        let configs = vec![
            (
                "vault".to_string(),
                c(&[("type", "crypt"), ("remote", "personal:Secret")]),
            ),
            (
                "all".to_string(),
                c(&[("type", "union"), ("upstreams", "personal: work:x")]),
            ),
            (
                "other".to_string(),
                c(&[("type", "alias"), ("remote", "personal2:")]),
            ),
            ("personal".to_string(), c(&[("type", "drive")])),
        ];
        assert_eq!(dependents("personal", &configs), ["vault", "all"]);
        assert_eq!(dependents("work", &configs), ["all"]);
        assert!(dependents("personal2", &configs) == ["other"]);
        let spaced = vec![
            (
                "vault".to_string(),
                c(&[("type", "crypt"), ("remote", "my drive:Secret")]),
            ),
            ("drive".to_string(), c(&[("type", "drive")])),
        ];
        assert_eq!(dependents("my drive", &spaced), ["vault"]);
        assert!(
            dependents("drive", &spaced).is_empty(),
            "\"my drive:\" is not drive"
        );
    }

    #[test]
    fn dependents_through_others_and_quoted_upstreams() {
        let c = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>()
        };
        let configs = vec![
            (
                "outer".to_string(),
                c(&[("type", "crypt"), ("remote", "middle:secret")]),
            ),
            (
                "middle".to_string(),
                c(&[("type", "alias"), ("remote", "archive:")]),
            ),
            ("loop".to_string(), c(&[("type", "alias"), ("remote", "outer:")])),
            ("back".to_string(), c(&[("type", "alias"), ("remote", "loop:")])),
            (
                "archive".to_string(),
                c(&[("type", "alias"), ("remote", "back:")]),
            ),
        ];
        assert_eq!(
            dependents("archive", &configs),
            ["middle", "outer", "loop", "back"],
            "a cycle ends"
        );
        let union = vec![
            (
                "combined".to_string(),
                c(&[("upstreams", "\"archive with space:x\" media:")]),
            ),
            (
                "joined".to_string(),
                c(&[("upstreams", "\"docs=archive with space:d\" pics=media:p")]),
            ),
            ("ro".to_string(), c(&[("upstreams", "other: media:path:ro")])),
        ];
        assert_eq!(dependents("archive with space", &union), ["combined", "joined"]);
        assert_eq!(dependents("media", &union), ["combined", "joined", "ro"]);
        assert_eq!(
            space_separated(r#""a ""b"" c:" d:  e"#),
            ["a \"b\" c:", "d:", "e"]
        );
    }

    #[test]
    fn debug_redacts_password() {
        let p = prov(
            "mega",
            "Mega",
            vec![
                ProviderOption {
                    is_password: true,
                    ..opt("pass")
                },
                ProviderOption {
                    sensitive: true,
                    ..opt("client_secret")
                },
            ],
        );
        let mut f = fields_for(&p, None);
        f[0].set_text("hunter2");
        f[1].set_text("s3cr3t");
        let dbg = format!("{f:?}");
        assert!(!dbg.contains("hunter2"), "{dbg}");
        assert!(!dbg.contains("s3cr3t"), "{dbg}");
    }

    #[test]
    fn authorize_command_keeps_every_argument() {
        let help = |line: &str| {
            format!(
                "For this to work...\n\nExecute the following on the machine:\n\n\t{line}\n\nThen paste the result.\n"
            )
        };
        assert_eq!(
            authorize_command(&help("rclone authorize \"dropbox\""), "dropbox"),
            "rclone authorize \"dropbox\""
        );
        assert_eq!(
            authorize_command(
                &help("rclone authorize \"dropbox\" \"eyJjbGllbnRfaWQiOiJhYmMifQ\""),
                "dropbox"
            ),
            "rclone authorize \"dropbox\" \"eyJjbGllbnRfaWQiOiJhYmMifQ\""
        );
        // A help without the line (another rclone): the type alone, as before.
        assert_eq!(
            authorize_command("Paste the token.", "box"),
            "rclone authorize \"box\""
        );
    }

    /// Koofr and Storj as rclone v1.76 describes them (`config providers`).
    fn real(kind: &str) -> Provider {
        let raw: crate::rc::types::RcProviders =
            serde_json::from_str(include_str!("../../../tests/fixtures/providers_koofr_storj.json")).unwrap();
        crate::backend::rc::providers_from(raw)
            .into_iter()
            .find(|p| p.name == kind)
            .unwrap()
    }

    fn set(fields: &mut [Field], name: &str, nth: usize, v: &str) {
        let applies = applicable(fields);
        fields
            .iter_mut()
            .zip(applies)
            .filter(|(f, a)| *a && f.option.name == name)
            .nth(nth)
            .unwrap()
            .0
            .set_text(v);
    }

    fn shown(fields: &[Field]) -> Vec<String> {
        fields
            .iter()
            .zip(applicable(fields))
            .filter(|(f, a)| *a && !f.option.advanced)
            .map(|(f, _)| f.option.name.clone())
            .collect()
    }

    #[test]
    fn rclones_provider_rules() {
        assert!(rule_matches("", "x"));
        assert!(rule_matches("a,b", "b"));
        assert!(!rule_matches("a,b", "c"));
        assert!(rule_matches("!a,b", "c"));
        assert!(!rule_matches("!a,b", "a"));
        assert!(
            rule_matches("a", ""),
            "no variant chosen: rclone shows every option"
        );
    }

    #[test]
    fn koofr_shows_and_sends_only_the_chosen_service() {
        let p = real("koofr");
        assert_eq!(p.options.iter().filter(|o| o.name == "password").count(), 3);
        let mut fields = fields_for(&p, None);
        // A new account starts on Koofr itself: no endpoint, one password.
        assert_eq!(variant(&fields), "koofr");
        assert_eq!(shown(&fields), ["provider", "user", "password"]);
        set(&mut fields, "user", 0, "me@example.com");
        let missing: Vec<_> = fields
            .iter()
            .zip(applicable(&fields))
            .filter(|(f, a)| *a && f.missing_required(false))
            .map(|(f, _)| f.option.name.clone())
            .collect();
        assert_eq!(missing, ["password"], "the endpoint of 'other' is not asked for");
        set(&mut fields, "password", 0, "secret");
        let sent = params_for(&fields, false);
        assert_eq!(sent.values["provider"], "koofr");
        assert_eq!(sent.values["password"], "secret");
        assert!(!sent.values.contains_key("endpoint"));

        // Switching to "other" brings its endpoint and its own password.
        set(&mut fields, "provider", 0, "other");
        assert_eq!(shown(&fields), ["provider", "endpoint", "user", "password"]);
        let p_other = fields
            .iter()
            .zip(applicable(&fields))
            .find(|(f, a)| *a && f.option.name == "password")
            .unwrap()
            .0;
        assert_eq!(p_other.option.provider, "other");
        set(&mut fields, "endpoint", 0, "https://dav.example");
        set(&mut fields, "password", 0, "other-secret");
        let sent = params_for(&fields, false);
        assert_eq!(
            sent.values["password"], "other-secret",
            "the koofr password does not overwrite it"
        );
        assert_eq!(sent.values["endpoint"], "https://dav.example");
    }

    #[test]
    fn storj_variants_follow_the_saved_provider_when_editing() {
        let p = real("storj");
        let saved = BTreeMap::from([("provider".to_string(), "new".to_string())]);
        let fields = fields_for(&p, Some(&saved));
        let names = shown(&fields);
        assert!(names.contains(&"api_key".to_string()), "{names:?}");
        assert!(!names.contains(&"access_grant".to_string()), "{names:?}");
        let fields = fields_for(&p, None);
        let names = shown(&fields);
        assert!(names.contains(&"access_grant".to_string()), "{names:?}");
        assert!(!names.contains(&"api_key".to_string()), "{names:?}");
        assert!(
            !params_for(&fields, false).values.contains_key("provider"),
            "its default is already set"
        );
    }
}
