//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/empty_lines_around_access_modifier_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 176
//! pinned examples run at the fixed `around` style with
//! `Layout/EmptyLinesAroundBlockBody` at its default `no_empty_lines`: the
//! `only_before` context and the `EmptyLinesAroundBlockBody: empty_lines`
//! context are replayed at the fixed defaults, like the inventory's fixed
//! results. Findings, messages and single-pass corrected bytes equal the
//! fixed replay recorded in the pinned RuboCop reference run; the
//! `:ruby27` and `:ruby34` contexts apply.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/EmptyLinesAroundAccessModifier";
const MODIFIERS: [&str; 4] = ["private", "protected", "public", "module_function"];

/// `(line, col, after-only message)` of each finding.
fn findings(source: &str) -> Vec<(u32, u32, bool)> {
    assert_eq!(
        ruby_prism::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&EmptyLinesAroundAccessModifier], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            let after = o.message.starts_with("Keep a blank line after `");
            assert!(
                after
                    || o.message
                        .starts_with("Keep a blank line before and after `"),
                "{}",
                o.message
            );
            (o.line, o.col, after)
        })
        .collect();
    found.sort_unstable();
    found
}

/// The single finding's correction, `None` when withheld.
fn corrected(source: &str) -> Option<String> {
    let found = correction_offenses(&EmptyLinesAroundAccessModifier, source);
    assert_eq!(found.len(), 1, "{source:?}");
    found[0].correction.as_ref()?;
    let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
    Some(String::from_utf8(applied.bytes).unwrap())
}

const BA: Option<bool> = Some(false);
const AFTER: Option<bool> = Some(true);

/// `(spec line, source, finding line and column, after-only message,
/// single-pass correction)` with `%m` standing for the modifier; an empty
/// correction for a finding means none is made.
type Pinned = (u32, &'static str, (u32, u32), Option<bool>, &'static str);

/// The `around` examples, run for all four modifiers.
const AROUND: &[Pinned] = &[
    (
        9,
        "class Test\n  something\n  %m\n\n  def test; end\nend\n",
        (3, 3),
        BA,
        "class Test\n  something\n\n  %m\n\n  def test; end\nend\n",
    ),
    (
        31,
        "class Test\n  something\n\n  %m\n  def test; end\nend\n",
        (4, 3),
        BA,
        "class Test\n  something\n\n  %m\n\n  def test; end\nend\n",
    ),
    (
        53,
        "class Test\n  something\n\n  # This comment is fine\n  %m\n\n  def test; end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        66,
        "class Test\n  def %m?\n    %m\n  end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        77,
        "class Test\n  attr_reader %m\n  def foo\n  end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        87,
        "class Test\n  def %m?\n    if true\n      %m\n    end\n  end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        99,
        "class Test\n  def %m?\n    %m if true\n  end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        109,
        "def foo\n  %m { do_something }\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        117,
        "class Test\n  something\n\n  %m # let's modify the rest\n  def test; end\nend\n",
        (4, 3),
        BA,
        "class Test\n  something\n\n  %m # let's modify the rest\n\n  def test; end\nend\n",
    ),
    (
        139,
        "class Test\n  %m\n\n  def test; end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        149,
        "module Test\n  %m\n\n  def test; end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        159,
        "class << self\n  %m\n\n  def test; end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        169,
        "class Foo <\n      Bar\n  %m\n\n  def do_something\n  end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        181,
        "class <<\n      self\n  %m\n\n  def do_something\n  end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (194, "%m\n\ndef do_something\nend\n", (0, 0), None, ""),
    (
        204,
        "class Test\n  %m\n  def test\n  end\nend\n",
        (2, 3),
        AFTER,
        "class Test\n  %m\n\n  def test\n  end\nend\n",
    ),
    (
        215,
        "# comment\n%m\n\ndef do_something\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        227,
        "included do\n  %m\n\n  def test; end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        237,
        "included do |foo|\n  %m\n\n  def test; end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        247,
        "included do\n  %m\n  def test\n  end\nend\n",
        (2, 3),
        AFTER,
        "included do\n  %m\n\n  def test\n  end\nend\n",
    ),
    (
        269,
        "included {\n  %m\n\n  def test; end\n}\n",
        (0, 0),
        None,
        "",
    ),
    (
        279,
        "included { |foo|\n  %m\n\n  def test; end\n}\n",
        (0, 0),
        None,
        "",
    ),
    (
        291,
        "class Test\n  def test; end\n\n  %m\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        301,
        "class Test < Base\n  def test; end\n\n  %m\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        311,
        "class << self\n  def test; end\n\n  %m\nend\n",
        (0, 0),
        None,
        "",
    ),
    (
        321,
        "class Test\n  %m\n  end_this!\nend\n",
        (2, 3),
        AFTER,
        "class Test\n  %m\n\n  end_this!\nend\n",
    ),
    (
        339,
        "class Test\r\n\r\n  %m\r\n\r\n  def test; end\r\nend\r\n",
        (0, 0),
        None,
        "",
    ),
    (350, "%m\n", (0, 0), None, ""),
    (356, "%m; foo\n.bar\n", (0, 0), None, ""),
    (364, "foo\n%m\nbar\n", (2, 1), BA, "foo\n\n%m\n\nbar\n"),
    // `EmptyLinesAroundBlockBody: no_empty_lines` (its default).
    (388, "Module.new do\n  %m\nend\n", (2, 3), AFTER, ""),
    (
        401,
        "Module.new do\n  %m\n  foo\nend\n",
        (2, 3),
        AFTER,
        "Module.new do\n  %m\n\n  foo\nend\n",
    ),
    (
        421,
        "Module.new do\n  foo\n  %m\nend\n",
        (3, 3),
        BA,
        "Module.new do\n  foo\n\n  %m\nend\n",
    ),
    // `EmptyLinesAroundBlockBody: empty_lines`, replayed at the default.
    (447, "Module.new do\n  %m\nend\n", (2, 3), AFTER, ""),
    (
        465,
        "Module.new do\n  %m\n  foo\nend\n",
        (2, 3),
        AFTER,
        "Module.new do\n  %m\n\n  foo\nend\n",
    ),
    (
        485,
        "Module.new do\n  foo\n  %m\nend\n",
        (3, 3),
        BA,
        "Module.new do\n  foo\n\n  %m\nend\n",
    ),
];

/// The `only_before` context replayed at `around`: `private`/`protected`.
const ONLY_BEFORE_SPECIAL: &[Pinned] = &[
    (
        512,
        "class Test\n  something\n\n  %m\n  def test; end\nend\n",
        (4, 3),
        BA,
        "class Test\n  something\n\n  %m\n\n  def test; end\nend\n",
    ),
    (
        523,
        "class Test\n  something\n\n  %m\n\n  def test; end\nend\n",
        (0, 0),
        None,
        "",
    ),
    (545, "class Test\n  %m\nend\n", (0, 0), None, ""),
];

/// The `only_before` context: `public`/`module_function`.
const ONLY_BEFORE_OTHER: &[Pinned] = &[(
    555,
    "module Kernel\n  %m\n\n  def do_something\n  end\nend\n",
    (0, 0),
    None,
    "",
)];

/// The `only_before` context for all four, then the `Ruby 2.7` and `Ruby 3.4`
/// contexts.
const ONLY_BEFORE_ALL_AND_VERSIONS: &[Pinned] = &[
    (
        568,
        "class Test\n  something\n  %m\n  def test; end\nend\n",
        (3, 3),
        BA,
        "class Test\n  something\n\n  %m\n\n  def test; end\nend\n",
    ),
    (588, "%m\n", (0, 0), None, ""),
    (
        598,
        "included do\n  _1\n  %m\n  def test; end\nend\n",
        (3, 3),
        BA,
        "included do\n  _1\n\n  %m\n\n  def test; end\nend\n",
    ),
    (619, "def foo\n  %m { _1 }\nend\n", (0, 0), None, ""),
    (
        631,
        "included do\n  it\n  %m\n  def test; end\nend\n",
        (3, 3),
        BA,
        "included do\n  it\n\n  %m\n\n  def test; end\nend\n",
    ),
    (652, "def foo\n  %m { it }\nend\n", (0, 0), None, ""),
];

fn run(table: &[Pinned], modifiers: &[&str]) -> usize {
    let mut count = 0;
    for modifier in modifiers {
        for &(line, source, (l, c), kind, after) in table {
            let source = source.replace("%m", modifier);
            let expected: Vec<_> = kind.map(|after| (l, c, after)).into_iter().collect();
            assert_eq!(findings(&source), expected, "{modifier} spec line {line}");
            if kind.is_some() {
                let fixed = corrected(&source);
                if after.is_empty() {
                    assert_eq!(fixed, None, "{modifier} spec line {line}");
                } else {
                    let after = after.replace("%m", modifier);
                    assert_eq!(fixed.as_deref(), Some(after.as_str()), "{modifier} {line}");
                    assert_correction(&EmptyLinesAroundAccessModifier, &source, &after);
                }
            }
            count += 1;
        }
    }
    count
}

#[test]
fn pinned_examples() {
    let count = run(AROUND, &MODIFIERS)
        + run(ONLY_BEFORE_SPECIAL, &["private", "protected"])
        + run(ONLY_BEFORE_OTHER, &["public", "module_function"])
        + run(ONLY_BEFORE_ALL_AND_VERSIONS, &MODIFIERS);
    assert_eq!(count, 176);
}

#[test]
fn messages_name_the_modifier() {
    for modifier in MODIFIERS {
        let found = lint_str_with(
            &[&EmptyLinesAroundAccessModifier],
            &format!("foo\n{modifier}\nbar\nclass A\n  {modifier}\n  x\nend\n"),
        );
        let messages: Vec<_> = found.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                format!("Keep a blank line before and after `{modifier}`."),
                format!("Keep a blank line after `{modifier}`."),
            ]
        );
    }
}

fn check(source: &str, expected: &[(u32, u32, bool)], after: Option<&str>) {
    assert_eq!(findings(source), expected, "{source:?}");
    if expected.len() == 1 {
        assert_eq!(corrected(source).as_deref(), after, "{source:?}");
        if let Some(after) = after {
            assert_correction(&EmptyLinesAroundAccessModifier, source, after);
        }
    }
}

/// Upstream's whole-investigation state: the class and the block visited
/// last before the modifier, not the enclosing ones; a block's position is
/// its call's start.
#[test]
fn last_visited_class_and_block_state() {
    for (source, expected, after) in [
        // A one-line class just before: `class_def?` holds on its next line.
        (
            "class A\n  class B; end\n  private\n  def x; end\nend\n",
            &[(3, 3, true)][..],
            Some("class A\n  class B; end\n  private\n\n  def x; end\nend\n"),
        ),
        // The enclosing class does not count once another one was visited.
        (
            "class A\n  class B\n  end\n  x\n  private\n\n  def y; end\nend\n",
            &[(5, 3, false)],
            Some("class A\n  class B\n  end\n  x\n\n  private\n\n  def y; end\nend\n"),
        ),
        // ... and the last class's end decides `body_end?`.
        (
            "class A\n  class B\n    x\n\n    private\n  end\n  y\nend\n",
            &[],
            None,
        ),
        // A one-line block just before: `block_start?`.
        (
            "class A\n  foo { }\n  private\n  def x; end\nend\n",
            &[(3, 3, true)],
            Some("class A\n  foo { }\n  private\n\n  def x; end\nend\n"),
        ),
        // A block starts at its call, here on the line of `foo(`; as the
        // block's first statement nothing is inserted before it.
        (
            "foo(\n  bar) do\n  private\n\n  def x; end\nend\n",
            &[(3, 3, false)],
            None,
        ),
        // `super` blocks and lambdas are blocks too.
        (
            "-> do\n  private\n  x\nend\n",
            &[(2, 3, true)],
            Some("-> do\n  private\n\n  x\nend\n"),
        ),
        // A superclass or `self` on a later line is the class's first line.
        (
            "class A <\n    B\n  private\n  def x; end\nend\n",
            &[(3, 3, true)],
            Some("class A <\n    B\n  private\n\n  def x; end\nend\n"),
        ),
    ] {
        check(source, expected, after);
    }
}

/// EmptyLinesAroundBlockBody's fixed `no_empty_lines` applies to `block`
/// nodes only, not to numbered or `it` blocks.
#[test]
fn block_body_interplay() {
    for (source, expected, after) in [
        (
            "foo do\n  x\n  private\nend\n",
            &[(3, 3, false)][..],
            Some("foo do\n  x\n\n  private\nend\n"),
        ),
        (
            "foo { |a|\n  x\n  private\n}\n",
            &[(3, 3, false)],
            Some("foo { |a|\n  x\n\n  private\n}\n"),
        ),
        (
            "-> {\n  x\n  private\n}\n",
            &[(3, 3, false)],
            Some("-> {\n  x\n\n  private\n}\n"),
        ),
        (
            "foo do\n  _1\n  private\nend\n",
            &[(3, 3, false)],
            Some("foo do\n  _1\n\n  private\n\nend\n"),
        ),
        (
            "foo do\n  it\n  private\nend\n",
            &[(3, 3, false)],
            Some("foo do\n  it\n\n  private\n\nend\n"),
        ),
        // A middle statement gets both line breaks.
        (
            "foo do\n  x\n  private\n  y\nend\n",
            &[(3, 3, false)],
            Some("foo do\n  x\n\n  private\n\n  y\nend\n"),
        ),
        // A sole statement after a comment line: only the line break before.
        (
            "foo do # a\n  # b\n  private\nend\n",
            &[(3, 3, false)],
            Some("foo do # a\n  # b\n\n  private\nend\n"),
        ),
    ] {
        check(source, expected, after);
    }
}

#[test]
fn scope_comments_and_lines() {
    for (source, expected, after) in [
        // Not bare or not in a macro scope.
        ("def f\n  x\n  private\n  y\nend\n", &[][..], None),
        ("class A\n  x\n  private :y\n  z\nend\n", &[], None),
        ("class A\n  x\n  self.private\n  z\nend\n", &[], None),
        (
            "class A\n  x\n  private()\n  z\nend\n",
            &[(3, 3, false)],
            Some("class A\n  x\n\n  private()\n\n  z\nend\n"),
        ),
        ("begin\n  x\n  private\n  y\nrescue\nend\n", &[], None),
        ("case x\nwhen 1\n  y\n  private\n  z\nend\n", &[], None),
        // A branch in a class body is a macro scope.
        (
            "class A\n  if c\n    x\n    private\n    y\n  end\nend\n",
            &[(4, 5, false)],
            Some("class A\n  if c\n    x\n\n    private\n\n    y\n  end\nend\n"),
        ),
        // Comment lines are skipped upward; a trailing comment is code.
        (
            "class A\n  x\n\n  # a\n  # b\n  private\n\n  y\nend\n",
            &[],
            None,
        ),
        (
            "class A\n  x # a\n  private\n\n  y\nend\n",
            &[(3, 3, false)],
            Some("class A\n  x # a\n\n  private\n\n  y\nend\n"),
        ),
        // Line 1 reads upward from the last line: blank with a final line
        // break, the code line without one (the edit before line 1 is held).
        ("private\nfoo\n", &[(1, 1, false)], Some("private\n\nfoo\n")),
        ("private\nfoo", &[(1, 1, false)], None),
        (
            "# a\nprivate\nfoo\n",
            &[(2, 1, false)],
            Some("# a\nprivate\n\nfoo\n"),
        ),
        // A whitespace-only line is blank; CRLF keeps its line ending.
        ("class A\n  x\n \t\n  private\n  \n  y\nend\n", &[], None),
        (
            "class A\r\n  x\r\n  private\r\n  y\r\nend\r\n",
            &[(3, 3, false)],
            Some("class A\r\n  x\r\n\r\n  private\r\n\r\n  y\r\nend\r\n"),
        ),
        // The next statement on the modifier's line skips it.
        ("class A\n  x\n  private; def y; end\nend\n", &[], None),
        // The line before `__END__` is the last code line.
        (
            "x\nprivate\n__END__\ny\n",
            &[(2, 1, false)],
            Some("x\n\nprivate\n__END__\ny\n"),
        ),
    ] {
        check(source, expected, after);
    }
}

/// The state is per file: a class seen in one file never reaches the next,
/// and a corrected file is re-read with its new lines.
#[test]
fn state_is_built_per_source() {
    let class_first = "class A\nend\n";
    assert!(findings(class_first).is_empty());
    // With a leaked `class_def?` (line 1 + 1) this would be the after-only
    // message.
    assert_eq!(findings("x\nprivate\ny\n"), [(2, 1, false)]);
    let file = crate::source::SourceFile::new("x.rb".into(), b"x\nprivate\ny\n".to_vec());
    let parsed = crate::parser::parse(&file.bytes);
    let root = parsed.node();
    let scopes = file.access_modifier_scopes(&root);
    assert!(std::ptr::eq(scopes, file.access_modifier_scopes(&root)));
    assert_eq!(scopes.at.len(), 1);
}

#[test]
fn metadata_is_default_on_and_safe() {
    let cop = EmptyLinesAroundAccessModifier;
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert!(cop.department_selection_enables());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
}
