//! fixed unsafe detector; no Ruby rewrite is authorized.
#[path = "support/stdin.rs"]
mod stdin;

use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/HashExcept";
const SOURCE: &str = "hash.reject { |k, v| k == :bar }\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
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
fn unchanged(dir: &Path, s: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, s).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), s);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
#[test]
fn registry_default_on_minimum_ruby_and_detection_only_pipeline() {
    use rustocop::cops::policy::{FIXED_RUBY_VERSION, RubyVersion};
    let r = rustocop::cops::Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled());
    assert!(!c.safe_fixes());
    assert_eq!(c.ruby_versions().minimum, Some(RubyVersion(3, 0)));
    assert_eq!(c.ruby_versions().maximum, None);
    assert_eq!(FIXED_RUBY_VERSION, RubyVersion(3, 4));
    assert!(c.path_policy().is_none());
    let report = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(SOURCE.as_bytes()),
        &[c.as_ref()],
        &Default::default(),
    );
    assert!(report.internal_errors.is_empty());
    assert!(report.errors.is_empty());
    assert_eq!(report.result.offenses.len(), 1);
    assert!(report.result.offenses[0].correction.is_none());
    let d = project();
    let o = unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
    assert!(String::from_utf8_lossy(&o.stdout).contains(COP));
}
#[test]
fn selectors_config_excludes_and_strict_no_new_knobs() {
    let d = project();
    for args in [
        vec!["--only", COP, "a.rb"],
        vec!["--only", COP, "--fix", "a.rb"],
        vec!["--fix", "a.rb"],
    ] {
        unchanged(d.path(), SOURCE.as_bytes(), &args, 1);
    }
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled = false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 0);
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 1);
    for config in [
        "exclude = [\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    for knob in [
        "ActiveSupportExtensionsEnabled = true",
        "SafeAutoCorrect = true",
        "TargetRubyVersion = 2.7",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{knob}\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 2);
    }
}
#[test]
fn directives_clean_error_and_stdin_no_write() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for target in [COP, "HashExcept", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                format!("{} # {prefix}:disable {target}\n", SOURCE.trim_end()),
            ] {
                unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 0);
            }
        }
        let s = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        let o = unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        assert_eq!(
            String::from_utf8_lossy(&o.stdout)
                .matches(&format!("({COP})"))
                .count(),
            1
        );
    }
    for (s, code) in [("hash.except(:bar)\n", 0), ("(\n", 2)] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
    for fix in [false, true] {
        let p = d.path().join("a.rb");
        fs::write(&p, "disk bytes\n").unwrap();
        let time = fs::metadata(&p).unwrap().modified().unwrap();
        let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
        c.current_dir(d.path())
            .args(["--only", COP, "--stdin", "a.rb"]);
        if fix {
            c.arg("--fix");
        }
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        stdin::write_stdin(child.stdin.take().unwrap(), SOURCE.as_bytes(), fix).unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }), "{o:?}");
        if fix {
            assert!(o.stdout.is_empty(), "{o:?}");
            assert!(
                String::from_utf8_lossy(&o.stderr)
                    .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                "{o:?}"
            );
        } else {
            assert!(o.stderr.is_empty(), "{o:?}");
            assert!(String::from_utf8_lossy(&o.stdout).contains(COP), "{o:?}");
        }
        assert_eq!(fs::read(&p).unwrap(), b"disk bytes\n");
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    }
}
#[test]
fn all_13_applicable_pinned_correction_calls_are_never_edits() {
    // RuboCop style/hash_except_spec.rb, MIT, (c) 2012-26 Bozhidar Batsov.
    // Correction-call lines: 18,29,40,51,75,131,142,153,164,175,186,197,208.
    let cases = [
        (
            "{foo: 1, bar: 2, baz: 3}.select { |k,v| !%i[foo bar].include?(k) }",
            ":foo, :bar",
        ),
        (
            "{foo: 1, bar: 2, baz: 3}.filter { |k,v| ![:foo, :bar].include?(k) }",
            ":foo, :bar",
        ),
        (
            "{foo: 1, bar: 2, baz: 3}.reject { |k,v| %I[#{foo} bar].include?(k) }",
            ":\"#{foo}\", :bar",
        ),
        (
            "{foo: 1, bar: 2, baz: 3}.reject { |k,v| %W[#{foo} bar].include?(k) }",
            "\"#{foo}\", 'bar'",
        ),
        (
            "{foo: 1, bar: 2, baz: 3}.reject { |k,v| [:bar].include?(k) }",
            ":bar",
        ),
        (
            "{foo: 1, bar: 2, baz: 3}.reject { |k,v| k == :bar }",
            ":bar",
        ),
        (
            "{foo: 1, bar: 2, baz: 3}&.reject { |k,v| k == :bar }",
            ":bar",
        ),
        (
            "{foo: 1, bar: 2, baz: 3}.reject { |k,v| :bar == k }",
            ":bar",
        ),
        (
            "{foo: 1, bar: 2, baz: 3}.select { |k,v| k != :bar }",
            ":bar",
        ),
        (
            "{foo: 1, bar: 2, baz: 3}.select { |k,v| :bar != k }",
            ":bar",
        ),
        ("hash.reject { |k,v| k == 'str' }", "'str'"),
        ("hash.reject { |k,v| k.eql?(0.0) }", "0.0"),
        (
            "{foo: 1, bar: 2, baz: 3}.filter { |k,v| k != :bar }",
            ":bar",
        ),
    ];
    assert_eq!(cases.len(), 13);
    let d = project();
    for (s, key) in cases {
        let o = unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        assert!(
            String::from_utf8_lossy(&o.stdout).contains(&format!("Use `except({key})` instead.")),
            "{o:?}"
        );
    }
}
#[test]
fn argument_modes_dynamic_keys_unknown_receivers_and_sources_never_rewrite() {
    let d = project();
    for (s, code) in [
        ("unknown.reject { |k,v| k.eql?(effect()) }", 1),
        (
            "Hash.new { |h,k| effect(k) }.select { |k,v| k != :missing }",
            1,
        ),
        ("hash.reject { |k,v| k.eql?(a: effect()) }", 1),
        ("hash.reject { |k,v| k.eql?({a: effect()}) }", 1),
        ("hash.reject { |k,v| k.eql?(&effect()) }", 1),
        ("hash.reject { |k,v| keys.include?(k) }", 1),
        ("hash.reject(kw: 1) { |k,v| k == :bar }", 0),
        ("hash.reject({kw: 1}) { |k,v| k == :bar }", 0),
        ("hash.reject(&effect())", 0),
        ("hash.reject { |k,v| k == :a || k == :b }", 0),
        ("hash.reject { _1 == :bar }", 0),
        ("hash.reject { it == :bar }", 0),
        ("hash.reject { |k,v:| k == :bar }", 0),
        ("hash.reject { |k,v| k == `never execute` }", 0),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
}
#[test]
fn source_envelopes_and_original_positions_preserve_bytes_and_mtime() {
    let d = project();
    for s in [
        "\u{feff}# encoding: UTF-8\r\nhash.reject do |k,v|\r\n k.eql?(__LINE__)\r\nend\r\np DATA.read\r\n__END__\r\nraw\0\r\n".as_bytes().to_vec(),
        b"# encoding: ISO-8859-1\r\nhash.reject { |k,v| k == 'caf\xe9' }\r\n".to_vec(),
        b"hash.reject { |k,v| k.eql?(<<~KEY) }\n#{effect()}\nKEY\n".to_vec(),
        b"consume(<<~TEXT)\n#{hash.reject { |k,v| k == :bar }}\nTEXT\n".to_vec(),
        "é = 1; hash.reject { |clé,v| clé == :bar }\n".as_bytes().to_vec(),
    ] { unchanged(d.path(),&s,&["--only",COP,"--fix","a.rb"],1); }
    let o = unchanged(
        d.path(),
        "é = 1; hash.reject { |k,v| k == :bar }\n".as_bytes(),
        &["--only", COP, "a.rb"],
        1,
    );
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("a.rb:1:13:"),
        "{o:?}"
    );
}
#[test]
fn bounded_wide_and_nested_patterns_use_the_main_dispatch_only() {
    let mut s = String::new();
    for _ in 0..6000 {
        s.push_str("hash.reject { |k,v| [:a, effect()].include?(k) }\n");
        s.push_str("hash.select { |k,v| k != :bar && k != :baz }\n");
    }
    // Nested candidates also occur as dynamic keys of outer candidates.
    let nested = (0..8).fold(":bar".to_string(), |inner, _| {
        format!("hash.reject {{ |k,v| k.eql?({inner}) }}")
    });
    s.push_str(&nested);
    let r = rustocop::cops::Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    let report = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(s.as_bytes()),
        &[c.as_ref()],
        &Default::default(),
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.internal_errors.is_empty());
    assert_eq!(report.result.offenses.len(), 6008);
    assert!(
        report
            .result
            .offenses
            .iter()
            .all(|o| o.correction.is_none())
    );
}
#[test]
fn controlled_runtime_explains_evaluation_missing_keys_defaults_and_dispatch_risks() {
    // Only ordinary fixtures we own; commands and application code never run.
    let source = r#"
trace = []
hash = Hash.new { |h,k| trace << [:default,k]; :fallback }
hash[:a] = 1; hash[:b] = 2; hash[:c] = 3
original = hash.reject { |k,v| (trace << :keys; [:missing,:b]).include?(k) }
raise unless original.keys == [:a,:c] && hash.keys == [:a,:b,:c]
raise unless trace == [:keys,:keys,:keys]
trace.clear
replacement = hash.except(*(trace << :keys; [:missing,:b]))
raise unless replacement.keys == [:a,:c] && trace == [:keys]
raise unless hash[:absent] == :fallback && trace.last == [:default,:absent]
# Eql? accepts dynamic values but its suggested splat is not scalar equality.
hash = {[:a,:b] => 1, :a => 2, :b => 3}
raise unless hash.reject { |k,v| k.eql?([:a,:b]) }.values == [2,3]
raise unless hash.except(:a,:b).values == [1]
# Receiver identity is deliberately not inferred by the unsafe detector.
class Lookalike
  def reject; yield(:bar,1); :original; end
  def except(*); :different; end
end
raise unless Lookalike.new.reject { |k,v| k == :bar } == :original
raise unless Lookalike.new.except(:bar) == :different
puts :ok
"#;
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
        let o = Command::new(&ruby)
            .args(["-e", "raise unless RUBY_VERSION.start_with?('3.4.')"])
            .output()
            .unwrap();
        assert!(o.status.success(), "{o:?}");
        rubies.push(ruby);
    } else {
        eprintln!("SKIP native Ruby 3.4 runtime; set RUSTOCOP_RUBY34");
    }
    for ruby in rubies {
        let Ok(o) = Command::new(&ruby).args(["-e", source]).output() else {
            continue;
        };
        assert!(o.status.success(), "{ruby}: {o:?}");
        assert_eq!(o.stdout, b"ok\n");
    }
    let d = project();
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
}
