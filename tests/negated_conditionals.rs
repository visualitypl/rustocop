//! real pipeline/CLI controls and controlled Ruby semantic witnesses.
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
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
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
fn all_33_pinned_correction_calls_keep_unknown_dispatch_unmodified() {
    let d = project();
    let mut calls = 0;
    for (cop, cases) in fixtures() {
        for c in cases {
            if !c["correction"].is_string() {
                continue;
            }
            calls += 1;
            // x/a_condition/condition.nil? do not prove that !, != and !~
            // are complements of truthiness, == and =~. Each withheld example
            // remains an offense; canonical closed-immediate adaptations run below.
            unchanged(
                d.path(),
                cop,
                c["source"].as_str().unwrap().as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                c["positions"].as_array().unwrap().len(),
            );
        }
    }
    assert_eq!(calls, 33);
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
fn excludes_directives_and_disabled_interior_spans_are_atomic() {
    let d = project();
    for (cop, source, _) in CASES {
        for config in [
            "exclude = [\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                unchanged(
                    d.path(),
                    cop,
                    format!("# {prefix}:disable {target}\n{source}").as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            unchanged(
                d.path(),
                cop,
                format!("# {prefix}:disable-next {cop}\n{source}").as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            unchanged(
                d.path(),
                cop,
                source
                    .replacen('\n', &format!(" # {prefix}:disable {cop}\n"), 1)
                    .as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
    unchanged(
        d.path(),
        ELSE,
        b"if !false\n # rubocop:disable Lint/Debugger\n a\nelse\n b\nend\n",
        &["--only", ELSE, "--fix", "a.rb"],
        1,
    );
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
#[test]
fn source_envelopes_keep_bom_crlf_encoding_data_payloads_and_line_values() {
    let d = project();
    for (cop, source, after) in CASES {
        let before = format!(
            "\u{feff}# encoding: UTF-8\r\n{}p __LINE__\r\n__END__\r\nraw\0payload\r\n",
            source.replace('\n', "\r\n")
        );
        let expected = format!(
            "\u{feff}# encoding: UTF-8\r\n{}p __LINE__\r\n__END__\r\nraw\0payload\r\n",
            after.replace('\n', "\r\n")
        );
        fs::write(d.path().join("a.rb"), &before).unwrap();
        let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(0), "{o:?}");
        assert_eq!(
            fs::read(d.path().join("a.rb")).unwrap(),
            expected.as_bytes()
        );
    }
    for (cop, source) in [
        (
            IF,
            b"if !false\n DATA.pos\nend\n__END__\npayload\n".as_slice(),
        ),
        (UNLESS, b"unless !false\n DATA.pos\nend\n__END__\npayload\n"),
        (ELSE, b"if !false\n __LINE__\nelse\n 3\nend\n"),
        (
            ELSE,
            b"if !false\n <<~TEXT\n  literal\n TEXT\nelse\n :other\nend\n",
        ),
        (
            ELSE,
            b"if !false\n :a\nelse\n :b\nend\np File.read(__FILE__)\n",
        ),
        (
            ELSE,
            b"# coding: ISO-8859-1\nif !false\n '\xe9'\nelse\n :b\nend\n",
        ),
    ] {
        unchanged(d.path(), cop, source, &["--only", cop, "--fix", "a.rb"], 1);
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
fn runtime_custom_dispatch_bindings_and_control_flow_are_not_guessed() {
    let d = project();
    for (cop, source) in [
        (
            IF,
            "class Thing\n def !; puts :negated; true; end\nend\nx = Thing.new\nif !x\n puts :taken\nend\n",
        ),
        (
            UNLESS,
            "class Thing\n def !; puts :negated; true; end\nend\nx = Thing.new\nunless !x\n puts :taken\nend\n",
        ),
        (
            ELSE,
            "class Thing\n def !=(other); puts :ne; true; end\n def ==(other); puts :eq; true; end\nend\nx = Thing.new\np(x != 1 ? :a : :b)\n",
        ),
        (
            ELSE,
            "def a; :method; end\np(if !false\n a\nelse\n a = 1\nend)\n",
        ),
        (
            ELSE,
            "def f\n begin\n  if !false\n   return :yes\n  else\n   return :no\n  end\n ensure\n  puts :ensured\n end\nend\np f\n",
        ),
        (
            ELSE,
            "def f\n if !false\n  :a\n elsif !true\n  :b\n else\n  :c\n end\nend\np f\n",
        ),
    ] {
        fs::write(d.path().join("before.rb"), source).unwrap();
        let expected = usize::from(!source.contains("elsif"));
        unchanged(
            d.path(),
            cop,
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            expected,
        );
        observations_equal(d.path());
    }
}
#[test]
fn runtime_preceding_comparisons_and_implicit_conversions_keep_bytes_and_behavior() {
    let d = project();
    // Preload unknown objects without placing a visible declaration barrier in
    // the linted source. Their hooks invalidate the later core-negation proof.
    let preload = d.path().join("preload.rb");
    fs::write(
        &preload,
        "def invalidate\n FalseClass.class_eval { define_method(:!) { false } }\nend\n$x = Object.new\ndef $x.==(other); invalidate; false; end\ndef $x.to_a; invalidate; []; end\ndef $x.to_ary; invalidate; []; end\ndef $x.to_hash; invalidate; {}; end\ndef $x.to_s; invalidate; ''; end\ndef $x.hash; invalidate; 0; end\n",
    )
    .unwrap();
    for (cop, source, expected) in [
        (IF, "1 != $x; if !false; puts :taken; end\n", ""),
        (
            UNLESS,
            "1 != $x; unless !false; puts :taken; end\n",
            "taken\n",
        ),
        (
            ELSE,
            "p(*$x, (if !false; :taken; else; :skipped; end))\n",
            ":skipped\n",
        ),
        (
            ELSE,
            "p({**$x}, (if !false; :taken; else; :skipped; end))\n",
            "{}\n:skipped\n",
        ),
        (
            ELSE,
            "p(\"#{$x}\", (if !false; :taken; else; :skipped; end))\n",
            "\"\"\n:skipped\n",
        ),
        (
            ELSE,
            "p({$x => 1}.size, (if !false; :taken; else; :skipped; end))\n",
            "1\n:skipped\n",
        ),
        (
            ELSE,
            "a, b = $x; p(if !false; :taken; else; :skipped; end)\n",
            ":skipped\n",
        ),
    ] {
        fs::write(d.path().join("before.rb"), source).unwrap();
        fs::write(d.path().join("a.rb"), source).unwrap();
        let modified = fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        let mut rubies = vec!["ruby".to_owned()];
        if let Ok(ruby) = crate::support::runtime::ruby() {
            rubies.push(ruby);
        }
        for ruby in rubies {
            if Command::new(&ruby).arg("--version").output().is_err() {
                continue;
            }
            for file in ["before.rb", "a.rb"] {
                let result = Command::new(&ruby)
                    .arg("-r")
                    .arg(&preload)
                    .arg(d.path().join(file))
                    .output()
                    .unwrap();
                assert!(result.status.success(), "{ruby}: {result:?}");
                assert_eq!(result.stdout, expected.as_bytes(), "{ruby}: {source}");
            }
        }
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        assert_eq!(
            fs::metadata(d.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        assert!(String::from_utf8_lossy(&o.stdout).contains(&format!("({cop})")));
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
