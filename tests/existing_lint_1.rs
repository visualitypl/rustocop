//! Source-only pipeline/CLI controls. Inputs are NEVER executed.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use rustocop::{config::ResolvedConfig, cops::Cop};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/existing_lint_1.json")).unwrap()
}
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({cop})"))
        .count()
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    }
    let mut held = fs::File::open(&path).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(
            out.status.code(),
            Some(status),
            "{}: {out:?}",
            String::from_utf8_lossy(bytes)
        );
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let mut held_bytes = Vec::new();
    held.read_to_end(&mut held_bytes).unwrap();
    assert_eq!(held_bytes, bytes);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
    out
}
#[test]
fn all_original_byte_reference_rows_match_pipeline_exact_ranges_messages_no_edits() {
    let config = ResolvedConfig::default();
    let registry = rustocop::cops::Registry::new();
    let mut mismatches = Vec::new();
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let cop: &dyn Cop = registry
            .cops()
            .iter()
            .find(|c| c.name() == row["cop"].as_str().unwrap())
            .unwrap()
            .as_ref();
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&bytes),
            &[cop],
            &config,
        );
        if row.get("parser_rejection").is_some() || row.get("native_invalid").is_some() {
            assert!(!report.errors.is_empty(), "{row}");
            assert!(report.result.offenses.is_empty());
            continue;
        }
        if !report.errors.is_empty() || !report.internal_errors.is_empty() {
            mismatches.push(format!("{}\n{report:?}", String::from_utf8_lossy(&bytes)));
            continue;
        }
        let mut actual: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        actual.sort();
        let expected: Vec<(u32, u32, u32, String)> = serde_json::from_value(
            row.get("project_findings")
                .unwrap_or(&row["findings"])
                .clone(),
        )
        .unwrap();
        if actual != expected {
            mismatches.push(format!(
                "{}\nactual: {actual:?}\nexpected: {expected:?}",
                String::from_utf8_lossy(&bytes)
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
#[test]
fn later_line_directives_overlap_exact_reports_not_enclosing_calls() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for (cop, source, expected) in [
            (
                "Lint/AmbiguousBlockAssociation",
                "foo bar {\nx\n} # PREFIX:disable COP\n",
                0,
            ),
            (
                "Lint/AmbiguousOperator",
                "foo *[\nx\n] # PREFIX:disable COP\n",
                1,
            ),
            (
                "Lint/AmbiguousRange",
                "x || foo(\nx\n)..2 # PREFIX:disable COP\n",
                0,
            ),
            (
                "Lint/AmbiguousRegexpLiteral",
                "p /x\ny/ # PREFIX:disable COP\n",
                1,
            ),
            (
                "Lint/AssignmentInCondition",
                "if x = foo(\nx\n); end # PREFIX:disable COP\n",
                1,
            ),
            (
                "Lint/BinaryOperatorWithIdenticalOperands",
                "x ==\nx # PREFIX:disable COP\n",
                0,
            ),
            ("Lint/Debugger", "byebug(\nx\n) # PREFIX:disable COP\n", 0),
        ] {
            let text = source.replace("PREFIX", prefix).replace("COP", cop);
            assert_eq!(
                count(
                    &unchanged(
                        d.path(),
                        text.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        Some(expected as i32)
                    ),
                    cop
                ),
                expected
            );
        }
    }
}

#[test]
fn native_ruby34_syntax_only_all_reference_and_adversarial_original_bytes() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-W0", "-c"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            out.status.success(),
            row.get("parser_rejection").is_none() && row.get("native_invalid").is_none(),
            "{row}: {out:?}"
        );
    }
}
