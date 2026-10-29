//! Fixed Style policy and landed Naming policy.
//! Metadata is not registration: only real cops in `Registry` are selectable.

/// Explicit upstream `Enabled: false`, plus the existing frozen-comment exception.
/// Pending is enabled; absence from this list does not imply implementation.
const OPT_IN: &[&str] = &[
    "Lint/ConstantResolution",
    "Naming/InclusiveLanguage",
    "Style/ArrayCoercion",
    "Style/ArrayFirstLast",
    "Style/AsciiComments",
    "Style/AutoResourceCleanup",
    "Style/ClassMethodsDefinitions",
    "Style/CollectionMethods",
    "Style/ConstantVisibility",
    "Style/Copyright",
    "Style/DateTime",
    "Style/DisableCopsWithinSourceCodeDirective",
    "Style/DocumentationMethod",
    "Style/FrozenStringLiteralComment",
    "Style/HashLookupMethod",
    "Style/ImplicitRuntimeError",
    "Style/InlineComment",
    "Style/InvertibleUnlessCondition",
    "Style/IpAddresses",
    "Style/MethodCallWithArgsParentheses",
    "Style/MethodCalledOnDoEndBlock",
    "Style/MissingElse",
    "Style/MultilineMethodSignature",
    "Style/OptionHash",
    "Style/RedundantStructKeywordInit",
    "Style/RequireOrder",
    "Style/ReturnNil",
    "Style/Send",
    "Style/SingleLineBlockParams",
    "Style/StaticClass",
    "Style/StringHashKeys",
    "Style/StringMethods",
    "Style/TopLevelMethodDefinition",
    "Style/TrailingCommaInBlockArgs",
    "Style/UnlessLogicalOperators",
    "Style/YodaExpression",
];

pub(crate) fn default_enabled(name: &str) -> bool {
    !OPT_IN.contains(&name)
}

/// Ruby applicability is fixed, not a parser setting or public target knob.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RubyVersion(pub u8, pub u8);

pub const FIXED_RUBY_VERSION: RubyVersion = RubyVersion(3, 4);

/// Inclusive upstream min/max bounds. An absent bound is unbounded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RubyVersions {
    pub minimum: Option<RubyVersion>,
    pub maximum: Option<RubyVersion>,
}

impl RubyVersions {
    pub(crate) fn supports(self, version: RubyVersion) -> bool {
        self.minimum.is_none_or(|min| version >= min)
            && self.maximum.is_none_or(|max| version <= max)
    }
}

pub(crate) fn ruby_versions(name: &str) -> RubyVersions {
    let minimum = match name {
        "Style/Dir" | "Style/SymbolArray" => Some(RubyVersion(2, 0)),
        "Style/FrozenStringLiteralComment" | "Style/HashFetchChain" | "Style/SafeNavigation" => {
            Some(RubyVersion(2, 3))
        }
        "Style/CollectionCompact"
        | "Style/ComparableClamp"
        | "Style/DirEmpty"
        | "Style/FileEmpty"
        | "Style/HashTransformValues"
        | "Style/UnpackFirst" => Some(RubyVersion(2, 4)),
        "Style/BitwisePredicate" | "Style/HashSlice" | "Style/HashTransformKeys" => {
            Some(RubyVersion(2, 5))
        }
        "Lint/ErbNewArguments"
        | "Style/MapToHash"
        | "Style/ObjectThen"
        | "Style/ReduceToHash"
        | "Style/SlicingWithRange" => Some(RubyVersion(2, 6)),
        "Lint/DuplicateMatchPattern"
        | "Lint/EmptyInPattern"
        | "Style/ArgumentsForwarding"
        | "Style/InPatternThen"
        | "Style/MultilineInPatternThen"
        | "Style/NumberedParameters"
        | "Style/NumberedParametersLimit"
        | "Style/TallyMethod" => Some(RubyVersion(2, 7)),
        "Style/AmbiguousEndlessMethodDefinition"
        | "Style/EndlessMethod"
        | "Style/HashExcept"
        | "Style/RedundantInterpolationUnfreeze" => Some(RubyVersion(3, 0)),
        "Naming/BlockForwarding" | "Style/ArrayIntersect" | "Style/NestedFileDirname" => {
            Some(RubyVersion(3, 1))
        }
        "Style/DataInheritance" | "Style/RedundantStructKeywordInit" => Some(RubyVersion(3, 2)),
        "Style/ItBlockParameter" => Some(RubyVersion(3, 4)),
        "Style/ReverseFind" => Some(RubyVersion(4, 0)),
        _ => None,
    };
    // YAML.load became safe by default in Ruby 3.1 (Psych 4). Keep the
    // registered legacy rule genuinely inactive, including explicit selection.
    let maximum = match name {
        "Security/YAMLLoad" => Some(RubyVersion(3, 0)),
        "Lint/ItWithoutArgumentsInBlock" => Some(RubyVersion(3, 3)),
        "Lint/UselessElseWithoutRescue" => Some(RubyVersion(2, 5)),
        "Lint/NonDeterministicRequireOrder" => Some(RubyVersion(2, 7)),
        _ => None,
    };
    RubyVersions { minimum, maximum }
}

/// Pinned, cop-level path eligibility; not user-configurable Include settings.
/// Compiled once and applied before dispatch, including explicit files/stdin.
#[derive(Debug)]
pub struct PathPolicy {
    includes: Vec<globset::GlobMatcher>,
    excludes: Vec<globset::GlobMatcher>,
}

impl PathPolicy {
    pub fn new(includes: &[&str], excludes: &[&str]) -> anyhow::Result<Self> {
        Ok(Self {
            includes: crate::config::compile_globs(includes, "pinned cop Include")?,
            excludes: crate::config::compile_globs(excludes, "pinned cop Exclude")?,
        })
    }

    pub(crate) fn allows(&self, relative: &std::path::Path) -> bool {
        (self.includes.is_empty() || crate::config::matches_any(&self.includes, relative))
            && !crate::config::matches_any(&self.excludes, relative)
    }
}

const PATHS: &[(&str, &[&str], &[&str])] = &[
    ("Naming/FileName", &[], &["Rakefile.rb"]),
    ("Naming/PredicatePrefix", &[], &["spec/**/*"]),
    ("Style/Documentation", &[], &["spec/**/*", "test/**/*"]),
    (
        "Style/DocumentationMethod",
        &[],
        &["spec/**/*", "test/**/*"],
    ),
    ("Style/FrozenStringLiteralComment", &[], &["**/*.arb"]),
    (
        "Style/IpAddresses",
        &[],
        &["**/*.gemfile", "**/Gemfile", "**/gems.rb", "**/*.gemspec"],
    ),
    ("Style/NumericPredicate", &[], &["spec/**/*"]),
    ("Style/OneClassPerFile", &[], &["spec/**/*", "test/**/*"]),
];

pub(crate) fn path_policy(name: &str) -> Option<&'static PathPolicy> {
    use std::{collections::BTreeMap, sync::LazyLock};
    static COMPILED: LazyLock<BTreeMap<&str, PathPolicy>> = LazyLock::new(|| {
        PATHS
            .iter()
            .map(|&(name, includes, excludes)| {
                (
                    name,
                    PathPolicy::new(includes, excludes).expect("valid pinned path policy"),
                )
            })
            .collect()
    });
    COMPILED.get(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RawConfig, ResolvedConfig};
    use crate::cops::{Cop, Registry, select_active};
    use serde_json::Value;

    fn inventory() -> Value {
        serde_json::from_str(include_str!("../../tools/oracle/style_reference.json")).unwrap()
    }

    #[test]
    fn all_299_enable_states_follow_explicit_false_and_frozen_exception() {
        let inventory = inventory();
        let records = inventory["cops"].as_array().unwrap();
        assert_eq!(records.len(), 299);
        let mut enabled = 0;
        let mut pending = 0;
        for record in records {
            let name = record["name"].as_str().unwrap();
            let expected =
                record["upstream_enabled"] != false && name != "Style/FrozenStringLiteralComment";
            assert_eq!(default_enabled(name), expected, "{name}");
            enabled += usize::from(expected);
            pending += usize::from(record["upstream_enabled"] == "pending");
        }
        assert_eq!((enabled, records.len() - enabled), (265, 34));
        assert!(pending > 0);
        for cop in Registry::new().cops() {
            assert_eq!(
                cop.default_enabled(),
                default_enabled(cop.name()),
                "{cop:?}"
            );
        }
    }

    /// Exercise real detection under future policy names, never production stubs.
    struct PolicyDebugger(&'static str);
    impl Cop for PolicyDebugger {
        fn name(&self) -> &'static str {
            self.0
        }
        fn ruby_versions(&self) -> RubyVersions {
            if self.0 == "Lint/LegacyProbe" {
                RubyVersions {
                    minimum: None,
                    maximum: Some(RubyVersion(3, 3)),
                }
            } else {
                ruby_versions(self.name())
            }
        }
        fn path_policy(&self) -> Option<&PathPolicy> {
            if self.0 == "Lint/IncludedProbe" {
                static POLICY: std::sync::LazyLock<PathPolicy> = std::sync::LazyLock::new(|| {
                    PathPolicy::new(&["lib/**/*.rb"], &["lib/skipped.rb"]).unwrap()
                });
                Some(&POLICY)
            } else {
                path_policy(self.name())
            }
        }
        fn check_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            cx: &crate::cops::NodeContext<'_, 'pr>,
            file: &crate::source::SourceFile,
            out: &mut Vec<crate::offense::Offense>,
        ) {
            let start = out.len();
            crate::cops::lint::Debugger.check_node(node, cx, file, out);
            for offense in &mut out[start..] {
                offense.cop = self.name();
            }
        }
    }

    #[test]
    fn fixed_ruby_gates_cannot_be_overridden_by_only_or_config() {
        use crate::cops::select_active_for_version;
        let registry = Registry::from_cops(vec![
            Box::new(PolicyDebugger("Style/ReverseFind")),
            Box::new(PolicyDebugger("Style/ItBlockParameter")),
            Box::new(PolicyDebugger("Lint/LegacyProbe")),
        ]);
        let raw = toml::from_str("[cops.\"Style/ReverseFind\"]\nenabled = true\n").unwrap();
        let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
        let only = vec!["Style/ReverseFind".into()];
        assert_eq!(FIXED_RUBY_VERSION, RubyVersion(3, 4));
        assert!(select_active(&registry, &config, &only, &[]).is_empty());
        let defaults = select_active(&registry, &config, &[], &[]);
        assert_eq!(
            defaults.iter().map(|c| c.name()).collect::<Vec<_>>(),
            ["Style/ItBlockParameter"]
        );
        let applicable =
            select_active_for_version(&registry, &config, &only, &[], RubyVersion(4, 0));
        // This is a gate test using Debugger's real AST matcher, NOT ReverseFind
        // detection evidence. Its feature task supplies that matcher and tests.
        let offenses = crate::pipeline::lint_str_with(&applicable, "binding.pry\n");
        assert_eq!(offenses.len(), 1);
        assert_eq!(offenses[0].cop, "Style/ReverseFind");
        assert_eq!((offenses[0].line, offenses[0].col), (1, 1));
        assert!(offenses[0].message.contains("debugger entry point"));
        assert!(crate::pipeline::lint_str_with(&applicable, "ordinary_call\n").is_empty());
        let old = select_active_for_version(&registry, &config, &[], &[], RubyVersion(3, 3));
        assert_eq!(
            old.iter().map(|c| c.name()).collect::<Vec<_>>(),
            ["Lint/LegacyProbe"]
        );
        assert!(
            select_active_for_version(&registry, &config, &only, &only, RubyVersion(4, 0))
                .is_empty()
        );
    }

    #[test]
    fn landed_legacy_lint_bounds_are_real_inclusive_and_unforceable() {
        let registry = Registry::new();
        for (name, max) in [
            ("Lint/ItWithoutArgumentsInBlock", RubyVersion(3, 3)),
            ("Lint/UselessElseWithoutRescue", RubyVersion(2, 5)),
            ("Lint/NonDeterministicRequireOrder", RubyVersion(2, 7)),
        ] {
            assert!(registry.has_cop(name));
            let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
            assert_eq!(
                cop.ruby_versions(),
                RubyVersions {
                    minimum: None,
                    maximum: Some(max)
                }
            );
            assert!(cop.default_enabled());
            assert!(!cop.safe_fixes());
            assert!(cop.path_policy().is_none());
            for settings in [
                format!("[cops.\"{name}\"]\nenabled = true\n"),
                format!("[cops.Lint]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
                format!("[cops.Lint]\nenabled = true\n[cops.\"{name}\"]\nenabled = false\n"),
            ] {
                let config =
                    ResolvedConfig::resolve(toml::from_str(&settings).unwrap(), None, &registry)
                        .unwrap();
                let only = vec![name.into()];
                assert!(select_active(&registry, &config, &only, &[]).is_empty());
                assert!(
                    !select_active(&registry, &config, &[], &[])
                        .iter()
                        .any(|c| c.name() == name)
                );
                let legacy =
                    crate::cops::select_active_for_version(&registry, &config, &only, &[], max);
                assert_eq!(legacy.len(), 1);
                assert_eq!(legacy[0].name(), name);
                let after_max = RubyVersion(max.0, max.1 + 1);
                assert!(
                    crate::cops::select_active_for_version(
                        &registry,
                        &config,
                        &only,
                        &[],
                        after_max
                    )
                    .is_empty()
                );
                let source = if name == "Lint/ItWithoutArgumentsInBlock" {
                    // A real CallNode even in the 3.4 AST, not an implicit read.
                    "items.each { def helper; it; end }"
                } else if name == "Lint/NonDeterministicRequireOrder" {
                    "Dir['*.rb'].each { |file| require file }"
                } else {
                    // Explicit direct-only recovered-tree seam: production
                    // rejects this before cop dispatch, regardless of version.
                    "begin\n work\nelse\n other\nend"
                };
                let parsed = crate::parser::parse(source.as_bytes());
                let file =
                    crate::source::SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
                let mut out = vec![];
                assert!(crate::cops::run_cops(&legacy, &parsed, &file, &mut out).is_empty());
                assert_eq!(out.len(), 1);
                assert_eq!(out[0].cop, name);
                assert!(out[0].correction.is_none());
                assert!(
                    crate::cops::select_active_for_version(&registry, &config, &only, &only, max)
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn min_and_max_are_inclusive_and_versions_are_not_decimal_numbers() {
        let versions = RubyVersions {
            minimum: Some(RubyVersion(3, 4)),
            maximum: Some(RubyVersion(3, 10)),
        };
        for (version, expected) in [
            (RubyVersion(3, 3), false),
            (RubyVersion(3, 4), true),
            (RubyVersion(3, 10), true),
            (RubyVersion(3, 11), false),
            (RubyVersion(4, 0), false),
        ] {
            assert_eq!(versions.supports(version), expected);
        }
    }

    #[test]
    fn pinned_paths_and_versions_match_every_inventory_record() {
        for record in inventory()["cops"].as_array().unwrap() {
            let name = record["name"].as_str().unwrap();
            let parse = |key: &str| {
                record[key].as_str().map(|v| {
                    let (major, minor) = v.split_once('.').unwrap();
                    RubyVersion(major.parse().unwrap(), minor.parse().unwrap())
                })
            };
            assert_eq!(
                ruby_versions(name),
                RubyVersions {
                    minimum: parse("minimum_target_ruby"),
                    maximum: parse("maximum_target_ruby")
                },
                "{name}"
            );
            let paths = PATHS.iter().find(|p| p.0 == name);
            for (key, actual) in [
                ("Include", paths.map_or(&[][..], |p| p.1)),
                ("Exclude", paths.map_or(&[][..], |p| p.2)),
            ] {
                let expected: Vec<_> = record["upstream_defaults"][key]
                    .as_array()
                    .map(|a| a.iter().map(|v| v.as_str().unwrap()).collect())
                    .unwrap_or_default();
                assert_eq!(actual, expected, "{name} {key}");
            }
        }
    }

    #[test]
    fn pinned_paths_and_include_fixture_apply_before_dispatch_in_the_pipeline() {
        use std::path::Path;
        let registry = Registry::from_cops(vec![
            Box::new(PolicyDebugger("Style/Documentation")),
            Box::new(PolicyDebugger("Lint/IncludedProbe")),
        ]);
        let config = ResolvedConfig::resolve(
            RawConfig::default(),
            Some(Path::new("/project/rustocop.toml")),
            &registry,
        )
        .unwrap();
        let active = select_active(&registry, &config, &[], &[]);
        for (path, expected) in [
            (
                "lib/a.rb",
                vec!["Lint/IncludedProbe", "Style/Documentation"],
            ),
            (
                "lib/deep/a.rb",
                vec!["Lint/IncludedProbe", "Style/Documentation"],
            ),
            ("lib/skipped.rb", vec!["Style/Documentation"]),
            ("spec/a.rb", vec![]),
            ("test/deep/a.rb", vec![]),
            ("./spec/a.rb", vec![]),
            ("/project/spec/a.rb", vec![]),
            (
                "/project/lib/a.rb",
                vec!["Lint/IncludedProbe", "Style/Documentation"],
            ),
            ("specification/a.rb", vec!["Style/Documentation"]),
        ] {
            let report = crate::pipeline::lint_one(
                Path::new(path),
                Path::new(path),
                Some(b"binding.pry\n"),
                &active,
                &config,
            );
            assert!(report.errors.is_empty());
            let actual: Vec<_> = report.result.offenses.iter().map(|o| o.cop).collect();
            assert_eq!(actual, expected, "{path}");
        }
    }

    #[test]
    fn path_policies_are_additive_to_user_exclusions_even_when_force_selected() {
        use std::path::Path;
        let registry = Registry::from_cops(vec![Box::new(PolicyDebugger("Style/Documentation"))]);
        let raw = toml::from_str("exclude = [\"lib/global.rb\"]\n[cops.\"Style/Documentation\"]\nexclude = [\"lib/local.rb\"]\n").unwrap();
        let config =
            ResolvedConfig::resolve(raw, Some(Path::new("/project/rustocop.toml")), &registry)
                .unwrap();
        let active = select_active(&registry, &config, &["Style".into()], &[]);
        for path in [
            "spec/a.rb",
            "test/a.rb",
            "lib/global.rb",
            "lib/local.rb",
            "/project/lib/local.rb",
            "/project/lib/global.rb",
        ] {
            let report = crate::pipeline::lint_one(
                Path::new(path),
                Path::new(path),
                Some(b"binding.pry\n"),
                &active,
                &config,
            );
            assert!(report.result.offenses.is_empty(), "{path}");
        }
        let report = crate::pipeline::lint_one(
            Path::new("lib/a.rb"),
            Path::new("lib/a.rb"),
            Some(b"binding.pry\n"),
            &active,
            &config,
        );
        assert_eq!(report.result.offenses.len(), 1);
    }

    /// Real safe matcher with a pinned path policy, only in this test registry.
    /// FrozenStringLiteralComment itself intentionally has no corrections.
    #[cfg(unix)]
    struct PathFilteredSignal;
    #[cfg(unix)]
    impl Cop for PathFilteredSignal {
        fn name(&self) -> &'static str {
            "Style/SignalException"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn path_policy(&self) -> Option<&PathPolicy> {
            path_policy("Style/FrozenStringLiteralComment")
        }
        fn check_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            cx: &crate::cops::NodeContext<'_, 'pr>,
            file: &crate::source::SourceFile,
            out: &mut Vec<crate::offense::Offense>,
        ) {
            assert!(!file.path.ends_with("a.arb"), "excluded cop was dispatched");
            crate::cops::style::SignalException.check_node(node, cx, file, out);
        }
    }

    #[cfg(unix)]
    #[test]
    fn pinned_paths_filter_fixes_and_alias_relint_without_changing_write_identity() {
        use crate::pipeline::{RunOptions, run};
        use std::{fs, os::unix::fs::symlink};
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("z.rb");
        let alias = dir.path().join("a.arb");
        let original = b"fail 'oops'\nbinding.pry\n";
        fs::write(&target, original).unwrap();
        symlink("z.rb", &alias).unwrap();
        let registry = Registry::from_cops(vec![
            Box::new(PathFilteredSignal),
            Box::new(crate::cops::style::StringLiterals),
            Box::new(crate::cops::lint::Debugger),
        ]);
        let raw = toml::from_str(
            "[cops.\"Style/StringLiterals\"]\nexclude = [\"z.rb\"]\n\
             [cops.\"Lint/Debugger\"]\nexclude = [\"a.arb\"]\n",
        )
        .unwrap();
        let config =
            ResolvedConfig::resolve(raw, Some(&dir.path().join("rustocop.toml")), &registry)
                .unwrap();
        let options = RunOptions {
            fix: true,
            ..RunOptions::default()
        };

        // The excluded pinned matcher may not propose a fix even on an
        // explicitly selected file. The other cop can still correct this alias.
        let only_signal = RunOptions {
            only: vec!["Style/SignalException".into()],
            fix: true,
            ..RunOptions::default()
        };
        let report = run(
            std::slice::from_ref(&alias),
            &only_signal,
            &config,
            &registry,
        )
        .unwrap();
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.corrections_applied, 0);
        assert_eq!(fs::read(&target).unwrap(), original);

        // Neither path alone permits both fixes. Coordinate both aliases once,
        // including when the pinned-excluded alias is the first write path.
        let report = run(
            &[alias.clone(), target.clone()],
            &options,
            &config,
            &registry,
        )
        .unwrap();
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.corrections_applied, 2);
        assert_eq!(fs::read(&target).unwrap(), b"raise \"oops\"\nbinding.pry\n");
        assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].path, std::path::Path::new("z.rb"));
        assert_eq!(report.results[0].offenses.len(), 1);
        assert_eq!(report.results[0].offenses[0].cop, "Lint/Debugger");
        let stable = run(&[target, alias], &options, &config, &registry).unwrap();
        assert!(!stable.has_errors(), "{stable:?}");
        assert_eq!(stable.corrections_applied, 0);
    }

    #[test]
    fn opt_in_selection_uses_existing_precedence_without_production_stubs() {
        let registry = Registry::from_cops(vec![
            Box::new(crate::cops::style::ArrayCoercion),
            Box::new(crate::cops::style::FrozenStringLiteralComment),
        ]);
        for name in ["Style/ArrayCoercion", "Style/FrozenStringLiteralComment"] {
            let config = ResolvedConfig::default();
            assert!(select_active(&registry, &config, &[], &[]).is_empty());
            let only = vec![name.to_owned()];
            assert_eq!(
                select_active(&registry, &config, &only, &[])[0].name(),
                name
            );
            assert!(select_active(&registry, &config, &only, &only).is_empty());
            let raw: RawConfig = toml::from_str(&format!(
                "[cops.Style]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"
            ))
            .unwrap();
            let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
            assert_eq!(select_active(&registry, &config, &[], &[])[0].name(), name);
        }
        assert!(Registry::new().has_cop("Style/ArrayCoercion"));
    }
}
