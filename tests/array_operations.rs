//! fixed array-operation policies and controlled ordinary Ruby only.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const JOIN: &str = "Style/ArrayJoin";
const COPS: [(&str, &str, bool); 3] = [
    (
        "Style/ArrayIntersectWithSingleElement",
        "arr.intersect?([x])\n",
        false,
    ),
    (JOIN, "%w[a b] * \",\"\n", true),
    ("Style/ConcatArrayLiterals", "arr.concat([x])\n", false),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    assert!(out.stderr.is_empty() || code == 2, "{out:?}");
    out
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let out = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    out
}
#[test]
fn registry_defaults_safe_capability_and_fixed_34_no_extra_gates() {
    use rustocop::cops::{
        Registry,
        policy::{DEFAULT_RUBY_VERSION, RubyVersion},
    };
    let registry = Registry::new();
    assert_eq!(DEFAULT_RUBY_VERSION, RubyVersion(3, 4));
    for (name, _, safe) in COPS {
        let c = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), safe);
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(c.path_policy().is_none());
    }
}
#[test]
fn default_only_except_configs_excludes_and_no_write_without_fix() {
    let d = project();
    for (name, s, _) in COPS {
        let out = unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(name));
        unchanged(d.path(), s.as_bytes(), &["--only", name, "a.rb"], 1);
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", name, "--except", name, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled = false\n".to_string(),
            format!("[cops.\"{name}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), s.as_bytes(), &["--only", name, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        for config in [
            "exclude = [\"a.rb\"]\n".to_string(),
            format!("[cops.\"{name}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", name, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{name}\"]\nTargetRubyVersion = 4.0\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 2);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn both_directive_prefixes_disable_enable_and_uncorrectable_bytes_mtime() {
    let d = project();
    for (name, source, safe) in COPS {
        for prefix in ["rubocop", "rustocop"] {
            for target in [name, name.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(
                        d.path(),
                        s.as_bytes(),
                        &["--only", name, "--fix", "a.rb"],
                        0,
                    );
                }
            }
            let s =
                format!("# {prefix}:disable {name}\n{source}# {prefix}:enable {name}\n{source}");
            unchanged(d.path(), s.as_bytes(), &["--only", name, "a.rb"], 1);
        }
        for source in ["[]\n", "(\n"] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", name, "--fix", "a.rb"],
                if source == "(\n" { 2 } else { 0 },
            );
        }
        if !safe {
            for s in [
                source.to_string(),
                format!(
                    "\u{feff}{}__LINE__\r\n__END__\r\nraw\0\r\n",
                    source.replace('\n', "\r\n")
                ),
            ] {
                unchanged(
                    d.path(),
                    s.as_bytes(),
                    &["--only", name, "--fix", "a.rb"],
                    1,
                );
            }
        }
    }
}
#[test]
fn actual_canonical_single_pass_and_safe_sibling_withholding() {
    let d = project();
    let s = "x = %w[a b] * \",\"\narr.concat([item])\narr.intersect?([item])\n";
    fs::write(d.path().join("a.rb"), s).unwrap();
    let selected =
        "Style/ArrayJoin,Style/ArrayIntersectWithSingleElement,Style/ConcatArrayLiterals";
    cli(d.path(), &["--only", selected, "--fix", "a.rb"], 1);
    let expected = "x = %w[a b].join(\",\")\narr.concat([item])\narr.intersect?([item])\n";
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
    unchanged(
        d.path(),
        expected.as_bytes(),
        &["--only", selected, "--fix", "a.rb"],
        1,
    );
    unchanged(
        d.path(),
        b"[x] * ','\n",
        &["--only", JOIN, "--fix", "a.rb"],
        1,
    );
    fs::write(d.path().join("a.rb"), "%w[a b] * \",\"\n").unwrap();
    cli(d.path(), &["--only", JOIN, "--fix", "a.rb"], 0);
    unchanged(
        d.path(),
        b"%w[a b].join(\",\")\n",
        &["--only", JOIN, "--fix", "a.rb"],
        0,
    );
}
#[test]
fn encoding_heredoc_observer_and_command_sources_not_executed() {
    let d = project();
    for (name, source) in [
        (
            JOIN,
            b"# encoding: ISO-8859-1\n%w[\xe9 a\\ b] * ','\n".as_slice(),
        ),
        (JOIN, b"# encoding: Windows-31J\n%w[\x83\\ a] * ','\n"),
        (JOIN, b"[`never execute`] * ','\n"),
        (JOIN, b"[<<~TEXT] * ','\nx\nTEXT\n"),
        (JOIN, b"[] * ','\np File.binread(__FILE__)\n"),
        (JOIN, b"[] * ','\np DATA.pos\n__END__\npayload\n"),
        (
            "Style/ConcatArrayLiterals",
            b"# encoding: ISO-8859-1\na.concat(%w[\xe9 a\\ b])\n",
        ),
        (
            "Style/ArrayIntersectWithSingleElement",
            b"# encoding: Windows-31J\na.intersect?(%w[\x83\\])\n",
        ),
    ] {
        unchanged(d.path(), source, &["--only", name, "--fix", "a.rb"], 1);
    }
    let before = b"\xef\xbb\xbf%w[a b] * \",\"\r\n__LINE__\r\n__END__\r\npayload\0\r\n";
    fs::write(d.path().join("a.rb"), before).unwrap();
    cli(d.path(), &["--only", JOIN, "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read(d.path().join("a.rb")).unwrap(),
        b"\xef\xbb\xbf%w[a b].join(\",\")\r\n__LINE__\r\n__END__\r\npayload\0\r\n"
    );
}
#[test]
fn dense_candidates_complete_in_one_pass_without_quadratic_source_scans() {
    let d = project();
    for (name, s, safe) in COPS {
        let s = s.repeat(6000);
        if safe {
            fs::write(d.path().join("a.rb"), s).unwrap();
            cli(d.path(), &["--only", name, "--fix", "a.rb"], 0);
            assert_eq!(
                fs::read_to_string(d.path().join("a.rb")).unwrap(),
                "%w[a b].join(\",\")\n".repeat(6000)
            );
        } else {
            let out = unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", name, "--fix", "a.rb"],
                1,
            );
            assert_eq!(
                String::from_utf8_lossy(&out.stdout)
                    .matches(&format!("({name})"))
                    .count(),
                6000
            );
        }
    }
}
fn ruby_observation(ruby: &str, source: &str) -> (Vec<u8>, Vec<u8>) {
    // Keep magic comments in their real header position, including the native
    // 3.4 frozen/chilled-string modes; wrapping must not silently disable them.
    let (header, body) = if source.starts_with("# frozen_string_literal:") {
        source.split_once('\n').unwrap()
    } else {
        ("", source)
    };
    let wrapped = format!("{header}\nbegin\n{body}\nrescue => e\np [e.class.name, e.message]\nend");
    let o = Command::new(ruby)
        .args(["-W:deprecated", "-e", &wrapped])
        .output()
        .unwrap();
    assert!(o.status.success(), "{ruby}: {o:?}");
    (o.stdout, o.stderr)
}
fn runtime_canonicals(ruby: &str) {
    let d = project();
    for expr in [
        "[] * ','",
        "[nil, [1, false], :x] * ','",
        "%w[a\\ b c\\]d] * %q|,|",
        "%i[a b] * ','",
        "%W[a b] * ','",
        "['x'] * ?\\ ",
        "['x'].*(%q , )",
        "[\"\\xFF\"] * ','",
        "['x'] * \"\\xFF\"",
        "[['a'], [], [nil, 2.5]] * ','",
    ] {
        for magic in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            let s = format!(
                "{magic}x = {expr}\np [x.bytes, x.encoding.name, x.frozen?, (x << '!').bytes]\n"
            );
            fs::write(d.path().join("a.rb"), &s).unwrap();
            cli(d.path(), &["--only", JOIN, "--fix", "a.rb"], 0);
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            assert_ne!(s, after, "{expr}");
            assert_eq!(
                ruby_observation(ruby, &s),
                ruby_observation(ruby, &after),
                "{expr}"
            );
        }
    }
}
const UNSAFE_WITNESSES: &str = r#"
class ArrayLike
  def intersect?(a); [:intersect, a]; end
  def include?(x); [:include, x]; end
  def concat(a); [:concat, a]; end
  def push(*a); [:push, a]; end
end
x = ArrayLike.new
raise if x.intersect?([1]) == x.include?(1)
raise if x.concat([1]) == x.push(1)
# Different equality dispatch, even with real arrays.
class Element
  def hash; 0; end
  def eql?(x); true; end
  def ==(x); false; end
end
a = Array.new(20) { Element.new }; e = Element.new
raise unless a.intersect?([e]) && !a.include?(e)
# concat snapshots self before mutation, push(*self) has different argument APIs.
a = [1]; result = a.concat(a, [2]); raise unless result.equal?(a) && a == [1, 1, 2]
events = []; a = []
a.concat([(events << :first; 1)], [(events << :second; 2)])
raise unless events == [:first, :second] && a == [1, 2]
# Splats and unknown conversion methods are not correction proofs.
class Convertible
  def to_s; 'converted'; end
  def to_ary; ['nested']; end
end
raise unless [Convertible.new].join(',') == 'nested'
# Even literal receivers are unsafe after core methods are replaced. Our
# closed-dispatch guard leaves these detection cases unchanged.
class Array
  alias original_star *
  def *(separator); [:overridden_star, separator]; end
end
raise unless ([1] * ',') == [:overridden_star, ','] && [1].join(',') == '1'
class Array
  alias * original_star
  alias original_join join
  def join(separator); :overridden_join; end
end
raise unless ([1] * ',') == '1' && [1].join(',') == :overridden_join
class Array
  alias join original_join
end
puts 'array operation witnesses ok'
"#;
fn witnesses(ruby: &str) {
    let out = Command::new(ruby)
        .args(["-e", UNSAFE_WITNESSES])
        .output()
        .unwrap();
    assert!(out.status.success(), "{ruby}: {out:?}");
    assert_eq!(out.stdout, b"array operation witnesses ok\n");
    runtime_canonicals(ruby);
}
#[test]
fn controlled_host_ruby_runtime_and_unsafe_counterexamples() {
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        witnesses("ruby");
    }
}
#[test]
fn controlled_fixed_34_runtime_and_unsafe_counterexamples() {
    if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
        let out = Command::new(&ruby)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).starts_with("3.4."));
        witnesses(&ruby);
    } else {
        eprintln!("SKIP native Ruby 3.4 runtime; set RUSTOCOP_RUBY34");
    }
}
#[test]
fn curated_original_literal_array_peers_and_default_style_layout_stabilize_within_six_passes() {
    let d = project();
    let source = "x = %w(a b) * ','; p x\n";
    for only in [
        Some("Style/ArrayJoin,Style/ArrayIntersectWithSingleElement,Style/ConcatArrayLiterals"),
        Some(
            "Style/ArrayJoin,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation,Style/EmptyStringInsideInterpolation,Style/RedundantInterpolationUnfreeze,Style/CommandLiteral,Style/FormatString,Style/FormatStringToken,Style/RedundantFormat,Style/StringConcatenation,Style/LineEndConcatenation,Style/RedundantLineContinuation,Style/StringChars,Style/StringMethods,Style/Strip,Style/UnpackFirst,Style/ArrayCoercion,Style/ArrayFirstLast,Style/ArrayIntersect,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let args = only.map_or(vec!["--fix", "a.rb"], |s| {
                vec!["--only", s, "--fix", "a.rb"]
            });
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            let after = fs::read(d.path().join("a.rb")).unwrap();
            if Command::new("ruby").arg("--version").output().is_ok() {
                assert_eq!(
                    ruby_observation("ruby", source),
                    ruby_observation("ruby", std::str::from_utf8(&after).unwrap())
                );
            }
            if after == before {
                stable = true;
                break;
            }
        }
        assert!(stable, "{only:?}");
        assert!(
            fs::read_to_string(d.path().join("a.rb"))
                .unwrap()
                .contains(".join(")
        );
    }
    // Unsafe ConcatArrayLiterals never joins its upstream incompatible fix set.
    let only = "Style/ConcatArrayLiterals,Style/MethodCallWithArgsParentheses,Style/TrailingCommaInArguments";
    unchanged(
        d.path(),
        b"arr.concat([x])\n",
        &["--only", only, "--fix", "a.rb"],
        1,
    );
}
