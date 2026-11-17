//! Ported from RuboCop first_array_element_indentation_spec.rb at 97f30c4
//! (MIT, Copyright (c) 2012–26 Bozhidar Batsov).
use super::FirstArrayElementIndentation as Cop;
use crate::{
    cops::{Cop as _, layout::element_indentation::corrected},
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

/// The examples at the fixed `special_inside_parentheses` style: the operand,
/// setter, assignment and both method-argument contexts. The
/// `ArrayAlignment: with_fixed_indentation`, per-cop width, `consistent` and
/// `align_brackets` contexts are configuration skips.
#[test]
fn pinned_examples() {
    // accepts correctly indented first element (spec line 61)
    none(
        r#"a << [
  1
]
"#,
    );
    // registers an offense and corrects incorrectly indented first element (spec line 69)
    offense(
        r#"a << [
 1
]
"#,
        &[(
            2,
            2,
            "Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is.",
        )],
    );
    correction(
        r#"a << [
 1
]
"#,
        r#"a << [
  1
]
"#,
    );
    // registers an offense and corrects incorrectly indented ] (spec line 84)
    offense(
        r#"a << [
  ]
"#,
        &[(
            2,
            3,
            "Indent the right bracket the same as the start of the line where the left bracket is.",
        )],
    );
    correction(
        r#"a << [
  ]
"#,
        r#"a << [
]
"#,
    );
    // accepts correctly indented first element (spec line 126)
    none(
        r#"config.rack_cache = [
  "rails:/",
  "rails:/",
  false
]
"#,
    );
    // registers an offense and corrects incorrectly indented first element (spec line 136)
    offense(
        r#"config.rack_cache = [
"rails:/",
"rails:/",
false
]
"#,
        &[(
            2,
            1,
            "Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is.",
        )],
    );
    correction(
        r#"config.rack_cache = [
"rails:/",
"rails:/",
false
]
"#,
        r#"config.rack_cache = [
  "rails:/",
"rails:/",
false
]
"#,
    );
    // registers an offense and corrects incorrectly indented first element (spec line 157)
    offense(
        r#"a = [
    1,
  2,
 3
]
"#,
        &[(
            2,
            5,
            "Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is.",
        )],
    );
    correction(
        r#"a = [
    1,
  2,
 3
]
"#,
        r#"a = [
  1,
  2,
 3
]
"#,
    );
    // accepts correctly indented first element (spec line 176)
    none(
        r#"a = [
  1
]
"#,
    );
    // accepts several elements per line (spec line 184)
    none(
        r#"a = [
  1, 2
]
"#,
    );
    // accepts a first element on the same line as the left bracket (spec line 192)
    none(
        r#"a = [1,
     2]
"#,
    );
    // accepts single line array (spec line 199)
    none(
        r#"a = [1, 2]
"#,
    );
    // accepts an empty array (spec line 203)
    none(
        r#"a = []
"#,
    );
    // accepts multi-assignments with brackets (spec line 207)
    none(
        r#"a, b = [b, a]
"#,
    );
    // accepts multi-assignments with no brackets (spec line 211)
    none(
        r#"a, b = b, a
"#,
    );
    // accepts special indentation for first argument (spec line 219)
    none(
        r#"h = [
  1
]
func([
       1
     ])
func(x, [
       1
     ])
h = [1
]
func([1
     ])
func(x, [1
     ])
"#,
    );
    // registers an offense and corrects 'consistent' indentation (spec line 239)
    offense(
        r#"func([
  1
])
"#,
        &[
            (
                2,
                3,
                "Use 2 spaces for indentation in an array, relative to the first position after the preceding left parenthesis.",
            ),
            (
                3,
                1,
                "Indent the right bracket the same as the first position after the preceding left parenthesis.",
            ),
        ],
    );
    correction(
        r#"func([
  1
])
"#,
        r#"func([
       1
     ])
"#,
    );
    // registers an offense and corrects 'consistent' indentation (spec line 256)
    offense(
        r#"receiver&.func([
  1
])
"#,
        &[
            (
                2,
                3,
                "Use 2 spaces for indentation in an array, relative to the first position after the preceding left parenthesis.",
            ),
            (
                3,
                1,
                "Indent the right bracket the same as the first position after the preceding left parenthesis.",
            ),
        ],
    );
    correction(
        r#"receiver&.func([
  1
])
"#,
        r#"receiver&.func([
                 1
               ])
"#,
    );
    // registers an offense and corrects 'align_brackets' indentation (spec line 273)
    offense(
        r#"var = [
        1
      ]
"#,
        &[
            (
                2,
                9,
                "Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is.",
            ),
            (
                3,
                7,
                "Indent the right bracket the same as the start of the line where the left bracket is.",
            ),
        ],
    );
    correction(
        r#"var = [
        1
      ]
"#,
        r#"var = [
  1
]
"#,
    );
    // accepts special indentation for second argument (spec line 289)
    none(
        r#"body.should have_tag("input", [
                       :name])
"#,
    );
    // accepts normal indentation for array within array (spec line 296)
    none(
        r#"puts(
  [
    [1, 2]
  ]
)
"#,
    );
    // registers an offense for incorrectly indented multi-line array that is the value of a single pair hash (spec line 306)
    offense(
        r#"func(x: [
      :a, :b])
"#,
        &[(
            2,
            7,
            "Use 2 spaces for indentation in an array, relative to the first position after the preceding left parenthesis.",
        )],
    );
    correction(
        r#"func(x: [
      :a, :b])
"#,
        r#"func(x: [
       :a, :b])
"#,
    );
    // registers an offense for a multi-line array that is a value of a multi pairs hash  (spec line 319)
    offense(
        r#"func(x: [
  :a,
       :b
],
     y: [
       :c,
       :d
     ])
"#,
        &[
            (
                2,
                3,
                "Use 2 spaces for indentation in an array, relative to the parent hash key.",
            ),
            (
                4,
                1,
                "Indent the right bracket the same as the parent hash key.",
            ),
        ],
    );
    correction(
        r#"func(x: [
  :a,
       :b
],
     y: [
       :c,
       :d
     ])
"#,
        r#"func(x: [
       :a,
       :b
     ],
     y: [
       :c,
       :d
     ])
"#,
    );
    // accepts indent based on the preceding left parenthesis  (spec line 346)
    none(
        r#"func(:x, y: [
       :a,
       :b
     ], z: [
       :c,
       :d
     ])
"#,
    );
    // accepts indent based on the left brace when the outer hash key and  (spec line 359)
    none(
        r#"func(x:
       [
         :a,
         :b
       ],
     y: [
       :a,
       :b
     ])
"#,
    );
    // accepts bracketless array (spec line 492)
    none(
        r#"func 1, 2
"#,
    );
    // accepts single line array with brackets (spec line 496)
    none(
        r#"func x, [1, 2]
"#,
    );
    // accepts a correctly indented multi-line array with brackets (spec line 500)
    none(
        r#"func x, [
  1, 2]
"#,
    );
    // registers an offense and corrects incorrectly indented multi-line array with brackets (spec line 507)
    offense(
        r#"func x, [
       1, 2]
"#,
        &[(
            2,
            8,
            "Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is.",
        )],
    );
    correction(
        r#"func x, [
       1, 2]
"#,
        r#"func x, [
  1, 2]
"#,
    );
    // registers an offense for incorrectly indented multi-line array that is the value of a single pair hash (spec line 520)
    offense(
        r#"func x: [
       :a, :b]
"#,
        &[(
            2,
            8,
            "Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is.",
        )],
    );
    correction(
        r#"func x: [
       :a, :b]
"#,
        r#"func x: [
  :a, :b]
"#,
    );
    // registers an offense for a multi-line array that is a value of a multi pairs hash  (spec line 533)
    offense(
        r#"func x: [
  :a,
       :b
],
     y: [
       :c,
       :d
     ]
"#,
        &[
            (
                2,
                3,
                "Use 2 spaces for indentation in an array, relative to the parent hash key.",
            ),
            (
                4,
                1,
                "Indent the right bracket the same as the parent hash key.",
            ),
        ],
    );
    correction(
        r#"func x: [
  :a,
       :b
],
     y: [
       :c,
       :d
     ]
"#,
        r#"func x: [
       :a,
       :b
     ],
     y: [
       :c,
       :d
     ]
"#,
    );
    // accepts indent based on the start of the line where the left bracket is (spec line 560)
    none(
        r#"func :x, y: [
  :a,
  :b
], z: [
  :c,
  :d
]
"#,
    );
    // accepts indent based on the left bracket when the outer hash key and  (spec line 573)
    none(
        r#"func x:
        [
          :a,
          :b
        ],
      y: [
        :a,
        :b
      ]
"#,
    );
}

const LINE: &str = "Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is.";
const PAREN: &str = "Use 2 spaces for indentation in an array, relative to the first position after the preceding left parenthesis.";
const RLINE: &str =
    "Indent the right bracket the same as the start of the line where the left bracket is.";
const RPAREN: &str =
    "Indent the right bracket the same as the first position after the preceding left parenthesis.";

#[test]
fn parser_send_lineage_selects_the_left_parenthesis() {
    // Blocks, `&.` calls and `super` do not stop upstream's `on_node` walk,
    // so the outer `(` is the base; a `send` (receiver or argument) does.
    for source in [
        "foo([\n      1\n    ])\n",
        "foo(bar { [\n      1\n    ] })\n",
        "foo(a&.bar([\n      1\n    ]))\n",
        "foo(super([\n      1\n    ]))\n",
        "foo(a.b += [\n      1\n    ])\n",
        "foo(**{ a: [\n      1\n    ] })\n",
    ] {
        none(source);
    }
    offense(
        "foo(a.bar([\n      1\n    ]))\n",
        &[(2, 7, PAREN), (3, 5, RPAREN)],
    );
    offense("foo [\n      1\n    ]\n", &[(2, 7, LINE), (3, 5, RLINE)]);
    // A `(` on another line than `[` is not a base.
    offense("foo(\n  [\n  1\n  ]\n)\n", &[(3, 3, LINE)]);
    offense(
        "foo([\n  1\n]).bar([\n  2\n])\n",
        &[(2, 3, PAREN), (3, 1, RPAREN), (4, 3, PAREN), (5, 1, RPAREN)],
    );
}

#[test]
fn pairs_empty_arrays_and_brackets() {
    // A pair is the base only when a later sibling starts on a later line.
    offense(
        "x = { a: [\n        1\n      ], b: 2 }\n",
        &[(2, 9, LINE), (3, 7, RLINE)],
    );
    // A key array's pair starts its value on another line.
    offense(
        "x = { [\n        1\n      ] => 2,\n      c: 3 }\n",
        &[(2, 9, LINE), (3, 7, RLINE)],
    );
    none("y = [\n]\n");
    offense("z = [\n  ]\n", &[(2, 3, RLINE)]);
    offense("foo(x, [\n])\n", &[(2, 1, RPAREN)]);
    // The right bracket is checked even when the first element is aligned,
    // never when the first element shares the `[` line.
    offense("v = [[\n  1\n  ]]\n", &[(3, 3, RLINE)]);
    none("v = [1,\n  2\n    ]\n");
    // Percent literals and implicit arrays.
    offense("x = %w(\n    a b\n  )\n", &[(2, 5, LINE), (3, 3, RLINE)]);
    none("a = 1,\n      2\n");
    none("a, b = [\n  1, 2\n]\n");
}
