//! `rustocop.toml` loading, discovery, strict validation, and resolution.
//!
//! Design decisions (see docs/configuration.md):
//! - TOML only; a missing file means "use the defaults".
//! - Discovered by walking up from the current directory to the git root,
//!   first hit wins.
//! - Strict: unknown keys, unknown cops/departments, or invalid settings fail
//!   the run instead of being silently ignored.
//! - Per-cop and per-department `enabled`, per-cop and global `exclude`
//!   globs, and for each cop the settings that it declares
//!   (`Cop::settings`), such as `max` for `Layout/LineLength`.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use globset::{Glob, GlobMatcher};
use serde::Deserialize;

mod ruby_version;

use crate::{
    cops::{
        Cop, Registry,
        policy::{DEFAULT_RUBY_VERSION, RubyVersion, SUPPORTED_RUBY_VERSIONS},
        settings::{Value, Values},
    },
    offense::Department,
};

/// The config file name.
pub const CONFIG_FILE_NAME: &str = "rustocop.toml";

/// Excludes that always apply, on top of `.gitignore` handling.
pub const BUILTIN_EXCLUDES: &[&str] = &["db/schema.rb", "vendor/**", "node_modules/**"];

/// The raw, deserialized config file.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields, default)]
pub struct RawConfig {
    /// The Ruby version the code runs on, as `"3.4"` or `"4.0"`.
    pub ruby_version: Option<String>,
    /// Glob patterns for paths to skip, relative to the config file.
    pub exclude: Vec<String>,
    /// Per-cop or per-department settings, keyed by full cop name (`Lint/Debugger`)
    /// or department name (`Style`).
    pub cops: BTreeMap<String, RawCopSettings>,
}

/// Settings for a single cop or a whole department.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct RawCopSettings {
    /// Whether the cop(s) run at all.
    pub enabled: Option<bool>,
    /// Glob patterns for paths this cop should skip (not valid for departments).
    /// Preserve presence so even an explicitly empty department setting is rejected.
    pub exclude: Option<Vec<String>>,
    /// Every other key: the cop's own settings, such as `max`. A key is
    /// valid when the cop declares it (`Cop::settings`), which `resolve`
    /// checks. A `max` on a cop that is off by default also switches the
    /// cop on.
    #[serde(flatten)]
    pub settings: BTreeMap<String, toml::Value>,
}

/// The validated, run-ready config.
#[derive(Debug)]
pub struct ResolvedConfig {
    /// The directory the config file lives in (or the cwd when there is no
    /// config file). Glob patterns are matched against paths relative to it.
    pub root: PathBuf,
    /// The Ruby version the run targets: the parser grammar and the cops that apply.
    pub ruby_version: RubyVersion,
    /// Where `ruby_version` comes from.
    pub ruby_version_source: RubyVersionSource,
    /// Combined builtin + user global excludes.
    pub global_excludes: Vec<GlobMatcher>,
    /// Settings keyed by full cop name or department name.
    pub cop_settings: BTreeMap<String, CopSettings>,
    /// The values of the settings that cops declare, keyed by cop name.
    pub settings: BTreeMap<String, Values>,
    /// The cops `settings` configure, built with their values, keyed by
    /// cop name. `cops::select_active` runs these instead of the
    /// registry's.
    pub configured: BTreeMap<String, Box<dyn Cop>>,
}

/// Where the Ruby version of a run comes from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RubyVersionSource {
    /// No file declares a version.
    #[default]
    Default,
    /// `ruby_version` in this config file.
    Config(PathBuf),
    /// A project file that declares `declared`.
    File { path: PathBuf, declared: String },
}

/// Validated settings for a cop or department.
#[derive(Debug, Default)]
pub struct CopSettings {
    /// Explicit enable/disable override, if set.
    pub enabled: Option<bool>,
    /// Exclude globs for this cop (always empty for department settings).
    pub excludes: Vec<GlobMatcher>,
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        ResolvedConfig {
            root: PathBuf::new(),
            ruby_version: DEFAULT_RUBY_VERSION,
            ruby_version_source: RubyVersionSource::Default,
            global_excludes: Vec::new(),
            cop_settings: BTreeMap::new(),
            settings: BTreeMap::new(),
            configured: BTreeMap::new(),
        }
    }
}

impl ResolvedConfig {
    /// Load, discover, and validate the config. `None` config path means the
    /// defaults. Without a `ruby_version` setting, the Ruby version comes from
    /// the project's version files, and is the default when there are none.
    pub fn load(registry: &Registry) -> Result<ResolvedConfig> {
        let (mut config, version_is_set) = match find_config()? {
            Some((path, raw)) => {
                let version_is_set = raw.ruby_version.is_some();
                (
                    ResolvedConfig::resolve(raw, Some(&path), registry)?,
                    version_is_set,
                )
            }
            None => (
                ResolvedConfig {
                    root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                    ruby_version: DEFAULT_RUBY_VERSION,
                    ruby_version_source: RubyVersionSource::Default,
                    global_excludes: compile_globs(BUILTIN_EXCLUDES, "builtin excludes")?,
                    cop_settings: BTreeMap::new(),
                    settings: BTreeMap::new(),
                    configured: BTreeMap::new(),
                },
                false,
            ),
        };
        if !version_is_set && let Some(detected) = ruby_version::detect(&config.root) {
            config.ruby_version = detected.version;
            config.ruby_version_source = RubyVersionSource::File {
                path: detected.path,
                declared: detected.declared,
            };
        }
        Ok(config)
    }

    /// One line that names the Ruby version of the run and where it comes from.
    pub fn ruby_version_report(&self) -> String {
        let source = match &self.ruby_version_source {
            RubyVersionSource::Default => "default; no file declares a version".into(),
            RubyVersionSource::Config(path) => format!("ruby_version in {}", path.display()),
            RubyVersionSource::File { path, declared } => {
                format!("{} declares {declared}", path.display())
            }
        };
        format!("Ruby {} ({source})", self.ruby_version)
    }

    /// Validate the raw config against the cop registry and compile globs.
    pub fn resolve(
        raw: RawConfig,
        config_path: Option<&Path>,
        registry: &Registry,
    ) -> Result<Self> {
        let root = match config_path.and_then(Path::parent) {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };

        let ruby_version_source = match &raw.ruby_version {
            None => RubyVersionSource::Default,
            Some(_) => RubyVersionSource::Config(
                config_path.map_or_else(|| CONFIG_FILE_NAME.into(), Path::to_path_buf),
            ),
        };
        let ruby_version = match raw.ruby_version {
            None => DEFAULT_RUBY_VERSION,
            Some(text) => SUPPORTED_RUBY_VERSIONS
                .into_iter()
                .find(|version| version.to_string() == text)
                .with_context(|| {
                    let supported: Vec<_> = SUPPORTED_RUBY_VERSIONS
                        .iter()
                        .map(|version| format!("\"{version}\""))
                        .collect();
                    format!(
                        "unsupported `ruby_version = \"{text}\"` in {}; use {}",
                        config_path
                            .map_or_else(|| CONFIG_FILE_NAME.into(), |p| p.display().to_string()),
                        supported.join(" or ")
                    )
                })?,
        };

        let mut cop_settings = BTreeMap::new();
        let mut settings = BTreeMap::new();
        let mut configured = BTreeMap::new();
        for (key, raw) in raw.cops {
            let is_cop = registry.has_cop(&key);
            let is_department = Department::parse(&key).is_some();
            if !is_cop && !is_department {
                let suggestion = registry
                    .suggest_name(&key)
                    .map(|name| format!("; did you mean `{name}`?"))
                    .unwrap_or_default();
                bail!(
                    "unknown cop or department `{key}` in {}{suggestion}",
                    config_path
                        .map_or_else(|| CONFIG_FILE_NAME.into(), |p| p.display().to_string())
                );
            }
            if is_department && raw.exclude.is_some() {
                bail!(
                    "`exclude` is not a valid setting for department `{key}`; use global or per-cop exclusions"
                );
            }
            let declared = registry.cop(&key).map_or(&[][..], |cop| cop.settings());
            let mut values = Values::default();
            for (name, value) in &raw.settings {
                let Some(setting) = declared.iter().find(|setting| setting.name == name) else {
                    bail!("`{name}` is not a valid setting for `{key}`");
                };
                values = values.with(setting.name, setting.parse(&key, value)?);
            }
            let excludes = compile_globs(raw.exclude.as_deref().unwrap_or_default(), &key)?;
            // A limit for a cop that is off by default asks for the cop.
            let wanted =
                values.count("max").is_some() && !crate::cops::policy::default_enabled(&key);
            cop_settings.insert(
                key.clone(),
                CopSettings {
                    enabled: raw.enabled.or(wanted.then_some(true)),
                    excludes,
                },
            );
            if !values.is_empty() {
                settings.insert(key, values);
            }
        }

        // The quotes that other cops take from Style/StringLiterals: those of
        // its style, and single quotes when the cop is disabled, as in
        // RuboCop.
        let style = |cop: &str| settings.get(cop)?.choice("enforced_style");
        let string_literals = cop_settings
            .get("Style/StringLiterals")
            .and_then(|s| s.enabled)
            .or_else(|| cop_settings.get("Style").and_then(|s| s.enabled))
            .unwrap_or(true);
        let single_quotes =
            !string_literals || style("Style/StringLiterals") == Some("single_quotes");
        // Quoted symbols have them unless Style/QuotedSymbols names a style
        // of its own.
        if let None | Some("same_as_string_literals") = style("Style/QuotedSymbols")
            && single_quotes
            && registry.has_cop("Style/QuotedSymbols")
        {
            let own: &mut Values = settings
                .entry("Style/QuotedSymbols".to_owned())
                .or_default();
            *own = std::mem::take(own).with("enforced_style", Value::Choice("single_quotes"));
        }

        for (name, values) in &settings {
            let cop = registry.cop(name).and_then(|cop| cop.configure(values));
            let Some(cop) = cop else {
                bail!("`{name}` declares settings that it cannot be built with");
            };
            configured.insert(name.clone(), cop);
        }

        // Style rules that construct guard/modifier examples use the configured
        // LineLength policy even when --only selects just the Style rule.
        let line_length_enabled = cop_settings
            .get("Layout/LineLength")
            .and_then(|s| s.enabled)
            .or_else(|| cop_settings.get("Layout").and_then(|s| s.enabled))
            .unwrap_or(true);
        let line_length_max = settings
            .get("Layout/LineLength")
            .and_then(|v| v.count("max"));
        let line_length_limit = line_length_enabled.then(|| line_length_max.unwrap_or(120));
        for cop in registry.cops() {
            if let Some(configured_cop) = cop.with_line_length_limit(line_length_limit) {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        let endless_enabled = cop_settings
            .get("Style/EndlessMethod")
            .and_then(|s| s.enabled)
            .or_else(|| cop_settings.get("Style").and_then(|s| s.enabled))
            .unwrap_or(true);
        for cop in registry.cops() {
            if let Some(configured_cop) = cop.with_endless_methods(endless_enabled) {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        let fetch_env_enabled = cop_settings
            .get("Style/FetchEnvVar")
            .and_then(|s| s.enabled)
            .or_else(|| cop_settings.get("Style").and_then(|s| s.enabled))
            .unwrap_or(true);
        for cop in registry.cops() {
            if let Some(configured_cop) = cop.with_fetch_env_var(fetch_env_enabled) {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        let bitwise_enabled = cop_settings
            .get("Style/BitwisePredicate")
            .and_then(|s| s.enabled)
            .or_else(|| cop_settings.get("Style").and_then(|s| s.enabled))
            .unwrap_or(true);
        for cop in registry.cops() {
            if let Some(configured_cop) = cop.with_bitwise_predicate(bitwise_enabled) {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        let companion_enabled = |name: &str| {
            cop_settings
                .get(name)
                .and_then(|s| s.enabled)
                .or_else(|| cop_settings.get("Style").and_then(|s| s.enabled))
                .unwrap_or_else(|| crate::cops::policy::default_enabled(name))
        };
        for cop in registry.cops() {
            if let Some(configured_cop) = cop.with_missing_else_companions(
                companion_enabled("Style/GuardClause"),
                companion_enabled("Style/UnlessElse"),
            ) {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        for cop in registry.cops() {
            if let Some(configured_cop) =
                cop.with_empty_else_missing_else(companion_enabled("Style/MissingElse"))
            {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        for cop in registry.cops() {
            if let Some(configured_cop) = cop
                .with_frozen_string_comment(companion_enabled("Style/FrozenStringLiteralComment"))
            {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        for cop in registry.cops() {
            if let Some(configured_cop) = cop.with_dig_chain(companion_enabled("Style/DigChain")) {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        let constant_resolution_enabled = cop_settings
            .get("Lint/ConstantResolution")
            .and_then(|s| s.enabled)
            .or_else(|| cop_settings.get("Lint").and_then(|s| s.enabled))
            .unwrap_or(false);
        for cop in registry.cops() {
            if let Some(configured_cop) = cop.with_constant_resolution(constant_resolution_enabled)
            {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        // Upstream's `cop_enabled?`: only the cop's own configured setting
        // enables the opt-in SingleLineBlockChain (never its department).
        let single_line_block_chain = cop_settings
            .get("Layout/SingleLineBlockChain")
            .and_then(|s| s.enabled)
            .unwrap_or(false);
        for cop in registry.cops() {
            let base = configured
                .get(cop.name())
                .map_or(cop.as_ref(), |configured| configured.as_ref());
            if let Some(configured_cop) = base.with_single_line_block_chain(single_line_block_chain)
            {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        // Upstream's `cop_enabled?`: the configured Layout/SpaceInsideParens,
        // or its department, never the CLI selection.
        let space_inside_parens = cop_settings
            .get("Layout/SpaceInsideParens")
            .and_then(|s| s.enabled)
            .or_else(|| cop_settings.get("Layout").and_then(|s| s.enabled))
            .unwrap_or(true);
        for cop in registry.cops() {
            if let Some(configured_cop) = cop.with_space_inside_parens(space_inside_parens) {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        // The cops that write a string literal take its quotes from
        // Style/StringLiterals, on top of what else configures them.
        for cop in registry.cops() {
            let base = configured
                .get(cop.name())
                .map_or(cop.as_ref(), |configured| configured.as_ref());
            if let Some(configured_cop) = base.with_single_quoted_strings(single_quotes) {
                configured.insert(cop.name().to_owned(), configured_cop);
            }
        }

        let mut global_excludes = compile_globs(BUILTIN_EXCLUDES, "builtin excludes")?;
        global_excludes.extend(compile_globs(&raw.exclude, "exclude")?);

        Ok(ResolvedConfig {
            root,
            ruby_version,
            ruby_version_source,
            global_excludes,
            cop_settings,
            settings,
            configured,
        })
    }
}

/// Find `rustocop.toml` walking up from the cwd to the git root, first hit wins.
///
/// Returns the config path and its parsed contents.
pub fn find_config() -> Result<Option<(PathBuf, RawConfig)>> {
    let cwd = std::env::current_dir().context("could not determine the current directory")?;
    find_config_from(&cwd)
}

/// Like [`find_config`] but starting from an explicit directory (testable).
pub fn find_config_from(start: &Path) -> Result<Option<(PathBuf, RawConfig)>> {
    let git_root = find_git_root(start);
    for dir in start.ancestors() {
        let candidate = dir.join(CONFIG_FILE_NAME);
        if candidate.is_file() {
            let text = fs::read_to_string(&candidate)
                .with_context(|| format!("failed to read {}", candidate.display()))?;
            let raw: RawConfig = toml::from_str(&text)
                .with_context(|| format!("invalid config in {}", candidate.display()))?;
            return Ok(Some((candidate, raw)));
        }
        if git_root == Some(dir) {
            break;
        }
    }
    Ok(None)
}

/// The closest ancestor of `start` that contains a `.git` entry.
fn find_git_root(start: &Path) -> Option<&Path> {
    start.ancestors().find(|dir| dir.join(".git").exists())
}

/// Compile glob patterns, with good error messages.
pub fn compile_globs<S: AsRef<str>>(patterns: &[S], context: &str) -> Result<Vec<GlobMatcher>> {
    patterns
        .iter()
        .map(|pattern| {
            Glob::new(pattern.as_ref())
                .with_context(|| {
                    format!("invalid glob pattern `{}` in `{context}`", pattern.as_ref())
                })
                .map(|glob| glob.compile_matcher())
        })
        .collect()
}

/// Compile the builtin exclude patterns.
pub fn builtin_excludes() -> Result<Vec<GlobMatcher>> {
    compile_globs(BUILTIN_EXCLUDES, "builtin excludes")
}

/// Whether any matcher matches the given (relative) path string.
pub fn matches_any(matchers: &[GlobMatcher], path: &Path) -> bool {
    let path = path.to_string_lossy();
    let path = path.trim_start_matches("./");
    matchers.iter().any(|m| m.is_match(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_top_level_keys() {
        let raw: Result<RawConfig> = toml::from_str("unknown_key = true\n").context("test");
        assert!(raw.is_err());
    }

    #[test]
    fn ruby_version_selects_a_supported_version_and_defaults_to_3_4() {
        let registry = Registry::new();
        let resolve = |text: &str| {
            ResolvedConfig::resolve(toml::from_str(text).unwrap(), None, &registry)
                .map(|config| config.ruby_version)
        };
        assert_eq!(resolve("").unwrap(), RubyVersion(3, 4));
        assert_eq!(
            resolve("ruby_version = \"3.4\"\n").unwrap(),
            RubyVersion(3, 4)
        );
        assert_eq!(
            resolve("ruby_version = \"4.0\"\n").unwrap(),
            RubyVersion(4, 0)
        );
        for unsupported in ["3.3", "4.1", "4", "4.0.9", "latest", ""] {
            let error = resolve(&format!("ruby_version = \"{unsupported}\"\n")).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "unsupported `ruby_version = \"{unsupported}\"` in rustocop.toml; use \"3.4\" or \"4.0\""
                )
            );
        }
        assert!(toml::from_str::<RawConfig>("ruby_version = 4.0\n").is_err());
    }

    #[test]
    fn accepts_minimal_config() {
        let raw: RawConfig = toml::from_str("").unwrap();
        assert!(raw.exclude.is_empty());
        assert!(raw.cops.is_empty());
    }

    #[test]
    fn parses_cop_and_department_settings() {
        let raw: RawConfig = toml::from_str(
            r#"
exclude = ["spec/fixtures/**"]

[cops."Lint/Debugger"]
enabled = false
exclude = ["spec/support/debugger_stub.rb"]

[cops."Style"]
enabled = false
"#,
        )
        .unwrap();
        assert_eq!(raw.exclude, ["spec/fixtures/**"]);
        let debugger = raw.cops.get("Lint/Debugger").unwrap();
        assert_eq!(debugger.enabled, Some(false));
        assert_eq!(
            debugger.exclude.as_deref().unwrap(),
            ["spec/support/debugger_stub.rb"]
        );
        let style = raw.cops.get("Style").unwrap();
        assert_eq!(style.enabled, Some(false));
    }

    #[test]
    fn rejects_unknown_cop() {
        let registry = Registry::new();
        let raw: RawConfig = toml::from_str("[cops.\"Lint/NoSuchCop\"]\nenabled = true").unwrap();
        let err = ResolvedConfig::resolve(raw, None, &registry).unwrap_err();
        assert!(err.to_string().contains("Lint/NoSuchCop"));
    }

    #[test]
    fn unknown_cop_errors_suggest_the_closest_registered_name() {
        let registry = Registry::new();
        for (typo, expected) in [
            ("Lint/Debbuger", "Lint/Debugger"),
            ("Style/StringLiteral", "Style/StringLiterals"),
            ("Lnit/Debugger", "Lint/Debugger"),
        ] {
            let raw: RawConfig =
                toml::from_str(&format!("[cops.\"{typo}\"]\nenabled = false")).unwrap();
            let err =
                ResolvedConfig::resolve(raw, Some(Path::new("project/rustocop.toml")), &registry)
                    .unwrap_err()
                    .to_string();
            assert!(
                err.contains(&format!("unknown cop or department `{typo}`")),
                "{err}"
            );
            assert!(err.contains("project/rustocop.toml"), "{err}");
            assert!(
                err.contains(&format!("did you mean `{expected}`?")),
                "{err}"
            );
        }
    }

    #[test]
    fn unknown_department_errors_suggest_the_closest_department() {
        let registry = Registry::new();
        for (typo, expected) in [("Layot", "Layout"), ("lint", "Lint")] {
            let raw: RawConfig =
                toml::from_str(&format!("[cops.\"{typo}\"]\nenabled = false")).unwrap();
            let err = ResolvedConfig::resolve(raw, None, &registry)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains(&format!("did you mean `{expected}`?")),
                "{err}"
            );
        }
    }

    #[test]
    fn unrelated_unknown_names_keep_the_documentation_hint_without_a_suggestion() {
        let registry = Registry::new();
        let raw: RawConfig =
            toml::from_str("[cops.\"Zzzzzzzzzzzzzzzz\"]\nenabled = false").unwrap();
        let err = ResolvedConfig::resolve(raw, None, &registry)
            .unwrap_err()
            .to_string();
        assert!(err.contains("Zzzzzzzzzzzzzzzz"), "{err}");
        assert!(err.contains("unknown cop or department"), "{err}");
        assert!(!err.contains("did you mean"), "{err}");
    }

    #[test]
    fn rejects_max_on_cop_without_it() {
        let registry = Registry::new();
        let raw: RawConfig = toml::from_str("[cops.\"Lint/Debugger\"]\nmax = 10").unwrap();
        let err = ResolvedConfig::resolve(raw, None, &registry).unwrap_err();
        assert!(err.to_string().contains("`max`"));
    }

    #[test]
    fn line_length_max_configures_the_cop_that_runs() {
        let registry = Registry::new();
        let raw: RawConfig = toml::from_str("[cops.\"Layout/LineLength\"]\nmax = 100").unwrap();
        let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
        assert_eq!(config.settings["Layout/LineLength"].count("max"), Some(100));
        let active = crate::cops::select_active(&registry, &config, &[], &[]);
        let line_length: Vec<_> = active
            .iter()
            .filter(|cop| cop.name() == "Layout/LineLength")
            .collect();
        assert_eq!(line_length.len(), 1, "{active:?}");
        let source = format!("{}\n", "a".repeat(101));
        let found = crate::pipeline::lint_str_with(&[*line_length[0]], &source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "Line is too long. [101/100]");

        // Without `max`, the default.
        let config = ResolvedConfig::default();
        let active = crate::cops::select_active(&registry, &config, &["Layout".into()], &[]);
        assert!(crate::pipeline::lint_str_with(&active, &source).is_empty());
    }

    #[test]
    fn cops_that_write_a_string_have_the_quotes_of_string_literals() {
        let registry = Registry::new();
        let lint = |toml: &str, cop: &str, source: &str| {
            let raw: RawConfig = toml::from_str(toml).unwrap();
            let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
            let active = crate::cops::select_active(&registry, &config, &[cop.into()], &[]);
            let found = crate::pipeline::lint_str_with(&active, source);
            found.into_iter().map(|o| o.message).collect::<Vec<_>>()
        };
        let single = "[cops.\"Style/StringLiterals\"]\nenforced_style = \"single_quotes\"\n";
        for (toml, q) in [("", '"'), (single, '\'')] {
            let regexp = lint(toml, "Style/RedundantRegexpArgument", "'foo'.split(/f/)\n");
            let message = format!("Use string `{q}f{q}` as argument instead of regexp `/f/`.");
            assert_eq!(regexp, [message], "{toml}");
            let literal = lint(toml, "Style/EmptyLiteral", "x = String.new\n");
            let message = format!("Use string literal `{q}{q}` instead of `String.new`.");
            assert_eq!(literal, [message], "{toml}");
        }
        // A disabled Style/StringLiterals means single quotes, for the
        // quoted symbols too.
        let disabled = "[cops.\"Style/StringLiterals\"]\nenabled = false\n";
        assert_eq!(
            lint(disabled, "Style/EmptyLiteral", "x = String.new\n"),
            ["Use string literal `''` instead of `String.new`."]
        );
        for (toml, flagged) in [
            ("", ":'a b'\n"),
            (single, ":\"a b\"\n"),
            (disabled, ":\"a b\"\n"),
        ] {
            assert_eq!(
                lint(toml, "Style/QuotedSymbols", flagged).len(),
                1,
                "{toml}"
            );
        }
        let double = format!(
            "{disabled}[cops.\"Style/QuotedSymbols\"]\nenforced_style = \"double_quotes\"\n"
        );
        assert_eq!(lint(&double, "Style/QuotedSymbols", ":'a b'\n").len(), 1);
        // The quotes come on top of the frozen-string policy of EmptyLiteral.
        let frozen =
            format!("{single}[cops.\"Style/FrozenStringLiteralComment\"]\nenabled = true\n");
        assert!(lint(&frozen, "Style/EmptyLiteral", "x = String.new\n").is_empty());
    }

    #[test]
    fn line_length_max_must_be_positive() {
        let registry = Registry::new();
        let raw: RawConfig = toml::from_str("[cops.\"Layout/LineLength\"]\nmax = 0").unwrap();
        let err = ResolvedConfig::resolve(raw, None, &registry).unwrap_err();
        assert!(err.to_string().contains("at least 1"), "{err}");
        let raw: RawConfig = toml::from_str("[cops.\"Layout/LineLength\"]\nmax = -1").unwrap();
        let err = ResolvedConfig::resolve(raw, None, &registry).unwrap_err();
        assert!(err.to_string().contains("at least 1"), "{err}");
    }

    #[test]
    fn rejects_max_on_a_department() {
        let registry = Registry::new();
        let raw: RawConfig = toml::from_str("[cops.\"Layout\"]\nmax = 100").unwrap();
        let err = ResolvedConfig::resolve(raw, None, &registry).unwrap_err();
        assert!(
            err.to_string()
                .contains("`max` is not a valid setting for `Layout`")
        );
    }

    #[test]
    fn rejects_department_exclude_even_when_empty() {
        let registry = Registry::new();
        for department in Department::ALL {
            for exclude in ["[]", "[\"x.rb\"]"] {
                let raw: RawConfig =
                    toml::from_str(&format!("[cops.\"{department}\"]\nexclude = {exclude}"))
                        .unwrap();
                let err = ResolvedConfig::resolve(raw, None, &registry).unwrap_err();
                assert!(
                    err.to_string().contains(&format!(
                        "`exclude` is not a valid setting for department `{department}`"
                    )),
                    "{err}"
                );
            }
        }
    }

    #[test]
    fn rejects_invalid_glob() {
        let registry = Registry::new();
        let raw: RawConfig = toml::from_str("exclude = [\"[unclosed\"]").unwrap();
        let err = ResolvedConfig::resolve(raw, None, &registry).unwrap_err();
        assert!(err.to_string().contains("[unclosed"));
    }

    /// A test-owned tempdir laid out as `outer/repo/.git` plus
    /// `outer/repo/app/models`, with `files` (relative to `outer`) written
    /// into it. Returns the tempdir guard, the repo root and the subdirectory.
    ///
    /// Every file a discovery test writes lives under `outer`, and the `.git`
    /// entry bounds the walk-up inside it, so tests neither touch the shared
    /// temp directory nor see each other's (or anyone's) stray configs.
    fn repo_tree(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let outer = tempfile::tempdir().unwrap();
        let repo = outer.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let sub = repo.join("app/models");
        std::fs::create_dir_all(&sub).unwrap();
        for (name, contents) in files {
            std::fs::write(outer.path().join(name), contents).unwrap();
        }
        (outer, repo, sub)
    }

    #[test]
    fn discovers_config_from_subdirectory() {
        // The git-root boundary is covered by `stops_at_the_git_root`.
        let (_outer, repo, sub) = repo_tree(&[(
            "repo/rustocop.toml",
            "[cops.\"Lint/Debugger\"]\nenabled = false\n",
        )]);

        let (path, raw) = find_config_from(&sub)
            .unwrap()
            .expect("config should be discovered");
        assert_eq!(path, repo.join(CONFIG_FILE_NAME));
        assert_eq!(raw.cops["Lint/Debugger"].enabled, Some(false));
    }

    #[test]
    fn discovers_config_at_the_git_root_itself() {
        let (_outer, repo, _sub) = repo_tree(&[("repo/rustocop.toml", "")]);
        let (path, _) = find_config_from(&repo)
            .unwrap()
            .expect("config at the root");
        assert_eq!(path, repo.join(CONFIG_FILE_NAME));
    }

    #[test]
    fn nearest_config_wins() {
        let (_outer, repo, sub) = repo_tree(&[
            ("repo/rustocop.toml", "unused = true"),
            ("repo/app/rustocop.toml", ""),
        ]);
        let (path, _) = find_config_from(&sub).unwrap().expect("nearest config");
        assert_eq!(path, repo.join("app").join(CONFIG_FILE_NAME));
    }

    #[test]
    fn stops_at_the_git_root() {
        // A config above the git root is not part of the project.
        let (_outer, _repo, sub) = repo_tree(&[("rustocop.toml", "unused = true")]);
        assert!(find_config_from(&sub).unwrap().is_none());
    }

    #[test]
    fn finds_nothing_when_absent() {
        let (_outer, repo, sub) = repo_tree(&[]);
        assert!(find_config_from(&sub).unwrap().is_none());
        assert!(find_config_from(&repo).unwrap().is_none());
    }

    #[test]
    fn reports_invalid_config_with_its_path() {
        let (_outer, repo, sub) = repo_tree(&[("repo/rustocop.toml", "unused = true")]);
        let err = find_config_from(&sub).unwrap_err();
        assert!(
            err.to_string()
                .contains(&repo.join(CONFIG_FILE_NAME).display().to_string()),
            "{err:#}"
        );
    }
}
