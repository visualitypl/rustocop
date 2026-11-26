//! Pinned RuboCop style/{unless_logical_operators,yoda_condition,
//! yoda_expression}_spec.rb examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use crate::support::cli::project;
use serde::Deserialize;
use std::{fs, path::Path, process::Command};
const COPS: [&str; 3] = [
    "Style/UnlessLogicalOperators",
    "Style/YodaCondition",
    "Style/YodaExpression",
];
const SOURCES: [&str; 3] = ["work unless a || b && c\n", "1 == value\n", "1 + value\n"];
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
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
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
    serde_json::from_str(include_str!("fixtures/unless_yoda_pinned.json")).unwrap()
}
fn lint(cop: &str, source: &str) -> Vec<rustocop::offense::Offense> {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
    let r = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(source.as_bytes()),
        &active,
        &config,
    );
    assert!(r.errors.is_empty() && r.internal_errors.is_empty(), "{r:?}");
    r.result.offenses
}
#[test]
fn all_applicable_pinned_calls_have_exact_positions_messages_and_no_edits() {
    let mut counts = [0; 3];
    for c in cases() {
        counts[COPS.iter().position(|&cop| cop == c.cop).unwrap()] += 1;
        let o = lint(&c.cop, &c.source);
        assert_eq!(
            o.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "{}:{} {}",
            c.cop,
            c.spec_line,
            c.source
        );
        assert_eq!(
            o.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
            c.messages,
            "{}:{}",
            c.cop,
            c.spec_line
        );
        assert!(o.iter().all(|o| o.correction.is_none()));
    }
    assert_eq!(counts, [20, 39, 14]);
}
#[test]
fn every_unsafe_pinned_correction_retains_bytes_mtime_and_offenses() {
    let d = project();
    let mut counts = [0; 3];
    for c in cases().into_iter().filter(|c| c.correction.is_some()) {
        counts[COPS.iter().position(|&cop| cop == c.cop).unwrap()] += 1;
        let o = unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", &c.cop, "--fix", "a.rb"],
            1,
        );
        assert!(
            String::from_utf8_lossy(&o.stdout).contains(&c.cop),
            "{}:{}",
            c.cop,
            c.spec_line
        );
    }
    assert_eq!(counts, [0, 13, 7]);
}
#[test]
fn defaults_selectors_config_excludes_directives_and_no_write() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (i, (cop, source)) in COPS.into_iter().zip(SOURCES).enumerate() {
        let registered: Vec<_> = registry.cops().iter().filter(|r| r.name() == cop).collect();
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].default_enabled(), i == 1);
        assert!(!registered[0].safe_fixes());
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .arg("a.rb")
            .output()
            .unwrap();
        assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
        assert_eq!(String::from_utf8_lossy(&o.stdout).contains(cop), i == 1);
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
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .arg("a.rb")
                .output()
                .unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            assert!(!String::from_utf8_lossy(&o.stdout).contains(cop));
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                1,
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=true\nexclude=[\"a.rb\"]\n"),
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
            format!("[cops.\"{cop}\"]\nEnforcedStyle=\"other\"\n"),
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
use crate::support::stdin as stdin_support;
#[test]
fn stdin_never_writes_and_fix_is_rejected() {
    let d = project();
    fs::write(d.path().join("a.rb"), b"# sentinel\n").unwrap();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
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
            if fix {
                assert!(o.stdout.is_empty(), "{o:?}");
                assert!(
                    String::from_utf8_lossy(&o.stderr)
                        .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                    "{o:?}"
                );
            }
            assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"# sentinel\n");
        }
    }
}
#[test]
fn detection_only_preserves_source_envelopes_and_line_positions() {
    let d = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for (prefix, suffix) in [
            ("\u{feff}", "\r\n"),
            ("# coding: us-ascii\r\n", "\r\np __LINE__\r\n"),
            (
                "# coding: windows-1252\n# café\n",
                "\n__END__\nraw\0data\r\n",
            ),
        ] {
            let s = format!("{prefix}{}{suffix}", source.trim_end());
            assert_eq!(lint(cop, &s).len(), 1);
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
        }
    }
}
#[test]
fn truthiness_dispatch_short_circuit_side_effects_and_assignment_binding_are_not_rewritten() {
    let d = project();
    let run = |ruby: &str, source: &str| {
        let o = Command::new(ruby)
            .args(["--disable-gems", "-e", source])
            .output()
            .unwrap();
        assert!(o.status.success(), "{source}: {o:?}");
        o.stdout
    };
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        for (cop, before, unsafe_after) in [
            (
                COPS[1],
                "class X; def ==(x); :truthy; end; end; x=X.new; p(1 == x)",
                "class X; def ==(x); :truthy; end; end; x=X.new; p(x == 1)",
            ),
            (
                COPS[1],
                "class X; def <(x); :truthy; end; def >(x); nil; end; end; X_CONST=X.new; p(X_CONST < X.new)",
                "class X; def <(x); :truthy; end; def >(x); nil; end; end; X_CONST=X.new; p(X.new > X_CONST)",
            ),
            (
                COPS[2],
                "class Integer; def +(x); :left; end; end; class X; def +(x); :right; end; end; p(1 + X.new)",
                "class Integer; def +(x); :left; end; end; class X; def +(x); :right; end; end; p(X.new + 1)",
            ),
            (
                COPS[2],
                "$events=[]; module NS; C=1; end; def ns; $events << :constant; NS; end; def read; $events << :rhs; 2; end; p(ns::C + read); p $events",
                "$events=[]; module NS; C=1; end; def ns; $events << :constant; NS; end; def read; $events << :rhs; 2; end; p(read + ns::C); p $events",
            ),
        ] {
            assert_ne!(run(&ruby, before), run(&ruby, unsafe_after));
            assert!(!lint(cop, before).is_empty());
            unchanged(
                d.path(),
                before.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                1,
            );
            assert_eq!(
                run(&ruby, before),
                run(&ruby, &fs::read_to_string(d.path().join("a.rb")).unwrap())
            );
        }
        for (source, count) in [
            (
                "$events=[]; def probe(x); $events << x; x; end; result=:unset; result=:body unless probe(nil) || probe(:truthy) && probe(false); p [result,$events]",
                1,
            ),
            (
                "$events=[]; def probe(x); $events << x; x; end; result=:unset; result=:body unless (assigned=probe(:truthy)) && probe(nil) or probe(false); p [result,assigned,$events]",
                1,
            ),
            // Mixed kinds nested under a call are intentionally not a logical
            // root; do not reinterpret command-call/assignment precedence.
            (
                "$events=[]; def take(x); $events << x; x; end; result=:body unless take a = false || nil && :truthy; p [result,a,$events]",
                0,
            ),
        ] {
            assert_eq!(lint(COPS[0], source).len(), count, "{source}");
            let before = run(&ruby, source);
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COPS[0], "--fix", "a.rb"],
                i32::from(count != 0),
            );
            assert_eq!(
                before,
                run(&ruby, &fs::read_to_string(d.path().join("a.rb")).unwrap())
            );
        }
    }
}
#[test]
fn many_siblings_have_bounded_predicate_operand_and_ancestor_work() {
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let s = source.repeat(6000);
        let o = lint(cop, &s);
        assert_eq!(o.len(), 6000);
        assert!(o.iter().all(|o| o.correction.is_none()));
        assert_eq!(o.last().unwrap().line, 6000);
    }
}
