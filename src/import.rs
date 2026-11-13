//! `--import-rubocop-config`: a `rustocop.toml` from a project's
//! `.rubocop.yml`.
//!
//! A `rustocop.toml` says much less than a `.rubocop.yml`: the paths to skip,
//! the Ruby version, whether a cop or department is on, the paths a cop
//! skips, the `max` of `Layout/LineLength` and the Metrics cops, the
//! `enforced_style` of the cops for the quotes of literals, and the
//! `allowed_patterns` of `Layout/LineLength`. The import carries those over
//! and lists every other setting, so that none is dropped silently.

use std::{collections::BTreeMap, fmt::Write as _, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use yaml_rust2::{Yaml, YamlLoader};

use crate::{
    cops::{Registry, policy::SUPPORTED_RUBY_VERSIONS},
    offense::Department,
};

/// The file an import reads.
// The current directory only; RuboCop also looks in parent
// directories and in `.config/`.
pub const RUBOCOP_FILE_NAME: &str = ".rubocop.yml";

/// The settings of a `rustocop.toml` whose default in RuboCop is another
/// one: a RuboCop config that does not name the setting means this value.
const RUBOCOP_DEFAULTS: &[(&str, &str, &str)] = &[
    ("Style/StringLiterals", "EnforcedStyle", "single_quotes"),
    (
        "Style/StringLiteralsInInterpolation",
        "EnforcedStyle",
        "single_quotes",
    ),
];

/// How deep `inherit_from` can nest. A file that inherits from itself ends
/// here.
const INHERIT_DEPTH: usize = 10;

/// The settings of a RuboCop config, by section (`AllCops`, a department or
/// a cop) and key.
type Sections = BTreeMap<String, BTreeMap<String, Yaml>>;

/// What an import gives.
#[derive(Default)]
pub struct Import {
    /// The text of the `rustocop.toml`; empty when nothing carries over.
    pub toml: String,
    /// The files read, an inherited file before the one that names it.
    pub files: Vec<String>,
    /// The settings that a `rustocop.toml` cannot say, one line each.
    pub dropped: Vec<String>,
    /// Whether the config inherits from a gem or a URL. Those are not read,
    /// so what the config leaves unsaid is not known to be RuboCop's default.
    base_unread: bool,
}

/// Import the RuboCop config at `path` and the files it inherits from.
pub fn import(path: &Path, registry: &Registry) -> Result<Import> {
    let mut import = Import::default();
    let mut sections = load(path, 0, &mut import)?;
    if !import.base_unread {
        for (cop, key, value) in RUBOCOP_DEFAULTS {
            let section = sections.entry((*cop).to_owned()).or_default();
            let default = || Yaml::String((*value).to_owned());
            section.entry((*key).to_owned()).or_insert_with(default);
        }
    }
    let mut settings = String::new();
    let mut unknown = Vec::new();
    // `AllCops` first: its keys are top-level ones, which TOML wants before
    // the tables.
    let all_cops = sections.remove_entry("AllCops");
    for (name, section) in all_cops.into_iter().chain(sections) {
        let top = name == "AllCops";
        let cop = registry.has_cop(&name);
        if !top && !cop && Department::parse(&name).is_none() {
            unknown.push(name);
            continue;
        }
        let (mut lines, mut dropped) = (String::new(), Vec::new());
        for (key, value) in section {
            match (key.as_str(), value) {
                ("Enabled", Yaml::Boolean(on)) if !top => {
                    let _ = writeln!(lines, "enabled = {on}");
                }
                // A department takes no `exclude`.
                ("Exclude", Yaml::Array(patterns)) if top || cop => {
                    let mut kept = String::new();
                    for pattern in patterns {
                        let pattern = pattern.into_string().unwrap_or_default();
                        let glob = pattern.trim_start_matches("./");
                        // Paths are matched relative to the config file, so
                        // an absolute pattern matches nothing, and neither
                        // does a regexp, which YAML gives as `/.../`.
                        if glob.starts_with('/') || globset::Glob::new(glob).is_err() {
                            dropped.push(format!("Exclude `{pattern}`"));
                        } else {
                            let _ = writeln!(kept, "  {},", toml::Value::String(glob.into()));
                        }
                    }
                    if !kept.is_empty() {
                        let _ = write!(lines, "exclude = [\n{kept}]\n");
                    }
                }
                ("Max", Yaml::Integer(max))
                    if max > 0 && registry.cop_declares_max(&name).is_some() =>
                {
                    let _ = writeln!(lines, "max = {max}");
                }
                ("AllowedPatterns", Yaml::Array(patterns))
                    if registry.cop_allows_patterns(&name) =>
                {
                    let mut kept = String::new();
                    for pattern in patterns {
                        let pattern = pattern.into_string().unwrap_or_default();
                        match rust_regex(&pattern) {
                            Some(regex) => {
                                let _ = writeln!(kept, "  {},", toml::Value::String(regex));
                            }
                            None => dropped.push(format!("AllowedPatterns `{pattern}`")),
                        }
                    }
                    if !kept.is_empty() {
                        let _ = write!(lines, "allowed_patterns = [\n{kept}]\n");
                    }
                }
                ("EnforcedStyle", Yaml::String(style))
                    if registry.cop_styles(&name).contains(&style.as_str()) =>
                {
                    let _ = writeln!(lines, "enforced_style = \"{style}\"");
                }
                ("TargetRubyVersion", Yaml::Real(version) | Yaml::String(version))
                    if top
                        && SUPPORTED_RUBY_VERSIONS
                            .iter()
                            .any(|v| v.to_string() == version) =>
                {
                    let _ = writeln!(lines, "ruby_version = \"{version}\"");
                }
                _ => dropped.push(key),
            }
        }
        if !lines.is_empty() {
            if !top {
                let _ = writeln!(settings, "\n[cops.\"{name}\"]");
            }
            settings.push_str(&lines);
        }
        if !dropped.is_empty() {
            import
                .dropped
                .push(format!("{name}: {}", dropped.join(", ")));
        }
    }
    if !unknown.is_empty() {
        import.dropped.push(format!(
            "not rustocop cops or departments: {}",
            unknown.join(", ")
        ));
    }
    if !settings.is_empty() {
        import.toml = format!(
            "# Written by `rustocop --import-rubocop-config` from {}.\n{settings}",
            import.files.join(" and ")
        );
    }
    Ok(import)
}

/// `pattern`, an entry of `AllowedPatterns`, as a regular expression of the
/// `regex` crate; `None` when it is not one. An entry is the source of a
/// Ruby regexp, or a regexp literal.
// The YAML parser drops the `!ruby/regexp` tag, so a literal is
// known by its slashes. The two syntaxes share what most patterns use; the
// classes `\d`, `\w` and `\s` are ASCII in Ruby and Unicode here.
fn rust_regex(pattern: &str) -> Option<String> {
    let literal = pattern
        .strip_prefix('/')
        .and_then(|rest| rest.rsplit_once('/'));
    let regex = match literal {
        Some((source, flags)) => {
            let mut prefix = String::new();
            for flag in flags.chars() {
                // Ruby's `m` lets `.` match a newline, which is `s` here.
                prefix.push_str(match flag {
                    'i' => "(?i)",
                    'x' => "(?x)",
                    'm' => "(?s)",
                    _ => return None,
                });
            }
            prefix + source
        }
        None => pattern.to_owned(),
    };
    regex::bytes::Regex::new(&regex).is_ok().then_some(regex)
}

/// The sections of the config at `path`, laid over those of the files it
/// inherits from.
fn load(path: &Path, depth: usize, import: &mut Import) -> Result<Sections> {
    if depth > INHERIT_DEPTH {
        bail!(
            "{}: `inherit_from` nests more than {INHERIT_DEPTH} files deep",
            path.display()
        );
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let documents = YamlLoader::load_from_str(&text)
        .map_err(|err| anyhow!("invalid YAML in {}: {err}", path.display()))?;
    let mut top = documents
        .into_iter()
        .next()
        .and_then(Yaml::into_hash)
        .unwrap_or_default();
    let key = |name: &str| Yaml::String(name.into());

    // `inherit_mode: merge: [Exclude]` joins the `Exclude` lists of this file
    // and the inherited ones.
    let join_excludes = top.remove(&key("inherit_mode")).is_some_and(|mode| {
        let merged = mode["merge"].as_vec();
        merged.is_some_and(|keys| keys.contains(&key("Exclude")))
    });
    let inherited = match top.remove(&key("inherit_from")) {
        Some(Yaml::Array(files)) => files,
        Some(file) => vec![file],
        None => Vec::new(),
    };
    let mut sections = Sections::new();
    for file in inherited.iter().filter_map(Yaml::as_str) {
        if file.contains("://") {
            import.dropped.push(format!("inherit_from: {file}"));
            import.base_unread = true;
            continue;
        }
        // The `Exclude` paths of an inherited file are read as
        // relative to the imported file. RuboCop reads those of a
        // `.rubocop*.yml` in another directory as relative to that directory.
        let parent = load(&path.with_file_name(file), depth + 1, import)?;
        merge(&mut sections, parent, join_excludes);
    }
    import.files.push(path.display().to_string());

    let mut own = Sections::new();
    for (name, value) in top {
        let Some(name) = name.into_string() else {
            continue;
        };
        match value {
            // The name of a section starts with a capital letter; `require`,
            // `plugins` and `inherit_gem` are directives.
            Yaml::Hash(section) if name.starts_with(char::is_uppercase) => {
                let pair = |(key, value): (Yaml, Yaml)| Some((key.into_string()?, value));
                own.insert(name, section.into_iter().filter_map(pair).collect());
            }
            _ => {
                import.base_unread |= name == "inherit_gem";
                if !import.dropped.contains(&name) {
                    import.dropped.push(name);
                }
            }
        }
    }
    merge(&mut sections, own, join_excludes);
    Ok(sections)
}

/// Lay `over` on `base` as RuboCop does for `inherit_from`: a key of the
/// inheriting file replaces the inherited one, unless both are `Exclude`
/// lists to join.
fn merge(base: &mut Sections, over: Sections, join_excludes: bool) {
    for (name, section) in over {
        let merged = base.entry(name).or_default();
        for (key, value) in section {
            if join_excludes
                && key == "Exclude"
                && let (Some(Yaml::Array(list)), Yaml::Array(more)) = (merged.get_mut(&key), &value)
            {
                list.extend(more.iter().cloned());
            } else {
                merged.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RawConfig, ResolvedConfig, matches_any};

    fn import_files(files: &[(&str, &str)]) -> Result<Import> {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        let mut import = import(&dir.path().join(files[0].0), &Registry::new())?;
        let root = format!("{}/", dir.path().display());
        for file in &mut import.files {
            *file = file.replace(&root, "");
        }
        import.toml = import.toml.replace(&root, "");
        Ok(import)
    }

    #[test]
    fn it_carries_over_what_a_rustocop_toml_can_say_and_lists_the_rest() {
        let import = import_files(&[(
            ".rubocop.yml",
            "require:\n  - rubocop-rails\n\
             AllCops:\n  NewCops: enable\n  TargetRubyVersion: 4.0\n  Exclude:\n    - 'bin/**/*'\n    - ./tmp/*\n    - !ruby/regexp /_pb\\.rb$/\n\
             Style:\n  Enabled: false\n  Exclude: ['a.rb']\n\
             Style/StringLiterals:\n  Enabled: true\n  EnforcedStyle: single_quotes\n\
             Style/HashSyntax:\n  EnforcedStyle: ruby19\n\
             Layout/LineLength:\n  Max: 100\n  Exclude:\n    - \"spec/**/*\"\n  AllowedPatterns:\n    - !ruby/regexp /\\A +(it|describe) [\"']/i\n    - '^\\s*#'\n    - '(?<=a)b'\n\
             Metrics/MethodLength:\n  Max: 20\n  Enabled: pending\n\
             Rails/Output:\n  Enabled: false\n\
             Lint/Debugger:\n  Exclude: []\n",
        )])
        .unwrap();
        assert_eq!(
            import.toml,
            "# Written by `rustocop --import-rubocop-config` from .rubocop.yml.\n\
             exclude = [\n  \"bin/**/*\",\n  \"tmp/*\",\n]\n\
             ruby_version = \"4.0\"\n\
             \n\
             [cops.\"Layout/LineLength\"]\n\
             allowed_patterns = [\n  '''(?i)\\A +(it|describe) [\"']''',\n  '^\\s*#',\n]\n\
             exclude = [\n  \"spec/**/*\",\n]\n\
             max = 100\n\
             \n\
             [cops.\"Metrics/MethodLength\"]\n\
             max = 20\n\
             \n\
             [cops.\"Style\"]\n\
             enabled = false\n\
             \n\
             [cops.\"Style/StringLiterals\"]\n\
             enabled = true\n\
             enforced_style = \"single_quotes\"\n\
             \n\
             [cops.\"Style/StringLiteralsInInterpolation\"]\n\
             enforced_style = \"single_quotes\"\n"
        );
        assert_eq!(
            import.dropped,
            [
                "require",
                "AllCops: Exclude `/_pb\\.rb$/`, NewCops",
                "Layout/LineLength: AllowedPatterns `(?<=a)b`",
                "Metrics/MethodLength: Enabled",
                "Style: Exclude",
                "Style/HashSyntax: EnforcedStyle",
                "not rustocop cops or departments: Rails/Output",
            ]
        );
        // The config is one that rustocop accepts.
        let raw: RawConfig = toml::from_str(&import.toml).unwrap();
        let config = ResolvedConfig::resolve(raw, None, &Registry::new()).unwrap();
        assert!(matches_any(&config.global_excludes, Path::new("tmp/x.rb")));
        assert_eq!(config.params["Layout/LineLength"], 100);
    }

    #[test]
    fn an_inheriting_file_replaces_the_keys_of_the_files_it_inherits_from() {
        let todo = "Lint/Debugger:\n  Exclude:\n    - a.rb\n\
                    Lint/Void:\n  Enabled: false\n  Exclude:\n    - b.rb\n";
        let main = "inherit_from:\n  - .rubocop_todo.yml\n  - https://example.com/base.yml\n\
                    Lint/Debugger:\n  Exclude:\n    - c.rb\n";
        let import = import_files(&[(".rubocop.yml", main), (".rubocop_todo.yml", todo)]).unwrap();
        assert_eq!(import.files, [".rubocop_todo.yml", ".rubocop.yml"]);
        assert_eq!(
            import.toml,
            "# Written by `rustocop --import-rubocop-config` from .rubocop_todo.yml and .rubocop.yml.\n\
             \n\
             [cops.\"Lint/Debugger\"]\n\
             exclude = [\n  \"c.rb\",\n]\n\
             \n\
             [cops.\"Lint/Void\"]\n\
             enabled = false\n\
             exclude = [\n  \"b.rb\",\n]\n"
        );
        assert_eq!(
            import.dropped,
            ["inherit_from: https://example.com/base.yml"]
        );

        // `inherit_mode` joins the lists; a single file name is accepted too.
        let main = "inherit_mode:\n  merge:\n    - Exclude\n\
                    inherit_from: .rubocop_todo.yml\n\
                    Lint/Debugger:\n  Exclude:\n    - c.rb\n";
        let import = import_files(&[(".rubocop.yml", main), (".rubocop_todo.yml", todo)]).unwrap();
        assert!(
            import
                .toml
                .contains("[cops.\"Lint/Debugger\"]\nexclude = [\n  \"a.rb\",\n  \"c.rb\",\n]\n")
        );
    }

    #[test]
    fn nothing_to_carry_over_gives_no_config_and_bad_files_are_errors() {
        // A gem's config is not read, so no default of RuboCop is assumed.
        let import = import_files(&[(
            ".rubocop.yml",
            "inherit_gem:\n  standard: config/base.yml\n\
             Style/HashSyntax:\n  EnforcedStyle: ruby19\n",
        )])
        .unwrap();
        assert_eq!(import.toml, "");
        assert_eq!(
            import.dropped,
            ["inherit_gem", "Style/HashSyntax: EnforcedStyle"]
        );
        // RuboCop's own defaults are single quotes; rustocop's are double.
        assert_eq!(
            import_files(&[(".rubocop.yml", "")]).unwrap().toml,
            "# Written by `rustocop --import-rubocop-config` from .rubocop.yml.\n\
             \n\
             [cops.\"Style/StringLiterals\"]\n\
             enforced_style = \"single_quotes\"\n\
             \n\
             [cops.\"Style/StringLiteralsInInterpolation\"]\n\
             enforced_style = \"single_quotes\"\n"
        );

        let error = |files| format!("{:#}", import_files(files).err().unwrap());
        assert!(
            error(&[(".rubocop.yml", "inherit_from: missing.yml\n")]).contains("could not read")
        );
        assert!(
            error(&[(".rubocop.yml", "inherit_from: .rubocop.yml\n")])
                .contains("nests more than 10 files deep")
        );
        assert!(error(&[(".rubocop.yml", "Style: [\n")]).contains("invalid YAML in"));
    }
}
