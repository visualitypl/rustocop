//! Ported from RuboCop first_hash_element_indentation_spec.rb at 97f30c4
//! (MIT, Copyright (c) 2012–26 Bozhidar Batsov).
use super::FirstHashElementIndentation as Cop;
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

/// The examples at the fixed `special_inside_parentheses` style with
/// `HashAlignment`'s fixed `key` styles: the shared `right brace` example
/// and the operand, setter, assignment and both method-argument contexts.
/// The `separator` HashAlignment, per-cop width, `consistent` and
/// `align_braces` contexts are configuration skips.
#[test]
fn pinned_examples() {
    // registers an offense and corrects incorrectly indented } (spec line 27)
    offense(
        r#"a << {
  }
"#,
        &[(
            2,
            3,
            "Indent the right brace the same as the start of the line where the left brace is.",
        )],
    );
    correction(
        r#"a << {
  }
"#,
        r#"a << {
}
"#,
    );
    // accepts correctly indented first pair (spec line 163)
    none(
        r#"a << {
  a: 1
}
"#,
    );
    // registers an offense and corrects incorrectly indented first pair (spec line 171)
    offense(
        r#"a << {
 a: 1
}
"#,
        &[(
            2,
            2,
            "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
        )],
    );
    correction(
        r#"a << {
 a: 1
}
"#,
        r#"a << {
  a: 1
}
"#,
    );
    // accepts correctly indented first pair (spec line 190)
    none(
        r#"config.rack_cache = {
  :metastore => "rails:/",
  :entitystore => "rails:/",
   :verbose => false
}
"#,
    );
    // registers an offense and corrects incorrectly indented first pair (spec line 200)
    offense(
        r#"config.rack_cache = {
:metastore => "rails:/",
:entitystore => "rails:/",
:verbose => false
}
"#,
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
        )],
    );
    correction(
        r#"config.rack_cache = {
:metastore => "rails:/",
:entitystore => "rails:/",
:verbose => false
}
"#,
        r#"config.rack_cache = {
  :metastore => "rails:/",
:entitystore => "rails:/",
:verbose => false
}
"#,
    );
    // registers an offense and corrects incorrectly indented first pair (spec line 221)
    offense(
        r#"a = {
    a: 1,
  b: 2,
 c: 3
}
"#,
        &[(
            2,
            5,
            "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
        )],
    );
    correction(
        r#"a = {
    a: 1,
  b: 2,
 c: 3
}
"#,
        r#"a = {
  a: 1,
  b: 2,
 c: 3
}
"#,
    );
    // does not register an offense when the first pair follows a multiline double splat in the middle of a line (spec line 240)
    none(
        r#"a = { **foo.bar do
  baz
end, 'a' => 1 }
"#,
    );
    // registers an offense and corrects the first pair on the line after a double splat on the same line as the left brace (spec line 248)
    offense(
        r#"a = { **foo,
          a: 1 }
"#,
        &[(
            2,
            11,
            "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
        )],
    );
    correction(
        r#"a = { **foo,
          a: 1 }
"#,
        r#"a = { **foo,
  a: 1 }
"#,
    );
    // does not register an offense for the first pair when it follows a double splat on the same line (spec line 261)
    none(
        r#"a = {
  **foo,   a: 1
}
"#,
    );
    // registers an offense and corrects only the key line when value starts on next line (spec line 269)
    offense(
        r#"a = {
    a:
  {
    b: 1
  }
}
"#,
        &[(
            2,
            5,
            "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
        )],
    );
    correction(
        r#"a = {
    a:
  {
    b: 1
  }
}
"#,
        r#"a = {
  a:
  {
    b: 1
  }
}
"#,
    );
    // registers an offense and corrects key and value lines when value starts on the same line (spec line 290)
    offense(
        r#"a = {
    a: {
      b: 1
    }
}
"#,
        &[(
            2,
            5,
            "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
        )],
    );
    correction(
        r#"a = {
    a: {
      b: 1
    }
}
"#,
        r#"a = {
  a: {
    b: 1
  }
}
"#,
    );
    // registers an offense and corrects value lines when value starts on the same line (spec line 309)
    offense(
        r#"a = {
    a: [
      1,
      2
    ]
}
"#,
        &[(
            2,
            5,
            "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
        )],
    );
    correction(
        r#"a = {
    a: [
      1,
      2
    ]
}
"#,
        r#"a = {
  a: [
    1,
    2
  ]
}
"#,
    );
    // accepts correctly indented first pair (spec line 330)
    none(
        r#"a = {
  a: 1
}
"#,
    );
    // accepts several pairs per line (spec line 338)
    none(
        r#"a = {
  a: 1, b: 2
}
"#,
    );
    // accepts a first pair on the same line as the left brace (spec line 346)
    none(
        r#"a = { "a" => 1,
      "b" => 2 }
"#,
    );
    // accepts single line hash (spec line 353)
    none(
        r#"a = { a: 1, b: 2 }
"#,
    );
    // accepts an empty hash (spec line 357)
    none(
        r#"a = {}
"#,
    );
    // accepts special indentation for first argument (spec line 396)
    none(
        r#"h = {
  a: 1
}
func({
       a: 1
     })
func(x, {
       a: 1
     })
h = { a: 1
}
func({ a: 1
     })
func(x, { a: 1
     })
"#,
    );
    // registers an offense and corrects 'consistent' indentation (spec line 416)
    offense(
        r#"func({
  a: 1
})
"#,
        &[
            (
                2,
                3,
                "Use 2 spaces for indentation in a hash, relative to the first position after the preceding left parenthesis.",
            ),
            (
                3,
                1,
                "Indent the right brace the same as the first position after the preceding left parenthesis.",
            ),
        ],
    );
    correction(
        r#"func({
  a: 1
})
"#,
        r#"func({
       a: 1
     })
"#,
    );
    // registers an offense and corrects 'consistent' indentation (spec line 433)
    offense(
        r#"receiver&.func({
  a: 1
})
"#,
        &[
            (
                2,
                3,
                "Use 2 spaces for indentation in a hash, relative to the first position after the preceding left parenthesis.",
            ),
            (
                3,
                1,
                "Indent the right brace the same as the first position after the preceding left parenthesis.",
            ),
        ],
    );
    correction(
        r#"receiver&.func({
  a: 1
})
"#,
        r#"receiver&.func({
                 a: 1
               })
"#,
    );
    // registers an offense and corrects 'align_braces' indentation (spec line 450)
    offense(
        r#"var = {
        a: 1
      }
"#,
        &[
            (
                2,
                9,
                "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
            ),
            (
                3,
                7,
                "Indent the right brace the same as the start of the line where the left brace is.",
            ),
        ],
    );
    correction(
        r#"var = {
        a: 1
      }
"#,
        r#"var = {
  a: 1
}
"#,
    );
    // accepts special indentation for second argument (spec line 466)
    none(
        r#"body.should have_tag("input", :attributes => {
                       :name => /q\[(id_eq)\]/ })
"#,
    );
    // accepts normal indentation for hash within hash (spec line 473)
    none(
        r#"scope = scope.where(
  klass.table_name => {
    reflection.type => model.base_class.sti_name
  }
)
"#,
    );
    // registers an offense for incorrectly indented hash that is the value of a single pair hash (spec line 483)
    offense(
        r#"func(x: {
      a: 1, b: 2 })
"#,
        &[(
            2,
            7,
            "Use 2 spaces for indentation in a hash, relative to the first position after the preceding left parenthesis.",
        )],
    );
    correction(
        r#"func(x: {
      a: 1, b: 2 })
"#,
        r#"func(x: {
       a: 1, b: 2 })
"#,
    );
    // registers an offense and corrects only the key line when value starts on next line in a method call (spec line 496)
    offense(
        r#"func({
         a:
       {
         b: 1
       }
     })
"#,
        &[(
            2,
            10,
            "Use 2 spaces for indentation in a hash, relative to the first position after the preceding left parenthesis.",
        )],
    );
    correction(
        r#"func({
         a:
       {
         b: 1
       }
     })
"#,
        r#"func({
       a:
       {
         b: 1
       }
     })
"#,
    );
    // registers an offense for a hash that is a value of a multi pairs hash (spec line 517)
    offense(
        r#"func(x: {
  a: 1,
       b: 2
},
     y: {
       c: 1,
       d: 2
     })
"#,
        &[
            (
                2,
                3,
                "Use 2 spaces for indentation in a hash, relative to the parent hash key.",
            ),
            (
                4,
                1,
                "Indent the right brace the same as the parent hash key.",
            ),
        ],
    );
    correction(
        r#"func(x: {
  a: 1,
       b: 2
},
     y: {
       c: 1,
       d: 2
     })
"#,
        r#"func(x: {
       a: 1,
       b: 2
     },
     y: {
       c: 1,
       d: 2
     })
"#,
    );
    // accepts indent based on the preceding left parenthesis (spec line 544)
    none(
        r#"func(:x, y: {
       a: 1,
       b: 2
     }, z: {
       c: 1,
       d: 2
     })
"#,
    );
    // accepts indent based on the left brace when the outer hash key and  (spec line 557)
    none(
        r#"func(x:
       {
         a: 1,
         b: 2
       },
     y: {
       c: 1,
       d: 2
     })
"#,
    );
    // accepts braceless hash (spec line 711)
    none(
        r#"func a: 1, b: 2
"#,
    );
    // accepts single line hash with braces (spec line 715)
    none(
        r#"func x, { a: 1, b: 2 }
"#,
    );
    // accepts a correctly indented multi-line hash with braces (spec line 719)
    none(
        r#"func x, {
  a: 1, b: 2 }
"#,
    );
    // registers an offense for incorrectly indented multi-line hash with braces (spec line 726)
    offense(
        r#"func x, {
       a: 1, b: 2 }
"#,
        &[(
            2,
            8,
            "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
        )],
    );
    correction(
        r#"func x, {
       a: 1, b: 2 }
"#,
        r#"func x, {
  a: 1, b: 2 }
"#,
    );
    // registers an offense for the first inner hash member not based on the start of line  (spec line 739)
    offense(
        r#"func x: :foo, y: {
                a: 1, b: 2 }
"#,
        &[(
            2,
            17,
            "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.",
        )],
    );
    correction(
        r#"func x: :foo, y: {
                a: 1, b: 2 }
"#,
        r#"func x: :foo, y: {
  a: 1, b: 2 }
"#,
    );
    // registers an offense for a hash that is a value of a multi pairs hash (spec line 753)
    offense(
        r#"func x: {
  a: 1,
       b: 2
},
     y: {
       c: 1,
       d: 2
     }
"#,
        &[
            (
                2,
                3,
                "Use 2 spaces for indentation in a hash, relative to the parent hash key.",
            ),
            (
                4,
                1,
                "Indent the right brace the same as the parent hash key.",
            ),
        ],
    );
    correction(
        r#"func x: {
  a: 1,
       b: 2
},
     y: {
       c: 1,
       d: 2
     }
"#,
        r#"func x: {
       a: 1,
       b: 2
     },
     y: {
       c: 1,
       d: 2
     }
"#,
    );
    // accepts indent based on the start of the line where the left brace is (spec line 780)
    none(
        r#"func :x, y: {
  a: 1,
  b: 2
}, z: {
  c: 1,
  d: 2
}
"#,
    );
    // accepts indent based on the left brace when the outer hash key and  (spec line 793)
    none(
        r#"{
  x:
    {
      a: 1,
      b: 2
    },
  y: {
    c: 1,
    d: 2
  }
}
"#,
    );
}

const LINE: &str = "Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.";
const PAREN: &str = "Use 2 spaces for indentation in a hash, relative to the first position after the preceding left parenthesis.";
const KEY: &str = "Use 2 spaces for indentation in a hash, relative to the parent hash key.";
const RLINE: &str =
    "Indent the right brace the same as the start of the line where the left brace is.";
const RPAREN: &str =
    "Indent the right brace the same as the first position after the preceding left parenthesis.";

#[test]
fn first_pairs_splats_and_bases() {
    offense("x = {\n    a: 1\n  }\n", &[(2, 5, LINE), (3, 3, RLINE)]);
    // Keyword splats are not pairs; a first pair that does not begin its
    // line is not checked, but the brace still is.
    offense("y = { **h,\n      a: 1 }\n", &[(2, 7, LINE)]);
    offense("z = { **foo(\n  1), a: 1,\n  }\n", &[(3, 3, RLINE)]);
    offense("v = {\n  }\n", &[(2, 3, RLINE)]);
    none("w = {\n}\n");
    offense("foo(x, {\n})\n", &[(2, 1, RPAREN)]);
    // The parent key is a base only with a later sibling on a later line.
    none("foo(a: {\n      b: 1\n    }, c: 2)\n");
    offense(
        "foo(a: {\n  b: 1\n}, c: 2)\n",
        &[(2, 3, PAREN), (3, 1, RPAREN)],
    );
    offense(
        "foo(a: {\n  b: 1\n},\n    c: 2)\n",
        &[
            (2, 3, KEY),
            (
                3,
                1,
                "Indent the right brace the same as the parent hash key.",
            ),
        ],
    );
    // Blocks and `&.` calls do not hide the outer parenthesis; a `send` does.
    none("foo(bar { {\n      a: 1\n    } })\n");
    none("foo(a&.bar({\n      a: 1\n    }))\n");
    offense(
        "foo(a.bar({\n      a: 1\n    }))\n",
        &[(2, 7, PAREN), (3, 5, RPAREN)],
    );
    // Braceless hashes and hash patterns are not hash literals.
    none("foo(a: 1,\n        b: 2)\n");
    none("case x\nin {\n      a: 1\n    }\nend\n");
}
