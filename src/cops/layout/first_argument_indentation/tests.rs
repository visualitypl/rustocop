//! Ported from RuboCop first_argument_indentation_spec.rb at 97f30c4
//! (MIT, Copyright (c) 2012–26 Bozhidar Batsov).
use super::FirstArgumentIndentation as Cop;
use crate::{
    cops::{Cop as _, layout::element_indentation::corrected, testing::correction_offenses},
    pipeline::lint_str,
};

fn found(source: &str) -> Vec<(u32, u32, String)> {
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == Cop.name())
        .map(|o| (o.line, o.col, o.message))
        .collect()
}
fn offense(source: &str, expected: &[(u32, u32, &str)]) {
    let expected: Vec<_> = expected
        .iter()
        .map(|(l, c, m)| (*l, *c, (*m).to_owned()))
        .collect();
    assert_eq!(found(source), expected, "{source}");
}
fn correction(before: &str, after: &str) {
    assert_eq!(corrected(&Cop, before), after, "{before}");
}
/// `expect_correction(..., loop: false)`: one correction pass.
fn correction_once(before: &str, after: &str) {
    let offenses = correction_offenses(&Cop, before);
    let applied = crate::corrections::apply(before.as_bytes(), &offenses).unwrap();
    assert_eq!(String::from_utf8(applied.bytes).unwrap(), after, "{before}");
}
fn none(source: &str) {
    assert_eq!(found(source), [], "{source}");
}

/// The shared `common behavior` examples at `IndentationWidth: 2`, as the
/// fixed `special_for_inner_method_call_in_parentheses` context runs them.
/// The width-4 and per-cop `IndentationWidth: 4` contexts are configuration
/// skips.
#[test]
fn pinned_common_behavior() {
    // registers an offense and corrects an over-indented first argument (spec line 12)
    offense(
        r#"run(
    :foo,
    bar: 3
)
"#,
        &[(
            2,
            5,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"run(
    :foo,
    bar: 3
)
"#,
        r#"run(
  :foo,
    bar: 3
)
"#,
    );
    // registers an offense and corrects an over-indented first argument of `super` (spec line 29)
    offense(
        r#"super(
    :foo,
    bar: 3
)
"#,
        &[(
            2,
            5,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"super(
    :foo,
    bar: 3
)
"#,
        r#"super(
  :foo,
    bar: 3
)
"#,
    );
    // registers an offense and corrects an over-indented first argument on an alphanumeric method name (spec line 46)
    offense(
        r#"self.run(
    :foo,
    bar: 3
)
"#,
        &[(
            2,
            5,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"self.run(
    :foo,
    bar: 3
)
"#,
        r#"self.run(
  :foo,
    bar: 3
)
"#,
    );
    // registers an offense and corrects an over-indented first argument on a pipe method name (spec line 63)
    offense(
        r#"self.|(
    :foo,
    bar: 3
)
"#,
        &[(
            2,
            5,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"self.|(
    :foo,
    bar: 3
)
"#,
        r#"self.|(
  :foo,
    bar: 3
)
"#,
    );
    // registers an offense and corrects an over-indented first argument on a plus sign method name (spec line 80)
    offense(
        r#"self.+(
    :foo,
    bar: 3
)
"#,
        &[(
            2,
            5,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"self.+(
    :foo,
    bar: 3
)
"#,
        r#"self.+(
  :foo,
    bar: 3
)
"#,
    );
    // registers an offense and corrects an under-indented first argument (spec line 97)
    offense(
        r#"run(
 :foo,
    bar: 3
)
"#,
        &[(
            2,
            2,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"run(
 :foo,
    bar: 3
)
"#,
        r#"run(
  :foo,
    bar: 3
)
"#,
    );
    // registers an offense and corrects lines affected by another offense (spec line 114)
    offense(
        r#"foo(
 bar(
  7
)
)
"#,
        &[
            (
                2,
                2,
                "Indent the first argument one step more than the start of the previous line.",
            ),
            (3, 3, "Bad indentation of the first argument."),
        ],
    );
    correction_once(
        r#"foo(
 bar(
  7
)
)
"#,
        r#"foo(
  bar(
   7
 )
)
"#,
    );
    // registers an offense and corrects over-indented first arguments in nested method calls (spec line 135)
    offense(
        r#"foo
  .bar(
  baz(
      qux
    )
  )
"#,
        &[
            (
                3,
                3,
                "Indent the first argument one step more than the start of the previous line.",
            ),
            (4, 7, "Bad indentation of the first argument."),
        ],
    );
    correction(
        r#"foo
  .bar(
  baz(
      qux
    )
  )
"#,
        r#"foo
  .bar(
    baz(
      qux
      )
  )
"#,
    );
    // registers an offense and corrects over-indented first arguments in nested method calls with hash arguments (spec line 157)
    offense(
        r#"foo
  .bar(
  bar(
      key: value
    )
  )
"#,
        &[
            (
                3,
                3,
                "Indent the first argument one step more than the start of the previous line.",
            ),
            (4, 7, "Bad indentation of the first argument."),
        ],
    );
    correction(
        r#"foo
  .bar(
  bar(
      key: value
    )
  )
"#,
        r#"foo
  .bar(
    bar(
      key: value
      )
  )
"#,
    );
    // registers an offense and corrects an under-indented 1st argument (spec line 180)
    offense(
        r#"receiver&.run(
 :foo,
    bar: 3
)
"#,
        &[(
            2,
            2,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"receiver&.run(
 :foo,
    bar: 3
)
"#,
        r#"receiver&.run(
  :foo,
    bar: 3
)
"#,
    );
    // accepts an unindented value (spec line 199)
    none(
        r#"foo.baz =
bar
"#,
    );
    // accepts a correctly indented first argument and does not care  (spec line 208)
    none(
        r#"x = run(
  :foo,
    bar: 3
)
"#,
    );
    // accepts a correctly indented first argument (spec line 219)
    none(
        r#"x =
  run(
    :foo)
"#,
    );
    // registers an offense and corrects an under-indented first argument (spec line 227)
    offense(
        r#"@x =
  run(
  :foo)
"#,
        &[(
            3,
            3,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"@x =
  run(
  :foo)
"#,
        r#"@x =
  run(
    :foo)
"#,
    );
    // accepts a first argument that is not preceded by a line break (spec line 244)
    none(
        r#"run :foo,
    bar: 3
"#,
    );
    // accepts a correctly indented first argument (spec line 252)
    none(
        r#"puts x.
  merge(
    b: 2
  )
"#,
    );
    // registers an offense and corrects an over-indented first argument (spec line 261)
    offense(
        r#"puts x.
  merge(
      b: 2
  )
"#,
        &[(
            3,
            7,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"puts x.
  merge(
      b: 2
  )
"#,
        r#"puts x.
  merge(
    b: 2
  )
"#,
    );
    // accepts a correctly indented first argument preceded by an empty line (spec line 278)
    none(
        r#"puts x.
  merge(

    b: 2
  )
"#,
    );
    // accepts a correctly indented first argument (spec line 289)
    none(
        r#"puts x.
  merge( # EOL comment
    # comment
    b: 2
  )
"#,
    );
    // registers an offense and corrects an under-indented first argument (spec line 299)
    offense(
        r#"puts x.
  merge(
  # comment
  b: 2
  )
"#,
        &[(
            4,
            3,
            "Indent the first argument one step more than the start of the previous line (not counting the comment).",
        )],
    );
    correction(
        r#"puts x.
  merge(
  # comment
  b: 2
  )
"#,
        r#"puts x.
  merge(
  # comment
    b: 2
  )
"#,
    );
    // accepts method calls with no arguments (spec line 320)
    none(
        r#"run()
run_again
"#,
    );
    // accepts operator calls (spec line 327)
    none(
        r#"params = default_cfg.keys - %w(Description) -
         cfg.keys
"#,
    );
    // does not view []= as an outer method call (spec line 334)
    none(
        r#"@subject_results[subject] = original.update(
  mutation_results: (dup << mutation_result),
  tests:            test_result.tests
)
"#,
    );
    // does not view chained call as an outer method call (spec line 343)
    none(
        r#"A = Regexp.union(
  /[A-Za-z_][A-Za-z\d_]*[!?=]?/,
  *AST::Types::OPERATOR_METHODS.map(&:to_s)
).freeze
"#,
    );
}

/// The `special_for_inner_method_call_in_parentheses` context, except the
/// `ArgumentAlignment: with_fixed_indentation` sub-context (another style).
#[test]
fn pinned_special_for_inner_method_call_in_parentheses() {
    // registers an offense and corrects an over-indented first argument (spec line 532)
    offense(
        r#"run(:foo, defaults.merge(
                        bar: 3))
"#,
        &[(
            2,
            25,
            "Indent the first argument one step more than `defaults.merge(`.",
        )],
    );
    correction(
        r#"run(:foo, defaults.merge(
                        bar: 3))
"#,
        r#"run(:foo, defaults.merge(
            bar: 3))
"#,
    );
    // registers an offense and corrects an under-indented first argument (spec line 545)
    offense(
        r#"run(:foo, defaults.
          merge(
  bar: 3))
"#,
        &[(
            3,
            3,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"run(:foo, defaults.
          merge(
  bar: 3))
"#,
        r#"run(:foo, defaults.
          merge(
            bar: 3))
"#,
    );
    // accepts a correctly indented first argument in interpolation (spec line 560)
    none(
        r#"puts %(
  <p>
    #{Array(
      42
    )}
  </p>
)
"#,
    );
    // accepts a correctly indented first argument with fullwidth characters (spec line 572)
    none(
        r#"puts('Ｒｕｂｙ', f(
                   a))
"#,
    );
    // corrects the entire method call when closing parenthesis is on a separate line (spec line 579)
    offense(
        r#"instance_variable_set(
  "@diagnostics",
  Diagnostic.where(
              'limit >= 10 and limit < 100',
            )
)
"#,
        &[(
            4,
            15,
            "Indent the first argument one step more than `Diagnostic.where(`.",
        )],
    );
    correction(
        r#"instance_variable_set(
  "@diagnostics",
  Diagnostic.where(
              'limit >= 10 and limit < 100',
            )
)
"#,
        r#"instance_variable_set(
  "@diagnostics",
  Diagnostic.where(
    'limit >= 10 and limit < 100',
  )
)
"#,
    );
    // corrects the entire method call with multiple arguments (spec line 600)
    offense(
        r#"instance_variable_set(
  "@diagnostics",
  Diagnostic.where(
        'limit >= ? and limit < ?',
        LIMITS[:lowest_value], LIMITS[:highest_value]
      )
)
"#,
        &[(
            4,
            9,
            "Indent the first argument one step more than `Diagnostic.where(`.",
        )],
    );
    correction(
        r#"instance_variable_set(
  "@diagnostics",
  Diagnostic.where(
        'limit >= ? and limit < ?',
        LIMITS[:lowest_value], LIMITS[:highest_value]
      )
)
"#,
        r#"instance_variable_set(
  "@diagnostics",
  Diagnostic.where(
    'limit >= ? and limit < ?',
    LIMITS[:lowest_value], LIMITS[:highest_value]
  )
)
"#,
    );
    // corrects the entire method call chain (spec line 623)
    offense(
        r#"instance_variable_set(
  "@diagnostic_ids",
  Diagnostic.where(
        'limit >= ? and limit < ?',
        LIMITS[:lowest_value], LIMITS[:highest_value]
      )
      .where(id: 1)
      .ids,
)
"#,
        &[(
            4,
            9,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"instance_variable_set(
  "@diagnostic_ids",
  Diagnostic.where(
        'limit >= ? and limit < ?',
        LIMITS[:lowest_value], LIMITS[:highest_value]
      )
      .where(id: 1)
      .ids,
)
"#,
        r#"instance_variable_set(
  "@diagnostic_ids",
  Diagnostic.where(
    'limit >= ? and limit < ?',
    LIMITS[:lowest_value], LIMITS[:highest_value]
  )
  .where(id: 1)
  .ids,
)
"#,
    );
    // corrects only the first argument when a method call chain beginning its line is under-indented (spec line 650)
    offense(
        r#"foo.bar(
  baz(:qux).quux(
corge,
    grault).garply
)
"#,
        &[(
            3,
            1,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"foo.bar(
  baz(:qux).quux(
corge,
    grault).garply
)
"#,
        r#"foo.bar(
  baz(:qux).quux(
    corge,
    grault).garply
)
"#,
    );
    // corrects only the first argument when a method call not beginning its line is under-indented (spec line 669)
    offense(
        r#"foo(bar, baz.qux(
quux,
corge
))
"#,
        &[(
            2,
            1,
            "Indent the first argument one step more than `baz.qux(`.",
        )],
    );
    correction(
        r#"foo(bar, baz.qux(
quux,
corge
))
"#,
        r#"foo(bar, baz.qux(
           quux,
corge
))
"#,
    );
    // accepts a first argument with consistent style indentation (spec line 688)
    none(
        r#"run :foo, defaults.merge(
  bar: 3)
"#,
    );
    // corrects only the first argument in a method chain, not the entire chain (spec line 695)
    offense(
        r#"run :foo, Diagnostic.where(
              'limit >= 10',
            )
            .ids
"#,
        &[(
            2,
            15,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"run :foo, Diagnostic.where(
              'limit >= 10',
            )
            .ids
"#,
        r#"run :foo, Diagnostic.where(
  'limit >= 10',
            )
            .ids
"#,
    );
    // corrects only the first argument when not inside a parenthesized call (spec line 715)
    offense(
        r#"sequences = CourseSequence
  .includes(
      course_sequence_memberships: {
        course: :course_components
      }
    )
  .joins(:courses)
"#,
        &[(
            3,
            7,
            "Indent the first argument one step more than the start of the previous line.",
        )],
    );
    correction(
        r#"sequences = CourseSequence
  .includes(
      course_sequence_memberships: {
        course: :course_components
      }
    )
  .joins(:courses)
"#,
        r#"sequences = CourseSequence
  .includes(
    course_sequence_memberships: {
      course: :course_components
    }
    )
  .joins(:courses)
"#,
    );
    // accepts properly indented first argument in a method chain (spec line 738)
    none(
        r#"sequences = CourseSequence
  .includes(
    course_sequence_memberships: {
      course: :course_components
    }
  )
  .joins(:courses)
"#,
    );
}

const PREV: &str = "Indent the first argument one step more than the start of the previous line.";

#[test]
fn parser_parents_block_passes_and_operator_forms() {
    // A block pass is the parser's first argument; `super` is checked too.
    offense("foo(\n      &blk)\n", &[(2, 7, PREV)]);
    offense("super(\n      1)\n", &[(2, 7, PREV)]);
    // Only an argument of a parenthesized `send` uses the inner-call base.
    offense(
        "foo(bar(\n  1))\n",
        &[(2, 3, "Indent the first argument one step more than `bar(`.")],
    );
    for (source, line, col) in [
        ("foo(bar(\n        1) { })\n", 2, 9),
        ("a&.foo(bar(\n         1))\n", 2, 10),
        ("foo&.bar(\n      1)\n", 2, 7),
        ("x.+(\n      1)\n", 2, 7),
        ("x.[](\n      1)\n", 2, 7),
        ("foo.(\n      1)\n", 2, 7),
    ] {
        offense(source, &[(line, col, PREV)]);
    }
    for source in [
        "foo(*bar(\n  1))\n",
        "foo(a: bar(\n  1))\n",
        "foo(&bar(\n  1))\n",
        "foo[bar(\n  1)]\n",
        "foo bar(\n  1)\n",
        "x&.+(\n      1)\n",
        "foo.bar = baz(\n  1)\n",
        "def m\n  yield(\n        1)\nend\n",
        "foo(1,\n  2) do\nend\n",
        "x = y +\n      z\n",
        "foo[\n      1]\n",
    ] {
        none(source);
    }
}

#[test]
fn previous_code_lines_comments_and_columns() {
    offense(
        "foo(\n  # c\n\n      1)\n",
        &[(
            4,
            7,
            "Indent the first argument one step more than the start of the previous line (not counting the comment).",
        )],
    );
    // A document comment's first line is skipped, its body is not.
    offense(
        "=begin\nx\n=end\nfoo(\n=begin\ny\n=end\n      1)\n",
        &[(8, 7, PREV)],
    );
    // Display columns: a fullwidth receiver is two cells wide.
    none("puts('Ｒｕｂｙ', f(\n                   a))\n");
    offense(
        "puts('Ｒｕｂｙ', f(\n                 a))\n",
        &[(2, 18, "Indent the first argument one step more than `f(`.")],
    );
    // A tab counts as one column, like upstream.
    offense("foo(\n\t1)\n", &[(2, 2, PREV)]);
    // An inner offense inside an outer one is reported without an edit.
    offense(
        "foo(\n  bar(\n       baz(\n    1)))\n",
        &[
            (3, 8, "Indent the first argument one step more than `bar(`."),
            (4, 5, "Bad indentation of the first argument."),
        ],
    );
    let found = correction_offenses(&Cop, "foo(\n  bar(\n       baz(\n    1)))\n");
    assert!(found[0].correction.is_some() && found[1].correction.is_none());
}

#[test]
fn corrections_shift_arguments_and_chains_with_guards() {
    correction(
        "call(\n  Foo.bar(\n          <<~SQL,\n            body\n          SQL\n          2\n        )\n)\n",
        "call(\n  Foo.bar(\n    <<~SQL,\n            body\n          SQL\n    2\n  )\n)\n",
    );
    correction(
        "run(x, Diagnostic.where(\n              'a',\n            ).ids)\n",
        "run(x, Diagnostic.where(\n  'a',\n).ids)\n",
    );
    correction(
        "expect(foo.bar(\n               1\n             ))\n",
        "expect(foo.bar(\n         1\n             ))\n",
    );
    // A chain line that cannot move by the whole shift holds the edit (the
    // shared seam never shifts part of a subtree); the finding stays.
    let source = "call(\n  a.b\n   .c(\n             1,\n           )\n)\n";
    let found = correction_offenses(&Cop, source);
    assert_eq!(found.len(), 1);
    assert!(found[0].correction.is_none());
    // Closed observers hold the edit; other readers do not.
    for observer in [
        "p __FILE__",
        "p $PROGRAM_NAME",
        "p caller_locations",
        "SCRIPT_LINES__ = {}",
    ] {
        let source = format!("foo(\n      1)\n{observer}\n");
        let found = correction_offenses(&Cop, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
    correction(
        "foo(\n      1)\np STDIN.read\n",
        "foo(\n  1)\np STDIN.read\n",
    );
}
