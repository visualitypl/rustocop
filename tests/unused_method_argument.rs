//! RuboCop MIT examples are SOURCE ONLY; authored native controls separate.
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const COP: &str = "Lint/UnusedMethodArgument";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/unused_method_argument.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    crate::support::fixture::value(&row[key])
}
#[test]
fn pinned_main_source_and_all_original_bytes_match_exact_pipeline_findings() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let rows = rows();
    assert_eq!(rows.len(), 4076);
    let mut failures = Vec::new();
    for row in rows {
        let source = bytes(&row, "bytes");
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        if row.get("parser_rejection").is_some() {
            assert!(
                !report.errors.is_empty() && report.result.offenses.is_empty(),
                "{row}: {report:?}"
            );
            continue;
        }
        assert!(report.errors.is_empty(), "{row}: {report:?}");
        let mut found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        found.sort();
        let expected: Vec<Finding> = crate::support::fixture::value(&row["findings"]);
        if found != expected {
            failures.push(format!(
                "{}: expected {expected:?}, found {found:?}",
                String::from_utf8_lossy(&source)
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
fn native_ruby34_syntax_only_for_every_reference_input_and_canonical_output() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    assert!(
        String::from_utf8_lossy(
            &Command::new(&ruby)
                .arg("--version")
                .output()
                .unwrap()
                .stdout
        )
        .contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        for key in ["bytes", "project_after", "upstream_after"] {
            if row.get(key).is_none() {
                continue;
            }
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-W0", "-c"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&bytes(&row, key))
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                row.get("parser_rejection").is_none(),
                "{row}: {out:?}"
            );
        }
    }
}
