//! Ported pinned RuboCop style/{and_or,case_equality,combinable_defined}
//! specs (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
use crate::support::cli::project;
use serde::Deserialize;
use std::{fs, path::Path, process::Command};
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
use crate::support::stdin as stdin_support;
const COPS: [&str; 3] = [
    "Style/AndOr",
    "Style/CaseEquality",
    "Style/CombinableDefined",
];
#[derive(Deserialize)]
struct Case {
    cop: String,
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
}
fn lint(cop: &str, s: &str) -> Vec<rustocop::offense::Offense> {
    let r = rustocop::cops::Registry::new();
    let cfg = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&r, &cfg, &[cop.into()], &[]);
    let result = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(s.as_bytes()),
        &active,
        &cfg,
    );
    assert!(
        result.errors.is_empty() && result.internal_errors.is_empty(),
        "{s}: {result:?}"
    );
    result.result.offenses
}
#[test]
fn every_pinned_detection_call_has_exact_positions_and_messages() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/boolean_predicates_pinned.json")).unwrap();
    let mut counts = [0; 3];
    for c in cases {
        counts[COPS.iter().position(|&cop| cop == c.cop).unwrap()] += 1;
        let found = lint(&c.cop, &c.source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "{}:{}: {}",
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
    assert_eq!(counts, [38, 25, 39]);
}
#[test]
fn default_selectors_enabling_excludes_and_directives_use_real_cli() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (i, source) in [
        "work if a and b\n",
        "(1..10) === 7\n",
        "module Foo; Bar=1; end; defined?(Foo) && defined?(Foo::Bar)\n",
    ]
    .into_iter()
    .enumerate()
    {
        let cop = COPS[i];
        let rule = registry.cops().iter().find(|r| r.name() == cop).unwrap();
        assert!(rule.default_enabled());
        assert_eq!(rule.safe_fixes(), i != 0);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let out = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
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
            // Other default cops can still report the lookalike source.
            fs::write(d.path().join("a.rb"), source).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .arg("a.rb")
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), source.as_bytes());
            assert!(!String::from_utf8_lossy(&out.stdout).contains(cop));
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        let out = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
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
    }
}
#[test]
fn parentheses_semicolon_and_layout_peers_preserve_single_pass_binding() {
    let d = project();
    let source = "result=((1..10)===7); result\n";
    let only =
        "Style/CaseEquality,Style/RedundantParentheses,Style/Semicolon,Layout/SpaceAroundOperators";
    fs::write(d.path().join("a.rb"), source).unwrap();
    for _ in 0..3 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", only, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert!(matches!(output.status.code(), Some(0 | 1)), "{output:?}");
    }
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_eq!(after, "result = (1..10).include?(7)\n result\n");
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", only, "--fix", "a.rb"],
        0,
    );
    for ruby in rubies() {
        for s in [source, &after] {
            fs::write(d.path().join("a.rb"), format!("{s}p result\n")).unwrap();
            assert_eq!(observe(&ruby, d.path()), b"true\n");
        }
    }
}
#[test]
fn stdin_never_writes_and_fix_is_explicitly_rejected() {
    let d = project();
    let p = d.path().join("a.rb");
    fs::write(&p, "# on-disk sentinel\n").unwrap();
    let mtime = fs::metadata(&p).unwrap().modified().unwrap();
    for (cop, source) in COPS.into_iter().zip([
        "work if a and b\n",
        "(1..10) === 7\n",
        "defined?(Foo) && defined?(Foo::Bar)\n",
    ]) {
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
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.status.code(), Some(if fix { 2 } else { 1 }), "{out:?}");
            if fix {
                assert!(String::from_utf8_lossy(&out.stderr).contains("--fix"));
            } else {
                assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
            }
            assert_eq!(fs::read(&p).unwrap(), b"# on-disk sentinel\n");
            assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), mtime);
        }
    }
}
fn rubies() -> Vec<String> {
    let mut r = Vec::new();
    if Command::new("ruby").arg("--version").output().is_ok() {
        r.push("ruby".into());
    }
    if let Ok(p) = crate::support::runtime::ruby() {
        r.push(p);
    }
    r
}
fn observe(ruby: &str, dir: &Path) -> Vec<u8> {
    let o = Command::new(ruby)
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap();
    assert!(o.status.success(), "{ruby}: {o:?}");
    o.stdout
}
#[test]
fn safe_rewrites_preserve_real_values_not_just_parse_success() {
    let d = project();
    for (cop, source, expected) in [
        (
            COPS[1],
            "p [(1..10) === 7, (1..10) === -7, Array === [], Integer === -7, NilClass === nil]\n",
            "[true, false, true, true, true]\n",
        ),
        (
            COPS[2],
            "module Foo; module Bar; Baz=1; end; Quux=2; end; p [defined?(Foo) && defined?(Foo::Bar) && defined?(Foo::Bar::Baz), (defined?(Foo::Bar) and defined?(Foo)), defined?(Foo) && defined?(Foo::Bar) && defined?(Foo::Quux)]\n",
            "[\"constant\", \"constant\", \"constant\"]\n",
        ),
    ] {
        for ruby in rubies() {
            fs::write(d.path().join("a.rb"), source).unwrap();
            let before = observe(&ruby, d.path());
            assert_eq!(before, expected.as_bytes());
            cli(d.path(), &["--only", cop, "--fix", "a.rb"], 0);
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            assert_ne!(after, source);
            assert_eq!(observe(&ruby, d.path()), before);
        }
    }
}
