//! Examples ported from RuboCop's
//! `spec/rubocop/cop/metrics/block_length_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! All 38 pinned examples (the shared `allow an offense on an allowed
//! method` pair twice, for `foo` and `Gem::Specification.new`) run verbatim
//! at the fixed defaults and match the fixed replay recorded in
//! the pinned RuboCop reference run (each source SHA-256 was checked
//! against it when the rows were ported): the spec sets `Max: 2`, so none
//! of them reaches the fixed `Max: 25`. Each upstream finding is then ported
//! at the fixed threshold with the same shape, position and end line (the
//! four "correct beginning and end lines" examples the shim could not run
//! are asserted through the end line). The `--lsp` example's header-only
//! range does not apply: rustocop has no LSP mode and reports the whole
//! block. Spec-custom `CountComments`, `CountAsOne`, `AllowedMethods`,
//! `AllowedPatterns`, `IgnoredMethods` and `ExcludedMethods` settings are not
//! applicable; their ports assert the fixed behaviour (only `refine` is
//! allowed). The invalid-`CountAsOne` example raises only for its custom
//! setting and is offense-free at the fixed defaults.
use super::*;
use crate::pipeline::lint_str_with;

const COP: &str = "Metrics/BlockLength";

/// `(line, col, end line, message)` of every finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, u32, String)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&BlockLength::default()], source)
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
    format!("Block has too many lines. [{length}/25]")
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
        32,
        r#"something do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        43,
        r#"something do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        56,
        r#"something do
  a = 1
  a = 2
end
"#,
    ),
    (
        65,
        r#"something do
  a = 1


  a = 4
end
"#,
    ),
    (
        187,
        r#"[
  :a,
  :b,
  :c,
].each do
  a = 1
  a = 2
end
"#,
    ),
    (
        200,
        r#"something do
end
"#,
    ),
    (
        207,
        r#"something {
  a = 1
  a = 2
  a = 3
}
"#,
    ),
    (
        218,
        r#"something do
  something do
    a = 2
    a = 3
    a = 4
    a = 5
  end
end
"#,
    ),
    (
        233,
        r#"something do
  a = 1
  #a = 2
  #a = 3
  a = 4
end
"#,
    ),
    (
        90,
        r#"something do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        96,
        r#"something do
  a = _1
  a = _2
  a = _3
end
"#,
    ),
    (
        107,
        r#"something do
  a = _1
  a = _2
  a = _3
end
"#,
    ),
    (
        120,
        r#"something do
  a = _1
  a = _2
end
"#,
    ),
    (
        129,
        r#"something do
  a = _1


  a = _2
end
"#,
    ),
    (
        142,
        r#"something do
  a = it
  a = it
  a = it
end
"#,
    ),
    (
        153,
        r#"something do
  a = it
  a = it
  a = it
end
"#,
    ),
    (
        166,
        r#"something do
  a = it
  a = it
end
"#,
    ),
    (
        175,
        r#"something do
  a = it


  a = it
end
"#,
    ),
    (
        245,
        r#"Class.new do
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
        260,
        r#"Module.new do
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
        275,
        r##"Person = Struct.new(:first_name, :last_name) do
  def full_name
    "#{first_name} #{last_name}"
  end

  def foo
    a = 1
    a = 2
    a = 3
    a = 4
    a = 5
    a = 6
  end
end
"##,
    ),
    (
        296,
        r#"something do
  a = _1
  a = _2
  a = _3
end
"#,
    ),
    (
        313,
        r#"something do
  a = 1
  #a = 2
  a = 3
end
"#,
    ),
    (
        328,
        r#"something do
  a = 1
  a = [
    2,
    3
  ]
end
"#,
    ),
    (
        10,
        r#"something do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        21,
        r#"foo do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        10,
        r#"something do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        21,
        r#"Gem::Specification.new do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        351,
        r#"Foo::
  Bar.baz do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        366,
        r#"Foo::Bar.baz do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        381,
        r#"Foo::Bar.baz do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        392,
        r#"Foo::Bar.bar do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        409,
        r#"Foo::Bar.baz do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        420,
        r#"Foo::Bar.bar do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        436,
        r#"Foo::Bar.baz do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        447,
        r#"Foo::Bar.bar do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        464,
        r#"Foo::Bar.baz do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
    (
        475,
        r#"Foo::Bar.bar do
  a = 1
  a = 2
  a = 3
end
"#,
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 38);
    for (line, source) in PINNED {
        assert_eq!(findings(source), [], "spec line {line}");
    }
}

#[test]
fn blocks_at_and_over_the_fixed_max_with_their_end_line() {
    for param in ["a = 1", "a = _1", "a = it"] {
        let at = format!(
            "something do\n{}end\n",
            body(25, "  ", |_| param.to_string())
        );
        assert_eq!(findings(&at), [], "{param}");
        let over = format!(
            "something do\n{}end\n",
            body(26, "  ", |_| param.to_string())
        );
        assert_eq!(findings(&over), [(1, 1, 28, msg(26))], "{param}");
    }
    let braces = format!("something {{\n{}}}\n", assigns(26, "  "));
    assert_eq!(findings(&braces), [(1, 1, 28, msg(26))]);
}

#[test]
fn blank_comment_and_receiver_lines_are_not_counted() {
    let source = format!(
        "something do\n{}\n\n  # a = 0\n  #a = 0\nend\n",
        assigns(25, "  ")
    );
    assert_eq!(findings(&source), []);
    let source = format!(
        "[\n  :a,\n  :b,\n  :c,\n].each do\n{}end\n",
        assigns(25, "  ")
    );
    assert_eq!(findings(&source), []);
    assert_eq!(findings("something do\nend\n"), []);
}

#[test]
fn nested_blocks_are_each_measured() {
    let source = format!(
        "something do\n  something do\n{}  end\nend\n",
        assigns(26, "    ")
    );
    assert_eq!(
        findings(&source),
        [(1, 1, 30, msg(28)), (2, 3, 29, msg(26))]
    );
}

#[test]
fn class_constructors_are_skipped() {
    for header in [
        "Class.new do",
        "::Class.new(Base) do",
        "Module.new do",
        "Struct.new(:a, :b) do",
        "Person = Struct.new(:first_name, :last_name) do",
        "Data.define(:a) do",
        "::Data.define do",
    ] {
        let source = format!("{header}\n{}end\n", assigns(30, "  "));
        assert_eq!(findings(&source), [], "{header}");
    }
    for header in [
        "Class&.new do",
        "Foo::Class.new do",
        "Data.new do",
        "Struct.define do",
        "klass.new do",
    ] {
        let source = format!("{header}\n{}end\n", assigns(26, "  "));
        assert_eq!(findings(&source), [(1, 1, 28, msg(26))], "{header}");
    }
}

#[test]
fn only_refine_is_allowed_with_any_receiver() {
    for header in [
        "refine String do",
        "Foo.refine(String) do",
        "foo&.refine(String) do",
    ] {
        let source = format!("{header}\n{}end\n", assigns(30, "  "));
        assert_eq!(findings(&source), [], "{header}");
    }
    // The spec's custom allow lists do not apply at the fixed defaults.
    for header in [
        "foo do",
        "Gem::Specification.new do",
        "Foo::Bar.baz do",
        "Foo::Bar.bar do",
        "refined do",
    ] {
        let source = format!("{header}\n{}end\n", assigns(26, "  "));
        assert_eq!(findings(&source), [(1, 1, 28, msg(26))], "{header}");
    }
    let source = format!("Foo::\n  Bar.baz do\n{}end\n", assigns(26, "  "));
    assert_eq!(findings(&source), [(1, 1, 29, msg(26))]);
}

#[test]
fn arrays_and_comments_follow_the_fixed_counting() {
    let source = format!(
        "something do\n  a = 1\n  a = [\n{}  ]\nend\n",
        body(23, "    ", |i| format!("{i},"))
    );
    assert_eq!(findings(&source), [(1, 1, 28, msg(26))]);
    let source = format!("something do\n{}  #a = 2\nend\n", assigns(25, "  "));
    assert_eq!(findings(&source), []);
}

#[test]
fn lambdas_super_and_define_method_blocks_are_blocks() {
    for header in [
        "-> do",
        "->(x) do",
        "lambda do",
        "super do",
        "super(1) do",
        "define_method(:m) do",
        "foo.bar(1).baz do",
    ] {
        let source = format!("{header}\n{}end\n", assigns(26, "  "));
        assert_eq!(findings(&source), [(1, 1, 28, msg(26))], "{header}");
    }
    let source = format!(
        "x = foo.map do\n{}end.select {{ |y| y }}\n",
        assigns(26, "  ")
    );
    assert_eq!(findings(&source), [(1, 5, 28, msg(26))]);
}

#[test]
fn block_arguments_and_one_line_spans_are_not_measured() {
    let source = format!(
        "foo(&blk)\nfoo(<<~TEXT) {{ bar }}\n{}TEXT\n",
        assigns(30, "  ")
    );
    assert_eq!(findings(&source), []);
    let source = format!("foo {{ bar(<<~TEXT) }}\n{}TEXT\n", assigns(30, "  "));
    assert_eq!(findings(&source), []);
}

#[test]
fn rescue_and_ensure_bodies() {
    let source = format!(
        "foo do\n{}rescue\n{}ensure\nend\n",
        assigns(20, "  "),
        assigns(4, "  ")
    );
    assert_eq!(findings(&source), [(1, 1, 28, msg(26))]);
}

#[test]
fn the_pinned_gemspec_exclusion_is_a_path_policy() {
    let cop = BlockLength::default();
    let policy = cop.path_policy().expect("pinned Exclude");
    assert!(!policy.allows(std::path::Path::new("foo.gemspec")));
    assert!(!policy.allows(std::path::Path::new("a/b/foo.gemspec")));
    assert!(policy.allows(std::path::Path::new("foo.rb")));
    assert!(policy.allows(std::path::Path::new("gemspec.rb")));
}
