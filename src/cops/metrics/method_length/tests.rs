//! Examples ported from RuboCop's
//! `spec/rubocop/cop/metrics/method_length_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! All 33 pinned examples run verbatim at the fixed defaults and match the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! source SHA-256 was checked against it when the rows were ported): the
//! spec sets `Max: 5`, so none of them reaches the fixed `Max: 10`. Each
//! upstream finding is then ported at the fixed threshold (five more body
//! lines, `[11/10]`), with the same shape, position and end line. The
//! spec-custom `CountComments: true`, `CountAsOne: ['array']`,
//! `AllowedMethods: ['foo']` and `AllowedPatterns: [/_name/]` examples are
//! not applicable at the fixed defaults; their ports assert the fixed
//! behaviour (comments not counted, arrays not folded, nothing allowed).
use super::*;
use crate::pipeline::lint_str_with;

const COP: &str = "Metrics/MethodLength";

/// `(line, col, end line, message)` of every finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, u32, String)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&MethodLength::default()], source)
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
    format!("Method has too many lines. [{length}/10]")
}

/// `count` statement lines `{indent}a = {i}` (or `{indent}{f(i)}`).
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
        177,
        r#"def m
  a = 1
  a = 2
  a = 3
  a = 4
end
"#,
    ),
    (
        188,
        r#"def m(x,
      y,
      z)
  a = 1
  a = 2
  a = 3
  a = 4
end
"#,
    ),
    (
        201,
        r#"def m()
  a = 1
  a = 2
  a = 3
  a = 4


  a = 7
end
"#,
    ),
    (
        215,
        r#"def m()
end
"#,
    ),
    (
        222,
        r#"def one_line; 10 end
def self.m()
  a = 1
  a = 2
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        235,
        r#"def one_line(test) 10 end
def self.m()
  a = 1
  a = 2
  a = 4
  a = 5
  a = 6
end
"#,
    ),
    (
        248,
        r#"def m
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
        262,
        r#"def m()
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
        275,
        r#"def foo
  do_something do
    <<~HEREDOC
      text
    HEREDOC
  end
end
"#,
    ),
    (
        287,
        r#"def m
  a = 1
  a = 2
  a = <<~TEXT
    text
  TEXT
  __ENCODING__
end
"#,
    ),
    (
        8,
        r#"def m
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
end
"#,
    ),
    (
        20,
        r#"def m
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
        36,
        r#"define_method(:m) do
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
end
"#,
    ),
    (
        48,
        r#"define_method(:m) do
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
        62,
        r##"define_method(:"#{method_name}=") do
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"##,
    ),
    (
        76,
        r##"define_method("#{method_name}=") do
  a = 1
  a = 2
  a = 3
  a = 4
  a = 5
  a = 6
end
"##,
    ),
    (
        90,
        r#"define_method do
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
        107,
        r#"define_method(:m) do
  a = _1
  a = _2
  a = _3
  a = _4
  a = _5
  a = _6
end
"#,
    ),
    (
        125,
        r#"define_method(:m) do
  a = it
  a = it
  a = it
  a = it
  a = it
  a = it
end
"#,
    ),
    (
        142,
        r#"def self.m
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
        158,
        r#"class K
  class << self
    def m
      a = 1
      a = 2
      a = 3
      a = 4
      a = 5
      a = 6
    end
  end
end
"#,
    ),
    (
        304,
        r#"def m
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
        323,
        r#"def m
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
        337,
        r#"def foo
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
        350,
        r#"define_method(:foo) do
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
        363,
        r#"define_method(:foo) do
  a = _1
  a = _2
  a = _3
  a = _4
  a = _5
  a = _6
end
"#,
    ),
    (
        376,
        r#"define_method(:foo) do
  a = it
  a = it
  a = it
  a = it
  a = it
  a = it
end
"#,
    ),
    (
        393,
        r#"def user_name
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
        406,
        r#"define_method(:user_name) do
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
        419,
        r#"define_method(:user_name) do
  a = _1
  a = _2
  a = _3
  a = _4
  a = _5
  a = _6
end
"#,
    ),
    (
        432,
        r#"define_method(:user_name) do
  a = it
  a = it
  a = it
  a = it
  a = it
  a = it
end
"#,
    ),
    (
        445,
        r#"def firstname
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
        464,
        r#"def m
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
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 33);
    for (line, source) in PINNED {
        assert_eq!(findings(source), [], "spec line {line}");
    }
}

#[test]
fn instance_methods_at_and_over_the_fixed_max() {
    assert_eq!(findings(&format!("def m\n{}end\n", assigns(10, "  "))), []);
    assert_eq!(
        findings(&format!("def m\n{}end\n", assigns(11, "  "))),
        [(1, 1, 13, msg(11))]
    );
}

#[test]
fn define_method_blocks_of_every_kind_and_name() {
    for header in [
        "define_method(:m) do",
        "define_method(:\"#{method_name}=\") do",
        "define_method(\"#{method_name}=\") do",
        "define_method do",
        // Upstream's allow lists are empty at the fixed defaults.
        "define_method(:foo) do",
        "define_method(:user_name) do",
        "self.class.define_method(:m) do",
        "klass&.define_method(:m) do",
    ] {
        assert_eq!(
            findings(&format!("{header}\n{}end\n", assigns(10, "  "))),
            [],
            "{header}"
        );
        assert_eq!(
            findings(&format!("{header}\n{}end\n", assigns(11, "  "))),
            [(1, 1, 13, msg(11))],
            "{header}"
        );
    }
    // numblock and itblock.
    for param in ["_1", "it"] {
        let source = format!(
            "define_method(:m) do\n{}end\n",
            body(11, "  ", |_| format!("a = {param}"))
        );
        assert_eq!(findings(&source), [(1, 1, 13, msg(11))], "{param}");
    }
    let braces = format!("define_method(:m) {{\n{}}}\n", assigns(11, "  "));
    assert_eq!(findings(&braces), [(1, 1, 13, msg(11))]);
}

#[test]
fn class_and_singleton_class_methods() {
    assert_eq!(
        findings(&format!("def self.m\n{}end\n", assigns(11, "  "))),
        [(1, 1, 13, msg(11))]
    );
    let source = format!(
        "class K\n  class << self\n    def m\n{}    end\n  end\nend\n",
        assigns(11, "      ")
    );
    assert_eq!(findings(&source), [(3, 5, 15, msg(11))]);
}

#[test]
fn signature_blank_and_comment_lines_are_not_counted() {
    let source = format!("def m(x,\n      y,\n      z)\n{}end\n", assigns(10, "  "));
    assert_eq!(findings(&source), []);
    let source = format!("def m()\n{}\n\n  a = 7\nend\n", assigns(9, "  "));
    assert_eq!(findings(&source), []);
    // CountComments is fixed to false (the spec's `true` context does not apply).
    let source = format!("def m\n{}  #a = 2\n  # a = 3\nend\n", assigns(10, "  "));
    assert_eq!(findings(&source), []);
    assert_eq!(findings("def m()\nend\n"), []);
}

#[test]
fn one_liner_methods_do_not_fool_the_count() {
    for one_liner in ["def one_line; 10 end", "def one_line(test) 10 end"] {
        let source = format!("{one_liner}\ndef self.m()\n{}end\n", assigns(11, "  "));
        assert_eq!(findings(&source), [(2, 1, 14, msg(11))], "{one_liner}");
    }
}

#[test]
fn a_method_ending_with_a_block_counts_the_block_lines() {
    let source = format!("def m\n  something do\n{}  end\nend\n", assigns(9, "    "));
    assert_eq!(findings(&source), [(1, 1, 13, msg(11))]);
}

#[test]
fn heredocs_count_to_their_terminator() {
    // Port of the heredoc-and-`__ENCODING__` example.
    let source = format!(
        "def m\n{}  a = <<~TEXT\n    text\n  TEXT\n  __ENCODING__\nend\n",
        assigns(7, "  ")
    );
    assert_eq!(findings(&source), [(1, 1, 13, msg(11))]);
    // Port of the heredoc-in-a-block example: RuboCop counts up to the
    // heredoc terminator, not the block's `end` (10 lines here).
    let source = format!(
        "def foo\n  do_something do\n    <<~HEREDOC\n{}    HEREDOC\n  end\nend\n",
        body(7, "      ", |i| format!("text {i}"))
    );
    assert_eq!(findings(&source), []);
    let source = format!(
        "def foo\n  do_something do\n    <<~HEREDOC\n{}    HEREDOC\n  end\nend\n",
        body(8, "      ", |i| format!("text {i}"))
    );
    assert_eq!(findings(&source), [(1, 1, 14, msg(11))]);
    // A heredoc that ends the body: its lines count through the terminator.
    let source = format!(
        "def m\n{}  foo(<<~TEXT)\n    one\n    two\n  TEXT\nend\n",
        assigns(7, "  ")
    );
    assert_eq!(findings(&source), [(1, 1, 13, msg(11))]);
}

#[test]
fn arrays_are_not_folded_at_the_fixed_defaults() {
    let source = format!(
        "def m\n  a = 1\n  a = [\n{}  ]\nend\n",
        body(9, "    ", |i| format!("{i},"))
    );
    assert_eq!(findings(&source), [(1, 1, 14, msg(12))]);
}

#[test]
fn allow_list_examples_are_reported_at_the_fixed_defaults() {
    for name in ["m", "foo", "user_name", "firstname"] {
        let source = format!("def {name}\n{}end\n", assigns(11, "  "));
        assert_eq!(findings(&source), [(1, 1, 13, msg(11))], "{name}");
    }
}

#[test]
fn rescue_ensure_and_endless_bodies() {
    let source = format!(
        "def m\n{}rescue\n{}end\n",
        assigns(6, "  "),
        assigns(4, "  ")
    );
    assert_eq!(findings(&source), [(1, 1, 13, msg(11))]);
    // A bare `ensure` ends RuboCop's `ensure` body node.
    let source = format!("def m\n{}ensure\nend\n", assigns(10, "  "));
    assert_eq!(findings(&source), [(1, 1, 13, msg(11))]);
    let source = format!("def m\n{}rescue\nend\n", assigns(9, "  "));
    assert_eq!(findings(&source), []);
    let source = format!("def m = (\n{})\n", assigns(10, "  "));
    assert_eq!(findings(&source), [(1, 1, 12, msg(12))]);
}

#[test]
fn lookalikes_are_not_methods() {
    for header in [
        "define_method(:m, &blk)\nfoo do",
        "-> do",
        "lambda do",
        "defined_method(:m) do",
    ] {
        let source = format!("{header}\n{}end\n", assigns(11, "  "));
        assert_eq!(findings(&source), [], "{header}");
    }
    // A long heredoc after a one-line definition: the definition spans one line.
    let source = format!("def m = foo(<<~TEXT)\n{}TEXT\n", assigns(12, "  "));
    assert_eq!(findings(&source), []);
}

#[test]
fn nested_definitions_are_each_measured() {
    let source = format!(
        "def outer\n  def inner\n{}  end\nend\n",
        assigns(11, "    ")
    );
    assert_eq!(
        findings(&source),
        [(1, 1, 15, msg(13)), (2, 3, 14, msg(11))]
    );
}

#[test]
fn block_comments_and_interpolation_lines_follow_upstream_text_rules() {
    // `=begin` documentation is not a `#` comment line.
    let source = format!(
        "def m\n{}=begin\nnote\n=end\n  a = 9\nend\n",
        assigns(7, "  ")
    );
    assert_eq!(findings(&source), [(1, 1, 13, msg(11))]);
}

#[test]
fn heredoc_bodies_end_at_the_last_parser_descendant() {
    // The body is the `if`: its `end` is not a descendant, a bare `else`
    // after `elsif` is (parser's elsif node ends there).
    let lines = |n| body(n, "    ", |i| format!("text {i}"));
    let source = format!(
        "def m\n  if a\n    <<~T\n{}    T\n  elsif b\n    x\n  else\n  end\nend\n",
        lines(4)
    );
    // Lines 2..=10: if, <<~T, 4 texts, T, elsif, x, else = 10.
    assert_eq!(findings(&source), []);
    let source = format!(
        "def m\n  if a\n    <<~T\n{}    T\n  elsif b\n    x\n  else\n  end\nend\n",
        lines(5)
    );
    assert_eq!(findings(&source), [(1, 1, 14, msg(11))]);
    // Without the elsif the bare `else` is no descendant's end: 10 lines.
    let source = format!(
        "def m\n  if a\n    <<~T\n{}    T\n    x\n    y\n  else\n  end\nend\n",
        lines(5)
    );
    assert_eq!(findings(&source), []);
    // A bare `ensure` of an explicit `begin` ends its parser `ensure` node.
    let source = format!(
        "def m\n  begin\n    <<~T\n{}    T\n  ensure\n  end\nend\n",
        lines(7)
    );
    assert_eq!(findings(&source), [(1, 1, 14, msg(11))]);
}
