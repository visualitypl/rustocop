//! The settings a cop takes in `rustocop.toml`, beyond `enabled` and
//! `exclude`.
//!
//! A cop declares its settings ([`Cop::settings`](super::Cop::settings)) and
//! builds a copy of itself from their values
//! ([`Cop::configure`](super::Cop::configure)). The config and the import of
//! a `.rubocop.yml` read the declarations, so a new setting needs no code in
//! either: declare it on the cop and read it in `configure`.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use regex::bytes::Regex;

/// A setting that a cop declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    /// The key in `rustocop.toml`: the name of the parameter in RuboCop, in
    /// snake case (`EnforcedStyle` is `enforced_style`).
    pub name: &'static str,
    /// The values that the setting takes.
    pub kind: Kind,
}

/// The values that a setting takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A whole number of at least 1.
    Count,
    /// One of these strings.
    Choice(&'static [&'static str]),
    /// A list of regular expressions, in the syntax of the `regex` crate.
    Patterns,
}

/// `max`, RuboCop's `Max`.
pub const MAX: Setting = Setting {
    name: "max",
    kind: Kind::Count,
};

/// The value of a setting, valid for its [`Kind`].
#[derive(Debug, Clone)]
pub enum Value {
    Count(usize),
    /// The string as the setting declares it.
    Choice(&'static str),
    Patterns(Vec<Regex>),
}

impl Setting {
    /// Whether `key` is the name of this setting in a `.rubocop.yml`.
    // Compares without case and underscores, which needs no table
    // of acronyms (`AllowURI`). A key in the wrong case matches too, and
    // RuboCop ignores such a key; convert the name if that ever matters.
    pub fn is_rubocop_key(&self, key: &str) -> bool {
        key.eq_ignore_ascii_case(&self.name.replace('_', ""))
    }

    /// The value that `raw` gives this setting of `cop`.
    pub fn parse(&self, cop: &str, raw: &toml::Value) -> Result<Value> {
        let name = self.name;
        match (self.kind, raw) {
            (Kind::Count, toml::Value::Integer(count)) => match usize::try_from(*count) {
                Ok(count) if count > 0 => Ok(Value::Count(count)),
                _ => bail!("`{name}` for `{cop}` must be at least 1"),
            },
            (Kind::Count, _) => bail!("`{name}` for `{cop}` must be a whole number"),
            (Kind::Choice(choices), toml::Value::String(text)) => {
                if let Some(choice) = choices.iter().find(|choice| *choice == text) {
                    return Ok(Value::Choice(choice));
                }
                let known: Vec<_> = choices.iter().map(|c| format!("\"{c}\"")).collect();
                bail!(
                    "unknown `{name} = \"{text}\"` for `{cop}`; use {}",
                    known.join(" or ")
                )
            }
            (Kind::Choice(_), _) => bail!("`{name}` for `{cop}` must be a string"),
            (Kind::Patterns, toml::Value::Array(entries)) => {
                let compile = |entry: &toml::Value| {
                    let Some(pattern) = entry.as_str() else {
                        bail!("`{name}` for `{cop}` must be a list of strings");
                    };
                    Regex::new(pattern)
                        .with_context(|| format!("invalid `{name}` entry `{pattern}` for `{cop}`"))
                };
                let patterns = entries.iter().map(compile).collect::<Result<_>>()?;
                Ok(Value::Patterns(patterns))
            }
            (Kind::Patterns, _) => bail!("`{name}` for `{cop}` must be a list of strings"),
        }
    }
}

/// The values that a config gives the settings of one cop, by setting name.
/// A setting that the config does not name has no value here.
#[derive(Debug, Clone, Default)]
pub struct Values(BTreeMap<&'static str, Value>);

impl Values {
    /// These values and `value` for the setting `name`.
    #[must_use]
    pub fn with(mut self, name: &'static str, value: Value) -> Self {
        self.0.insert(name, value);
        self
    }

    /// Whether the config names no setting.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The value of the [`Kind::Count`] setting `name`.
    pub fn count(&self, name: &str) -> Option<usize> {
        match self.0.get(name)? {
            Value::Count(count) => Some(*count),
            _ => None,
        }
    }

    /// The value of the [`Kind::Choice`] setting `name`.
    pub fn choice(&self, name: &str) -> Option<&'static str> {
        match self.0.get(name)? {
            Value::Choice(choice) => Some(choice),
            _ => None,
        }
    }

    /// The value of the [`Kind::Patterns`] setting `name`.
    pub fn patterns(&self, name: &str) -> Option<&[Regex]> {
        match self.0.get(name)? {
            Value::Patterns(patterns) => Some(patterns),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(kind: Kind, toml: &str) -> Result<Values, String> {
        let setting = Setting { name: "key", kind };
        let raw: toml::Table = toml::from_str(&format!("key = {toml}")).unwrap();
        match setting.parse("Dept/Cop", &raw["key"]) {
            Ok(value) => Ok(Values::default().with("key", value)),
            Err(error) => Err(error.to_string()),
        }
    }

    #[test]
    fn a_value_is_read_as_the_kind_of_its_setting() {
        assert_eq!(parse(Kind::Count, "7").unwrap().count("key"), Some(7));
        let choice = Kind::Choice(&["one", "two"]);
        assert_eq!(parse(choice, "'two'").unwrap().choice("key"), Some("two"));
        let patterns = parse(Kind::Patterns, r"['^a', 'b$']").unwrap();
        assert_eq!(patterns.patterns("key").unwrap().len(), 2);
        // A value is read only as its own kind, and only under its name.
        assert_eq!(patterns.count("key"), None);
        assert_eq!(patterns.choice("other"), None);

        for (kind, toml, message) in [
            (Kind::Count, "0", "`key` for `Dept/Cop` must be at least 1"),
            (Kind::Count, "-1", "`key` for `Dept/Cop` must be at least 1"),
            (
                Kind::Count,
                "'7'",
                "`key` for `Dept/Cop` must be a whole number",
            ),
            (
                choice,
                "'three'",
                "unknown `key = \"three\"` for `Dept/Cop`; use \"one\" or \"two\"",
            ),
            (choice, "1", "`key` for `Dept/Cop` must be a string"),
            (
                Kind::Patterns,
                "'a'",
                "`key` for `Dept/Cop` must be a list of strings",
            ),
            (
                Kind::Patterns,
                "[1]",
                "`key` for `Dept/Cop` must be a list of strings",
            ),
            (
                Kind::Patterns,
                "['(?<=a)b']",
                "invalid `key` entry `(?<=a)b` for `Dept/Cop`",
            ),
        ] {
            assert_eq!(parse(kind, toml).unwrap_err(), message, "{toml}");
        }
    }

    #[test]
    fn a_rubocop_key_is_the_name_in_camel_case() {
        let named = |name| Setting {
            name,
            kind: Kind::Count,
        };
        assert!(MAX.is_rubocop_key("Max"));
        assert!(named("enforced_style").is_rubocop_key("EnforcedStyle"));
        assert!(named("allow_uri").is_rubocop_key("AllowURI"));
        assert!(!named("enforced_style").is_rubocop_key("EnforcedStyleForMultiline"));
    }
}
