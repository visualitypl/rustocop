//! Ported pinned RuboCop style/{nil_comparison,non_nil_check,not}
//! specs (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
use serde::Deserialize;
use std::{fs, path::Path, process::Command};
const COPS: [&str; 3] = ["Style/NilComparison", "Style/NonNilCheck", "Style/Not"];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) -> std::process::Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    o
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> std::process::Output {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
#[derive(Deserialize)]
struct Case {
    cop: String,
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("fixtures/nil_not_pinned.json")).unwrap()
}
#[test]
fn every_applicable_pinned_call_has_exact_positions_and_messages() {
    let mut counts = [0; 3];
    for c in cases() {
        counts[COPS.iter().position(|&cop| cop == c.cop).unwrap()] += 1;
        let registry = rustocop::cops::Registry::new();
        let config = rustocop::config::ResolvedConfig::default();
        let active =
            rustocop::cops::select_active(&registry, &config, std::slice::from_ref(&c.cop), &[]);
        let result = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(c.source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            result.errors.is_empty() && result.internal_errors.is_empty(),
            "{result:?}"
        );
        let found = result.result.offenses;
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "{}:{} {}",
            c.cop,
            c.spec_line,
            c.source
        );
        assert_eq!(
            found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
            c.messages,
            "{}:{}",
            c.cop,
            c.spec_line
        );
    }
    assert_eq!(counts, [6, 12, 13]);
}
#[test]
fn every_pinned_correction_is_canonical_or_individually_withheld() {
    let d = project();
    let mut counts = [0; 3];
    let mut applied = 0;
    for c in cases().into_iter().filter(|c| c.correction.is_some()) {
        counts[COPS.iter().position(|&cop| cop == c.cop).unwrap()] += 1;
        // All six NilComparison and four NonNilCheck calls have unknown
        // receivers/operators. Not spec line 42 inverts an unknown comparator.
        if c.cop != COPS[2] || c.spec_line == 42 {
            let o = unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", &c.cop, "--fix", "a.rb"],
                1,
            );
            assert!(String::from_utf8_lossy(&o.stdout).contains(&c.cop));
        } else {
            fs::write(d.path().join("a.rb"), &c.source).unwrap();
            cli(d.path(), &["--only", &c.cop, "--fix", "a.rb"], 0);
            let expected = c.correction.unwrap();
            assert_eq!(
                fs::read_to_string(d.path().join("a.rb")).unwrap(),
                expected,
                "{}:{}",
                c.cop,
                c.spec_line
            );
            unchanged(
                d.path(),
                expected.as_bytes(),
                &["--only", &c.cop, "--fix", "a.rb"],
                0,
            );
            applied += 1;
        }
    }
    assert_eq!(counts, [6, 4, 12]);
    assert_eq!(applied, 11);
}
#[test]
fn defaults_selectors_config_excludes_directives_and_no_write() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (cop, source) in COPS
        .into_iter()
        .zip(["nil == nil\n", "false != nil\n", "not test\n"])
    {
        let rule = registry.cops().iter().find(|r| r.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        for except in [cop, "Style"] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--except", except, "--fix", "a.rb"],
                0,
            );
        }
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(d.path().join("a.rb"), source).unwrap();
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .arg("a.rb")
                .output()
                .unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            assert!(!String::from_utf8_lossy(&o.stdout).contains(cop));
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        assert!(
            String::from_utf8_lossy(&unchanged(d.path(), source.as_bytes(), &["a.rb"], 1).stdout)
                .contains(cop)
        );
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nIncludeSemanticChanges=true\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            2,
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
        }
        unchanged(d.path(), b"(\n", &["--only", cop, "--fix", "a.rb"], 2);
        unchanged(d.path(), b"a=1\n", &["--only", cop, "--fix", "a.rb"], 0);
    }
}
#[path = "support/stdin.rs"]
mod stdin_support;
#[test]
fn stdin_reports_and_rejects_fix_without_writes() {
    let d = project();
    let path = d.path().join("a.rb");
    fs::write(&path, b"# sentinel\n").unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    for (cop, source) in COPS
        .into_iter()
        .zip(["x == nil\n", "x != nil\n", "not x\n"])
    {
        for fix in [false, true] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            cmd.current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            if fix {
                cmd.arg("--fix");
            }
            let mut child = cmd.spawn().unwrap();
            stdin_support::write_stdin(child.stdin.take().unwrap(), source.as_bytes(), fix)
                .unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }), "{o:?}");
            assert_eq!(fs::read(&path).unwrap(), b"# sentinel\n");
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
        }
    }
}
#[test]
fn source_envelopes_preserve_bom_crlf_line_and_data_bytes() {
    let d = project();
    for (cop, before, after) in [
        (COPS[0], "nil == nil", "nil.nil?"),
        (COPS[1], "false != nil", "!false.nil?"),
        (COPS[2], "not test", "!test"),
    ] {
        for (prefix, suffix) in [
            ("\u{feff}", "\r\n"),
            ("# coding: us-ascii\r\n", "\r\n__LINE__\r\n"),
            (
                "# coding: windows-1252\n# café\n",
                "\n__END__\nraw\0data\r\n",
            ),
        ] {
            let source = format!("{prefix}{before}{suffix}");
            let expected = format!("{prefix}{after}{suffix}");
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
            cli(d.path(), &["--only", cop, "--fix", "a.rb"], 0);
            assert_eq!(
                fs::read(d.path().join("a.rb")).unwrap(),
                expected.as_bytes()
            );
            unchanged(
                d.path(),
                expected.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for suffix in ["\nFile.read(__FILE__)\n", "\np DATA.pos\n__END__\ndata"] {
            unchanged(
                d.path(),
                format!("{before}{suffix}").as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                1,
            );
        }
    }
    for (cop, source) in [
        (COPS[0], "nil == # keep\n nil\np __LINE__\n"),
        (COPS[1], "false != # keep\n nil\np __LINE__\n"),
        (COPS[2], "not # keep\n x\np __LINE__\n"),
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn safe_not_edits_preserve_custom_negation_truthiness_calls_and_control_flow() {
    let d = project();
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(p) = std::env::var("RUSTOCOP_RUBY34") {
        rubies.push(p);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let run = |source: &str| {
            let o = Command::new(&ruby)
                .args(["--disable-gems", "-e", source])
                .output()
                .unwrap();
            assert!(o.status.success(), "{source}: {o:?}");
            o.stdout
        };
        for source in [
            "$log=[]; def mark(x); $log << x; x; end; result=(not mark(false) && mark(:truthy)); p [result,$log]",
            "$log=[]; def mark(x); $log << x; x; end; result=(not mark(:truthy) || mark(false)); p [result,$log]",
            "$log=[]; def choose(x); $log << x; x; end; result=(not choose :truthy); p [result,$log]",
            "a=nil; result=(not a = false); p [a,result,__LINE__]",
            "class X; def !; :not_boolean; end; end; x=X.new; result=(not x); p result",
            "class X; def ~; :truthy; end; end; x=X.new; result=(not ~x); p result",
            "a=false; result=(not a ? :one : :two); p result",
            "1.upto(6) { |n| $.=n; puts n if not 1..5 }",
            "x=0; [false,true].each { |b| x+=1 unless not(b) }; p x",
            "$log=[]; def check; [1,2,3].each { |x| $log << x if not x.even? }; end; check; p $log",
        ] {
            fs::write(d.path().join("a.rb"), source).unwrap();
            cli(d.path(), &["--only", COPS[2], "--fix", "a.rb"], 0);
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            assert_ne!(after, source);
            assert_eq!(run(source), run(&after));
        }
        for (cop, before, after) in [
            (COPS[0], "nil == nil", "nil.nil?"),
            (COPS[0], "-1 == nil", "(-1).nil?"),
            (COPS[1], "false != nil", "!false.nil?"),
            (COPS[1], "-1 != nil", "!(-1).nil?"),
            (COPS[1], "1.!=(nil).to_s", "(!1.nil?).to_s"),
            (COPS[2], "not 1 < 2", "1 >= 2"),
        ] {
            // The proof is deliberately exercised before any unknown p call.
            let source = format!("result=({before}); p [result,__LINE__]");
            fs::write(d.path().join("a.rb"), &source).unwrap();
            cli(d.path(), &["--only", cop, "--fix", "a.rb"], 0);
            let fixed = fs::read_to_string(d.path().join("a.rb")).unwrap();
            assert_eq!(fixed, format!("result=({after}); p [result,__LINE__]"));
            assert_eq!(run(&source), run(&fixed));
        }
    }
}
fn assert_not_grouping_corrections(cases: &[(&str, &str)]) {
    let d = project();
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(path) = std::env::var("RUSTOCOP_RUBY34") {
        rubies.push(path);
    }
    for &(source, expected) in cases {
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--only", COPS[2], "--fix", "a.rb"], 0);
        let fixed = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert_eq!(fixed, expected);
        for ruby in &rubies {
            if Command::new(ruby).arg("--version").output().is_err() {
                continue;
            }
            let run = |s: &str| {
                let o = Command::new(ruby)
                    .args(["--disable-gems", "-e", s])
                    .output()
                    .unwrap();
                assert!(o.status.success(), "{ruby}: {s}: {o:?}");
                o.stdout
            };
            assert_eq!(run(source), run(&fixed), "{ruby}: {source}");
        }
        unchanged(
            d.path(),
            fixed.as_bytes(),
            &["--only", COPS[2], "--fix", "a.rb"],
            0,
        );
    }
}
#[test]
fn not_pattern_matches_preserve_results_and_captures() {
    assert_not_grouping_corrections(&[
        (
            "result=(not 1 in String); p result",
            "result=(!(1 in String)); p result",
        ),
        (
            "result=(not [1] in [x]); p [result,x]",
            "result=(!([1] in [x])); p [result,x]",
        ),
        (
            "result=(not 1 in Integer => x); p [result,x]",
            "result=(!(1 in Integer => x)); p [result,x]",
        ),
        (
            "result=(not(1 in String)); p result",
            "result=(!(1 in String)); p result",
        ),
    ]);
}
#[test]
fn nested_not_comparison_fixes_compose_in_one_pass() {
    for (operator, inverse) in [
        ("==", "!="),
        ("!=", "=="),
        ("<", ">="),
        (">=", "<"),
        (">", "<="),
        ("<=", ">"),
    ] {
        assert_not_grouping_corrections(&[
            (
                &format!("result=(not not 1 {operator} 2); p result"),
                &format!("result=(!(1 {inverse} 2)); p result"),
            ),
            (
                &format!("result=(not not not 1 {operator} 2); p result"),
                &format!("result=(!!(1 {inverse} 2)); p result"),
            ),
        ]);
    }
}
#[test]
fn unsafe_comparison_rewrites_have_observable_counterexamples_and_retain_bytes() {
    let d = project();
    let run = |source: &str| {
        let o = Command::new("ruby")
            .args(["--disable-gems", "-e", source])
            .output()
            .unwrap();
        assert!(o.status.success(), "{o:?}");
        o.stdout
    };
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    for (cop, setup, before, unsafe_after) in [
        (
            COPS[0],
            "class X; def ==(x); :truthy; end; def nil?; false; end; end; x=X.new;",
            "x == nil",
            "x.nil?",
        ),
        (
            COPS[0],
            "class X; def ===(x); true; end; def nil?; false; end; end; x=X.new;",
            "x === nil",
            "x.nil?",
        ),
        (
            COPS[1],
            "class X; def !=(x); :truthy; end; def nil?; true; end; end; x=X.new;",
            "x != nil",
            "!x.nil?",
        ),
        (COPS[1], "x=false;", "x != nil", "x"),
        (
            COPS[2],
            "class X; def <(x); false; end; def >=(x); false; end; end; x=X.new;",
            "not x < 1",
            "x >= 1",
        ),
        (COPS[2], "x=0.0/0;", "not x < 1", "x >= 1"),
        (
            COPS[0],
            "$n=0; def read; $n+=1; Object.new; end;",
            "read == nil",
            "read",
        ),
    ] {
        let source = format!("{setup} result=({before}); p result");
        let unsafe_source = format!("{setup} result=({unsafe_after}); p result");
        assert_ne!(run(&source), run(&unsafe_source));
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn curated_style_layout_composition_stabilizes_without_cycles() {
    let d = project();
    for only in [
        Some("Style/NilComparison,Style/NonNilCheck,Style/Not"),
        Some(
            "Style/NilComparison,Style/NonNilCheck,Style/Not,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        fs::write(
            d.path().join("a.rb"),
            "nil == nil\nfalse != nil\nnot test\n",
        )
        .unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(only) = only {
                args.extend(["--only", only]);
            }
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            if fs::read(d.path().join("a.rb")).unwrap() == before {
                stable = true;
                break;
            }
        }
        assert!(stable);
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(
            !after.contains(" == nil") && !after.contains(" != nil") && !after.contains("not test"),
            "{after}"
        );
    }
}
