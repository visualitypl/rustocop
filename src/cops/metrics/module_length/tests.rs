//! Examples ported from RuboCop's
//! `spec/rubocop/cop/metrics/module_length_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! All 21 pinned examples run verbatim at the fixed defaults and match the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! source SHA-256 was checked against it when the rows were ported): the
//! spec sets `Max: 5`, so none of them reaches the fixed `Max: 100`. Each
//! upstream finding is then ported at the fixed threshold with the same
//! shape, position and end line (the two examples the shim could not run are
//! asserted through the end line). The `--lsp` example's header-only range
//! does not apply: rustocop has no LSP mode and reports the whole node.
//! Spec-custom `CountComments: true` and `CountAsOne: ['array']` examples are
//! not applicable; their ports assert the fixed behaviour.
use super::*;
use crate::pipeline::lint_str_with;

const COP: &str = "Metrics/ModuleLength";

/// `(line, col, end line, message)` of every finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, u32, String)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&ModuleLength], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col, o.end_line, o.message)
        })
        .collect();
    found.sort_unstable();
    found
}

fn msg(length: usize) -> String {
    format!("Module has too many lines. [{length}/100]")
}

fn body(count: usize, indent: &str, line: impl Fn(usize) -> String) -> String {
    (1..=count)
        .map(|i| format!("{indent}{}\n", line(i)))
        .collect()
}

fn assigns(count: usize, indent: &str) -> String {
    body(count, indent, |i| format!("a = {i}"))
}

/// `(spec line, source)`; every fixed-default replay in the inventory is offense-free.
const PINNED: &[(u32, &str)] = &[
    (
        7,
        r#"module Test
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        21,
        r#"module Test
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        37,
        r#"module Test
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
end
"#,
    ),
    (
        49,
        r#"module Test
  a = 1
  a = 2
  a = 3
  a = 4
end
"#,
    ),
    (
        60,
        r#"module Test
  a = 1
  a = 2
  a = 3
  a = 4


  a = 7
end
"#,
    ),
    (
        74,
        r#"module Test
end
"#,
    ),
    (
        82,
        r#"module NamespaceModule
  module TestOne
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
  end
  module TestTwo
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
  end
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
end
"#,
    ),
    (
        108,
        r#"module NamespaceModule
  module TestOne
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
  end
  module TestTwo
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
  end
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        138,
        r#"module NamespaceModule
  class TestOne
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
  end
  class TestTwo
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
  end
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
end
"#,
    ),
    (
        164,
        r#"module Test
  class << self
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
    a = 6
  end
end
"#,
    ),
    (
        180,
        r#"module NamespaceModule
  class << self
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
  end
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        201,
        r#"module NamespaceModule
  class TestOne
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
  end
  class TestTwo
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
  end
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        231,
        r#"module Test
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        251,
        r#"module Test
  a = 1
  #a = 2
  a = 3
  #a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        269,
        r#"module Test
  a = 1
  a = [
    2,
    3,
    4,
    5
  ]
end
"#,
    ),
    (
        285,
        r#"Foo = Module.new do
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        301,
        r#"Foo = ::Module.new do
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        318,
        r#"Foo = Module.new do
  a(_1)
  b(_1)
  c(_1)
  d(_1)
  e(_1)
  f(_1)
end
"#,
    ),
    (
        334,
        r#"Foo = ::Module.new do
  a(_1)
  b(_1)
  c(_1)
  d(_1)
  e(_1)
  f(_1)
end
"#,
    ),
    (
        352,
        r#"Foo = Module.new do
  a(it)
  b(it)
  c(it)
  d(it)
  e(it)
  f(it)
end
"#,
    ),
    (
        368,
        r#"Foo = ::Module.new do
  a(it)
  b(it)
  c(it)
  d(it)
  e(it)
  f(it)
end
"#,
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 21);
    for (line, source) in PINNED {
        assert_eq!(findings(source), [], "spec line {line}");
    }
}

#[test]
fn modules_at_and_over_the_fixed_max_with_their_end_line() {
    assert_eq!(
        findings(&format!("module Test\n{}end\n", assigns(100, "  "))),
        []
    );
    assert_eq!(
        findings(&format!("module Test\n{}end\n", assigns(101, "  "))),
        [(1, 1, 103, msg(101))]
    );
    let source = format!("module Test\n{}\n\n  a = 7\nend\n", assigns(99, "  "));
    assert_eq!(findings(&source), []);
    assert_eq!(findings("module Test\nend\n"), []);
    // Upstream reads the line after each body line number.
    let source = format!("module Test\n\n{}end\n", assigns(100, "  "));
    assert_eq!(findings(&source), [(1, 1, 103, msg(101))]);
}

#[test]
fn inner_modules_and_classes_are_not_counted() {
    let inner = |kind: &str, name: &str| format!("  {kind} {name}\n{}  end\n", assigns(50, "    "));
    for kind in ["module", "class"] {
        for direct in [100, 101] {
            let source = format!(
                "module NamespaceModule\n{}{}{}end\n",
                inner(kind, "TestOne"),
                inner(kind, "TestTwo"),
                assigns(direct, "  ")
            );
            let expected = if direct == 101 {
                vec![(1, 1, 207, msg(101))]
            } else {
                vec![]
            };
            assert_eq!(findings(&source), expected, "{kind} {direct}");
        }
    }
    let source = format!("module A\n  module B\n{}  end\nend\n", assigns(101, "    "));
    assert_eq!(findings(&source), [(2, 3, 104, msg(101))]);
}

#[test]
fn singleton_class_lines_are_counted() {
    let source = format!(
        "module Test\n  class << self\n{}  end\nend\n",
        assigns(99, "    ")
    );
    assert_eq!(findings(&source), [(1, 1, 103, msg(101))]);
    let source = format!(
        "module NamespaceModule\n  class << self\n{}  end\n{}end\n",
        assigns(50, "    "),
        assigns(49, "  ")
    );
    assert_eq!(findings(&source), [(1, 1, 103, msg(101))]);
}

#[test]
fn comments_and_arrays_follow_the_fixed_counting() {
    let source = format!(
        "module Test\n{}end\n",
        body(120, "  ", |_| "# comment".into())
    );
    assert_eq!(findings(&source), []);
    let source = format!(
        "module Test\n  a = [\n{}  ]\nend\n",
        body(100, "    ", |i| format!("{i},"))
    );
    assert_eq!(findings(&source), [(1, 1, 104, msg(102))]);
}

#[test]
fn module_new_bound_to_a_constant_is_reported_at_the_name() {
    for header in [
        "Foo = Module.new do",
        "Foo = ::Module.new do",
        "Foo = Module.new() {",
    ] {
        let close = if header.ends_with('{') { "}" } else { "end" };
        for (param, lines) in [
            ("a = 1", 100),
            ("a = 1", 101),
            ("a(_1)", 101),
            ("a(it)", 101),
        ] {
            let source = format!("{header}\n{}{close}\n", body(lines, "  ", |_| param.into()));
            let expected = if lines == 101 {
                vec![(1, 1, 1, msg(101))]
            } else {
                vec![]
            };
            assert_eq!(findings(&source), expected, "{header} {param}");
        }
    }
    let source = format!(
        "class C\n  Foo = Module.new do\n{}  end\nend\n",
        assigns(101, "    ")
    );
    assert_eq!(findings(&source), [(2, 3, 2, msg(101))]);
}

#[test]
fn module_new_lookalikes() {
    for header in [
        "foo = Module.new do",
        "Foo ||= Module.new do",
        "Foo, Bar = Module.new do",
        "A::Foo = Module.new do",
        "::Foo = Module.new do",
        "Foo = Module.new(Base) do",
        "Foo = Module&.new do",
        "Foo = Bar::Module.new do",
        "Foo = Bar = Module.new.tap do",
        "Foo = Class.new do",
        "Module.new do",
    ] {
        let source = format!("{header}\n{}end\n", assigns(101, "  "));
        assert_eq!(findings(&source), [], "{header}");
    }
    // `Foo = Bar = Module.new`: only `Bar` is assigned the block.
    let source = format!("Foo = Bar = Module.new do\n{}end\n", assigns(101, "  "));
    assert_eq!(findings(&source), [(1, 7, 1, msg(101))]);
}
