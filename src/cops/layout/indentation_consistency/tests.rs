//! Examples ported from RuboCop's `spec/rubocop/cop/layout/indentation_consistency_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every
//! example, including the two `indented_internal_methods` examples, is replayed
//! at the fixed `normal` style; findings, messages and single-pass corrected
//! bytes match the fixed replay recorded in the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str,
};

const COP: &str = "Layout/IndentationConsistency";
const MSG: &str = "Inconsistent indentation detected.";

type Reported = (u32, &'static str, &'static [(u32, u32)], &'static str);

fn positions(source: &str) -> Vec<(u32, u32)> {
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            (o.line, o.col)
        })
        .collect()
}

/// Every pinned example without an offense at the fixed defaults.
const ACCEPTED: &[(u32, &str)] = &[
    (8, "\"#{}\"\n"),
    (14, "public\n\ndef foo\nend\n"),
    (106, "if cond then func1 else func2 end"),
    (110, "if a1\n  b1\nelsif a2\n  b2\nelse\n  c\nend\n"),
    (
        122,
        "if    @io == $stdout then str << \"$stdout\"\nelsif @io == $stdin  then str << \"$stdin\"\nelsif @io == $stderr then str << \"$stderr\"\nelse                      str << @io.class.to_s\nend\n",
    ),
    (
        132,
        "if File.exist?('config.save')\nthen ConfigTable.load\nelse ConfigTable.new\nend\n",
    ),
    (
        141,
        "p 'Ｒｕｂｙ', if a then b\n                        c\n              end\n",
    ),
    (149, "if a\nelse\nend\n"),
    (
        157,
        "var = if a\n  0\nend\n@var = if a\n  0\nend\n$var = if a\n  0\nend\nvar ||= if a\n  0\nend\nvar &&= if a\n  0\nend\nvar -= if a\n  0\nend\nVAR = if a\n  0\nend\n",
    ),
    (183, "var = if a\n  0\nelse\n  1\nend\n"),
    (194, "var = if a\n  0\nelse\n  1\nend.abc.join(\"\")\n"),
    (205, "var = if a\n  0\nelse\n  1\nend.abc.tap {}\n"),
    (215, "var = if a\n        0\n      end\n"),
    (
        223,
        "var = if a\n        0\n      else\n        1\n      end\n",
    ),
    (233, "var =\n  if a\n    0\n  else\n    1\n  end\n"),
    (246, "if a\n  a rescue nil\nelse\n  a rescue nil\nend\n"),
    (275, "unless a\nelse\nend\n"),
    (331, "case a\nwhen b\n  c\n  c\nwhen d\nelse\n  f\nend\n"),
    (
        344,
        "case sexp.loc.keyword.source\nwhen 'if'     then cond, body, _else = *sexp\nwhen 'unless' then cond, _else, body = *sexp\nelse               cond, body = *sexp\nend\n",
    ),
    (
        354,
        "case sexp.loc.keyword.source\nwhen 'if'\nthen cond, body, _else = *sexp\nend\n",
    ),
    (
        364,
        "case code_type\n  when 'ruby', 'sql', 'plain'\n    code_type\n  when 'erb'\n    'ruby; html-script: true'\n  when \"html\"\n    'xml'\n  else\n    'plain'\nend\n",
    ),
    (432, "while a\nend\n"),
    (458, "for var in 1..10\nend\n"),
    (501, "def test\nend\n"),
    (508, "def self.test\nend\n"),
    (615, "class Test\nend\n"),
    (
        622,
        "class Test\n  public\n\n  def e\n  end\n\n  protected\n\n  def f\n  end\n\n  private\n\n  def g\n  end\nend\n",
    ),
    (709, "module Test\nend\n"),
    (796, "a = func do\n  b\n  c\nend\n"),
    (805, "a = func do\nend\n"),
];

/// Every pinned example with offenses: (spec line, source, findings, single-pass correction).
const REPORTED: &[Reported] = &[
    (
        24,
        "public\n\n  def foo\n  end\n",
        &[(3, 3)],
        "public\n\ndef foo\nend\n",
    ),
    (
        43,
        "if cond\n func\n  func\nend\n",
        &[(3, 3)],
        "if cond\n func\n func\nend\n",
    ),
    (
        60,
        "if cond\n  func1\nelse\n func2\n  func2\nend\n",
        &[(5, 3)],
        "if cond\n  func1\nelse\n func2\n func2\nend\n",
    ),
    (
        81,
        "if a1\n  b1\nelsif a2\n b2\nb3\nelse\n  c\nend\n",
        &[(5, 1)],
        "if a1\n  b1\nelsif a2\n b2\n b3\nelse\n  c\nend\n",
    ),
    (
        258,
        "unless cond\n func\n  func\nend\n",
        &[(3, 3)],
        "unless cond\n func\n func\nend\n",
    ),
    (
        285,
        "case a\nwhen b\n c\n    d\nend\n",
        &[(4, 5)],
        "case a\nwhen b\n c\n d\nend\n",
    ),
    (
        304,
        "case a\nwhen b\n  c\nwhen d\n  e\nelse\n   f\n  g\nend\n",
        &[(8, 3)],
        "case a\nwhen b\n  c\nwhen d\n  e\nelse\n   f\n   g\nend\n",
    ),
    (
        381,
        "while cond\n func\n  func\nend\n",
        &[(3, 3)],
        "while cond\n func\n func\nend\n",
    ),
    (
        398,
        "something = begin\n func1\n   func2\nend while cond\n",
        &[(3, 4)],
        "something = begin\n func1\n func2\nend while cond\n",
    ),
    (
        415,
        "until cond\n func\n  func\nend\n",
        &[(3, 3)],
        "until cond\n func\n func\nend\n",
    ),
    (
        441,
        "for var in 1..10\n func\nfunc\nend\n",
        &[(3, 1)],
        "for var in 1..10\n func\n func\nend\n",
    ),
    (
        467,
        "def test\n    func1\n     func2\nend\n",
        &[(3, 6)],
        "def test\n    func1\n    func2\nend\n",
    ),
    (
        484,
        "def self.test\n   func\n    func\nend\n",
        &[(3, 5)],
        "def self.test\n   func\n   func\nend\n",
    ),
    (
        520,
        "class Cat\n  def meow\n    puts('Meow!')\n  end\n\n  protected\n\n    def can_we_be_friends?(another_cat)\n      # some logic\n    end\n\n    def related_to?(another_cat)\n      # some logic\n    end\n\n  private\n\n              # Here we go back an indentation level again. This is a\n              # violation of the indented_internal_methods style,\n              # but it's not for this cop to report.\n              # Layout/IndentationWidth will handle it.\n  def meow_at_3am?\n    rand < 0.8\n  end\n\n              # As long as the indentation of this method is\n              # consistent with that of the last one, we're fine.\n  def meow_at_4am?\n    rand < 0.8\n  end\nend\n",
        &[(8, 5), (12, 5)],
        "class Cat\n  def meow\n    puts('Meow!')\n  end\n\n  protected\n\n  def can_we_be_friends?(another_cat)\n    # some logic\n  end\n\n  def related_to?(another_cat)\n    # some logic\n  end\n\n  private\n\n              # Here we go back an indentation level again. This is a\n              # violation of the indented_internal_methods style,\n              # but it's not for this cop to report.\n              # Layout/IndentationWidth will handle it.\n  def meow_at_3am?\n    rand < 0.8\n  end\n\n              # As long as the indentation of this method is\n              # consistent with that of the last one, we're fine.\n  def meow_at_4am?\n    rand < 0.8\n  end\nend\n",
    ),
    (
        556,
        "Foo = Struct.new(:foo) do\n  def meow\n    puts('Meow!')\n  end\n\n  protected\n\n    def can_we_be_friends?(another_cat)\n      # some logic\n    end\n\n    def related_to?(another_cat)\n      # some logic\n    end\n\n  private\n\n              # Here we go back an indentation level again. This is a\n              # violation of the indented_internal_methods style,\n              # but it's not for this cop to report.\n              # Layout/IndentationWidth will handle it.\n  def meow_at_3am?\n    rand < 0.8\n  end\n\n              # As long as the indentation of this method is\n              # consistent with that of the last one, we're fine.\n  def meow_at_4am?\n    rand < 0.8\n  end\nend\n",
        &[(8, 5), (12, 5)],
        "Foo = Struct.new(:foo) do\n  def meow\n    puts('Meow!')\n  end\n\n  protected\n\n  def can_we_be_friends?(another_cat)\n    # some logic\n  end\n\n  def related_to?(another_cat)\n    # some logic\n  end\n\n  private\n\n              # Here we go back an indentation level again. This is a\n              # violation of the indented_internal_methods style,\n              # but it's not for this cop to report.\n              # Layout/IndentationWidth will handle it.\n  def meow_at_3am?\n    rand < 0.8\n  end\n\n              # As long as the indentation of this method is\n              # consistent with that of the last one, we're fine.\n  def meow_at_4am?\n    rand < 0.8\n  end\nend\n",
    ),
    (
        594,
        "class Test\n    def func1\n    end\n  def func2\n  end\nend\n",
        &[(4, 3)],
        "class Test\n    def func1\n    end\n    def func2\n    end\nend\n",
    ),
    (
        644,
        "class Test\npublic\n\n  def e\n  end\n\nprotected\n\n  def f\n  end\n\nprivate\n\n def g\n end\nend\n",
        &[(14, 2)],
        "class Test\npublic\n\n  def e\n  end\n\nprotected\n\n  def f\n  end\n\nprivate\n\n  def g\n  end\nend\n",
    ),
    (
        688,
        "module Test\n    def func1\n    end\n     def func2\n     end\nend\n",
        &[(4, 6)],
        "module Test\n    def func1\n    end\n    def func2\n    end\nend\n",
    ),
    (
        716,
        "module Test\n  def pub\n  end\n  private\n    def priv\n    end\nend\n",
        &[(5, 5)],
        "module Test\n  def pub\n  end\n  private\n  def priv\n  end\nend\n",
    ),
    (
        740,
        "module Test\n  private\n    def priv\n    end\nend\n",
        &[(3, 5)],
        "module Test\n  private\n  def priv\n  end\nend\n",
    ),
    (
        762,
        "a = func do\n b\n  c\nend\n",
        &[(3, 3)],
        "a = func do\n b\n c\nend\n",
    ),
    (
        779,
        "func {\n   b\n  c\n}\n",
        &[(3, 3)],
        "func {\n   b\n   c\n}\n",
    ),
    (
        812,
        "require 'spec_helper'\ndescribe ArticlesController do\n  render_views\n    describe \"GET 'index'\" do\n            it \"returns success\" do\n            end\n        describe \"admin user\" do\n             before(:each) do\n            end\n        end\n    end\nend\n",
        &[(4, 5), (7, 9)],
        "require 'spec_helper'\ndescribe ArticlesController do\n  render_views\n  describe \"GET 'index'\" do\n          it \"returns success\" do\n          end\n      describe \"admin user\" do\n           before(:each) do\n          end\n      end\n  end\nend\n",
    ),
];

#[test]
fn pinned_accepted_examples() {
    for (line, source) in ACCEPTED {
        assert!(positions(source).is_empty(), "spec line {line}: {source}");
    }
}

#[test]
fn pinned_reported_examples_and_single_pass_corrections() {
    for (line, source, expected, after) in REPORTED {
        assert_eq!(positions(source), *expected, "spec line {line}: {source}");
        assert_correction(&IndentationConsistency, source, after);
    }
}

#[test]
fn metadata_is_fixed() {
    let cop = IndentationConsistency;
    assert_eq!(cop.name(), COP);
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert!(cop.autocorrect_incompatible_with().is_empty());
}

#[test]
fn kwbegin_parentheses_and_rescue_bodies_check_direct_siblings() {
    for (source, expected, after) in [
        ("begin\n  a\n   b\nend\n", (3, 4), "begin\n  a\n  b\nend\n"),
        (
            "x = begin\n  a\n   b\nrescue\n  c\nend\n",
            (3, 4),
            "x = begin\n  a\n  b\nrescue\n  c\nend\n",
        ),
        (
            "begin\n  a\nrescue\n  b\n c\nensure\n  d\nend\n",
            (5, 2),
            "begin\n  a\nrescue\n  b\n  c\nensure\n  d\nend\n",
        ),
        ("x = (\n  a\n b\n)\n", (3, 2), "x = (\n  a\n  b\n)\n"),
        (
            "def f\n  a\n   b\nrescue\n  c\nend\n",
            (3, 4),
            "def f\n  a\n  b\nrescue\n  c\nend\n",
        ),
        (
            "foo do\n  a\n    b\nend\n",
            (3, 5),
            "foo do\n  a\n  b\nend\n",
        ),
        (
            "case x\nin 1\n  a\n b\nend\n",
            (4, 2),
            "case x\nin 1\n  a\n  b\nend\n",
        ),
        ("-> {\n  a\n b\n}\n", (3, 2), "-> {\n  a\n  b\n}\n"),
        ("BEGIN {\n  a\n b\n}\n", (3, 2), "BEGIN {\n  a\n  b\n}\n"),
    ] {
        assert_eq!(positions(source), [expected], "{source}");
        assert_correction(&IndentationConsistency, source, after);
    }
}

#[test]
fn single_statements_and_same_line_statements_are_not_items() {
    for source in [
        "begin\n  a\nend\n",
        "x = (a)\n",
        "def f\n    a\nend\n",
        "a; b\nc\n",
        "a = 1; b = 2\n\nc = 3\n",
        "if x\n  a; b\n  c\nend\n",
        "foo(<<~A)\n  body\nA\nbar\n",
        "x = \"#{a; b}\"\n",
        "  # comment\nfoo\n",
    ] {
        assert!(positions(source).is_empty(), "{source}");
    }
    // The second statement on the first item's line is skipped; the next
    // line is still compared against the first item.
    assert_eq!(positions("a; b\n c\n"), [(2, 2)]);
}

#[test]
fn the_first_statement_need_not_begin_its_line() {
    let source = "def f; a\n  b\nend\n";
    assert_eq!(positions(source), [(2, 3)]);
    assert_correction(&IndentationConsistency, source, "def f; a\n       b\nend\n");
    // A later statement that does not begin its line is never checked.
    assert!(positions("def f\n  a\n  (b; c\n  )\nend\n").is_empty());
}

#[test]
fn leading_bare_access_modifiers_set_the_base_when_indented_deeper() {
    for (source, expected, after) in [
        (
            "class A\n    private\n  def f; end\nend\n",
            (3, 3),
            "class A\n    private\n    def f; end\nend\n",
        ),
        (
            "foo do\n  private\n    def f; end\nend\n",
            (3, 5),
            "foo do\n  private\n  def f; end\nend\n",
        ),
        (
            "x = Class.new do\n      private\n    def f; end\nend\n",
            (3, 5),
            "x = Class.new do\n      private\n      def f; end\nend\n",
        ),
        (
            "begin\n  private\n    def f; end\nend\n",
            (3, 5),
            "begin\n  private\n  def f; end\nend\n",
        ),
        ("  public\ndef f; end\n", (2, 1), "  public\n  def f; end\n"),
        (
            "class A\n  if x\n    protected\n  def f; end\n  end\nend\n",
            (4, 3),
            "class A\n  if x\n    protected\n    def f; end\n  end\nend\n",
        ),
        (
            "class A\n  if x\n  else\n    protected\n  def f; end\n  end\nend\n",
            (5, 3),
            "class A\n  if x\n  else\n    protected\n    def f; end\n  end\nend\n",
        ),
    ] {
        assert_eq!(positions(source), [expected], "{source}");
        assert_correction(&IndentationConsistency, source, after);
    }
    // Outdented to (or beyond) the parser parent: the first other statement
    // is the base.
    for source in [
        "class A\nprivate\n  def f; end\n  def g; end\nend\n",
        "  class A\n  private\n    def f; end\n  end\n",
        "foo do\nprivate\n  def f; end\nend\n",
        "  x = Class.new do\n  private\n    def f; end\n  end\n",
    ] {
        assert!(positions(source).is_empty(), "{source}");
    }
}

#[test]
fn modifier_lookalikes_are_ordinary_items() {
    for (source, expected) in [
        ("class A\n  def a; end\n    private :a\nend\n", (3, 5)),
        (
            "class A\n  def a; end\n    private def b; end\nend\n",
            (3, 5),
        ),
        ("class A\n  def a; end\n    self.private\nend\n", (3, 5)),
        ("def f\n  a\n    private\nend\n", (3, 5)),
        ("class A\n    private()\n  def f; end\nend\n", (3, 3)),
    ] {
        assert_eq!(positions(source), [expected], "{source}");
    }
    // Bare modifiers anywhere in the list are skipped, not compared.
    assert!(positions("class A\n  def a; end\n    private\n  def b; end\nend\n").is_empty());
    assert!(positions("class A\n  def a; end\nprotected\n  def b; end\nend\n").is_empty());
}

#[test]
fn display_columns_crlf_and_bom() {
    let source = "if a\r\n  b\r\n   c\r\nend\r\n";
    assert_eq!(positions(source), [(3, 4)]);
    assert_correction(
        &IndentationConsistency,
        source,
        "if a\r\n  b\r\n  c\r\nend\r\n",
    );
    let source = "\u{feff}a\n b\n";
    assert_eq!(positions(source), [(2, 2)]);
    assert_correction(&IndentationConsistency, source, "\u{feff}a\nb\n");
    // Fullwidth characters are two display columns wide.
    let source = format!("x = ['Ｒｕ', if a then b\n{}c\nend]\n", " ".repeat(23));
    assert!(positions(&source).is_empty(), "{source}");
    let source = format!("x = ['Ｒｕ', if a then b\n{}c\nend]\n", " ".repeat(21));
    assert_eq!(positions(&source), [(2, 22)]);
}

#[test]
fn nested_offenses_inside_an_offense_are_reported_without_an_edit() {
    let source = "def f\n  a\n    foo do\n      b\n     c\n    end\nend\n";
    assert_eq!(positions(source), [(3, 5), (5, 6)]);
    let found = correction_offenses(&IndentationConsistency, source);
    assert!(found[0].correction.is_some() && found[1].correction.is_none());
    assert_correction(
        &IndentationConsistency,
        source,
        "def f\n  a\n  foo do\n    b\n   c\n  end\nend\n",
    );
    // A misaligned sibling elsewhere does not hold an unrelated edit.
    let source = "def f\n  a\n   b\nend\ndef g\n  c\n d\nend\n";
    let found = correction_offenses(&IndentationConsistency, source);
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|o| o.correction.is_some()));
}

#[test]
fn literal_text_is_never_shifted() {
    // Heredoc bodies keep their bytes; the opener line moves.
    let source = "def f\n  a\n   x = <<~E\n    body\n   E\nend\n";
    assert_eq!(positions(source), [(3, 4)]);
    assert_correction(
        &IndentationConsistency,
        source,
        "def f\n  a\n  x = <<~E\n    body\n   E\nend\n",
    );
    let source = "def f\n  a\n   x = \"one\n   two\"\nend\n";
    assert_correction(
        &IndentationConsistency,
        source,
        "def f\n  a\n  x = \"one\n   two\"\nend\n",
    );
    // A statement inside interpolation starts inside literal text: the
    // finding stays and no edit is made.
    let source = "x = \"#{a\n   b}\"\n";
    assert_eq!(positions(source), [(2, 4)]);
    let found = correction_offenses(&IndentationConsistency, source);
    assert!(found[0].correction.is_none());
    // A block comment inside the shifted range holds the edit.
    let source = "def f\n  a\n   foo(\n=begin\n=end\n   )\nend\n";
    assert_eq!(positions(source), [(3, 4)]);
    let found = correction_offenses(&IndentationConsistency, source);
    assert!(found[0].correction.is_none());
}

#[test]
fn listed_source_observers_hold_the_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p $PROGRAM_NAME",
        "p caller",
        "p caller_locations",
        "p method(:f).source_location",
        "SCRIPT_LINES__ = {}",
        "p DATA.read\n__END__\npayload",
    ] {
        let source = format!("def f\n  a\n   b\nend\n{observer}\n");
        let found = correction_offenses(&IndentationConsistency, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
    // Out-of-scope readers do not hold it.
    let source = "def f\n  a\n   b\nend\np File.read('example.rb')\n";
    assert_correction(
        &IndentationConsistency,
        source,
        "def f\n  a\n  b\nend\np File.read('example.rb')\n",
    );
}

/// In a class, module or singleton class body, `Layout/IndentationWidth`
/// realigns every member to the keyword column plus two. When the base is not
/// indented past the keyword, realigning an item that is already there would
/// move it against IndentationWidth's realignment of the base in the same
/// pass, swapping them; that item's edit is withheld and its finding stays.
#[test]
fn items_at_the_body_width_defer_to_indentation_width_of_the_base() {
    use crate::cops::layout::IndentationWidth;
    let edits = |source: &str| -> Vec<bool> {
        correction_offenses(&IndentationConsistency, source)
            .iter()
            .map(|o| o.correction.is_some())
            .collect()
    };
    for source in [
        "class Foo\nattr :a\n  attr :b\n  attr :c\nend\n",
        "module Foo\na\n  b\nend\n",
        "class << self\na\n  b\nend\n",
        "  class Foo\n  a\n    b\n  end\n",
        "  module Foo\na\n    b\n  end\n",
    ] {
        let found = correction_offenses(&IndentationConsistency, source);
        assert!(
            !found.is_empty() && found.iter().all(|o| o.correction.is_none()),
            "{source}"
        );
        // Together with IndentationWidth one pass makes the body consistent.
        let found = crate::pipeline::lint_str_with_corrections(
            &[&IndentationConsistency, &IndentationWidth],
            source,
        );
        let fixed = crate::corrections::apply(source.as_bytes(), &found)
            .unwrap()
            .bytes;
        let fixed = String::from_utf8(fixed).unwrap();
        assert!(positions(&fixed).is_empty(), "{source} -> {fixed}");
    }
    // A base indented past the keyword (the pinned corrections), items
    // elsewhere, a base at the body width and other bodies still correct; a
    // body on the keyword line is not checked by IndentationWidth.
    assert_eq!(edits("module Foo\n    a\n  b\nend\n"), [true]);
    assert_eq!(edits("class Foo\n a\n  b\nend\n"), [true]);
    assert_eq!(edits("class Foo\nattr :a\n   attr :b\nend\n"), [true]);
    assert_eq!(edits("class Foo\n  attr :a\nattr :b\nend\n"), [true]);
    assert_eq!(edits("def f\na\n  b\nend\n"), [true]);
    assert_eq!(edits("foo do\na\n  b\nend\n"), [true]);
    assert_eq!(edits("class Foo; a\n  b\nend\n"), [true]);
    assert_eq!(
        edits("class Foo\n  def f\n  a\n    b\n  end\nend\n"),
        [true]
    );
}
