//! Examples ported from RuboCop's `spec/rubocop/cop/layout/multiline_operation_indentation_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every
//! example of every style context (`aligned`, `indented`, per-cop
//! `IndentationWidth`, the shared `common` group and the interpolated keyword
//! groups) is replayed at the fixed `aligned` style; findings, messages and
//! single-pass corrected bytes match the fixed replay recorded in
//! the pinned RuboCop reference run. The two `next if`/`next unless`
//! examples (`:ruby32, unsupported_on: :prism`) are top-level `next`, a Ruby
//! 3.4 syntax error here: `top_level_next_is_a_syntax_error` accounts for them.
use super::*;
use crate::{cops::testing::assert_correction, pipeline::lint_str};

const COP: &str = "Layout/MultilineOperationIndentation";

type Reported = (
    &'static [u32],
    &'static str,
    &'static [(u32, u32, &'static str)],
    &'static str,
);

/// Findings sorted by position (RuboCop reports outer operations first).
fn findings(source: &str) -> Vec<(u32, u32, String)> {
    let mut found: Vec<_> = lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| (o.line, o.col, o.message))
        .collect();
    found.sort();
    found
}

/// Every distinct pinned source without an offense at the fixed defaults,
/// with the spec lines that use it.
const ACCEPTED: &[(&[u32], &str)] = &[
    (&[249, 491], "if a +\n   b\n  something\nend\n"),
    (
        &[309],
        "def config_to_allow_offenses\n  a +\n    b == c\nend\n",
    ),
    (
        &[318],
        "a = b.map do |c|\n  c +\n    d\nend\n\nrequires_interpolation = node.children.any? do |s|\n  s.type == :dstr ||\n    s.source_range.source =~ REGEXP\nend\n",
    ),
    (&[332], "one ||\n  two ? 3 : 5\n"),
    (&[446, 668], "a = b +\n    c +\n    d\n"),
    (
        &[454],
        "tmp_dir = ENV['TMPDIR'] || ENV['TMP'] || ENV['TEMP'] ||\n          Etc.systmpdir || '/tmp'\n",
    ),
    (&[18], "call a,\n     !b\n"),
    (&[25], "begin\nrescue =>\n  A[]\nend\n"),
    (&[34], "a +\n  b\n"),
    (
        &[41],
        "a = b.map do |c|\n  c +\n    b +\n    d do\n      x +\n        y\n    end\nend\n",
    ),
    (
        &[80],
        "a\n .(args)\n\nFoo\n.a\n  .b\n\nFoo\n.a\n  .b(c)\n\nFoo.&(\n    foo,\n    bar\n)\n\nexpect { Foo.new }.\n  to change { Bar.count }.\n      from(1).to(2)\n",
    ),
    (
        &[155],
        "dm[i][j] = [\n  dm[i-1][j-1] +\n    (this[j-1] == that[i-1] ? 0 : sub),\n  dm[i][j-1] + ins,\n  dm[i-1][j] + del\n].min\n",
    ),
    (&[166], "a =\n  'foo' +\n  'bar'\n"),
    (&[174], "a['test'] =\n  'foo' +\n  'bar'\n"),
    (&[182], "a ||\n  b\n"),
    (&[189], "a &&\n  b &&\n  c\n"),
    (
        &[197],
        "for x in a\n  something &&\n    something_else\nend\n",
    ),
    (&[206], "(a +\n b)\n"),
    (
        &[213],
        "subject.each do |item|\n  result = resolve(locale) and return result\nend and nil\n",
    ),
    (
        &[221],
        "payment = Models::IncomingPayments[\n        id:      input['incoming-payment-id'],\n           user_id: @user[:id]]\n",
    ),
    (
        &[508],
        "MSG = 'Use 2 (not %d) spaces for indenting a ' \\\n      'broken line.'\n",
    ),
    (
        &[515],
        "Parser::Source::Range.new(expr.source_buffer,\n                          begin_pos,\n                          begin_pos + line.length)\n",
    ),
    (&[523], "a(b +\n    c +\nd)\n"),
    (
        &[531],
        "arg_array.size == a.size && (\n  arg_array == a ||\n  arg_array.map(&:children) == a.map(&:children)\n)\n",
    ),
    (&[746], "until a +\n      b\n  something\nend\n"),
    (&[593], "if a\n  something &&\n    something_else\nend\n"),
    (
        &[593],
        "unless a\n  something &&\n    something_else\nend\n",
    ),
    (&[593], "while a\n  something &&\n    something_else\nend\n"),
    (&[593], "until a\n  something &&\n    something_else\nend\n"),
];

/// Every distinct pinned source with offenses: spec lines, source,
/// (line, column, message) findings and the single-pass correction.
const REPORTED: &[Reported] = &[
    (
        &[258, 482],
        "if a +\n    b\n  something\nend\n",
        &[(
            2,
            5,
            "Align the operands of a condition in an `if` statement spanning multiple lines.",
        )],
        "if a +\n   b\n  something\nend\n",
    ),
    (
        &[275, 540],
        "def config_to_allow_offenses\n  a +\n  b == c\nend\n",
        &[(
            3,
            3,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "def config_to_allow_offenses\n  a +\n    b == c\nend\n",
    ),
    (
        &[292],
        "private def config_to_allow_offenses\n  a +\n  b == c\nend\n",
        &[(
            3,
            3,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "private def config_to_allow_offenses\n  a +\n    b == c\nend\n",
    ),
    (
        &[339],
        "it \"should convert \" +\n  \"a to \" +\n  \"b\" do\nend\n",
        &[
            (
                3,
                3,
                "Align the operands of an expression spanning multiple lines.",
            ),
            (
                2,
                3,
                "Align the operands of an expression spanning multiple lines.",
            ),
        ],
        "it \"should convert \" +\n   \"a to \" +\n   \"b\" do\nend\n",
    ),
    (
        &[357],
        "puts a, 1 +\n  2\n",
        &[(
            2,
            3,
            "Align the operands of an expression spanning multiple lines.",
        )],
        "puts a, 1 +\n        2\n",
    ),
    (
        &[371],
        "def f\n  flash[:error] = 'Here is a string ' \\\n                  'That spans' <<\n      'multiple lines'\nend\n",
        &[(
            4,
            7,
            "Align the operands of an expression in an assignment spanning multiple lines.",
        )],
        "def f\n  flash[:error] = 'Here is a string ' \\\n                  'That spans' <<\n                  'multiple lines'\nend\n",
    ),
    (
        &[390],
        "Error = 'Here is a string ' +\n        'That spans' <<\n  'multiple lines'\n",
        &[(
            3,
            3,
            "Align the operands of an expression in an assignment spanning multiple lines.",
        )],
        "Error = 'Here is a string ' +\n        'That spans' <<\n        'multiple lines'\n",
    ),
    (
        &[405],
        "unless a +\n  b\n  something\nend\n",
        &[(
            2,
            3,
            "Align the operands of a condition in an `unless` statement spanning multiple lines.",
        )],
        "unless a +\n       b\n  something\nend\n",
    ),
    (
        &[461],
        "bar *= Foo +\n  a +\n       b(c)\n",
        &[(
            2,
            3,
            "Align the operands of an expression in an assignment spanning multiple lines.",
        )],
        "bar *= Foo +\n       a +\n       b(c)\n",
    ),
    (
        &[54],
        "a +\nb\n",
        &[(
            2,
            1,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "a +\n  b\n",
    ),
    (
        &[67],
        "a +\n b\n",
        &[(
            2,
            2,
            "Use 2 (not 1) spaces for indenting an expression spanning multiple lines.",
        )],
        "a +\n  b\n",
    ),
    (
        &[104],
        "a ||\n   b\nc and\n   d\n",
        &[
            (
                2,
                4,
                "Use 2 (not 3) spaces for indenting an expression spanning multiple lines.",
            ),
            (
                4,
                4,
                "Use 2 (not 3) spaces for indenting an expression spanning multiple lines.",
            ),
        ],
        "a ||\n  b\nc and\n  d\n",
    ),
    (
        &[122],
        "a ||\n  b ||\n    c\n",
        &[(
            3,
            5,
            "Use 2 (not 4) spaces for indenting an expression spanning multiple lines.",
        )],
        "a ||\n  b ||\n  c\n",
    ),
    (
        &[138],
        "[\n a +\n b\n]\n",
        &[(
            3,
            2,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "[\n a +\n   b\n]\n",
    ),
    (
        &[230],
        "a +\nb < 3\n",
        &[(
            2,
            1,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "a +\n  b < 3\n",
    ),
    (
        &[620],
        "for n in a +\n  b\nend\n",
        &[(
            2,
            3,
            "Align the operands of a collection in a `for` statement spanning multiple lines.",
        )],
        "for n in a +\n         b\nend\n",
    ),
    (
        &[635],
        "for n in a +\n    b\nend\n",
        &[(
            2,
            5,
            "Align the operands of a collection in a `for` statement spanning multiple lines.",
        )],
        "for n in a +\n         b\nend\n",
    ),
    (
        &[643],
        "a = b +\n  c +\n  d\n",
        &[
            (
                3,
                3,
                "Align the operands of an expression in an assignment spanning multiple lines.",
            ),
            (
                2,
                3,
                "Align the operands of an expression in an assignment spanning multiple lines.",
            ),
        ],
        "a = b +\n    c +\n    d\n",
    ),
    (
        &[651],
        "a ||\n  b\nc and\n    d\n",
        &[(
            4,
            5,
            "Use 2 (not 4) spaces for indenting an expression spanning multiple lines.",
        )],
        "a ||\n  b\nc and\n  d\n",
    ),
    (
        &[687],
        "if a +\n        b\n  something\nend\n",
        &[(
            2,
            9,
            "Align the operands of a condition in an `if` statement spanning multiple lines.",
        )],
        "if a +\n   b\n  something\nend\n",
    ),
    (
        &[428],
        "if a or\n    b\n  something\nend\n",
        &[(
            2,
            5,
            "Align the operands of a condition in an `if` statement spanning multiple lines.",
        )],
        "if a or\n   b\n  something\nend\n",
    ),
    (
        &[428],
        "unless a or\n    b\n  something\nend\n",
        &[(
            2,
            5,
            "Align the operands of a condition in an `unless` statement spanning multiple lines.",
        )],
        "unless a or\n       b\n  something\nend\n",
    ),
    (
        &[428],
        "while a or\n    b\n  something\nend\n",
        &[(
            2,
            5,
            "Align the operands of a condition in a `while` statement spanning multiple lines.",
        )],
        "while a or\n      b\n  something\nend\n",
    ),
    (
        &[428],
        "until a or\n    b\n  something\nend\n",
        &[(
            2,
            5,
            "Align the operands of a condition in an `until` statement spanning multiple lines.",
        )],
        "until a or\n      b\n  something\nend\n",
    ),
    (
        &[563],
        "if receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\nend\nif receiver.\n    nil?\nend\n",
        &[
            (
                3,
                5,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
            (
                2,
                5,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
        ],
        "if receiver.nil? &&\n   !args.empty? &&\n   FORBIDDEN_METHODS.include?(method_name)\nend\nif receiver.\n    nil?\nend\n",
    ),
    (
        &[575],
        "if receiver.nil? &&\n  !args.empty? &&\n  FORBIDDEN_METHODS.include?(method_name)\nend\n",
        &[
            (
                3,
                3,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
            (
                2,
                3,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
        ],
        "if receiver.nil? &&\n   !args.empty? &&\n   FORBIDDEN_METHODS.include?(method_name)\nend\n",
    ),
    (
        &[563],
        "unless receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\nend\nunless receiver.\n    nil?\nend\n",
        &[
            (
                3,
                5,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
            (
                2,
                5,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
        ],
        "unless receiver.nil? &&\n       !args.empty? &&\n       FORBIDDEN_METHODS.include?(method_name)\nend\nunless receiver.\n    nil?\nend\n",
    ),
    (
        &[575],
        "unless receiver.nil? &&\n  !args.empty? &&\n  FORBIDDEN_METHODS.include?(method_name)\nend\n",
        &[
            (
                3,
                3,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
            (
                2,
                3,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
        ],
        "unless receiver.nil? &&\n       !args.empty? &&\n       FORBIDDEN_METHODS.include?(method_name)\nend\n",
    ),
    (
        &[563],
        "while receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\nend\nwhile receiver.\n    nil?\nend\n",
        &[
            (
                3,
                5,
                "Align the operands of a condition in a `while` statement spanning multiple lines.",
            ),
            (
                2,
                5,
                "Align the operands of a condition in a `while` statement spanning multiple lines.",
            ),
        ],
        "while receiver.nil? &&\n      !args.empty? &&\n      FORBIDDEN_METHODS.include?(method_name)\nend\nwhile receiver.\n    nil?\nend\n",
    ),
    (
        &[575],
        "while receiver.nil? &&\n  !args.empty? &&\n  FORBIDDEN_METHODS.include?(method_name)\nend\n",
        &[
            (
                3,
                3,
                "Align the operands of a condition in a `while` statement spanning multiple lines.",
            ),
            (
                2,
                3,
                "Align the operands of a condition in a `while` statement spanning multiple lines.",
            ),
        ],
        "while receiver.nil? &&\n      !args.empty? &&\n      FORBIDDEN_METHODS.include?(method_name)\nend\n",
    ),
    (
        &[563],
        "until receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\nend\nuntil receiver.\n    nil?\nend\n",
        &[
            (
                3,
                5,
                "Align the operands of a condition in an `until` statement spanning multiple lines.",
            ),
            (
                2,
                5,
                "Align the operands of a condition in an `until` statement spanning multiple lines.",
            ),
        ],
        "until receiver.nil? &&\n      !args.empty? &&\n      FORBIDDEN_METHODS.include?(method_name)\nend\nuntil receiver.\n    nil?\nend\n",
    ),
    (
        &[575],
        "until receiver.nil? &&\n  !args.empty? &&\n  FORBIDDEN_METHODS.include?(method_name)\nend\n",
        &[
            (
                3,
                3,
                "Align the operands of a condition in an `until` statement spanning multiple lines.",
            ),
            (
                2,
                3,
                "Align the operands of a condition in an `until` statement spanning multiple lines.",
            ),
        ],
        "until receiver.nil? &&\n      !args.empty? &&\n      FORBIDDEN_METHODS.include?(method_name)\nend\n",
    ),
    (
        &[604],
        "return unless receiver.nil? &&\n  !args.empty? &&\n  FORBIDDEN_METHODS.include?(method_name)\n",
        &[
            (
                3,
                3,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
            (
                2,
                3,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
        ],
        "return unless receiver.nil? &&\n              !args.empty? &&\n              FORBIDDEN_METHODS.include?(method_name)\n",
    ),
    (
        &[604],
        "return if receiver.nil? &&\n  !args.empty? &&\n  FORBIDDEN_METHODS.include?(method_name)\n",
        &[
            (
                3,
                3,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
            (
                2,
                3,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
        ],
        "return if receiver.nil? &&\n          !args.empty? &&\n          FORBIDDEN_METHODS.include?(method_name)\n",
    ),
    (
        &[706],
        "if receiver.nil? &&\n        !args.empty? &&\n        FORBIDDEN_METHODS.include?(method_name)\nend\nif receiver.\n        nil?\nend\n",
        &[
            (
                3,
                9,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
            (
                2,
                9,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
        ],
        "if receiver.nil? &&\n   !args.empty? &&\n   FORBIDDEN_METHODS.include?(method_name)\nend\nif receiver.\n        nil?\nend\n",
    ),
    (
        &[718],
        "if receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\nend\n",
        &[
            (
                3,
                5,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
            (
                2,
                5,
                "Align the operands of a condition in an `if` statement spanning multiple lines.",
            ),
        ],
        "if receiver.nil? &&\n   !args.empty? &&\n   FORBIDDEN_METHODS.include?(method_name)\nend\n",
    ),
    (
        &[736],
        "if a\n  something &&\n        something_else\nend\n",
        &[(
            3,
            9,
            "Use 2 (not 6) spaces for indenting an expression spanning multiple lines.",
        )],
        "if a\n  something &&\n    something_else\nend\n",
    ),
    (
        &[706],
        "unless receiver.nil? &&\n        !args.empty? &&\n        FORBIDDEN_METHODS.include?(method_name)\nend\nunless receiver.\n        nil?\nend\n",
        &[
            (
                3,
                9,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
            (
                2,
                9,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
        ],
        "unless receiver.nil? &&\n       !args.empty? &&\n       FORBIDDEN_METHODS.include?(method_name)\nend\nunless receiver.\n        nil?\nend\n",
    ),
    (
        &[718],
        "unless receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\nend\n",
        &[
            (
                3,
                5,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
            (
                2,
                5,
                "Align the operands of a condition in an `unless` statement spanning multiple lines.",
            ),
        ],
        "unless receiver.nil? &&\n       !args.empty? &&\n       FORBIDDEN_METHODS.include?(method_name)\nend\n",
    ),
    (
        &[736],
        "unless a\n  something &&\n        something_else\nend\n",
        &[(
            3,
            9,
            "Use 2 (not 6) spaces for indenting an expression spanning multiple lines.",
        )],
        "unless a\n  something &&\n    something_else\nend\n",
    ),
    (
        &[706],
        "while receiver.nil? &&\n        !args.empty? &&\n        FORBIDDEN_METHODS.include?(method_name)\nend\nwhile receiver.\n        nil?\nend\n",
        &[
            (
                3,
                9,
                "Align the operands of a condition in a `while` statement spanning multiple lines.",
            ),
            (
                2,
                9,
                "Align the operands of a condition in a `while` statement spanning multiple lines.",
            ),
        ],
        "while receiver.nil? &&\n      !args.empty? &&\n      FORBIDDEN_METHODS.include?(method_name)\nend\nwhile receiver.\n        nil?\nend\n",
    ),
    (
        &[718],
        "while receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\nend\n",
        &[
            (
                3,
                5,
                "Align the operands of a condition in a `while` statement spanning multiple lines.",
            ),
            (
                2,
                5,
                "Align the operands of a condition in a `while` statement spanning multiple lines.",
            ),
        ],
        "while receiver.nil? &&\n      !args.empty? &&\n      FORBIDDEN_METHODS.include?(method_name)\nend\n",
    ),
    (
        &[736],
        "while a\n  something &&\n        something_else\nend\n",
        &[(
            3,
            9,
            "Use 2 (not 6) spaces for indenting an expression spanning multiple lines.",
        )],
        "while a\n  something &&\n    something_else\nend\n",
    ),
    (
        &[706],
        "until receiver.nil? &&\n        !args.empty? &&\n        FORBIDDEN_METHODS.include?(method_name)\nend\nuntil receiver.\n        nil?\nend\n",
        &[
            (
                3,
                9,
                "Align the operands of a condition in an `until` statement spanning multiple lines.",
            ),
            (
                2,
                9,
                "Align the operands of a condition in an `until` statement spanning multiple lines.",
            ),
        ],
        "until receiver.nil? &&\n      !args.empty? &&\n      FORBIDDEN_METHODS.include?(method_name)\nend\nuntil receiver.\n        nil?\nend\n",
    ),
    (
        &[718],
        "until receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\nend\n",
        &[
            (
                3,
                5,
                "Align the operands of a condition in an `until` statement spanning multiple lines.",
            ),
            (
                2,
                5,
                "Align the operands of a condition in an `until` statement spanning multiple lines.",
            ),
        ],
        "until receiver.nil? &&\n      !args.empty? &&\n      FORBIDDEN_METHODS.include?(method_name)\nend\n",
    ),
    (
        &[736],
        "until a\n  something &&\n        something_else\nend\n",
        &[(
            3,
            9,
            "Use 2 (not 6) spaces for indenting an expression spanning multiple lines.",
        )],
        "until a\n  something &&\n    something_else\nend\n",
    ),
];

#[test]
fn pinned_accepted_examples() {
    for (lines, source) in ACCEPTED {
        assert!(
            findings(source).is_empty(),
            "spec lines {lines:?}: {source}"
        );
    }
}

#[test]
fn pinned_reported_examples_and_single_pass_corrections() {
    for (lines, source, expected, after) in REPORTED {
        let mut expected: Vec<_> = expected
            .iter()
            .map(|(line, col, message)| (*line, *col, (*message).to_owned()))
            .collect();
        expected.sort();
        assert_eq!(findings(source), expected, "spec lines {lines:?}: {source}");
        assert_correction(&MultilineOperationIndentation, source, after);
    }
}

#[test]
fn top_level_next_is_a_syntax_error() {
    // Spec line 612 (`next if`/`next unless`, `:ruby32, unsupported_on:
    // :prism`): the fixed Ruby 3.4 grammar rejects a top-level `next`, so the
    // file is an error with no finding; inside a block it is checked.
    for keyword in ["if", "unless"] {
        let source = format!("next {keyword} 5 ||\n  7\n");
        assert!(crate::parser::parse(source.as_bytes()).errors().count() > 0);
        assert!(findings(&source).is_empty());
        let source = format!("loop do\n  next {keyword} 5 ||\n    7\nend\n");
        assert_eq!(
            findings(&source),
            [(
                3,
                5,
                format!(
                    "Align the operands of a condition in an `{keyword}` statement spanning multiple lines."
                )
            )]
        );
    }
}

#[test]
fn metadata_is_fixed() {
    let cop = MultilineOperationIndentation;
    assert_eq!(cop.name(), COP);
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert!(cop.autocorrect_incompatible_with().is_empty());
}

#[test]
fn operators_keywords_and_contexts() {
    let use_2 = "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.";
    for (source, expected, after) in [
        ("a +\nb\n", (2, 1, use_2.to_owned()), "a +\n  b\n"),
        ("a and\nb\n", (2, 1, use_2.to_owned()), "a and\n  b\n"),
        ("a ||\nb\n", (2, 1, use_2.to_owned()), "a ||\n  b\n"),
        (
            "x = a +\n  b\n",
            (
                2,
                3,
                "Align the operands of an expression in an assignment spanning multiple lines."
                    .to_owned(),
            ),
            "x = a +\n    b\n",
        ),
        (
            "x =\n  a +\n b\n",
            (
                3,
                2,
                "Align the operands of an expression in an assignment spanning multiple lines."
                    .to_owned(),
            ),
            "x =\n  a +\n  b\n",
        ),
        (
            "foo a +\n  b\n",
            (
                2,
                3,
                "Align the operands of an expression spanning multiple lines.".to_owned(),
            ),
            "foo a +\n    b\n",
        ),
        // The whole right operand moves, every line.
        (
            "a +\nfoo(1,\n  2)\n",
            (2, 1, use_2.to_owned()),
            "a +\n  foo(1,\n    2)\n",
        ),
    ] {
        assert_eq!(findings(source), [expected], "{source}");
        assert_correction(&MultilineOperationIndentation, source, after);
    }
    for source in [
        // Dotted calls belong to MultilineMethodCallIndentation.
        "a\n.b\n",
        "a.+(\nb)\n",
        // Unary operators, `[]` and grouped expressions are not checked.
        "!\na\n",
        "a[\nb]\n",
        "(a +\nb)\n",
        "foo(a +\nb)\n",
        // A `private def` argument keeps the two-space indentation.
        "private def f(a)\n  a +\n    b\nend\n",
        "a +\n  b\n",
    ] {
        assert!(findings(source).is_empty(), "{source}");
    }
}
