//! pinned examples and controlled runtime witnesses (never corpus code).
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/ComparableClamp";
const SOURCE: &str = "if 2 < 1; 1; elsif 3 < 2; 3; else; 2; end\n";
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
fn fixed_defaults_selectors_config_and_exclusions() {
    let r = rustocop::cops::Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled() && c.safe_fixes());
    assert_eq!(
        c.ruby_versions().minimum,
        Some(rustocop::cops::policy::RubyVersion(2, 4))
    );
    let d = project();
    assert!(
        String::from_utf8_lossy(&unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1).stdout)
            .contains(COP)
    );
    unchanged(d.path(), SOURCE.as_bytes(), &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled=false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled=false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        // Isolate this cop's enablement contract from independent comparison
        // diagnostics and default-on semicolon/one-line conditional corrections.
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &[
                "--except",
                "Style/MinMaxComparison,Style/IfWithSemicolon,Style/OneLineConditional",
                "--fix",
                "a.rb",
            ],
            0,
        );
        unchanged(d.path(), SOURCE.as_bytes(), &["--only", COP, "a.rb"], 1);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    assert!(
        String::from_utf8_lossy(&unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1).stdout)
            .contains(COP)
    );
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    for key in [
        "EnforcedStyle=\"other\"",
        "TargetRubyVersion=3.4",
        "SafeAutoCorrect=true",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 2);
    }
}
#[test]
fn directives_clean_and_error_files_never_write() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for target in [COP, "ComparableClamp", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                format!("{} # {prefix}:disable {target}\n", SOURCE.trim_end()),
            ] {
                unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 0);
            }
        }
        let s = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        let o = unchanged(d.path(), s.as_bytes(), &["--only", COP, "a.rb"], 1);
        assert_eq!(
            String::from_utf8_lossy(&o.stdout)
                .matches(&format!("({COP})"))
                .count(),
            1
        );
    }
    for (s, code) in [("p :clean\n", 0), ("(\n", 2)] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
}
#[test]
fn stdin_is_read_only() {
    let d = project();
    for fix in [false, true] {
        fs::write(d.path().join("a.rb"), "disk bytes\n").unwrap();
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
        assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }));
        if fix {
            assert!(o.stdout.is_empty(), "{o:?}");
            assert!(
                String::from_utf8_lossy(&o.stderr)
                    .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                "{o:?}"
            );
        }
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"disk bytes\n");
    }
}
#[derive(serde::Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
}
#[test]
fn all_20_applicable_pinned_examples_and_10_individually_withheld_corrections() {
    // RuboCop style/comparable_clamp_spec.rb, MIT, (c) 2012-26 Bozhidar Batsov.
    // Four Ruby <=2.3 examples are inapplicable at fixed 3.4 (unsupported Prism).
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/comparable_clamp_pinned.json")).unwrap();
    assert_eq!(cases.len(), 20);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 10);
    let d = project();
    let r = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&r, &config, &[COP.into()], &[]);
    for c in cases {
        let report = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(c.source.as_bytes()),
            &active,
            &config,
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        assert_eq!(
            report
                .result
                .offenses
                .iter()
                .map(|o| (o.line, o.col))
                .collect::<Vec<_>>(),
            c.positions,
            "spec:{}",
            c.spec_line
        );
        assert_eq!(
            report
                .result
                .offenses
                .iter()
                .map(|o| o.message.clone())
                .collect::<Vec<_>>(),
            c.messages,
            "spec:{}",
            c.spec_line
        );
        // Every upstream correction uses unknown calls (`x`, `low`, `high`,
        // `a + b`), so each remains reported without a behavior-changing edit.
        unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            if c.positions.is_empty() { 0 } else { 1 },
        );
        if let Some(after) = c.correction {
            let adapt = |s: &str| {
                s.replace("a + b", "2")
                    .replace('x', "2")
                    .replace("low", "1")
                    .replace("high", "3")
                    .replace("condition", "false")
                    .replace("do_something", "0")
                    .replace("(2).clamp", "2.clamp")
            };
            fs::write(d.path().join("a.rb"), adapt(&c.source)).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
            assert_eq!(
                fs::read_to_string(d.path().join("a.rb")).unwrap(),
                adapt(&after),
                "spec:{}",
                c.spec_line
            );
        }
    }
}
fn rubies() -> Vec<String> {
    let mut paths = vec!["ruby".into()];
    if let Ok(path) = crate::support::runtime::ruby() {
        paths.push(path);
    }
    paths
}
fn observe(ruby: &str, dir: &Path) -> Vec<u8> {
    let o = Command::new(ruby)
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap();
    assert!(o.status.success(), "{o:?}");
    o.stdout
}
#[test]
fn integer_values_binding_control_flow_and_negative_unary_semantics() {
    let d = project();
    for (x, low, high) in [(0, 1, 3), (2, 1, 3), (4, 1, 3), (-2, -3, -1), (1, 1, 1)] {
        let s = format!(
            "result = if {x} < {low}; {low}; elsif {high} < {x}; {high}; else; {x}; end\np [result, result.object_id]\n"
        );
        fs::write(d.path().join("a.rb"), &s).unwrap();
        let expected: Vec<_> = rubies().iter().map(|r| observe(r, d.path())).collect();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert!(
            fs::read_to_string(d.path().join("a.rb"))
                .unwrap()
                .contains(".clamp")
        );
        assert_eq!(
            rubies()
                .iter()
                .map(|r| observe(r, d.path()))
                .collect::<Vec<_>>(),
            expected
        );
    }
}
#[test]
fn comparator_evaluation_nil_nan_unordered_identity_and_reversed_bounds_are_unchanged() {
    let d = project();
    let cases = [
        (
            "log=[]; x=-> { log << :x; 2 }; low=-> { log << :low; 1 }; high=-> { log << :high; 3 }; result=if x.call < low.call; low.call; elsif high.call < x.call; high.call; else; x.call; end; p [result,log]\n",
            "[2, [:x, :low, :high, :x, :x]]\n",
        ),
        (
            "class C; def <(other); false; end; def clamp(*); raise; end; end; x=C.new; low=C.new; high=C.new; result=if x < low; low; elsif high < x; high; else; x; end; p result.equal?(x)\n",
            "true\n",
        ),
        (
            "class C; include Comparable; attr_reader :n; def initialize(n,log); @n=n; @log=log; end; def <=>(o); @log << [n,o.n]; n <=> o.n; end; end; log=[]; x=C.new(2,log); low=C.new(1,log); high=C.new(3,log); result=if x < low; low; elsif high < x; high; else; x; end; p [result.equal?(x),log]\n",
            "[true, [[2, 1], [3, 2]]]\n",
        ),
        (
            "x=Float::NAN; low=1; high=3; result=if x < low; low; elsif high < x; high; else; x; end; p result.nan?\n",
            "true\n",
        ),
        (
            "x=nil; low=1; high=3; begin; if x < low; low; elsif high < x; high; else; x; end; rescue NoMethodError; puts :nil_comparison; end\n",
            "nil_comparison\n",
        ),
        (
            "x=2; low=3; high=1; result=if x < low; low; elsif high < x; high; else; x; end; p result\n",
            "3\n",
        ),
        (
            "x=+'a'; low=+'a'; high=+'z'; result=if x < low; low; elsif high < x; high; else; x; end; p [result.equal?(x),result.equal?(low)]\n",
            "[true, false]\n",
        ),
        (
            "class C; def <(o); false; end; def <=>(o); nil; end; end; x=C.new; low=C.new; high=C.new; result=if x < low; low; elsif high < x; high; else; x; end; p result.equal?(x)\n",
            "true\n",
        ),
        ("p [[2,3].max,1].min\n", "1\n"),
        (
            "class Integer; def clamp(*); raise; end; end\np(if 2 < 1; 1; elsif 3 < 2; 3; else; 2; end)\n",
            "2\n",
        ),
    ];
    for (s, expected) in cases {
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        for ruby in rubies() {
            assert_eq!(observe(&ruby, d.path()), expected.as_bytes(), "{s}");
        }
    }
}
#[test]
fn source_envelopes_comments_line_data_and_encodings() {
    let d = project();
    for s in [
        "if 2 < 1\n 1 # keep\nelsif 3 < 2\n 3\nelse\n 2\nend\n",
        "if 2 < 1\n 1\nelsif 3 < 2\n 3\nelse\n 2\nend\np __LINE__\n",
        "p DATA.pos\nif 2 < 1; 1; elsif 3 < 2; 3; else; 2; end\n__END__\npayload\n",
        "if 2 < 3; 3; elsif 1 < 2; 1; else; 2; end\n",
        "if 2.0 < 1.0; 1.0; elsif 3.0 < 2.0; 3.0; else; 2.0; end\n",
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
    }
    for header in [
        b"\xef\xbb\xbf# encoding: UTF-8\r\n".as_slice(),
        b"# encoding: ISO-8859-1\r\n# caf\xe9\r\n",
    ] {
        let suffix = b"p __LINE__\r\n__END__\r\nraw\0\r\n";
        let mut s = header.to_vec();
        s.extend(SOURCE.replace('\n', "\r\n").as_bytes());
        s.extend(suffix);
        let mut expected = header.to_vec();
        expected.extend(b"2.clamp(1, 3)\r\n");
        expected.extend(suffix);
        fs::write(d.path().join("a.rb"), s).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), expected);
    }
}
#[test]
fn style_layout_and_independent_safe_siblings_stabilize() {
    let d = project();
    let s = format!("a = {SOURCE}b = {SOURCE}p [a,b,'ok']\n");
    for selector in [
        COP.to_string(),
        format!(
            "{COP},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException"
        ),
        "Style,Layout".into(),
        String::new(),
    ] {
        fs::write(d.path().join("a.rb"), &s).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec![];
            if !selector.is_empty() {
                args.extend(["--only", selector.as_str()]);
            }
            args.extend(["--fix", "a.rb"]);
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                matches!(o.status.code(), Some(0 | 1)) && o.stderr.is_empty(),
                "{o:?}"
            );
            for ruby in rubies() {
                assert_eq!(observe(&ruby, d.path()), b"[2, 2, \"ok\"]\n");
            }
            if before == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb"))
                .unwrap()
                .matches(".clamp")
                .count(),
            2
        );
    }
}
