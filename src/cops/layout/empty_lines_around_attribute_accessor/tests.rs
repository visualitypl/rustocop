//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/empty_lines_around_attribute_accessor_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 21
//! pinned examples run at the fixed defaults (`AllowAliasSyntax: true`,
//! `AllowedMethods: [alias_method, public, protected, private]`): the
//! `AllowAliasSyntax: false` and `AllowedMethods: []` contexts replay as no
//! offense, like the inventory's fixed results. Findings, messages and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/EmptyLinesAroundAttributeAccessor";

fn positions(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        ruby_prism::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&EmptyLinesAroundAttributeAccessor], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            (o.line, o.col)
        })
        .collect();
    found.sort_unstable();
    found
}

fn check(source: &str, finding: &[(u32, u32)], after: &str) {
    assert_eq!(positions(source), finding, "{source}");
    if !finding.is_empty() {
        assert_correction(&EmptyLinesAroundAttributeAccessor, source, after);
    }
}

fn held(source: &str) {
    let found = correction_offenses(&EmptyLinesAroundAttributeAccessor, source);
    assert_eq!(found.len(), 1, "{source:?}");
    assert!(found[0].correction.is_none(), "{source:?}");
}

/// `(spec line, source, fixed-default finding, single-pass correction)`.
type Pinned = (u32, &'static str, Option<(u32, u32)>, &'static str);

const PINNED: &[Pinned] = &[
    (
        5,
        "attr_accessor :foo\ndef do_something\nend\n",
        Some((1, 1)),
        "attr_accessor :foo\n\ndef do_something\nend\n",
    ),
    (
        21,
        "attr_accessor :foo # comment\ndef do_something\nend\n",
        Some((1, 1)),
        "attr_accessor :foo # comment\n\ndef do_something\nend\n",
    ),
    (
        37,
        "attr_accessor :foo\n# comment\ndef do_something\nend\n",
        Some((1, 1)),
        "attr_accessor :foo\n\n# comment\ndef do_something\nend\n",
    ),
    (
        55,
        "attr_accessor :foo\nattr_reader :bar\nattr_writer :baz\n# comment\ndef do_something\nend\n",
        Some((3, 1)),
        "attr_accessor :foo\nattr_reader :bar\nattr_writer :baz\n\n# comment\ndef do_something\nend\n",
    ),
    (
        77,
        "attr_accessor :foo\n# comment\n# comment\ndef do_something\nend\n",
        Some((1, 1)),
        "attr_accessor :foo\n\n# comment\n# comment\ndef do_something\nend\n",
    ),
    (
        98,
        "attr_accessor :foo\n# rubocop:disable Department/Cop\ndef do_something\nend\n",
        Some((1, 1)),
        "attr_accessor :foo\n\n# rubocop:disable Department/Cop\ndef do_something\nend\n",
    ),
    (
        117,
        "# rubocop:disable Department/Cop\nattr_accessor :foo\n# rubocop:enable Department/Cop\ndef do_something\nend\n",
        Some((2, 1)),
        "# rubocop:disable Department/Cop\nattr_accessor :foo\n# rubocop:enable Department/Cop\n\ndef do_something\nend\n",
    ),
    (
        138,
        "# rubocop:disable Department/Cop\nattr_accessor :foo\n# rubocop:enable Department/Cop\n# comment\ndef do_something\nend\n",
        Some((2, 1)),
        "# rubocop:disable Department/Cop\nattr_accessor :foo\n# rubocop:enable Department/Cop\n\n# comment\ndef do_something\nend\n",
    ),
    (
        160,
        "attr_accessor :foo\n\ndef do_something\nend\n",
        None,
        "",
    ),
    (
        170,
        "# rubocop:disable Department/Cop\nattr_accessor :foo\n# rubocop:enable Department/Cop\n\ndef do_something\nend\n",
        None,
        "",
    ),
    (
        181,
        "attr_accessor :foo; def do_something\n  foo\nend\n",
        None,
        "",
    ),
    (189, "attr_accessor :foo", None, ""),
    (
        193,
        "attr_accessor :foo\nattr_reader :bar\nattr_writer :baz\n\ndef do_something\nend\n",
        None,
        "",
    ),
    (
        204,
        "attr_accessor :foo\nattr_reader :bar\nattr_writer :baz\n\n# comment\ndef do_something\nend\n",
        None,
        "",
    ),
    (216, "class Foo\n  attr_accessor :foo\nend\n", None, ""),
    (224, "class Foo\n  attr.foo\nend\n", None, ""),
    (
        232,
        "if condition\n  attr_reader :foo\nelse\n  do_something\nend\n",
        None,
        "",
    ),
    (
        245,
        "attr_accessor :foo\nalias foo? foo\n\ndef do_something\nend\n",
        None,
        "",
    ),
    // `AllowAliasSyntax: false` context: the fixed default allows `alias`.
    (
        259,
        "attr_accessor :foo\nalias foo? foo\n\ndef do_something\nend\n",
        None,
        "",
    ),
    (
        283,
        "attr_accessor :foo\nprivate :foo\n\ndef do_something\nend\n",
        None,
        "",
    ),
    // `AllowedMethods: []` context: the fixed default allows `private`.
    (
        297,
        "attr_accessor :foo\nprivate :foo\n\ndef do_something\nend\n",
        None,
        "",
    ),
];

#[test]
fn pinned_examples() {
    for &(line, source, finding, after) in PINNED {
        assert_eq!(
            positions(source),
            finding.into_iter().collect::<Vec<_>>(),
            "spec line {line}"
        );
        if finding.is_some() {
            assert_correction(&EmptyLinesAroundAttributeAccessor, source, after);
        }
    }
    assert_eq!(PINNED.len(), 21);
}

#[test]
fn followers_and_lookalikes() {
    for (source, finding, after) in [
        // Every accessor name; `alias` of globals and the allowed methods.
        ("attr :a\nattr_writer :b\nattr_reader :c\n", &[][..], ""),
        ("attr_reader :a\nalias $x $y\n", &[], ""),
        ("attr_reader :a\nalias_method :b, :a\n", &[], ""),
        ("attr_reader :a\npublic\n", &[], ""),
        ("attr_reader :a\nprotected :a\n", &[], ""),
        ("attr_reader :a\nself.private(:a)\n", &[], ""),
        ("attr_reader :a\nprivate def b; end\n", &[], ""),
        // Not an accessor send: no arguments, a receiver, `csend`, a block.
        (
            "attr_reader :a\nattr_reader\n",
            &[(1, 1)],
            "attr_reader :a\n\nattr_reader\n",
        ),
        (
            "attr_reader :a\nself.attr_reader :b\n",
            &[(1, 1)],
            "attr_reader :a\n\nself.attr_reader :b\n",
        ),
        (
            "attr_reader :a\nfoo&.private\n",
            &[(1, 1)],
            "attr_reader :a\n\nfoo&.private\n",
        ),
        (
            "attr_reader :a\nprivate { x }\n",
            &[(1, 1)],
            "attr_reader :a\n\nprivate { x }\n",
        ),
        // A modifier form is an `if` node, not a send.
        (
            "attr_reader :a\nattr_reader :b if c\n",
            &[(1, 1)],
            "attr_reader :a\n\nattr_reader :b if c\n",
        ),
        ("self.attr_reader :a\nx\n", &[], ""),
        ("attr_reader\nx\n", &[], ""),
        ("x = attr_reader :a\ny\n", &[], ""),
        // A sibling starting on the accessor's last line.
        ("attr_reader :a,\n  :b; x\ny\n", &[], ""),
        // A multi-line accessor is checked after its last line.
        (
            "attr_reader :a,\n  :b\nx\n",
            &[(1, 1)],
            "attr_reader :a,\n  :b\n\nx\n",
        ),
        // Several statements in a branch are a parser `begin`: checked.
        (
            "if c\n  attr_reader :a\n  x\nend\n",
            &[(2, 3)],
            "if c\n  attr_reader :a\n\n  x\nend\n",
        ),
        ("unless c\n  attr_reader :a\nend\n", &[], ""),
        // A sole protected statement is followed by its `rescue` clause or
        // its `ensure` body (upstream's parser siblings).
        (
            "begin\n  attr_reader :a\nrescue\n  x\nend\n",
            &[(2, 3)],
            "begin\n  attr_reader :a\n\nrescue\n  x\nend\n",
        ),
        (
            "def f\n  attr_reader :a\nensure\n  x\nend\n",
            &[(2, 3)],
            "def f\n  attr_reader :a\n\nensure\n  x\nend\n",
        ),
        ("def f\n  attr_reader :a\nensure\n  private\nend\n", &[], ""),
        ("def f\n  attr_reader :a\nensure\nend\n", &[], ""),
        (
            "begin\n  attr_reader :a\n  b\nrescue\nend\n",
            &[(2, 3)],
            "begin\n  attr_reader :a\n\n  b\nrescue\nend\n",
        ),
        // Both spellings of an enable directive; a CRLF line ending.
        ("attr_reader :a\n# rustocop:enable Foo/Bar\n\nx\n", &[], ""),
        (
            "attr_reader :a\n# rustocop:enable Foo/Bar\nx\n",
            &[(1, 1)],
            "attr_reader :a\n# rustocop:enable Foo/Bar\n\nx\n",
        ),
        (
            "attr_reader :a\r\nx\r\n",
            &[(1, 1)],
            "attr_reader :a\r\n\r\nx\r\n",
        ),
        ("attr_reader :a\n__END__\nx\n", &[], ""),
    ] {
        check(source, finding, after);
    }
}

#[test]
fn held_insertions() {
    for source in [
        "attr_reader :a\nx\np __FILE__\n",
        "attr_reader :a\nx\np $PROGRAM_NAME\n",
        "attr_reader :a\nx\np caller_locations\n",
        "attr_reader :a\nx\np method(:x).source_location\n",
        "attr_reader :a\nx\np SCRIPT_LINES__\n",
        "attr_reader :a\nx\np __LINE__\n",
        "attr_reader :a, <<~X.to_sym\n  body\nX\nx\n",
    ] {
        held(source);
    }
}

#[test]
fn metadata_is_default_on_and_safe() {
    let cop = EmptyLinesAroundAttributeAccessor;
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert!(cop.department_selection_enables());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
}
