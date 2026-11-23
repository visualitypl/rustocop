//! `Lint/IncompatibleIoSelectWithFiberScheduler` — scheduler-compatible IO waits.

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct IncompatibleIoSelectWithFiberScheduler;

impl Cop for IncompatibleIoSelectWithFiberScheduler {
    fn name(&self) -> &'static str {
        "Lint/IncompatibleIoSelectWithFiberScheduler"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"select"
            || call.is_safe_navigation()
            || !call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["IO"]))
        {
            return;
        }
        let args = ast::call_arguments(&call);
        if args.get(2).is_some_and(|n| !empty_reference_children(n)) {
            return;
        }
        let read = args.first();
        let write = args.get(1);
        let (io, method) = if let Some(io) = single_io(read).filter(|_| empty_io(write)) {
            (io, "wait_readable")
        } else if let Some(io) = single_io(write).filter(|_| empty_io(read)) {
            (io, "wait_writable")
        } else {
            return;
        };
        let timeout = args
            .get(3)
            .map(|n| format!("({})", source(n.location().as_slice(), file)))
            .unwrap_or_default();
        let preferred = format!(
            "{}.{method}{timeout}",
            source(io.location().as_slice(), file)
        );
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        // Not when an assignment consumes the (different) return value.
        let correction = (cx.corrections_requested()
            && !cx.parent().is_some_and(crate::cops::ast::is_assignment))
        .then(|| {
            let text = |n: &Node<'_>| String::from_utf8_lossy(n.location().as_slice()).into_owned();
            let timeout = args
                .get(3)
                .map(|n| format!("({})", text(n)))
                .unwrap_or_default();
            crate::offense::Correction {
                range: start..node.location().end_offset(),
                replacement: format!("{}.{method}{timeout}", text(&io)),
            }
        });
        let current = source(&file.bytes[start..end], file);
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!("Use `{preferred}` instead of `{current}`."),
            correction,
        });
    }
}

fn single_io<'pr>(node: Option<&Node<'pr>>) -> Option<Node<'pr>> {
    let array = node?.as_array_node()?;
    if array.elements().len() != 1 {
        return None;
    }
    array
        .elements()
        .iter()
        .next()
        // Prism uses SplatNode for both ordinary splats and anonymous rest
        // forwarding; RuboCop rejects only the former (`splat`, not
        // `forwarded_restarg`).
        .filter(|n| n.as_splat_node().is_none_or(|s| s.expression().is_none()))
}

fn empty_io(node: Option<&Node<'_>>) -> bool {
    node.is_none_or(|n| {
        n.as_nil_node().is_some() || n.as_array_node().is_some_and(|a| a.elements().is_empty())
    })
}

// The pinned code asks `excepts.children.empty?`, not whether its value is nil
// or an empty IO array. Preserve that shallow AST predicate, not type flow.
fn empty_reference_children(node: &Node<'_>) -> bool {
    match node {
        Node::NilNode { .. }
        | Node::TrueNode { .. }
        | Node::FalseNode { .. }
        | Node::SelfNode { .. }
        | Node::RedoNode { .. }
        | Node::RetryNode { .. }
        | Node::ForwardingArgumentsNode { .. } => true,
        Node::ForwardingSuperNode { .. } => {
            node.as_forwarding_super_node().unwrap().block().is_none()
        }
        Node::SplatNode { .. } => node.as_splat_node().unwrap().expression().is_none(),
        Node::StringNode { .. } => {
            let n = node.as_string_node().unwrap();
            n.unescaped().is_empty()
                && (n.content_loc().as_slice().contains(&b'\n')
                    || n.opening_loc().is_some_and(|o| {
                        o.as_slice().starts_with(b"%") || o.as_slice().starts_with(b"<<")
                    }))
        }
        Node::SymbolNode { .. } => {
            let n = node.as_symbol_node().unwrap();
            n.closing_loc().is_some()
                && n.value_loc().is_none_or(|v| {
                    v.as_slice().is_empty()
                        || (v.as_slice().contains(&b'\n') && n.unescaped().is_empty())
                })
        }
        Node::XStringNode { .. } => {
            let n = node.as_x_string_node().unwrap();
            n.content_loc().as_slice().is_empty()
                || (n.unescaped().is_empty()
                    && (n.content_loc().as_slice().contains(&b'\n')
                        || n.opening_loc().as_slice().starts_with(b"<<")))
        }
        Node::InterpolatedStringNode { .. } => {
            let n = node.as_interpolated_string_node().unwrap();
            n.opening_loc()
                .is_some_and(|o| o.as_slice().starts_with(b"<<"))
                && n.parts()
                    .iter()
                    .all(|p| p.as_string_node().is_some_and(|s| s.unescaped().is_empty()))
        }
        Node::InterpolatedXStringNode { .. } => {
            let n = node.as_interpolated_x_string_node().unwrap();
            n.opening_loc().as_slice().starts_with(b"<<")
                && n.parts()
                    .iter()
                    .all(|p| p.as_string_node().is_some_and(|s| s.unescaped().is_empty()))
        }
        Node::ArrayNode { .. } => node.as_array_node().unwrap().elements().is_empty(),
        Node::HashNode { .. } => node.as_hash_node().unwrap().elements().is_empty(),
        Node::KeywordHashNode { .. } => node.as_keyword_hash_node().unwrap().elements().is_empty(),
        Node::ParenthesesNode { .. } => node.as_parentheses_node().unwrap().body().is_none(),
        Node::BeginNode { .. } => {
            let n = node.as_begin_node().unwrap();
            n.statements().is_none()
                && n.rescue_clause().is_none()
                && n.else_clause().is_none()
                && n.ensure_clause().is_none()
        }
        Node::ReturnNode { .. } => node.as_return_node().unwrap().arguments().is_none(),
        Node::BreakNode { .. } => node.as_break_node().unwrap().arguments().is_none(),
        Node::NextNode { .. } => node.as_next_node().unwrap().arguments().is_none(),
        Node::YieldNode { .. } => node.as_yield_node().unwrap().arguments().is_none(),
        Node::SuperNode { .. } => {
            let n = node.as_super_node().unwrap();
            n.arguments().is_none() && n.block().is_none()
        }
        _ => false,
    }
}

// Quote the reference source buffer, retaining multiline text but normalizing
// CRLF and its declared encoding locally; original offsets/bytes stay untouched.
fn source(bytes: &[u8], file: &SourceFile) -> String {
    String::from_utf8_lossy(&file.match_pattern_source(bytes)).replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/incompatible_io_select_with_fiber_scheduler_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/IncompatibleIoSelectWithFiberScheduler")
            .collect()
    }

    #[test]
    fn pinned_read_write_timeout_and_assignment_examples() {
        for (call, preferred) in [
            ("IO.select([io], [], [])", "io.wait_readable"),
            ("IO.select([io])", "io.wait_readable"),
            (
                "IO.select([io], [], [], timeout)",
                "io.wait_readable(timeout)",
            ),
            ("::IO.select([io], [], [])", "io.wait_readable"),
            (
                "::IO.select([io], [], [], timeout)",
                "io.wait_readable(timeout)",
            ),
            (
                "IO.select([io], nil, nil, timeout)",
                "io.wait_readable(timeout)",
            ),
            (
                "IO.select(nil, [io], nil, timeout)",
                "io.wait_writable(timeout)",
            ),
            ("IO.select([], [io], [])", "io.wait_writable"),
            (
                "IO.select([], [io], [], timeout)",
                "io.wait_writable(timeout)",
            ),
            (
                "IO.select([self], nil, nil, timeout)",
                "self.wait_readable(timeout)",
            ),
            (
                "IO.select(nil, [self], nil, timeout)",
                "self.wait_writable(timeout)",
            ),
            ("IO.select([rp], [])", "rp.wait_readable"),
            ("IO.select([], [wp])", "wp.wait_writable"),
        ] {
            let found = offenses(&format!("rs, ws =\n  {call}\n"));
            assert_eq!(found.len(), 1, "{call}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!("Use `{preferred}` instead of `{call}`.")
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn anonymous_forwarded_splats_are_single_io_values() {
        let found = offenses("def wait_for_io(*)\n  IO.select([*])\n  IO.select(nil, [*])\nend\n");
        assert_eq!(found.len(), 2);
        for (offense, line, call, preferred) in [
            (&found[0], 2, "IO.select([*])", "*.wait_readable"),
            (&found[1], 3, "IO.select(nil, [*])", "*.wait_writable"),
        ] {
            assert_eq!(
                (offense.line, offense.col, offense.end_line),
                (line, 3, line)
            );
            assert_eq!(
                offense.message,
                format!("Use `{preferred}` instead of `{call}`.")
            );
            assert!(offense.correction.is_none());
        }
    }

    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "IO.select([foo, bar], [], [])",
            "IO.select([foo, bar])",
            "IO.select([], [foo, bar], [])",
            "IO.select([rp], [wp], [])",
            "IO.select([rp], [], [excepts])",
            "collection.select { |item| item.do_something? }",
            "IO.select([*reads])",
            "IO.select(nil, [*writes])",
            "IO&.select([io])",
            "Other::IO.select([io])",
            "IO.select",
            "IO.select([])",
            "IO.select(([io]))",
            "(IO).select([io])",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
