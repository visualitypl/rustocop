//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use rustocop::offense::Offense;
const CONSISTENCY: &str = "Lint/SafeNavigationConsistency";
const EMPTY: &str = "Lint/SafeNavigationWithEmpty";
fn offenses(source: &str, cop: &str) -> Vec<Offense> {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
    rustocop::pipeline::lint_one(
        std::path::Path::new("example.rb"),
        std::path::Path::new("example.rb"),
        Some(source.as_bytes()),
        &active,
        &config,
    )
    .result
    .offenses
}
#[test]
fn pinned_boolean_and_conditional_diagnostics() {
    for (source, cop, col, message) in [
        (
            "foo&.bar && foo&.baz",
            CONSISTENCY,
            16,
            "Use `.` instead of unnecessary `&.`.",
        ),
        (
            "foo&.bar || foo.baz",
            CONSISTENCY,
            16,
            "Use `&.` for consistency with safe navigation.",
        ),
        (
            "return unless foo&.empty?",
            EMPTY,
            15,
            "Avoid calling `empty?` with the safe navigation operator in conditionals.",
        ),
    ] {
        let found = offenses(source, cop);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!((found[0].line, found[0].col), (1, col));
        assert_eq!(found[0].message, message);
    }
}
#[test]
fn pinned_lookalikes() {
    for source in [
        "foo&.bar && foo.baz",
        "foo&.bar || other.baz",
        "foo&.bar || foo.nil?",
        "foo&.bar && (foo.baz || foo.qux)",
    ] {
        assert!(offenses(source, CONSISTENCY).is_empty(), "{source}");
    }
    for source in [
        "return if foo.empty?",
        "return if foo&.bar&.empty?",
        "bar = foo&.empty?",
        "if (foo&.empty?); end",
        "if !foo&.empty?; end",
        "while foo&.empty?; end",
    ] {
        assert!(offenses(source, EMPTY).is_empty(), "{source}");
    }
}
#[test]
fn mixed_newline_source_fixed_policy_pipeline_positions_and_detection_only() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/safe_navigation_consistency_empty.json"
    ))
    .unwrap();
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[CONSISTENCY.into()], &[]);
    let cases: Vec<_> = rows.iter().filter(|r| r["origin"] == "newline").collect();
    assert_eq!(cases.len(), 98);
    for row in cases {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let report = rustocop::pipeline::lint_one(
            std::path::Path::new("example.rb"),
            std::path::Path::new("example.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}: {report:?}"
        );
        let found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none(), "{row}");
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        let expected: Vec<(u32, u32, u32, String)> =
            serde_json::from_value(row["findings"].clone()).unwrap();
        assert_eq!(found, expected, "{}: {:?}", row["case"], bytes);
    }
}
#[test]
fn all_pinned_original_byte_fixtures() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/safe_navigation_consistency_empty.json"
    ))
    .unwrap();
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let mut failures = Vec::new();
    for row in rows {
        let cop = row["cop"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let report = rustocop::pipeline::lint_one(
            std::path::Path::new("example.rb"),
            std::path::Path::new("example.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        if row.get("parser_rejection").is_some() {
            assert!(!report.errors.is_empty());
            assert!(report.result.offenses.is_empty());
            continue;
        }
        assert!(report.errors.is_empty(), "{row}: {report:?}");
        let mut found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> =
            serde_json::from_value(row["findings"].clone()).unwrap();
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
