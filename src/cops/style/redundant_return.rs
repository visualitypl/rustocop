//! `Style/RedundantReturn` — flags a `return` that is the last thing a
//! method (or `define_method` block, or lambda) evaluates.
//!
//! Matches RuboCop's semantics. With `allow_multiple_return_values`
//! (RuboCop's `AllowMultipleReturnValues`, off by default) a `return` of
//! several values is accepted. The bodies checked are those of `def`
//! and `def obj.name`, of a literal block passed to `define_method`,
//! `define_singleton_method` or `lambda` (any receiver, not `&.`), and of a
//! `->` lambda. From such a body, the final expression is followed through
//! the branches that produce the value: the last statement of a sequence,
//! `begin…end` and `(…)`; both branches of `if`/`unless` (not a ternary)
//! and every `elsif`; every `when`/`in` body and the `else` of a
//! `case`; the `rescue` bodies and the `else` of a `rescue` (the main body
//! only when there is no `else`), and both sides of a `rescue` modifier;
//! the main body under an `ensure`. A `return` found there is reported at
//! its keyword; with several values the message suggests an array.

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        settings::{Kind, Setting, Values},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Style/RedundantReturn` cop.
#[derive(Default)]
pub struct RedundantReturn {
    /// `allow_multiple_return_values`.
    allow_multiple: bool,
}

/// Methods whose literal block is a method or lambda body.
const BODY_METHODS: [&[u8]; 3] = [b"define_method", b"define_singleton_method", b"lambda"];

impl Cop for RedundantReturn {
    fn name(&self) -> &'static str {
        "Style/RedundantReturn"
    }

    fn settings(&self) -> &'static [Setting] {
        const ALLOW_MULTIPLE_RETURN_VALUES: Setting = Setting {
            name: "allow_multiple_return_values",
            kind: Kind::Flag,
        };
        &[ALLOW_MULTIPLE_RETURN_VALUES]
    }

    fn configure(&self, values: &Values) -> Option<Box<dyn Cop>> {
        Some(Box::new(RedundantReturn {
            allow_multiple: values
                .flag("allow_multiple_return_values")
                .unwrap_or(self.allow_multiple),
        }))
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let body = match node {
            Node::DefNode { .. } => node.as_def_node().unwrap().body(),
            Node::LambdaNode { .. } => node.as_lambda_node().unwrap().body(),
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if call.is_safe_navigation() || !BODY_METHODS.contains(&call.name().as_slice()) {
                    return;
                }
                match call.block().and_then(|block| block.as_block_node()) {
                    Some(block) => block.body(),
                    None => return,
                }
            }
            _ => return,
        };
        if let Some(body) = body {
            self.check_branch(&body, file, out);
        }
    }
}

impl RedundantReturn {
    /// RuboCop's `check_branch`: follow the expression that produces the
    /// value of `node` to the `return`s there.
    fn check_branch(&self, node: &Node<'_>, file: &SourceFile, out: &mut Vec<Offense>) {
        match node {
            Node::ReturnNode { .. } => self.report(node, file, out),
            Node::StatementsNode { .. } => {
                if let Some(last) = node.as_statements_node().unwrap().body().iter().last() {
                    self.check_branch(&last, file, out);
                }
            }
            Node::ParenthesesNode { .. } => {
                if let Some(body) = node.as_parentheses_node().unwrap().body() {
                    self.check_branch(&body, file, out);
                }
            }
            Node::BeginNode { .. } => {
                // An `ensure` never produces the value: only what it wraps.
                let begin = node.as_begin_node().unwrap();
                let Some(rescue) = begin.rescue_clause() else {
                    self.check_optional(begin.statements().map(|s| s.as_node()), file, out);
                    return;
                };
                let mut clause = Some(rescue);
                while let Some(current) = clause {
                    self.check_optional(current.statements().map(|s| s.as_node()), file, out);
                    clause = current.subsequent();
                }
                match begin.else_clause() {
                    Some(else_clause) => self.check_optional(
                        else_clause.statements().map(|s| s.as_node()),
                        file,
                        out,
                    ),
                    None => self.check_optional(begin.statements().map(|s| s.as_node()), file, out),
                }
            }
            Node::RescueModifierNode { .. } => {
                let rescue = node.as_rescue_modifier_node().unwrap();
                self.check_branch(&rescue.rescue_expression(), file, out);
                self.check_branch(&rescue.expression(), file, out);
            }
            Node::IfNode { .. } => {
                let if_node = node.as_if_node().unwrap();
                if if_node.if_keyword_loc().is_none() {
                    return; // A ternary.
                }
                self.check_optional(if_node.statements().map(|s| s.as_node()), file, out);
                self.check_optional(if_node.subsequent(), file, out);
            }
            Node::UnlessNode { .. } => {
                let unless = node.as_unless_node().unwrap();
                self.check_optional(unless.statements().map(|s| s.as_node()), file, out);
                self.check_optional(unless.else_clause().map(|e| e.as_node()), file, out);
            }
            Node::ElseNode { .. } => {
                let statements = node.as_else_node().unwrap().statements();
                self.check_optional(statements.map(|s| s.as_node()), file, out);
            }
            Node::CaseNode { .. } => {
                let case = node.as_case_node().unwrap();
                for condition in case.conditions().iter() {
                    if let Some(when) = condition.as_when_node() {
                        self.check_optional(when.statements().map(|s| s.as_node()), file, out);
                    }
                }
                self.check_optional(case.else_clause().map(|e| e.as_node()), file, out);
            }
            Node::CaseMatchNode { .. } => {
                let case = node.as_case_match_node().unwrap();
                for condition in case.conditions().iter() {
                    if let Some(in_node) = condition.as_in_node() {
                        self.check_optional(in_node.statements().map(|s| s.as_node()), file, out);
                    }
                }
                self.check_optional(case.else_clause().map(|e| e.as_node()), file, out);
            }
            _ => {}
        }
    }

    fn check_optional(&self, node: Option<Node<'_>>, file: &SourceFile, out: &mut Vec<Offense>) {
        if let Some(node) = node {
            self.check_branch(&node, file, out);
        }
    }

    fn report(&self, node: &Node<'_>, file: &SourceFile, out: &mut Vec<Offense>) {
        let return_node = node.as_return_node().unwrap();
        let values = return_node
            .arguments()
            .map_or(0, |arguments| arguments.arguments().iter().count());
        if self.allow_multiple && values > 1 {
            return;
        }
        let message = if values > 1 {
            "Redundant `return` detected. To return multiple values, use an array."
        } else {
            "Redundant `return` detected."
        };
        let (line, col) = file.line_col(return_node.keyword_loc().start_offset());
        out.push(Offense {
            correction: return_correction(&return_node, file),
            cop: self.name(),
            severity: self.severity(),
            message: message.to_string(),
            line,
            end_line: line,
            col,
        });
    }
}

/// Keep coordinated keyword removal and value wrapping atomic. In particular,
/// bare returns must become nil (not the preceding statement's value).
fn return_correction(node: &ruby_prism::ReturnNode<'_>, file: &SourceFile) -> Option<Correction> {
    let loc = node.location();
    let arguments: Vec<_> = node
        .arguments()
        .map(|args| args.arguments().iter().collect())
        .unwrap_or_default();
    let empty_parentheses = arguments.len() == 1
        && arguments[0]
            .as_parentheses_node()
            .is_some_and(|paren| paren.body().is_none());
    if arguments.is_empty() || empty_parentheses {
        // Unusual commented `return( ... )`: leave it alone rather than drop comments.
        if file
            .comments()
            .iter()
            .any(|comment| comment.start < loc.end_offset() && comment.end > loc.start_offset())
        {
            return None;
        }
        let correction = Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement: "nil".into(),
        };
        return Some(correction);
    }
    let mut end = node.keyword_loc().end_offset();
    while file
        .bytes
        .get(end)
        .is_some_and(|b| matches!(b, b' ' | b'\t'))
    {
        end += 1;
    }
    while file.bytes.get(end) == Some(&b'\n') {
        end += 1;
    }
    let mut edits = vec![Correction {
        range: loc.start_offset()..end,
        replacement: String::new(),
    }];
    let first = &arguments[0];
    // `splat_argument?`: the first argument loses its `*`.
    if arguments.iter().any(|a| a.as_splat_node().is_some())
        && let Some(splat) = first.as_splat_node()
    {
        let star = splat.operator_loc();
        edits.push(Correction {
            range: star.start_offset()..star.end_offset(),
            replacement: String::new(),
        });
    }
    let wrapper = if arguments.len() > 1 {
        Some(("[", "]"))
    } else if first.as_keyword_hash_node().is_some() {
        Some(("{", "}"))
    } else {
        None
    };
    if let Some((open, close)) = wrapper {
        let start = first.location().start_offset();
        let end = arguments.last()?.location().end_offset();
        edits.push(Correction {
            range: start..start,
            replacement: open.into(),
        });
        edits.push(Correction {
            range: end..end,
            replacement: close.into(),
        });
    }
    super::correction::combine(file, edits)
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/style/redundant_return_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! `AllowMultipleReturnValues: false`.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantReturn")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn corrects_returns_preserving_values_and_comments() {
        // RuboCop expect_correction examples (MIT), AllowMultipleReturnValues: false.
        for (before, after) in [
            (
                "def func\n  return something\nensure\n  2\nend\n",
                "def func\n  something\nensure\n  2\nend\n",
            ),
            (
                "def func\n  one\n  return()\n  # comment\nend\n",
                "def func\n  one\n  nil\n  # comment\nend\n",
            ),
            ("def func\n  return\nend\n", "def func\n  nil\nend\n"),
            (
                "def func\n  return (foo && bar), 42\nend\n",
                "def func\n  [(foo && bar), 42]\nend\n",
            ),
            (
                "def func\n  return :a => 1, :b => 2\nend\n",
                "def func\n  {:a => 1, :b => 2}\nend\n",
            ),
            (
                "def func\n  return something if other\nend\n",
                "def func\n  something if other\nend\n",
            ),
            ("-> { return(1) }\n", "-> { (1) }\n"),
            (
                "def func\n  return [ # comment\n    1\n  ] # trailing\nend\n",
                "def func\n  [ # comment\n    1\n  ] # trailing\nend\n",
            ),
            (
                "def func\n  return <<~TEXT\n    text\n  TEXT\nend\n",
                "def func\n  <<~TEXT\n    text\n  TEXT\nend\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&RedundantReturn::default(), before, after);
        }
    }

    #[test]
    fn leaves_multiline_empty_returns_when_later_magic_line_values_would_change() {
        for source in [
            "def foo\n  return(\n  )\nend\np __LINE__\n",
            "def foo\r\n  return(\r\n  )\r\nend\np __LINE__\r\n",
            "def foo\n  return(\n  )\nend\ndef bar\n  __LINE__\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        crate::cops::testing::assert_correction(
            &RedundantReturn::default(),
            "def foo\n  return()\nend\np __LINE__\n",
            "def foo\n  nil\nend\np __LINE__\n",
        );
    }

    #[test]
    fn leaves_commented_empty_returns_uncorrected() {
        let source = "def func\n  return( # comment\n  )\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn flags_return_with_message() {
        let found = offenses("def func\n  return something\nensure\n  2\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 3));
        assert_eq!(found[0].message, "Redundant `return` detected.");
    }

    #[test]
    fn flags_multiple_values_with_message() {
        for source in [
            "def func\n  return something, test\nend\n",
            "def Test.func\n  return something, test\nend\n",
            "def func\n  one\n  two\n  return something, test\nend\n",
            "def self.func\n  one\n  two\n  return something, test\nend\n",
            "def do_something\n  return (foo && bar), 42\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!(
                found[0].message,
                "Redundant `return` detected. To return multiple values, use an array.",
                "{source:?}"
            );
        }
        for source in [
            "def func\n  return {:a => 1, :b => 2}\nend\n",
            "def func\n  return :a => 1, :b => 2\nend\n",
            "def func\n  some_preceding_statements\n  return *something\nend\n",
            "def func\n  one\n  two\n  return()\n  # comment\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!(
                found[0].message, "Redundant `return` detected.",
                "{source:?}"
            );
        }
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for (source, expected) in [
            ("def Test.func\n  return something\nend\n", vec![(2, 3)]),
            (
                "def func\n  some_preceding_statements\n  return something\nend\n",
                vec![(3, 3)],
            ),
            (
                "define_method(:foo) do\n  return something\nend\n",
                vec![(2, 3)],
            ),
            (
                "define_singleton_method(:foo) do\n  return something\nend\n",
                vec![(2, 3)],
            ),
            (
                "define_method(:foo) do\n  some_preceding_statements\n  return something\nend\n",
                vec![(3, 3)],
            ),
            (
                "lambda do\n  some_preceding_statements\n  return something\nend\n",
                vec![(3, 3)],
            ),
            (
                "-> do\n  some_preceding_statements\n  return something\nend\n",
                vec![(3, 3)],
            ),
            (
                "def self.func\n  some_preceding_statements\n  return something\nend\n",
                vec![(3, 3)],
            ),
            (
                "def func\n  return something if something_else\nend\n",
                vec![(2, 3)],
            ),
            (
                "def func\n  one\n  two\n  return\n  # comment\nend\n",
                vec![(4, 3)],
            ),
            (
                "def func\n  some_preceding_statements\n  begin\n    return 1\n  end\nend\n",
                vec![(4, 5)],
            ),
            (
                "def func\n  1\n  2\n  return 3\nrescue SomeException\n  4\n  return 5\n\
                 rescue AnotherException\n  return 6\nensure\n  return 7\nend\n",
                vec![(4, 3), (7, 3), (9, 3)],
            ),
            (
                "def func\n  return 3\nrescue SomeException\nelse\n  return 4\nend\n",
                vec![(5, 3)],
            ),
            (
                "def func\n  some_preceding_statements\n  if x\n    return 1\n  elsif y\n    \
                 return 2\n  else\n    return 3\n  end\nend\n",
                vec![(4, 5), (6, 5), (8, 5)],
            ),
            (
                "def func\n  some_preceding_statements\n  case x\n  when y then return 1\n  \
                 when z then return 2\n  when q\n  else\n    return 3\n  end\nend\n",
                vec![(4, 15), (5, 15), (8, 5)],
            ),
            (
                "def func\n  some_preceding_statements\n  case x\n  in y then return 1\n  \
                 in z then return 2\n  in q\n  else\n    return 3\n  end\nend\n",
                vec![(4, 13), (5, 13), (8, 5)],
            ),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_examples() {
        for source in [
            "proc do\n  some_preceding_statements\n  return something\nend\n",
            "def func\n  something if something_else\nend\n",
            "def func\nend\n",
            "def func\n  if x\n  elsif y\n  else\n  end\nend\n",
            "def func\n  case x\n  when y then 1\n  when z # do nothing\n  else\n    3\n  end\nend\n",
            "def func\n  case x\n  in y then 1\n  in z # do nothing\n  else\n    3\n  end\nend\n",
            "lambda.call(block)\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn flags_other_final_returns() {
        for (source, expected) in [
            ("def f\n  (return 1)\nend\n", vec![(2, 4)]),
            ("def f = (return 1)\n", vec![(1, 10)]),
            (
                "def f\n  unless x\n    return 1\n  else\n    return 2\n  end\nend\n",
                vec![(3, 5), (5, 5)],
            ),
            ("def f\n  return x rescue nil\nend\n", vec![(2, 3)]),
            ("def f\n  x rescue return 1\nend\n", vec![(2, 12)]),
            (
                "def f\n  begin\n    a\n  rescue\n    return b\n  end\nend\n",
                vec![(5, 5)],
            ),
            ("def f\n  a\nensure\n  return b\nend\n", vec![]),
            ("foo.lambda { return 1 }\n", vec![(1, 14)]),
            ("define_method(:x) { return _1 }\n", vec![(1, 21)]),
            ("define_method(:x) { return it }\n", vec![(1, 21)]),
            ("-> { return 1 }\n", vec![(1, 6)]),
            (
                "def f\n  if a\n    if b\n      return 1\n    end\n  end\nend\n",
                vec![(4, 7)],
            ),
            ("class A\n  def f\n    return 1\n  end\nend\n", vec![(3, 5)]),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_lookalikes() {
        for source in [
            "def f\n  return 1 if x\n  2\nend\n",
            "def f\n  x ? (return 1) : 2\nend\n",
            "def f\n  foo { return 1 }\nend\n",
            "def f\n  items.each do |i|\n    return i\n  end\nend\n",
            "def f\n  while x\n    return 1\n  end\nend\n",
            "obj&.define_method(:x) { return 1 }\n",
            "define_method(:x, &blk)\n",
            "def f\n  return 1\n  2\nend\n",
            "def f\n  a && return\nend\n",
            "return 1\n",
            "foo { return 1 }\n",
            "def f\n  begin\n    return 1\n  end while x\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn allow_multiple_return_values_accepts_several_values() {
        let cop = RedundantReturn {
            allow_multiple: true,
        };
        let found = |source: &str| crate::pipeline::lint_str_with(&[&cop], source);
        assert!(
            found(
                "def pair
  return 1, 2
end
"
            )
            .is_empty()
        );
        assert!(
            found(
                "def splat
  return 1, *rest
end
"
            )
            .is_empty()
        );
        for source in [
            "def one
  return 1
end
",
            "def list
  return [1, 2]
end
",
        ] {
            let offenses = found(source);
            assert_eq!(offenses.len(), 1, "{source}");
            assert_eq!(offenses[0].message, "Redundant `return` detected.");
        }
    }

    #[test]
    fn honors_directives() {
        assert!(
            offenses("def f\n  return 1 # rubocop:disable Style/RedundantReturn\nend\n").is_empty()
        );
    }
}
