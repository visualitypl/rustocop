//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/class_structure_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 52 pinned
//! examples, replayed individually at the fixed defaults: findings and
//! messages equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source SHA-256 was checked
//! against it when the rows were ported). The spec configures a custom
//! `ExpectedOrder`/`Categories`; rustocop has no such knob, so six examples
//! (lines 325, 345, 945, 1055, 1109 and 1193) expect findings only for the
//! custom `attribute_macros`, `macros` or `all_constants` categories and
//! have none at the fixed defaults, as the replay records. Every
//! `expect_correction` is out of scope: upstream's reorder is
//! `SafeAutoCorrect: false` and the cop is detection-only.
use super::*;
use crate::pipeline::{lint_str_with, lint_str_with_corrections};

const COP: &str = "Layout/ClassStructure";

fn findings(source: &str) -> Vec<(u32, u32, String)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&ClassStructure], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col, o.message)
        })
        .collect();
    found.sort_unstable();
    found
}

fn msg(category: &str, previous: &str) -> String {
    format!("`{category}` is supposed to appear before `{previous}`.")
}

/// `(spec line, source, findings)`.
type Pinned = (u32, &'static str, &'static [(u32, u32, &'static str)]);

const PINNED: &[Pinned] = &[
    (
        45,
        r#"class GridTask
  DESC = 'Grid Task' # grid task name OID, subclasses should set this
  extend Helpers::MakeFromFile
end
"#,
        &[(
            3,
            3,
            r#"`module_inclusion` is supposed to appear before `constants`."#,
        )],
    ),
    (
        64,
        r#"class Person
  # extend and include go first
  extend SomeModule
  include AnotherModule

  # inner classes
  CustomError = Class.new(StandardError)

  # constants are next
  SOME_CONSTANT = 20

  # afterwards we have attribute macros
  attr_reader :name

  # then we have public delegate macros
  delegate :to_s, to: :name

  # followed by other macros (if any)
  validates :name

  # public class methods are next in line
  def self.some_method
  end

  # initialization goes between class methods and other instance methods
  def initialize
  end

  # followed by other public instance methods
  def some_method
  end

  # protected attribute macros and methods go next
  protected

  attr_reader :protected_name

  def some_protected_method
  end

  # private attribute macros, delegate macros and methods are grouped near the end
  private

  attr_reader :private_name

  delegate :some_private_delegate, to: :name

  def some_private_method
  end
end
"#,
        &[],
    ),
    (
        121,
        r#"class Person
  CONST = 'wrong place'
  include AnotherModule
  extend SomeModule
end
"#,
        &[(
            3,
            3,
            r#"`module_inclusion` is supposed to appear before `constants`."#,
        )],
    ),
    (
        142,
        r#"class A
  test&.private_methods(def foo; end)
end
"#,
        &[],
    ),
    (
        150,
        r#"class A
  def foo; end if bar
end
"#,
        &[],
    ),
    (
        158,
        r#"class A
  configure { |c| c.foo = 1 }
end
"#,
        &[],
    ),
    (
        168,
        r#"class Foo
  begin
    private def do_internal_work; end
    public def do_something; end
  end
end
"#,
        &[(
            4,
            5,
            r#"`public_methods` is supposed to appear before `private_methods`."#,
        )],
    ),
    (
        189,
        r#"class Foo
  begin
    begin
      private def do_internal_work; end
      public def do_something; end
    end
  end
end
"#,
        &[(
            5,
            7,
            r#"`public_methods` is supposed to appear before `private_methods`."#,
        )],
    ),
    (
        214,
        r#"class Foo
  begin
    public def do_something; end
    private def do_internal_work; end
  end
end
"#,
        &[],
    ),
    (
        225,
        r#"class Foo
  begin
    require 'optional_dependency'
  rescue LoadError
    nil
  end
end
"#,
        &[],
    ),
    (
        238,
        r#"class Foo
  def instance_method; end
  def self.class_method; end
end
"#,
        &[(
            3,
            3,
            r#"`public_class_methods` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        279,
        r#"class MyClass
  def public_method
  end

  private

  def first_private_method
  end

  def second_private_method
  end

  protected

  def first_protected_method
  end

  def second_protected_method
  end
end
"#,
        &[(
            15,
            3,
            r#"`protected_methods` is supposed to appear before `private_methods`."#,
        )],
    ),
    (
        325,
        r#"class Person
  include AnotherModule
  extend SomeModule

  CustomError = Class.new(StandardError)

  validates :name

  attr_reader :name

  def self.some_public_class_method
  end

  def initialize
  end

  def some_public_method
  end


  def yet_other_public_method
  end

  protected

  def some_protected_method
  end

  def other_public_method
  end

  private :other_public_method

  private

  def some_private_method
  end
end
"#,
        &[],
    ),
    (
        330,
        r#"class Person
  def name; end

  foo = 5
  LIMIT = foo + 1
end
"#,
        &[(
            5,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        345,
        r#"class Foo
  # This is a comment for macro method.
  validates :attr
  attr_reader :foo
end
"#,
        &[],
    ),
    (
        364,
        r#"class Foo
  def name; end

  LIMIT = 10
  CONST = 'wrong place'.freeze
  RECURSIVE_BASIC_LITERALS_CONST = [1, 2].freeze
  DYNAMIC_CONST = foo.freeze
end
"#,
        &[(
            4,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        389,
        r#"class Foo
  def name; end

  PRIVATE_CONST1 = 1
  PRIVATE_CONST2 = 2
  private_constant :PRIVATE_CONST1, :PRIVATE_CONST2
  PUBLIC_CONST = 'public'
end
"#,
        &[(
            7,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        414,
        r#"class Foo
  def do_something
  end

  CONSTANT = <<~EOS
    str
  EOS
end
"#,
        &[(
            5,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        439,
        r#"class Foo
  def do_something
  end

  CONSTANT = <<~EOS
    #{str}
  EOS
end
"#,
        &[(
            5,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        464,
        r#"class Foo
  def do_something
  end

  CONSTANT = <<~`EOS`
    str
  EOS
end
"#,
        &[(
            5,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        489,
        r#"class Foo
  def do_something
  end
  CONSTANT = <<~EOS
    str
  EOS
end
"#,
        &[(
            4,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        512,
        r#"class Foo
  def do_something
  end
  CONSTANT = <<~EOS
    str
  EOS
  def do_something_else
  end
end
"#,
        &[(
            4,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        539,
        r#"class Foo
  def do_something
  end

  CONSTANT = {
    foo: <<~EOS,
      str
    EOS
    bar: 1
  }.freeze
end
"#,
        &[(
            5,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        570,
        r#"class Foo
  def do_something
  end

  include Module.new(<<~EOS)
    str
  EOS
end
"#,
        &[(
            5,
            3,
            r#"`module_inclusion` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        595,
        r#"class Foo
  def instance_method
    'instance method'
  end

  def self.class_method
    <<~EOS
      class method
    EOS
  end
end
"#,
        &[(
            6,
            3,
            r#"`public_class_methods` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        627,
        r#"class A
  private def foo
  end

  public def bar
  end
end
"#,
        &[(
            5,
            3,
            r#"`public_methods` is supposed to appear before `private_methods`."#,
        )],
    ),
    (
        650,
        r#"class A
  private def foo
  end

  def bar
  end
end
"#,
        &[(
            5,
            3,
            r#"`public_methods` is supposed to appear before `private_methods`."#,
        )],
    ),
    (
        673,
        r#"class A
  private def foo; end

  def bar; end

  private def baz; end

  def qux; end
end
"#,
        &[
            (
                4,
                3,
                r#"`public_methods` is supposed to appear before `private_methods`."#,
            ),
            (
                8,
                3,
                r#"`public_methods` is supposed to appear before `private_methods`."#,
            ),
        ],
    ),
    (
        701,
        r#"class A
  private_class_method def self.do_internal_work
  end

  public_class_method def self.do_something
  end
end
"#,
        &[(
            5,
            3,
            r#"`public_class_methods` is supposed to appear before `private_class_methods`."#,
        )],
    ),
    (
        724,
        r#"class A
  public_class_method def self.do_something
  end

  private_class_method def self.do_internal_work
  end
end
"#,
        &[],
    ),
    (
        736,
        r#"class A
  private

  def do_something
  end

  private_class_method def self.do_internal_work
  end
end
"#,
        &[(
            7,
            3,
            r#"`private_class_methods` is supposed to appear before `private_methods`."#,
        )],
    ),
    (
        763,
        r#"class A
  private

  private_class_method def self.do_internal_work
  end

  def do_something
  end
end
"#,
        &[],
    ),
    (
        777,
        r#"class A
  def foo
  end
  private :foo

  def bar
  end
end
"#,
        &[(
            6,
            3,
            r#"`public_methods` is supposed to appear before `private_methods`."#,
        )],
    ),
    (
        802,
        r#"class A
  def foo
  end

  def baz
  end
  private :foo, :baz

  def bar
  end
end
"#,
        &[(
            9,
            3,
            r#"`public_methods` is supposed to appear before `private_methods`."#,
        )],
    ),
    (
        833,
        r#"class A
  def bar
  end

  def foo
  end
  private :foo, :baz
end
"#,
        &[],
    ),
    (
        846,
        r#"class A
  def self.do_internal_work
  end
  private_class_method :do_internal_work

  def self.do_something
  end
end
"#,
        &[(
            6,
            3,
            r#"`public_class_methods` is supposed to appear before `private_class_methods`."#,
        )],
    ),
    (
        871,
        r#"class A
  def self.first_internal
  end

  def self.second_internal
  end
  private_class_method :first_internal, :second_internal

  def self.do_something
  end
end
"#,
        &[(
            9,
            3,
            r#"`public_class_methods` is supposed to appear before `private_class_methods`."#,
        )],
    ),
    (
        902,
        r#"class A
  def self.do_something
  end

  def self.do_internal_work
  end
  private_class_method :do_internal_work
end
"#,
        &[],
    ),
    (
        915,
        r#"class A
  def self.do_something
  end
  private_class_method :do_something
  public_class_method :do_something

  def do_other_thing
  end
end
"#,
        &[],
    ),
    (
        929,
        r#"class A
  private

  def self.still_public
  end

  def instance_method
  end
end
"#,
        &[],
    ),
    (
        945,
        r#"class A
  private

  attr_accessor :foo

  def initialize
  end
end
"#,
        &[],
    ),
    (
        972,
        r#"class Foo
  def do_something; end
  include M
  CONST = 1
end
"#,
        &[
            (
                3,
                3,
                r#"`module_inclusion` is supposed to appear before `public_methods`."#,
            ),
            (
                4,
                3,
                r#"`constants` is supposed to appear before `public_methods`."#,
            ),
        ],
    ),
    (
        992,
        r#"class Foo
  def self.do_something; end

  def initialize; end

  include M
  CONST = 1
end
"#,
        &[
            (
                6,
                3,
                r#"`module_inclusion` is supposed to appear before `initializer`."#,
            ),
            (
                7,
                3,
                r#"`constants` is supposed to appear before `initializer`."#,
            ),
        ],
    ),
    (
        1020,
        r#"class Foo
  def do_something; end
  FIRST = 1
  SECOND = 2
end
"#,
        &[(
            3,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        1041,
        r#"class Foo
  def do_something; end
  CONST = [*foo].freeze
  include M
end
"#,
        &[
            (
                3,
                3,
                r#"`constants` is supposed to appear before `public_methods`."#,
            ),
            (
                4,
                3,
                r#"`module_inclusion` is supposed to appear before `public_methods`."#,
            ),
        ],
    ),
    (
        1055,
        r#"class Foo
  validates :name
  CONST = do_something.freeze
  attr_reader :foo
end
"#,
        &[],
    ),
    (
        1069,
        r#"class Foo
  def do_something; end
  FIRST = 1
  DYNAMIC = do_something.freeze
  SECOND = 2
end
"#,
        &[(
            3,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        1092,
        r#"class Foo
  private

  def do_internal_work; end

  public

  def do_something; end
end
"#,
        &[(
            8,
            3,
            r#"`public_methods` is supposed to appear before `private_methods`."#,
        )],
    ),
    (
        1109,
        r#"class Foo
  private

  def do_internal_work; end

  public

  attr_reader :foo
end
"#,
        &[],
    ),
    (
        1126,
        r#"class Foo
  def do_something; end

  private

  CONST = 1
end
"#,
        &[(
            6,
            3,
            r#"`constants` is supposed to appear before `public_methods`."#,
        )],
    ),
    (
        1152,
        r#"class << self
  CONST = 'wrong place'
  include AnotherModule
  extend SomeModule
end
"#,
        &[(
            3,
            3,
            r#"`module_inclusion` is supposed to appear before `constants`."#,
        )],
    ),
    (
        1193,
        r#"class Foo
  attr_accessor :foo
  CONST = 'wrong place'
end
"#,
        &[],
    ),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 52);
    for &(line, source, expected) in PINNED {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(l, c, m)| (l, c, m.to_owned()))
            .collect();
        assert_eq!(findings(source), expected, "spec line {line} {source:?}");
        // Detection-only: correction generation never yields an edit.
        assert!(
            lint_str_with_corrections(&[&ClassStructure], source)
                .iter()
                .all(|o| o.correction.is_none()),
            "spec line {line}"
        );
    }
}

#[test]
fn metadata() {
    let cop = ClassStructure;
    assert_eq!(cop.name(), COP);
    assert!(!cop.default_enabled());
    assert!(!cop.department_selection_enables());
    assert!(!cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(cop.max_setting().is_none() && cop.path_policy().is_none());
    assert_eq!(cop.ruby_versions(), Default::default());
}

fn one(source: &str, line: u32, col: u32, category: &str, previous: &str) {
    assert_eq!(
        findings(source),
        vec![(line, col, msg(category, previous))],
        "{source:?}"
    );
}

fn none(source: &str) {
    assert_eq!(findings(source), vec![], "{source:?}");
}

#[test]
fn fixed_default_order_is_accepted_in_full() {
    none(
        "class A\n  include M\n  prepend N\n  extend O\n  X = 1\n  Y::Z = 2\n  \
         def self.a; end\n  def initialize; end\n  def b; end\n  protected\n  \
         def c; end\n  private_class_method def self.d; end\n  private\n  \
         def e; end\nend\n",
    );
    one(
        "class A\n  X = 1\n  prepend N\nend\n",
        3,
        3,
        "module_inclusion",
        "constants",
    );
    one(
        "class A\n  def initialize; end\n  def self.a; end\nend\n",
        3,
        3,
        "public_class_methods",
        "initializer",
    );
    one(
        "class A\n  def a; end\n  def initialize; end\nend\n",
        3,
        3,
        "initializer",
        "public_methods",
    );
    // `initialize` is the initializer whatever its visibility section.
    one(
        "class A\n  private\n  def a; end\n  def initialize; end\nend\n",
        4,
        3,
        "initializer",
        "private_methods",
    );
    one(
        "class A\n  protected def a; end\n  public def b; end\nend\n",
        3,
        3,
        "public_methods",
        "protected_methods",
    );
}

#[test]
fn custom_categories_and_order_are_not_knobs() {
    // Attribute macros, delegates, validations and associations are not
    // categories at the fixed defaults: never ordered.
    for source in [
        "class A\n  validates :a\n  attr_reader :a\nend\n",
        "class A\n  def a; end\n  attr_reader :a\n  has_many :b\n  delegate :c, to: :d\nend\n",
        "class A\n  private\n  attr_accessor :a\n  def initialize; end\nend\n",
        "class A\n  attr_accessor :a\n  X = 1\nend\n",
    ] {
        none(source);
    }
}

#[test]
fn every_class_and_singleton_class_is_checked_but_not_modules_or_blocks() {
    one(
        "class << self\n  def a; end\n  include M\nend\n",
        3,
        3,
        "module_inclusion",
        "public_methods",
    );
    one(
        "class A\n  def a; end\n  class B\n    include M\n  end\n  X = 1\nend\n",
        6,
        3,
        "constants",
        "public_methods",
    );
    one(
        "class A\n  class << self\n    def a; end\n    X = 1\n  end\nend\n",
        4,
        5,
        "constants",
        "public_methods",
    );
    for source in [
        "module M\n  def a; end\n  include N\nend\n",
        "Foo = Class.new do\n  def a; end\n  include M\nend\n",
        "Struct.new(:a) do\n  def a; end\n  X = 1\nend\n",
        "class A\n  include M\nend\n",
        "class A; end\n",
        "class A\nend\n",
        // A body with `rescue`/`ensure` is one `rescue` element.
        "class A\n  def a; end\n  include M\nrescue\n  nil\nend\n",
        "class A\n  def a; end\n  include M\nensure\n  nil\nend\n",
        // Nested classes and other statements are not ordered elements.
        "class A\n  def a; end\n  class B; end\n  module C; end\n  class << self; end\n  @x = 1\n  x = 2\nend\n",
    ] {
        none(source);
    }
}

#[test]
fn highest_priority_disorder_and_same_category_groups() {
    // The category blocking the order is the highest priority seen so far.
    assert_eq!(
        findings("class A\n  def a; end\n  def self.b; end\n  include M\nend\n"),
        vec![
            (3, 3, msg("public_class_methods", "public_methods")),
            (4, 3, msg("module_inclusion", "public_methods")),
        ]
    );
    // A same-category run is reported once; ignored elements do not split
    // it, a different category does.
    assert_eq!(
        findings(
            "class A\n  X = 1\n  def a; end\n  Y = 2\n  foo\n  Z = 3\n  def self.b; end\n  W = 4\nend\n"
        ),
        vec![
            (4, 3, msg("constants", "public_methods")),
            (7, 3, msg("public_class_methods", "public_methods")),
            (8, 3, msg("constants", "public_methods")),
        ]
    );
    // A later element of an earlier category is reported again.
    one(
        "class A\n  include M\n  X = 1\n  include N\nend\n",
        4,
        3,
        "module_inclusion",
        "constants",
    );
}

#[test]
fn bare_and_inline_visibility() {
    // A bare modifier opens a section; with arguments, a receiver or a block
    // argument it does not.
    one(
        "class A\n  private def a; end\n  protected\n  def b; end\nend\n",
        4,
        3,
        "protected_methods",
        "private_methods",
    );
    for source in [
        "class A\n  private def a; end\n  protected :foo\n  def b; end\nend\n",
        "class A\n  private def a; end\n  protected(&foo)\n  def b; end\nend\n",
        "class A\n  private def a; end\n  self.protected\n  def b; end\nend\n",
    ] {
        one(source, 4, 3, "public_methods", "private_methods");
    }
    none("class A\n  private\n  def a; end\n  self.public\n  def b; end\nend\n");
    one(
        "class A\n  private\n  def a; end\n  public()\n  def b; end\nend\n",
        5,
        3,
        "public_methods",
        "private_methods",
    );
    // The last right sibling naming the method wins, in any argument.
    one(
        "class A\n  def a; end\n  public :b, :a\n  private :x, :a\n  def b; end\nend\n",
        5,
        3,
        "public_methods",
        "private_methods",
    );
    none("class A\n  def a; end\n  private :a\n  public :a\n  def b; end\nend\n");
    // A string or a block-wrapped call does not name the method.
    none("class A\n  def a; end\n  private 'a'\n  def b; end\nend\n");
    none("class A\n  def a; end\n  private(:a) { }\n  def b; end\nend\n");
    // A section never reaches into a `begin` block: its siblings are its own.
    one(
        "class A\n  private\n  def a; end\n  begin\n    def b; end\n  end\nend\n",
        5,
        5,
        "public_methods",
        "private_methods",
    );
}

#[test]
fn def_modifiers_and_class_method_visibility() {
    one(
        "class A\n  private def a; end\n  protected memoize def b; end\nend\n",
        3,
        3,
        "protected_methods",
        "private_methods",
    );
    one(
        "class A\n  private def a; end\n  private_class_method def b; end\nend\n",
        3,
        3,
        "private_class_methods",
        "private_methods",
    );
    one(
        "class A\n  private(def a; end)\n  def b; end\nend\n",
        3,
        3,
        "public_methods",
        "private_methods",
    );
    // Unknown modifiers and receivers make the element unordered.
    for source in [
        "class A\n  private def a; end\n  module_function def b; end\nend\n",
        "class A\n  private def a; end\n  memoize def b; end\nend\n",
        "class A\n  private def a; end\n  self.public def b; end\nend\n",
    ] {
        none(source);
    }
    // Only `private_class_method`/`public_class_method` reach a singleton
    // method; `private :name` does not.
    one(
        "class A\n  def b; end\n  def self.a; end\n  private :a\nend\n",
        3,
        3,
        "public_class_methods",
        "public_methods",
    );
    none("class A\n  def b; end\n  def self.a; end\n  private_class_method :a\nend\n");
    one(
        "class A\n  def obj.a; end\n  private_class_method :a\n  def self.b; end\nend\n",
        4,
        3,
        "public_class_methods",
        "private_class_methods",
    );
    one(
        "class A\n  def a; end\n  def self.initialize; end\nend\n",
        3,
        3,
        "public_class_methods",
        "public_methods",
    );
}

#[test]
fn sends_and_blocks_are_classified_by_method_name() {
    for source in [
        "class A\n  def a; end\n  Foo.include M\nend\n",
        "class A\n  def a; end\n  include(M)\nend\n",
        "class A\n  def a; end\n  extend M do\n  end\nend\n",
        "class A\n  def a; end\n  prepend(M) { |m| m }\nend\n",
        "class A\n  def a; end\n  include(&m)\nend\n",
    ] {
        one(source, 3, 3, "module_inclusion", "public_methods");
    }
    // Safe navigation and numbered/`it` blocks are other node types.
    for source in [
        "class A\n  def a; end\n  foo&.include M\nend\n",
        "class A\n  def a; end\n  foo&.include(M) { }\nend\n",
        "class A\n  def a; end\n  include(M) { _1 }\nend\n",
        "class A\n  def a; end\n  include(M) { it }\nend\n",
        "class A\n  def a; end\n  included do\n  end\nend\n",
    ] {
        none(source);
    }
    // A call named like a category is that category, with its section.
    one(
        "class A\n  private\n  def a; end\n  public\n  methods\nend\n",
        5,
        3,
        "public_methods",
        "private_methods",
    );
    one(
        "class A\n  def a; end\n  constants\nend\n",
        3,
        3,
        "constants",
        "public_methods",
    );
}

#[test]
fn constants_and_private_constants() {
    for source in [
        "class A\n  def a; end\n  Foo::X = 1\nend\n",
        "class A\n  def a; end\n  ::X = 1\nend\n",
        "class A\n  def a; end\n  X = foo + 1\nend\n",
        // Only a symbol names a private constant.
        "class A\n  def a; end\n  X = 1\n  private_constant 'X'\nend\n",
        // Another statement list does not make it private.
        "class A\n  def a; end\n  begin\n    X = 1\n  end\n  private_constant :X\nend\n",
        // A namespaced constant is never private.
        "class A\n  def a; end\n  self::X = 1\n  private_constant :X\nend\n",
    ] {
        let (line, col) = if source.contains("begin") {
            (4, 5)
        } else {
            (3, 3)
        };
        one(source, line, col, "constants", "public_methods");
    }
    for source in [
        "class A\n  def a; end\n  X = 1\n  private_constant :X\nend\n",
        "class A\n  def a; end\n  X = 1\n  self.private_constant :Y, :X\nend\n",
        "class A\n  private_constant :X\n  def a; end\n  X = 1\nend\n",
        "class A\n  def a; end\n  X ||= 1\n  Y += 1\n  Z, W = 1, 2\nend\n",
    ] {
        none(source);
    }
}

#[test]
fn begin_and_parentheses_flattening() {
    // A sole parenthesized body is the class's statement list, as in the
    // parser AST; a nested one is an unordered element.
    one(
        "class A\n  (\n    def a; end\n    include M\n  )\nend\n",
        4,
        5,
        "module_inclusion",
        "public_methods",
    );
    none("class A\n  def a; end\n  (include M)\nend\n");
    none("class A\n  ((def a; end; include M))\nend\n");
    // `begin` blocks are flattened in order across levels.
    assert_eq!(
        findings(
            "class A\n  def a; end\n  begin\n    X = 1\n    begin\n      include M\n    end\n  end\n  def self.b; end\nend\n"
        ),
        vec![
            (4, 5, msg("constants", "public_methods")),
            (6, 7, msg("module_inclusion", "public_methods")),
            (9, 3, msg("public_class_methods", "public_methods")),
        ]
    );
    none("class A\n  def a; end\n  begin\n    include M\n  rescue\n  end\nend\n");
    none("class A\n  def a; end\n  begin\n    include M\n  ensure\n  end\nend\n");
}

#[test]
fn positions_are_characters_and_cover_the_element() {
    one(
        "class A; def é; end; include M; end\n",
        1,
        22,
        "module_inclusion",
        "public_methods",
    );
    let found = lint_str_with(
        &[&ClassStructure],
        "class A\n  def a; end\n  def self.b\n    1\n  end\nend\n",
    );
    assert_eq!(
        found
            .iter()
            .map(|o| (o.line, o.end_line))
            .collect::<Vec<_>>(),
        vec![(3, 5)]
    );
    // CRLF and a BOM keep the positions.
    one(
        "\u{feff}class A\r\n  def a; end\r\n  include M\r\nend\r\n",
        3,
        3,
        "module_inclusion",
        "public_methods",
    );
}
