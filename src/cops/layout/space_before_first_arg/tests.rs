//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_before_first_arg_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 12 pinned examples
//! (11 distinct sources) run at the fixed `AllowForAlignment: true`; the
//! `AllowForAlignment: false` context is replayed at that default. Findings
//! and single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (source and corrected SHA-256 of
//! every row were checked against it when the rows were ported).
use super::*;
use crate::cops::testing::{assert_correction, correction_offenses};

const COP: &str = "Layout/SpaceBeforeFirstArg";
const MESSAGE: &str = "Put one space between the method name and the first argument.";

/// `(line, col)` of each finding from the full default pipeline.
fn findings(source: &str) -> Vec<(u32, u32)> {
    crate::pipeline::lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert_eq!(o.message, MESSAGE);
            assert_eq!(o.end_line, o.line);
            (o.line, o.col)
        })
        .collect()
}

/// `(spec lines, source, findings, corrected source)`.
type Pinned = (
    &'static [u32],
    &'static str,
    &'static [(u32, u32)],
    &'static str,
);

const PINNED: &[Pinned] = &[
    (
        &[9],
        "something  x\na.something  y, z\n",
        &[(1, 10), (2, 12)],
        "something x\na.something y, z\n",
    ),
    (
        &[36],
        "something'hello'\na.something'hello world'\n",
        &[(1, 10), (2, 12)],
        "something 'hello'\na.something 'hello world'\n",
    ),
    (&[65], "something x\na.something y, z\n", &[], ""),
    (&[72], "something +\n  x\n", &[], ""),
    (&[79], "something.x =\n  y\n", &[], ""),
    (&[86], "something \\\n  x\n", &[], ""),
    (
        &[24],
        "a&.something  y, z\n",
        &[(1, 13)],
        "a&.something y, z\n",
    ),
    (
        &[51],
        "obj = a_method(arg, arg2)\nobj.no_parenthesized'asdf'\n",
        &[(2, 21)],
        "obj = a_method(arg, arg2)\nobj.no_parenthesized 'asdf'\n",
    ),
    (
        &[94, 108],
        "form.inline_input   :full_name,     as: :string\nform.disabled_input :password,      as: :passwd\nform.masked_input   :zip_code,      as: :string\nform.masked_input   :email_address, as: :email\nform.masked_input   :phone_number,  as: :tel\n",
        &[],
        "",
    ),
    (&[133], "something(x)\na.something(y, z)\n", &[], ""),
    (&[140], "something(  x  )", &[], ""),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    let mut examples = 0;
    for &(spec, source, expected, after) in PINNED {
        examples += spec.len();
        assert_eq!(findings(source), expected, "spec lines {spec:?}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&SpaceBeforeFirstArg, source).is_empty(),
                "{spec:?}"
            );
        } else {
            assert_correction(&SpaceBeforeFirstArg, source, after);
        }
    }
    assert_eq!(examples, 12);
}

#[test]
fn only_regular_unparenthesized_calls_on_their_first_line() {
    for source in [
        "foo(x)\n",
        "a.b(1, 2)\n",
        "foo.(  1)\n",
        "foo\n",
        "x.y  =  1\n",
        "x&.y  =  1\n",
        "a  + b\n",
        "a.+  b\n",
        "a  [1]  =  2\n",
        "!  x\n",
        "not  x\n",
        "-> x { x }\n",
        "def m(*) = super  1\n",
        "def m = yield  1\n",
        "defined?  x\n",
        "foo \\\n  x\n",
        "a\n  .something  y\n",
        "foo x\n",
        "\"#{foo x}\"\n",
        // Aligned first arguments are allowed.
        "form.inline_input   :a,  b\nform.disabled_input :c,  d\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
    for (source, expected) in [
        ("x  [1]\n", vec![(1, 2)]),
        ("foo  a: 1\n", vec![(1, 4)]),
        ("foo  &blk\n", vec![(1, 4)]),
        ("foo\t\tx\n", vec![(1, 4)]),
        ("a&.b  c\n", vec![(1, 5)]),
        ("\"#{foo  x}\"\n", vec![(1, 7)]),
        ("foo  <<~EOS\n  x\nEOS\n", vec![(1, 4)]),
        ("foo  x do\nend\n", vec![(1, 4)]),
        ("\u{feff}é.foo  x\r\n", vec![(1, 6)]),
        ("puts:a\n", vec![(1, 5)]),
        ("foo@x\n", vec![(1, 4)]),
        ("foo->{}\n", vec![(1, 4)]),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
    }
}

#[test]
fn corrections_keep_the_lexing_and_ast() {
    for (before, after) in [
        ("foo  -1\n", "foo -1\n"),
        ("foo  *a\n", "foo *a\n"),
        ("foo  **h\n", "foo **h\n"),
        ("foo  [1]\n", "foo [1]\n"),
        ("foo  (1), 2\n", "foo (1), 2\n"),
        ("foo  ::Bar\n", "foo ::Bar\n"),
        ("foo  ?a\n", "foo ?a\n"),
        ("foo  /a/\n", "foo /a/\n"),
        ("foo  %w[a]\n", "foo %w[a]\n"),
        ("foo  <<~EOS\n  x\nEOS\n", "foo <<~EOS\n  x\nEOS\n"),
        ("foo  a: 1\n", "foo a: 1\n"),
        ("foo\t\tx\n", "foo x\n"),
        ("foo'x'\n", "foo 'x'\n"),
        ("a.foo?\"#{1}\"\n", "a.foo? \"#{1}\"\n"),
        ("x  [1]\r\n", "x [1]\r\n"),
    ] {
        assert_correction(&SpaceBeforeFirstArg, before, after);
        let (a, b) = (
            crate::parser::parse(before.as_bytes()),
            crate::parser::parse(after.as_bytes()),
        );
        assert_eq!(
            crate::cops::ast::fingerprint_ignoring_grouping(&a.node(), before.as_bytes()),
            crate::cops::ast::fingerprint_ignoring_grouping(&b.node(), after.as_bytes()),
            "{before:?}"
        );
        assert_eq!(a.warnings().count(), b.warnings().count(), "{before:?}");
    }
    // A missing blank is only inserted before a quoted string.
    for source in ["puts:a\n", "foo@x\n", "foo->{}\n"] {
        let found = correction_offenses(&SpaceBeforeFirstArg, source);
        assert_eq!(found.len(), 1, "{source:?}");
        assert!(found[0].correction.is_none(), "{source:?}");
    }
}

#[test]
fn listed_observers_and_the_incompatible_companion_hold_the_edit() {
    for observer in [
        "__FILE__",
        "__dir__",
        "$0",
        "$PROGRAM_NAME",
        "caller",
        "caller_locations",
        "method(:x).source_location",
        "SCRIPT_LINES__",
        "DATA.read\n__END__\nx",
    ] {
        let source = format!("foo  1\np {observer}\n");
        let found = correction_offenses(&SpaceBeforeFirstArg, &source);
        assert_eq!(found.len(), 1, "{source:?}");
        assert!(found[0].correction.is_none(), "{source:?}");
    }
    assert_correction(&SpaceBeforeFirstArg, "foo  1\np DATA\n", "foo 1\np DATA\n");
    let registry = crate::cops::Registry::new();
    let other = registry
        .cops()
        .iter()
        .find(|c| c.name() == "Style/MethodCallWithArgsParentheses")
        .unwrap();
    let found = crate::pipeline::lint_str_with_corrections(
        &[&SpaceBeforeFirstArg, other.as_ref()],
        "foo  1\n",
    );
    let ours: Vec<_> = found.iter().filter(|o| o.cop == COP).collect();
    assert_eq!(ours.len(), 1);
    assert!(ours[0].correction.is_none());
    assert_eq!(
        SpaceBeforeFirstArg.autocorrect_incompatible_with(),
        ["Style/MethodCallWithArgsParentheses"]
    );
}
