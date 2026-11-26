//! all three cops are unsafe/detection-only. Runtime scripts below
//! are controlled ordinary counterexamples, never application or command code.
use crate::support::cli::project;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [(&str, &str, bool); 3] = [
    ("Style/ArrayCoercion", "[*paths]\n", false),
    ("Style/ArrayFirstLast", "arr[0]\n", false),
    ("Style/ArrayIntersect", "(a & b).any?\n", true),
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
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
#[test]
fn registry_fixed_defaults_gates_and_no_safe_capabilities() {
    use rustocop::cops::policy::{DEFAULT_RUBY_VERSION, RubyVersion};
    let registry = rustocop::cops::Registry::new();
    for (name, _, enabled) in COPS {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert_eq!(cop.default_enabled(), enabled);
        assert!(!cop.safe_fixes());
        assert!(cop.path_policy().is_none());
        assert_eq!(DEFAULT_RUBY_VERSION, RubyVersion(3, 4));
        if name == "Style/ArrayIntersect" {
            assert_eq!(cop.ruby_versions().minimum, Some(RubyVersion(3, 1)));
            assert_eq!(cop.ruby_versions().maximum, None);
        } else {
            assert_eq!(cop.ruby_versions(), Default::default());
        }
    }
}
#[test]
fn cli_default_opt_in_only_except_config_and_no_writes() {
    let d = project();
    for (cop, source, enabled) in COPS {
        for fix in [false, true] {
            let mut args = vec!["a.rb"];
            if fix {
                args.insert(0, "--fix");
            }
            let out = unchanged(d.path(), source.as_bytes(), &args, i32::from(enabled));
            assert_eq!(String::from_utf8_lossy(&out.stdout).contains(cop), enabled);
            let args = if fix {
                vec!["--only", cop, "--fix", "a.rb"]
            } else {
                vec!["--only", cop, "a.rb"]
            };
            assert!(
                String::from_utf8_lossy(&unchanged(d.path(), source.as_bytes(), &args, 1).stdout)
                    .contains(cop)
            );
        }
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
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                1,
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--except", cop, "--fix", "a.rb"],
            0,
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn global_and_per_cop_excludes_strict_options_and_stdin() {
    let d = project();
    for (cop, source, _) in COPS {
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
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nEnforcedStyle = \"other\"\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            2,
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
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
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
    }
}
#[test]
fn both_directive_prefixes_disable_enable_and_opt_in_scope() {
    let d = project();
    for (cop, source, enabled) in COPS {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
            let s = format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
            if !enabled {
                let s = format!(
                    "{source}# {prefix}:enable {cop}\n{source}# {prefix}:disable {cop}\n{source}"
                );
                let out = unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 1);
                assert_eq!(
                    String::from_utf8_lossy(&out.stdout)
                        .matches(&format!("({cop})"))
                        .count(),
                    1
                );
            }
        }
    }
    // Multiline intersection reports the whole call/block. Indexing reports
    // only the brackets (or the dotted selector through its arguments).
    for (cop, source, code) in [
        (
            "Style/ArrayIntersect",
            "(a &\n b).any? # rubocop:disable Style/ArrayIntersect\n",
            0,
        ),
        (
            "Style/ArrayIntersect",
            "a.any? { |x|\n b.member?(x)\n} # rustocop:disable Style/ArrayIntersect\n",
            0,
        ),
        (
            "Style/ArrayFirstLast",
            "arr[0] {\n effect # rubocop:disable Style/ArrayFirstLast\n}\n",
            1,
        ),
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            code,
        );
    }
}
#[test]
fn encoding_percent_arrays_bom_crlf_line_data_and_unsafe_sources_stay_byte_identical() {
    let d = project();
    for (cop, source, _) in COPS {
        for envelope in [
            format!(
                "\u{feff}{}p __LINE__\r\n__END__\r\nraw\\data\0\r\n",
                source.replace('\n', "\r\n")
            ),
            format!("# frozen_string_literal: true\n{source}p __LINE__\n"),
            format!("# encoding: UTF-8\n{source}"),
        ] {
            unchanged(
                d.path(),
                envelope.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                1,
            );
        }
        for s in ["arr.first\n", "(\n"] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                if s == "(\n" { 2 } else { 0 },
            );
        }
    }
    for (cop, source) in [
        (
            "Style/ArrayCoercion",
            b"# encoding: ISO-8859-1\n[*%w[\xe9 a\\ b]]\n".as_slice(),
        ),
        (
            "Style/ArrayFirstLast",
            b"# encoding: Windows-31J\n%w[\x83\\ a][0]\n",
        ),
        (
            "Style/ArrayIntersect",
            b"# encoding: ISO-8859-1\na.any? { |e| %w[\xe9 a\\ b].include?(e) }\n",
        ),
        (
            "Style/ArrayIntersect",
            b"a.any? { %I[a#{effect} b].include?(it) }\n",
        ),
        ("Style/ArrayCoercion", b"[*`never executed`]\n"),
        ("Style/ArrayFirstLast", b"`never executed`[0]\n"),
        ("Style/ArrayIntersect", b"(`never executed` & b).any?\n"),
    ] {
        unchanged(d.path(), source, &["--only", cop, "--fix", "a.rb"], 1);
    }
}
#[test]
fn sole_block_pass_intersections_report_but_never_write_even_with_fix() {
    let d = project();
    let cop = "Style/ArrayIntersect";
    for source in [
        "a.intersection(&b).any?\n",
        "a.intersection(&nil).empty?\n",
        "a&.intersection(&:foo)&.any?.to_s\n",
        "(a.&(&factory())).size > 0\n",
        "def f(&)\n  a.intersection(&)&.count.positive?.to_s\nend\n",
    ] {
        for fix in [false, true] {
            let mut args = vec!["--only", cop, "a.rb"];
            if fix {
                args.insert(0, "--fix");
            }
            let out = unchanged(d.path(), source.as_bytes(), &args, 1);
            let text = String::from_utf8_lossy(&out.stdout);
            assert_eq!(text.matches("(Style/ArrayIntersect)").count(), 1, "{text}");
        }
        for prefix in ["rubocop", "rustocop"] {
            let disabled = format!("# {prefix}:disable {cop}\n{source}");
            unchanged(
                d.path(),
                disabled.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
    for source in [
        "a.intersection(b, &b).any?\n",
        "a.intersection(b) { effect }.any?\n",
        "(a&.&(&b)).any?\n",
        "a.intersection(&b)&.none?\n",
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            0,
        );
    }
}
#[test]
fn safe_peer_corrections_do_not_enable_unsafe_array_rewrites() {
    let d = project();
    let source = "[*paths]\narr[0]\n(a & b).any?\n'x'\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    let only = "Style/ArrayCoercion,Style/ArrayFirstLast,Style/ArrayIntersect,Style/StringLiterals";
    cli(d.path(), &["--only", only, "--fix", "a.rb"], 1);
    let expected = "[*paths]\narr[0]\n(a & b).any?\n\"x\"\n";
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
    let time = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    cli(d.path(), &["--only", only, "--fix", "a.rb"], 1);
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        time
    );
    // Actual defaults do not opt in coercion or indexing, and keep intersection.
    let out = cli(d.path(), &["--fix", "a.rb"], 1);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Style/ArrayIntersect"));
    assert!(!text.contains("Style/ArrayCoercion"));
    assert!(!text.contains("Style/ArrayFirstLast"));
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
}
#[test]
fn dense_candidates_use_bounded_main_ast_work_and_never_write() {
    let d = project();
    for (cop, source, _) in COPS {
        let source = source.repeat(6000);
        let out = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout)
                .matches(&format!("({cop})"))
                .count(),
            6000
        );
    }
}
const WITNESSES: &str = r#"
# Kernel.Array may retain identity, unlike a splat copy; nil/hash conversion
# differs from simply wrapping a value after an Array class check.
x = []; y = [*x]; raise unless y == x && !y.equal?(x) && Array(x).equal?(x)
x = nil; x = [x] unless x.is_a?(Array); raise unless x == [nil] && Array(nil) == []
x = {a: 1}; y = [x]; raise unless y == [{a: 1}] && Array(x) == [[:a, 1]]
class ArrayLike
  def to_ary; [1]; end
  def to_a; [2]; end
  def [](n); [:indexed, n]; end
  def first; :not_indexed; end
end
x = ArrayLike.new; raise unless [*x] == [2] && Array(x) == [1]
raise unless x[0] == [:indexed, 0] && x.first == :not_indexed
h = {0 => :zero}; raise unless h[0] == :zero && h.first == [0, :zero]
s = 'abc'; raise unless s[0] == 'a' && !s.respond_to?(:first)
# any? filters false/nil values; intersect? only asks if an element is shared.
raise unless ([nil] & [nil]).any? == false && [nil].intersect?([nil]) == true
raise unless ([false] & [false]).none? == true && ![false].intersect?([false]) == false
# Moving a block receiver changes evaluation count/order and drops early exit.
events = []
left = [1, 2]
result = left.any? { |e| events << e; [2].member?(e) }
raise unless result && events == [1, 2]
events = []
result = left.intersect?((events << :once; [2]))
raise unless result && events == [:once]
# Mutation/identity and compound assignment are deliberately not rewritten.
a = [1]; a[0] += 1; raise unless a == [2]
a = nil; raise unless a&.intersection([])&.none? == nil && !a&.intersect?([]) == true
puts 'array witnesses ok'
"#;
fn witnesses(ruby: &str) {
    let out = Command::new(ruby).args(["-e", WITNESSES]).output().unwrap();
    assert!(out.status.success(), "{ruby}: {out:?}");
    assert_eq!(out.stdout, b"array witnesses ok\n");
}
#[test]
fn controlled_host_ruby_counterexamples() {
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        witnesses("ruby");
    }
}
#[test]
fn controlled_fixed_34_ruby_counterexamples() {
    if let Ok(ruby) = crate::support::runtime::ruby() {
        let out = Command::new(&ruby)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .starts_with(crate::support::runtime::version_prefix())
        );
        witnesses(&ruby);
    } else {
        eprintln!("SKIP native Ruby 3.4 runtime; set RUSTOCOP_RUBY34");
    }
}
