//! Examples ported from RuboCop's `spec/rubocop/cop/layout/empty_line_after_guard_clause_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 59
//! examples that parse as Ruby 3.4 run at the fixed defaults; findings,
//! messages and single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run. The `Ruby <= 3.2` context (spec
//! line 30, `unsupported_on: :prism`) puts `next` directly in a `def`, which
//! Prism rejects for Ruby 3.4 (`Invalid next`): it is a syntax error here, and
//! the same guard inside a block is checked instead.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str,
};

const COP: &str = "Layout/EmptyLineAfterGuardClause";
const MESSAGE: &str = "Add empty line after guard clause.";

fn positions(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        ruby_prism::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MESSAGE);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            (o.line, o.col)
        })
        .collect()
}

fn check(source: &str, finding: Option<(u32, u32)>, after: &str) {
    assert_eq!(
        positions(source),
        finding.into_iter().collect::<Vec<_>>(),
        "{source}"
    );
    if finding.is_some() {
        assert_correction(&EmptyLineAfterGuardClause, source, after);
    }
}

fn held(source: &str) {
    let found = correction_offenses(&EmptyLineAfterGuardClause, source);
    assert_eq!(found.len(), 1, "{source:?}");
    assert!(found[0].correction.is_none(), "{source:?}");
}

/// `(spec line, source, fixed-default finding, single-pass correction)`.
type Pinned = (u32, &'static str, Option<(u32, u32)>, &'static str);

const PINNED: &[Pinned] = &[
    (5, "return unless item.positive?\n", None, ""),
    (
        11,
        "def foo\n  return if need_return?\n  foobar\nend\n",
        Some((2, 3)),
        "def foo\n  return if need_return?\n\n  foobar\nend\n",
    ),
    (
        49,
        "def foo\n  return another_object if something_different?\n  begin\n    bar\n  rescue SomeException\n    baz\n  end\nend\n",
        Some((2, 3)),
        "def foo\n  return another_object if something_different?\n\n  begin\n    bar\n  rescue SomeException\n    baz\n  end\nend\n",
    ),
    (
        76,
        "def foo\n  raise ArgumentError, <<-MSG unless path\n    Must be called with mount point\n  MSG\n  bar\nend\n",
        Some((4, 1)),
        "def foo\n  raise ArgumentError, <<-MSG unless path\n    Must be called with mount point\n  MSG\n\n  bar\nend\n",
    ),
    (
        99,
        "def foo\n  raise ArgumentError, <<-MSG if path\n    Must be called with mount point\n  MSG\n  bar\nend\n",
        Some((4, 1)),
        "def foo\n  raise ArgumentError, <<-MSG if path\n    Must be called with mount point\n  MSG\n\n  bar\nend\n",
    ),
    (
        122,
        "raise(<<-FAIL) unless true\n  #{1 + 1}\nFAIL\n1\n",
        Some((3, 1)),
        "raise(<<-FAIL) unless true\n  #{1 + 1}\nFAIL\n\n1\n",
    ),
    (
        141,
        "def foo\n  raise unless $1 == o\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        152,
        "def foo\n  raise SerializationError.new(\"Unsupported argument type: #{argument.class.name}\") unless serializer\n  serializer.serialize(argument)\nend\n",
        Some((2, 3)),
        "def foo\n  raise SerializationError.new(\"Unsupported argument type: #{argument.class.name}\") unless serializer\n\n  serializer.serialize(argument)\nend\n",
    ),
    (
        170,
        "def foo\n  render :foo and return if condition\n  do_something\nend\n",
        Some((2, 3)),
        "def foo\n  render :foo and return if condition\n\n  do_something\nend\n",
    ),
    (
        188,
        "def foo\n  render :foo or return if condition\n  do_something\nend\n",
        Some((2, 3)),
        "def foo\n  render :foo or return if condition\n\n  do_something\nend\n",
    ),
    (
        206,
        "def foo\n  return if condition\n  # rubocop:disable Department/Cop\n  bar\n  # rubocop:enable Department/Cop\nend\n",
        Some((2, 3)),
        "def foo\n  return if condition\n\n  # rubocop:disable Department/Cop\n  bar\n  # rubocop:enable Department/Cop\nend\n",
    ),
    (
        228,
        "def foo\n  # rubocop:disable Department/Cop\n  return if condition\n  # rubocop:enable Department/Cop\n  bar\nend\n",
        Some((3, 3)),
        "def foo\n  # rubocop:disable Department/Cop\n  return if condition\n  # rubocop:enable Department/Cop\n\n  bar\nend\n",
    ),
    (
        250,
        "def foo\n  foo += 1 if need_add?\n  foobar\nend\n",
        None,
        "",
    ),
    (
        259,
        "def method\n  if truthy\n    raise <<-MSG\n      This is an error.\n    MSG\n  end\n\n  value\nend\n",
        None,
        "",
    ),
    (
        274,
        "def method\n  if truthy\n    raise <<-MSG\n      This is an error.\n    MSG\n  end\n  value\nend\n",
        Some((6, 3)),
        "def method\n  if truthy\n    raise <<-MSG\n      This is an error.\n    MSG\n  end\n\n  value\nend\n",
    ),
    (
        300,
        "def foo\n  if something?\n    return\n  end\nend\n",
        None,
        "",
    ),
    (
        310,
        "def foo\n  raise ArgumentError, 'HTTP redirect too deep' if limit.zero?\n\n  foobar\nend\n",
        None,
        "",
    ),
    (
        320,
        "def foo\n  # rubocop:disable Department/Cop\n  return if condition\n  # rubocop:enable Department/Cop\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        332,
        "def foo\n  # :nocov:\n  return if condition\n  # :nocov:\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        344,
        "def foo\n  # :nocov: legacy adapter\n  return if condition\n  # :nocov: legacy adapter\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        356,
        "def foo\n  # simplecov:disable\n  return if condition\n  # simplecov:enable\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        368,
        "def foo\n  # simplecov:disable line, branch legacy adapter\n  return if condition\n  # simplecov:enable line, branch legacy adapter\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        380,
        "def foo\n  #simplecov:disable\n  return if condition\n  #simplecov:enable\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        392,
        "def foo\n  # simplecov : disable\n  return if condition\n  # simplecov : enable\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        404,
        "def foo\n  # simplecov:disable\n  return if condition\n  # simplecov:enable\n  bar\nend\n",
        Some((3, 3)),
        "def foo\n  # simplecov:disable\n  return if condition\n  # simplecov:enable\n\n  bar\nend\n",
    ),
    (
        426,
        "def foo\n  object.tap { |obj| return another_object if something? }\n  foobar\nend\n",
        None,
        "",
    ),
    (
        435,
        "return if condition\nfoo do\n  bar\nend\nbaz\n",
        Some((1, 1)),
        "return if condition\n\nfoo do\n  bar\nend\nbaz\n",
    ),
    (
        455,
        "return if condition; foo do\n  bar\nend\nbaz\n",
        None,
        "",
    ),
    (
        464,
        "return if condition; foo do\n  bar(_1)\nend\nbaz\n",
        None,
        "",
    ),
    (
        473,
        "def foo\n  return another_object if something?\n  return another_object if something_else?\n  return another_object if something_different?\n\n  foobar\nend\n",
        None,
        "",
    ),
    (
        485,
        "def foo\n  render :bar and return if condition1?\n  render :baz and return if condition2?\n\n  foobar\nend\n",
        None,
        "",
    ),
    (
        496,
        "def foo\n  return if something?\n  if something_else?\n    raise bar(\n      baz\n    )\n  end\nend\n",
        None,
        "",
    ),
    (
        509,
        "def foo\n  return if something?\n  if something_else?\n    fail bar(\n      baz\n    )\n  end\nend\n",
        None,
        "",
    ),
    (
        522,
        "def foo\n  return if something?\n  if something_else?\n    return bar(\n      baz\n    )\n  end\nend\n",
        None,
        "",
    ),
    (
        535,
        "collection.each do |item|\n  break if something?\n  if something_else?\n    break bar(\n      baz\n    )\n  end\nend\n",
        None,
        "",
    ),
    (
        548,
        "collection.each do |item|\n  next if something?\n  if something_else?\n    next bar(\n      baz\n    )\n  end\nend\n",
        None,
        "",
    ),
    (
        561,
        "def foo\n  return if something?\n  if something_else?\n    bar(\n      baz\n    )\n  end\nend\n",
        Some((2, 3)),
        "def foo\n  return if something?\n\n  if something_else?\n    bar(\n      baz\n    )\n  end\nend\n",
    ),
    (
        587,
        "def foo\n  return another_object if something_different?\nend\n",
        None,
        "",
    ),
    (
        595,
        "def foo\n  begin\n    return another_object if something_different?\n  rescue SomeException\n    bar\n  end\nend\n",
        None,
        "",
    ),
    (
        607,
        "def foo\n  begin\n    return another_object if something_different?\n  ensure\n    bar\n  end\nend\n",
        None,
        "",
    ),
    (
        619,
        "def foo\n  begin\n    bar\n  rescue SomeException\n    return another_object if something_different?\n  else\n    bar\n  end\nend\n",
        None,
        "",
    ),
    (
        633,
        "def foo\n  if cond\n    return another_object if something_different?\n  else\n    bar\n  end\nend\n",
        None,
        "",
    ),
    (
        645,
        "def foo\n  if cond\n    return another_object if something_different?\n  elsif\n    bar\n  end\nend\n",
        None,
        "",
    ),
    (
        657,
        "def foo\n  raise ArgumentError, <<-MSG unless path\n    Must be called with mount point\n  MSG\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        669,
        "def foo\n  raise ArgumentError, <<-MSG unless path\n    foo\n    bar\n    baz\n  MSG\n\n  bar\nend\n",
        None,
        "",
    ),
    (
        683,
        "def foo\n  raise ArgumentError, <<~END.squish.it.good unless guard\n    A multiline message\n    that will be squished.\n  END\n\n  return_value\nend\n",
        None,
        "",
    ),
    (
        696,
        "def foo\n  raise ArgumentError, call(<<~END.squish) unless guard\n    A multiline message\n    that will be squished.\n  END\n\n  return_value\nend\n",
        None,
        "",
    ),
    (
        709,
        "def foo\n  raise ArgumentError, (<<~END.squish) unless guard\n    A multiline message\n    that will be squished.\n  END\n\n  return_value\nend\n",
        None,
        "",
    ),
    (
        722,
        "def foo\n  return true if <<~TEXT.length > bar\n    hi\n  TEXT\n\n  false\nend\n",
        None,
        "",
    ),
    (
        734,
        "def foo\n  raise if <<~TEXT.length > bar\n    hi\n  TEXT\n\n  baz\nend\n",
        None,
        "",
    ),
    (
        746,
        "def foo\n  puts 'some action happens here'\nrescue => e\n  a_check ? raise(e) : other_thing\n  true\nend\n",
        Some((4, 3)),
        "def foo\n  puts 'some action happens here'\nrescue => e\n  a_check ? raise(e) : other_thing\n\n  true\nend\n",
    ),
    (
        768,
        "def foo\n  return unless need_next?\n  end_this!\nend\n",
        Some((2, 3)),
        "def foo\n  return unless need_next?\n\n  end_this!\nend\n",
    ),
    (
        786,
        "def foo\n  return if foo?\n  return if bar?\n  foobar\nend\n",
        Some((3, 3)),
        "def foo\n  return if foo?\n  return if bar?\n\n  foobar\nend\n",
    ),
    (
        806,
        "def foo\n  puts(<<~MSG) and return if bar\n    A multiline\n    message\n  MSG\n\n  baz\nend\n",
        None,
        "",
    ),
    (
        819,
        "def foo\n  puts(<<~MSG) and return if bar\n    A multiline\n    message\n  MSG\n  baz\nend\n",
        Some((5, 1)),
        "def foo\n  puts(<<~MSG) and return if bar\n    A multiline\n    message\n  MSG\n\n  baz\nend\n",
    ),
    (
        843,
        "def foo(item)\n  return unless item.positive?; item * 2\nend\n",
        None,
        "",
    ),
    (
        851,
        "def foo(item)\n  return unless item.positive?;\n  item * 2\nend\n",
        Some((2, 3)),
        "def foo(item)\n  return unless item.positive?;\n\n  item * 2\nend\n",
    ),
    (869, "if cond then return end.then { 42 }\n", None, ""),
    (
        875,
        "def foo(item)\n  return unless item.positive?;\n\n  item * 2\nend\n",
        None,
        "",
    ),
];

#[test]
fn pinned_examples() {
    for &(line, source, finding, after) in PINNED {
        assert_eq!(
            positions(source),
            finding.into_iter().collect::<Vec<_>>(),
            "spec line {line}"
        );
        if finding.is_some() {
            assert_correction(&EmptyLineAfterGuardClause, source, after);
        }
    }
}

/// Spec line 30 (`Ruby <= 3.2`, `unsupported_on: :prism`).
#[test]
fn next_directly_in_a_method_is_a_ruby34_syntax_error() {
    let source = "def foo\n  next unless need_next? # comment\n  foobar\nend\n";
    assert!(ruby_prism::parse(source.as_bytes()).errors().count() > 0);
    check(
        "foo do\n  next unless need_next? # comment\n  foobar\nend\n",
        Some((2, 3)),
        "foo do\n  next unless need_next? # comment\n\n  foobar\nend\n",
    );
}

#[test]
fn guard_clause_shapes() {
    for (source, finding, after) in [
        // `fail`, `break`, `next` and `unless`/`if` bodies of one statement.
        (
            "def f\n  fail ArgumentError unless x\n  y\nend\n",
            Some((2, 3)),
            "def f\n  fail ArgumentError unless x\n\n  y\nend\n",
        ),
        (
            "loop do\n  break if done?\n  work\nend\n",
            Some((2, 3)),
            "loop do\n  break if done?\n\n  work\nend\n",
        ),
        (
            "def f\n  if x then return end\n  y\nend\n",
            Some((2, 20)),
            "def f\n  if x then return end\n\n  y\nend\n",
        ),
        (
            "def f\n  unless x\n    return\n  end\n  y\nend\n",
            Some((4, 3)),
            "def f\n  unless x\n    return\n  end\n\n  y\nend\n",
        ),
        // `&&` reads as `and`; a multi-line guard is not single-line.
        (
            "def f\n  log && return if x\n  y\nend\n",
            Some((2, 3)),
            "def f\n  log && return if x\n\n  y\nend\n",
        ),
        ("def f\n  return(\n    1\n  ) if x\n  y\nend\n", None, ""),
        // Not a guard: a receiver, a block, several statements, an else.
        ("def f\n  Kernel.raise if x\n  y\nend\n", None, ""),
        ("def f\n  raise { 1 } if x\n  y\nend\n", None, ""),
        (
            "def f\n  if x\n    a\n    return\n  end\n  y\nend\n",
            None,
            "",
        ),
        ("def f\n  x = 1 if y\n  z\nend\n", None, ""),
        // A sole statement: its parser parent is the enclosing node.
        ("foo do\n  return if x\nend\n", None, ""),
        ("x = [1].each { |a| next if a }\n", None, ""),
        // A keyword `begin` has no same-line exemption.
        (
            "begin\n  return if x; y\nend\n",
            Some((2, 3)),
            "begin\n  return if x; y\n\nend\n",
        ),
        // Whitespace-only and CRLF lines.
        ("def f\n  return if x\n  \t\n  y\nend\n", None, ""),
        (
            "def f\r\n  return if x\r\n  y\r\nend\r\n",
            Some((2, 3)),
            "def f\r\n  return if x\r\n\r\n  y\r\nend\r\n",
        ),
        // An enable directive in either spelling, followed by a blank line.
        (
            "def f\n  return if x\n  # rustocop:enable Foo/Bar\n\n  y\nend\n",
            None,
            "",
        ),
        (
            "def f\n  return if x\n  # rustocop:enable Foo/Bar\n  y\nend\n",
            Some((2, 3)),
            "def f\n  return if x\n  # rustocop:enable Foo/Bar\n\n  y\nend\n",
        ),
        // A pattern guard is not an `if` node.
        ("case x\nin Integer if y\n  return\nend\n", None, ""),
    ] {
        check(source, finding, after);
    }
}

#[test]
fn heredoc_guards() {
    for (source, finding) in [
        // Two heredocs: upstream's search finds the first argument's body,
        // and the line after its terminator starts the second body.
        (
            "def f\n  raise X.new(<<~A, <<~B) if x\n    a\n  A\n    b\n  B\n  y\nend\n",
            (4, 1),
        ),
        // A heredoc inside an array argument is not found: the guard's own
        // line ends where the heredoc body is pending.
        (
            "def f\n  raise [<<~A].first if x\n    a\n  A\n\n  y\nend\n",
            (2, 3),
        ),
    ] {
        assert_eq!(positions(source), vec![finding], "{source}");
        held(source);
    }
    // A heredoc body that ends before the next statement is fine.
    check(
        "def f\n  raise X, <<~A if x\n    a\n  A\n  y = <<~B\n    b\n  B\nend\n",
        Some((4, 1)),
        "def f\n  raise X, <<~A if x\n    a\n  A\n\n  y = <<~B\n    b\n  B\nend\n",
    );
}

#[test]
fn metadata_is_fixed() {
    let cop = EmptyLineAfterGuardClause;
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert!(cop.department_selection_enables());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
}
