//! `Layout/ElseAlignment` — align `else` and `elsif` with the keyword that
//! opens their construct. As in RuboCop (`CheckAssignment`), the `else` of an
//! `if` that is the value of an assignment, or the last argument of a call,
//! aligns with the start of that assignment or call instead when
//! Layout/EndAlignment's `enforced_style_align_with` is not `keyword` and the
//! `if` starts on its line.

use std::ops::Range;

use ruby_prism::{Location, Node};

use super::alignment::{Alignment, EndAlignWith, assignment_owner, start_line_range, starts_line};
use crate::{
    cops::{Companions, Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

#[derive(Default)]
pub struct ElseAlignment {
    /// Layout/EndAlignment's `enforced_style_align_with`.
    end_align_with: EndAlignWith,
}

impl Cop for ElseAlignment {
    fn name(&self) -> &'static str {
        "Layout/ElseAlignment"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn with_companions(&self, companions: &Companions) -> Option<Box<dyn Cop>> {
        Some(Box::new(ElseAlignment {
            end_align_with: companions.end_align_with,
        }))
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut check = |base: Range<usize>, keyword: Location<'pr>| {
            if let Some(offense) = self.misaligned(node, cx, file, base, keyword) {
                out.push(offense);
            }
        };
        match node {
            Node::IfNode { .. } => {
                // Every `elsif` and `else` of a chain aligns with the opening
                // `if` (upstream's nested and later-visited elsif nodes both
                // resolve to that keyword), or with the assignment it is the
                // value of. Upstream hands that base down the chain only up
                // to a branch keyword that does not begin its line. Ternaries
                // and elsif nodes themselves are not chain heads.
                let head = node.as_if_node().unwrap();
                let Some(keyword) = head.if_keyword_loc().filter(|k| k.as_slice() == b"if") else {
                    return;
                };
                let keyword = keyword.start_offset()..keyword.end_offset();
                let mut base = self.if_base(node, cx, file, keyword.clone());
                let mut next = head.subsequent();
                while let Some(branch) = next {
                    if let Some(elsif) = branch.as_if_node() {
                        let Some(branch_keyword) = elsif.if_keyword_loc() else {
                            return;
                        };
                        let branch_start = branch_keyword.start_offset();
                        check(base.clone(), branch_keyword);
                        if !starts_line(file, branch_start) {
                            base = keyword.clone();
                        }
                        next = elsif.subsequent();
                    } else if let Some(other) = branch.as_else_node() {
                        check(base.clone(), other.else_keyword_loc());
                        next = None;
                    } else {
                        next = None;
                    }
                }
            }
            Node::UnlessNode { .. } => {
                let unless = node.as_unless_node().unwrap();
                if let Some(other) = unless.else_clause() {
                    let keyword = unless.keyword_loc();
                    let keyword = keyword.start_offset()..keyword.end_offset();
                    let base = self.if_base(node, cx, file, keyword);
                    check(base, other.else_keyword_loc());
                }
            }
            Node::CaseNode { .. } => {
                let case = node.as_case_node().unwrap();
                let last = case
                    .conditions()
                    .iter()
                    .filter_map(|c| c.as_when_node())
                    .last();
                if let (Some(other), Some(when)) = (case.else_clause(), last) {
                    let keyword = when.keyword_loc();
                    check(
                        keyword.start_offset()..keyword.end_offset(),
                        other.else_keyword_loc(),
                    );
                }
            }
            Node::CaseMatchNode { .. } => {
                let case = node.as_case_match_node().unwrap();
                let last = case
                    .conditions()
                    .iter()
                    .filter_map(|c| c.as_in_node())
                    .last();
                if let (Some(other), Some(clause)) = (case.else_clause(), last) {
                    let keyword = clause.in_loc();
                    check(
                        keyword.start_offset()..keyword.end_offset(),
                        other.else_keyword_loc(),
                    );
                }
            }
            Node::BeginNode { .. } => {
                let begin = node.as_begin_node().unwrap();
                let (Some(_), Some(other)) = (begin.rescue_clause(), begin.else_clause()) else {
                    return;
                };
                if let Some(base) = rescue_base(node, cx, file) {
                    check(base, other.else_keyword_loc());
                }
            }
            _ => {}
        }
    }
}

impl ElseAlignment {
    /// The base of an `if` or `unless` chain: its keyword, or the
    /// assignment or call whose value it is (`check_assignment`).
    fn if_base<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        keyword: Range<usize>,
    ) -> Range<usize> {
        match assignment_owner(node, cx.ancestors(), false, false) {
            Some(owner) if self.end_align_with.aligns_with_owner(file, owner, node) => {
                owner.location().start_offset()..owner.location().end_offset()
            }
            _ => keyword,
        }
    }

    /// `check_alignment`: an `else`/`elsif` that begins its line must share
    /// the base's character column (a line-1 BOM is not a column).
    fn misaligned<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        base: Range<usize>,
        keyword: Location<'pr>,
    ) -> Option<Offense> {
        let start = keyword.start_offset();
        if !starts_line(file, start) {
            return None;
        }
        let (line, col) = file.line_col(start);
        let delta = i64::from(file.line_col(base.start).1) - i64::from(col);
        if delta == 0 {
            return None;
        }
        // `base_range.source[/^\S*/]`.
        let word = &file.bytes[base.clone()];
        let word = &word[..word
            .iter()
            .position(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r'))
            .unwrap_or(word.len())];
        // `AlignmentCorrector.correct` on the keyword's own line, through the
        // shared guarded seam (literal interiors, `=begin`, observers).
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = cx.ancestors().next_back().unwrap_or(node);
                Alignment {
                    expression: start..keyword.end_offset(),
                    delta,
                }
                .correction(file, root)
            })
            .flatten();
        Some(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Align `{}` with `{}`.",
                String::from_utf8_lossy(keyword.as_slice()),
                String::from_utf8_lossy(word),
            ),
            line,
            end_line: line,
            col,
        })
    }
}

/// `base_range_of_rescue` for a body with `rescue ... else`: explicit
/// `begin`; a definition's parent `send` selector (`private def`) or its
/// `def`; the first line of a block; or the class-like keyword.
fn rescue_base<'pr>(
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
) -> Option<Range<usize>> {
    let begin = node.as_begin_node()?;
    let range = |l: Location<'_>| l.start_offset()..l.end_offset();
    if let Some(keyword) = begin.begin_keyword_loc() {
        return Some(range(keyword));
    }
    let mut ancestors = cx.ancestors();
    let owner = ancestors.next()?;
    match owner {
        Node::DefNode { .. } => {
            let def = owner.as_def_node().unwrap();
            let parent = ancestors.next();
            let call = match parent {
                Some(Node::ArgumentsNode { .. }) => ancestors.next().and_then(|n| n.as_call_node()),
                Some(Node::CallNode { .. }) => parent.and_then(|n| n.as_call_node()).filter(|c| {
                    c.receiver().is_some_and(|r| {
                        r.location().start_offset() == owner.location().start_offset()
                    })
                }),
                _ => None,
            };
            Some(
                match call
                    .filter(|c| !c.is_safe_navigation())
                    .and_then(|c| c.message_loc())
                {
                    Some(selector) => range(selector),
                    None => range(def.def_keyword_loc()),
                },
            )
        }
        Node::BlockNode { .. } => {
            let call = ancestors.next()?;
            Some(start_line_range(file, call.location().start_offset()))
        }
        Node::LambdaNode { .. } => Some(start_line_range(file, owner.location().start_offset())),
        Node::ClassNode { .. } => Some(range(owner.as_class_node().unwrap().class_keyword_loc())),
        Node::ModuleNode { .. } => {
            Some(range(owner.as_module_node().unwrap().module_keyword_loc()))
        }
        Node::SingletonClassNode { .. } => Some(range(
            owner.as_singleton_class_node().unwrap().class_keyword_loc(),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/else_alignment_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The
    //! spec's default `Layout/EndAlignment: variable` examples are replayed at
    //! the fixed `keyword` style; the results match the fixed replay recorded
    //! in the pinned RuboCop reference run.

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
            .filter(|o| o.cop == "Layout/ElseAlignment")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    /// Every pinned example without an offense at the fixed defaults.
    const ACCEPTED: &[&str] = &[
        // 8 accepts a ternary if
        "cond ? func1 : func2",
        // 51 with if statement accepts indentation after else when if is on new line after assignment
        "Rails.application.config.ideal_postcodes_key =\n  if Rails.env.production? || Rails.env.staging?\n    \"AAAA-AAAA-AAAA-AAAA\"\n  else\n    \"BBBB-BBBB-BBBB-BBBB\"\n  end\n",
        // 62 with if statement accepts a one line if statement
        "if cond then func1 else func2 end",
        // 66 with if statement accepts a correctly aligned if/elsif/else/end
        "if a1\n  b1\nelsif a2\n  b2\nelse\n  c\nend\n",
        // 365 with if statement accepts an if/else branches with rescue clauses
        "if a\n  a rescue nil\nelse\n  a rescue nil\nend\n",
        // 81 with if statement for a file with byte order mark accepts a correctly aligned if/elsif/els
        "\u{feff}if a1\n  b1\nelsif a2\n  b2\nelse\n  c\nend\n",
        // 119 with if statement with assignment when alignment style is variable and end is aligned with
        "foo[bar] = if baz\n  derp\nend\n",
        // 159 with if statement with assignment when alignment style is variable and end is aligned with
        "foo.bar = if baz\n            derp1\n          elsif meh\n            derp2\n          else\n            derp3\n          end\n",
        // 183 with if statement with assignment when alignment style is variable and end is aligned with
        "foo[bar] = if baz\n             derp1\n           else\n             derp2\n           end\n",
        // 202 with if statement with assignment when alignment style is variable and end is aligned with
        "var = if a\n        0\n      else\n        1\n      end\n",
        // 307 with if statement with assignment when alignment style is keyword by choice behaves like a
        "var = if a\n        0\n      end\n",
        // 325 with if statement with assignment when alignment style is keyword by choice behaves like a
        "var =\n  if a\n    0\n  else\n    1\n  end\n",
        // 336 with if statement with assignment when alignment style is keyword by choice behaves like a
        "var = while a\n        b\n      end\n",
        // 344 with if statement with assignment when alignment style is keyword by choice behaves like a
        "var = until a\n        b\n      end\n",
        // 396 with unless accepts a correctly aligned else in an otherwise empty unless
        "unless a\nelse\nend\n",
        // 404 with unless accepts an empty unless
        "unless a\nend\n",
        // 438 with case accepts correctly aligned case/when/else
        "case a\nwhen b\n  c\n  c\nwhen d\nelse\n  f\nend\n",
        // 451 with case accepts case without else
        "case superclass\nwhen /\\A(#{NAMESPACEMATCH})(?:\\s|\\Z)/\n  $1\nwhen \"self\"\n  namespace.path\nend\n",
        // 535 with case accepts else aligned with when but not with case
        "case code_type\n  when 'ruby', 'sql', 'plain'\n    code_type\n  when 'erb'\n    'ruby; html-script: true'\n  when \"html\"\n    'xml'\n  else\n    'plain'\nend\n",
        // 493 with case >= Ruby 2.7 with case match accepts correctly aligned case/when/else
        "case 0\nin 0\n  foo\nin -1..1\n  bar\nin Integer\n  baz\nelse\n  qux\nend\n",
        // 508 with case >= Ruby 2.7 with case match accepts correctly aligned empty else
        "case 0\nin 0\n  foo\nin -1..1\n  bar\nin Integer\n  baz\nelse\nend\n",
        // 522 with case >= Ruby 2.7 with case match accepts case match without else
        "case 0\nin a\n  p a\nend\n",
        // 552 with def/defs accepts an empty def body
        "def test\nend\n",
        // 559 with def/defs accepts an empty defs body
        "def self.test\nend\n",
        // 567 with def/defs when modifier and def are on the same line accepts a correctly aligned body
        "private def test\n  something\nrescue\n  handling\nelse\n  something_else\nend\n",
        // 642 with begin/rescue/else/ensure/end accepts a correctly aligned else
        "begin\n  raise StandardError.new('Fail') if rand(2).odd?\nrescue StandardError => error\n  $stderr.puts error.message\nelse\n  $stdout.puts 'Lucky you!'\nend\n",
        // 656 with def/rescue/else/ensure/end accepts a correctly aligned else
        "def my_func(string)\n  puts string\nrescue => e\n  puts e\nelse\n  puts e\nensure\n  puts 'I love methods that print'\nend\n",
        // 699 with def/rescue/else/end accepts a correctly aligned else
        "def my_func\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nrescue\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        // 742 with class/rescue/else/end accepts a correctly aligned else
        "class MyClass\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        // 779 with module/rescue/else/end accepts a correctly aligned else
        "module MyModule\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        // 816 with sclass/rescue/else/ensure/end accepts a correctly aligned else
        "class << self\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nensure\n  puts 'cleanup'\nend\n",
        // 859 ensure/rescue/else in Block Argument accepts a correctly aligned else
        "array_like.each do |n|\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nrescue\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        // 873 ensure/rescue/else in Block Argument accepts a correctly aligned else from numblock
        "array_like.each do\n  _1\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nrescue\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        // 888 ensure/rescue/else in Block Argument accepts a correctly aligned else from itblock
        "array_like.each do\n  it\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nrescue\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        // 903 ensure/rescue/else in Block Argument accepts a correctly aligned else with assignment
        "result = array_like.each do |n|\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nrescue\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        // 917 ensure/rescue/else in Block Argument accepts a correctly aligned else when the block is wi
        "foo << array_like.map do |n|\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        // 929 ensure/rescue/else in Block Argument accepts a correctly aligned else when the block is a
        "foo(array_like.map do |n|\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend)\n",
        // 941 ensure/rescue/else in Block Argument accepts a correctly aligned else with an assignment a
        "result = foo << array_like.map do |n|\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
    ];

    #[test]
    fn pinned_accepted_examples() {
        for source in ACCEPTED {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    /// (source, findings, single-pass correction) for every pinned example
    /// with an offense at the fixed defaults.
    /// (line, column, message)
    type Finding = (u32, u32, &'static str);

    const REPORTED: &[(&str, &[Finding], &str)] = &[
        (
            "if cond\n  func1\n else\n func2\nend\n",
            &[(3, 2, "Align `else` with `if`.")],
            "if cond\n  func1\nelse\n func2\nend\n",
        ),
        (
            "    if a1\n      b1\n  elsif a2\n      b2\n    end\n",
            &[(3, 3, "Align `elsif` with `if`.")],
            "    if a1\n      b1\n    elsif a2\n      b2\n    end\n",
        ),
        // The spec's `variable`-style accepted assignments.
        (
            "foo.bar = if baz\n  derp1\nelse\n  derp2\nend\n",
            &[(3, 1, "Align `else` with `if`.")],
            "foo.bar = if baz\n  derp1\n          else\n  derp2\nend\n",
        ),
        (
            "foo.bar = if baz\n  derp1\nelsif meh\n  derp2\nelse\n  derp3\nend\n",
            &[
                (3, 1, "Align `elsif` with `if`."),
                (5, 1, "Align `else` with `if`."),
            ],
            "foo.bar = if baz\n  derp1\n          elsif meh\n  derp2\n          else\n  derp3\nend\n",
        ),
        (
            "var = if a\n  0\nelse\n  1\nend\n",
            &[(3, 1, "Align `else` with `if`.")],
            "var = if a\n  0\n      else\n  1\nend\n",
        ),
        (
            "var = if a\n  0\nelse\n  1\nend.abc.join(\"\")\n",
            &[(3, 1, "Align `else` with `if`.")],
            "var = if a\n  0\n      else\n  1\nend.abc.join(\"\")\n",
        ),
        (
            "var = if a\n  0\nelse\n  1\nend.abc.tap {}\n",
            &[(3, 1, "Align `else` with `if`.")],
            "var = if a\n  0\n      else\n  1\nend.abc.tap {}\n",
        ),
        // The keyword-style shared examples.
        (
            "var = if a\n  0\nelse b\n  1\nend\n",
            &[(3, 1, "Align `else` with `if`.")],
            "var = if a\n  0\n      else b\n  1\nend\n",
        ),
        (
            "var = if a\n  0\nelsif b\n  1\nend\n",
            &[(3, 1, "Align `elsif` with `if`.")],
            "var = if a\n  0\n      elsif b\n  1\nend\n",
        ),
        (
            "var = case condition\n      when a\n        0\nelse\n  1\nend\n",
            &[(4, 1, "Align `else` with `when`.")],
            "var = case condition\n      when a\n        0\n      else\n  1\nend\n",
        ),
        (
            "var = case expr\n      in a\n        0\nelse\n  1\nend\n",
            &[(4, 1, "Align `else` with `in`.")],
            "var = case expr\n      in a\n        0\n      else\n  1\nend\n",
        ),
        (
            "unless cond\n   func1\n else\n   func2\nend\n",
            &[(3, 2, "Align `else` with `unless`.")],
            "unless cond\n   func1\nelse\n   func2\nend\n",
        ),
        (
            "case a\nwhen b\n  c\nwhen d\n  e\n else\n  f\nend\n",
            &[(6, 2, "Align `else` with `when`.")],
            "case a\nwhen b\n  c\nwhen d\n  e\nelse\n  f\nend\n",
        ),
        (
            "case 0\nin 0\n  foo\nin -1..1\n  bar\nin Integer\n  baz\n else\n  qux\nend\n",
            &[(8, 2, "Align `else` with `in`.")],
            "case 0\nin 0\n  foo\nin -1..1\n  bar\nin Integer\n  baz\nelse\n  qux\nend\n",
        ),
        (
            "private def test\n          something\n        rescue\n          handling\n        else\n          something_else\n        end\n",
            &[(5, 9, "Align `else` with `private`.")],
            "private def test\n          something\n        rescue\n          handling\nelse\n          something_else\n        end\n",
        ),
        (
            "def my_func\n  puts 'do something outside block'\n  begin\n    puts 'do something error prone'\n  rescue SomeException, SomeOther => e\n    puts 'wrongly intended error handling'\n  rescue\n    puts 'wrongly intended error handling'\nelse\n    puts 'wrongly intended normal case handling'\n  ensure\n    puts 'wrongly intended common handling'\n  end\nend\n",
            &[(9, 1, "Align `else` with `begin`.")],
            "def my_func\n  puts 'do something outside block'\n  begin\n    puts 'do something error prone'\n  rescue SomeException, SomeOther => e\n    puts 'wrongly intended error handling'\n  rescue\n    puts 'wrongly intended error handling'\n  else\n    puts 'wrongly intended normal case handling'\n  ensure\n    puts 'wrongly intended common handling'\n  end\nend\n",
        ),
        (
            "def my_func(string)\n  puts string\nrescue => e\n  puts e\n  else\n  puts e\nensure\n  puts 'I love methods that print'\nend\n",
            &[(5, 3, "Align `else` with `def`.")],
            "def my_func(string)\n  puts string\nrescue => e\n  puts e\nelse\n  puts e\nensure\n  puts 'I love methods that print'\nend\n",
        ),
        (
            "def my_func\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nrescue\n  puts 'error handling'\n  else\n  puts 'normal handling'\nend\n",
            &[(7, 3, "Align `else` with `def`.")],
            "def my_func\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nrescue\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        ),
        (
            "class MyClass\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\n  else\n  puts 'normal handling'\nend\n",
            &[(5, 3, "Align `else` with `class`.")],
            "class MyClass\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        ),
        (
            "module MyModule\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\n  else\n  puts 'normal handling'\nend\n",
            &[(5, 3, "Align `else` with `module`.")],
            "module MyModule\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        ),
        (
            "class << self\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\n  else\n  puts 'normal handling'\nensure\n  puts 'cleanup'\nend\n",
            &[(5, 3, "Align `else` with `class`.")],
            "class << self\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nensure\n  puts 'cleanup'\nend\n",
        ),
        (
            "foo << array_like.map do |n|\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\n  else\n  puts 'normal handling'\nend\n",
            &[(5, 3, "Align `else` with `foo`.")],
            "foo << array_like.map do |n|\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        ),
        (
            "array_like.each do |n|\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nrescue\n  puts 'error handling'\n  else\n  puts 'normal handling'\nend\n",
            &[(7, 3, "Align `else` with `array_like.each`.")],
            "array_like.each do |n|\n  puts 'do something error prone'\nrescue SomeException\n  puts 'error handling'\nrescue\n  puts 'error handling'\nelse\n  puts 'normal handling'\nend\n",
        ),
    ];

    #[test]
    fn pinned_reported_examples_and_single_pass_corrections() {
        for (before, expected, after) in REPORTED {
            let found = positions(before);
            let expected: Vec<_> = expected
                .iter()
                .map(|&(l, c, m)| (l, c, m.to_owned()))
                .collect();
            assert_eq!(found, expected, "{before}");
            assert_correction(&ElseAlignment::default(), before, after);
        }
        let found = offenses(REPORTED[0].0);
        assert_eq!(found[0].severity, Severity::Convention);
        assert_eq!(found[0].end_line, 3);
    }

    #[test]
    fn elsif_chains_align_with_the_opening_if() {
        for source in [
            "if a\n  1\nelsif b\n  2\nelsif c\n  3\nelse\n  4\nend\n",
            "if a then 1 elsif b then 2\nelse 3\nend\n",
            "if a\n  1\nelse\n  if b\n    2\n  else\n    3\n  end\nend\n",
            "x = a ?\n  b :\n  c\n",
            "x = (a ?\n  b\n: c)\n",
            "foo if bar\n",
            "while a\n  b\nend\n",
            "case a\nwhen 1 then 2 else 3\nend\n",
            "begin\n  a\nrescue\n  b\nelse c\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // A later elsif/else is still checked after an inline elsif.
        let source = "if a\n  1\nelsif b then 2 elsif c\n  3\n  else\n  4\nend\n";
        assert_eq!(
            positions(source),
            [(5, 3, "Align `else` with `if`.".into())]
        );
        let source = "if a\n  1\n  elsif b\n  2\n    elsif c\n  3\n else\nend\n";
        assert_eq!(
            positions(source),
            [
                (3, 3, "Align `elsif` with `if`.".into()),
                (5, 5, "Align `elsif` with `if`.".into()),
                (7, 2, "Align `else` with `if`.".into()),
            ]
        );
        assert_correction(
            &ElseAlignment::default(),
            source,
            "if a\n  1\nelsif b\n  2\nelsif c\n  3\nelse\nend\n",
        );
    }

    #[test]
    fn rescue_else_bases_follow_the_owner() {
        for (source, base, col) in [
            ("x = begin\n  a\nrescue\n  b\nelse\n  c\nend\n", "begin", 1),
            (
                "foo bar def baz\n  a\nrescue\n  b\nelse\n  c\nend\n",
                "bar",
                1,
            ),
            (
                "obj.private def baz\n  a\nrescue\n  b\n  else\n  c\nend\n",
                "private",
                3,
            ),
            ("x = def baz\n  a\nrescue\n  b\n  else\nend\n", "def", 3),
            ("-> do\n  a\nrescue\n  b\n  else\nend\n", "->", 3),
            ("super do\n  a\nrescue\n  b\n  else\nend\n", "super", 3),
            (
                "super(1) do\n  a\nrescue\n  b\n  else\nend\n",
                "super(1)",
                3,
            ),
            ("def self.f\n  a\nrescue\n  b\n  else\nend\n", "def", 3),
        ] {
            let found = positions(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                found[0].2,
                format!("Align `else` with `{base}`."),
                "{source}"
            );
            assert_eq!(found[0].1, col, "{source}");
        }
        assert_correction(
            &ElseAlignment::default(),
            "x = begin\n  a\nrescue\n  b\nelse\n  c\nend\n",
            "x = begin\n  a\nrescue\n  b\n    else\n  c\nend\n",
        );
    }

    #[test]
    fn columns_are_characters_and_bom_is_ignored() {
        for source in [
            "\u{feff}if a\n  1\nelse\n  2\nend\n",
            "é = 1; if a\n  1\n       else\n  2\nend\n",
            "\tif a\n\t\t1\n\telse\n\t\t2\n\tend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let before = "\u{feff}if a\r\n  1\r\n  else\r\n  2\r\nend\r\n";
        assert_eq!(
            positions(before),
            [(3, 3, "Align `else` with `if`.".into())]
        );
        assert_correction(
            &ElseAlignment::default(),
            before,
            "\u{feff}if a\r\n  1\r\nelse\r\n  2\r\nend\r\n",
        );
        // A tab before `else` is one column; shifting left past it is held.
        let before = "  if a\n    1\n\telse\n    2\n  end\n";
        assert_eq!(
            positions(before),
            [(3, 2, "Align `else` with `if`.".into())]
        );
        assert_correction(
            &ElseAlignment::default(),
            before,
            "  if a\n    1\n\t else\n    2\n  end\n",
        );
        let before = "if a\n  1\n\telse\n  2\nend\n";
        assert_eq!(
            positions(before),
            [(3, 2, "Align `else` with `if`.".into())]
        );
        assert!(
            correction_offenses(&ElseAlignment::default(), before)[0]
                .correction
                .is_none()
        );
    }

    #[test]
    fn literal_interiors_keep_their_bytes() {
        let source = "x = <<~H\n  #{if a\n    1\n      else\n    2\n    end}\nH\n";
        let found = correction_offenses(&ElseAlignment::default(), source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        for source in [
            "x = \"if a\n  else\nend\"\n",
            "# if a\n  # else\n",
            "x = %w[if\n  else]\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn both_directive_spellings_disable_the_finding() {
        for prefix in ["rustocop", "rubocop"] {
            for target in ["Layout/ElseAlignment", "Layout", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\nif x\n  1\n else\nend\n"),
                    format!("if x\n  1\n else # {prefix}:disable {target}\nend\n"),
                ] {
                    assert!(offenses(&source).is_empty(), "{source}");
                    assert!(correction_offenses(&ElseAlignment::default(), &source).is_empty());
                }
            }
        }
    }
}
