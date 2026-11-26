//! controlled synthetic programs only; command literals are never run.
use crate::support::cli::project;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const BRACES: &str = "Style/HashAsLastArrayItem";
const CONVERSION: &str = "Style/HashConversion";
const BOTH: &str = "Style/HashAsLastArrayItem,Style/HashConversion";
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
fn fixed(s: &[u8], only: &str) -> Vec<u8> {
    let d = project();
    let p = d.path().join("a.rb");
    fs::write(&p, s).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    c.current_dir(d.path());
    if !only.is_empty() {
        c.args(["--only", only]);
    }
    let o = c.args(["--fix", "a.rb"]).output().unwrap();
    assert!(matches!(o.status.code(), Some(0 | 1)), "{only}: {o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    fs::read(p).unwrap()
}
#[test]
fn registration_defaults_capabilities_and_plain_lint_are_read_only() {
    let d = project();
    let r = rustocop::cops::Registry::new();
    for (cop, s, safe) in [
        (BRACES, "p [a: 1]\n", true),
        (CONVERSION, "p Hash[ary]\n", false),
    ] {
        let cops: Vec<_> = r.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(cops.len(), 1);
        let c = cops[0];
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), safe);
        assert!(c.path_policy().is_none());
        assert!(c.ruby_versions().minimum.is_none());
        assert!(c.ruby_versions().maximum.is_none());
        assert!(c.max_setting().is_none());
        let o = unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
    }
}
#[test]
fn selectors_config_fixed_options_and_relative_excludes() {
    let d = project();
    fs::create_dir(d.path().join("lib")).unwrap();
    for (cop, s, after, safe) in [
        (BRACES, "p [a: 1]\n", "p [{a: 1}]\n", true),
        (CONVERSION, "p Hash[ary]\n", "p Hash[ary]\n", false),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(d.path().join("rustocop.toml"),format!("[cops.Style]\nenabled = false\n[cops.Layout]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n")).unwrap();
        cli(d.path(), &["--fix", "a.rb"], i32::from(!safe));
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        unchanged(
            d.path(),
            after.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            i32::from(!safe),
        );
        for config in [
            "exclude = [\"lib/a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                &d.path().join("lib"),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        let option = if safe {
            "EnforcedStyle = \"no_braces\""
        } else {
            "AllowSplatArgument = false"
        };
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 2);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_all_forms_and_atomic_interior_disabled_spans() {
    let d = project();
    for (cop, s) in [(BRACES, "p [a: 1]"), (CONVERSION, "p Hash[ary]")] {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{s}\n"),
                    format!("# {prefix}:disable-next {target}\n{s}\n"),
                    format!("{s} # {prefix}:disable {target}\n"),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
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
    }
    let s = "p [a: 1,\n# rubocop:disable Style/HashAsLastArrayItem\nb: 2\n# rubocop:enable Style/HashAsLastArrayItem\n]\np [c: 3]\n";
    let a = String::from_utf8(fixed(s.as_bytes(), BRACES)).unwrap();
    assert_eq!(a, s.replace("[c: 3]", "[{c: 3}]"));
}
#[test]
fn individual_encoding_withhold_does_not_abort_safe_siblings_or_files() {
    let d = project();
    let s = b"# encoding: ISO-8859-1\np [\"caf\xe9\" => 1]\np [safe: 2]\n";
    fs::write(d.path().join("safe.rb"), "p [safe: 3]\n").unwrap();
    let o = unchanged(
        d.path(),
        b"# encoding: ISO-8859-1\np [\"caf\xe9\" => 1]\n",
        &["--only", BRACES, "--fix", "a.rb", "safe.rb"],
        1,
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("1 correction"));
    assert_eq!(
        fs::read_to_string(d.path().join("safe.rb")).unwrap(),
        "p [{safe: 3}]\n"
    );
    assert_eq!(
        fixed(s, BRACES),
        b"# encoding: ISO-8859-1\np [\"caf\xe9\" => 1]\np [{safe: 2}]\n"
    );
}
fn rubies() -> Vec<String> {
    let mut r = Vec::new();
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        r.push("ruby".to_owned());
    } else {
        eprintln!("host Ruby runtime gate skipped: ruby is unavailable");
    }
    if let Ok(path) = crate::support::runtime::ruby() {
        let o = Command::new(&path)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(o.status.success());
        assert!(
            o.stdout
                .starts_with(crate::support::runtime::version_prefix().as_bytes()),
            "{o:?}"
        );
        r.push(path);
    } else {
        eprintln!("native Ruby 3.4 gate skipped: set RUSTOCOP_RUBY34");
    }
    r
}
fn observe(ruby: &str, source: &[u8]) -> Output {
    let d = project();
    let p = d.path().join("program.rb");
    fs::write(&p, source).unwrap();
    let o = Command::new(ruby)
        .current_dir(d.path())
        .args(["-W:deprecated", "program.rb"])
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{ruby}: {o:?}\n{}",
        String::from_utf8_lossy(source)
    );
    o
}
fn equal_runtime(ruby: &str, before: &[u8], after: &[u8]) {
    let b = observe(ruby, before);
    let a = observe(ruby, after);
    assert_eq!(
        String::from_utf8_lossy(&a.stdout),
        String::from_utf8_lossy(&b.stdout),
        "{ruby}: {}",
        String::from_utf8_lossy(after)
    );
    // No file/line-sensitive warnings should be introduced by wrappers.
    assert_eq!(
        String::from_utf8_lossy(&a.stderr),
        String::from_utf8_lossy(&b.stderr),
        "{ruby}"
    );
}
#[test]
fn native_first_kwsplat_exemption_and_unsafe_conversion_witnesses() {
    let d = project();
    let s = r#"begin; p [**nil]; rescue TypeError; p :nil_error; end
begin; p [{**nil}]; rescue TypeError; p :nil_error; end
p [**{}]; p [{**{}}]
p Hash[[[1,2],[3]]]
begin; p [[1,2],[3]].to_h; rescue => e; p [e.class.name,e.message]; end
class Custom
  def to_hash; {a: 1}; end
end
p Hash[Custom.new]
begin; p Custom.new.to_h; rescue NoMethodError; p :no_to_h; end
"#;
    for ruby in rubies() {
        let o = observe(&ruby, s.as_bytes());
        let out = String::from_utf8(o.stdout).unwrap();
        if ruby != "ruby" {
            assert!(out.starts_with("[]\n[{}]\n[]\n[{}]\n"), "{ruby}: {out}");
        }
        assert!(out.contains("[]\n[{}]\n"), "{ruby}: {out}");
        assert!(out.contains("ArgumentError"));
        assert!(out.contains("no_to_h"));
    }
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", BOTH, "--fix", "a.rb"],
        1,
    );
}
#[test]
fn selected_peers_and_actual_defaults_stabilize_within_six_explicit_passes() {
    let s =
        "x = [1, :a => 2, b: 3,]\np(x)\nx = [a: __LINE__, b: \"value\"]\np(x)\nHash[[[1,2],[3]]]\n";
    for selector in [
        BOTH,
        "Style/HashAsLastArrayItem,Style/HashConversion,Style/TrailingCommaInArrayLiteral,Style/TrailingCommaInHashLiteral,Style/MethodCallWithArgsParentheses,Style/HashSyntax,Style/StringLiterals,Style/RedundantReturn,Style/Semicolon,Style/SignalException,Style/MethodCallWithoutArgsParentheses,Style/WordArray,Style/SymbolArray,Style/Sample,Layout",
        "Style,Layout",
        "",
    ] {
        let mut current = s.as_bytes().to_vec();
        let mut stable = false;
        for _ in 0..6 {
            let a = fixed(&current, selector);
            for ruby in rubies() {
                equal_runtime(&ruby, &current, &a);
            }
            if a == current {
                stable = true;
                break;
            }
            current = a;
        }
        assert!(stable, "{selector}: {}", String::from_utf8_lossy(&current));
        assert!(String::from_utf8_lossy(&current).contains("Hash["));
        assert!(!String::from_utf8_lossy(&current).contains("x = [a:"));
    }
}
#[test]
fn bounded_many_arrays_arguments_and_unsafe_siblings() {
    let source = "[a: 1, b: 2]\nHash[Hash[ary]]\n".repeat(6000);
    let a = fixed(source.as_bytes(), BOTH);
    assert_eq!(
        a,
        "[{a: 1, b: 2}]\nHash[Hash[ary]]\n".repeat(6000).as_bytes()
    );
    assert_eq!(fixed(&a, BOTH), a);
    let s = format!("Hash[{}]\n", vec!["Hash[ary]"; 6000].join(","));
    let d = project();
    let o = unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", CONVERSION, "--fix", "a.rb"],
        1,
    );
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .matches(&format!("({CONVERSION})"))
            .count(),
        1
    );
}
