//! keyword/hash identity and safe atomic brace removal through the CLI.
use crate::support::cli::project;
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const BRACES: &str = "Style/RedundantDoubleSplatHashBraces";
const KEYS: &str = "Style/StringHashKeys";
const SELECT: &str = "Style/RedundantDoubleSplatHashBraces,Style/StringHashKeys";
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
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
fn fix(dir: &Path, source: &[u8], selector: Option<&str>) -> Vec<u8> {
    fs::write(dir.join("a.rb"), source).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    c.current_dir(dir).args(["--fix", "a.rb"]);
    if let Some(selector) = selector {
        c.args(["--only", selector]);
    }
    let o = c.output().unwrap();
    assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    fs::read(dir.join("a.rb")).unwrap()
}
#[test]
fn real_registry_pipeline_and_default_opt_in_policy() {
    let r = rustocop::cops::Registry::new();
    for (name, enabled, safe, source) in [
        (BRACES, true, true, "f(**{a: 1})"),
        (KEYS, false, false, "{'a'=>1}"),
    ] {
        let cop = r.cops().iter().find(|c| c.name() == name).unwrap();
        assert_eq!(cop.default_enabled(), enabled);
        assert_eq!(cop.safe_fixes(), safe);
        assert!(cop.path_policy().is_none());
        assert_eq!(cop.ruby_versions().minimum, None);
        let report = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(source.as_bytes()),
            &[cop.as_ref()],
            &Default::default(),
        );
        assert!(report.errors.is_empty());
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.result.offenses.len(), 1);
        assert!(report.result.offenses[0].correction.is_none());
    }
    let d = project();
    let o = unchanged(d.path(), b"f(**{a: 1}); x = {\"a\" => 1}\n", &["a.rb"], 1);
    let text = String::from_utf8(o.stdout).unwrap();
    assert!(text.contains(BRACES));
    assert!(!text.contains(KEYS));
    unchanged(d.path(), b"{ \"a\" => 1 }\n", &["--fix", "a.rb"], 0);
    unchanged(
        d.path(),
        b"{'a'=>1}\n",
        &["--only", KEYS, "--fix", "a.rb"],
        1,
    );
}
#[test]
fn selectors_config_excludes_and_strict_fixed_surface() {
    let d = project();
    for (cop, source) in [(BRACES, "f(**{ a: 1 })\n"), (KEYS, "{ \"a\" => 1 }\n")] {
        unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled = false\n".to_string(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 0);
            let fixed = fix(d.path(), source.as_bytes(), Some(cop));
            assert_eq!(fixed != source.as_bytes(), cop == BRACES);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        for config in [
            "exclude = [\"a.rb\"]\n".to_string(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for knob in [
            "EnforcedStyle = 'braces'",
            "SafeAutoCorrect = true",
            "TargetRubyVersion = 3.4",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{knob}\n"),
            )
            .unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_disabled_spans_and_independent_siblings() {
    let d = project();
    for (cop, source) in [(BRACES, "f(**{a: 1})"), (KEYS, "{'a'=>1}")] {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.split('/').nth(1).unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}\n"),
                    format!("# {prefix}:disable-next {target}\n{source}\n"),
                    format!("{source} # {prefix}:disable {target}\n"),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
            let s =
                format!("# {prefix}:disable {cop}\n{source}\n# {prefix}:enable {cop}\n{source}\n");
            let after = fix(d.path(), s.as_bytes(), Some(cop));
            let expected = if cop == BRACES {
                s.replacen(
                    &format!("# {prefix}:enable {cop}\n{source}"),
                    &format!("# {prefix}:enable {cop}\nf(a: 1)"),
                    1,
                )
            } else {
                s
            };
            assert_eq!(after, expected.as_bytes());
        }
    }
    // A disabled nested span must not be edited as part of the outer atomic fix.
    let s = b"f(**{a: 1,\n # rustocop:disable-next Style/RedundantDoubleSplatHashBraces\n **{b: 2}})\nf(**{c: 3})\n";
    assert_eq!(
        fix(d.path(), s, Some(BRACES)),
        s.strip_suffix(b"f(**{c: 3})\n")
            .unwrap()
            .iter()
            .copied()
            .chain(b"f(c: 3)\n".iter().copied())
            .collect::<Vec<_>>()
    );
}
#[test]
fn stdin_lints_but_rejected_fix_never_writes_either_cop() {
    use std::process::Stdio;
    let d = project();
    let p = d.path().join("a.rb");
    fs::write(&p, b"on disk\n").unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    for (cop, source) in [(BRACES, "f(**{a: 1})\n"), (KEYS, "{'a'=>1}\n")] {
        for fixing in [false, true] {
            let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            c.current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"]);
            if fixing {
                c.arg("--fix");
            }
            let mut child = c
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            stdin::write_stdin(child.stdin.take().unwrap(), source.as_bytes(), fixing).unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(if fixing { 2 } else { 1 }), "{o:?}");
            if fixing {
                assert!(o.stdout.is_empty());
                assert!(
                    String::from_utf8_lossy(&o.stderr).contains("cannot be used with '--fix'"),
                    "{o:?}"
                );
            } else {
                assert!(o.stderr.is_empty());
                assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
            }
            assert_eq!(fs::read(&p).unwrap(), b"on disk\n");
            assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
        }
    }
}
#[test]
fn all_three_unsafe_pinned_string_key_corrections_hold_bytes_and_mtime() {
    // RuboCop string_hash_keys_spec.rb correction lines 10, 21, 32 (MIT,
    // Copyright (c) 2012-26 Bozhidar Batsov). Intentionally never convert keys.
    let d = project();
    for s in [
        "{ 'one' => 1 }\n",
        "{ 'one' => 1, two: 2, 3 => 3 }\n",
        "{ 'one two :' => 1 }\n",
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", KEYS, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn source_envelopes_decoded_keys_and_original_columns() {
    let d = project();
    for s in [
        b"# encoding: ISO-8859-1\r\nf(**{a: 'caf\xe9'})\r\np DATA.read\r\n__END__\r\nraw\0\r\n"
            .to_vec(),
        b"{ <<~KEY => 1 }\ntext\nKEY\n".to_vec(),
        b"# encoding: ISO-8859-1\n{'caf\xe9' => 1}\n".to_vec(),
        b"# encoding: ISO-8859-1\nf(**{a: 'caf\xe9'})\n".to_vec(),
        b"# encoding: ASCII-8BIT\n{\"\\xff\" => 1}\n".to_vec(),
        "\u{feff}# encoding: UTF-8\r\n{\"a\" => __LINE__}\r\n__END__\r\nraw\0\r\n"
            .as_bytes()
            .to_vec(),
    ] {
        let code = if s.starts_with(b"{ <<") { 0 } else { 1 };
        unchanged(d.path(), &s, &["--only", SELECT, "--fix", "a.rb"], code);
    }
    let before = "\u{feff}# encoding: UTF-8\r\nf(**{a: __LINE__, **{\"café\": 2}})\r\np __LINE__\r\n__END__\r\nraw\0\r\n";
    let after = before.replace(
        "**{a: __LINE__, **{\"café\": 2}}",
        "a: __LINE__, \"café\": 2",
    );
    assert_eq!(
        fix(d.path(), before.as_bytes(), Some(BRACES)),
        crate::support::lf::lf(&after)
    );
    let o = unchanged(
        d.path(),
        "é = 1; {\"clé\" => 2}\n".as_bytes(),
        &["--only", KEYS, "a.rb"],
        1,
    );
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("a.rb:1:9:"),
        "{o:?}"
    );
}
#[test]
fn fixed_no_comma_incompatibility_and_peers_stabilize_with_bounded_explicit_passes() {
    let d = project();
    for selector in [
        Some(BRACES),
        Some(
            "Style/RedundantDoubleSplatHashBraces,Style/TrailingCommaInHashLiteral,Style/HashSyntax,Style/StringLiterals,Style/Semicolon,Style/RedundantParentheses,Layout",
        ),
        Some("Style,Layout"),
        None,
    ] {
        let mut source =
            b"def f(**kw)\n  kw\nend\np f(**{a: 'x', **{b: 2,},}); p f(**{:c => 3})\n".to_vec();
        let rubies = rubies();
        let observations: Vec<_> = rubies.iter().map(|ruby| observe(ruby, &source)).collect();
        let mut stable = false;
        for _ in 0..6 {
            let next = fix(d.path(), &source, selector);
            for (ruby, expected) in rubies.iter().zip(&observations) {
                assert_eq!(&observe(ruby, &next), expected, "{selector:?}: {ruby}");
            }
            if next == source {
                stable = true;
                break;
            }
            source = next;
        }
        assert!(stable, "{selector:?}: {}", String::from_utf8_lossy(&source));
        if selector == Some(BRACES) {
            assert!(String::from_utf8_lossy(&source).contains("f(a: 'x', b: 2)"));
        }
    }
}
fn rubies() -> Vec<String> {
    let mut r = vec!["ruby".to_string()];
    if let Ok(native) = crate::support::runtime::ruby() {
        let o = Command::new(&native)
            .args([
                "-e",
                crate::support::runtime::script("raise unless RUBY_VERSION.start_with?('3.4.')")
                    .as_str(),
            ])
            .output()
            .unwrap();
        assert!(o.status.success(), "{o:?}");
        r.push(native);
    } else {
        eprintln!("SKIP native Ruby 3.4; set RUSTOCOP_RUBY34");
    }
    r
}
fn observe(ruby: &str, s: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let s = std::str::from_utf8(s).unwrap();
    let o = Command::new(ruby).args(["-e", s]).output().unwrap();
    assert!(o.status.success(), "{ruby}: {s}\n{o:?}");
    (o.stdout, o.stderr)
}
#[test]
fn controlled_runtime_explains_merge_and_string_key_withholds() {
    let s = r#"
trace = []
class Coerce
  def initialize(trace); @trace = trace; end
  def to_hash; @trace << :coerce; {a: 2}; end
end
h = {a: 1}.merge(Coerce.new(trace)) { |k,l,r| trace << [k,l,r]; l }
raise unless h == {a: 1} && trace == [:coerce,[:a,1,2]]
# Never remove merge dispatch, even from literal Hash receivers.
class Hash
  alias old_merge merge
  def merge(*args,**kw); :overridden; end
end
raise unless {a: 1}.merge({b: 2}) == :overridden
h = Hash.new { |hash,key| [:missing,key] }; h['a'] = 1; h[:a] = 2
raise unless h.keys == ['a',:a] && h['missing'] == [:missing,'missing']
puts :ok
"#;
    for ruby in rubies() {
        assert_eq!(observe(&ruby, s.as_bytes()).0, b"ok\n");
    }
    let d = project();
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", KEYS, "--fix", "a.rb"],
        0,
    );
}
