//! Ported from RuboCop closing_parenthesis_indentation_spec.rb at 97f30c4
//! (MIT, Copyright (c) 2012–26 Bozhidar Batsov).
use super::ClosingParenthesisIndentation as Cop;
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
fn none(source: &str) {
    assert_eq!(found(source), [], "{source}");
}

/// Every example of the pinned spec except the final context, which sets a
/// per-cop `IndentationWidth: 1` that the fixed width 2 does not offer.
#[test]
fn pinned_examples() {
    // registers an offense for misaligned ) (spec line 6)
    offense(
        r#"some_method(
  a
  )
"#,
        &[(3, 3, "Indent `)` to column 0 (not 2)")],
    );
    correction(
        r#"some_method(
  a
  )
"#,
        r#"some_method(
  a
)
"#,
    );
    // accepts a correctly aligned ) (spec line 21)
    none(
        r#"some_method(
  a
)
"#,
    );
    // registers an offense for misaligned ) (spec line 31)
    offense(
        r#"some_method(a
)
"#,
        &[(2, 1, "Align `)` with `(`.")],
    );
    correction(
        r#"some_method(a
)
"#,
        r#"some_method(a
           )
"#,
    );
    // accepts a correctly aligned ) (spec line 44)
    none(
        r#"some_method(a
           )
"#,
    );
    // does not register an offense when using keyword arguments (spec line 51)
    none(
        r#"some_method(x: 1,
  y: 2,
  z: 3
)
"#,
    );
    // does not register an offense when using keyword splat arguments (spec line 60)
    none(
        r#"some_method(x,
  **options
)
"#,
    );
    // accepts a correctly indented ) (spec line 68)
    none(
        r#"some_method(a,
  x: 1,
  y: 2
)
b =
  some_method(a,
             )
"#,
    );
    // accepts a correctly indented ) inside a block (spec line 80)
    none(
        r#"block_adds_extra_indentation do
  some_method(a,
    x: 1,
    y: 2
  )
  b =
    some_method(a,
               )
end
"#,
    );
    // registers an offense and corrects misindented ) when ) is aligned with the params (spec line 94)
    offense(
        r#"some_method(a,
  x: 1,
  y: 2
            )
b =
  some_method(a,
              )
"#,
        &[
            (4, 13, "Indent `)` to column 0 (not 12)"),
            (7, 15, "Align `)` with `(`."),
        ],
    );
    correction(
        r#"some_method(a,
  x: 1,
  y: 2
            )
b =
  some_method(a,
              )
"#,
        r#"some_method(a,
  x: 1,
  y: 2
)
b =
  some_method(a,
             )
"#,
    );
    // does not register offense for aligned parens when first parameter is a hash (spec line 118)
    none(
        r#"some_method({ foo: 1, bar: 2 },
  x: 1,
  y: 2
)
"#,
    );
    // accepts empty () (spec line 129)
    none(
        r#"some_method()
"#,
    );
    // can handle indentation up against the left edge (spec line 133)
    none(
        r#"some_method(
)
"#,
    );
    // accepts a correctly aligned ) against ( (spec line 140)
    none(
        r#"some_method(
           )
"#,
    );
    // accepts ) on the same level as ( with args on same line (spec line 149)
    none(
        r#"where(
  "multiline
   condition", second_arg
)
"#,
    );
    // accepts ) on the same level as ( with second arg on new line (spec line 158)
    none(
        r#"where(
  "multiline
   condition",
  second_arg
)
"#,
    );
    // registers an offense for misaligned ) (spec line 172)
    offense(
        r#"foo = some_method(
                   a
  )
"#,
        &[(3, 3, "Align `)` with `(`.")],
    );
    correction(
        r#"foo = some_method(
                   a
  )
"#,
        r#"foo = some_method(
                   a
                 )
"#,
    );
    // accepts a correctly aligned ) (spec line 187)
    none(
        r#"foo = some_method(
                   a
                 )
"#,
    );
    // registers an offense for misaligned ) (spec line 197)
    offense(
        r#"foo = some_method(a
)
some_method(a,
            x: 1,
            y: 2
            )
"#,
        &[
            (2, 1, "Align `)` with `(`."),
            (6, 13, "Align `)` with `(`."),
        ],
    );
    correction(
        r#"foo = some_method(a
)
some_method(a,
            x: 1,
            y: 2
            )
"#,
        r#"foo = some_method(a
                 )
some_method(a,
            x: 1,
            y: 2
           )
"#,
    );
    // can handle inner method calls (spec line 219)
    none(
        r#"expect(response).to contain_exactly(
                      { a: 1, b: 'x' },
                      { a: 2, b: 'y' }
                    )
"#,
    );
    // can handle individual arguments that are broken over lines (spec line 228);
    // the spec's interpolating heredoc turns `"\n"` into a real newline.
    for escape in ["\n", "\\n"] {
        none(&format!(
            "corrector.insert_before(\n  range,\n  \"{escape}\" + ' ' * (node.loc.keyword.column +\n                indent_steps * configured_width)\n)\n"
        ));
    }
    // can handle indentation up against the left edge (spec line 238)
    none(
        r#"foo(
a: b
)
"#,
    );
    // can handle hash arguments that are not broken over lines (spec line 246)
    none(
        r#"corrector.insert_before(
                         range,
                         arg_1: 'foo', arg_2: 'bar'
                       )
"#,
    );
    // accepts a correctly aligned ) (spec line 255)
    none(
        r#"foo = some_method(a
                 )
"#,
    );
    // accepts a correctly indented ) (spec line 262)
    none(
        r#"foo = some_method(a,
                  x: 1,
                  y: 2
                 )
b =
  some_method(a,
             )
"#,
    );
    // accepts empty () (spec line 276)
    none(
        r#"foo = some_method()
"#,
    );
    // accepts a correctly aligned ) against ( (spec line 280)
    none(
        r#"foo = some_method(
                 )
"#,
    );
    // can handle indentation up against the left edge (spec line 287)
    none(
        r#"foo = some_method(
)
"#,
    );
    // can handle indentation up against the method (spec line 294)
    none(
        r#"foo = some_method(
      )
"#,
    );
    // registers an offense for misaligned ) (spec line 301)
    offense(
        r#"foo = some_method(
  )
"#,
        &[(2, 3, "Indent `)` to column 0 (not 2)")],
    );
    correction(
        r#"foo = some_method(
  )
"#,
        r#"foo = some_method(
)
"#,
    );
    // can handle multiple chains with differing breaks (spec line 317)
    none(
        r#"good = foo.methA(:arg1, :arg2, options: :hash)
         .methB(
           :arg1,
           :arg2,
         )
         .methC

good = foo.methA(
           :arg1,
           :arg2,
           options: :hash,
         )
         .methB(
           :arg1,
           :arg2,
         )
         .methC
"#,
    );
    // registers an offense and corrects method chains (spec line 339)
    offense(
        r#"good = foo.methA(:arg1, :arg2, options: :hash)
         .methB(
           :arg1,
           :arg2,
     )
         .methC

good = foo.methA(
           :arg1,
           :arg2,
           options: :hash,
    )
         .methB(
           :arg1,
           :arg2,
       )
         .methC
"#,
        &[
            (5, 6, "Indent `)` to column 9 (not 5)"),
            (12, 5, "Indent `)` to column 9 (not 4)"),
            (16, 8, "Indent `)` to column 9 (not 7)"),
        ],
    );
    correction(
        r#"good = foo.methA(:arg1, :arg2, options: :hash)
         .methB(
           :arg1,
           :arg2,
     )
         .methC

good = foo.methA(
           :arg1,
           :arg2,
           options: :hash,
    )
         .methB(
           :arg1,
           :arg2,
       )
         .methC
"#,
        r#"good = foo.methA(:arg1, :arg2, options: :hash)
         .methB(
           :arg1,
           :arg2,
         )
         .methC

good = foo.methA(
           :arg1,
           :arg2,
           options: :hash,
         )
         .methB(
           :arg1,
           :arg2,
         )
         .methC
"#,
    );
    // registers an offense and corrects misaligned ) (spec line 385)
    offense(
        r#"receiver&.some_method(
  a
  )
"#,
        &[(3, 3, "Indent `)` to column 0 (not 2)")],
    );
    correction(
        r#"receiver&.some_method(
  a
  )
"#,
        r#"receiver&.some_method(
  a
)
"#,
    );
    // registers an offense for misaligned ) (spec line 404)
    offense(
        r#"def some_method(
  a
  )
end
"#,
        &[(3, 3, "Indent `)` to column 0 (not 2)")],
    );
    correction(
        r#"def some_method(
  a
  )
end
"#,
        r#"def some_method(
  a
)
end
"#,
    );
    // accepts a correctly aligned ) (spec line 421)
    none(
        r#"def some_method(
  a
)
end
"#,
    );
    // registers an offense for misaligned ) (spec line 432)
    offense(
        r#"def some_method(a
)
end
"#,
        &[(2, 1, "Align `)` with `(`.")],
    );
    correction(
        r#"def some_method(a
)
end
"#,
        r#"def some_method(a
               )
end
"#,
    );
    // accepts a correctly aligned ) (spec line 447)
    none(
        r#"def some_method(a
               )
end
"#,
    );
    // accepts empty () (spec line 455)
    none(
        r#"def some_method()
end
"#,
    );
    // registers an offense for misaligned ) (spec line 466)
    offense(
        r#"w = x * (
  y + z
  )
"#,
        &[(3, 3, "Indent `)` to column 0 (not 2)")],
    );
    correction(
        r#"w = x * (
  y + z
  )
"#,
        r#"w = x * (
  y + z
)
"#,
    );
    // accepts a correctly aligned ) (spec line 481)
    none(
        r#"w = x * (
  y + z
)
"#,
    );
    // registers an offense for misaligned ) (spec line 491)
    offense(
        r#"w = x * (y + z
)
"#,
        &[(2, 1, "Align `)` with `(`.")],
    );
    correction(
        r#"w = x * (y + z
)
"#,
        r#"w = x * (y + z
        )
"#,
    );
    // accepts a correctly aligned ) (spec line 504)
    none(
        r#"w = x * (y + z
        )
"#,
    );
    // accepts ) that does not begin its line (spec line 511)
    none(
        r#"w = x * (y + z +
        a)
"#,
    );
    // accepts begin nodes that are not grouped expressions (spec line 520)
    none(
        r#"def a
  x
  y
end
"#,
    );
}

#[test]
fn parser_node_kinds_and_lookalikes() {
    // `not(...)`, `^(...)`, `foo.()`, block passes and keyword splats.
    offense(
        "not(\n  x\n  )\n",
        &[(3, 3, "Indent `)` to column 0 (not 2)")],
    );
    none("not(\n      x\n)\n");
    offense(
        "case 1\nin ^(\n  2\n  )\nend\n",
        &[(4, 3, "Indent `)` to column 0 (not 2)")],
    );
    offense(
        "foo.(\n  1\n  )\n",
        &[(3, 3, "Indent `)` to column 0 (not 2)")],
    );
    offense("foo(a,\n    &blk\n)\n", &[(3, 1, "Align `)` with `(`.")]);
    offense(
        "foo(\n  &blk\n  )\n",
        &[(3, 3, "Indent `)` to column 0 (not 2)")],
    );
    offense(
        "def g(*a, **b\n     )\nend\n",
        &[(2, 6, "Indent `)` to column 0 (not 5)")],
    );
    offense(
        "y = ({a: 1,\n      b: 2}\n)\n",
        &[(3, 1, "Align `)` with `(`.")],
    );
    offense(
        "def fw(...)\n  bar(\n    ...\n    )\nend\n",
        &[(4, 5, "Indent `)` to column 2 (not 4)")],
    );
    offense(
        "foo.bar(1, 2\n  ).baz\n",
        &[(2, 3, "Indent `)` to column 0 (not 2)")],
    );
    offense(
        "z = foo(\n        )\n",
        &[(2, 9, "Indent `)` to column 0 (not 8)")],
    );
    // The call's own start is the third no-element candidate.
    none("w = foo.bar(\n    )\n");
    offense(
        "foo(**opts,\n    a: 1\n)\n",
        &[(3, 1, "Align `)` with `(`.")],
    );
    // Brackets, `super`, `yield`, lambdas, blocks and multiple targets are
    // not parser `send`/`args`/`begin` parentheses.
    for source in [
        "foo[\n  1\n  ]\n",
        "super(\n  1\n  )\n",
        "def f\n  yield(\n    1\n    )\nend\n",
        "->(\n  a\n  ) {}\n",
        "foo(a) do\n  end\n",
        "(a,\n  b) = 1, 2\n",
        "def f((a,\n  b))\nend\n",
        "defined?(\n  a\n  )\n",
        "foo(1,\n\t2\n)\n",
        "qux(1\n   ) { }\n",
        "def self.h(\n)\nend\n",
    ] {
        none(source);
    }
}

#[test]
fn character_columns_and_interpolation() {
    // Columns count characters, not bytes or display cells.
    offense(
        "\"é\" + foo(é,\n           b\n          )\n",
        &[(3, 11, "Indent `)` to column 0 (not 10)")],
    );
    correction(
        "\"é\" + foo(é,\n           b\n          )\n",
        "\"é\" + foo(é,\n           b\n)\n",
    );
    // An interpolation's `}` is reported as upstream does, never shifted.
    for source in [
        "x = \"#{\n  a\n    }\"\n",
        "x = :\"#{\n  a\n    }\"\n",
        "x = /#{\n  a\n    }/\n",
        "x = <<~A\n  #{\n    a\n      }\nA\n",
    ] {
        assert_eq!(found(source).len(), 1, "{source}");
        let fixes = correction_offenses(&Cop, source);
        assert!(fixes.iter().all(|o| o.correction.is_none()), "{source}");
    }
}

#[test]
fn corrections_are_guarded() {
    correction(
        "w = foo.bar(\n  )\nputs(<<~A, 1\n  body\nA\n  )\n",
        "w = foo.bar(\n)\nputs(<<~A, 1\n  body\nA\n)\n",
    );
    // Tab indentation is never shortened by removing bytes; positive
    // shifts insert spaces after it, in characters as upstream does.
    let source = "if x\n\tfoo(\n\t\t1\n\t\t)\nend\n";
    assert_eq!(found(source).len(), 1);
    assert!(correction_offenses(&Cop, source)[0].correction.is_none());
    correction(
        "if x\n\tfoo(1,\n\t2\n)\nend\n",
        "if x\n\tfoo(1,\n\t2\n )\nend\n",
    );
    // Closed observers hold the edit and keep the finding.
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p caller",
        "p DATA\n__END__\nx",
    ] {
        let source = format!("foo(\n  a\n  )\n{observer}\n");
        let fixes = correction_offenses(&Cop, &source);
        assert_eq!(fixes.len(), 1, "{source}");
        assert!(fixes[0].correction.is_none(), "{source}");
    }
    correction(
        "foo(\n  a\n  )\np File.read('x.rb')\n",
        "foo(\n  a\n)\np File.read('x.rb')\n",
    );
}
