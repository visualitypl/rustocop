//! pinned Ruby numeric idioms, CLI contracts and controlled observations.
#[path = "support/stdin.rs"]
mod stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/EvenOdd";
const CASES: [(&str, &str); 3] = [
    ("Style/BitwisePredicate", "(variable & flags).positive?\n"),
    (COP, "x % 2 == 0\n"),
    ("Style/ExponentialNotation", "10e6\n"),
];
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
    let mtime = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), s);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), mtime);
    o
}
#[test]
fn defaults_selectors_config_excludes_and_fixed_version() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (cop, s) in CASES {
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let c = r.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), cop == COP);
        if cop == "Style/BitwisePredicate" {
            assert_eq!(
                c.ruby_versions().minimum,
                Some(rustocop::cops::policy::RubyVersion(2, 5))
            );
        }
        assert!(
            String::from_utf8_lossy(&unchanged(d.path(), s.as_bytes(), &["a.rb"], 1).stdout)
                .contains(cop)
        );
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), &config).unwrap();
            // NumericPredicate independently reports the zero comparison when
            // EvenOdd alone is disabled; neither rule rewrites unknown x.
            let numeric_remains = cop == COP && !config.contains("[cops.Style]");
            let o = unchanged(
                d.path(),
                s.as_bytes(),
                &["--fix", "a.rb"],
                i32::from(numeric_remains),
            );
            if numeric_remains {
                assert!(String::from_utf8_lossy(&o.stdout).contains("Style/NumericPredicate"));
            }
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
        }
        for key in [
            "EnforcedStyle=\"engineering\"",
            "TargetRubyVersion=2.4",
            "SafeAutoCorrect=true",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 2);
        }
    }
}
#[test]
fn directives_clean_and_error_files_retain_bytes_and_mtime() {
    let d = project();
    for (cop, s) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{s}"),
                    format!("# {prefix}:disable-next {target}\n{s}"),
                    format!("{} # {prefix}:disable {target}\n", s.trim_end()),
                ] {
                    unchanged(
                        d.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "a.rb"],
                        0,
                    );
                }
            }
            let s = format!("# {prefix}:disable {cop}\n{s}# {prefix}:enable {cop}\n{s}");
            let o = unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (s, code) in [("p :clean\n", 0), ("(\n", 2), ("01e2\n", 2)] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
}
#[derive(serde::Deserialize)]
struct Case {
    cop: String,
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
}
#[test]
fn all_46_pinned_examples_and_27_correction_calls() {
    // RuboCop style/{bitwise_predicate,even_odd,exponential_notation}_spec.rb,
    // MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    // Three pre-2.5 examples and 18 alternate-style examples are inapplicable.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/numeric_predicates_pinned.json")).unwrap();
    assert_eq!(cases.len(), 46);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 27);
    let d = project();
    let r = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let mut applied = 0;
    for c in cases {
        let active = rustocop::cops::select_active(&r, &config, std::slice::from_ref(&c.cop), &[]);
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
            "{}:{}",
            c.cop,
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
            "{}:{}",
            c.cop,
            c.spec_line
        );
        if c.cop == COP && c.source == "1 % 2 != 1\n" {
            fs::write(d.path().join("a.rb"), &c.source).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
            assert_eq!(
                fs::read_to_string(d.path().join("a.rb")).unwrap(),
                c.correction.unwrap()
            );
            applied += 1;
        } else {
            // 9 unsafe bitwise and 17 uncertain EvenOdd correction calls each
            // retain the original source; no offense is hidden for safety.
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", &c.cop, "--fix", "a.rb"],
                if c.positions.is_empty() { 0 } else { 1 },
            );
        }
    }
    assert_eq!(applied, 1);
}
#[test]
fn safe_literal_matrix_is_canonical_and_stable() {
    let d = project();
    for receiver in [
        "0", "1", "-1", "-3", "+2", "0x10", "0b11", "07", "1_000", "(2)",
    ] {
        for (op, n, method) in [
            ("==", 0, "even"),
            ("!=", 0, "odd"),
            ("==", 1, "odd"),
            ("!=", 1, "even"),
        ] {
            let s = format!("({receiver} % 02) {op} {n}\n");
            let base = if receiver.starts_with(['-', '+']) {
                format!("({receiver})")
            } else {
                receiver.into()
            };
            let after = format!("{base}.{method}?\n");
            fs::write(d.path().join("a.rb"), s).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
            unchanged(
                d.path(),
                after.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
    }
}
#[test]
fn source_envelopes_comments_line_values_and_data_are_preserved() {
    let d = project();
    for s in [
        "\u{feff}2 % 2 == 0\r\n",
        "# coding: us-ascii\n2 % 2 == 0\n",
        "2 % 2 == 0\n__LINE__\n",
        "2 % 2 == 0\n__END__\npayload\r\n",
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "a.rb"], 1);
        fs::write(d.path().join("a.rb"), s).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            s.replace("2 % 2 == 0", "2.even?")
        );
    }
    for s in [
        "(2 % #keep\n2) == 0\n",
        "2 % 2 == 0 # rubocop:disable Lint\n",
        "(2 %\n2) == 0\n__LINE__\n",
        "2 % 2 == 0\np File.binread(__FILE__)\n",
        "2 % 2 == 0\nDATA.rewind; p DATA.read\n__END__\npayload\n",
        "2 % 2 == 0\nclass Integer; def even?; false; end; end\n",
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
    }
}
fn rubies() -> Vec<String> {
    let mut r = vec!["ruby".into()];
    if let Ok(p) = std::env::var("RUSTOCOP_RUBY34") {
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
fn runtime_preserves_negative_zero_parity_values_and_line_numbers() {
    let d = project();
    let s = "result = [(-3 % 2) == 1, -2 % 2 != 1, 0 % 2 == 0, +2 % 2 != 0, 0x11 % 2 == 1, __LINE__]\np result\np __LINE__\n";
    for ruby in rubies() {
        fs::write(d.path().join("a.rb"), s).unwrap();
        let before = observe(&ruby, d.path());
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), s);
        assert_eq!(before, observe(&ruby, d.path()));
        assert_eq!(before, b"[true, true, true, false, true, 1]\n3\n");
    }
}
#[test]
fn runtime_custom_numeric_dispatch_evaluation_and_control_flow_are_not_rewritten() {
    let d = project();
    let s = "class NumericLike\n  def %(n); puts [:modulo,n].inspect; 0; end\n  def even?; puts :wrong; false; end\n  def &(flags); puts :intersection; 0; end\n  def nobits?(flags); puts :wrong; false; end\nend\ndef value; puts :receiver; NumericLike.new; end\np(value % 2 == 0)\np((value & 3).zero?)\np(1.0 % 2 == 1)\np(1r % 2 == 1)\np(-3 % 2 == 1)\n";
    for ruby in rubies() {
        fs::write(d.path().join("a.rb"), s).unwrap();
        let before = observe(&ruby, d.path());
        unchanged(
            d.path(),
            s.as_bytes(),
            &[
                "--only",
                "Style/EvenOdd,Style/BitwisePredicate",
                "--fix",
                "a.rb",
            ],
            1,
        );
        assert_eq!(before, observe(&ruby, d.path()));
        assert_eq!(
            before,
            b"receiver\n[:modulo, 2]\ntrue\nreceiver\nintersection\ntrue\ntrue\ntrue\ntrue\n"
        );
    }
    for ruby in rubies() {
        let s = "def run\n [1,2].each do |n|\n  next if n % 2 == 0\n  return :odd if n % 2 != 0\n end\nend\np run\n";
        fs::write(d.path().join("a.rb"), s).unwrap();
        let before = observe(&ruby, d.path());
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        assert_eq!(before, observe(&ruby, d.path()));
    }
}
#[test]
fn runtime_visible_operator_override_and_negative_bitmask_witnesses() {
    let d = project();
    for ruby in rubies() {
        let s = "class Integer\n def %(other); 0; end\nend\np(3 % 2 == 0)\n";
        fs::write(d.path().join("a.rb"), s).unwrap();
        let before = observe(&ruby, d.path());
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        assert_eq!(before, b"true\n");
        assert_eq!(before, observe(&ruby, d.path()));
        // For negative masks `positive?` and Integer#anybits? differ even on
        // built-in Integers; BitwisePredicate must never emit safe corrections.
        let s = "p((-1 & -1).positive?)\np((-1).anybits?(-1))\n";
        fs::write(d.path().join("a.rb"), s).unwrap();
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", "Style/BitwisePredicate", "--fix", "a.rb"],
            1,
        );
        assert_eq!(observe(&ruby, d.path()), b"false\ntrue\n");
    }
}
#[test]
fn peer_and_default_corrections_stabilize_without_oscillation() {
    let d = project();
    let s = "values = [2 % 2 == 0, 3 % 2 != 0];\np values\n";
    for only in [
        Some("Style/EvenOdd,Style/RedundantParentheses,Style/Semicolon,Layout"),
        None,
    ] {
        fs::write(d.path().join("a.rb"), s).unwrap();
        let mut stable = false;
        for _ in 0..4 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let args = if let Some(c) = only {
                vec!["--only", c, "--fix", "a.rb"]
            } else {
                vec!["--fix", "a.rb"]
            };
            cli(d.path(), &args, 0);
            let after = fs::read(d.path().join("a.rb")).unwrap();
            stable |= before == after;
        }
        assert!(stable);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "values = [2.even?, 3.odd?]\np values\n"
        );
    }
}
#[test]
fn stdin_does_not_write_even_when_fix_is_rejected() {
    let d = project();
    for (cop, s) in CASES {
        for fix in [false, true] {
            fs::write(d.path().join("a.rb"), "disk bytes\n").unwrap();
            let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            c.current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"]);
            if fix {
                c.arg("--fix");
            }
            let mut child = c
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            stdin::write_stdin(child.stdin.take().unwrap(), s.as_bytes(), fix).unwrap();
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
}
