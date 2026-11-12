//! `rustocop.toml` loading, discovery, strict validation, and resolution.
//!
//! Design decisions (see the project README):
//! - TOML only; a missing file means "use the defaults".
//! - Discovered by walking up from the current directory to the git root,
//!   first hit wins.
//! - Strict: unknown keys, unknown cops/departments, or invalid settings fail
//!   the run instead of being silently ignored.
//! - Minimal surface: per-cop and per-department `enabled`, per-cop and
//!   global `exclude` globs, and `max` for `Layout/LineLength`. There are no
//!   per-cop style parameters — defaults are the product; disagreeing teams
//!   disable a cop rather than reconfigure it.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use globset::{Glob, GlobMatcher};
use serde::Deserialize;

use crate::{
    cops::{Cop, Registry},
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
    /// Glob patterns for paths to skip, relative to the config file.
    pub exclude: Vec<String>,
    /// Per-cop or per-department settings, keyed by full cop name (`Lint/Debugger`)
    /// or department name (`Style`).
    pub cops: BTreeMap<String, RawCopSettings>,
}

/// Settings for a single cop or a whole department.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields, default)]
pub struct RawCopSettings {
    /// Whether the cop(s) run at all.
    pub enabled: Option<bool>,
    /// Glob patterns for paths this cop should skip (not valid for departments).
    /// Preserve presence so even an explicitly empty department setting is rejected.
    pub exclude: Option<Vec<String>>,
    /// A numeric parameter. Only valid for cops that declare one
    /// (`Layout/LineLength` today).
    pub max: Option<usize>,
}

/// The validated, run-ready config.
#[derive(Debug)]
pub struct ResolvedConfig {
    /// The directory the config file lives in (or the cwd when there is no
    /// config file). Glob patterns are matched against paths relative to it.
    pub root: PathBuf,
    /// Combined builtin + user global excludes.
    pub global_excludes: Vec<GlobMatcher>,
    /// Settings keyed by full cop name or department name.
    pub cop_settings: BTreeMap<String, CopSettings>,
    /// Numeric parameters, keyed by cop name.
    pub params: BTreeMap<String, usize>,
    /// The cops `params` configure, built with their settings, keyed by
    /// cop name. `cops::select_active` runs these instead of the
    /// registry's.
    pub configured: BTreeMap<String, Box<dyn Cop>>,
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
            global_excludes: Vec::new(),
            cop_settings: BTreeMap::new(),
            params: BTreeMap::new(),
            configured: BTreeMap::new(),
        }
    }
}

impl ResolvedConfig {
    /// Load, discover, and validate the config. `None` config path means the
    /// defaults.
    pub fn load(registry: &Registry) -> Result<ResolvedConfig> {
        let found = find_config()?;
        match found {
            Some((path, raw)) => ResolvedConfig::resolve(raw, Some(&path), registry),
            None => {
                let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                Ok(ResolvedConfig {
                    root,
                    global_excludes: compile_globs(BUILTIN_EXCLUDES, "builtin excludes")?,
                    cop_settings: BTreeMap::new(),
                    params: BTreeMap::new(),
                    configured: BTreeMap::new(),
                })
            }
        }
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

        let mut cop_settings = BTreeMap::new();
        let mut params = BTreeMap::new();
        let mut configured = BTreeMap::new();
        for (key, settings) in raw.cops {
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
            if is_department && settings.exclude.is_some() {
                bail!(
                    "`exclude` is not a valid setting for department `{key}`; use global or per-cop exclusions"
                );
            }
            if let Some(max) = settings.max {
                let cop = match registry.cop_declares_max(&key) {
                    Some(_) => registry.cop_with_max(&key, max),
                    None => None,
                };
                let Some(cop) = cop else {
                    bail!("`max` is not a valid setting for `{key}`");
                };
                if max == 0 {
                    bail!("`max` for `{key}` must be at least 1");
                }
                params.insert(key.clone(), max);
                configured.insert(key.clone(), cop);
            }
            let excludes = compile_globs(settings.exclude.as_deref().unwrap_or_default(), &key)?;
            cop_settings.insert(
                key.clone(),
                CopSettings {
                    enabled: settings.enabled,
                    excludes,
                },
            );
        }

        // Style rules that construct guard/modifier examples use the configured
        // LineLength policy even when --only selects just the Style rule.
        let line_length_enabled = cop_settings
            .get("Layout/LineLength")
            .and_then(|s| s.enabled)
            .or_else(|| cop_settings.get("Layout").and_then(|s| s.enabled))
            .unwrap_or(true);
        let line_length_limit =
            line_length_enabled.then(|| params.get("Layout/LineLength").copied().unwrap_or(120));
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

        let mut global_excludes = compile_globs(BUILTIN_EXCLUDES, "builtin excludes")?;
        global_excludes.extend(compile_globs(&raw.exclude, "exclude")?);

        Ok(ResolvedConfig {
            root,
            global_excludes,
            cop_settings,
            params,
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
        assert_eq!(config.params.get("Layout/LineLength"), Some(&100));
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
    fn line_length_max_must_be_positive() {
        let registry = Registry::new();
        let raw: RawConfig = toml::from_str("[cops.\"Layout/LineLength\"]\nmax = 0").unwrap();
        let err = ResolvedConfig::resolve(raw, None, &registry).unwrap_err();
        assert!(err.to_string().contains("at least 1"), "{err}");
        let raw: Result<RawConfig, _> = toml::from_str("[cops.\"Layout/LineLength\"]\nmax = -1");
        assert!(raw.is_err());
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
