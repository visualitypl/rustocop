//! `Lint/AmbiguousOperator` — flags a unary `+`, `-`, `*`, `**` or `&` that
//! starts the first argument of a call without parentheses (`puts *args`,
//! `do_something -42`), where it reads like a binary operator.
//!
//! Matches RuboCop's semantics, which are driven by the parser's own
//! diagnostic: a place Prism warns about an ambiguous first argument or
//! argument prefix is reported, at the operator, when RuboCop finds the node
//! it belongs to:
//!
//! - a splat, keyword splat or block argument with an operand, or any block
//!   argument (`foo &`), under any call, `super` or `yield`;
//! - otherwise the first argument of a call without safe navigation: a
//!   unary `+`/`-` operand (`foo -x`), or an anonymous `*`/`**` forward
//!   (`bar *`). So `foo&.bar -1` and `super *` are not reported.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/AmbiguousOperator` cop.
pub struct AmbiguousOperator;

/// An ambiguous prefix operator Prism warns about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prefix {
    Plus,
    Minus,
    Star,
    StarStar,
    Ampersand,
}

impl Prefix {
    /// The prefix Prism's warning `message` is about, if it is one of these.
    /// ruby-prism's Rust API exposes no warning ids, so warnings are told
    /// apart by their fixed message; the tests fail if an upgrade rewords
    /// one.
    fn from_warning(message: &str) -> Option<Self> {
        match message {
            "ambiguous first argument; put parentheses or a space even after `+` operator" => {
                Some(Prefix::Plus)
            }
            "ambiguous first argument; put parentheses or a space even after `-` operator" => {
                Some(Prefix::Minus)
            }
            "ambiguous `*` has been interpreted as an argument prefix" => Some(Prefix::Star),
            "ambiguous `**` has been interpreted as an argument prefix" => Some(Prefix::StarStar),
            "ambiguous `&` has been interpreted as an argument prefix" => Some(Prefix::Ampersand),
            _ => None,
        }
    }

    /// The operator, what it is, and what it could have meant.
    fn describe(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Prefix::Plus => ("+", "positive number", "an addition"),
            Prefix::Minus => ("-", "negative number", "a subtraction"),
            Prefix::Star => ("*", "splat", "a multiplication"),
            Prefix::StarStar => ("**", "keyword splat", "an exponent"),
            Prefix::Ampersand => ("&", "block", "a binary AND"),
        }
    }

    fn message(self) -> String {
        let (operator, actual, possible) = self.describe();
        format!(
            "Ambiguous {actual} operator. Parenthesize the method arguments if it's surely a \
             {actual} operator, or add a whitespace to the right of the `{operator}` if it \
             should be {possible}."
        )
    }
}

/// RuboCop's `add_parentheses` on the call taking the ambiguous argument.
fn add_parentheses(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let call = if node.as_call_node().is_some() {
        node
    } else {
        cx.ancestors()
            .find(|n| n.as_arguments_node().is_none() && n.as_keyword_hash_node().is_none())?
    };
    let (selector_end, end) = if let Some(c) = call.as_call_node() {
        (
            c.message_loc()?.end_offset(),
            crate::cops::ast::source_end(call),
        )
    } else if let Some(s) = call.as_super_node() {
        let end = s
            .arguments()
            .map_or(s.keyword_loc().end_offset(), |a| a.location().end_offset());
        let end = s
            .block()
            .filter(|b| b.as_block_argument_node().is_some())
            .map_or(end, |b| b.location().end_offset().max(end));
        (s.keyword_loc().end_offset(), end)
    } else {
        let y = call.as_yield_node()?;
        (y.keyword_loc().end_offset(), y.location().end_offset())
    };
    Some(crate::offense::Correction {
        range: selector_end..end,
        replacement: format!(
            "({})",
            std::str::from_utf8(file.bytes.get(selector_end + 1..end)?).ok()?
        ),
    })
}

impl Cop for AmbiguousOperator {
    fn name(&self) -> &'static str {
        "Lint/AmbiguousOperator"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if file.warnings().is_empty() {
            return;
        }
        let start = node.location().start_offset();
        // Each warning has exactly one trigger node, so nothing is reported
        // twice.
        let hit = match node {
            Node::SplatNode { .. } => node
                .as_splat_node()
                .unwrap()
                .expression()
                .and(prefix_at(file, start))
                .filter(|prefix| *prefix == Prefix::Star)
                .map(|prefix| (prefix, start)),
            Node::AssocSplatNode { .. } => node
                .as_assoc_splat_node()
                .unwrap()
                .value()
                .and(prefix_at(file, start))
                .filter(|prefix| *prefix == Prefix::StarStar)
                .map(|prefix| (prefix, start)),
            Node::BlockArgumentNode { .. } => prefix_at(file, start)
                .filter(|prefix| *prefix == Prefix::Ampersand)
                .map(|prefix| (prefix, start)),
            Node::CallNode { .. } => first_argument_prefix(file, &node.as_call_node().unwrap()),
            _ => None,
        };
        let Some((prefix, offset)) = hit else {
            return;
        };
        let (line, col) = file.line_col(offset);
        out.push(Offense {
            correction: cx
                .corrections_requested()
                .then(|| add_parentheses(node, cx, file))
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message: prefix.message(),
            line,
            end_line: line,
            col,
        });
    }
}

/// The ambiguous prefix Prism warned about at `offset`, if any.
fn prefix_at(file: &SourceFile, offset: usize) -> Option<Prefix> {
    let warnings = file.warnings();
    let first = warnings.partition_point(|warning| warning.range.start < offset);
    warnings[first..]
        .iter()
        .take_while(|warning| warning.range.start == offset)
        .find_map(|warning| Prefix::from_warning(&warning.message))
}

/// The prefix to report for a call's first argument, and where: a unary
/// `+`/`-` operand, or an anonymous `*`/`**` forward. Calls with safe
/// navigation are skipped, as RuboCop only looks at `send` nodes here.
fn first_argument_prefix(
    file: &SourceFile,
    call: &ruby_prism::CallNode<'_>,
) -> Option<(Prefix, usize)> {
    if call.is_safe_navigation() {
        return None;
    }
    let argument = call.arguments()?.arguments().iter().next()?;
    let offset = argument.location().start_offset();
    let prefix = prefix_at(file, offset)?;
    let reported = match prefix {
        Prefix::Plus | Prefix::Minus => true,
        Prefix::Star => argument
            .as_splat_node()
            .is_some_and(|splat| splat.expression().is_none()),
        Prefix::StarStar => argument.as_keyword_hash_node().is_some_and(|hash| {
            hash.elements().iter().next().is_some_and(|element| {
                element
                    .as_assoc_splat_node()
                    .is_some_and(|splat| splat.value().is_none())
            })
        }),
        Prefix::Ampersand => false,
    };
    reported.then_some((prefix, offset))
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/ambiguous_operator_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/AmbiguousOperator")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_unary_plus_with_message() {
        let found = offenses("do_something(+24)\ndo_something +42\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 14));
        assert_eq!(
            found[0].message,
            "Ambiguous positive number operator. Parenthesize the method arguments if it's \
             surely a positive number operator, or add a whitespace to the right of the `+` if \
             it should be an addition."
        );
        assert_eq!(positions("do_something\ndo_something +42\n"), vec![(2, 14)]);
    }

    #[test]
    fn flags_unary_minus_with_message() {
        let found = offenses("do_something(-24)\ndo_something -42\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 14));
        assert_eq!(
            found[0].message,
            "Ambiguous negative number operator. Parenthesize the method arguments if it's \
             surely a negative number operator, or add a whitespace to the right of the `-` if \
             it should be a subtraction."
        );
    }

    #[test]
    fn flags_splat_with_message() {
        let found = offenses("array = [1, 2, 3]\nputs *array\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 6));
        assert_eq!(
            found[0].message,
            "Ambiguous splat operator. Parenthesize the method arguments if it's surely a splat \
             operator, or add a whitespace to the right of the `*` if it should be a \
             multiplication."
        );
    }

    #[test]
    fn flags_block_ampersand_with_message() {
        let found = offenses("process = proc { do_something }\n2.times &process\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 9));
        assert_eq!(
            found[0].message,
            "Ambiguous block operator. Parenthesize the method arguments if it's surely a block \
             operator, or add a whitespace to the right of the `&` if it should be a binary AND."
        );
    }

    #[test]
    fn flags_keyword_splat_with_message() {
        let found = offenses("do_something **kwargs\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 14));
        assert_eq!(
            found[0].message,
            "Ambiguous keyword splat operator. Parenthesize the method arguments if it's surely a \
             keyword splat operator, or add a whitespace to the right of the `**` if it should \
             be an exponent."
        );
    }

    #[test]
    fn ignores_spaced_and_parenthesized_operators() {
        for source in [
            "do_something + 42\n",
            "do_something(+42)\n",
            "do_something - 42\n",
            "do_something(-42)\n",
            "array = [1, 2, 3]\nputs * array\n",
            "array = [1, 2, 3]\nputs(*array)\n",
            "process = proc { do_something }\n2.times & process\n",
            "process = proc { do_something }\n2.times(&process)\n",
            "do_something ** kwargs\n",
            "do_something(**kwargs)\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_unary_operator_after_safe_navigation() {
        assert!(offenses("do_something&.* -1\n").is_empty());
        assert!(offenses("foo&.bar -1\n").is_empty());
    }

    // The examples below were checked against RuboCop 1.92 with both parser
    // engines.

    #[test]
    fn flags_more_unary_operands() {
        assert_eq!(positions("foo -x\n"), vec![(1, 5)]);
        assert_eq!(positions("foo +x\n"), vec![(1, 5)]);
        assert_eq!(positions("foo -1.abs\n"), vec![(1, 5)]);
        assert_eq!(positions("foo -1, 2\n"), vec![(1, 5)]);
        assert_eq!(positions("a.b -1\n"), vec![(1, 5)]);
        assert_eq!(positions("foo -x.y\n"), vec![(1, 5)]);
        assert_eq!(positions("!foo -1\n"), vec![(1, 6)]);
    }

    #[test]
    fn flags_prefixes_under_any_call() {
        assert_eq!(positions("foo&.bar *a\n"), vec![(1, 10)]);
        assert_eq!(positions("foo **h, a: 1\n"), vec![(1, 5)]);
        assert_eq!(positions("foo &blk\n"), vec![(1, 5)]);
        assert_eq!(
            positions("def n(*a)\n  yield *a\n  super *a\nend\n"),
            vec![(2, 9), (3, 9)]
        );
    }

    #[test]
    fn flags_anonymous_forwarding_in_calls() {
        assert_eq!(positions("def m(*); bar *; end\n"), vec![(1, 15)]);
        assert_eq!(positions("def m(&); bar &; end\n"), vec![(1, 15)]);
        assert_eq!(positions("def m(**); bar **; end\n"), vec![(1, 16)]);
        assert_eq!(positions("def m(&); super &; end\n"), vec![(1, 17)]);
    }

    #[test]
    fn ignores_anonymous_splats_forwarded_to_super_and_unary_keywords() {
        for source in [
            "def m(*); super *; end\n",
            "def m(**); super **; end\n",
            "def m(**); bar a, **; end\n",
            "def m; yield -1; end\n",
            "def m; super -1; end\n",
            "foo.bar &:baz\n",
            "foo = 1\nfoo -1\n",
            "foo = 1\nfoo *2\n",
            "foo a, *b\n",
            "foo(a, -1)\n",
            "x = -1\n",
            "[*a]\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
