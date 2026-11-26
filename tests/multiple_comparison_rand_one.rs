//! Source-only pinned AST and CLI checks. Never execute fixture input.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = ["Lint/MultipleComparison", "Lint/RandOne"];
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/multiple_comparison_rand_one.json")).unwrap();
    assert_eq!(rows.len(), 1114);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        19
    );
    rows
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
fn pinned_main_source_original_bytes_and_send_shapes_match_without_edits() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let mut failures = Vec::new();
    for row in rows() {
        let cop = row["cop"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        if row.get("parser_rejection").is_some() {
            assert!(
                !report.errors.is_empty(),
                "reference rejection unexpectedly accepted: {row}: {report:?}"
            );
            assert!(report.result.offenses.is_empty());
            continue;
        }
        assert!(report.errors.is_empty(), "{row}: {report:?}");
        let mut found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Warning);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        found.sort();
        let expected: Vec<Finding> = crate::support::fixture::value(&row["findings"]);
        if found != expected {
            failures.push(format!(
                "{cop} {}: expected {expected:?}, found {found:?}",
                String::from_utf8_lossy(&bytes)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
#[test]
fn binary_comment_bytes_have_a_lossless_utf8_diagnostic_not_replacement_characters() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COPS[1].into()], &[]);
    let bytes = b"# coding: ASCII-8BIT\nrand(\n # \xE9\n 1\n)\n";
    let report = rustocop::pipeline::lint_one(
        Path::new("example.rb"),
        Path::new("example.rb"),
        Some(bytes),
        &active,
        &config,
    );
    assert!(
        report.errors.is_empty() && report.internal_errors.is_empty(),
        "{report:?}"
    );
    assert_eq!(report.result.offenses.len(), 1);
    let o = &report.result.offenses[0];
    assert_eq!(
        o.message,
        "`rand(\n # \\xE9\n 1\n)` always returns `0`. Perhaps you meant `rand(2)` or `rand`?"
    );
    assert!(o.correction.is_none());
    let dir = project_with_lib();
    let out = unchanged(
        dir.path(),
        bytes,
        &["--only", COPS[1], "--fix", "lib/a.rb"],
        Some(1),
    );
    assert!(
        std::str::from_utf8(&out.stdout)
            .unwrap()
            .contains(&o.message)
    );
}

#[test]
fn native_authored_counterexample_explains_why_upstream_rewrite_is_not_a_fix() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    // Separately authored bounded control, NOT a reference/fixture/corpus input.
    // There is no rand invocation, application access or eval here.
    let source = b"class Counter\n  attr_reader :calls\n  def initialize; @calls = 0; end\n  def pivot; @calls += 1; 2; end\nend\nfirst = Counter.new\nbegin\n  result = 0 < first.pivot < 4\nrescue NoMethodError\n  result = :comparison_on_boolean\nend\np [result, first.calls]\nsecond = Counter.new\np [0 < second.pivot && second.pivot < 4, second.calls]\n";
    let mut child = Command::new(ruby)
        .arg("-W0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"[:comparison_on_boolean, 1]\n[true, 2]\n");
    assert!(out.stderr.is_empty(), "{out:?}");
}

#[test]
fn native_ruby34_syntax_only_never_execute_comparisons_or_random_calls() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
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
            row.get("parser_rejection").is_none(),
            "{row}: {out:?}"
        );
    }
}
