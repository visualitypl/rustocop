//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/empty_lines_after_module_inclusion_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 61
//! pinned examples (18 per inclusion method plus 7) run at the fixed
//! defaults; findings, messages and single-pass corrected bytes equal the
//! fixed replay recorded in the pinned RuboCop reference run. The
//! `:ruby27` numbered-block example applies (Ruby 3.4 accepts `_1`).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/EmptyLinesAfterModuleInclusion";

fn positions(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        ruby_prism::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&EmptyLinesAfterModuleInclusion], source)
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
        assert_correction(&EmptyLinesAfterModuleInclusion, source, after);
    }
}

fn held(source: &str) {
    let found = correction_offenses(&EmptyLinesAfterModuleInclusion, source);
    assert_eq!(found.len(), 1, "{source:?}");
    assert!(found[0].correction.is_none(), "{source:?}");
}

/// `(spec line, source, fixed-default finding, single-pass correction)` with
/// `%m` standing for the inclusion method.
type Pinned = (u32, &'static str, Option<(u32, u32)>, &'static str);

const PER_METHOD: &[Pinned] = &[
    (
        6,
        "%m Foo\ndef do_something\nend\n",
        Some((1, 1)),
        "%m Foo\n\ndef do_something\nend\n",
    ),
    (
        22,
        "%m Foo # my comment\ndef do_something\nend\n",
        Some((1, 1)),
        "%m Foo # my comment\n\ndef do_something\nend\n",
    ),
    (
        38,
        "%m Foo\n# my comment\ndef do_something\nend\n",
        Some((1, 1)),
        "%m Foo\n\n# my comment\ndef do_something\nend\n",
    ),
    (
        56,
        "# rubocop:disable Department/Cop\n%m Foo\n# rubocop:enable Department/Cop\ndef do_something\nend\n",
        Some((2, 1)),
        "# rubocop:disable Department/Cop\n%m Foo\n# rubocop:enable Department/Cop\n\ndef do_something\nend\n",
    ),
    (
        76,
        "# rubocop:disable Department/Cop\n%m Foo\n# rubocop:enable Department/Cop\n# comment\ndef do_something\nend\n",
        Some((2, 1)),
        "# rubocop:disable Department/Cop\n%m Foo\n# rubocop:enable Department/Cop\n\n# comment\ndef do_something\nend\n",
    ),
    (
        98,
        "class Foo\n  %m Bar, Baz\n  def do_something\n  end\nend\n",
        Some((2, 3)),
        "class Foo\n  %m Bar, Baz\n\n  def do_something\n  end\nend\n",
    ),
    (
        118,
        "class Bar\n  %m Foo\n  def do_something\n  end\nend\n",
        Some((2, 3)),
        "class Bar\n  %m Foo\n\n  def do_something\n  end\nend\n",
    ),
    (
        138,
        "Class.new do\n  %m Foo\n  def do_something\n  end\nend\n",
        Some((2, 3)),
        "Class.new do\n  %m Foo\n\n  def do_something\n  end\nend\n",
    ),
    (
        159,
        "# rubocop:disable Department/Cop\n%m Foo\n# rubocop:enable Department/Cop\n\ndef do_something\nend\n",
        None,
        "",
    ),
    (170, "%m Foo\n\ndef do_something\nend\n", None, ""),
    (179, "%m Foo\n%m Foo\n\ndef do_something\nend\n", None, ""),
    (
        189,
        "module Baz\n  class Bar\n    %m Foo\n  end\nend\n",
        None,
        "",
    ),
    (199, "Class.new do\n  %m Foo\nend\n", None, ""),
    (
        207,
        "%m Module.new do |arg|\n  do_something(arg)\nend\n",
        None,
        "",
    ),
    (215, "%m Module.new do\n  do_something(_1)\nend\n", None, ""),
    (223, "%m Module.new do\n  do_something(it)\nend\n", None, ""),
    (
        231,
        "if condition\n  %m Foo\nelse\n  do_something\nend\n",
        None,
        "",
    ),
    (241, "%m Foo", None, ""),
];

const SHARED: &[Pinned] = &[
    (
        246,
        "module Foo\n  module Bar\n    extend Baz\n    prepend Baz\n    include Baz\n  end\nend\n",
        None,
        "",
    ),
    (
        258,
        "it 'something' do\n  expect(foo).to include(bar), \"baz\"\nend\n",
        None,
        "",
    ),
    (
        266,
        "it 'something' do\n  match([include(foo), anything])\nend\n",
        None,
        "",
    ),
    (
        274,
        "class Foo\n  includes = [include, sdk_include].compact.map do |inc|\n    inc + \"blah\"\n  end.join(' ')\nend\n",
        None,
        "",
    ),
    (
        284,
        "class Foo\n  include Bar\n  include Baz if condition\n  include Qux\nend\n",
        None,
        "",
    ),
    (
        294,
        "begin\n  include Foo\nrescue NameError\n  include Bar\nend\n",
        None,
        "",
    ),
    (
        304,
        "begin\n  include Foo\nensure\n  do_something\nend\n",
        None,
        "",
    ),
];

#[test]
fn pinned_examples() {
    let mut count = 0;
    for method in ["include", "extend", "prepend"] {
        for &(line, source, finding, after) in PER_METHOD {
            let source = source.replace("%m", method);
            let after = after.replace("%m", method);
            assert_eq!(
                positions(&source),
                finding.into_iter().collect::<Vec<_>>(),
                "{method} spec line {line}"
            );
            if finding.is_some() {
                assert_correction(&EmptyLinesAfterModuleInclusion, &source, &after);
            }
            count += 1;
        }
    }
    for &(line, source, finding, _) in SHARED {
        assert_eq!(
            positions(source),
            finding.into_iter().collect::<Vec<_>>(),
            "spec line {line}"
        );
        count += 1;
    }
    assert_eq!(count, 61);
}

#[test]
fn followers_and_lookalikes() {
    for (source, finding, after) in [
        // Modifier `unless`/`while`/`until` bodies and a receiver or argument
        // free inclusion are grouped like `include Baz if condition`.
        ("include A\ninclude B unless c\nx\n", &[][..], ""),
        ("include A\nextend B while c\n", &[], ""),
        ("include A\nprepend B until c\n", &[], ""),
        ("include A\nfoo.include(B)\n", &[], ""),
        ("include A\ninclude\n", &[], ""),
        // A ternary, a non-inclusion modifier body, a safe-navigation call
        // and an inclusion with a literal block are not inclusion sends.
        (
            "include A\nc ? include(B) : x\n",
            &[(1, 1)],
            "include A\n\nc ? include(B) : x\n",
        ),
        ("include A\nx if c\n", &[(1, 1)], "include A\n\nx if c\n"),
        (
            "include A\nfoo&.include(B)\n",
            &[(1, 1)],
            "include A\n\nfoo&.include(B)\n",
        ),
        (
            "include A\ninclude(B) { x }\n",
            &[(1, 1)],
            "include A\n\ninclude(B) { x }\n",
        ),
        // A receiver, no arguments, a value position or a literal block are
        // not checked; a `&block` argument is an argument.
        ("self.include A\nx\n", &[], ""),
        ("x = include A\ny\n", &[], ""),
        ("[include(A)]\ny\n", &[], ""),
        ("include A do\nend\ny\n", &[], ""),
        // A multi-line inclusion is checked after its last line.
        (
            "include A,\n        B\nx\n",
            &[(1, 1)],
            "include A,\n        B\n\nx\n",
        ),
        // Several statements in a branch are a parser `begin`: checked.
        (
            "if c\n  include A\n  x\nend\n",
            &[(2, 3)],
            "if c\n  include A\n\n  x\nend\n",
        ),
        // Upstream has no same-line exemption for inclusions.
        (
            "include A; def x\n  y\nend\n",
            &[(1, 1)],
            "include A; def x\n\n  y\nend\n",
        ),
        // `rustocop:enable` is an enable directive too; `disable` is not.
        ("include A\n# rustocop:enable Foo/Bar\n\nx\n", &[], ""),
        (
            "include A\n# rustocop:enable Foo/Bar\nx\n",
            &[(1, 1)],
            "include A\n# rustocop:enable Foo/Bar\n\nx\n",
        ),
        (
            "include A\n# rubocop:disable Foo/Bar\n\nx\n",
            &[(1, 1)],
            "include A\n\n# rubocop:disable Foo/Bar\n\nx\n",
        ),
        // CRLF keeps its line ending; whitespace-only lines are blank.
        ("include A\r\nx\r\n", &[(1, 1)], "include A\r\n\r\nx\r\n"),
        ("include A\n  \t\nx\n", &[], ""),
        // The line after the code is the `__END__` line: past the code.
        ("include A\n__END__\nx\n", &[], ""),
    ] {
        check(source, finding, after);
    }
    // The block-pass conversion is held by the shared code-object barrier.
    assert_eq!(positions("include &b\ny\n"), [(1, 1)]);
}

#[test]
fn held_insertions() {
    for source in [
        "include A\nx\np __FILE__\n",
        "include A\nx\np __dir__\n",
        "include A\nx\np $0\n",
        "include A\nx\np caller\n",
        "include A\nx\np __LINE__\n",
        "include A\np DATA.read\n__END__\nx\n",
        // The inclusion's line opens a heredoc whose body follows.
        "include Module.new(<<~X)\n  body\nX\nx\n",
    ] {
        held(source);
    }
}

#[test]
fn metadata_is_default_on_and_safe() {
    let cop = EmptyLinesAfterModuleInclusion;
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert!(cop.department_selection_enables());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
}
