//! Current project safety proofs, distinct from upstream class flags.
//! Canonical cases adapt the attributed RuboCop/feature tests (MIT,
//! Copyright (c) 2012-26 Bozhidar Batsov). Reference fixtures are never executed.
use rustocop::{
    config::ResolvedConfig,
    cops::Registry,
    pipeline::{self, RunOptions},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::{Command, Output},
};

use crate::support::deprecated_constructor_native as native;
use crate::support::stdin;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Canonical {
    cop: String,
    before: String,
    after: String,
    held: Option<String>,
    boundary: String,
}
fn cases() -> Vec<Canonical> {
    serde_json::from_str(include_str!("fixtures/lint_integration_corrections.json")).unwrap()
}
fn selection(c: &Canonical) -> String {
    if c.cop == "Lint/RedundantCopDisableDirective" {
        format!("{},Lint/Debugger", c.cop)
    } else {
        c.cop.clone()
    }
}
fn inventory() -> Value {
    serde_json::from_str(include_str!("../tools/oracle/lint_reference.json")).unwrap()
}
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn ordinary(out: &Output) {
    assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
}
fn unchanged(root: &Path, source: &str, args: &[&str]) -> Output {
    let path = root.join("test.rb");
    fs::write(&path, source).unwrap();
    let metadata = fs::metadata(&path).unwrap();
    let out = cli(root, args);
    assert_eq!(
        fs::read(&path).unwrap(),
        source.as_bytes(),
        "{args:?}: {out:?}"
    );
    let after = fs::metadata(&path).unwrap();
    assert_eq!(
        metadata.modified().unwrap(),
        after.modified().unwrap(),
        "{out:?}"
    );
    assert_eq!(metadata.permissions(), after.permissions(), "{out:?}");
    out
}

#[test]
fn all160_defaults_versions_severity_ownership_and_exact_current520_projection() {
    let data = inventory();
    let registry = Registry::new();
    let actual: BTreeSet<_> = registry.cops().iter().map(|c| c.name()).collect();
    let mut expected: BTreeSet<_> = data["target_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert_eq!(expected.len(), 506);
    expected.extend([
        "Bundler/GemFilename",
        "Gemspec/AttributeAssignment",
        "Gemspec/DuplicatedAssignment",
        "Metrics/ParameterLists",
        "Layout/AccessModifierIndentation",
        "Layout/AssignmentIndentation",
        "Layout/ArgumentAlignment",
        "Layout/ArrayAlignment",
        "Layout/HashAlignment",
        "Layout/ParameterAlignment",
        "Layout/FirstParameterIndentation",
        "Layout/BeginEndAlignment",
        "Layout/DefEndAlignment",
        "Layout/BlockAlignment",
        "Layout/ClosingParenthesisIndentation",
        "Layout/FirstArgumentIndentation",
        "Layout/FirstArrayElementIndentation",
        "Layout/FirstHashElementIndentation",
        "Layout/CaseIndentation",
        "Layout/ElseAlignment",
        "Layout/RescueEnsureAlignment",
        "Layout/HeredocIndentation",
        "Layout/ClosingHeredocIndentation",
        "Layout/CommentIndentation",
        "Layout/InitialIndentation",
        "Layout/HeredocArgumentClosingParenthesis",
        "Layout/IndentationConsistency",
        "Layout/MultilineMethodCallIndentation",
        "Layout/MultilineOperationIndentation",
        "Layout/DotPosition",
        "Layout/ConditionPosition",
        "Layout/MultilineAssignmentLayout",
        "Layout/SpaceBeforeComment",
        "Layout/LeadingCommentSpace",
        "Layout/EmptyComment",
        "Layout/LeadingEmptyLines",
        "Layout/EndOfLine",
        "Layout/EmptyLineAfterMagicComment",
        "Layout/EmptyLineAfterGuardClause",
        "Layout/EmptyLineAfterMultilineCondition",
        "Layout/EmptyLinesAfterModuleInclusion",
        "Layout/EmptyLinesAroundAttributeAccessor",
        "Layout/EmptyLinesAroundAccessModifier",
        "Layout/EmptyLinesAroundBeginBody",
        "Layout/EmptyLinesAroundBlockBody",
        "Layout/EmptyLinesAroundExceptionHandlingKeywords",
        "Layout/EmptyLinesAroundArguments",
        "Layout/ExtraSpacing",
        "Layout/SpaceBeforeFirstArg",
        "Layout/SpaceBeforeSemicolon",
        "Layout/FirstArrayElementLineBreak",
        "Layout/FirstHashElementLineBreak",
        "Layout/FirstMethodArgumentLineBreak",
        "Layout/FirstMethodParameterLineBreak",
        "Layout/MultilineArrayLineBreaks",
        "Layout/MultilineHashKeyLineBreaks",
        "Layout/MultilineMethodArgumentLineBreaks",
        "Layout/MultilineMethodParameterLineBreaks",
        "Layout/MultilineArrayBraceLayout",
        "Layout/MultilineHashBraceLayout",
        "Layout/MultilineMethodCallBraceLayout",
        "Layout/MultilineMethodDefinitionBraceLayout",
        "Layout/MultilineBlockLayout",
        "Layout/BlockEndNewline",
        "Layout/SingleLineBlockChain",
        "Layout/RedundantLineBreak",
        "Layout/LineContinuationLeadingSpace",
        "Layout/LineContinuationSpacing",
        "Layout/LineEndStringConcatenationIndentation",
        "Layout/SpaceAfterColon",
        "Layout/SpaceAfterMethodName",
        "Layout/SpaceAfterNot",
        "Layout/SpaceAroundKeyword",
        "Layout/SpaceAroundMethodCallOperator",
        "Layout/SpaceAroundBlockParameters",
        "Layout/SpaceInLambdaLiteral",
        "Layout/SpaceBeforeBlockBraces",
        "Layout/SpaceBeforeBrackets",
        "Layout/SpaceInsideArrayPercentLiteral",
        "Layout/SpaceInsidePercentLiteralDelimiters",
        "Layout/SpaceInsideRangeLiteral",
        "Layout/SpaceInsideStringInterpolation",
        "Layout/ClassStructure",
        "Metrics/MethodLength",
        "Metrics/BlockLength",
        "Metrics/ClassLength",
        "Metrics/ModuleLength",
        "Metrics/BlockNesting",
        "Metrics/CollectionLiteralLength",
        "Metrics/CyclomaticComplexity",
        "Metrics/PerceivedComplexity",
        "Metrics/AbcSize",
        "Migration/DepartmentName",
        "Bundler/DuplicatedGem",
        "Bundler/DuplicatedGroup",
        "Bundler/GemComment",
        "Bundler/GemVersion",
        "Bundler/InsecureProtocolSource",
        "Bundler/OrderedGems",
        "Gemspec/OrderedDependencies",
        "Gemspec/AddRuntimeDependency",
        "Gemspec/DependencyVersion",
        "Gemspec/DevelopmentDependencies",
        "Gemspec/DeprecatedAttributeAssignment",
        "Gemspec/RequireMFA",
        "Gemspec/RequiredRubyVersion",
        "Gemspec/RubyVersionGlobalsUsage",
    ]);
    assert_eq!(actual, expected);
    assert_eq!(registry.cops().len(), actual.len());
    let records = data["cops"].as_array().unwrap();
    assert_eq!(records.len(), 160);
    for r in records {
        let name = r["name"].as_str().unwrap();
        let c = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert_eq!(
            c.default_enabled(),
            r["default_enabled"].as_bool().unwrap(),
            "{name}"
        );
        assert_eq!(format!("{:?}", c.severity()), "Error", "{name}");
        let bounds = c.ruby_versions();
        for (actual, key) in [
            (bounds.minimum, "minimum_ruby"),
            (bounds.maximum, "maximum_ruby"),
        ] {
            let actual = actual.map(|v| f64::from(v.0) + f64::from(v.1) / 10.0);
            assert_eq!(actual, r[key].as_f64(), "{name}/{key}");
        }
    }
    assert_eq!(env!("CARGO_PKG_VERSION"), "1.0.0");
}

#[test]
fn exactly39_real_canonical_cases_are_current_obligations_not_class_flags() {
    let data = inventory();
    let expected: BTreeSet<_> = data["new_required_source_corrections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .filter(|n| *n != "Lint/TopLevelReturnWithArgument")
        .collect();
    let cases = cases();
    let actual: BTreeSet<_> = cases.iter().map(|c| c.cop.as_str()).collect();
    assert_eq!(cases.len(), 39);
    assert_eq!(actual, expected);
    let registry = Registry::new();
    let capabilities: BTreeSet<_> = registry
        .cops()
        .iter()
        .filter(|c| c.name().starts_with("Lint/") && c.safe_fixes())
        .map(|c| c.name())
        .collect();
    assert_eq!(capabilities, expected);
    let d = project();
    let path = d.path().join("test.rb");
    for c in cases {
        assert_ne!(c.before, c.after, "{}: not a real edit", c.cop);
        assert!(c.boundary.len() > 30);
        let options = RunOptions {
            only: selection(&c).split(',').map(str::to_owned).collect(),
            fix: true,
            ..RunOptions::default()
        };
        fs::write(&path, &c.before).unwrap();
        let report = pipeline::run(
            std::slice::from_ref(&path),
            &options,
            &ResolvedConfig::default(),
            &registry,
        )
        .unwrap();
        assert!(!report.has_errors(), "{}: {report:?}", c.cop);
        assert!(report.corrections_applied > 0, "{}: no actual edit", c.cop);
        assert_eq!(fs::read(&path).unwrap(), c.after.as_bytes(), "{}", c.cop);
        if let Some(held) = c.held {
            fs::write(&path, &held).unwrap();
            let report = pipeline::run(
                std::slice::from_ref(&path),
                &options,
                &ResolvedConfig::default(),
                &registry,
            )
            .unwrap();
            assert!(
                !report.has_errors() && report.offense_count() > 0,
                "{}: held finding lost: {report:?}",
                c.cop
            );
            assert_eq!(report.corrections_applied, 0, "{}: unsafe edit", c.cop);
            assert_eq!(fs::read(&path).unwrap(), held.as_bytes());
        }
    }
}

#[test]
fn all39_actual_cli_canonical_edits_no_fix_no_writes_and_idempotence() {
    let d = project();
    for c in cases() {
        let selected = selection(&c);
        let lint = unchanged(d.path(), &c.before, &["--only", &selected, "test.rb"]);
        ordinary(&lint);
        assert_eq!(lint.status.code(), Some(1), "{}: {lint:?}", c.cop);
        assert!(String::from_utf8_lossy(&lint.stdout).contains(&c.cop));
        let out = cli(d.path(), &["--only", &selected, "--fix", "test.rb"]);
        ordinary(&out);
        assert_eq!(
            fs::read(d.path().join("test.rb")).unwrap(),
            c.after.as_bytes(),
            "{}: {out:?}",
            c.cop
        );
        let out = unchanged(
            d.path(),
            &c.after,
            &["--only", &selected, "--fix", "test.rb"],
        );
        ordinary(&out);
        if let Some(held) = c.held {
            let out = unchanged(d.path(), &held, &["--only", &selected, "--fix", "test.rb"]);
            ordinary(&out);
            assert_eq!(out.status.code(), Some(1), "{}: {out:?}", c.cop);
            assert!(String::from_utf8_lossy(&out.stdout).contains(&c.cop));
        }
    }
}

#[test]
fn every_canonical_obeys_selection_config_and_both_directive_prefixes() {
    let d = project();
    for c in cases() {
        let out = unchanged(
            d.path(),
            &c.before,
            &["--only", &c.cop, "--except", "Lint", "--fix", "test.rb"],
        );
        assert_eq!(out.status.code(), Some(0), "{}: {out:?}", c.cop);
        // Department disable is overridden only by an explicit cop enable.
        fs::write(
            d.path().join("rustocop.toml"),
            format!(
                "[cops.Lint]\nenabled = false\n[cops.\"{}\"]\nenabled = false\n",
                c.cop
            ),
        )
        .unwrap();
        let out = unchanged(
            d.path(),
            &c.before,
            &[
                "--fix",
                "test.rb",
                "--except",
                "Style,Layout,Naming,Security,Bundler,Metrics",
            ],
        );
        assert_eq!(out.status.code(), Some(0), "{}: {out:?}", c.cop);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        // Meta-directive cops intentionally inspect directive comments; their
        // own lifecycle scope/partial-removal controls are covered separately.
        if c.cop.contains("CopDirective") {
            continue;
        }
        for prefix in ["rubocop", "rustocop"] {
            for start in [
                format!("# {prefix}:disable {}\n", c.cop),
                format!("# {prefix}:push -{}\n", c.cop),
            ] {
                let source = format!("{start}{}", c.before);
                let out = unchanged(d.path(), &source, &["--only", &c.cop, "--fix", "test.rb"]);
                assert_eq!(out.status.code(), Some(0), "{}: {out:?}", c.cop);
            }
        }
    }
}

#[test]
fn three_policy_exceptions_three_version_gates_and_no_index_are_no_write() {
    let d = project();
    for (cop, source, status) in [
        ("Lint/MultipleComparison", "1 < 2 < 3\n", 1),
        ("Lint/PercentSymbolArray", "%i(:alpha, :beta,)\n", 1),
        ("Lint/TopLevelReturnWithArgument", "return 7, 8\n", 1),
        ("Lint/ItWithoutArgumentsInBlock", "[1].map { it }\n", 0),
        (
            "Lint/UselessElseWithoutRescue",
            "begin; nil; else; nil; end\n",
            2,
        ),
        (
            "Lint/NonDeterministicRequireOrder",
            "Dir['*.rb'].each { |path| require path }\n",
            0,
        ),
        ("Lint/ArgumentMismatch", "def call(a); end\ncall\n", 0),
        (
            "Lint/UnusedPrivateMethod",
            "class Foo\n  private\n  def unused; end\nend\n",
            0,
        ),
    ] {
        let out = unchanged(d.path(), source, &["--only", cop, "--fix", "test.rb"]);
        assert_eq!(out.status.code(), Some(status), "{cop}: {out:?}");
    }
}

#[test]
fn fatal_syntax_and_fixed_path_exclusion_survive_force_and_fix() {
    let d = project();
    let out = unchanged(
        d.path(),
        "def broken(\nalpha + beta * gamma\n",
        &[
            "--only",
            "Lint/AmbiguousOperatorPrecedence",
            "--fix",
            "test.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    fs::write(d.path().join("test.jb"), "return 7, 8\n").unwrap();
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Lint/TopLevelReturnWithArgument\"]\nenabled = true\n",
    )
    .unwrap();
    let out = cli(
        d.path(),
        &[
            "--only",
            "Lint/TopLevelReturnWithArgument",
            "--fix",
            "test.jb",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read(d.path().join("test.jb")).unwrap(),
        b"return 7, 8\n"
    );
}

#[test]
fn incompatible_lint_style_pair_both_orders_retains_uncertain_api_changes() {
    let d = project();
    let source =
        "def sample(unused)\n  if block_given?; [1].each { |value| yield value }; end\nend\n";
    for only in [
        "Lint/UnusedMethodArgument,Style/ExplicitBlockArgument",
        "Style/ExplicitBlockArgument,Lint/UnusedMethodArgument",
    ] {
        let out = unchanged(d.path(), source, &["--only", only, "--fix", "test.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains("Lint/UnusedMethodArgument"));
    }
}

#[test]
fn explicit_lint_style_layout_naming_security_composition_has_exact_output() {
    let d = project();
    let source =
        "alpha + beta * gamma\nvalue = 'integration372'  \ndef BadName; end\n[value].hash\n";
    let expected =
        "alpha + (beta * gamma)\nvalue = \"integration372\"\ndef BadName; end\n[value].hash\n";
    for selection in [
        "Lint/AmbiguousOperatorPrecedence,Style/StringLiterals,Layout/TrailingWhitespace,Naming/MethodName,Security/CompoundHash",
        "Security/CompoundHash,Naming/MethodName,Layout/TrailingWhitespace,Style/StringLiterals,Lint/AmbiguousOperatorPrecedence",
    ] {
        fs::write(d.path().join("test.rb"), source).unwrap();
        let out = cli(d.path(), &["--only", selection, "--fix", "test.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(
            fs::read(d.path().join("test.rb")).unwrap(),
            expected.as_bytes()
        );
        for cop in ["Naming/MethodName", "Security/CompoundHash"] {
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(cop),
                "{cop}: {out:?}"
            );
        }
        ordinary(&unchanged(
            d.path(),
            expected,
            &["--only", selection, "--fix", "test.rb"],
        ));
    }
}

#[test]
fn canonical_stdin_is_detection_only_and_fix_rejected_without_touching_named_file() {
    let d = project();
    let source = "if false\n  number + 0\nend\n";
    for fix in [false, true] {
        fs::write(d.path().join("test.rb"), "sentinel\n").unwrap();
        let metadata = fs::metadata(d.path().join("test.rb")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
        command.current_dir(d.path()).args([
            "--only",
            "Lint/UselessNumericOperation",
            "--stdin",
            "test.rb",
        ]);
        if fix {
            command.arg("--fix");
        }
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        stdin::write_stdin(child.stdin.take().unwrap(), source.as_bytes(), fix).unwrap();
        let out = child.wait_with_output().unwrap();
        if fix {
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("--stdin"),
                "{out:?}"
            );
        }
        assert_eq!(out.status.code(), Some(if fix { 2 } else { 1 }), "{out:?}");
        assert_eq!(fs::read(d.path().join("test.rb")).unwrap(), b"sentinel\n");
        assert_eq!(
            fs::metadata(d.path().join("test.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            metadata.modified().unwrap()
        );
    }
}

#[test]
fn authored_native_warning_and_live_rescue_boundaries_are_independent_of_reference() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    // Separately authored closed programs, not loaded reference/corpus fixtures.
    let before =
        "begin; raise StandardError; rescue StandardError, nil; puts :integration_caught; end";
    let after = "begin; raise StandardError; rescue StandardError; puts :integration_caught; end";
    let a = native::run(&ruby, before);
    let b = native::run(&ruby, after);
    assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
    assert_eq!(a.stdout, b"integration_caught\n");
    let a = native::run(&ruby, "END { puts :integration_finished }; return 7, 8");
    let b = native::run(&ruby, "END { puts :integration_finished }; return");
    assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
    assert_eq!(a.stdout, b.stdout);
    assert_ne!(
        a.stderr, b.stderr,
        "return rewrite must remain a warning-changing no-edit exception"
    );
}

#[test]
#[cfg(unix)]
fn every_canonical_preserves_symlink_mode_held_fd_and_readonly_failure() {
    use std::{
        io::Read,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let d = project();
    let path = d.path().join("test.rb");
    symlink(&path, d.path().join("alias.rb")).unwrap();
    for c in cases() {
        fs::write(&path, &c.before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let mut held = fs::File::open(&path).unwrap();
        let selected = selection(&c);
        let out = cli(d.path(), &["--only", &selected, "--fix", "alias.rb"]);
        ordinary(&out);
        // Self-require uses supplied lexical basename, never symlink target.
        if c.cop == "Lint/RequireRelativeSelfPath" {
            assert_eq!(fs::read(&path).unwrap(), c.before.as_bytes());
            continue;
        }
        assert_eq!(
            fs::read(&path).unwrap(),
            c.after.as_bytes(),
            "{}: {out:?}",
            c.cop
        );
        let mut original = Vec::new();
        held.read_to_end(&mut original).unwrap();
        assert_eq!(original, c.before.as_bytes());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(
            fs::symlink_metadata(d.path().join("alias.rb"))
                .unwrap()
                .is_symlink()
        );
        fs::write(&path, &c.before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = cli(d.path(), &["--only", &selected, "--fix", "alias.rb"]);
        assert_eq!(out.status.code(), Some(2), "{}: {out:?}", c.cop);
        assert_eq!(fs::read(&path).unwrap(), c.before.as_bytes());
        let new = fs::metadata(&path).unwrap();
        assert_eq!(new.modified().unwrap(), old.modified().unwrap());
        assert_eq!(new.permissions(), old.permissions());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    }
}

#[test]
fn multiline_next_push_toml_exclusions_and_force_precedence() {
    let d = project();
    let cop = "Lint/AmbiguousOperatorPrecedence";
    let before = "alpha + beta * gamma\n";
    for prefix in ["rustocop", "rubocop"] {
        for directive in [
            format!("# {prefix}:disable-next {cop}\n"),
            format!("# {prefix}:next -{cop}\n"),
        ] {
            let source = format!("{directive}{before}{before}");
            fs::write(d.path().join("test.rb"), &source).unwrap();
            let out = cli(d.path(), &["--only", cop, "--fix", "test.rb"]);
            ordinary(&out);
            assert_eq!(
                fs::read_to_string(d.path().join("test.rb")).unwrap(),
                format!("{directive}{before}alpha + (beta * gamma)\n")
            );
        }
        let source =
            format!("# {prefix}:disable-next {cop}\nconsume(\n  alpha + beta * gamma\n)\n{before}");
        fs::write(d.path().join("test.rb"), &source).unwrap();
        let out = cli(d.path(), &["--only", cop, "--fix", "test.rb"]);
        ordinary(&out);
        assert_eq!(
            fs::read_to_string(d.path().join("test.rb")).unwrap(),
            format!(
                "# {prefix}:disable-next {cop}\nconsume(\n  alpha + beta * gamma\n)\nalpha + (beta * gamma)\n"
            )
        );
    }
    fs::write(d.path().join("rustocop.toml"), format!("[cops.Lint]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\nexclude = [\"test.rb\"]\n")).unwrap();
    let out = unchanged(d.path(), before, &["--only", cop, "--fix", "test.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Lint]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
    )
    .unwrap();
    fs::write(d.path().join("test.rb"), before).unwrap();
    let out = cli(
        d.path(),
        &[
            "--except",
            "Style,Layout,Naming,Security,Bundler,Metrics",
            "--fix",
            "test.rb",
        ],
    );
    ordinary(&out);
    assert_eq!(
        fs::read_to_string(d.path().join("test.rb")).unwrap(),
        "alpha + (beta * gamma)\n"
    );
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{cop}\"]\nEnforcedStyle = 'invented'\n"),
    )
    .unwrap();
    let out = unchanged(d.path(), before, &["--only", cop, "--fix", "test.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[test]
#[cfg(unix)]
fn script_permission_has_no_chmod_and_fatal_encoding_never_fixes() {
    use std::os::unix::fs::PermissionsExt;
    let d = project();
    let path = d.path().join("test.rb");
    fs::write(&path, "#!/usr/bin/env ruby\nputs :integration\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let before = fs::read(&path).unwrap();
    let out = cli(
        d.path(),
        &["--only", "Lint/ScriptPermission", "--fix", "test.rb"],
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), before);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(new.permissions(), old.permissions());
    assert_eq!(new.modified().unwrap(), old.modified().unwrap());
    let before = b"# encoding: UTF-8\n'\xff'\nalpha + beta * gamma\n";
    fs::write(&path, before).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = cli(
        d.path(),
        &[
            "--only",
            "Lint/AmbiguousOperatorPrecedence",
            "--fix",
            "test.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        old.modified().unwrap()
    );
}
