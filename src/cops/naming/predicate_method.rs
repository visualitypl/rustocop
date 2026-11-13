//! `Naming/PredicateMethod` — the pinned conservative return-value matcher.
//! This is deliberately syntax, not type/flow inference. In particular,
//! descendant returns cross nested defs/blocks, and rescue/ensure/kwbegin are
//! not conditional branches in the reference. No corrections are emitted.

use std::collections::HashMap;

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};

pub struct PredicateMethod;

impl Cop for PredicateMethod {
    fn name(&self) -> &'static str {
        "Naming/PredicateMethod"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        let name = def.name();
        let name = name.as_slice();
        if matches!(name, b"initialize" | b"call")
            || ast::is_operator_method(name)
            || def.body().is_none()
        {
            return;
        }
        let root = cx.ancestors().last().unwrap_or(node);
        let values = file.predicate_method_returns(root).methods[&def.name_loc().start_offset()];
        let predicate = name.ends_with(b"?");
        let message = if values == BOOLEAN && !predicate {
            "Predicate method names should end with `?`."
        } else if predicate && values & LITERAL != 0 && values & (BOOLEAN | CALL) == 0 {
            "Non-predicate method names should not end with `?`."
        } else {
            return;
        };
        let loc = def.name_loc();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: message.into(),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1)),
            correction: None,
        });
    }
}

// Union of the reference's processed return values. Identity/deduplication does
// not affect any of its any?/all? queries. Unknown calls (including wayward
// predicates) and super veto both diagnostics in conservative mode; other
// unknown expressions prevent only the all-boolean diagnostic.
const BOOLEAN: u8 = 1;
const LITERAL: u8 = 2;
const CALL: u8 = 4;
const UNKNOWN: u8 = 8;

/// A lazy, source-local index. One complete main-AST walk folds descendant
/// returns bottom-up, without per-method subtree walks or retained AST handles.
#[derive(Default)]
pub(crate) struct Returns {
    methods: HashMap<usize, u8>,
    #[cfg(test)]
    pub(crate) visited: usize,
}

impl Returns {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        struct Frame<'pr> {
            node: Node<'pr>,
            returns: u8,
            body_returns: u8,
        }
        #[derive(Default)]
        struct Builder<'pr> {
            stack: Vec<Frame<'pr>>,
            facts: Returns,
        }
        impl<'pr> NodeVisitor<'pr> for Builder<'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visited += 1;
                }
                let returns = if node.as_return_node().is_some() {
                    extract_return(&node)
                } else {
                    0
                };
                self.stack.push(Frame {
                    node,
                    returns,
                    body_returns: 0,
                });
            }
            fn leave(&mut self) {
                let frame = self.stack.pop().unwrap();
                if let Some(def) = frame.node.as_def_node()
                    && let Some(body) = def.body()
                {
                    self.facts.methods.insert(
                        def.name_loc().start_offset(),
                        frame.body_returns | tail(&body),
                    );
                }
                if let Some(parent) = self.stack.last_mut() {
                    parent.returns |= frame.returns;
                    if let Some(def) = parent.node.as_def_node()
                        && let Some(body) = def.body()
                        && body.location().start_offset() == frame.node.location().start_offset()
                        && body.location().end_offset() == frame.node.location().end_offset()
                    {
                        parent.body_returns = frame.returns;
                    }
                }
            }
        }
        let mut builder = Builder::default();
        walk::walk(&mut builder, root);
        builder.facts
    }
}

/// Prism statement lists of one child are transparent; parentheses are always
/// a reference `begin`. Strip only a *single* begin layer in last_value, not
/// arbitrary expression wrappers (notably not explicit keyword begin).
fn tail(node: &Node<'_>) -> u8 {
    if let Some(statements) = node.as_statements_node() {
        if statements.body().len() == 1 {
            return tail(&statements.body().iter().next().unwrap());
        }
        return statements
            .body()
            .last()
            .map_or(LITERAL, |n| extract_return(&n));
    }
    if let Some(parens) = node.as_parentheses_node() {
        return parens
            .body()
            .and_then(|n| n.as_statements_node().map_or(Some(n), |s| s.body().last()))
            .map_or(LITERAL, |n| extract_return(&n));
    }
    extract_return(node)
}

fn extract_return(node: &Node<'_>) -> u8 {
    if let Some(statements) = node.as_statements_node() {
        return if statements.body().len() == 1 {
            extract_return(&statements.body().iter().next().unwrap())
        } else {
            UNKNOWN
        };
    }
    let Some(ret) = node.as_return_node() else {
        return value(node);
    };
    let Some(args) = ret.arguments() else {
        return LITERAL;
    };
    let Some(first) = args.arguments().iter().next() else {
        return LITERAL;
    };
    // ReturnNode includes WrappedArguments: a first `begin` replaces the entire
    // argument list with its direct children, even when trailing arguments exist
    // (and `(a; b)` is a multiple return here).
    if let Some(parens) = first.as_parentheses_node() {
        let Some(body) = parens.body() else {
            return LITERAL;
        };
        if let Some(statements) = body.as_statements_node() {
            return if statements.body().len() == 1 {
                value(&statements.body().iter().next().unwrap())
            } else {
                LITERAL
            };
        }
        return value(&body);
    }
    if args.arguments().len() != 1 {
        return LITERAL;
    }
    value(&first)
}

fn branch(node: Option<Node<'_>>) -> u8 {
    node.map_or(LITERAL, |n| tail(&n))
}

/// process_return_values: only conditionals and logical clauses are flattened.
/// It does not unwrap arbitrary begins, assignments, blocks or return nodes.
fn value(node: &Node<'_>) -> u8 {
    if let Some(statements) = node.as_statements_node() {
        return if statements.body().len() == 1 {
            value(&statements.body().iter().next().unwrap())
        } else {
            UNKNOWN
        };
    }
    if matches!(node, Node::TrueNode { .. } | Node::FalseNode { .. }) {
        return BOOLEAN;
    }
    if ast::is_literal(node) {
        return LITERAL;
    }
    match node {
        Node::IfNode { .. } => {
            let n = node.as_if_node().unwrap();
            let mut values = branch(n.statements().map(|s| s.as_node()));
            let mut subsequent = n.subsequent();
            if subsequent.is_none() {
                return values | LITERAL;
            }
            // The reference flattens `IfNode#branches` across elsif, then
            // adds nil only if the *outer* if has no else_branch. An elsif
            // counts as that branch even when the chain has no final else.
            // Do not recursively synthesize a missing else for each elsif.
            while let Some(next) = subsequent {
                if let Some(elsif) = next.as_if_node() {
                    values |= branch(elsif.statements().map(|s| s.as_node()));
                    subsequent = elsif.subsequent();
                } else {
                    values |= branch(Some(next));
                    break;
                }
            }
            values
        }
        Node::UnlessNode { .. } => {
            let n = node.as_unless_node().unwrap();
            branch(n.statements().map(|s| s.as_node()))
                | branch(n.else_clause().map(|e| e.as_node()))
        }
        Node::ElseNode { .. } => branch(
            node.as_else_node()
                .unwrap()
                .statements()
                .map(|s| s.as_node()),
        ),
        Node::CaseNode { .. } => {
            let n = node.as_case_node().unwrap();
            let mut values = n
                .else_clause()
                .map_or(LITERAL, |e| branch(e.statements().map(|s| s.as_node())));
            for when in n.conditions().iter() {
                values |= branch(
                    when.as_when_node()
                        .unwrap()
                        .statements()
                        .map(|s| s.as_node()),
                );
            }
            values
        }
        Node::CaseMatchNode { .. } => {
            let n = node.as_case_match_node().unwrap();
            let mut values = n
                .else_clause()
                .map_or(LITERAL, |e| branch(e.statements().map(|s| s.as_node())));
            for clause in n.conditions().iter() {
                values |= branch(
                    clause
                        .as_in_node()
                        .unwrap()
                        .statements()
                        .map(|s| s.as_node()),
                );
            }
            values
        }
        Node::WhileNode { .. } => {
            let n = node.as_while_node().unwrap();
            if n.is_begin_modifier() {
                UNKNOWN
            } else {
                branch(n.statements().map(|s| s.as_node()))
            }
        }
        Node::UntilNode { .. } => {
            let n = node.as_until_node().unwrap();
            if n.is_begin_modifier() {
                UNKNOWN
            } else {
                branch(n.statements().map(|s| s.as_node()))
            }
        }
        Node::AndNode { .. } => {
            let n = node.as_and_node().unwrap();
            value(&n.left()) | value(&n.right())
        }
        Node::OrNode { .. } => {
            let n = node.as_or_node().unwrap();
            value(&n.left()) | value(&n.right())
        }
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            if call.block().is_some_and(|b| b.as_block_node().is_some()) {
                return UNKNOWN;
            }
            let name = call.name();
            let name = name.as_slice();
            if !matches!(name, b"infinite?" | b"nonzero?")
                && (name.ends_with(b"?")
                    || matches!(
                        name,
                        b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<" | b"!"
                    ))
            {
                BOOLEAN
            } else {
                CALL
            }
        }
        Node::SuperNode { .. } => {
            if node
                .as_super_node()
                .unwrap()
                .block()
                .is_some_and(|b| b.as_block_node().is_some())
            {
                UNKNOWN
            } else {
                CALL
            }
        }
        Node::ForwardingSuperNode { .. } => {
            if node.as_forwarding_super_node().unwrap().block().is_some() {
                UNKNOWN
            } else {
                CALL
            }
        }
        _ => UNKNOWN,
    }
}
#[cfg(test)]
mod tests {
    //! RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/PredicateMethod")
            .collect()
    }
    #[test]
    fn explicit_returns_calls_and_conservative_unknowns() {
        for source in [
            "def foo; log; return true; end",
            "def foo; x == y; end",
            "def foo; object&.ready?; end",
            "def foo; !x; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "def foo?; return 5 if x; true; end",
            "def foo?; return true if x; 5; end",
            "def foo?; return unknown if x; 5; end",
            "def foo?; return super if x; 5; end",
            "def foo; x.nonzero?; end",
            "def foo; x.infinite?; end",
            "def call; true; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn exact_pinned_scope_tail_rescue_ensure_and_wrapped_arguments() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns08_syntax.json")).unwrap();
        let mut failures = Vec::new();
        for row in &rows {
            let found = offenses(row["source"].as_str().unwrap());
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            if serde_json::to_value(positions).unwrap() != row["positions"] {
                failures.push(format!("{row}: {found:?}"));
            }
            assert_eq!(
                serde_json::to_value(found.iter().map(|o| &o.message).collect::<Vec<_>>()).unwrap(),
                row["messages"],
                "{row}"
            );
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
    #[test]
    fn all_applicable_specs_and_source_examples() {
        let pinned: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns08_pinned.json")).unwrap();
        assert_eq!(pinned["calls"].as_array().unwrap().len(), 626);
        assert_eq!(pinned["excluded"].as_array().unwrap().len(), 636);
        for row in pinned["calls"].as_array().unwrap().iter().chain(
            serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
                "../../../tests/fixtures/ns08_source.json"
            ))
            .unwrap()
            .iter(),
        ) {
            let found = offenses(row["source"].as_str().unwrap());
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(
                serde_json::to_value(positions).unwrap(),
                row["positions"],
                "{row}"
            );
            assert_eq!(
                serde_json::to_value(found.iter().map(|o| &o.message).collect::<Vec<_>>()).unwrap(),
                row["messages"],
                "{row}"
            );
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }
    #[test]
    fn elsif_branches_do_not_invent_an_implicit_nil() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/ns11_predicate_branches.json"
        ))
        .unwrap();
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let found = offenses(source);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(
                serde_json::to_value(positions).unwrap(),
                row["positions"],
                "{source}"
            );
            for offense in found {
                assert_eq!(
                    offense.message,
                    if source.starts_with("def ready;") {
                        "Predicate method names should end with `?`."
                    } else {
                        "Non-predicate method names should not end with `?`."
                    }
                );
                assert!(offense.correction.is_none());
            }
        }
    }
    #[test]
    fn boolean_tail_and_literal_tail_report_method_names() {
        for (source, col, message) in [
            (
                "def foo; true; end",
                5,
                "Predicate method names should end with `?`.",
            ),
            (
                "def self.foo? = 5",
                10,
                "Non-predicate method names should not end with `?`.",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!(found[0].message, message);
            assert!(found[0].correction.is_none());
        }
    }
}
