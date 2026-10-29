//! `Style/RequireOrder` — detect unsorted feature loads, never reorder side effects.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct RequireOrder;
impl Cop for RequireOrder {
    fn name(&self) -> &'static str {
        "Style/RequireOrder"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // A running maximum replaces RuboCop's repeated backwards sibling
        // searches. Each sibling and intervening source byte is examined O(1)
        // times, even for an entire descending sequence of loads.
        if let Some(statements) = node.as_statements_node() {
            check_sequence(statements.body().iter(), self, file, out);
        } else if let Some(call) = node.as_call_node() {
            // Prism's ArgumentsNode is transparent in RuboCop: receiver and
            // positional arguments are siblings, including operator calls.
            check_sequence(
                call.receiver().into_iter().chain(
                    call.arguments()
                        .into_iter()
                        .flat_map(|a| a.arguments().iter()),
                ),
                self,
                file,
                out,
            );
        } else if let Some(arguments) = node.as_arguments_node() {
            if cx.parent().is_none_or(|n| n.as_call_node().is_none()) {
                check_sequence(arguments.arguments().iter(), self, file, out);
            }
        } else if let Some(array) = node.as_array_node() {
            check_sequence(array.elements().iter(), self, file, out);
        } else if let Some(pair) = node.as_assoc_node() {
            check_sequence([pair.key(), pair.value()].into_iter(), self, file, out);
        } else if let Some(and) = node.as_and_node() {
            check_sequence([and.left(), and.right()].into_iter(), self, file, out);
        } else if let Some(or) = node.as_or_node() {
            check_sequence([or.left(), or.right()].into_iter(), self, file, out);
        } else if let Some(range) = node.as_range_node() {
            check_sequence(
                range.left().into_iter().chain(range.right()),
                self,
                file,
                out,
            );
        } else if let Some(when) = node.as_when_node() {
            check_sequence(
                when.conditions()
                    .iter()
                    .chain(when.statements().map(|s| sibling_body(s.as_node()))),
                self,
                file,
                out,
            );
        } else if let Some(while_node) = node.as_while_node() {
            check_sequence(
                std::iter::once(while_node.predicate())
                    .chain(while_node.statements().map(|s| sibling_body(s.as_node()))),
                self,
                file,
                out,
            );
        } else if let Some(until) = node.as_until_node() {
            check_sequence(
                std::iter::once(until.predicate())
                    .chain(until.statements().map(|s| sibling_body(s.as_node()))),
                self,
                file,
                out,
            );
        } else if let Some(for_node) = node.as_for_node() {
            check_sequence(
                std::iter::once(for_node.collection())
                    .chain(for_node.statements().map(|s| sibling_body(s.as_node()))),
                self,
                file,
                out,
            );
        } else if let Some(class) = node.as_class_node() {
            check_sequence(
                class
                    .superclass()
                    .into_iter()
                    .chain(class.body().map(sibling_body)),
                self,
                file,
                out,
            );
        } else if let Some(class) = node.as_singleton_class_node() {
            check_sequence(
                std::iter::once(class.expression()).chain(class.body().map(sibling_body)),
                self,
                file,
                out,
            );
        } else if let Some(write) = node.as_constant_path_write_node() {
            check_sequence(
                write
                    .target()
                    .parent()
                    .into_iter()
                    .chain(std::iter::once(write.value())),
                self,
                file,
                out,
            );
        } else if let Some(rescue) = node.as_rescue_node() {
            // Upstream wraps the exception list in an array, so the rescue
            // body is not a sibling of its individual exceptions.
            check_sequence(rescue.exceptions().iter(), self, file, out);
        }
    }
}
// Prism wraps even singleton bodies in StatementsNode; RuboCop only wraps
// multi-statement bodies. Keep the latter as boundaries, not flattened lists.
fn sibling_body<'pr>(node: Node<'pr>) -> Node<'pr> {
    if let Some(statements) = node.as_statements_node()
        && statements.body().len() == 1
    {
        return statements.body().iter().next().unwrap();
    }
    node
}
fn modifier_parts<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>)> {
    let (keyword, body, predicate) = if let Some(conditional) = node.as_if_node() {
        (
            conditional.if_keyword_loc()?,
            conditional.statements()?,
            conditional.predicate(),
        )
    } else {
        let conditional = node.as_unless_node()?;
        (
            conditional.keyword_loc(),
            conditional.statements()?,
            conditional.predicate(),
        )
    };
    if keyword.start_offset() < body.location().start_offset() {
        return None;
    }
    let mut children = body.body().iter();
    let child = children.next()?;
    children.next().is_none().then_some((child, predicate))
}
fn required<'pr>(node: &Node<'pr>) -> Option<CallNode<'pr>> {
    let call = node.as_call_node()?;
    (!call.is_safe_navigation()
        && matches!(call.name().as_slice(), b"require" | b"require_relative")
        && call.block().is_none_or(|b| b.as_block_node().is_none()))
    .then_some(call)
}
fn check_sequence<'pr>(
    nodes: impl Iterator<Item = Node<'pr>>,
    cop: &RequireOrder,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) -> usize {
    let mut inspected = 0;
    let mut maximum: Option<(&[u8], ruby_prism::StringNode<'pr>)> = None;
    let mut previous_start = 0;
    let mut previous_end = 0;
    for node in nodes {
        let parts = modifier_parts(&node);
        let (body, predicate) = match &parts {
            Some((body, predicate)) => (required(body), required(predicate)),
            None => (required(&node), None),
        };
        // Both a modifier's body and its direct predicate sends search the
        // enclosing sibling list upstream. Neither becomes the other's sibling.
        for call in body.iter().chain(predicate.iter()) {
            let Some(path) = first_path(call) else {
                continue;
            };
            let start = call.location().start_offset();
            let end = ast::source_end(&call.as_node());
            if let Some((old_name, old_value)) = &maximum {
                // Loop modifiers have AST-order siblings in reverse source
                // order (predicate, body). Join their ranges in either order.
                let joined_start = previous_start.min(start);
                let joined_end = previous_end.max(end);
                inspected += joined_end - joined_start;
                if *old_name == call.name().as_slice()
                    && path.unescaped() < old_value.unescaped()
                    && !section_break(file, joined_start, joined_end)
                {
                    let (line, col) = file.line_col(start);
                    out.push(Offense {
                        cop: cop.name(),
                        severity: cop.severity(),
                        message: format!(
                            "Sort `{}` in alphabetical order.",
                            String::from_utf8_lossy(call.name().as_slice())
                        ),
                        line,
                        col,
                        end_line: file.line_of(end.saturating_sub(1).max(start)),
                        correction: None,
                    });
                }
            }
        }
        // The preceding modifier must contain exactly one bare, one-argument
        // require send, not a nested conditional or a receiver/arity lookalike.
        let Some(call) = body.filter(|c| {
            c.receiver().is_none() && (parts.is_none() || ast::call_arguments(c).len() == 1)
        }) else {
            maximum = None;
            continue;
        };
        let Some(path) = first_path(&call) else {
            maximum = None;
            continue;
        };
        let name = call.name().as_slice();
        if let Some((old_name, _)) = &maximum {
            let end = ast::source_end(&call.as_node());
            let start = previous_start.min(call.location().start_offset());
            let end = previous_end.max(end);
            inspected += end - start;
            if *old_name != name || section_break(file, start, end) {
                maximum = None;
            }
        }
        if maximum
            .as_ref()
            .is_none_or(|(_, old)| old.unescaped() < path.unescaped())
        {
            maximum = Some((name, path));
        }
        previous_start = call.location().start_offset();
        previous_end = ast::source_end(&call.as_node());
    }
    inspected
}
fn first_path<'pr>(call: &CallNode<'pr>) -> Option<ruby_prism::StringNode<'pr>> {
    call.arguments()?
        .arguments()
        .iter()
        .next()?
        .as_string_node()
}
fn section_break(file: &SourceFile, start: usize, end: usize) -> bool {
    file.bytes[start..end]
        .split(|&b| b == b'\n')
        .skip(1)
        .any(|line| line.is_empty() || line == b"\r")
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/require_order_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    fn offenses(source: &str) -> Vec<Offense> {
        correction_offenses(&RequireOrder, source)
    }
    #[test]
    fn pinned_unsorted_sections_positions_and_messages() {
        for method in ["require", "require_relative"] {
            let source = format!("{method} 'd'\n{method} 'a'\n{method} 'b'\n{method} 'c'\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 3, "{source}");
            for (o, line) in found.iter().zip(2..=4) {
                assert_eq!((o.line, o.col, o.end_line), (line, 1, line));
                assert_eq!(o.message, format!("Sort `{method}` in alphabetical order."));
                assert!(o.correction.is_none());
            }
        }
        for (source, line, col) in [
            ("require 'b' # comment\nrequire 'a'", 2, 1),
            ("require 'b'\n# comment\nrequire 'a'", 3, 1),
            ("require 'b'; require 'a'", 1, 14),
            ("foo -> { require 'b'\n         require 'a' }", 2, 10),
            (
                "begin\n effect\nrescue\n require 'b'\n require 'a'\nend",
                5,
                2,
            ),
            (
                "require 'd'\nif foo\n require 'b'\n require 'a'\nend\nrequire 'c'",
                4,
                2,
            ),
            ("require 'b'\nrequire 'c'\nrequire 'a'", 3, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
        }
        for keyword in ["if", "unless"] {
            let source = format!("require 'c'\nrequire 'a' {keyword} foo\nrequire 'b'");
            let found = offenses(&source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                [(2, 1), (3, 1)],
                "{source}"
            );
        }
    }
    #[test]
    fn modifier_predicate_loads_and_strict_predecessor_shapes() {
        for keyword in ["if", "unless"] {
            let source = format!("require 'b'\nfoo {keyword} require('a')");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col),
                (2, 8 + u32::from(keyword == "unless") * 4)
            );
            for source in [
                format!("require 'b', extra {keyword} cond\nrequire 'a'"),
                format!("require 'b' {keyword} inner {keyword} outer\nrequire 'a'"),
                format!("require 'b'\nrequire 'a' {keyword} inner {keyword} outer"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn when_conditions_and_loop_siblings_match_reference_positions() {
        for (source, col) in [
            ("case x; when require('b'), require('a'); end", 28),
            ("case x; when require('b'); require('a'); end", 28),
            ("while require('b'); require('a'); end", 21),
            ("until require('b'); require('a'); end", 21),
            ("for x in require('b'); require('a'); end", 24),
            ("require('a') while require('b')", 1),
            ("require('a') until require('b')", 1),
            ("class C < require('b'); require('a'); end", 25),
            ("class << require('b'); require('a'); end", 24),
            ("require('b')::C = require('a')", 19),
            (
                "begin; effect; rescue require('c'), require('b'); require('a'); end",
                37,
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(found[0].message, "Sort `require` in alphabetical order.");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn structural_bodies_do_not_flatten_multiple_statements_or_conditionals() {
        for source in [
            "while require('b'); require('a'); effect; end",
            "until require('b'); require('a'); effect; end",
            "for x in require('b'); require('a'); effect; end",
            "class C < require('b'); require('a'); effect; end",
            "class << require('b'); require('a'); effect; end",
            "begin; require('a'); end while require('b')",
            "if require('b'); require('a'); end",
            "unless require('b'); require('a'); end",
            "case x; when require('b'); require('a'); effect; end",
            "case x; when require('b'); end; require('a')",
            "begin; effect; rescue require('b'); require('a'); end",
            "while require('b');\n\nrequire('a'); end",
            "require('a') while require('a')",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let found = offenses("while require('z'); require('b'); require('a'); end");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 35));
    }
    #[test]
    fn long_descending_sections_use_bounded_sibling_and_byte_work() {
        let source: String = (0..4000)
            .rev()
            .map(|i| format!("require '{i:04}'\n"))
            .collect();
        let file = SourceFile::new("large.rb".into(), source.into_bytes());
        let parsed = ruby_prism::parse(&file.bytes);
        let body = parsed.node().as_program_node().unwrap().statements();
        let mut visited = 0;
        let mut found = Vec::new();
        let inspected = check_sequence(
            body.body().iter().inspect(|_| visited += 1),
            &RequireOrder,
            &file,
            &mut found,
        );
        assert_eq!(visited, 4000);
        assert_eq!(found.len(), 3999);
        assert!(inspected <= 4 * file.bytes.len(), "{inspected}");
    }
    #[test]
    fn long_when_condition_lists_use_bounded_sibling_and_byte_work() {
        let paths = (0..4000)
            .rev()
            .map(|i| format!("require('{i:04}')"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("case x; when {paths}; end");
        let file = SourceFile::new("large.rb".into(), source.into_bytes());
        let parsed = ruby_prism::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        let case = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap()
            .as_case_node()
            .unwrap();
        let when = case
            .conditions()
            .iter()
            .next()
            .unwrap()
            .as_when_node()
            .unwrap();
        let mut visited = 0;
        let mut found = Vec::new();
        let inspected = check_sequence(
            when.conditions().iter().inspect(|_| visited += 1),
            &RequireOrder,
            &file,
            &mut found,
        );
        assert_eq!(visited, 4000);
        assert_eq!(found.len(), 3999);
        assert!(inspected <= 4 * file.bytes.len(), "{inspected}");
    }
    #[test]
    fn decoded_paths_receivers_arguments_blocks_and_expression_siblings() {
        for source in [
            "[require('b'), require('a')]",
            "foo(require('b'), require('a'))",
            "require('b') && require('a')",
            "require('b') || require('a')",
            "require('b') + require('a')",
            "require('b').foo(require('a'))",
            "{require('b') => require('a')}",
            "require('b')..require('a')",
            "require 'b'\nKernel.require 'a'",
            "require 'b'\nself.require 'a'",
            "require 'b'\nrequire 'a', extra",
            "require 'b'\nrequire 'a', &block",
            "require \"b\\x2fc\"\nrequire 'b/a'",
            "require 'b'\n \nrequire 'a'",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "require 'b'\nrequire path\nrequire 'a'",
            "require 'b'\nrequire :path\nrequire 'a'",
            "require 'b'\nrequire \"#{path}\"\nrequire 'a'",
            "Kernel.require 'b'\nrequire 'a'",
            "require 'b'\nloader&.require 'a'",
            "require 'b'\nrequire('a') { effect }",
            "require('b') { effect }\nrequire 'a'",
            "require 'b'\r\n\r\nrequire 'a'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_sorted_sections_and_boundaries() {
        for source in [
            "require 'a'\nrequire 'b'",
            "require 'a'\nrequire \"b\"",
            "require 'a'",
            "require 'b'\nrequire 'd'\n\nrequire 'a'\nrequire 'c'",
            "require 'b'\nrequire_relative 'a'",
            "require 'c'\nrequire_relative 'b'\nrequire 'a'",
            "do_something(require)",
            "require 'e'\nBundler.require(:default)\nrequire 'c'",
            "require 'c'\nBundler.require\nrequire 'a'",
            "require 'b'\nbegin; end\nrequire 'a'",
            "require 'c'\nif foo\n require 'a'\nend\nrequire 'b'",
            "require 'c'\nunless foo\n require 'a'\nend\nrequire 'b'",
            "require 'd'\nif foo\n require 'a'\n require 'b'\nend\nrequire 'c'",
            "require 'c'\nif foo\n if bar\n require 'a'\n end\nend\nrequire 'b'",
            "begin\neffect\nrescue\n require 'a'\nend",
            "require 'a'\nrequire 'a'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
