//! fixed zero-length policy and unsafe detection-only CLI contracts.
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/ZeroLengthPredicate";
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
fn all_74_pinned_calls_and_37_unsafe_correction_calls() {
    // RuboCop spec/rubocop/cop/style/zero_length_predicate_spec.rb,
    // MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/zero_length_predicate_pinned.json")).unwrap();
    assert_eq!(cases.len(), 74);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 37);
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
#[test]
fn default_enabled_selectors_config_and_strict_fixed_options() {
    let d = project();
    let r = rustocop::cops::Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled());
    assert!(!c.safe_fixes());
    let source = b"collection.size.zero?\n";
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
    unchanged(
        d.path(),
        source,
        &["--only", COP, "--except", "Style", "--fix", "a.rb"],
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
        "AllowedMethods=[]",
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
fn user_excludes_and_both_directive_prefixes() {
    let d = project();
    let source = b"collection.size.zero?\n";
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 0);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    // No built-in spec exclusion (unlike NumericPredicate).
    fs::create_dir(d.path().join("spec")).unwrap();
    fs::write(d.path().join("spec/a.rb"), source).unwrap();
    cli(d.path(), &["--only", COP, "spec/a.rb"], 1);
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "ZeroLengthPredicate", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\ncollection.size.zero?\n"),
                format!("# {prefix}:disable-next {target}\ncollection.size.zero?\n"),
                format!("collection.size.zero? # {prefix}:disable {target}\n"),
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
            "# {prefix}:disable {COP}\ncollection.size.zero?\n# {prefix}:enable {COP}\ncollection.size.zero?\n"
        );
        let o = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert!(String::from_utf8_lossy(&o.stdout).contains("a.rb:4:12:"));
    }
}
#[test]
fn no_writes_for_source_envelopes_clean_and_error_files() {
    let d = project();
    for source in [
        "\u{feff}collection.size == 0\r\n",
        "# coding: us-ascii\ncollection.size == 0\n",
        "# coding: windows-1252\n# café\ncollection.size == 0\n",
        "collection.size < __LINE__\n",
        "collection.size == 0\n__LINE__\n",
        "collection.size == 0\n__END__\npayload\r\n",
        "collection.size == - # keep unary dispatch\n0\n",
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
        b"# coding: windows-1252\n# caf\xe9\ncollection.size == 0\n",
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
    for (source, code) in [
        ("collection.empty?\n", 0),
        ("collection.size == 0.0\n", 0),
        ("(\n", 2),
    ] {
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
fn custom_collection_numeric_dispatch_evaluation_and_control_flow_are_not_rewritten() {
    let d = project();
    let source = "class NumberLike\n def ==(n); puts :equal; true; end\n def >(n); puts :greater; true; end\n def <(n); puts :less; true; end\n def zero?; puts :zero; true; end\nend\nclass Collection\n def size; puts :size; NumberLike.new; end\n def empty?; puts :empty; false; end\nend\ndef value; puts :receiver; Collection.new; end\ndef run\n return :yes if value.size == 0 && value.size > 0 && value.size < 1 && value.size.zero?\n :no\nend\np run\np Collection.new.empty?\np(nil&.size == 0)\np(!nil&.empty?)\np __LINE__\n";
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
        assert_eq!(before, b"receiver\nsize\nequal\nreceiver\nsize\ngreater\nreceiver\nsize\nless\nreceiver\nsize\nzero\n:yes\nempty\nfalse\nfalse\ntrue\n20\n");
    }
}
#[test]
fn literal_receivers_and_overridden_integer_unary_dispatch_are_not_rewritten() {
    let d = project();
    let source = "class Integer\n def ==(other); false; end\n def -@; 1; end\nend\np([].size == 0)\np([].empty?)\np([].size == - 0)\np __LINE__\n";
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
    let source = "x.size.zero?\n".repeat(6000);
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
        assert_eq!((o.line, o.col), (i as u32 + 1, 3));
        assert!(o.correction.is_none());
    }
}
#[test]
fn numeric_predicate_peer_does_not_rewrite_unsafe_comparisons() {
    let d = project();
    let source = b"collection.size == 0\n";
    let o = unchanged(
        d.path(),
        source,
        &[
            "--only",
            "Style/ZeroLengthPredicate,Style/NumericPredicate",
            "--fix",
            "a.rb",
        ],
        1,
    );
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains(COP) && text.contains("Style/NumericPredicate"));
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
        stdin::write_stdin(child.stdin.take().unwrap(), b"collection.size.zero?\n", fix).unwrap();
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
