//! Safe-correction integration tests, with production Style cops and synthetic
//! cops for unsafe/conflicting/crashing infrastructure cases.
//!
//! Style fixture examples ported from RuboCop's spec/rubocop/cop/style specs
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

use super::*;
use crate::{
    cops::{Cop, NodeContext},
    offense::Correction,
};

struct Fixer {
    name: &'static str,
    method: &'static [u8],
    replacement: Option<&'static str>,
    safe: bool,
}

impl Cop for Fixer {
    fn name(&self) -> &'static str {
        self.name
    }
    fn safe_fixes(&self) -> bool {
        self.safe
    }
    fn unsafe_fixes(&self) -> bool {
        !self.safe
    }
    fn check_node<'pr>(
        &self,
        node: &ruby_prism::Node<'pr>,
        _: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != self.method {
            return;
        }
        let loc = call.message_loc().unwrap();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "test fix".into(),
            line,
            end_line: line,
            col,
            correction: self.replacement.map(|text| Correction {
                range: loc.start_offset()..loc.end_offset(),
                replacement: text.into(),
            }),
        });
    }
}

fn registry() -> Registry {
    Registry::from_cops(vec![
        Box::new(Fixer {
            name: "Style/SafeFix",
            method: b"before",
            replacement: Some("after"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Style/UnsafeFix",
            method: b"risky",
            replacement: Some("changed"),
            safe: false,
        }),
        Box::new(Fixer {
            name: "Lint/Unfixable",
            method: b"unfixable",
            replacement: None,
            safe: true,
        }),
    ])
}

fn project(source: &str) -> (tempfile::TempDir, ResolvedConfig) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("x.rb"), source).unwrap();
    let config = ResolvedConfig {
        root: dir.path().to_path_buf(),
        ..ResolvedConfig::default()
    };
    (dir, config)
}

fn run_single_pass(dir: &Path, config: &ResolvedConfig, registry: &Registry) -> RunReport {
    run(
        &[dir.to_path_buf()],
        &RunOptions {
            fix: true,
            fix_passes: Some(1),
            ..RunOptions::default()
        },
        config,
        registry,
    )
    .unwrap()
}

fn run_project(dir: &Path, config: &ResolvedConfig, registry: &Registry, fix: bool) -> RunReport {
    run(
        &[dir.to_path_buf()],
        &RunOptions {
            fix,
            ..RunOptions::default()
        },
        config,
        registry,
    )
    .unwrap()
}

#[test]
fn correction_validation_rejects_ruby_4_logical_chains_without_writing() {
    for replacement in ["foo\n  && bar", "foo\n  || bar"] {
        let registry = Registry::from_cops(vec![Box::new(Fixer {
            name: "Style/VersionFix",
            method: b"before",
            replacement: Some(replacement),
            safe: true,
        })]);
        let (dir, config) = project("before\n");
        let path = dir.path().join("x.rb");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let report = run_project(dir.path(), &config, &registry, true);
        assert_eq!(report.exit_code(), EXIT_ERRORS);
        assert_eq!(report.corrections_applied, 0);
        assert_eq!(report.offense_count(), 1);
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].message,
            "corrections of Style/VersionFix failed validation and were not applied"
        );
        assert_eq!(fs::read(&path).unwrap(), b"before\n");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn a_run_at_ruby_4_parses_and_validates_with_the_ruby_4_grammar() {
    use crate::cops::policy::{DEFAULT_RUBY_VERSION, RubyVersion};
    for replacement in ["foo\n  && bar", "foo\n  || bar"] {
        let registry = Registry::from_cops(vec![Box::new(Fixer {
            name: "Style/VersionFix",
            method: b"before",
            replacement: Some(replacement),
            safe: true,
        })]);
        let (dir, mut config) = project("before\n");
        config.ruby_version = RubyVersion(4, 0);
        let path = dir.path().join("x.rb");
        let report = run_project(dir.path(), &config, &registry, true);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.corrections_applied, 1);
        assert_eq!(
            fs::read(&path).unwrap(),
            format!("{replacement}\n").as_bytes()
        );

        let accepted = run_project(dir.path(), &config, &registry, false);
        assert!(accepted.errors.is_empty(), "{:?}", accepted.errors);
        config.ruby_version = DEFAULT_RUBY_VERSION;
        let rejected = run_project(dir.path(), &config, &registry, false);
        assert!(
            rejected.errors[0]
                .message
                .starts_with("syntax error on line 2"),
            "{:?}",
            rejected.errors
        );
    }
}

#[test]
fn corrections_that_exceed_the_ast_depth_budget_are_not_written() {
    // At the limit; the correction's two arrays go past it.
    let source = format!("{}before{}\n", "[".repeat(4093), "]".repeat(4093));
    let registry = Registry::from_cops(vec![Box::new(Fixer {
        name: "Style/DeepFix",
        method: b"before",
        replacement: Some("[[after]]"),
        safe: true,
    })]);
    let (dir, config) = project(&source);
    let report = run_project(dir.path(), &config, &registry, true);
    assert_eq!(report.exit_code(), EXIT_ERRORS);
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(report.offense_count(), 1);
    assert!(report.internal_errors.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(
        report.errors[0].message,
        "corrections of Style/DeepFix failed validation and were not applied"
    );
    assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), source);
}

#[test]
fn corrections_that_exhaust_parser_budgets_are_not_written() {
    struct ParserBudgetFix(String);

    impl Cop for ParserBudgetFix {
        fn name(&self) -> &'static str {
            "Style/ParserBudgetFix"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            if file.bytes != b"before\n" {
                return;
            }
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "test parser validation".into(),
                line: 1,
                end_line: 1,
                col: 1,
                correction: Some(Correction {
                    range: 0..file.bytes.len(),
                    replacement: self.0.clone(),
                }),
            });
        }
    }

    for replacement in [
        format!(
            "{}{}",
            "def foo(*args)\nbar(*args)\n".repeat(2100),
            "end\n".repeat(2100)
        ),
        format!(
            "value in {}capture{}\n",
            "[".repeat(12_000),
            "]".repeat(12_000)
        ),
    ] {
        let registry = Registry::from_cops(vec![Box::new(ParserBudgetFix(replacement))]);
        let (dir, config) = project("before\n");
        let report = run_project(dir.path(), &config, &registry, true);
        assert_eq!(report.exit_code(), EXIT_ERRORS);
        assert_eq!(report.corrections_applied, 0);
        assert_eq!(report.offense_count(), 1);
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].message,
            "corrections of Style/ParserBudgetFix failed validation and were not applied"
        );
        assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn control_flow_hash_fixes_do_not_block_unrelated_corrections() {
    let registry = Registry::from_cops(vec![
        Box::new(crate::cops::style::HashSyntax::default()),
        Box::new(crate::cops::style::SignalException),
    ]);
    for (before, after) in [
        (
            "p [1].map { next :a => 1 }\nfail\n",
            "p [1].map { next {a: 1} }\nraise\n",
        ),
        (
            "loop { break :a => 1, :b => 2 }\nfail\n",
            "loop { break {a: 1, b: 2} }\nraise\n",
        ),
    ] {
        let (dir, config) = project(before);
        let report = run_project(dir.path(), &config, &registry, true);
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.corrections_applied, 2);
        assert_eq!(report.offense_count(), 0);
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), after);
    }
}

#[test]
fn fixture_pairs_apply_safe_fixes_and_report_remaining_offenses() {
    for (before, after, count, remaining) in [
        (
            include_str!("../../tests/fixtures/fix/safe.before.rb"),
            include_str!("../../tests/fixtures/fix/safe.after.rb"),
            2,
            0,
        ),
        (
            include_str!("../../tests/fixtures/fix/mixed.before.rb"),
            include_str!("../../tests/fixtures/fix/mixed.after.rb"),
            2,
            2,
        ),
        (
            include_str!("../../tests/fixtures/fix/directives.before.rb"),
            include_str!("../../tests/fixtures/fix/directives.after.rb"),
            1,
            0,
        ),
    ] {
        let (dir, config) = project(before);
        let report = run_project(dir.path(), &config, &registry(), true);
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), after);
        assert_eq!(report.corrections_applied, count);
        assert_eq!(report.offense_count(), remaining);
        assert_eq!(
            report.exit_code(),
            if remaining == 0 {
                EXIT_CLEAN
            } else {
                EXIT_OFFENSES
            }
        );
        assert!(crate::output::render_text(&report).contains(&format!(
            "{count} correction{} applied, {remaining} offense{} remaining",
            if count == 1 { "" } else { "s" },
            if remaining == 1 { "" } else { "s" }
        )));
    }
}

#[test]
fn detection_only_never_rewrites_even_when_cops_supply_safe_fixes() {
    let (dir, config) = project("before\nrisky\n");
    let report = run_project(dir.path(), &config, &registry(), false);
    assert_eq!(report.offense_count(), 2);
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        b"before\nrisky\n"
    );
    assert_eq!(
        crate::output::render_text(&report),
        "x.rb:1:1: C: test fix (Style/SafeFix)\nx.rb:2:1: C: test fix (Style/UnsafeFix)\n\n1 file inspected, 2 offenses detected\n"
    );
}

#[test]
fn production_style_fixture_pairs() {
    macro_rules! fixture {
        ($name:literal, $cop:literal, $remaining:literal) => {
            (
                $cop,
                include_str!(concat!("../../tests/fixtures/fix/", $name, ".before.rb")),
                include_str!(concat!("../../tests/fixtures/fix/", $name, ".after.rb")),
                $remaining,
            )
        };
    }
    let registry = Registry::new();
    for (cop, before, after, remaining) in [
        fixture!("string_literals", "Style/StringLiterals", 0),
        fixture!(
            "string_literals_in_interpolation",
            "Style/StringLiteralsInInterpolation",
            0
        ),
        fixture!("hash_syntax", "Style/HashSyntax", 0),
        fixture!("redundant_return", "Style/RedundantReturn", 0),
        fixture!("semicolon", "Style/Semicolon", 1),
        fixture!("redundant_parentheses", "Style/RedundantParentheses", 1),
        fixture!("signal_exception", "Style/SignalException", 0),
    ] {
        let (dir, config) = project(before);
        for fix in [false, true] {
            let report = run(
                &[dir.path().to_path_buf()],
                &RunOptions {
                    fix,
                    fix_passes: fix.then_some(1),
                    only: vec![cop.into()],
                    ..RunOptions::default()
                },
                &config,
                &registry,
            )
            .unwrap();
            assert!(!report.has_errors(), "{cop}: {report:?}");
            assert_eq!(
                fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                if fix { after } else { before },
                "{cop}"
            );
            if fix {
                assert!(report.corrections_applied > 0, "{cop}");
                assert_eq!(report.offense_count(), remaining, "{cop}");
            } else {
                assert_eq!(report.corrections_applied, 0);
            }
        }
    }
}

#[test]
fn production_corrections_reach_disabled_interior_lines_as_in_rubocop() {
    // RuboCop 1.92 (pinned) `--only COP -a` output. A pair on a disabled line
    // is its own offense, so it keeps its syntax.
    for (cop, source, after) in [
        (
            "Style/RedundantParentheses",
            "(\n  1 # rubocop:disable Style/RedundantParentheses\n)\n",
            "(\n  1 # rubocop:disable Style/RedundantParentheses\n)\n",
        ),
        (
            "Style/HashSyntax",
            "return :a => 1,\n  :b => 2 # rubocop:disable Style/HashSyntax\n",
            "return {a: 1,\n  :b => 2} # rubocop:disable Style/HashSyntax\n",
        ),
        (
            "Style/HashSyntax",
            "return :a => 1,\n  :b => 2 # rustocop:disable Style/HashSyntax\n",
            "return {a: 1,\n  :b => 2} # rustocop:disable Style/HashSyntax\n",
        ),
        (
            "Style/RedundantReturn",
            "def func\n  return 1,\n    2 # rubocop:disable Style/RedundantReturn\nend\n",
            "def func\n  [1,\n    2] # rubocop:disable Style/RedundantReturn\nend\n",
        ),
    ] {
        let (dir, config) = project(source);
        let report = run(
            &[dir.path().to_path_buf()],
            &RunOptions {
                fix: true,
                only: vec![cop.into()],
                ..RunOptions::default()
            },
            &config,
            &Registry::new(),
        )
        .unwrap();
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(
            fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            after,
            "{cop}"
        );
    }
}

#[test]
fn production_overlaps_defer_whole_corrections_and_converge() {
    let (dir, config) = project("def func\n  return ({ :a => 'x' });\nend\n");
    let registry = Registry::new();
    let first = run_single_pass(dir.path(), &config, &registry);
    assert!(!first.has_errors(), "{first:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        "def func\n  { :a => 'x' }\nend\n"
    );
    assert_eq!(first.offense_count(), 2);
    for _ in 0..2 {
        let report = run_single_pass(dir.path(), &config, &registry);
        assert!(!report.has_errors(), "{report:?}");
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        "def func\n  { a: \"x\" }\nend\n"
    );
    let final_report = run_single_pass(dir.path(), &config, &registry);
    assert_eq!(final_report.corrections_applied, 0);
    assert_eq!(final_report.offense_count(), 0);
}

#[test]
fn production_fixes_preserve_non_utf8_bytes_and_detection() {
    let (dir, config) = project("");
    let source = b"# encoding: ISO-8859-1\nx = '\xff'\nfail\n";
    fs::write(dir.path().join("x.rb"), source).unwrap();
    let report = run(
        &[dir.path().to_path_buf()],
        &RunOptions {
            fix: true,
            only: vec![
                "Style/StringLiterals".into(),
                "Style/SignalException".into(),
            ],
            ..RunOptions::default()
        },
        &config,
        &Registry::new(),
    )
    .unwrap();
    assert!(!report.has_errors(), "{report:?}");
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(report.offense_count(), 1);
    assert_eq!(report.results[0].offenses[0].cop, "Style/StringLiterals");
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        b"# encoding: ISO-8859-1\nx = '\xff'\nraise\n"
    );
}

#[test]
fn encoded_columns_do_not_change_correction_byte_ranges_or_output() {
    use crate::cops::{layout::TrailingWhitespace, lint::Debugger, style::SignalException};

    let registry = Registry::from_cops(vec![
        Box::new(TrailingWhitespace),
        Box::new(Debugger),
        Box::new(SignalException),
    ]);
    for (prefix, col) in super::tests::encoded_position_prefixes() {
        let mut source = prefix.to_vec();
        source.extend_from_slice(b"fail; binding.pry  \n");
        let (dir, config) = project("");
        let path = dir.path().join("x.rb");
        fs::write(&path, &source).unwrap();
        let collected = lint_one_in_mode(
            &path,
            Path::new("x.rb"),
            SourceInput::Memory(&source),
            &CopSelection::explicit(&[&TrailingWhitespace, &Debugger, &SignalException], &config),
            &config,
            CorrectionMode::Collect,
        );
        assert!(collected.errors.is_empty(), "{collected:?}");
        assert!(collected.internal_errors.is_empty(), "{collected:?}");
        let fail = collected
            .result
            .offenses
            .iter()
            .find(|o| o.cop == "Style/SignalException")
            .unwrap();
        assert_eq!((fail.line, fail.col), (2, col));
        let correction = fail.correction.as_ref().unwrap();
        assert_eq!(correction.range, prefix.len()..prefix.len() + 4);
        assert_eq!(&source[correction.range.clone()], b"fail");
        let whitespace = collected
            .result
            .offenses
            .iter()
            .find(|o| o.cop == "Layout/TrailingWhitespace")
            .unwrap();
        assert_eq!((whitespace.line, whitespace.col), (2, col + 17));
        assert_eq!(
            whitespace.correction.as_ref().unwrap().range,
            source.len() - 3..source.len() - 1
        );
        assert_eq!(fs::read(&path).unwrap(), source, "collect must not write");

        let report = run_project(dir.path(), &config, &registry, true);
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.corrections_applied, 2);
        let mut expected = prefix.to_vec();
        expected.extend_from_slice(b"raise; binding.pry\n");
        assert_eq!(fs::read(&path).unwrap(), expected);
        assert_eq!(report.offense_count(), 1);
        let remaining = &report.results[0].offenses[0];
        assert_eq!(remaining.cop, "Lint/Debugger");
        assert_eq!((remaining.line, remaining.col), (2, col + 7));
    }
}

#[test]
fn config_excludes_and_selectors_also_prevent_fixes() {
    let (dir, mut config) = project("before\n");
    config.cop_settings.insert(
        "Style/SafeFix".into(),
        crate::config::CopSettings {
            excludes: crate::config::compile_globs(&["x.rb"], "test").unwrap(),
            ..Default::default()
        },
    );
    let report = run_project(dir.path(), &config, &registry(), true);
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
    config.cop_settings.clear();
    let report = run(
        &[dir.path().to_path_buf()],
        &RunOptions {
            fix: true,
            except: vec!["Style/SafeFix".into()],
            ..Default::default()
        },
        &config,
        &registry(),
    )
    .unwrap();
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
}

#[cfg(unix)]
#[test]
fn excludes_hold_for_absolute_paths_through_a_symlink_to_the_root() {
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let root = base.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("x.rb"), "before\n").unwrap();
    std::os::unix::fs::symlink(&root, base.join("alias")).unwrap();
    let supplied = [base.join("alias/x.rb")];
    let fix = RunOptions {
        fix: true,
        ..RunOptions::default()
    };
    let mut config = ResolvedConfig {
        root: root.clone(),
        ..ResolvedConfig::default()
    };

    let report = run(&supplied, &RunOptions::default(), &config, &registry()).unwrap();
    assert_eq!(report.offense_count(), 1);
    assert_eq!(report.results[0].path, Path::new("x.rb"));

    config.cop_settings.insert(
        "Style/SafeFix".into(),
        crate::config::CopSettings {
            excludes: crate::config::compile_globs(&["x.rb"], "test").unwrap(),
            ..Default::default()
        },
    );
    let report = run(&supplied, &fix, &config, &registry()).unwrap();
    assert_eq!(report.files_inspected, 1);
    assert_eq!(report.offense_count(), 0);
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(fs::read(root.join("x.rb")).unwrap(), b"before\n");

    config.cop_settings.clear();
    config.global_excludes = crate::config::compile_globs(&["x.rb"], "test").unwrap();
    let report = run(&supplied, &fix, &config, &registry()).unwrap();
    assert_eq!(report.files_inspected, 0);
    assert_eq!(fs::read(root.join("x.rb")).unwrap(), b"before\n");

    config.global_excludes.clear();
    let report = run(&supplied, &fix, &config, &registry()).unwrap();
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(fs::read(root.join("x.rb")).unwrap(), b"after\n");
}

#[test]
fn only_autocorrect_all_applies_unsafe_corrections() {
    let (dir, config) = project("before; risky\n");
    let report = run(
        &[dir.path().to_path_buf()],
        &RunOptions {
            fix: true,
            fix_unsafe: true,
            ..RunOptions::default()
        },
        &config,
        &registry(),
    )
    .unwrap();
    assert_eq!(report.corrections_applied, 2);
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        b"after; changed\n"
    );
}

#[test]
fn remaining_positions_are_recomputed_after_length_changing_fixes() {
    let (dir, config) = project("before; risky\n");
    let report = run_project(dir.path(), &config, &registry(), true);
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(report.offense_count(), 1);
    assert_eq!(report.results[0].offenses[0].col, 8);
    assert!(
        crate::output::render_github(&report).contains("1 correction applied, 1 offense remaining")
    );
}

fn rewriting(steps: &[(&'static str, &'static [u8], &'static str)]) -> Registry {
    Registry::from_cops(
        steps
            .iter()
            .map(|&(name, method, replacement)| {
                Box::new(Fixer {
                    name,
                    method,
                    replacement: Some(replacement),
                    safe: true,
                }) as Box<dyn Cop>
            })
            .collect(),
    )
}

#[test]
fn a_cop_whose_edits_break_the_file_does_not_block_the_others() {
    let registry = rewriting(&[
        ("Style/Breaks", b"broken", "("),
        ("Style/Works", b"before", "after"),
    ]);
    let (dir, config) = project("before\nbroken\nbefore\n");
    let report = run_project(dir.path(), &config, &registry, true);
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        b"after\nbroken\nafter\n"
    );
    assert_eq!(report.corrections_applied, 2);
    assert_eq!(report.offense_count(), 1);
    assert!(report.internal_errors.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(
        report.errors[0].message,
        "corrections of Style/Breaks failed validation and were not applied"
    );
    assert_eq!(report.exit_code(), EXIT_ERRORS);
}

#[test]
fn a_fix_run_repeats_passes_until_nothing_changes() {
    let registry = rewriting(&[
        ("Style/First", b"first", "second"),
        ("Style/Second", b"second", "third"),
        ("Style/Third", b"third", "done"),
    ]);
    let (dir, config) = project("first\nsecond\n");
    let report = run_project(dir.path(), &config, &registry, true);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"done\ndone\n");
    assert_eq!(report.corrections_applied, 5);
    assert_eq!(report.offense_count(), 0);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
}

#[test]
fn corrections_that_undo_each_other_stop_with_an_error() {
    let registry = rewriting(&[
        ("Style/Ping", b"ping", "pong"),
        ("Style/Pong", b"pong", "ping"),
    ]);
    let (dir, config) = project("ping\n");
    let report = run_project(dir.path(), &config, &registry, true);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"pong\n");
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(report.offense_count(), 1);
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0].message.contains("earlier version"),
        "{:?}",
        report.errors
    );
    assert_eq!(report.exit_code(), EXIT_ERRORS);
}

#[test]
fn corrections_that_never_settle_stop_at_the_pass_limit() {
    let registry = rewriting(&[("Style/Grow", b"grow", "grow; 1")]);
    let (dir, config) = project("grow\n");
    let report = run_project(dir.path(), &config, &registry, true);
    let expected = format!("grow{}\n", "; 1".repeat(MAX_CORRECTION_PASSES));
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        expected.as_bytes()
    );
    assert_eq!(report.corrections_applied, MAX_CORRECTION_PASSES);
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0].message.contains("did not settle"),
        "{:?}",
        report.errors
    );
}

#[test]
fn a_later_pass_that_fails_validation_keeps_the_earlier_passes() {
    // Each pass nests the chain 40 calls deeper, until one passes the depth
    // limit and fails validation.
    const STEP: usize = 40;
    let deeper = Box::leak(format!("deep{}", ".more".repeat(STEP)).into_boxed_str());
    let registry = rewriting(&[("Style/Deep", b"deep", deeper)]);
    let (dir, config) = project("deep\n");
    let report = run_project(dir.path(), &config, &registry, true);
    let applied = report.corrections_applied;
    assert!(applied > 1 && applied < MAX_CORRECTION_PASSES, "{applied}");
    let expected = format!("deep{}\n", ".more".repeat(STEP * applied));
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        expected.as_bytes()
    );
    assert!(
        crate::parser::parse(expected.as_bytes())
            .depth_error()
            .is_none()
    );
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0].message
            == "corrections of Style/Deep failed validation and were not applied",
        "{:?}",
        report.errors
    );
}

#[test]
fn overlapping_cops_have_a_deterministic_winner_and_are_relinted() {
    for reverse in [false, true] {
        let mut cops: Vec<Box<dyn Cop>> = vec![
            Box::new(Fixer {
                name: "Style/Z",
                method: b"before",
                replacement: Some("z"),
                safe: true,
            }),
            Box::new(Fixer {
                name: "Style/A",
                method: b"before",
                replacement: Some("a"),
                safe: true,
            }),
        ];
        if reverse {
            cops.reverse();
        }
        let (dir, config) = project("before\n");
        let report = run_project(dir.path(), &config, &Registry::from_cops(cops), true);
        assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"a\n");
        assert_eq!(report.corrections_applied, 1);
        assert_eq!(report.offense_count(), 0);
    }
}

#[test]
fn syntax_errors_and_cop_panics_prevent_rewrites() {
    for source in ["before(\n", "before\nboom\n"] {
        let mut registry = registry();
        // A separate registry includes a cop which panics on `boom`.
        if source.contains("boom") {
            registry = Registry::from_cops(vec![
                Box::new(Fixer {
                    name: "Style/SafeFix",
                    method: b"before",
                    replacement: Some("after"),
                    safe: true,
                }),
                Box::new(crate::cops::testing::PanicOnCall),
            ]);
        }
        let (dir, config) = project(source);
        let report = run_project(dir.path(), &config, &registry, true);
        assert!(report.has_errors());
        assert_eq!(report.corrections_applied, 0);
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), source);
    }
}

#[test]
fn corrections_producing_invalid_ruby_are_not_written() {
    let registry = Registry::from_cops(vec![Box::new(Fixer {
        name: "Style/Broken",
        method: b"before",
        replacement: Some("("),
        safe: true,
    })]);
    let (dir, config) = project("before\n");
    let report = run_project(dir.path(), &config, &registry, true);
    assert!(report.has_errors());
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(report.offense_count(), 1);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
}

#[test]
fn correction_span_edits_disabled_lines_outside_the_highlight_as_rubocop_does() {
    struct WideFix;
    impl Cop for WideFix {
        fn name(&self) -> &'static str {
            "Style/WideFix"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "wide fix".into(),
                line: 1,
                end_line: 1,
                col: 1,
                correction: Some(Correction {
                    range: 0..file.bytes.len(),
                    replacement: "after\n".into(),
                }),
            });
        }
    }
    for prefix in ["rubocop", "rustocop"] {
        for tail in [
            format!("before # {prefix}:disable Style/WideFix\n"),
            format!("# {prefix}:disable Style/WideFix\nbefore\n# {prefix}:enable Style/WideFix\n"),
            format!("# {prefix}:todo Style/WideFix\nbefore\n"),
            format!("# {prefix}:disable-next Style/WideFix\nbefore\n"),
            format!("# {prefix}:todo-next Style/WideFix\nbefore\n"),
            format!("# {prefix}:next -Style/WideFix\nbefore\n"),
            format!("# {prefix}:push -Style/WideFix\nbefore\n# {prefix}:pop\n"),
            format!("# {prefix}:push\n# {prefix}:disable Style/WideFix\nbefore\n# {prefix}:pop\n"),
            format!(
                "# {prefix}:disable Style/WideFix\n# {prefix}:enable-next Style/WideFix\nbefore\nbefore\n"
            ),
            format!(
                "# {prefix}:disable Style/WideFix\n# {prefix}:next +Style/WideFix\nbefore\nbefore\n"
            ),
            format!(
                "# {prefix}:disable Style/WideFix\n# {prefix}:push +Style/WideFix\nbefore\n# {prefix}:pop\nbefore\n"
            ),
        ] {
            let source = format!("before\n{tail}");
            let (dir, config) = project(&source);
            let report = run_project(
                dir.path(),
                &config,
                &Registry::from_cops(vec![Box::new(WideFix)]),
                true,
            );
            // RuboCop checks only that the offense's line is enabled.
            assert_eq!(report.corrections_applied, 1, "{source}");
            assert_eq!(
                fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                "after\n"
            );
        }
    }
}

#[test]
fn several_files_and_deferred_fixes_use_a_single_pass() {
    let (dir, config) = project("before\n");
    fs::write(dir.path().join("a.rb"), "before\n").unwrap();
    let registry = Registry::from_cops(vec![
        Box::new(Fixer {
            name: "Style/First",
            method: b"before",
            replacement: Some("after"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Style/Second",
            method: b"after",
            replacement: Some("done"),
            safe: true,
        }),
    ]);
    let report = run_single_pass(dir.path(), &config, &registry);
    assert_eq!(report.files_inspected, 2);
    assert_eq!(report.corrections_applied, 2);
    assert_eq!(report.offense_count(), 2);
    assert_eq!(report.results[0].path, PathBuf::from("a.rb"));
    for path in ["a.rb", "x.rb"] {
        assert_eq!(fs::read(dir.path().join(path)).unwrap(), b"after\n");
    }
}

#[test]
fn write_failure_reports_an_error_and_leaves_original_offenses() {
    let (dir, config) = project("before\n");
    // Use a directory as the write destination, but supply source bytes to
    // exercise the same write-error path without depending on user privileges.
    let report = lint_one_with_fix(
        dir.path(),
        Path::new("x.rb"),
        Some(b"before\n"),
        &CopSelection::new(&registry(), &config, &[], &[]),
        &config,
        Some(Passes {
            max: MAX_CORRECTION_PASSES,
            explicit: false,
            unsafe_fixes: false,
        }),
    );
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(report.result.offenses.len(), 1);
    assert!(
        report.errors[0]
            .message
            .starts_with("could not write corrections:")
    );
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
}

#[cfg(unix)]
#[test]
fn writer_keeps_the_inode_owner_and_hard_links() {
    use std::os::unix::fs::MetadataExt;
    let (dir, _) = project("before\n");
    let path = dir.path().join("x.rb");
    let link = dir.path().join("link.rb");
    fs::hard_link(&path, &link).unwrap();
    // Only root can give the file another owner; elsewhere the owner is ours.
    let owner = if rustix::process::geteuid().is_root() {
        std::os::unix::fs::chown(&path, Some(1234), Some(1234)).unwrap();
        (1234, 1234)
    } else {
        let metadata = fs::metadata(&path).unwrap();
        (metadata.uid(), metadata.gid())
    };
    let inode = fs::metadata(&path).unwrap().ino();
    write_corrected(&path, b"before\n", b"after, longer\n").unwrap();
    write_corrected(&path, b"after, longer\n", b"short\n").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    assert_eq!((metadata.uid(), metadata.gid()), owner);
    assert_eq!(metadata.ino(), inode);
    assert_eq!(fs::read(&link).unwrap(), b"short\n");
}

#[test]
fn writer_rejects_stale_sources() {
    let (dir, _) = project("newer source\n");
    assert!(write_corrected(&dir.path().join("x.rb"), b"before\n", b"after\n").is_err());
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        b"newer source\n"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn preserves_permissions_and_symlinks_and_coordinates_alias_writes() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (dir, config) = project("before\n");
    let path = dir.path().join("x.rb");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    let link = dir.path().join("link.rb");
    symlink(&path, &link).unwrap();
    let report = run(
        &[path.clone(), link.clone()],
        &RunOptions {
            fix: true,
            ..Default::default()
        },
        &config,
        &registry(),
    )
    .unwrap();
    assert_eq!(report.files_inspected, 2);
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(fs::read(&path).unwrap(), b"after\n");
    assert!(fs::symlink_metadata(link).unwrap().is_symlink());
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o750
    );
}

#[cfg(unix)]
#[test]
fn alias_fix_collection_requests_semicolon_corrections_without_writing_early() {
    let (dir, config) = project("x = 1;\n");
    std::os::unix::fs::symlink("x.rb", dir.path().join("alias.rb")).unwrap();
    let registry = Registry::from_cops(vec![Box::new(crate::cops::style::Semicolon::default())]);
    let report = run(
        &[dir.path().join("x.rb"), dir.path().join("alias.rb")],
        &RunOptions {
            fix: true,
            ..Default::default()
        },
        &config,
        &registry,
    )
    .unwrap();
    assert!(!report.has_errors(), "{report:?}");
    assert_eq!(report.files_inspected, 2);
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(report.offense_count(), 0);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"x = 1\n");
}

#[cfg(unix)]
#[test]
fn alias_fixes_merge_path_eligibility_in_one_pass_and_relint_every_path() {
    // Neither alias alone permits both fixes. A second pass through aliases
    // would also apply Deferred, contrary to the single-pass contract.
    let registry = Registry::from_cops(vec![
        Box::new(Fixer {
            name: "Style/First",
            method: b"before",
            replacement: Some("after"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Style/Other",
            method: b"old",
            replacement: Some("longer"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Style/Deferred",
            method: b"after",
            replacement: Some("done"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Lint/Unfixable",
            method: b"unfixable",
            replacement: None,
            safe: false,
        }),
    ]);
    for reverse in [false, true] {
        let (dir, mut config) = project("before; old; unfixable\n");
        let alias = dir.path().join("a.rb");
        std::os::unix::fs::symlink("x.rb", &alias).unwrap();
        for (cop, excluded) in [
            ("Style/First", "a.rb"),
            ("Style/Other", "x.rb"),
            ("Lint/Unfixable", "a.rb"),
        ] {
            config.cop_settings.insert(
                cop.into(),
                crate::config::CopSettings {
                    excludes: crate::config::compile_globs(&[excluded], "test").unwrap(),
                    ..Default::default()
                },
            );
        }
        let mut paths = vec![alias, dir.path().join("x.rb")];
        if reverse {
            paths.reverse();
        }
        let report = run(
            &paths,
            &RunOptions {
                fix: true,
                fix_passes: Some(1),
                ..Default::default()
            },
            &config,
            &registry,
        )
        .unwrap();
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.files_inspected, 2);
        assert_eq!(report.corrections_applied, 2);
        assert_eq!(report.offense_count(), 3);
        assert_eq!(report.exit_code(), EXIT_OFFENSES);
        assert_eq!(
            fs::read(dir.path().join("x.rb")).unwrap(),
            b"after; longer; unfixable\n"
        );
        assert_eq!(report.results[0].path, Path::new("a.rb"));
        assert_eq!(report.results[0].offenses.len(), 1);
        assert_eq!(report.results[0].offenses[0].cop, "Style/Deferred");
        assert_eq!(report.results[1].path, Path::new("x.rb"));
        assert_eq!(report.results[1].offenses[1].cop, "Lint/Unfixable");
        assert_eq!(report.results[1].offenses[1].col, 16);
    }
}

#[test]
fn fix_with_stdin_is_rejected_before_reading_or_writing() {
    let options = RunOptions {
        fix: true,
        stdin: Some("x.rb".into()),
        ..Default::default()
    };
    let err = run(&[], &options, &ResolvedConfig::default(), &registry()).unwrap_err();
    assert!(
        err.to_string()
            .contains("--fix cannot be used with --stdin")
    );
}

/// Each pass lints the corrected source afresh. RuboCop keeps the ranges a
/// cop ignored (`ignore_node`) across passes, so its single run stops at
/// `bar = do_something`; a second RuboCop run, like this one pass loop,
/// also removes the now reported `bar = `.
#[test]
fn passes_do_not_carry_ignored_ranges_over() {
    let (dir, config) = project("def some_method\n  foo = bar = do_something\nend\n");
    let registry = Registry::from_cops(vec![Box::new(crate::cops::lint::UselessAssignment)]);
    run_project(dir.path(), &config, &registry, true);
    assert_eq!(
        fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        "def some_method\n  do_something\nend\n"
    );
}
