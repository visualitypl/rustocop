//! real pipeline/CLI controls and controlled Ruby semantic witnesses.
use crate::support::cli::project;
use std::{fs, path::Path, process::Command};
const IF: &str = "Style/NegatedIf";
const UNLESS: &str = "Style/NegatedUnless";
const ELSE: &str = "Style/NegatedIfElseCondition";
const ALL: &str = "Style/NegatedIf,Style/NegatedUnless,Style/NegatedIfElseCondition";
const CASES: [(&str, &str, &str); 3] = [
    (IF, "if !false\n work\nend\n", "unless false\n work\nend\n"),
    (
        UNLESS,
        "unless !true\n work\nend\n",
        "if true\n work\nend\n",
    ),
    (
        ELSE,
        "if !false\n a\nelse\n b\nend\n",
        "if false\n b\nelse\n a\nend\n",
    ),
];
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, cop: &str, source: &[u8], args: &[&str], count: usize) {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(root, args);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(&format!("({cop})")))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
fn fixtures() -> Vec<(&'static str, Vec<serde_json::Value>)> {
    // RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    [
        (IF, include_str!("fixtures/negated_if_pinned.json")),
        (UNLESS, include_str!("fixtures/negated_unless_pinned.json")),
        (
            ELSE,
            include_str!("fixtures/negated_if_else_condition_pinned.json"),
        ),
    ]
    .into_iter()
    .map(|(cop, json)| (cop, serde_json::from_str(json).unwrap()))
    .collect()
}
#[test]
fn every_applicable_pinned_diagnostic_and_message_is_exact() {
    let mut calls = 0;
    for (cop, cases) in fixtures() {
        for c in cases {
            calls += 1;
            let registry = rustocop::cops::Registry::new();
            let cfg = rustocop::config::ResolvedConfig::default();
            let active = rustocop::cops::select_active(&registry, &cfg, &[cop.into()], &[]);
            let result = rustocop::pipeline::lint_one(
                Path::new("a.rb"),
                Path::new("a.rb"),
                Some(c["source"].as_str().unwrap().as_bytes()),
                &active,
                &cfg,
            );
            assert!(
                result.errors.is_empty() && result.internal_errors.is_empty(),
                "{result:?}"
            );
            let found = result.result.offenses;
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            assert_eq!(serde_json::json!(positions), c["positions"], "{cop}: {c}");
            assert_eq!(serde_json::json!(messages), c["messages"], "{cop}: {c}");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }
    assert_eq!(calls, 56);
}
#[test]
fn default_on_selectors_config_and_plain_lint_preserve_bytes() {
    let d = project();
    for (cop, source, _) in CASES {
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("a.rb");
            unchanged(d.path(), cop, source.as_bytes(), &args, count);
        }
        let r = rustocop::cops::Registry::new();
        let rule = r.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    }
}
#[test]
fn canonical_single_pass_corrections_stabilize_and_incompatible_peers_withhold() {
    let d = project();
    for (cop, source, after) in CASES {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(0), "{o:?}");
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        unchanged(
            d.path(),
            cop,
            after.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            0,
        );
    }
    for peer in ["Style/InverseMethods", "Style/Not"] {
        let selector = format!("{ELSE},{peer}");
        unchanged(
            d.path(),
            ELSE,
            CASES[2].1.as_bytes(),
            &["--only", &selector, "--fix", "a.rb"],
            1,
        );
    }
}
fn observations_equal(root: &Path) {
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby)
            .arg(root.join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby).arg(root.join("a.rb")).output().unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
    }
}
#[test]
fn runtime_canonical_fixes_preserve_values_effects_scope_and_branch_association() {
    let d = project();
    for (cop, source) in [
        (IF, "x = 0\nx = 1 if !false\np [x, defined?(x)]\n"),
        (UNLESS, "x = 0\nx = 1 unless !true\np [x, defined?(x)]\n"),
        (
            ELSE,
            "events = []\nx = 0\nvalue = if !false\n events << :a\n x = 1\n :yes\nelse\n events << :b\n x = 2\n :no\nend\np [value, x, events]\n",
        ),
        (
            ELSE,
            "events = []\nvalue = if !true\n begin\n  events << :a\n  :yes\n ensure\n  events << :ensure_a\n end\nelse\n begin\n  events << :b\n  :no\n ensure\n  events << :ensure_b\n end\nend\np [value, events]\n",
        ),
        (ELSE, "p(!false ? (!true ? :a : :b) : :c)\n"),
    ] {
        fs::write(d.path().join("before.rb"), source).unwrap();
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        assert_ne!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            source,
            "{o:?}"
        );
        observations_equal(d.path());
    }
}
#[test]
fn selected_style_and_layout_composition_is_deterministic_and_clean_errors_do_not_write() {
    let d = project();
    let source = "if !false\n  puts 'ok';\nend\n";
    let selectors = format!(
        "{ALL},Style/StringLiterals,Style/Semicolon,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    let mut outputs = vec![];
    for _ in 0..2 {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = cli(d.path(), &["--only", &selectors, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        outputs.push(fs::read(d.path().join("a.rb")).unwrap());
    }
    assert_eq!(outputs[0], outputs[1]);
    for (cop, _, _) in CASES {
        unchanged(
            d.path(),
            cop,
            b"work\n",
            &["--only", cop, "--fix", "a.rb"],
            0,
        );
        fs::write(d.path().join("a.rb"), "if broken\n").unwrap();
        let modified = fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(2), "{o:?}");
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"if broken\n");
        assert_eq!(
            fs::metadata(d.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
    }
}
