//! fixed numeric-predicate policy and detection-only CLI contracts.
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/NumericPredicate";
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
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let mtime = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), mtime);
    o
}
#[derive(serde::Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    toml: String,
    correction: Option<String>,
}
#[test]
fn all_23_pinned_calls_and_14_unsafe_correction_calls() {
    // RuboCop spec/rubocop/cop/style/numeric_predicate_spec.rb,
    // MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    // 25 alternate-config and five pre-2.5/unsupported-Prism examples do not apply.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/numeric_predicate_pinned.json")).unwrap();
    assert_eq!(cases.len(), 23);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 14);
    let d = project();
    let r = rustocop::cops::Registry::new();
    for c in cases {
        fs::write(d.path().join("rustocop.toml"), &c.toml).unwrap();
        let raw = toml::from_str(&c.toml).unwrap();
        let config = rustocop::config::ResolvedConfig::resolve(
            raw,
            Some(&d.path().join("rustocop.toml")),
            &r,
        )
        .unwrap();
        let active = rustocop::cops::select_active(&r, &config, &[COP.into()], &[]);
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
        assert!(
            report
                .result
                .offenses
                .iter()
                .all(|o| o.correction.is_none())
        );
        if let Some(correction) = c.correction {
            assert_ne!(correction, c.source);
        }
        unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            if c.positions.is_empty() { 0 } else { 1 },
        );
    }
}
#[test]
fn default_enabled_selectors_config_and_strict_fixed_options() {
    let d = project();
    let r = rustocop::cops::Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled());
    assert!(!c.safe_fixes());
    let source = b"number == 0\n";
    assert!(
        String::from_utf8_lossy(&unchanged(d.path(), source, &["a.rb"], 1).stdout).contains(COP)
    );
    unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    unchanged(
        d.path(),
        source,
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled=false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled=false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--fix", "a.rb"], 0);
        unchanged(d.path(), source, &["--only", COP, "a.rb"], 1);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    unchanged(d.path(), source, &["a.rb"], 1);
    for option in [
        "EnforcedStyle=\"comparison\"",
        "AllowedMethods=[\"where\"]",
        "AllowedPatterns=[\"zero\"]",
        "TargetRubyVersion=2.2",
        "max=1",
        "SafeAutoCorrect=true",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(d.path(), source, &["--fix", "a.rb"], 2);
    }
}
#[test]
fn bitwise_dependency_uses_configured_enablement_not_cli_selection_or_directives() {
    let d = project();
    let source = b"(number & flags) == 0\n";
    for (config, expected) in [
        ("".into(), 0),
        (
            "[cops.\"Style/BitwisePredicate\"]\nenabled=false\n".into(),
            1,
        ),
        (
            format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
            1,
        ),
        (
            "[cops.Style]\nenabled=false\n[cops.\"Style/BitwisePredicate\"]\nenabled=true\n".into(),
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            source,
            &[
                "--only",
                COP,
                "--except",
                "Style/BitwisePredicate",
                "--fix",
                "a.rb",
            ],
            expected,
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    unchanged(
        d.path(),
        b"(number & flags) == 0 # rubocop:disable Style/BitwisePredicate\n",
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    let o = unchanged(
        d.path(),
        source,
        &[
            "--only",
            "Style/NumericPredicate,Style/BitwisePredicate",
            "--fix",
            "a.rb",
        ],
        1,
    );
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(!text.contains(COP));
    assert!(text.contains("Style/BitwisePredicate"));
}
#[test]
fn path_policy_user_excludes_and_both_directive_prefixes() {
    let d = project();
    fs::create_dir(d.path().join("spec")).unwrap();
    let s = b"number == 0\n";
    fs::write(d.path().join("spec/a.rb"), s).unwrap();
    cli(d.path(), &["--only", COP, "--fix", "spec/a.rb"], 0);
    assert_eq!(fs::read(d.path().join("spec/a.rb")).unwrap(), s);
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), s, &["--only", COP, "--fix", "a.rb"], 0);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "NumericPredicate", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\nnumber == 0\n"),
                format!("# {prefix}:disable-next {target}\nnumber == 0\n"),
                format!("number == 0 # {prefix}:disable {target}\n"),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
        }
        let source = format!(
            "# {prefix}:disable {COP}\nnumber == 0\n# {prefix}:enable {COP}\nnumber == 0\n"
        );
        let o = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert!(String::from_utf8_lossy(&o.stdout).contains("a.rb:4:1:"));
    }
}
#[test]
fn no_writes_for_source_envelopes_clean_and_error_files() {
    let d = project();
    for source in [
        "\u{feff}number == 0\r\n",
        "# coding: us-ascii\nnumber == 0\n",
        "# coding: windows-1252\n# café\nnumber == 0\n",
        "number == 0\n__LINE__\n",
        "number == 0\n__END__\npayload\r\n",
        "foo - # keep\n1 == 0\n",
    ] {
        unchanged(d.path(), source.as_bytes(), &["--only", COP, "a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
    unchanged(
        d.path(),
        b"# coding: windows-1252\n# caf\xe9\nnumber == 0\n",
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
    for (source, code) in [("number.zero?\n", 0), ("number != 0\n", 0), ("(\n", 2)] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
}
fn rubies() -> Vec<String> {
    let mut r = vec!["ruby".into()];
    if let Ok(path) = crate::support::runtime::ruby() {
        r.push(path);
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
fn custom_numeric_dispatch_evaluation_and_control_flow_are_not_rewritten() {
    let d = project();
    let source = "class NumericLike\n def ==(n); puts [:equal,n].inspect; true; end\n def >(n); puts [:greater,n].inspect; true; end\n def <(n); puts [:less,n].inspect; true; end\n def zero?; false; end\n def positive?; false; end\n def negative?; false; end\nend\ndef value; puts :receiver; NumericLike.new; end\ndef run\n return :yes if value == 0 && value > 0 && value < 0\n :no\nend\np run\nx = NumericLike.new\np(x == 0)\np(x.zero?)\np [1.0 > 0, 1r > 0, 1i == 0, -0 == 0, __LINE__]\n";
    for ruby in rubies() {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let before = observe(&ruby, d.path());
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert_eq!(before, observe(&ruby, d.path()));
        assert_eq!(before, b"receiver\n[:equal, 0]\nreceiver\n[:greater, 0]\nreceiver\n[:less, 0]\n:yes\n[:equal, 0]\ntrue\nfalse\n[true, true, false, true, 18]\n");
    }
}
#[test]
fn overridden_integer_operators_are_not_replaced_even_for_literals() {
    let d = project();
    let source = "class Integer\n def ==(n); false; end\n def -@; 1; end\nend\np(0 == 0)\np(0.zero?)\np(- 0 == 0)\np __LINE__\n";
    for ruby in rubies() {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let before = observe(&ruby, d.path());
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert_eq!(before, observe(&ruby, d.path()));
        assert_eq!(before, b"false\ntrue\nfalse\n8\n");
    }
}
#[test]
fn bounded_6000_siblings_have_exact_positions_and_no_corrections() {
    let r = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&r, &config, &[COP.into()], &[]);
    let source = "x == 0\n".repeat(6000);
    let report = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(source.as_bytes()),
        &active,
        &config,
    );
    assert!(report.errors.is_empty() && report.internal_errors.is_empty());
    assert_eq!(report.result.offenses.len(), 6000);
    for (i, o) in report.result.offenses.iter().enumerate() {
        assert_eq!((o.line, o.col), (i as u32 + 1, 1));
        assert!(o.correction.is_none());
    }
}
#[test]
fn stdin_no_write_and_fix_rejection() {
    let d = project();
    for fix in [false, true] {
        fs::write(d.path().join("a.rb"), b"disk bytes\n").unwrap();
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
        stdin::write_stdin(child.stdin.take().unwrap(), b"number == 0\n", fix).unwrap();
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
