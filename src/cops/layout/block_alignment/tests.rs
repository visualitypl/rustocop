//! Examples ported from RuboCop's `spec/rubocop/cop/layout/block_alignment_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    offense::Severity,
    pipeline::lint_str,
};

fn offenses(source: &str) -> Vec<Offense> {
    assert_eq!(
        ruby_prism::parse(source.as_bytes()).errors().count(),
        0,
        "{source}"
    );
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == "Layout/BlockAlignment")
        .collect()
}

fn positions(source: &str) -> Vec<(u32, u32, String)> {
    let mut found: Vec<_> = offenses(source)
        .into_iter()
        .map(|o| (o.line, o.col, o.message))
        .collect();
    found.sort();
    found
}

/// Offenses and the single-pass correction (the source itself when no edit).
fn check(source: &str, expected: &[(u32, u32, &str)], fixed: Option<&str>) {
    let want: Vec<_> = expected
        .iter()
        .map(|(l, c, m)| (*l, *c, (*m).to_owned()))
        .collect();
    assert_eq!(positions(source), want, "{source}");
    if expected.is_empty() {
        assert!(fixed.is_none());
        return;
    }
    assert_correction(&BlockAlignment, source, fixed.unwrap_or(source));
}

/// Every pinned example at fixed defaults: (source, offenses, correction).
/// Examples declared for `start_of_line`/`start_of_block` are replayed at the
/// fixed `either` style; their expected results and corrected bytes match the
/// fixed-default replay recorded in the pinned RuboCop reference run.
#[allow(clippy::type_complexity)]
const PINNED: &[(&str, &[(u32, u32, &str)], Option<&str>)] = &[
    // spec:8 (either) when the block has no arguments registers an offense for mismatched block end
    (
        "test do\n  end\n",
        &[(2, 3, "`end` at 2, 2 is not aligned with `test do` at 1, 0.")],
        Some("test do\nend\n"),
    ),
    // spec:23 (either) when the block has arguments registers an offense for mismatched block end
    (
        "test do |ala|\n  end\n",
        &[(
            2,
            3,
            "`end` at 2, 2 is not aligned with `test do |ala|` at 1, 0.",
        )],
        Some("test do |ala|\nend\n"),
    ),
    // spec:37 (either) accepts a block end that does not begin its line
    (
        "scope :bar, lambda { joins(:baz)\n                     .distinct }\n",
        &[],
        None,
    ),
    // spec:45 (either) when the block is a logical operand accepts a correctly aligned block end
    (
        "(value.is_a? Array) && value.all? do |subvalue|\n  type_check_value(subvalue, array_type)\nend\na || b do\nend\n",
        &[],
        None,
    ),
    // spec:56 (either) accepts end aligned with a variable
    ("variable = test do |ala|\nend\n", &[], None),
    // spec:64 (either) when there is an assignment chain registers an offense for an end aligned with the 2nd variable
    (
        "a = b = c = test do |ala|\n    end\n",
        &[(
            2,
            5,
            "`end` at 2, 4 is not aligned with `a = b = c = test do |ala|` at 1, 0.",
        )],
        Some("a = b = c = test do |ala|\nend\n"),
    ),
    // spec:77 (either) when there is an assignment chain accepts end aligned with the first variable
    ("a = b = c = test do |ala|\nend\n", &[], None),
    // spec:86 (either) and the block is an operand accepts end aligned with a variable
    (
        "b = 1 + preceding_line.reduce(0) do |a, e|\n  a + e.length + newline_length\nend + 1\n",
        &[],
        None,
    ),
    // spec:95 (either) registers an offense for mismatched block end with a variable
    (
        "variable = test do |ala|\n  end\n",
        &[(
            2,
            3,
            "`end` at 2, 2 is not aligned with `variable = test do |ala|` at 1, 0.",
        )],
        Some("variable = test do |ala|\nend\n"),
    ),
    // spec:109 (either) when the block is defined on the next line accepts end aligned with the block expression
    (
        "variable =\n  a_long_method_that_dont_fit_on_the_line do |v|\n    v.foo\n  end\n",
        &[],
        None,
    ),
    // spec:118 (either) when the block is defined on the next line registers an offense for mismatched end alignment
    (
        "variable =\n  a_long_method_that_dont_fit_on_the_line do |v|\n    v.foo\nend\n",
        &[(
            4,
            1,
            "`end` at 4, 0 is not aligned with `a_long_method_that_dont_fit_on_the_line do |v|` at 2, 2.",
        )],
        Some("variable =\n  a_long_method_that_dont_fit_on_the_line do |v|\n    v.foo\n  end\n"),
    ),
    // spec:138 (either) when the method part is a call chain that spans several lines accepts pretty alignment style
    (
        "def foo(bar)\n  bar.get_stuffs\n      .reject do |stuff|\n        stuff.with_a_very_long_expression_that_doesnt_fit_the_line\n      end.select do |stuff|\n        stuff.another_very_long_expression_that_doesnt_fit_the_line\n      end\n      .select do |stuff|\n        stuff.another_very_long_expression_that_doesnt_fit_the_line\n      end\nend\n",
        &[],
        None,
    ),
    // spec:154 (either) when the method part is a call chain that spans several lines registers offenses for misaligned ends
    (
        "def foo(bar)\n  bar.get_stuffs\n      .reject do |stuff|\n        stuff.with_a_very_long_expression_that_doesnt_fit_the_line\n        end.select do |stuff|\n        stuff.another_very_long_expression_that_doesnt_fit_the_line\n    end\n      .select do |stuff|\n        stuff.another_very_long_expression_that_doesnt_fit_the_line\n        end\nend\n",
        &[
            (
                5,
                9,
                "`end` at 5, 8 is not aligned with `bar.get_stuffs` at 2, 2 or `.reject do |stuff|` at 3, 6.",
            ),
            (
                7,
                5,
                "`end` at 7, 4 is not aligned with `bar.get_stuffs` at 2, 2 or `end.select do |stuff|` at 5, 8.",
            ),
            (
                10,
                9,
                "`end` at 10, 8 is not aligned with `bar.get_stuffs` at 2, 2 or `.select do |stuff|` at 8, 6.",
            ),
        ],
        Some(
            "def foo(bar)\n  bar.get_stuffs\n      .reject do |stuff|\n        stuff.with_a_very_long_expression_that_doesnt_fit_the_line\n  end.select do |stuff|\n        stuff.another_very_long_expression_that_doesnt_fit_the_line\n  end\n      .select do |stuff|\n        stuff.another_very_long_expression_that_doesnt_fit_the_line\n  end\nend\n",
        ),
    ),
    // spec:188 (either) when the method part is a call chain that spans several lines accepts end indented as the start of the block
    (
        "       my_object.chaining_this_very_long_method(with_a_parameter)\n           .and_one_with_a_block do\n         do_something\n       end\n# Other variant:\n       my_object.chaining_this_very_long_method(\n           with_a_parameter).and_one_with_a_block do\n         do_something\n       end\n",
        &[],
        None,
    ),
    // spec:203 (either) when the method part is a call chain that spans several lines accepts two kinds of end alignment
    (
        "# Aligned with start of line where do is:\nparams = default_options.merge(options)\n          .delete_if { |k, v| v.nil? }\n          .each_with_object({}) do |(k, v), new_hash|\n            new_hash[k.to_s] = v.to_s\n          end\n# Aligned with start of the whole expression:\nparams = default_options.merge(options)\n          .delete_if { |k, v| v.nil? }\n          .each_with_object({}) do |(k, v), new_hash|\n            new_hash[k.to_s] = v.to_s\nend\n",
        &[],
        None,
    ),
    // spec:222 (either) when there is a line break between the method arguments accepts `}` aligned with the start of the line where the method is called
    (
        "out\n  .brackets(lft: foo,\n            rgt: foo) {\n    process(scheme.constraint)\n  }\n",
        &[],
        None,
    ),
    // spec:232 (either) when there is a line break between the method arguments accepts `end` aligned with the start of the line where the method is called
    (
        "out\n  .brackets(lft: foo,\n            rgt: foo) do\n    process(scheme.constraint)\n  end\n",
        &[],
        None,
    ),
    // spec:232 (either) when there is a line break between the method arguments accepts `end` aligned with the start of the line where the method is called
    (
        "out\n  .brackets(lft: foo,\n            rgt: foo) do\n    process(scheme.constraint)\n  end\n",
        &[],
        None,
    ),
    // spec:243 (either) when there is a line break between the method arguments accepts `}` aligned with the start of the line where the method is called when the block is chained
    (
        "out\n  .brackets(lft: foo,\n            rgt: foo) {\n    process(scheme.constraint)\n  }.to_s\n",
        &[],
        None,
    ),
    // spec:253 (either) when there is a line break between the method arguments accepts `}` aligned with the start of the line where the `{` is
    (
        "out\n  .brackets(lft: foo,\n            rgt: foo) {\n    process(scheme.constraint)\n            }\n",
        &[],
        None,
    ),
    // spec:264 (either) when there is a line break between the method arguments registers an offense and corrects when `}` is aligned with neither the start of the expression nor the start of the line where the method is called
    (
        "out\n  .brackets(lft: foo,\n            rgt: foo) {\n    process(scheme.constraint)\n      }\n",
        &[(
            5,
            7,
            "`}` at 5, 6 is not aligned with `out` at 1, 0 or `.brackets(lft: foo,` at 2, 2.",
        )],
        Some(
            "out\n  .brackets(lft: foo,\n            rgt: foo) {\n    process(scheme.constraint)\n}\n",
        ),
    ),
    // spec:285 (either) when variables of a mass assignment spans several lines accepts end aligned with the variables
    ("e,\nf = [5, 6].map do |i|\n  i - 5\nend\n", &[], None),
    // spec:294 (either) when variables of a mass assignment spans several lines registers an offense for end aligned with the block
    (
        "e,\nf = [5, 6].map do |i|\n  i - 5\n    end\n",
        &[(
            4,
            5,
            "`end` at 4, 4 is not aligned with `e,` at 1, 0 or `f = [5, 6].map do |i|` at 2, 0.",
        )],
        Some("e,\nf = [5, 6].map do |i|\n  i - 5\nend\n"),
    ),
    // spec:312 (either) accepts end aligned with an instance variable
    ("@variable = test do |ala|\nend\n", &[], None),
    // spec:319 (either) registers an offense for mismatched block end with an instance variable
    (
        "@variable = test do |ala|\n  end\n",
        &[(
            2,
            3,
            "`end` at 2, 2 is not aligned with `@variable = test do |ala|` at 1, 0.",
        )],
        Some("@variable = test do |ala|\nend\n"),
    ),
    // spec:332 (either) accepts end aligned with a class variable
    ("@@variable = test do |ala|\nend\n", &[], None),
    // spec:339 (either) registers an offense for mismatched block end with a class variable
    (
        "@@variable = test do |ala|\n  end\n",
        &[(
            2,
            3,
            "`end` at 2, 2 is not aligned with `@@variable = test do |ala|` at 1, 0.",
        )],
        Some("@@variable = test do |ala|\nend\n"),
    ),
    // spec:352 (either) accepts end aligned with a global variable
    ("$variable = test do |ala|\nend\n", &[], None),
    // spec:359 (either) registers an offense for mismatched block end with a global variable
    (
        "$variable = test do |ala|\n  end\n",
        &[(
            2,
            3,
            "`end` at 2, 2 is not aligned with `$variable = test do |ala|` at 1, 0.",
        )],
        Some("$variable = test do |ala|\nend\n"),
    ),
    // spec:372 (either) accepts end aligned with a constant
    ("CONSTANT = test do |ala|\nend\n", &[], None),
    // spec:379 (either) registers an offense for mismatched block end with a constant
    (
        "Module::CONSTANT = test do |ala|\n  end\n",
        &[(
            2,
            3,
            "`end` at 2, 2 is not aligned with `Module::CONSTANT = test do |ala|` at 1, 0.",
        )],
        Some("Module::CONSTANT = test do |ala|\nend\n"),
    ),
    // spec:392 (either) accepts end aligned with a method call
    (
        "parser.children << lambda do |token|\n  token << 1\nend\n",
        &[],
        None,
    ),
    // spec:400 (either) registers an offense for mismatched block end with a method call
    (
        "parser.children << lambda do |token|\n  token << 1\n  end\n",
        &[(
            3,
            3,
            "`end` at 3, 2 is not aligned with `parser.children << lambda do |token|` at 1, 0.",
        )],
        Some("parser.children << lambda do |token|\n  token << 1\nend\n"),
    ),
    // spec:415 (either) accepts end aligned with a method call with arguments
    (
        "@h[:f] = f.each_pair.map do |f, v|\n  v = 1\nend\n",
        &[],
        None,
    ),
    // spec:423 (either) registers an offense for mismatched end with a method call with arguments
    (
        "@h[:f] = f.each_pair.map do |f, v|\n  v = 1\n  end\n",
        &[(
            3,
            3,
            "`end` at 3, 2 is not aligned with `@h[:f] = f.each_pair.map do |f, v|` at 1, 0.",
        )],
        Some("@h[:f] = f.each_pair.map do |f, v|\n  v = 1\nend\n"),
    ),
    // spec:438 (either) does not raise an error for nested block in a method call
    ("expect(arr.all? { |o| o.valid? })", &[], None),
    // spec:442 (either) accepts end aligned with the block when the block is a method argument
    (
        "expect(arr.all? do |o|\n         o.valid?\n       end)\n",
        &[],
        None,
    ),
    // spec:450 (either) registers an offense for mismatched end not aligned with the block that is an argument
    (
        "expect(arr.all? do |o|\n  o.valid?\n  end)\n",
        &[(
            3,
            3,
            "`end` at 3, 2 is not aligned with `arr.all? do |o|` at 1, 7 or `expect(arr.all? do |o|` at 1, 0.",
        )],
        Some("expect(arr.all? do |o|\n  o.valid?\nend)\n"),
    ),
    // spec:465 (either) accepts end aligned with an op-asgn (+=, -=)
    (
        "rb += files.select do |file|\n  file << something\nend\n",
        &[],
        None,
    ),
    // spec:473 (either) registers an offense for mismatched block end with an op-asgn (+=, -=)
    (
        "rb += files.select do |file|\n  file << something\n  end\n",
        &[(3, 3, "`end` at 3, 2 is not aligned with `rb` at 1, 0.")],
        Some("rb += files.select do |file|\n  file << something\nend\n"),
    ),
    // spec:488 (either) accepts end aligned with an and-asgn (&&=)
    ("variable &&= test do |ala|\nend\n", &[], None),
    // spec:495 (either) registers an offense for mismatched block end with an and-asgn (&&=)
    (
        "variable &&= test do |ala|\n  end\n",
        &[(
            2,
            3,
            "`end` at 2, 2 is not aligned with `variable &&= test do |ala|` at 1, 0.",
        )],
        Some("variable &&= test do |ala|\nend\n"),
    ),
    // spec:508 (either) accepts end aligned with an or-asgn (||=)
    ("variable ||= test do |ala|\nend\n", &[], None),
    // spec:515 (either) registers an offense for mismatched block end with an or-asgn (||=)
    (
        "variable ||= test do |ala|\n  end\n",
        &[(
            2,
            3,
            "`end` at 2, 2 is not aligned with `variable ||= test do |ala|` at 1, 0.",
        )],
        Some("variable ||= test do |ala|\nend\n"),
    ),
    // spec:528 (either) accepts end aligned with a mass assignment
    ("var1, var2 = lambda do |test|\n  [1, 2]\nend\n", &[], None),
    // spec:536 (either) accepts end aligned with a call chain left hand side
    (
        "parser.diagnostics.consumer = lambda do |diagnostic|\n  diagnostics << diagnostic\nend\n",
        &[],
        None,
    ),
    // spec:544 (either) registers an offense for mismatched block end with a mass assignment
    (
        "var1, var2 = lambda do |test|\n  [1, 2]\n  end\n",
        &[(
            3,
            3,
            "`end` at 3, 2 is not aligned with `var1, var2` at 1, 0.",
        )],
        Some("var1, var2 = lambda do |test|\n  [1, 2]\nend\n"),
    ),
    // spec:560 (either) when multiple similar-looking blocks have misaligned ends registers an offense for each of them
    (
        "a = test do\n end\nb = test do\n end\n",
        &[
            (
                2,
                2,
                "`end` at 2, 1 is not aligned with `a = test do` at 1, 0.",
            ),
            (
                4,
                2,
                "`end` at 4, 1 is not aligned with `b = test do` at 3, 0.",
            ),
        ],
        Some("a = test do\nend\nb = test do\nend\n"),
    ),
    // spec:580 (either) on a splatted method call aligns end with the splat operator
    (
        "def get_gems_by_name\n  @gems ||= Hash[*get_latest_gems.map { |gem|\n                   [gem.name, gem, gem.full_name, gem]\n                 }.flatten]\nend\n",
        &[],
        None,
    ),
    // spec:590 (either) on a splatted method call registers an offense and corrects misaligned end braces
    (
        "def get_gems_by_name\n  @gems ||= Hash[*get_latest_gems.map { |gem|\n                   [gem.name, gem, gem.full_name, gem]\n              }.flatten]\nend\n",
        &[(
            4,
            15,
            "`}` at 4, 14 is not aligned with `*get_latest_gems.map { |gem|` at 2, 17 or `@gems ||= Hash[*get_latest_gems.map { |gem|` at 2, 2.",
        )],
        Some(
            "def get_gems_by_name\n  @gems ||= Hash[*get_latest_gems.map { |gem|\n                   [gem.name, gem, gem.full_name, gem]\n  }.flatten]\nend\n",
        ),
    ),
    // spec:611 (either) on a bit-flipped method call aligns end with the ~ operator
    (
        "def abc\n  @abc ||= A[~xyz { |x|\n               x\n             }.flatten]\nend\n",
        &[],
        None,
    ),
    // spec:621 (either) on a bit-flipped method call registers an offense and corrects misaligned end brace
    (
        "def abc\n  @abc ||= A[~xyz { |x|\n               x\n                        }.flatten]\nend\n",
        &[(
            4,
            25,
            "`}` at 4, 24 is not aligned with `~xyz { |x|` at 2, 13 or `@abc ||= A[~xyz { |x|` at 2, 2.",
        )],
        Some("def abc\n  @abc ||= A[~xyz { |x|\n               x\n  }.flatten]\nend\n"),
    ),
    // spec:642 (either) on a logically negated method call aligns end with the ! operator
    (
        "def abc\n  @abc ||= A[!xyz { |x|\n               x\n             }.flatten]\nend\n",
        &[],
        None,
    ),
    // spec:652 (either) on a logically negated method call registers an offense and corrects
    (
        "def abc\n  @abc ||= A[!xyz { |x|\n               x\n}.flatten]\nend\n",
        &[(
            4,
            1,
            "`}` at 4, 0 is not aligned with `!xyz { |x|` at 2, 13 or `@abc ||= A[!xyz { |x|` at 2, 2.",
        )],
        Some("def abc\n  @abc ||= A[!xyz { |x|\n               x\n  }.flatten]\nend\n"),
    ),
    // spec:673 (either) on an arithmetically negated method call aligns end with the - operator
    (
        "def abc\n  @abc ||= A[-xyz { |x|\n               x\n             }.flatten]\nend\n",
        &[],
        None,
    ),
    // spec:683 (either) on an arithmetically negated method call registers an offense and corrects
    (
        "def abc\n  @abc ||= A[-xyz { |x|\n               x\n                  }.flatten]\nend\n",
        &[(
            4,
            19,
            "`}` at 4, 18 is not aligned with `-xyz { |x|` at 2, 13 or `@abc ||= A[-xyz { |x|` at 2, 2.",
        )],
        Some("def abc\n  @abc ||= A[-xyz { |x|\n               x\n  }.flatten]\nend\n"),
    ),
    // spec:704 (either) when the block is terminated by } mentions } (not end) in the message
    (
        "test {\n  }\n",
        &[(2, 3, "`}` at 2, 2 is not aligned with `test {` at 1, 0.")],
        Some("test {\n}\n"),
    ),
    // spec:721 (start_of_line) with `EnforcedStyle: start_of_line` allows when start_of_line aligned
    ("foo.bar\n  .each do\n    baz\nend\n", &[], None),
    // spec:721 (start_of_line) with `EnforcedStyle: start_of_line` allows when start_of_line aligned
    ("foo.bar\n  .each do\n    baz\nend\n", &[], None),
    // spec:730 (start_of_line) with `EnforcedStyle: start_of_line` errors when do aligned
    ("foo.bar\n  .each do\n    baz\n  end\n", &[], None),
    // spec:730 (start_of_line) with `EnforcedStyle: start_of_line` errors when do aligned
    ("foo.bar\n  .each do\n    baz\n  end\n", &[], None),
    // spec:748 (start_of_line) with `EnforcedStyle: start_of_line` when a block is passed as a method argument does not register an offense when `}` is aligned with the start of the line for a literal lambda
    ("foo :x, ->(y) {\n  bar\n}\n", &[], None),
    // spec:756 (start_of_line) with `EnforcedStyle: start_of_line` when a block is passed as a method argument does not register an offense when `}` is aligned with the start of the line for a brace block
    ("foo :x, bar {\n  baz\n}\n", &[], None),
    // spec:764 (start_of_line) with `EnforcedStyle: start_of_line` when a block is passed as a method argument registers an offense and corrects when `}` is not aligned with the start of the line
    (
        "foo :x, ->(y) {\n  bar\n  }\n",
        &[(
            3,
            3,
            "`}` at 3, 2 is not aligned with `->(y) {` at 1, 8 or `foo :x, ->(y) {` at 1, 0.",
        )],
        Some("foo :x, ->(y) {\n  bar\n}\n"),
    ),
    // spec:781 (start_of_line) with `EnforcedStyle: start_of_line` inside a non-endless method does not register an offense when `end` is aligned with the block start
    ("def foo\n  bar do\n    baz\n  end\nend\n", &[], None),
    // spec:781 (start_of_line) with `EnforcedStyle: start_of_line` inside a non-endless method does not register an offense when `end` is aligned with the block start
    ("def foo\n  bar do\n    baz\n  end\nend\n", &[], None),
    // spec:791 (start_of_line) with `EnforcedStyle: start_of_line` inside a non-endless method registers an offense and corrects when `end` is aligned with the method start
    (
        "def foo\n  bar do\n    baz\nend\nend\n",
        &[(4, 1, "`end` at 4, 0 is not aligned with `bar do` at 2, 2.")],
        Some("def foo\n  bar do\n    baz\n  end\nend\n"),
    ),
    // spec:791 (start_of_line) with `EnforcedStyle: start_of_line` inside a non-endless method registers an offense and corrects when `end` is aligned with the method start
    (
        "def foo\n  bar do\n    baz\nend\nend\n",
        &[(4, 1, "`end` at 4, 0 is not aligned with `bar do` at 2, 2.")],
        Some("def foo\n  bar do\n    baz\n  end\nend\n"),
    ),
    // spec:812 (start_of_line) with `EnforcedStyle: start_of_line` inside a non-endless singleton method does not register an offense when `end` is aligned with the block start
    ("def self.foo\n  bar do\n    baz\n  end\nend\n", &[], None),
    // spec:812 (start_of_line) with `EnforcedStyle: start_of_line` inside a non-endless singleton method does not register an offense when `end` is aligned with the block start
    ("def self.foo\n  bar do\n    baz\n  end\nend\n", &[], None),
    // spec:822 (start_of_line) with `EnforcedStyle: start_of_line` inside a non-endless singleton method registers an offense and corrects when `end` is aligned with the method start
    (
        "def self.foo\n  bar do\n    baz\nend\nend\n",
        &[(4, 1, "`end` at 4, 0 is not aligned with `bar do` at 2, 2.")],
        Some("def self.foo\n  bar do\n    baz\n  end\nend\n"),
    ),
    // spec:822 (start_of_line) with `EnforcedStyle: start_of_line` inside a non-endless singleton method registers an offense and corrects when `end` is aligned with the method start
    (
        "def self.foo\n  bar do\n    baz\nend\nend\n",
        &[(4, 1, "`end` at 4, 0 is not aligned with `bar do` at 2, 2.")],
        Some("def self.foo\n  bar do\n    baz\n  end\nend\n"),
    ),
    // spec:843 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods does not register an offense when `end` is aligned with the start of the method definition
    ("def foo = bar do\n  baz\nend\n", &[], None),
    // spec:843 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods does not register an offense when `end` is aligned with the start of the method definition
    ("def foo = bar do\n  baz\nend\n", &[], None),
    // spec:851 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods does not register an offense when `end` is aligned with the start of a multiline method definition
    (
        "def foo = bar(123,\n              456) do\n  baz\nend\n",
        &[],
        None,
    ),
    // spec:851 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods does not register an offense when `end` is aligned with the start of a multiline method definition
    (
        "def foo = bar(123,\n              456) do\n  baz\nend\n",
        &[],
        None,
    ),
    // spec:860 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods registers an offense when `end` is not aligned with the start of the method definition
    (
        "def foo = bar do\n  baz\n    end\n",
        &[(
            3,
            5,
            "`end` at 3, 4 is not aligned with `def foo = bar do` at 1, 0.",
        )],
        Some("def foo = bar do\n  baz\nend\n"),
    ),
    // spec:860 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods registers an offense when `end` is not aligned with the start of the method definition
    (
        "def foo = bar do\n  baz\n    end\n",
        &[(
            3,
            5,
            "`end` at 3, 4 is not aligned with `def foo = bar do` at 1, 0.",
        )],
        Some("def foo = bar do\n  baz\nend\n"),
    ),
    // spec:875 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods does not register an offense when `end` is aligned with the start of a singleton method definition
    ("def self.foo = bar do\n  baz\nend\n", &[], None),
    // spec:875 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods does not register an offense when `end` is aligned with the start of a singleton method definition
    ("def self.foo = bar do\n  baz\nend\n", &[], None),
    // spec:883 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods does not register an offense when `end` is aligned with the start of a multiline singleton method definition
    (
        "def self.foo = bar(123,\n                   456) do\n  baz\nend\n",
        &[],
        None,
    ),
    // spec:883 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods does not register an offense when `end` is aligned with the start of a multiline singleton method definition
    (
        "def self.foo = bar(123,\n                   456) do\n  baz\nend\n",
        &[],
        None,
    ),
    // spec:892 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods registers an offense when `end` is not aligned with the start of a singleton method definition
    (
        "def self.foo = bar do\n  baz\n    end\n",
        &[(
            3,
            5,
            "`end` at 3, 4 is not aligned with `def self.foo = bar do` at 1, 0.",
        )],
        Some("def self.foo = bar do\n  baz\nend\n"),
    ),
    // spec:892 (start_of_line) with `EnforcedStyle: start_of_line` with endless methods registers an offense when `end` is not aligned with the start of a singleton method definition
    (
        "def self.foo = bar do\n  baz\n    end\n",
        &[(
            3,
            5,
            "`end` at 3, 4 is not aligned with `def self.foo = bar do` at 1, 0.",
        )],
        Some("def self.foo = bar do\n  baz\nend\n"),
    ),
    // spec:949 (start_of_block) with `EnforcedStyle: start_of_block` when there is a line break between the method arguments errors when `end` is aligned with the line where the `do` is
    (
        "out\n  .brackets(lft: foo,\n            rgt: foo) do\n    process(scheme.constraint)\n            end\n",
        &[],
        None,
    ),
    // spec:968 (start_of_block) with `EnforcedStyle: start_of_block` when there is a line break between the method arguments allows when `end` is aligned with the start of the line where `super` is called
    ("x = super(foo,\n          bar) do\n  baz\nend\n", &[], None),
    // spec:977 (start_of_block) with `EnforcedStyle: start_of_block` when there is a line break between the method arguments allows when `}` is aligned with the start of the line where the lambda is defined
    ("x = ->(a,\n       b) {\n  baz\n}\n", &[], None),
    // spec:986 (start_of_block) with `EnforcedStyle: start_of_block` when there is a line break between the method arguments allows when `end` is aligned with the line where the `do` is and the method has no parentheses
    (
        "foo bar,\n  baz,\n  key: value do |x|\n    process(x)\n  end\n",
        &[],
        None,
    ),
    // spec:996 (start_of_block) with `EnforcedStyle: start_of_block` when there is a line break between the method arguments errors when `end` is misaligned and the method has no parentheses
    (
        "foo bar,\n  baz,\n  key: value do |x|\n    process(x)\n      end\n",
        &[(
            5,
            7,
            "`end` at 5, 6 is not aligned with `foo bar,` at 1, 0 or `key: value do |x|` at 3, 2.",
        )],
        Some("foo bar,\n  baz,\n  key: value do |x|\n    process(x)\nend\n"),
    ),
    // spec:1096 (start_of_block) with `EnforcedStyle: start_of_block` with endless methods registers an offense when `end` is not aligned with the start of a multiline method definition
    (
        "def foo = bar(123,\n              456) do\n  baz\n    end\n",
        &[(
            4,
            5,
            "`end` at 4, 4 is not aligned with `def foo = bar(123,` at 1, 0.",
        )],
        Some("def foo = bar(123,\n              456) do\n  baz\nend\n"),
    ),
    // spec:1145 (start_of_block) with `EnforcedStyle: start_of_block` with endless methods registers an offense when `end` is not aligned with the start of a multiline singleton method definition
    (
        "def self.foo = bar(123,\n                   456) do\n  baz\n    end\n",
        &[(
            4,
            5,
            "`end` at 4, 4 is not aligned with `def self.foo = bar(123,` at 1, 0.",
        )],
        Some("def self.foo = bar(123,\n                   456) do\n  baz\nend\n"),
    ),
    // spec:1180 (either) Ruby 2.7 accepts end aligned with a call chain left hand side
    (
        "parser.diagnostics.consumer = lambda do\n  _1 << diagnostic\nend\n",
        &[],
        None,
    ),
    // spec:1188 (either) Ruby 2.7 registers an offense for mismatched block end with a mass assignment
    (
        "var1, var2 = lambda do\n  [_1, _2]\n  end\n",
        &[(
            3,
            3,
            "`end` at 3, 2 is not aligned with `var1, var2` at 1, 0.",
        )],
        Some("var1, var2 = lambda do\n  [_1, _2]\nend\n"),
    ),
    // spec:1205 (either) Ruby 3.4 accepts end aligned with a call chain left hand side
    (
        "parser.diagnostics.consumer = lambda do\n  it << diagnostic\nend\n",
        &[],
        None,
    ),
    // spec:1221 (either) when `Layout/IndentationStyle` enforces tabs registers an offense and corrects `end` indented with spaces
    (
        "\tfoo do |v|\n\t\tv\n  end\n",
        &[(
            3,
            3,
            "`end` at 3, 2 is not aligned with `foo do |v|` at 1, 1.",
        )],
        Some("\tfoo do |v|\n\t\tv\n end\n"),
    ),
    // spec:1236 (either) when `Layout/IndentationStyle` enforces tabs registers an offense and corrects `end` indented with too many tabs
    (
        "\tx = foo.map do |v|\n\t\tv\n\t\tend\n",
        &[(
            3,
            3,
            "`end` at 3, 2 is not aligned with `x = foo.map do |v|` at 1, 1.",
        )],
        Some("\tx = foo.map do |v|\n\t\tv\n\tend\n"),
    ),
    // spec:1251 (either) when `Layout/IndentationStyle` enforces tabs does not register an offense when `end` is aligned with tabs
    ("\tfoo do |v|\n\t\tv\n\tend\n", &[], None),
    // spec:1259 (either) when `Layout/IndentationStyle` enforces tabs registers an offense and corrects `end` with the tabs and spaces of the anchor line
    (
        "\t  foo do |v|\n\t\tv\nend\n",
        &[(
            3,
            1,
            "`end` at 3, 0 is not aligned with `foo do |v|` at 1, 3.",
        )],
        Some("\t  foo do |v|\n\t\tv\n   end\n"),
    ),
    // spec:1274 (either) when `Layout/IndentationStyle` enforces tabs registers an offense and corrects `end` with spaces when the anchor does not start its line
    (
        "foo(\n\t1, bar do |v|\n\t\tv\nend)\n",
        &[(
            4,
            1,
            "`end` at 4, 0 is not aligned with `bar do |v|` at 2, 4 or `1, bar do |v|` at 2, 1.",
        )],
        Some("foo(\n\t1, bar do |v|\n\t\tv\n    end)\n"),
    ),
    // spec:1291 (either) when `Layout/IndentationStyle` enforces tabs registers an offense but does not correct `end` already indented like the anchor line
    (
        "a(\n  **b(\n    c).d do\n  end)\n",
        &[(
            4,
            3,
            "`end` at 4, 2 is not aligned with `b(` at 2, 4 or `c).d do` at 3, 4.",
        )],
        None,
    ),
    // spec:1306 (start_of_block) when `Layout/IndentationStyle` enforces tabs when `EnforcedStyleAlignWith` is `start_of_block` registers an offense and corrects `end` with the indentation of the `do` line
    ("\tx = foo\n\t\t.map do |v|\n\t\tv\n\tend\n", &[], None),
];

#[test]
fn every_pinned_example_at_fixed_defaults() {
    assert_eq!(PINNED.len(), 101);
    for (source, expected, fixed) in PINNED {
        check(source, expected, *fixed);
    }
    let found = offenses("test do\n  end\n");
    assert_eq!((found[0].line, found[0].end_line, found[0].col), (2, 2, 3));
    assert_eq!(found[0].severity, Severity::Convention);
}

#[test]
fn numbered_it_lambda_and_super_blocks() {
    for (source, expected, fixed) in [
        (
            "foo do\n  _1\n  end\n",
            "`end` at 3, 2 is not aligned with `foo do` at 1, 0.",
            "foo do\n  _1\nend\n",
        ),
        (
            "foo {\n  it\n  }\n",
            "`}` at 3, 2 is not aligned with `foo {` at 1, 0.",
            "foo {\n  it\n}\n",
        ),
        (
            "x = -> do\n  1\n  end\n",
            "`end` at 3, 2 is not aligned with `x = -> do` at 1, 0.",
            "x = -> do\n  1\nend\n",
        ),
        (
            "def f\n  super do\n    x\n    end\nend\n",
            "`end` at 4, 4 is not aligned with `super do` at 2, 2.",
            "def f\n  super do\n    x\n  end\nend\n",
        ),
        (
            "def f(a)\n  super(a) {\n    x\n    }\nend\n",
            "`}` at 4, 4 is not aligned with `super(a) {` at 2, 2.",
            "def f(a)\n  super(a) {\n    x\n  }\nend\n",
        ),
        (
            "foo bar(1,\n        2) { |x|\n  x\n      }\n",
            "`}` at 4, 6 is not aligned with `bar(1,` at 1, 4 or `foo bar(1,` at 1, 0.",
            "foo bar(1,\n        2) { |x|\n  x\n}\n",
        ),
    ] {
        let found = positions(source);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!(found[0].2, expected, "{source}");
        assert_correction(&BlockAlignment, source, fixed);
        assert!(offenses(fixed).is_empty(), "{fixed}");
    }
    for source in [
        "foo do\n  _1\nend\n",
        "foo { it }\n",
        "-> {\n}\n",
        "x = ->(a,\n       b) {\n  a\n}\n",
        "def f\n  super do\n  end\nend\n",
        "lambda do |x|\n  x\nend\n",
        "foo(&blk)\n",
        "foo do\nend.bar do\nend\n",
    ] {
        assert!(offenses(source).is_empty(), "{source}");
    }
}

#[test]
fn lineage_targets_receivers_operators_and_assignments() {
    for (source, expected, fixed) in [
        // A csend parent is never a target; a send on the block is.
        (
            "x = foo do\n  end&.bar\n",
            "`end` at 2, 2 is not aligned with `foo do` at 1, 4 or `x = foo do` at 1, 0.",
            "x = foo do\nend&.bar\n",
        ),
        (
            "x = foo do\n  end.bar\n",
            "`end` at 2, 2 is not aligned with `x = foo do` at 1, 0.",
            "x = foo do\nend.bar\n",
        ),
        (
            "x = foo do\n  end[0]\n",
            "`end` at 2, 2 is not aligned with `foo do` at 1, 4 or `x = foo do` at 1, 0.",
            "x = foo do\nend[0]\n",
        ),
        (
            "arr << foo do\n  end\n",
            "`end` at 2, 2 is not aligned with `arr << foo do` at 1, 0.",
            "arr << foo do\nend\n",
        ),
        (
            "a.b += foo do\n  end\n",
            "`end` at 2, 2 is not aligned with `a.b` at 1, 0.",
            "a.b += foo do\nend\n",
        ),
        (
            "h[:k] ||= foo do\n  end\n",
            "`end` at 2, 2 is not aligned with `h[:k] ||= foo do` at 1, 0.",
            "h[:k] ||= foo do\nend\n",
        ),
        (
            "(a, b) = foo do\n  end\n",
            "`end` at 2, 2 is not aligned with `(a, b)` at 1, 0.",
            "(a, b) = foo do\nend\n",
        ),
        (
            "a.b = foo do\n  end\n",
            "`end` at 2, 2 is not aligned with `a.b = foo do` at 1, 0.",
            "a.b = foo do\nend\n",
        ),
        (
            "foo do\n  bar do\n    end\nend\n",
            "`end` at 3, 4 is not aligned with `bar do` at 2, 2.",
            "foo do\n  bar do\n  end\nend\n",
        ),
        // Multiple statements form the parser gem's root `begin`.
        (
            "x; foo do\n  end\n",
            "`end` at 2, 2 is not aligned with `foo do` at 1, 3 or `x; foo do` at 1, 0.",
            "x; foo do\nend\n",
        ),
        // A keyword `begin` body is not a separate node.
        (
            "begin\n  a; foo do\n    end\nend\n",
            "`end` at 3, 4 is not aligned with `foo do` at 2, 5 or `a; foo do` at 2, 2.",
            "begin\n  a; foo do\n     end\nend\n",
        ),
        (
            "foo(<<~X) do\n  a\nX\n  end\n",
            "`end` at 4, 2 is not aligned with `foo(<<~X) do` at 1, 0.",
            "foo(<<~X) do\n  a\nX\nend\n",
        ),
    ] {
        let found = positions(source);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!(found[0].2, expected, "{source}");
        assert_correction(&BlockAlignment, source, fixed);
    }
}

#[test]
fn ends_that_do_not_begin_their_line_or_are_in_literals() {
    for source in [
        "foo do\n  x end\n",
        "foo { |x|\n  x }\n",
        "x = \"foo do\n  end\"\n",
        "x = <<~H\n  foo do\n    end\nH\n",
        "# foo do\n  # end\n",
        "for x in y\n  end\n",
    ] {
        assert!(offenses(source).is_empty(), "{source}");
    }
    // Interpolation inside a literal keeps the finding but not the edit.
    for before in [
        "x = \"#{foo do\n  1\n   end}\"\n",
        "x = <<~H\n  #{foo do\n    1\n      end}\nH\n",
    ] {
        assert_eq!(offenses(before).len(), 1, "{before}");
        assert_correction(&BlockAlignment, before, before);
    }
}

#[test]
fn bom_crlf_and_whitespace_preserve_every_other_byte() {
    for (before, after) in [
        ("\u{feff}foo do\r\n  end\r\n", "\u{feff}foo do\r\nend\r\n"),
        ("é = foo do # 日本\r\n\t end", "é = foo do # 日本\r\nend"),
        ("  foo do\nend\n", "  foo do\n  end\n"),
        ("\tfoo do\n  end\n", "\tfoo do\n end\n"),
    ] {
        assert_eq!(offenses(before).len(), 1, "{before}");
        assert_correction(&BlockAlignment, before, after);
    }
    // Vertical tab or form feed before `end`: reported, never rewritten.
    let before = "foo do\n\x0b end\n";
    let found = correction_offenses(&BlockAlignment, before);
    assert_eq!(found.len(), 1);
    assert!(found[0].correction.is_none());
}

#[test]
fn closed_observers_hold_the_edit_but_keep_the_finding() {
    for tail in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p $PROGRAM_NAME",
        "p caller",
        "p caller_locations",
        "p method(:f).source_location",
        "SCRIPT_LINES__ = {}",
        "p DATA.read\n__END__\nx",
    ] {
        let source = format!("foo do\n  1\n  end\n{tail}\n");
        let found = correction_offenses(&BlockAlignment, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
    for tail in ["p File.read('example.rb')", "p `cat example.rb`", "p DATA"] {
        let before = format!("foo do\n  1\n  end\n{tail}\n");
        let after = format!("foo do\n  1\nend\n{tail}\n");
        assert_correction(&BlockAlignment, &before, &after);
    }
}

#[test]
fn both_directive_spellings_disable_the_finding() {
    for prefix in ["rustocop", "rubocop"] {
        for target in ["Layout/BlockAlignment", "Layout", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\nfoo do\n  end\n"),
                format!("foo do\n  end # {prefix}:disable {target}\n"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
                assert!(correction_offenses(&BlockAlignment, &source).is_empty());
            }
        }
    }
}
