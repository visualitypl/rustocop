//! CLI contracts and controlled Ruby semantic observations.
use std::{fs, path::Path, process::Command};
const SEMI: &str = "Style/IfWithSemicolon";
const INVERSE: &str = "Style/InvertibleUnlessCondition";
const BAD: &str = "if ready; work else other end\n";
const GOOD: &str = "ready ? work : other\n";
const INV_BAD: &str = "work unless odd?\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(
    root: &Path,
    cop: &str,
    source: &[u8],
    args: &[&str],
    count: usize,
    status: Option<i32>,
) -> std::process::Output {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(root, args);
    if let Some(status) = status {
        assert_eq!(o.status.code(), Some(status), "{o:?}");
    } else {
        assert_ne!(o.status.code(), Some(2), "{o:?}");
    }
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(cop))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    o
}
#[test]
fn legacy_encoded_inverse_diagnostics_and_class_exemptions_never_write() {
    let d = project();
    for (condition, preferred) in [
        (b"x < \xc9\xe9".as_slice(), None),
        (b"x < NS::\xc9\xe9", None),
        (b"x.odd? && x < ::Camel::\xc9\xe9", None),
        (b"x.odd? || (x < NS::\xc9\xe9)", None),
        (b"x < A\xaa", Some("x >= A�")),
        (b"x < \xc9\xc9", Some("x >= ��")),
        (b"x < Camel::\xc9\xc9", Some("x >= Camel::��")),
        (b"x < ::Camel::\xc9\xc9", Some("x >= ::Camel::��")),
        (b"x < \xc9\xe9::\xc9\xc9", Some("x >= ��::��")),
        (
            b"x.odd? && x < NS::\xc9\xc9",
            Some("x.even? || x >= NS::��"),
        ),
        (
            b"x.odd? || (\xe9 != \xc9\xc9)",
            Some("x.even? && (� == ��)"),
        ),
        (b"!\xe9", Some("�")),
        (b"any?(\xe9, \xc9\xc9)", Some("none?(�, ��)")),
        (b"x < __ENCODING__", Some("x >= __ENCODING__")),
    ] {
        let mut source = b"# encoding: ISO-8859-1\nwork unless ".to_vec();
        source.extend_from_slice(condition);
        source.push(b'\n');
        let count = usize::from(preferred.is_some());
        for fix in [false, true] {
            let mut args = vec!["--only", INVERSE];
            if fix {
                args.push("--fix");
            }
            args.push("a.rb");
            let output = unchanged(d.path(), INVERSE, &source, &args, count, Some(count as i32));
            assert!(output.stderr.is_empty(), "{output:?}");
            if let Some(preferred) = preferred {
                let expected = format!(
                    "a.rb:2:1: C: Prefer `if {preferred}` over `unless {}`. ({INVERSE})",
                    String::from_utf8_lossy(condition)
                );
                assert_eq!(
                    String::from_utf8(output.stdout).unwrap().lines().next(),
                    Some(expected.as_str())
                );
            }
        }
    }
}
#[test]
fn defaults_opt_in_selectors_config_and_no_implicit_writes() {
    let d = project();
    for (cop, source, default) in [(SEMI, BAD, 1), (INVERSE, INV_BAD, 0)] {
        for (config, selectors, count) in [
            (String::new(), vec![], default),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("a.rb");
            unchanged(d.path(), cop, source.as_bytes(), &args, count, None);
        }
        let registry = rustocop::cops::Registry::new();
        let cop_impl = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert_eq!(cop_impl.default_enabled(), default == 1);
        assert_eq!(cop_impl.safe_fixes(), cop == SEMI);
        assert_eq!(cop_impl.ruby_versions(), Default::default());
        assert!(cop_impl.path_policy().is_none() && cop_impl.max_setting().is_none());
    }
}
#[test]
fn excludes_and_both_directive_prefixes_prevent_edits() {
    let d = project();
    for (cop, source) in [(SEMI, BAD), (INVERSE, INV_BAD)] {
        for config in [
            "exclude = [\"a.rb\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
                Some(0),
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                unchanged(
                    d.path(),
                    cop,
                    format!("# {prefix}:disable {target}\n{source}").as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                    Some(0),
                );
            }
            unchanged(
                d.path(),
                cop,
                format!("# {prefix}:disable-next {cop}\n{source}").as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
                Some(0),
            );
            unchanged(
                d.path(),
                cop,
                format!("{} # {prefix}:disable {cop}\n", source.trim_end()).as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
                Some(0),
            );
        }
    }
    fs::write(
        d.path().join("a.rb"),
        format!("# rubocop:disable {SEMI}\n{BAD}# rubocop:enable {SEMI}\n{BAD}"),
    )
    .unwrap();
    let o = cli(d.path(), &["--only", SEMI, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        format!("# rubocop:disable {SEMI}\n{BAD}# rubocop:enable {SEMI}\n{GOOD}")
    );
}
#[test]
fn safe_canonical_fixes_stabilize_and_unsafe_pinned_examples_keep_bytes_and_mtime() {
    let d = project();
    fs::write(d.path().join("a.rb"), BAD).unwrap();
    let o = cli(d.path(), &["--only", SEMI, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), GOOD);
    unchanged(
        d.path(),
        SEMI,
        GOOD.as_bytes(),
        &["--only", SEMI, "--fix", "a.rb"],
        0,
        Some(0),
    );
    // RuboCop examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/invertible_unless_condition_pinned.json"
    ))
    .unwrap();
    for c in cases {
        unchanged(
            d.path(),
            INVERSE,
            c["source"].as_str().unwrap().as_bytes(),
            &["--only", INVERSE, "--fix", "a.rb"],
            c["positions"].as_array().unwrap().len(),
            None,
        );
    }
}
#[test]
fn source_sensitive_clean_and_error_files_are_never_rewritten() {
    let d = project();
    for source in [
        b"unless ready; x = 1 else x end\n".as_slice(),
        b"if ready; work; more end\np __LINE__\n".as_slice(),
        b"if ready; DATA.pos else 0 end\n__END__\npayload\n".as_slice(),
        b"# coding: ISO-8859-1\nif ready; '\xe9' else 0 end\n".as_slice(),
        b"if ready; work end # encoding: ascii-8bit\n".as_slice(),
    ] {
        unchanged(
            d.path(),
            SEMI,
            source,
            &["--only", SEMI, "--fix", "a.rb"],
            1,
            Some(1),
        );
    }
    for cop in [SEMI, INVERSE] {
        unchanged(
            d.path(),
            cop,
            b"work\n",
            &["--only", cop, "--fix", "a.rb"],
            0,
            Some(0),
        );
        unchanged(
            d.path(),
            cop,
            b"if broken\n",
            &["--only", cop, "--fix", "a.rb"],
            0,
            Some(2),
        );
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nmax = 80\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            cop,
            BAD.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            0,
            Some(2),
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn values_binding_evaluation_count_branch_effects_and_exception_flow_survive() {
    // Only this controlled fixture is executed, never corpus code.
    let source = r#"$trace = []
def observe(x); $trace << x; x; end
def choose(a,b)
  x = if observe(a); observe(:yes) else observe(:no) end
  y = unless observe(b); observe(:unless) else observe(:else) end
  [x,y]
end
def flow(a)
  begin
    if a; return :early else raise "failure" end
  rescue RuntimeError
    :rescued
  ensure
    $trace << :ensure
  end
end
def lexical(a)
  unless a; value = 7 else defined?(value) end
end
def branches(a,b)
  if a; :a elsif b; :b else :c end
end
class Witness
  def odd?; :truthy; end
end
w = Witness.new
work = :original
work = :changed unless w.odd?
p [nil,false,true].product([nil,false,true]).map { |a,b| choose(a,b) }
p [flow(true),flow(false),lexical(false),lexical(true)]
p [branches(true,false),branches(false,true),branches(false,false),work]
@value = nil
p(if @value ||= false; :yes else :no end)
p @value
def invoke; yield 12; end
p(if true; invoke &:to_s else :miss end)
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let selected = format!("{SEMI},{INVERSE}");
    let o = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    assert!(
        after.contains("observe(a) ? observe(:yes) : observe(:no)")
            && after.contains("unless a; value = 7 else defined?(value) end"),
        "{after}"
    );
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby)
            .arg(d.path().join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby)
            .arg(d.path().join("a.rb"))
            .output()
            .unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
        let observed = String::from_utf8_lossy(&after.stdout);
        assert!(
            observed.contains("[:early, :rescued, 7, \"local-variable\"]")
                && observed.contains("[:a, :b, :c, :original]\n:no\nfalse\n"),
            "{observed}"
        );
    }
}
#[test]
fn range_endpoint_corrections_preserve_runtime_values() {
    // Controlled witnesses for both endpoints and both beginless/endless forms.
    let mut source = String::new();
    let mut expected = String::new();
    for operator in ["..", "..."] {
        for (condition, value) in [("true", 2), ("false", 3)] {
            let branch = format!("if {condition}; 2 else 3 end");
            for (expression, range) in [
                (
                    format!("{branch}{operator}5"),
                    format!("{value}{operator}5"),
                ),
                (
                    format!("1{operator}{branch}"),
                    format!("1{operator}{value}"),
                ),
                (format!("{operator}{branch}"), format!("{operator}{value}")),
                (format!("{branch}{operator}"), format!("{value}{operator}")),
            ] {
                source.push_str(&format!("p({expression})\n"));
                expected.push_str(&format!("{range}\n"));
            }
        }
    }
    let d = project();
    fs::write(d.path().join("before.rb"), &source).unwrap();
    fs::write(d.path().join("a.rb"), &source).unwrap();
    let o = cli(d.path(), &["--only", SEMI, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(!after.contains("if "), "{after}");
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        for file in ["before.rb", "a.rb"] {
            let output = Command::new(&ruby)
                .arg(d.path().join(file))
                .output()
                .unwrap();
            assert!(output.status.success(), "{ruby} {file}: {output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                expected,
                "{ruby} {file}"
            );
        }
    }
}
#[test]
fn mixed_safe_style_and_layout_fixes_stabilize_within_eight_explicit_passes() {
    let d = project();
    fs::write(
        d.path().join("a.rb"),
        "def f\n if a; return ({ :key => 'value' }) else raise ('failure') end\nend\n",
    )
    .unwrap();
    let selected = format!(
        "{SEMI},{INVERSE},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/IndentationWidth,Layout/EndAlignment"
    );
    let mut stable = false;
    for _ in 0..8 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        let o = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        if old == fs::read(d.path().join("a.rb")).unwrap() {
            stable = true;
            break;
        }
    }
    assert!(stable);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(
        after.contains("key: \"value\"") && !after.contains("if a;"),
        "{after}"
    );
}
