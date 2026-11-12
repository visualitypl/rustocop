//! pinned detection and unsafe/no-write CLI contracts.
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/DoubleNegation";
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
    correction: Option<String>,
}
#[test]
fn default_enabled_selectors_and_fixed_config_contract() {
    let d = project();
    let r = rustocop::cops::Registry::new();
    let cop = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    let source = b"!!value\n";
    assert!(
        String::from_utf8_lossy(&unchanged(d.path(), source, &["a.rb"], 1).stdout).contains(COP)
    );
    unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    for target in [COP, "Style"] {
        unchanged(
            d.path(),
            source,
            &["--only", COP, "--except", target, "--fix", "a.rb"],
            0,
        );
    }
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
        "EnforcedStyle=\"forbidden\"",
        "SafeAutoCorrect=true",
        "max=1",
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
fn excludes_and_both_directive_prefixes() {
    let d = project();
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), b"!!false\n", &["--only", COP, "--fix", "a.rb"], 0);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "DoubleNegation", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n!!false\n"),
                format!("# {prefix}:disable-next {target}\n!!false\n"),
                format!("!!false # {prefix}:disable {target}\n"),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
        }
        let source =
            format!("# {prefix}:disable {COP}\n!!false\n# {prefix}:enable {COP}\n!!false\n");
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
fn envelopes_and_clean_error_files_are_never_written() {
    let d = project();
    for source in [
        b"\xef\xbb\xbf!!false\r\n".as_slice(),
        b"# coding: windows-1252\n# caf\xe9\n!!false\n",
        b"!!false\n__LINE__\n",
        b"!!false\n__END__\nraw\r\n\x00payload",
        b"!!(<<~TEXT)\n  value\nTEXT\n",
        b"!!false # keep\r\n",
    ] {
        unchanged(d.path(), source, &["--only", COP, "a.rb"], 1);
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    }
    for (source, code) in [("!false\n", 0), ("def foo; !!false; end\n", 0), ("(\n", 2)] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
}
#[test]
fn stdin_display_path_is_not_a_write_target() {
    use std::process::Stdio;
    let d = project();
    fs::write(d.path().join("a.rb"), "original bytes").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    stdin::write_stdin(child.stdin.take().unwrap(), b"!!false\n", false).unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stdout).contains("a.rb:1:1:"));
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "original bytes"
    );
    cli(d.path(), &["--only", COP, "--fix", "--stdin", "a.rb"], 2);
}
#[test]
fn truthiness_custom_negation_equality_and_evaluation_are_not_rewritten() {
    let d = project();
    let source = r#"class Probe
  def !; puts :bang; false; end
  def nil?; puts :nil_predicate; false; end
  def ==(other); puts :equal; :truthy; end
end
def effect(value); puts :effect; value; end
def run
  return :yes if !!effect(false) || !!(effect(Probe.new) == 0) && !!effect(0)
  :no
end
p run
p(!!false)
p(!false.nil?)
p(!!Probe.new)
p(!!(x = effect(nil)))
p x
p __LINE__
"#;
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        let observe = || {
            let o = Command::new(&ruby)
                .current_dir(d.path())
                .arg("a.rb")
                .output()
                .unwrap();
            assert!(o.status.success(), "{o:?}");
            o.stdout
        };
        fs::write(d.path().join("a.rb"), source).unwrap();
        let before = observe();
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert_eq!(before, observe());
        assert_eq!(before, b"effect\neffect\nequal\neffect\n:yes\nfalse\ntrue\nbang\ntrue\neffect\nfalse\nnil\n17\n");
    }
}
#[test]
fn six_thousand_method_siblings_use_constant_time_tail_lookup() {
    let r = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&r, &config, &[COP.into()], &[]);
    let source = format!("def foo\n{}finish\nend\n", "!!effect\n".repeat(6000));
    let report = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(source.as_bytes()),
        &active,
        &config,
    );
    assert!(report.errors.is_empty() && report.internal_errors.is_empty());
    let found = report.result.offenses;
    assert_eq!(found.len(), 6000);
    for (i, o) in found.iter().enumerate() {
        assert_eq!((o.line, o.col), (i as u32 + 2, 1));
        assert!(o.correction.is_none());
    }
}
#[test]
fn all_42_pinned_calls_and_18_unsafe_correction_calls() {
    // RuboCop spec/rubocop/cop/style/double_negation_spec.rb,
    // MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/double_negation_pinned.json")).unwrap();
    assert_eq!(cases.len(), 42);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 18);
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
