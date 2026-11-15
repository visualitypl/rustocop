//! real CLI controls and controlled ordinary Ruby (never corpus code).
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const WORD: &str = "Style/WordArray";
const SLICE: &str = "Style/SlicingWithRange";
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
fn fixed(s: &str, only: &str) -> String {
    let d = project();
    fs::write(d.path().join("a.rb"), s).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    command.current_dir(d.path());
    if !only.is_empty() {
        command.args(["--only", only]);
    }
    let o = command.args(["--fix", "a.rb"]).output().unwrap();
    assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    fs::read_to_string(d.path().join("a.rb")).unwrap()
}
#[test]
fn exact_defaults_safe_capability_and_version_gates() {
    use rustocop::cops::{
        Registry,
        policy::{DEFAULT_RUBY_VERSION, RubyVersion},
    };
    let registry = Registry::new();
    assert_eq!(DEFAULT_RUBY_VERSION, RubyVersion(3, 4));
    for name in [WORD, SLICE] {
        let c = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), name == WORD);
        assert!(c.path_policy().is_none());
        assert_eq!(c.ruby_versions().maximum, None);
    }
    let c = registry.cops().iter().find(|c| c.name() == SLICE).unwrap();
    assert_eq!(c.ruby_versions().minimum, Some(RubyVersion(2, 6)));
}
#[test]
fn selectors_config_excludes_and_no_write_contract() {
    let d = project();
    for (cop, s) in [(WORD, "[\"one\", \"two\"]\n"), (SLICE, "ary[1..-1]\n")] {
        let o = unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled = false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        for config in [
            "exclude = [\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nMinSize = 1\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 2);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn both_directives_all_scopes_and_reenable() {
    let d = project();
    for (cop, s) in [(WORD, "[\"one\", \"two\"]"), (SLICE, "ary[1..-1]")] {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{s}\n"),
                    format!("# {prefix}:disable-next {target}\n{s}\n"),
                    format!("{s} # {prefix}:disable {target}\n"),
                ] {
                    unchanged(
                        d.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "a.rb"],
                        0,
                    );
                }
            }
            let source = format!("# {prefix}:disable {cop}\n{s}\n# {prefix}:enable {cop}\n{s}\n");
            let o = unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (source, code) in [("[]\n", 0), ("(\n", 2)] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
    unchanged(
        d.path(),
        b"[\n'one', # rubocop:disable Style/WordArray\n 'two'\n]\n",
        &["--only", WORD, "--fix", "a.rb"],
        0,
    );
}
#[test]
fn stdin_is_detection_only_and_never_writes_its_display_file() {
    use std::io::Write;
    use std::process::Stdio;
    let d = project();
    fs::write(d.path().join("a.rb"), "unchanged\n").unwrap();
    for (cop, s) in [(WORD, "['one','two']\n"), (SLICE, "ary[1..-1]\n")] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "unchanged\n"
        );
    }
}
#[test]
fn unsafe_slicing_identity_and_custom_range_dispatch_witnesses() {
    let s = r#"a=[1,2]; b=a[0..-1]; b << 3; p [a,b,a.equal?(b)]
object=Object.new
def object.[](range); [range.begin,range.end,range.exclude_end?]; end
p [object[1..-1],object[1..],object[nil..42],object[..42]]
"#;
    assert_eq!(fixed(s, SLICE), s);
    if Command::new("ruby").arg("--version").output().is_ok() {
        let o = observe("ruby", s);
        assert!(String::from_utf8_lossy(&o.0).contains("[[1, 2], [1, 2, 3], false]"));
        assert!(String::from_utf8_lossy(&o.0).contains("[[1, -1, false], [1, nil, false]"));
    }
}
#[test]
fn unsafe_slicing_getters_array_like_and_mutation_remain_byte_exact() {
    let d = project();
    for source in [
        "ary[0..-1]\n",
        "ary[1..-1] += rhs\n",
        "ary[nil..finish()] ||= rhs\n",
        "ary[1...nil] &&= rhs\n",
        "proc { |r| r.sum }[2..-1]\n",
        "factory()[start()..-1]\n",
        "ary&.[](0..nil)\n",
    ] {
        fs::write(d.path().join("safe.rb"), "['one','two']\n").unwrap();
        unchanged(
            d.path(),
            source.as_bytes(),
            &[
                "--only",
                "Style/WordArray,Style/SlicingWithRange",
                "--fix",
                "a.rb",
                "safe.rb",
            ],
            1,
        );
        assert_eq!(
            fs::read_to_string(d.path().join("safe.rb")).unwrap(),
            "%w[one two]\n"
        );
    }
}
#[test]
fn multiline_word_fixes_do_not_abort_independent_corrections() {
    for nl in ["\n", "\r\n"] {
        for kind in ["w", "W"] {
            let source = format!(
                "x = %{kind}[a\\ b\\{nl}c d]{nl}y = [\"one\", \"two\"]{nl}p [x, y, __LINE__]{nl}"
            );
            let expected = format!(
                "x = [\"a b\\nc\",{nl} \"d\"]{nl}y = %w[one two]{nl}p [x, y, __LINE__]{nl}"
            );
            let d = project();
            fs::write(d.path().join("a.rb"), &source).unwrap();
            cli(d.path(), &["--only", WORD, "--fix", "a.rb"], 0);
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            assert_eq!(after, expected);
            unchanged(
                d.path(),
                after.as_bytes(),
                &["--only", WORD, "--fix", "a.rb"],
                0,
            );
            if Command::new("ruby").arg("--version").output().is_ok() {
                assert_eq!(observe("ruby", &source), observe("ruby", &after));
            }
        }
    }
}
fn observe(ruby: &str, source: &str) -> (Vec<u8>, Vec<u8>) {
    let o = Command::new(ruby)
        .args(["-W:deprecated", "-e", source])
        .output()
        .unwrap();
    assert!(o.status.success(), "{ruby}: {source}: {o:?}");
    (o.stdout, o.stderr)
}
fn runtime(ruby: &str) {
    let mut changed = 0;
    for magic in [
        "",
        "# frozen_string_literal: false\n",
        "# frozen_string_literal: true\n",
    ] {
        for literal in [
            "['one','two']",
            "['one','one']",
            "[\"one\\n\",\"hi\\tthere\"]",
            "[\"one\", ?\\n]",
            "['ВУЗ','中文网']",
            "[\"\\u0061\",\"two\"]",
            "[\"\\u4e2d\",\"two\"]",
            "%w[one\\ two three\\ four]",
            "%W[one\\ two three\\ four]",
            "%w[a\\ b]",
            "%w[a\\ b\\\nc]",
            "%w[a\\ b \\]c \\#{literal}]",
            "[\n'one',\n 'two',\n]",
            "[\"one\n\", \"two\"]",
            "[\"a\\\nb\",'two']",
            "[ 'one', \\\n 'two', \\\n 'three' ]",
        ] {
            let s = format!("{magic}x = {literal}\n");
            let after = fixed(&s, WORD);
            assert_ne!(s, after, "canonical must apply: {s}");
            changed += 1;
            let suffix = r#"p x.map { |s| [s.bytes, s.encoding.name, s.frozen?] }
p x.each_index.map { |i| x.each_index.map { |j| x[i].equal?(x[j]) } }
x.each { |s| begin; s << '!'; rescue => e; p e.class.name; end }
p x.map { |s| [s.bytes, s.encoding.name, s.frozen?] }
p __LINE__
"#;
            assert_eq!(
                observe(ruby, &format!("{s}{suffix}")),
                observe(ruby, &format!("{after}{suffix}")),
                "{s} -> {after}"
            );
            assert_eq!(fixed(&after, WORD), after);
        }
    }
    assert_eq!(changed, 48);
}
#[test]
fn controlled_host_runtime_every_child_bytes_encoding_identity_mutation_and_warnings() {
    if Command::new("ruby").arg("--version").output().is_ok() {
        runtime("ruby");
    }
}
#[test]
fn deferred_literals_and_encoded_ascii_children_keep_warning_and_identity_profiles() {
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        for magic in [
            "",
            "# frozen_string_literal: false\n",
            "# frozen_string_literal: true\n",
        ] {
            let s = format!(
                "{magic}def build; ['one','one']; end\nx=build; y=build\np x.zip(y).map {{ |a,b| [a.equal?(b),a.frozen?,a.encoding.name] }}\nx.each {{ |s| begin; s << '!'; rescue => e; p e.class.name; end }}\np y\n"
            );
            let after = fixed(&s, WORD);
            assert_ne!(s, after);
            assert_eq!(observe(&ruby, &s), observe(&ruby, &after));
        }
        let encodings = if ruby == "ruby" {
            vec![
                "US-ASCII",
                "binary",
                "Windows-31J",
                "EUC-JP",
                "EUC-KR",
                "CP949",
                "GB18030",
            ]
        } else {
            vec!["US-ASCII", "binary"]
        };
        for encoding in encodings {
            for mode in [
                "",
                "; frozen_string_literal: false",
                "; frozen_string_literal: true",
            ] {
                for literal in [
                    "['one','two']",
                    "[\"\\u0061\",'two']",
                    "[\"one\\n\",'two']",
                    "%W[a\\ b\\v\\f\\a\\b\\e\\x00\\x1f]",
                ] {
                    let s = format!(
                        "# encoding: {encoding}{mode}\nx={literal}\np x.map {{ |s| [s.bytes,s.encoding.name,s.frozen?] }}\nx.each {{ |s| begin; s << '!'; rescue => e; p e.class.name; end }}\np x\n"
                    );
                    let after = fixed(&s, WORD);
                    assert_ne!(s, after);
                    assert_eq!(observe(&ruby, &s), observe(&ruby, &after), "{ruby}: {s}");
                }
            }
        }
    }
}
#[test]
fn controlled_native_34_runtime_every_child_in_all_frozen_modes() {
    if let Ok(ruby) = crate::support::runtime::ruby() {
        let version = Command::new(&ruby)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(
            version
                .stdout
                .starts_with(crate::support::runtime::version_prefix().as_bytes())
        );
        for literal in ["['one','two']", "%w[one two]", "%W[one two]"] {
            for (magic, warnings) in [
                ("", 2),
                ("# frozen_string_literal: false\n", 0),
                ("# frozen_string_literal: true\n", 0),
            ] {
                let (_, stderr) = observe(
                    &ruby,
                    &format!(
                        "{magic}x={literal}\nx.each {{ |s| begin; s << '!'; rescue => e; p e.class.name; end }}\n"
                    ),
                );
                assert_eq!(
                    String::from_utf8_lossy(&stderr)
                        .matches("literal string will be frozen")
                        .count(),
                    warnings,
                    "{magic}{literal}"
                );
            }
        }
        runtime(&ruby);
    } else {
        eprintln!("SKIP native Ruby 3.4 runtime: set RUSTOCOP_RUBY34");
    }
}
#[test]
fn cesu_surrogates_and_carrier_keycaps_keep_reference_word_unit_counts() {
    let d = project();
    for (encoding, raw, hyphenated) in [
        ("CESU-8", b"\xed\xa1\x80\xed\xb0\x80".as_slice(), false),
        ("UTF8-DoCoMo", b"\xee\x9b\xa2", true),
        ("SJIS-DoCoMo", b"\xf9\x87", true),
    ] {
        for (prefix, suffix, count) in [
            (b"['".as_slice(), b"','two']\n".as_slice(), 1),
            (b"['a-", b"-c','two']\n", i32::from(hyphenated)),
            (b"%w[", b"\\ space two]\n", 1),
        ] {
            let mut source = format!("# encoding: {encoding}\n").into_bytes();
            source.extend_from_slice(prefix);
            source.extend_from_slice(raw);
            source.extend_from_slice(suffix);
            unchanged(d.path(), &source, &["--only", WORD, "--fix", "a.rb"], count);
        }
        // original declared-encoding coordinates, not the reference
        // buffer's composed/expanded prefix columns. Edits still use raw bytes.
        let mut source = format!("# encoding: {encoding}\ns='").into_bytes();
        source.extend_from_slice(raw);
        source.extend_from_slice(b"'; ['one','two']\n");
        let out = unchanged(d.path(), &source, &["--only", WORD, "a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(":2:8:"));
        cli(d.path(), &["--only", WORD, "--fix", "a.rb"], 0);
        let needle = b"['one','two']";
        let start = source
            .windows(needle.len())
            .position(|s| s == needle)
            .unwrap();
        let mut expected = source[..start].to_vec();
        expected.extend_from_slice(b"%w[one two]\n");
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), expected);
    }
}
#[test]
fn legacy_buffer_projection_binary_ascii_and_forced_utf8_profiles() {
    let d = project();
    for (encoding, word) in [
        ("Windows-31J", "ソ"),
        ("Shift_JIS", "あ"),
        ("EUC-JP", "あ"),
        ("EUC-KR", "가"),
        ("CP949", "갂"),
        ("GB18030", "𠀀"),
        ("Windows-1252", "é"),
        ("ISO-8859-1", "é"),
    ] {
        // Only this controlled source is transcoded, never executed as corpus.
        for (literal, count) in [
            (format!("['{word}','two']"), 1),
            (format!("[\"{word}\\tfoo\",'two']"), 1),
            (format!("['{word} !','two']"), 0),
            (format!("%w[{word}\\ space two]"), 1),
        ] {
            let source = format!("# encoding: {encoding}\n{literal}\n");
            let o = Command::new("ruby")
                .args(["-e", "print ARGV[0].encode(ARGV[1])", &source, encoding])
                .output()
                .unwrap();
            assert!(o.status.success(), "{o:?}");
            unchanged(
                d.path(),
                &o.stdout,
                &["--only", WORD, "--fix", "a.rb"],
                if count == 0 { 0 } else { 1 },
            );
        }
    }
    for encoding in ["Windows-31J", "EUC-JP", "EUC-KR", "CP949", "GB18030"] {
        let word = if matches!(encoding, "EUC-KR" | "CP949") {
            "가"
        } else {
            "あ"
        };
        let source = format!("# encoding: {encoding}\n[%w[{word}\\\n{word}], ['one','two']]\n");
        let o = Command::new("ruby")
            .args(["-e", "print ARGV[0].encode(ARGV[1])", &source, encoding])
            .output()
            .unwrap();
        assert!(o.status.success());
        unchanged(d.path(), &o.stdout, &["--only", WORD, "a.rb"], 1);
        cli(d.path(), &["--only", WORD, "--fix", "a.rb"], 0);
        // Compare bytes without a lossy decoded expectation.
        let needle = b"['one','two']";
        let start = o
            .stdout
            .windows(needle.len())
            .position(|s| s == needle)
            .unwrap();
        let mut bytes = o.stdout[..start].to_vec();
        bytes.extend_from_slice(b"%w[one two]");
        bytes.extend_from_slice(&o.stdout[start + needle.len()..]);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), bytes);
    }
    for (source, count) in [
        (
            b"# encoding: Windows-31J\n[\"\\xD0\\M-\\C-\\aword\", 'two']\n".as_slice(),
            1,
        ),
        (
            b"# encoding: Windows-31J\n[\"\\x82\\xa0\", 'two']\n".as_slice(),
            0,
        ),
        (b"# encoding: EUC-JP\n[\"\\xa4\\xa2\", 'two']\n", 0),
        (b"# encoding: binary\n['\xc3\xa9', 'two']\n", 1),
        (b"# encoding: binary\n['\xe9', 'two']\n", 0),
        (b"# encoding: US-ASCII\n[\"\\u4e2d\",'two']\n", 1),
    ] {
        unchanged(d.path(), source, &["--only", WORD, "--fix", "a.rb"], count);
    }
    for encoding in [
        "US-ASCII",
        "binary",
        "Windows-31J",
        "EUC-JP",
        "EUC-KR",
        "CP949",
        "GB18030",
    ] {
        let s = format!("# encoding: {encoding}\n['one','two']\n");
        assert_eq!(
            fixed(&s, WORD),
            format!("# encoding: {encoding}\n%w[one two]\n")
        );
    }
}
#[test]
fn interpolation_values_evaluation_order_and_binding_are_never_regrouped() {
    let d = project();
    for s in [
        r#"%W[#{effect} a\ b]"#,
        r#"%W[a\ b #{effect} #{other}]"#,
        r#"%W[a\ b #{x = 1} #{x}]"#,
        r#"%W[a\ b #{yield}]"#,
    ] {
        let s = format!("def f; {s}; end\n");
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", WORD, "--fix", "a.rb"],
            1,
        );
    }
    let s = r#"log=[]; x=%W[a\ b #{log << 1; 'x'} #{log << 2; 'y'}]; p [x,log]"#;
    assert_eq!(fixed(s, WORD), s);
    if Command::new("ruby").arg("--version").output().is_ok() {
        assert_eq!(observe("ruby", s), observe("ruby", &fixed(s, WORD)));
    }
}
#[test]
fn composition_selected_peers_and_real_defaults_stabilizes_within_six_explicit_passes() {
    let source = "x = ['one', 'two']; y = %w(one\\ two three\\ four)\np [x,y]\n";
    let peers = "Style/WordArray,Style/SlicingWithRange,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/BarePercentLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/StringLiterals,Style/TrailingCommaInArrayLiteral,Style/RedundantArrayConstructor,Style/NegativeArrayIndex,Style/ArrayJoin,Style/RedundantArrayFlatten,Layout";
    for only in [WORD, peers, "Style,Layout", ""] {
        let mut s = source.to_owned();
        let mut seen = std::collections::HashSet::new();
        for pass in 0..6 {
            assert!(seen.insert(s.clone()), "cycle: {only}");
            let next = fixed(&s, only);
            if Command::new("ruby").arg("--version").output().is_ok() {
                assert_eq!(
                    observe("ruby", source),
                    observe("ruby", &next),
                    "{only}: pass {pass}"
                );
            }
            if next == s {
                break;
            }
            assert!(pass < 5, "did not stabilize {only}");
            s = next;
        }
        assert!(s.contains("%w[one two]"), "{only}: {s}");
    }
}
