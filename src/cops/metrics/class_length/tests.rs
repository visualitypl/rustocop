//! Examples ported from RuboCop's
//! `spec/rubocop/cop/metrics/class_length_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! All 35 pinned examples run verbatim at the fixed defaults and match the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! source SHA-256 was checked against it when the rows were ported): the
//! spec sets `Max: 5`, so none of them reaches the fixed `Max: 100`. Each
//! upstream finding is then ported at the fixed threshold with the same
//! shape, position and end line (the three examples the shim could not run
//! are asserted through the end line). The two `--lsp` examples' header-only
//! ranges do not apply: rustocop has no LSP mode and reports the whole node.
//! Spec-custom `CountComments: true` and `CountAsOne: ['array']` examples are
//! not applicable; their ports assert the fixed behaviour.
use super::*;
use crate::pipeline::lint_str_with;

const COP: &str = "Metrics/ClassLength";

/// `(line, col, end line, message)` of every finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, u32, String)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&ClassLength::default()], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Refactor);
            assert!(o.correction.is_none());
            (o.line, o.col, o.end_line, o.message)
        })
        .collect();
    found.sort_unstable();
    found
}

fn msg(length: usize) -> String {
    format!("Class has too many lines. [{length}/100]")
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
        r#"class Test
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
        r#"class Test
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
        37,
        r#"class Test
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
        54,
        r#"class Test
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
end
"#,
    ),
    (
        66,
        r#"class Test
  a = 1
  a = 2
  a = 3
  a = 4
end
"#,
    ),
    (
        77,
        r#"class Test
  a = 1
  a = 2
  a = 3
  a = 4


  a = 7
end
"#,
    ),
    (
        91,
        r#"class Test
end
"#,
    ),
    (
        99,
        r#"class NamespaceClass
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
        125,
        r#"class NamespaceClass
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
        155,
        r#"class Test
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
        172,
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
        193,
        r#"class Test
  # comment
  # comment
  # comment
  # comment
  # comment
  # comment
end
"#,
    ),
    (
        210,
        r#"class Test
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
        224,
        r#"class Test
  # comment
  # comment
  # comment
  # comment
  # comment
  # comment
end
"#,
    ),
    (
        242,
        r#"class Test
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
        258,
        r#"X = Y = Z = do_something
"#,
    ),
    (
        266,
        r#"class << self
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
        280,
        r#"class << self
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
end
"#,
    ),
    (
        294,
        r#"Foo = Class.new do
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
        310,
        r#"Foo = ::Class.new do
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
        326,
        r#"Foo = Bar = Class.new do
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
        342,
        r#"Foo = Class.new do
  self.a = self::B = 1
  b(_1)
  c(_1)
  d(_1)
  e(_1)
  f(_1)
end
"#,
    ),
    (
        358,
        r#"Foo = Struct.new(:foo, :bar) do
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
        372,
        r#"Foo ||= Struct.new(:foo, :bar) do
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
        387,
        r#"Foo, Bar = Struct.new(:foo, :bar) do
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
        404,
        r#"Foo = Class.new do
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
        420,
        r#"Foo = ::Class.new do
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
        436,
        r#"Foo = Struct.new(:foo, :bar) do
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
        450,
        r#"Foo ||= Struct.new(:foo, :bar) do
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
        465,
        r#"Foo, Bar = Struct.new(:foo, :bar) do
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
        483,
        r#"Foo = Class.new do
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
        499,
        r#"Foo = ::Class.new do
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
        515,
        r#"Foo = Struct.new(:foo, :bar) do
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
        529,
        r#"Foo ||= Struct.new(:foo, :bar) do
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
        544,
        r#"Foo, Bar = Struct.new(:foo, :bar) do
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
    assert_eq!(PINNED.len(), 35);
    for (line, source) in PINNED {
        assert_eq!(findings(source), [], "spec line {line}");
    }
}

#[test]
fn classes_at_and_over_the_fixed_max_with_their_end_line() {
    assert_eq!(
        findings(&format!("class Test\n{}end\n", assigns(100, "  "))),
        []
    );
    assert_eq!(
        findings(&format!("class Test\n{}end\n", assigns(101, "  "))),
        [(1, 1, 103, msg(101))]
    );
    // Blank lines are not counted.
    let source = format!("class Test\n{}\n\n  a = 7\nend\n", assigns(99, "  "));
    assert_eq!(findings(&source), []);
    assert_eq!(findings("class Test\nend\n"), []);
}

#[test]
fn upstream_reads_the_line_after_each_body_line_number() {
    // Body numbers 2..=101 read lines 3..=102: the blank first body line is
    // skipped and `end` is counted, as upstream's processed_source[number].
    let source = format!("class Test\n\n{}end\n", assigns(100, "  "));
    assert_eq!(findings(&source), [(1, 1, 103, msg(101))]);
    // A comment right before `end` is the last line read; the first body
    // line is never read.
    let source = format!("class Test\n{}  # done\nend\n", assigns(100, "  "));
    assert_eq!(findings(&source), []);
}

#[test]
fn a_singleton_class_inside_is_counted_and_not_reported_itself() {
    let source = format!(
        "class Test\n  class << self\n{}  end\nend\n",
        assigns(99, "    ")
    );
    assert_eq!(findings(&source), [(1, 1, 103, msg(101))]);
    let source = format!(
        "class Test\n  def self.x\n    class << self\n{}    end\n  end\nend\n",
        assigns(10, "      ")
    );
    assert_eq!(findings(&source), []);
}

#[test]
fn inner_classes_and_modules_are_not_counted() {
    let inner = |kind: &str, name: &str| format!("  {kind} {name}\n{}  end\n", assigns(50, "    "));
    for direct in [100, 101] {
        let source = format!(
            "class NamespaceClass\n{}{}{}end\n",
            inner("class", "TestOne"),
            inner("module", "TestTwo"),
            assigns(direct, "  ")
        );
        let expected = if direct == 101 {
            vec![(1, 1, 207, msg(101))]
        } else {
            vec![]
        };
        assert_eq!(findings(&source), expected, "{direct}");
    }
    // A class whose body is a sole class is a namespace: zero lines.
    let source = format!("class A\n  class B\n{}  end\nend\n", assigns(101, "    "));
    assert_eq!(findings(&source), [(2, 3, 104, msg(101))]);
}

#[test]
fn comments_and_arrays_follow_the_fixed_counting() {
    let source = format!(
        "class Test\n{}end\n",
        body(120, "  ", |_| "# comment".into())
    );
    assert_eq!(findings(&source), []);
    let source = format!(
        "class Test\n  a = [\n{}  ]\nend\n",
        body(100, "    ", |i| format!("{i},"))
    );
    assert_eq!(findings(&source), [(1, 1, 104, msg(102))]);
}

#[test]
fn singleton_classes_outside_classes() {
    assert_eq!(
        findings(&format!("class << self\n{}end\n", assigns(100, "  "))),
        []
    );
    assert_eq!(
        findings(&format!("class << self\n{}end\n", assigns(101, "  "))),
        [(1, 1, 103, msg(101))]
    );
    // Inside a module (the `--lsp` example's shape): the whole node.
    let source = format!(
        "module Test\n  class << self\n{}  end\nend\n",
        assigns(101, "    ")
    );
    assert_eq!(findings(&source), [(2, 3, 104, msg(101))]);
}

#[test]
fn class_constructor_blocks_bound_to_constants() {
    for (header, col) in [
        ("Foo = Class.new do", 7),
        ("Foo = ::Class.new do", 7),
        ("Foo = Bar = Class.new do", 13),
        ("Foo = Struct.new(:foo, :bar) do", 7),
        ("Foo ||= Struct.new(:foo, :bar) do", 9),
        ("Foo &&= Class.new do", 9),
        ("Foo += Class.new do", 8),
        ("A::Foo = Class.new(Base) do", 10),
        ("::Foo ||= Class.new do", 11),
        ("Foo, Bar = Struct.new(:foo, :bar) do", 12),
        ("a, Foo = Class.new do", 10),
        ("Foo = Class.new {", 7),
    ] {
        let close = if header.ends_with('{') { "}" } else { "end" };
        for (param, lines) in [
            ("a = 1", 100),
            ("a(_1)", 101),
            ("a(it)", 101),
            ("a = 1", 101),
        ] {
            let source = format!("{header}\n{}{close}\n", body(lines, "  ", |_| param.into()));
            let expected = if lines == 101 {
                vec![(1, col, 103, msg(101))]
            } else {
                vec![]
            };
            assert_eq!(findings(&source), expected, "{header} {param}");
        }
    }
    // The chained assignment in the body is not a class definition.
    let source = format!(
        "Foo = Class.new do\n  self.a = self::B = 1\n{}end\n",
        body(100, "  ", |_| "b(_1)".into())
    );
    assert_eq!(findings(&source), [(1, 7, 103, msg(101))]);
}

#[test]
fn constructor_lookalikes_are_not_class_definitions() {
    for header in [
        "foo = Class.new do",
        "Foo = Module.new do",
        "Foo = Data.define do",
        "Foo = Class&.new do",
        "Foo = Bar::Class.new do",
        "Foo = Class.new.tap do",
        "*Foo, a = Class.new do",
        "(Foo, a), b = Class.new do",
        "Foo = [Class.new do",
        "Class.new do",
    ] {
        let close = if header.contains('[') { "end]" } else { "end" };
        let source = format!("{header}\n{}{close}\n", assigns(101, "  "));
        assert_eq!(findings(&source), [], "{header}");
    }
    assert_eq!(findings("X = Y = Z = do_something\n"), []);
    assert_eq!(
        findings(&format!(
            "Foo = Class.new do\n{}end\n",
            body(101, "", |_| String::new())
        )),
        []
    );
}
