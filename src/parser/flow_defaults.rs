//! Native Ruby 3.4 source-only flow/default scope regressions.
#[path = "../../tests/support/flow_defaults.rs"]
pub(super) mod witnesses;

use super::parse;

#[test]
fn flow_defaults_keep_main_parse_diagnostics_and_original_byte_ranges() {
    for case in witnesses::cases() {
        let parsed = parse(&case.source);
        let errors: Vec<_> = parsed.errors().collect();
        if case.valid {
            assert!(errors.is_empty(), "{case:?}: {errors:?}");
        } else {
            assert_eq!(errors.len(), 1, "{case:?}: {errors:?}");
            let error = &errors[0];
            assert_eq!(
                error.message(),
                format!("Invalid {}", case.keyword),
                "{case:?}"
            );
            assert_eq!(
                error.location().start_offset(),
                case.range.start,
                "{case:?}"
            );
            assert_eq!(error.location().end_offset(), case.range.end, "{case:?}");
            assert_eq!(
                error.location().as_slice(),
                &case.source[case.range.clone()]
            );
        }
        assert_eq!(parsed.depth_error(), None, "{case:?}");
        let recorded = crate::source::SourceFile::new("raw.rb".into(), case.source.clone());
        recorded.record_diagnostics(&parsed);
        let fallback = crate::source::SourceFile::new("raw.rb".into(), case.source.clone());
        assert_eq!(recorded.diagnostics(), fallback.diagnostics(), "{case:?}");
        assert_eq!(recorded.bytes, case.source);
        if !case.valid {
            assert_eq!(recorded.diagnostics().errors[0].range, case.range);
            assert_eq!(
                recorded.diagnostics().errors[0].message,
                format!("Invalid {}", case.keyword)
            );
        }
    }
}

#[test]
fn flow_defaults_compatibility_leaves_latest_api_unchanged() {
    let source = b"def f; foo { |x = (next; y)| x }; end";
    assert_eq!(ruby_prism::parse(source).errors().count(), 0);
    assert_eq!(parse(source).errors().count(), 1);
}

#[test]
fn flow_defaults_preserve_multiple_errors_and_sibling_scope_cleanup() {
    let source = b"foo { |x = (next; break 42; redo; y)| next }; foo { |x = bar { next }| x }\n";
    let parsed = parse(source);
    let errors: Vec<_> = parsed.errors().collect();
    assert_eq!(errors.len(), 3);
    for (error, (message, range)) in errors.iter().zip([
        ("Invalid next", 12..16),
        ("Invalid break", 18..26),
        ("Invalid redo", 28..32),
    ]) {
        assert_eq!(error.message(), message);
        assert_eq!(error.location().start_offset(), range.start);
        assert_eq!(error.location().end_offset(), range.end);
    }
    // Metadata fallbacks parse the same original source. A failed parse must
    // not leave a block-exit list/context behind for this or the next parse.
    for _ in 0..8 {
        let parsed = parse(b"foo { |x = (next; y)| x }");
        assert_eq!(parsed.errors().count(), 1);
        assert_eq!(parse(b"foo { |x = bar { next }| x }").errors().count(), 0);
    }
}

#[test]
fn flow_defaults_do_not_replace_grammar_or_depth_and_recovered_tree_guards() {
    for source in [
        b"foo { |x = [not false]| x }".as_slice(),
        b"items.map { it }",
        b"def f(...); target(...); end",
    ] {
        assert_eq!(parse(source).errors().count(), 0);
    }
    assert!(parse(b"foo\n && bar\n").errors().next().is_some());
    let source = format!("foo {{ |x = (next; root{})| x }}", ".foo".repeat(20_000));
    let parsed = parse(source.as_bytes());
    assert!(parsed.depth_error().is_some());
    assert!(parsed.errors().any(|e| e.message() == "Invalid next"));
    drop(parsed); // Recovered invalid trees still need bounded iterative teardown.
    let source = format!(
        "foo {{ |x = (next; {}1{})| x }}",
        "[".repeat(12_000),
        "]".repeat(12_000)
    );
    let parsed = parse(source.as_bytes());
    assert!(parsed.depth_error().is_some());
    assert!(parsed.errors().any(|e| e.message() == "nesting too deep"));
}
