//! real CLI policy, pinned corrections and controlled Ruby semantics.
//! Fixtures port RuboCop specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use crate::support::cli::project;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const EMPTY: &str = "Style/EmptyMethod";
const BASE: &str = "Style/RedundantConstantBase";
const FETCH: &str = "Style/RedundantFetchBlock";
const CASES: [(&str, &str); 3] = [
    (EMPTY, "def f\nend\n"),
    (BASE, "::A\n"),
    (FETCH, "hash.fetch(:key) { false }\n"),
];
fn cli(dir: &Path, args: &[&str], code: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    assert!(o.stderr.is_empty() || code == 2, "{o:?}");
    o
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
    o
}
fn fixed(dir: &Path, cop: &str, before: &[u8], after: &[u8]) {
    unchanged(dir, before, &["--only", cop, "a.rb"], 1);
    cli(dir, &["--only", cop, "--fix", "a.rb"], 0);
    assert_eq!(fs::read(dir.join("a.rb")).unwrap(), after);
    unchanged(dir, after, &["--only", cop, "--fix", "a.rb"], 0);
}
#[test]
fn exact_registry_defaults_and_safe_capabilities() {
    let registry = rustocop::cops::Registry::new();
    for (name, _) in CASES {
        let cops: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == name)
            .collect();
        assert_eq!(cops.len(), 1);
        assert!(cops[0].default_enabled());
        assert_eq!(cops[0].safe_fixes(), name != FETCH);
        assert!(cops[0].path_policy().is_none());
        assert_eq!(cops[0].ruby_versions(), Default::default());
    }
}
#[test]
fn defaults_selectors_config_excludes_and_no_write() {
    let d = project();
    for (cop, source) in CASES {
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled = false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "exclude = [\"*.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"*.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for setting in [
            "EnforcedStyle = \"expanded\"",
            "SafeForConstants = true",
            "TargetRubyVersion = 4.0",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{setting}\n"),
            )
            .unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn both_directive_prefixes_scopes_source_enabling_clean_and_error_files() {
    let d = project();
    for (cop, source) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                for kind in ["disable", "disable-next"] {
                    let source = format!("# {prefix}:{kind} {target}\n{source}");
                    unchanged(
                        d.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "a.rb"],
                        0,
                    );
                }
                let inline = if cop == EMPTY {
                    format!("def f # {prefix}:disable {target}\nend\n")
                } else {
                    format!("{} # {prefix}:disable {target}\n", source.trim_end())
                };
                unchanged(
                    d.path(),
                    inline.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            let source =
                format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            let o = unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        )
        .unwrap();
        let enabled = format!("# rustocop:enable {cop}\n{source}");
        unchanged(d.path(), enabled.as_bytes(), &["a.rb"], 1);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        unchanged(d.path(), b"nil\n", &["--only", cop, "--fix", "a.rb"], 0);
        unchanged(d.path(), b"(\n", &["--only", cop, "--fix", "a.rb"], 2);
    }
}
#[test]
fn stdin_is_report_only_and_fix_is_rejected() {
    let d = project();
    for (cop, source) in CASES {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        cli(d.path(), &["--only", cop, "--fix", "--stdin", "a.rb"], 2);
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
    }
}
#[test]
fn line_length_dependency_uses_config_not_selection() {
    let d = project();
    let source = "def long_name(a, b, c)\nend\n";
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Layout/LineLength\"]\nmax = 20\n",
    )
    .unwrap();
    unchanged(
        d.path(),
        source.as_bytes(),
        &[
            "--only",
            EMPTY,
            "--except",
            "Layout/LineLength",
            "--fix",
            "a.rb",
        ],
        1,
    );
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Layout/LineLength\"]\nenabled = false\nmax = 20\n",
    )
    .unwrap();
    fixed(
        d.path(),
        EMPTY,
        source.as_bytes(),
        b"def long_name(a, b, c); end\n",
    );
}
fn rubies() -> Vec<String> {
    ["ruby", "/opt/ruby34/bin/ruby"]
        .into_iter()
        .filter(|r| Command::new(r).arg("--version").output().is_ok())
        .map(str::to_owned)
        .collect()
}
fn runtime(ruby: &str, cop: &str, source: &str, should_change: bool) {
    let d = project();
    let path = d.path().join("a.rb");
    fs::write(&path, source).unwrap();
    let run = || {
        Command::new(ruby)
            .current_dir(d.path())
            .arg("a.rb")
            .output()
            .unwrap()
    };
    let before = run();
    cli(
        d.path(),
        &["--only", cop, "--fix", "a.rb"],
        if should_change { 0 } else { 1 },
    );
    assert_eq!(
        fs::read_to_string(path).unwrap() != source,
        should_change,
        "{source}"
    );
    let after = run();
    assert_eq!(before.status.code(), after.status.code(), "{source}");
    assert_eq!(before.stdout, after.stdout, "{source}");
    assert_eq!(before.stderr, after.stderr, "{source}");
}
#[test]
fn runtime_unsafe_fetch_overrides_nil_false_mutability_and_short_circuiting_never_change() {
    for ruby in rubies() {
        runtime(
            &ruby,
            FETCH,
            "$events = []\nclass Custom\n def fetch(key, *defaults); $events << [key, defaults]; block_given? ? yield : :different; end\nend\np Custom.new.fetch(:x) { false }\np $events\np nil&.fetch(:x) { 5 }\np [{a: false}.fetch(:a) { nil }, {a: nil}.fetch(:a) { false }]\n",
            false,
        );
        runtime(
            &ruby,
            FETCH,
            "# frozen_string_literal: true\nh = {}\np [h.fetch(:x) { 'value' }.frozen?, h.fetch(:x) { 'value' }, h.fetch(:x) {}]\n",
            false,
        );
        runtime(
            &ruby,
            FETCH,
            "$calls = 0\ndef build; $calls += 1; []; end\nh = {x: nil}\np h.fetch(:x) { build }\np h.fetch(:x) { [] }\np $calls\np h.fetch(:x) { false }\n",
            false,
        );
    }
}
#[test]
fn selected_style_and_layout_corrections_stabilize_within_three_explicit_passes() {
    let d = project();
    let path = d.path().join("a.rb");
    let source = "def f(a = 'x')\nend\n::A;\n";
    fs::write(&path, source).unwrap();
    let selection = "Style/EmptyMethod,Style/RedundantConstantBase,Style/MethodDefParentheses,Style/StringLiterals,Style/Semicolon,Style/RedundantReturn,Style/HashSyntax,Style/SignalException,Layout/TrailingWhitespace,Layout/EmptyLines";
    for _ in 0..3 {
        let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", selection, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
    }
    let after = fs::read(&path).unwrap();
    unchanged(d.path(), &after, &["--only", selection, "--fix", "a.rb"], 0);
    assert_eq!(
        String::from_utf8(after).unwrap(),
        "def f(a = \"x\"); end\n\nA\n"
    );
}
