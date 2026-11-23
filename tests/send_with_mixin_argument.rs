//! Pinned RuboCop examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use rustocop::offense::Offense;
const COP: &str = "Lint/SendWithMixinArgument";
fn offenses(s: &str) -> Vec<Offense> {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    rustocop::pipeline::lint_one(
        std::path::Path::new("example.rb"),
        std::path::Path::new("example.rb"),
        Some(s.as_bytes()),
        &active,
        &config,
    )
    .result
    .offenses
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/send_with_mixin_argument.json")).unwrap()
}
#[test]
fn original_byte_reference_rows() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let mut failures = Vec::new();
    for row in rows() {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let path = std::path::Path::new(row["path"].as_str().unwrap());
        let report = rustocop::pipeline::lint_one(path, path, Some(&bytes), &active, &config);
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        if row.get("parser_rejection").is_some() {
            assert!(!report.errors.is_empty(), "{row}: {report:?}");
            continue;
        }
        assert!(report.errors.is_empty(), "{row}: {report:?}");
        let mut actual: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
            .collect();
        actual.sort();
        let expected: Vec<(u32, u32, u32, String)> =
            crate::support::fixture::value(&row["findings"]);
        if actual != expected {
            failures.push(format!(
                "{}: expected {expected:?}, got {actual:?}",
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
fn pinned_matcher_positions_messages_and_lookalikes() {
    for dispatch in ["send", "public_send", "__send__"] {
        for method in ["include", "prepend", "extend"] {
            for receiver in ["Foo.", "A::Foo.", "self.", ""] {
                for literal in [format!(":{method}"), format!("'{method}'")] {
                    let bad = format!("{dispatch}({literal}, B::Bar, Baz)");
                    let source = format!("class Foo\n  {receiver}{bad}\nend\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col, found[0].end_line),
                        (2, 3 + receiver.len() as u32, 2)
                    );
                    assert_eq!(
                        found[0].message,
                        format!("Use `{method} B::Bar, Baz` instead of `{bad}`.")
                    );
                }
            }
        }
    }
    for s in [
        "Foo.send(:do_something, Bar)",
        "Foo.include Bar",
        "Foo.prepend Bar",
        "Foo.extend Bar",
        "x.send(:include, Bar)",
        "(Foo).send(:include, Bar)",
        "Foo&.send(:include, Bar)",
        "Foo.send(:include)",
        "Foo.send(:include, bar)",
        "Foo.send(:include, Bar, &blk)",
        "Foo.send(:include, *mods)",
        "Foo.send(:include, Bar, key: Baz)",
        "Foo.send(\"#{method}\", Bar)",
    ] {
        assert!(offenses(s).is_empty(), "{s}");
    }
}
