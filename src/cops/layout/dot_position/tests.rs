//! Examples ported from RuboCop's `spec/rubocop/cop/layout/dot_position_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every
//! example, including the `Trailing dots style` context, is replayed at the
//! fixed `leading` style; findings, messages and single-pass corrected bytes
//! match the fixed replay recorded in the pinned RuboCop reference run
//! (source and corrected SHA-256 checked against it when the examples were
//! ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str,
};

const COP: &str = "Layout/DotPosition";

type Reported = (u32, &'static str, &'static [(u32, u32)], &'static str);

fn findings(source: &str) -> Vec<(u32, u32, String)> {
    let mut found: Vec<_> = lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| (o.line, o.col, o.message))
        .collect();
    found.sort();
    found
}

fn positions(source: &str) -> Vec<(u32, u32)> {
    findings(source)
        .into_iter()
        .map(|(line, col, message)| {
            assert!(
                message == LEADING.replace("%s", ".") || message == LEADING.replace("%s", "&."),
                "{message}"
            );
            (line, col)
        })
        .collect()
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&DotPosition, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every pinned example without an offense at the fixed defaults.
const ACCEPTED: &[(u32, &str)] = &[
    (54, "something\n  .method_name\n"),
    (61, "puts something"),
    (78, "something.method_name"),
    (83, "something.\n# a comment here\n  method_name\n"),
    (93, "something.\n\n  method_name\n"),
    (139, "something\n  &.method_name\n"),
    (285, "foo(<<~HEREDOC).squish\n  something\nHEREDOC\n"),
    (359, "something\n  .method_name\n"),
    (379, "puts something"),
    (383, "foo(\n  bar\n).baz\n"),
    (391, "l\n.(1)\n"),
    (404, "something.method_name"),
    (417, "something\n  &.method_name\n"),
    (
        440,
        "my_method\n  .something(<<~HERE)\n    something\n  HERE\n  .somethingelse\n",
    ),
    (
        462,
        "my_method\n  .something(<<~HERE)\n    #{something}\n  HERE\n  .somethingelse\n",
    ),
    (
        484,
        "my_method\n  .something(<<~`HERE`)\n    ls -la\n  HERE\n  .somethingelse\n",
    ),
    (
        506,
        "my_method\n  .something(<<~HERE, true)\n    #{something}\n  HERE\n  .somethingelse\n",
    ),
    (
        528,
        "my_method\n  .something(<<~HERE, <<~THERE)\n    something\n  HERE\n    another thing\n  THERE\n  .somethingelse\n",
    ),
    (554, "foo(<<~HEREDOC).squish\n  something\nHEREDOC\n"),
    (565, "<<~HEREDOC\n  something\nHEREDOC\n  .method_name\n"),
    (584, "<<~`HEREDOC`\n  ls -la\nHEREDOC\n  .method_name\n"),
    (603, "<<~HEREDOC.squish\n  something\nHEREDOC\n"),
];

/// Every pinned example with offenses: (spec line, source, findings, single-pass correction).
const REPORTED: &[Reported] = &[
    (
        8,
        "something.\n  method_name\n",
        &[(1, 10)],
        "something\n  .method_name\n",
    ),
    (
        21,
        "something\n  .method_name\nsomething.\n  method_name\n",
        &[(3, 10)],
        "something\n  .method_name\nsomething\n  .method_name\n",
    ),
    (
        38,
        "foo\n  .bar\n  .\n  baz\n",
        &[(3, 3)],
        "foo\n  .bar\n  .baz\n",
    ),
    (65, "l.\n(1)\n", &[(1, 2)], "l\n.(1)\n"),
    (
        103,
        "something(\n  foo, bar\n).\n  method_name\n",
        &[(3, 2)],
        "something(\n  foo, bar\n)\n  .method_name\n",
    ),
    (
        122,
        "something\n  &.method_name\nsomething&.\n  method_name\n",
        &[(3, 10)],
        "something\n  &.method_name\nsomething\n  &.method_name\n",
    ),
    (
        148,
        "@objects = @objects.where(type: :a)\n\n@objects = @objects.\n  with_relation.\n  paginate\n",
        &[(3, 20), (4, 16)],
        "@objects = @objects.where(type: :a)\n\n@objects = @objects\n  .with_relation\n  .paginate\n",
    ),
    (
        171,
        "my_method.\n  something(<<~HERE).\n    something\n  HERE\n  somethingelse\n",
        &[(1, 10), (2, 21)],
        "my_method\n  .something(<<~HERE)\n    something\n  HERE\n  .somethingelse\n",
    ),
    (
        193,
        "my_method.\n  something(<<~HERE).\n    #{something}\n  HERE\n  somethingelse\n",
        &[(1, 10), (2, 21)],
        "my_method\n  .something(<<~HERE)\n    #{something}\n  HERE\n  .somethingelse\n",
    ),
    (
        215,
        "my_method.\n  something(<<~`HERE`).\n    ls -la\n  HERE\n  somethingelse\n",
        &[(1, 10), (2, 23)],
        "my_method\n  .something(<<~`HERE`)\n    ls -la\n  HERE\n  .somethingelse\n",
    ),
    (
        237,
        "my_method.\n  something(<<~HERE, true).\n    #{something}\n  HERE\n  somethingelse\n",
        &[(1, 10), (2, 27)],
        "my_method\n  .something(<<~HERE, true)\n    #{something}\n  HERE\n  .somethingelse\n",
    ),
    (
        259,
        "my_method.\n  something(<<~HERE, <<~THERE).\n    something\n  HERE\n    another thing\n  THERE\n  somethingelse\n",
        &[(1, 10), (2, 31)],
        "my_method\n  .something(<<~HERE, <<~THERE)\n    something\n  HERE\n    another thing\n  THERE\n  .somethingelse\n",
    ),
    (
        295,
        "my_method&.\n  something(<<~HERE)&.\n    something\n  HERE\n  somethingelse\n",
        &[(1, 10), (2, 21)],
        "my_method\n  &.something(<<~HERE)\n    something\n  HERE\n  &.somethingelse\n",
    ),
    (
        318,
        "<<~HEREDOC.\n  something\nHEREDOC\n  method_name\n",
        &[(1, 11)],
        "<<~HEREDOC\n  something\nHEREDOC\n  .method_name\n",
    ),
    (
        337,
        "<<~`HEREDOC`.\n  ls -la\nHEREDOC\n  method_name\n",
        &[(1, 13)],
        "<<~`HEREDOC`\n  ls -la\nHEREDOC\n  .method_name\n",
    ),
    (
        372,
        "something.\n  method_name\n",
        &[(1, 10)],
        "something\n  .method_name\n",
    ),
    (
        408,
        "File.new(something).\nreadlines.map.\ncompact.join(\"\n\")\n",
        &[(1, 20), (2, 14)],
        "File.new(something)\n.readlines.map\n.compact.join(\"\n\")\n",
    ),
    (
        430,
        "something&.\n  method_name\n",
        &[(1, 10)],
        "something\n  &.method_name\n",
    ),
];

#[test]
fn pinned_accepted_examples() {
    for (line, source) in ACCEPTED {
        assert_eq!(positions(source), [], "spec line {line}");
    }
}

#[test]
fn pinned_reported_examples_and_single_pass_corrections() {
    for (line, source, expected, after) in REPORTED {
        assert_eq!(positions(source), *expected, "spec line {line}");
        assert_correction(&DotPosition, source, after);
    }
}

#[test]
fn messages_name_the_actual_operator() {
    assert_eq!(
        findings("a.\n  b\na&.\n  b\n"),
        [
            (
                1,
                2,
                "Place the . on the next line, together with the method name.".to_owned()
            ),
            (
                3,
                2,
                "Place the &. on the next line, together with the method name.".to_owned()
            ),
        ]
    );
}

#[test]
fn metadata_is_fixed() {
    assert_eq!(DotPosition.name(), COP);
    assert!(DotPosition.default_enabled());
    assert!(DotPosition.safe_fixes());
    assert!(DotPosition.autocorrect_incompatible_with().is_empty());
}

#[test]
fn every_dotted_send_shape_is_checked() {
    for (source, expected, after) in [
        // Attribute and index-free writes are `send` nodes with a dot.
        ("a.\n  b = 1\n", (1, 2), "a\n  .b = 1\n"),
        ("a&.\n  b = 1\n", (1, 2), "a\n  &.b = 1\n"),
        // The call side of operator assignments.
        ("a.\n  b += 1\n", (1, 2), "a\n  .b += 1\n"),
        ("a.\n  b ||= 1\n", (1, 2), "a\n  .b ||= 1\n"),
        ("a&.\n  b &&= 1\n", (1, 2), "a\n  &.b &&= 1\n"),
        // Multiple-assignment targets.
        ("a.\n  b, c = 1, 2\n", (1, 2), "a\n  .b, c = 1, 2\n"),
        // Calls with literal blocks and block receivers.
        ("a.\n  b do\n  end\n", (1, 2), "a\n  .b do\n  end\n"),
        ("a do\nend.\n  b\n", (2, 4), "a do\nend\n  .b\n"),
        ("a { }.\n  b { }\n", (1, 6), "a { }\n  .b { }\n"),
        // Arguments without parentheses and operator-named methods.
        ("a.\n  b c\n", (1, 2), "a\n  .b c\n"),
        ("a.\n  +(1)\n", (1, 2), "a\n  .+(1)\n"),
        ("a.\n  ()\n", (1, 2), "a\n  .()\n"),
    ] {
        assert_eq!(positions(source), [expected], "{source}");
        assert_correction(&DotPosition, source, after);
    }
}

#[test]
fn double_colon_and_dotless_calls_are_not_checked() {
    for source in [
        "A::\n  b\n",
        "A::\n  b = 1\n",
        "a::\n  b(1)\n",
        "a +\n  b\n",
        "a[\n  1\n]\n",
        "a[\n  1\n] = 2\n",
        "!a\n",
        "a\n  .b\n  &.c\n  .()\n",
        "a.b.c\n",
        "a.b, c.d = 1, 2\n",
        "a = b.\n  # comment\n  c\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn heredoc_receivers_use_the_terminator_line() {
    // A heredoc nested in a receiver argument is not a direct argument: the
    // receiver ends on its own line, so the terminator lines form a gap.
    assert_eq!(positions("foo(x: <<~A).\n  body\nA\n  bar\n"), []);
    assert_eq!(positions("foo([<<~A]).\n  body\nA\n  bar\n"), []);
    // An index receiver is not a `send` either.
    assert_eq!(positions("a[<<~A].\n  body\nA\n  bar\n"), []);
    // A block receiver is not a `send`: its heredoc arguments do not count.
    assert_eq!(positions("foo(<<~A) { }.\n  body\nA\n  bar\n"), []);
    // A heredoc after a blank line is still a gap.
    assert_eq!(positions("foo(<<~A).\n  body\nA\n\n  bar\n"), []);
    // Plain and dash heredocs, and a heredoc argument without parentheses.
    for (source, expected, after) in [
        (
            "foo(<<A).\nbody\nA\n  bar\n",
            (1, 9),
            "foo(<<A)\nbody\nA\n  .bar\n",
        ),
        (
            "<<-A.\n  body\n  A\n  bar\n",
            (1, 5),
            "<<-A\n  body\n  A\n  .bar\n",
        ),
        (
            "foo <<~A.\n  body\nA\n  bar\n",
            (1, 9),
            "foo <<~A\n  body\nA\n  .bar\n",
        ),
    ] {
        assert_eq!(positions(source), [expected], "{source}");
        assert_correction(&DotPosition, source, after);
    }
}

#[test]
fn dot_only_lines_move_whole_and_other_lines_keep_their_bytes() {
    for (source, after) in [
        // Whitespace around a lone dot goes with its line, CRLF included.
        ("a\n  .b\n \t. \t\n  c\n", "a\n  .b\n  .c\n"),
        ("a\r\n  .b\r\n  .\r\n  c\r\n", "a\r\n  .b\r\n  .c\r\n"),
        // A lone safe-navigation operator is not a lone dot.
        ("a\n  &.\n  b\n", "a\n  \n  &.b\n"),
        // A comment after the dot stays on its line.
        ("a. # note\n  b\n", "a # note\n  .b\n"),
        ("a\n  . # note\n  b\n", "a\n   # note\n  .b\n"),
        // Trailing whitespace after the dot is kept.
        ("a.  \n  b\n", "a  \n  .b\n"),
    ] {
        assert_eq!(positions(source).len(), 1, "{source:?}");
        assert_eq!(fixed(source), after, "{source:?}");
    }
}

#[test]
fn columns_count_characters_and_skip_the_bom() {
    assert_eq!(positions("\u{feff}a.\n  b\n"), [(1, 2)]);
    assert_eq!(positions("ｒｕｂｙ.\n  b\n"), [(1, 5)]);
    assert_eq!(positions("\tfoo.\n\tbar\n"), [(1, 5)]);
    assert_eq!(fixed("\u{feff}a.\n  b\n"), "\u{feff}a\n  .b\n");
}

#[test]
fn listed_source_observers_hold_the_edit() {
    for observer in [
        "__FILE__",
        "__dir__",
        "$0",
        "$PROGRAM_NAME",
        "caller",
        "caller_locations",
        "method(:a).source_location",
        "SCRIPT_LINES__",
    ] {
        let source = format!("x = {observer}\na.\n  b\n");
        let found = correction_offenses(&DotPosition, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
    let data = "a.\n  b\nDATA.read\n__END__\nx\n";
    let found = correction_offenses(&DotPosition, data);
    assert_eq!(found.len(), 1);
    assert!(found[0].correction.is_none());
    // DATA without a data section is not an observer.
    assert_eq!(fixed("a.\n  b\nDATA\n"), "a\n  .b\nDATA\n");
}

#[test]
fn line_number_observers_hold_a_removed_dot_line() {
    // Moving a lone dot removes a physical line; a later `__LINE__` keeps the
    // finding without the edit, while an in-line move still corrects.
    let source = "a\n  .b\n  .\n  c\np __LINE__\n";
    let found = correction_offenses(&DotPosition, source);
    assert_eq!(found.len(), 1);
    assert!(found[0].correction.is_none());
    assert_eq!(fixed("a.\n  c\np __LINE__\n"), "a\n  .c\np __LINE__\n");
}

#[test]
fn findings_only_without_requested_corrections() {
    let offenses: Vec<_> = lint_str("a.\n  b\n")
        .into_iter()
        .filter(|o| o.cop == COP)
        .collect();
    assert_eq!(offenses.len(), 1);
    assert!(offenses[0].correction.is_none());
    assert_eq!(offenses[0].end_line, 1);
}
