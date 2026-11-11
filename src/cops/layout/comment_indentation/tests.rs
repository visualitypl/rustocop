//! Examples ported from RuboCop's `spec/rubocop/cop/layout/comment_indentation_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every
//! example, including the `AllowForAlignment: true`, AccessModifierIndentation
//! `outdent` and per-cop `IndentationWidth` contexts, is replayed at the fixed
//! defaults; findings, messages and single-pass corrected bytes match the
//! fixed replay recorded in the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    offense::Severity,
    pipeline::lint_str,
};

const COP: &str = "Layout/CommentIndentation";

fn positions(source: &str) -> Vec<(u32, u32, String)> {
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| (o.line, o.col, o.message))
        .collect()
}

/// Every pinned example without an offense at the fixed defaults.
const ACCEPTED: &[&str] = &[
    // 14 when allow_for_alignment is false behaves like any allow_for_alignment on outer level acce
    "# comment",
    // 18 when allow_for_alignment is false behaves like any allow_for_alignment on outer level acce
    "hello # comment",
    // 130 when allow_for_alignment is false behaves like any allow_for_alignment around program stru
    "#\ndef m\n  #\n  if a\n    #\n    b\n  # this is accepted\n  elsif aa\n    # this is accepted\n  else\n    #\n  end\n  #\n  case a\n  # this is accepted\n  when 0\n    #\n    b\n  end\n  # this is accepted\n  case a\n  # this is accepted\n  in 0\n    #\n    b\n  end\n  # this is accepted\nrescue\n# this is accepted\nensure\n  #\nend\n#\n",
    // 169 when allow_for_alignment is false behaves like any allow_for_alignment around program stru
    "def m\n  # comment\n\nend\n",
    // 181 when allow_for_alignment is false behaves like any allow_for_alignment near various kinds
    "#\na = {\n  #\n  x: [\n    1\n    #\n  ],\n  #\n  y: func(\n    1\n    #\n  )\n  #\n}\n#\n",
    // 201 when allow_for_alignment is false behaves like any allow_for_alignment near various kinds
    "#\nresult = []\n",
    // 418 when `Layout/AccessModifierIndentation EnforcedStyle: outdent` registers an offense with i
    "class A\n  # rubocop:disable\n  def foo\n  end\n# rubocop:enable\nprivate\n\n  def bar\n  end\nend\n",
    // 447 when `Layout/AccessModifierIndentation EnforcedStyle: outdent` does not register an offens
    "class A\n  # Explains what bar does.\n  private def bar\n  end\nend\n",
    // 457 when `Layout/AccessModifierIndentation EnforcedStyle: outdent` registers an offense for a
    "class A\n# Explains what bar does.\nprivate\n\n  def bar\n  end\nend\n",
];

type Reported = (
    &'static str,
    &'static [(u32, u32, &'static str)],
    &'static str,
);

/// Every pinned example with offenses, and its single-pass correction.
const REPORTED: &[Reported] = &[
    // 306 when allow_for_alignment is false registers an offense for comments with extra indentation
    (
        "def compile_sequence(seq, seq_var)\n  @seq_var = seq_var # Holds the name of the variable holding the AST::Node we are matching\n\n  context.with_temp_variables do |cur_child, cur_index, previous_index|\n    @cur_child_var = cur_child        # To hold the current child node\n    @cur_index_var = cur_index        # To hold the current child index (always >= 0)\n    @prev_index_var = previous_index  # To hold the child index before we enter the looping nodes\n    @cur_index = :seq_head            # Can be any of:\n                                      #   :seq_head : when the current child is actually the sequence head\n                                      #   :variadic_mode : child index held by @cur_index_var\n                                      #   >= 0 : when the current child index is known (from the beginning)\n                                      #   < 0 : when the index is known from the end, where -1 is *past the end*,\n                                      #         -2 is the last child, etc...\n                                      #         This shift of 1 from standard Ruby indices is stored in DELTA\n    @in_sync = false                  # `true` iff `@cur_child_var` and `@cur_index_var` correspond to `@cur_index`\n                                      # Must be true if `@cur_index` is `:variadic_mode`\n    compile_terms(seq)\n  end\nend\n",
        &[
            (
                14,
                39,
                "Incorrect indentation detected (column 38 instead of 4).",
            ),
            (
                16,
                39,
                "Incorrect indentation detected (column 38 instead of 4).",
            ),
        ],
        "def compile_sequence(seq, seq_var)\n  @seq_var = seq_var # Holds the name of the variable holding the AST::Node we are matching\n\n  context.with_temp_variables do |cur_child, cur_index, previous_index|\n    @cur_child_var = cur_child        # To hold the current child node\n    @cur_index_var = cur_index        # To hold the current child index (always >= 0)\n    @prev_index_var = previous_index  # To hold the child index before we enter the looping nodes\n    @cur_index = :seq_head            # Can be any of:\n    #   :seq_head : when the current child is actually the sequence head\n    #   :variadic_mode : child index held by @cur_index_var\n    #   >= 0 : when the current child index is known (from the beginning)\n    #   < 0 : when the index is known from the end, where -1 is *past the end*,\n    #         -2 is the last child, etc...\n    #         This shift of 1 from standard Ruby indices is stored in DELTA\n    @in_sync = false                  # `true` iff `@cur_child_var` and `@cur_index_var` correspond to `@cur_index`\n    # Must be true if `@cur_index` is `:variadic_mode`\n    compile_terms(seq)\n  end\nend\n",
    ),
    // 87 when allow_for_alignment is false behaves like any allow_for_alignment corrects same-colum
    (
        "class Foo\n# a\n# b\n\n# c\n\n# d\n  def bar; end\nend\n",
        &[(
            7,
            1,
            "Incorrect indentation detected (column 0 instead of 2).",
        )],
        "class Foo\n  # a\n  # b\n\n  # c\n\n  # d\n  def bar; end\nend\n",
    ),
    // 114 when allow_for_alignment is false behaves like any allow_for_alignment registers offenses
    (
        " #\n__END__\n  #\n",
        &[(
            1,
            2,
            "Incorrect indentation detected (column 1 instead of 0).",
        )],
        "#\n__END__\n  #\n",
    ),
    // 209 when allow_for_alignment is false behaves like any allow_for_alignment registers an offens
    (
        " # comment 1\n # comment 2\n # comment 3\nhash1 = { a: 0,\n     # comment 4\n          bb: 1,\n          ccc: 2 }\n  if a\n  #\n    b\n  # this is accepted\n  elsif aa\n    # so is this\n  elsif bb\n#\n  else\n   #\n  end\n  case a\n  # this is accepted\n  when 0\n    # so is this\n  when 1\n     #\n    b\n  when 2\n    # this is also accepted\n  end\n  case a\n  # this is accepted\n  in 0\n    # so is this\n  in 1\n  #\n    b\n  in 2\n    # this is also accepted\n  end\n",
        &[
            (
                3,
                2,
                "Incorrect indentation detected (column 1 instead of 0).",
            ),
            (
                5,
                6,
                "Incorrect indentation detected (column 5 instead of 10).",
            ),
            (
                9,
                3,
                "Incorrect indentation detected (column 2 instead of 4).",
            ),
            (
                15,
                1,
                "Incorrect indentation detected (column 0 instead of 4).",
            ),
            (
                17,
                4,
                "Incorrect indentation detected (column 3 instead of 4).",
            ),
            (
                24,
                6,
                "Incorrect indentation detected (column 5 instead of 4).",
            ),
            (
                34,
                3,
                "Incorrect indentation detected (column 2 instead of 4).",
            ),
        ],
        "# comment 1\n# comment 2\n# comment 3\nhash1 = { a: 0,\n          # comment 4\n          bb: 1,\n          ccc: 2 }\n  if a\n    #\n    b\n  # this is accepted\n  elsif aa\n    # so is this\n  elsif bb\n  #\n  else\n    #\n  end\n  case a\n  # this is accepted\n  when 0\n    # so is this\n  when 1\n    #\n    b\n  when 2\n    # this is also accepted\n  end\n  case a\n  # this is accepted\n  in 0\n    # so is this\n  in 1\n    #\n    b\n  in 2\n    # this is also accepted\n  end\n",
    ),
    // 22 when allow_for_alignment is false behaves like any allow_for_alignment on outer level regi
    (
        "=begin\nDoc comment\n=end\n  hello\n #\nhi\n",
        &[(
            5,
            2,
            "Incorrect indentation detected (column 1 instead of 0).",
        )],
        "=begin\nDoc comment\n=end\n  hello\n#\nhi\n",
    ),
    // 43 when allow_for_alignment is false behaves like any allow_for_alignment on outer level regi
    (
        " # comment\n",
        &[(
            1,
            2,
            "Incorrect indentation detected (column 1 instead of 0).",
        )],
        "# comment\n",
    ),
    // 54 when allow_for_alignment is false behaves like any allow_for_alignment on outer level regi
    (
        "  # comment\n",
        &[(
            1,
            3,
            "Incorrect indentation detected (column 2 instead of 0).",
        )],
        "# comment\n",
    ),
    // 65 when allow_for_alignment is false behaves like any allow_for_alignment on outer level regi
    (
        "# a\n  # b\n    # c\n# d\ndef test; end\n",
        &[
            (
                1,
                1,
                "Incorrect indentation detected (column 0 instead of 2).",
            ),
            (
                2,
                3,
                "Incorrect indentation detected (column 2 instead of 4).",
            ),
            (
                3,
                5,
                "Incorrect indentation detected (column 4 instead of 0).",
            ),
        ],
        "  # a\n    # b\n# c\n# d\ndef test; end\n",
    ),
    // 402 when `Layout/AccessModifierIndentation EnforcedStyle: outdent` does not register an offens
    (
        "class A\n  # rubocop:disable\n  def foo\n  end\n  # rubocop:enable\n\nprivate\n\n  def bar\n  end\nend\n",
        &[(
            5,
            3,
            "Incorrect indentation detected (column 2 instead of 0).",
        )],
        "class A\n  # rubocop:disable\n  def foo\n  end\n# rubocop:enable\n\nprivate\n\n  def bar\n  end\nend\n",
    ),
    // 479 when the cop is configured with `IndentationWidth` uses the per-cop `IndentationWidth` ove
    (
        "if foo\n # comment indented by 1 to align with the keyword `else` below\nelse\n  bar\nend\n",
        &[(
            2,
            2,
            "Incorrect indentation detected (column 1 instead of 2).",
        )],
        "if foo\n# comment indented by 1 to align with the keyword `else` below\nelse\n  bar\nend\n",
    ),
];

#[test]
fn pinned_accepted_examples() {
    for source in ACCEPTED {
        assert_eq!(positions(source), vec![], "{source}");
    }
}

#[test]
fn pinned_reported_examples_and_single_pass_corrections() {
    for (source, expected, after) in REPORTED {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(l, c, m)| (l, c, m.to_owned()))
            .collect();
        assert_eq!(positions(source), expected, "{source}");
        assert_correction(&CommentIndentation, source, after);
    }
}

fn msg(column: usize, correct: usize) -> String {
    format!("Incorrect indentation detected (column {column} instead of {correct}).")
}

#[test]
fn metadata_is_fixed() {
    let cop = CommentIndentation;
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.severity(), Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.max_setting().is_none() && cop.autocorrect_incompatible_with().is_empty());
}

#[test]
fn anchors_and_lookalikes() {
    for source in [
        // Not own-line comments.
        "x = 1 # a\n    y = 2\n",
        "=begin\n  doc\n=end\nx = 1\n",
        // A `#` inside a string is not a comment.
        "x = \"\n    # not a comment\n\"\n",
        // `endless`/`end_x` are not `end`; `inside` is not `in`.
        "def f\n  # a\n  endless\nend\n",
        "def f\n  # a\n  end_x\nend\n",
        "case x\nwhen 1\n  # a\n  inside\nend\n",
        // The last comment of a file aligns with column 0; a data section ends
        // the lines.
        "# a\n",
        "x = 1\n# a\n__END__\n  # data\n",
        // A bare access modifier is not outdented (AccessModifierIndentation
        // `indent`), nor `AllowForAlignment`.
        "class A\n  # a\n  private\nend\n",
    ] {
        assert_eq!(positions(source), vec![], "{source:?}");
    }
    assert_eq!(
        positions("class A\n# a\n  private\nend\n"),
        vec![(2, 1, msg(0, 2))]
    );
    assert_eq!(
        positions("x = 1 # a\n      # b\ny = 2\n"),
        vec![(2, 7, msg(6, 0))]
    );
    // Both alternatives before `else`, never a third.
    assert_eq!(
        positions("if a\n  b\n      # c\nelse\nend\n"),
        vec![(3, 7, msg(6, 2))]
    );
    // Blank lines (spaces, CR, NUL) are skipped to find the next line.
    assert_eq!(
        positions("  # a\n\n \r\n\0\nx = 1\n"),
        vec![(1, 3, msg(2, 0))]
    );
}

#[test]
fn canonical_edits_and_chains() {
    for (before, after) in [
        ("  # a\nx = 1\n", "# a\nx = 1\n"),
        ("\t# a\nx = 1\n", "# a\nx = 1\n"),
        ("def f\n# a\n  x\nend\n", "def f\n  # a\n  x\nend\n"),
        (
            "def f\r\n# a\r\n  x\r\nend\r\n",
            "def f\r\n  # a\r\n  x\r\nend\r\n",
        ),
        // The same-column chain above moves with the comment, as one edit.
        (
            "def f\n# a\n\n# b\n# c\n  x\nend\n",
            "def f\n  # a\n\n  # b\n  # c\n  x\nend\n",
        ),
        // A chain stops at code, a different column or a trailing comment.
        (
            "def f\n # a\ny # q\n# b\n# c\n  x\nend\n",
            "def f\n# a\ny # q\n  # b\n  # c\n  x\nend\n",
        ),
        ("x = [\n  1,\n# a\n]\n", "x = [\n  1,\n  # a\n]\n"),
        (
            "def f\n# \u{e9}t\u{e9}\n# b\n  x\nend\n",
            "def f\n  # \u{e9}t\u{e9}\n  # b\n  x\nend\n",
        ),
        // A shebang heading a chain stays at column 0; the rest moves.
        (
            "#!/usr/bin/env ruby\n\n# a\n  x\n",
            "#!/usr/bin/env ruby\n\n  # a\n  x\n",
        ),
    ] {
        assert_correction(&CommentIndentation, before, after);
    }
}

#[test]
fn uncorrectable_shapes_keep_the_finding() {
    for source in [
        // Upstream removes only spaces and tabs right before the `#`.
        "\x0b# a\nx = 1\n",
        // Comments in literal interpolation never move.
        "x = \"#{\n      # a\n  1}\"\n",
        "x = <<~A\n  #{\n      # a\n  1}\nA\n",
        "x = /#{\n      # a\n  1}/\n",
        // A first-line `#!` never moves: indenting a shebang breaks direct
        // execution, outdenting one creates it.
        "#!/usr/bin/env ruby\n  puts 1\n",
        "  #!/bin/sh\nputs 1\n",
    ] {
        let found = correction_offenses(&CommentIndentation, source);
        assert_eq!(found.len(), 1, "{source:?}");
        assert!(found[0].correction.is_none(), "{source:?}");
    }
}

#[test]
fn listed_source_observers_hold_the_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $PROGRAM_NAME",
        "p caller",
        "p SCRIPT_LINES__",
    ] {
        let source = format!("  # a\n{observer}\n");
        let found = correction_offenses(&CommentIndentation, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
    let data = "  # a\np DATA.read\n__END__\nx\n";
    assert!(
        correction_offenses(&CommentIndentation, data)[0]
            .correction
            .is_none()
    );
    assert_correction(
        &CommentIndentation,
        "  # a\np File.read('example.rb')\n",
        "# a\np File.read('example.rb')\n",
    );
}
