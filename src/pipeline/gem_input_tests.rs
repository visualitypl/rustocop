//! Supplied gem lifecycle controls; the gem cops have their own tests.
use super::*;
use crate::cops::{Cop, NodeContext};
use crate::offense::{GlobalOffense, Severity};

struct OrdinaryMustNotRun;
impl Cop for OrdinaryMustNotRun {
    fn name(&self) -> &'static str {
        "Lint/TestOrdinary"
    }
    fn check_source(&self, _: &SourceFile, _: &mut Vec<Offense>) {
        panic!("source dispatched")
    }
    fn check_parse(&self, _: &ruby_prism::ParseResult<'_>, _: &SourceFile, _: &mut Vec<Offense>) {
        panic!("parse dispatched")
    }
    fn check_global(
        &self,
        _: &ruby_prism::ParseResult<'_>,
        _: &SourceFile,
        _: &mut Vec<GlobalOffense>,
    ) {
        panic!("global source dispatched")
    }
    fn check_node<'pr>(
        &self,
        _: &ruby_prism::Node<'pr>,
        _: &NodeContext<'_, 'pr>,
        _: &SourceFile,
        _: &mut Vec<Offense>,
    ) {
        panic!("AST dispatched")
    }
}

// Pinned GemFilename basename/NO_LOCATION examples (RuboCop MIT,
// Copyright (c) 2012-26 Bozhidar Batsov). This is an unregistered test consumer,
// not an implementation or acceptance of the cop.
struct FilenameConsumer;
impl Cop for FilenameConsumer {
    fn name(&self) -> &'static str {
        "Bundler/GemFilename"
    }
    fn check_filename(&self, path: &Path, out: &mut Vec<GlobalOffense>) {
        let message = match path.file_name().and_then(|s| s.to_str()) {
            Some("gems.rb") => "`gems.rb` file was found but `Gemfile` is required",
            Some("gems.locked") => {
                "Expected a `Gemfile.lock` with `Gemfile` but found `gems.locked` file"
            }
            _ => return,
        };
        out.push(GlobalOffense {
            cop: self.name(),
            severity: Severity::Convention,
            message: format!("{message} (file path: {}).", path.display()),
        });
    }
}

#[test]
fn filename_findings_are_nonfatal_and_location_free() {
    for path in ["gems.rb", "spec/gems.rb", "gems.locked", "spec/gems.locked"] {
        let report = lint_one(
            Path::new(path),
            Path::new(path),
            Some(b"print 1"),
            &[&FilenameConsumer],
            &ResolvedConfig::default(),
        );
        assert!(report.errors.is_empty());
        assert!(report.internal_errors.is_empty());
        assert!(report.result.offenses.is_empty());
        assert_eq!(report.result.global_offenses.len(), 1, "{path}");
        assert_eq!(report.result.global_offenses[0].cop, "Bundler/GemFilename");
        assert!(
            report.result.global_offenses[0]
                .message
                .ends_with(&format!("(file path: {path})."))
        );
        let run = RunReport {
            files_inspected: 1,
            results: vec![report.result],
            ..RunReport::default()
        };
        assert_eq!(run.offense_count(), 1);
        assert_eq!(run.exit_code(), EXIT_OFFENSES);
        assert!(!crate::output::render_text(&run).contains(":1:1:"));
        assert!(!crate::output::render_github(&run).contains(",line="));
    }
    for path in [
        "Gemfile",
        "spec/Gemfile",
        "Gemfile.lock",
        "spec/Gemfile.lock",
        "other.gemfile",
        "gems.locked.rb",
    ] {
        let report = lint_one(
            Path::new(path),
            Path::new(path),
            Some(b"print 1"),
            &[&FilenameConsumer],
            &ResolvedConfig::default(),
        );
        assert!(report.result.global_offenses.is_empty(), "{path}");
    }
}

#[test]
fn opaque_locks_never_parse_or_dispatch_ordinary_cops() {
    for path in ["Gemfile.lock", "nested/gems.locked"] {
        for bytes in [
            &b"\xff\0\n(((("[..],
            b"binding.pry\n",
            b"",
            b"# rubocop:disable all\n",
        ] {
            let report = lint_one(
                Path::new(path),
                Path::new(path),
                Some(bytes),
                &[&OrdinaryMustNotRun],
                &ResolvedConfig::default(),
            );
            assert!(report.errors.is_empty(), "{report:?}");
            assert!(report.internal_errors.is_empty(), "{report:?}");
            assert!(report.result.offenses.is_empty());
        }
    }
}

// Only the empty-AST lifecycle fragment of RequiredRubyVersion, not its detector.
struct EmptyGemspecConsumer;
impl Cop for EmptyGemspecConsumer {
    fn name(&self) -> &'static str {
        "Gemspec/RequiredRubyVersion"
    }
    fn path_policy(&self) -> Option<&crate::cops::policy::PathPolicy> {
        static POLICY: std::sync::LazyLock<crate::cops::policy::PathPolicy> =
            std::sync::LazyLock::new(|| {
                crate::cops::policy::PathPolicy::new(&["**/*.gemspec"], &[]).unwrap()
            });
        Some(&POLICY)
    }
    fn check_global(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        _: &SourceFile,
        out: &mut Vec<GlobalOffense>,
    ) {
        if result
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .is_empty()
        {
            out.push(GlobalOffense {
                cop: self.name(),
                severity: Severity::Convention,
                message: "`required_ruby_version` should be specified.".into(),
            });
        }
    }
}

#[test]
fn valid_empty_and_comment_only_gemspecs_receive_global_lifecycle() {
    for bytes in [
        &b""[..],
        b"# comment\n",
        b"\xef\xbb\xbf# comment\r\n",
        b"__END__\n# rubocop:disable all\n\xff",
    ] {
        let report = lint_one(
            Path::new("bar.gemspec"),
            Path::new("bar.gemspec"),
            Some(bytes),
            &[&EmptyGemspecConsumer],
            &ResolvedConfig::default(),
        );
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.result.global_offenses.len(), 1);
        assert_eq!(
            report.result.global_offenses[0].message,
            "`required_ruby_version` should be specified."
        );
    }
    for path in ["Gemfile", "other.rb", "bar.gemspec"] {
        let report = lint_one(Path::new(path), Path::new(path), Some(b"Gem::Specification.new do |spec|\n  spec.required_ruby_version = \">= 3.4\"\nend\n"), &[&EmptyGemspecConsumer], &ResolvedConfig::default());
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(report.result.global_offenses.is_empty());
    }
    let report = lint_one(
        Path::new("other.rb"),
        Path::new("other.rb"),
        Some(b""),
        &[&EmptyGemspecConsumer],
        &ResolvedConfig::default(),
    );
    assert!(report.result.global_offenses.is_empty());
}

fn selected(
    registry: &Registry,
    path: &str,
    bytes: &[u8],
    config: &ResolvedConfig,
    only: &[&str],
    except: &[&str],
) -> FileReport {
    let only: Vec<_> = only.iter().map(|s| (*s).to_owned()).collect();
    let except: Vec<_> = except.iter().map(|s| (*s).to_owned()).collect();
    let active = CopSelection::new(registry, config, &only, &except);
    lint_one_with_fix(
        Path::new(path),
        Path::new(path),
        Some(bytes),
        &active,
        config,
        false,
    )
}

#[test]
fn global_findings_obey_both_directive_prefixes_without_claiming_line_one() {
    let registry = Registry::from_cops(vec![Box::new(FilenameConsumer)]);
    for prefix in ["rubocop", "rustocop"] {
        for (source, expected) in [
            (
                format!("# {prefix}:disable Bundler/GemFilename\nprint 1\n"),
                0,
            ),
            (format!("# {prefix}:disable Bundler\nprint 1\n"), 0),
            (format!("# {prefix}:disable all\nprint 1\n"), 0),
            (format!("\n# {prefix}:disable Bundler/GemFilename\n"), 1),
            (
                format!("# {prefix}:disable-next Bundler/GemFilename\nprint 1\n"),
                1,
            ),
            (format!("__END__\n# {prefix}:disable all\n"), 1),
        ] {
            let report = selected(
                &registry,
                "gems.rb",
                source.as_bytes(),
                &ResolvedConfig::default(),
                &[],
                &[],
            );
            assert!(report.errors.is_empty(), "{report:?}");
            assert_eq!(report.result.global_offenses.len(), expected, "{source}");
        }
        // Opaque bytes cannot turn into Ruby directives.
        let source = format!("# {prefix}:disable all\n");
        assert_eq!(
            selected(
                &registry,
                "gems.locked",
                source.as_bytes(),
                &ResolvedConfig::default(),
                &[],
                &[]
            )
            .result
            .global_offenses
            .len(),
            1
        );
    }
}

#[test]
fn global_findings_obey_config_cli_and_fixed_path_gates() {
    use crate::config::RawConfig;
    let registry = Registry::from_cops(vec![
        Box::new(FilenameConsumer),
        Box::new(EmptyGemspecConsumer),
    ]);
    for (toml, only, except, expected) in [
        ("", vec![], vec![], 1),
        (
            "[cops.\"Bundler/GemFilename\"]\nenabled = false",
            vec![],
            vec![],
            0,
        ),
        (
            "[cops.\"Bundler/GemFilename\"]\nenabled = false",
            vec!["Bundler/GemFilename"],
            vec![],
            1,
        ),
        ("", vec!["Gemspec"], vec![], 0),
        ("", vec!["Bundler"], vec!["Bundler/GemFilename"], 0),
        (
            "[cops.\"Bundler/GemFilename\"]\nexclude = [\"nested/**\"]",
            vec!["Bundler"],
            vec![],
            0,
        ),
        ("exclude = [\"nested/**\"]", vec!["Bundler"], vec![], 0),
    ] {
        let raw: RawConfig = toml::from_str(toml).unwrap();
        let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
        for path in ["nested/gems.rb", "nested/gems.locked"] {
            let report = selected(&registry, path, b"print 1", &config, &only, &except);
            assert!(report.errors.is_empty(), "{report:?}");
            assert_eq!(
                report.result.global_offenses.len(),
                expected,
                "{toml} {path}"
            );
        }
    }
    // A direct selector cannot bypass RequiredRubyVersion's genuine path gate.
    let report = selected(
        &registry,
        "Gemfile",
        b"",
        &ResolvedConfig::default(),
        &["Gemspec"],
        &[],
    );
    assert!(report.result.global_offenses.is_empty());
}

#[test]
fn selected_filename_callbacks_cannot_mask_ruby_syntax_encoding_or_depth_errors() {
    let registry = Registry::from_cops(vec![
        Box::new(FilenameConsumer),
        Box::new(EmptyGemspecConsumer),
    ]);
    let deep = format!("{}1{}", "[".repeat(100), "]".repeat(100));
    for path in ["gems.rb", "broken.gemspec"] {
        for bytes in [
            &b"def ("[..],
            b"\xff",
            b"# encoding: unknown-encoding\n",
            deep.as_bytes(),
        ] {
            let report = selected(&registry, path, bytes, &ResolvedConfig::default(), &[], &[]);
            assert!(!report.errors.is_empty(), "{path} {bytes:?}: {report:?}");
            assert!(report.result.global_offenses.is_empty());
            assert!(report.result.offenses.is_empty());
            assert!(report.internal_errors.is_empty());
        }
    }
}

struct OptInFilename;
impl Cop for OptInFilename {
    fn name(&self) -> &'static str {
        "Bundler/GemFilename"
    }
    fn default_enabled(&self) -> bool {
        false
    }
    fn check_filename(&self, path: &Path, out: &mut Vec<GlobalOffense>) {
        FilenameConsumer.check_filename(path, out);
    }
}

struct FutureFilename;
impl Cop for FutureFilename {
    fn name(&self) -> &'static str {
        "Bundler/GemFilename"
    }
    fn ruby_versions(&self) -> crate::cops::policy::RubyVersions {
        crate::cops::policy::RubyVersions {
            minimum: Some(crate::cops::policy::RubyVersion(4, 0)),
            maximum: None,
        }
    }
    fn check_filename(&self, _: &Path, _: &mut Vec<GlobalOffense>) {
        panic!("version-inactive filename callback")
    }
}

#[test]
fn disabled_global_cops_need_real_opt_in_and_selectors_cannot_bypass_versions() {
    let registry = Registry::from_cops(vec![Box::new(OptInFilename)]);
    let config = ResolvedConfig::default();
    assert!(
        selected(&registry, "gems.rb", b"", &config, &[], &[])
            .result
            .global_offenses
            .is_empty()
    );
    assert!(
        selected(
            &registry,
            "gems.locked",
            b"# rubocop:enable Bundler/GemFilename\n",
            &config,
            &[],
            &[]
        )
        .result
        .global_offenses
        .is_empty()
    );
    for path in ["gems.rb", "gems.locked"] {
        assert_eq!(
            selected(&registry, path, b"", &config, &["Bundler"], &[])
                .result
                .global_offenses
                .len(),
            1
        );
    }
    for prefix in ["rubocop", "rustocop"] {
        // An enable on line1 closes the initial disabled range inclusively;
        // a global finding still has no enabled suppression anchor in this pin.
        let source = format!("# {prefix}:enable Bundler/GemFilename\nprint 1\n");
        assert!(
            selected(&registry, "gems.rb", source.as_bytes(), &config, &[], &[])
                .result
                .global_offenses
                .is_empty()
        );
        assert!(
            selected(
                &registry,
                "gems.rb",
                source.as_bytes(),
                &config,
                &[],
                &["Bundler"]
            )
            .result
            .global_offenses
            .is_empty()
        );
    }
    let registry = Registry::from_cops(vec![Box::new(FutureFilename)]);
    for path in ["gems.rb", "gems.locked"] {
        let report = selected(
            &registry,
            path,
            b"# rubocop:enable Bundler/GemFilename\n",
            &config,
            &["Bundler"],
            &[],
        );
        assert!(report.internal_errors.is_empty());
        assert!(report.result.global_offenses.is_empty());
    }
}

struct PanickingFilename;
impl Cop for PanickingFilename {
    fn name(&self) -> &'static str {
        "Lint/FailedFilename"
    }
    fn check_filename(&self, _: &Path, out: &mut Vec<GlobalOffense>) {
        out.push(GlobalOffense {
            cop: self.name(),
            severity: Severity::Warning,
            message: "partial evidence".into(),
        });
        panic!("filename panic")
    }
    fn check_source(&self, _: &SourceFile, _: &mut Vec<Offense>) {
        panic!("must sit out rest of file")
    }
}

#[test]
fn filename_panics_are_isolated_and_error_status_wins_over_global_findings() {
    for path in ["gems.rb", "gems.locked"] {
        let report = lint_one(
            Path::new(path),
            Path::new(path),
            Some(b""),
            &[&PanickingFilename, &FilenameConsumer],
            &ResolvedConfig::default(),
        );
        assert_eq!(report.result.global_offenses.len(), 2);
        assert!(report.errors.is_empty());
        assert_eq!(report.internal_errors.len(), 1);
        assert_eq!(report.internal_errors[0].cop, Some("Lint/FailedFilename"));
        assert_eq!(report.internal_errors[0].position, None);
        assert_eq!(report.internal_errors[0].panic.message, "filename panic");
        let run = RunReport {
            results: vec![report.result],
            internal_errors: report.internal_errors,
            ..RunReport::default()
        };
        assert_eq!(run.exit_code(), EXIT_ERRORS);
    }
}

#[test]
fn global_suppression_is_not_misreported_or_corrected_as_redundant() {
    let registry = Registry::from_cops(vec![
        Box::new(FilenameConsumer),
        Box::new(crate::cops::lint::RedundantCopDisableDirective),
    ]);
    for prefix in ["rubocop", "rustocop"] {
        for (source, redundant) in [
            (format!("# {prefix}:disable Bundler/GemFilename\n"), false),
            (format!("\n# {prefix}:disable Bundler/GemFilename\n"), true),
        ] {
            let report = selected(
                &registry,
                "gems.rb",
                source.as_bytes(),
                &ResolvedConfig::default(),
                &[],
                &[],
            );
            assert_eq!(
                report
                    .result
                    .offenses
                    .iter()
                    .any(|o| o.cop == "Lint/RedundantCopDisableDirective"),
                redundant,
                "{report:?}"
            );
        }
    }
}

#[test]
fn run_keeps_global_only_results_and_fix_revalidation_retains_them() {
    let directory = tempfile::tempdir().unwrap();
    let ruby = directory.path().join("gems.rb");
    let lock = directory.path().join("gems.locked");
    fs::write(&ruby, b"value=7\nputs value\n").unwrap();
    fs::write(&lock, b"\xff\0(").unwrap();
    let registry = Registry::from_cops(vec![
        Box::new(FilenameConsumer),
        Box::new(crate::cops::layout::SpaceAroundOperators),
    ]);
    let config = ResolvedConfig {
        root: directory.path().to_path_buf(),
        ..ResolvedConfig::default()
    };
    let options = RunOptions {
        fix: true,
        ..RunOptions::default()
    };
    let run = run(&[ruby.clone(), lock.clone()], &options, &config, &registry).unwrap();
    assert_eq!(run.files_inspected, 2);
    assert_eq!(
        run.results.len(),
        2,
        "global-only files must not be dropped"
    );
    assert_eq!(run.offense_count(), 2);
    assert_eq!(run.corrections_applied, 1);
    assert_eq!(run.exit_code(), EXIT_OFFENSES);
    assert_eq!(fs::read(&ruby).unwrap(), b"value = 7\nputs value\n");
    assert_eq!(fs::read(&lock).unwrap(), b"\xff\0(");
    assert!(
        run.results
            .iter()
            .all(|r| r.offenses.is_empty() && r.global_offenses.len() == 1)
    );
    assert_eq!(
        crate::output::render_text(&run),
        "gems.locked: C: Expected a `Gemfile.lock` with `Gemfile` but found `gems.locked` file (file path: gems.locked). (Bundler/GemFilename)\ngems.rb: C: `gems.rb` file was found but `Gemfile` is required (file path: gems.rb). (Bundler/GemFilename)\n\n2 files inspected, 1 correction applied, 2 offenses remaining\n"
    );
}

#[test]
fn unreadable_filename_only_input_keeps_fatal_io_error_and_no_filename_findings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gems.locked");
    let report = lint_one(
        &path,
        Path::new("gems.locked"),
        None,
        &[&FilenameConsumer],
        &ResolvedConfig::default(),
    );
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].position, None);
    assert!(report.errors[0].message.starts_with("could not read file:"));
    assert!(report.result.global_offenses.is_empty());
    assert!(report.internal_errors.is_empty());
}

#[test]
fn atomic_writer_itself_refuses_opaque_lock_destinations() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["Gemfile.lock", "gems.locked"] {
        let path = directory.path().join(name);
        fs::write(&path, b"x=1\n").unwrap();
        let old = fs::metadata(&path).unwrap();
        let error = write_corrected(&path, b"x=1\n", b"x = 1\n").unwrap_err();
        assert!(error.to_string().contains("opaque gem lock"));
        assert_eq!(fs::read(&path).unwrap(), b"x=1\n");
        let current = fs::metadata(&path).unwrap();
        assert_eq!(old.modified().unwrap(), current.modified().unwrap());
        assert_eq!(old.permissions(), current.permissions());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(old.ino(), current.ino());
        }
    }
}

#[test]
fn actual_registry_and_config_admit_no_future_names() {
    let registry = Registry::new();
    for name in ["Bundler/Future", "Gemspec/Future"] {
        assert!(!registry.has_cop(name));
        for except in [false, true] {
            let mut options = RunOptions::default();
            if except {
                options.except.push(name.into());
            } else {
                options.only.push(name.into());
            }
            assert!(validate_selections(&options, &registry).is_err());
        }
        let raw = toml::from_str(&format!("[cops.\"{name}\"]\nenabled = true\n")).unwrap();
        assert!(ResolvedConfig::resolve(raw, None, &registry).is_err());
    }
}

#[test]
fn real_ruby_gem_inputs_keep_mandatory_fatal_syntax_gate() {
    for path in [
        "Gemfile",
        "ci.gemfile",
        "gems.rb",
        "pkg.gemspec",
        "Gemfile.lock.rb",
        "other.lock",
    ] {
        let report = lint_one(
            Path::new(path),
            Path::new(path),
            Some(b"def ("),
            &[&OrdinaryMustNotRun],
            &ResolvedConfig::default(),
        );
        assert!(!report.errors.is_empty(), "{path}: {report:?}");
        assert!(report.internal_errors.is_empty());
        assert!(report.result.offenses.is_empty());
    }
}
