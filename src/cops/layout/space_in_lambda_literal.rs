//! `Layout/SpaceInLambdaLiteral` — no blank between `->` and a lambda
//! literal's parameters.
//!
//! A port of RuboCop's cop (97f30c4, MIT) at the fixed `EnforcedStyle:
//! require_no_space`. `on_send` of the `lambda` send of a lambda literal
//! (`node.lambda_literal?`) with arguments (`node.parent.arguments?`): a
//! Prism `LambdaNode` whose parameters are a `BlockParametersNode` holding at
//! least one parameter or block-local variable. `->() {}`, `-> {}` and the
//! numbered-parameter and `it` lambdas have none, and `lambda do |a| end` is
//! a method call. Any gap between the end of `->` and the start of the
//! parameters, their opening parenthesis or, without one, the first
//! parameter (`-> a {}`, as RuboCop's `args` range), is reported at its
//! start (`Do not use spaces between `->` and `(` in lambda literals.`).
//!
//! The correction removes the gap when it is spaces and tabs: `->` is a
//! complete token and the parameters lex the same right after it. A gap
//! holding a line continuation (`-> \` + line break) keeps its finding
//! without an edit, since joining the lines would move every later line
//! (`__LINE__`). Edits are withheld for the listed source
//! observers.
use ruby_prism::Node;

use super::space_around_block_parameters::arguments;
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceInLambdaLiteral;

const MSG: &str = "Do not use spaces between `->` and `(` in lambda literals.";

impl Cop for SpaceInLambdaLiteral {
    fn name(&self) -> &'static str {
        "Layout/SpaceInLambdaLiteral"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_lambda_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(lambda) = node.as_lambda_node() else {
            return;
        };
        let Some(parameters) = lambda
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
        else {
            return;
        };
        if arguments(&parameters).is_empty() {
            return;
        }
        let start = lambda.operator_loc().end_offset();
        let end = parameters.location().start_offset();
        if start >= end {
            return;
        }
        let blank = file.bytes[start..end]
            .iter()
            .all(|b| matches!(b, b' ' | b'\t'));
        let correction = (blank && cx.corrections_requested()).then(|| Correction {
            range: start..end,
            replacement: String::new(),
        });
        let (line, col) = file.line_col(start);
        let (end_line, _) = file.line_col(end);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            end_line,
            col,
        });
    }
}

#[cfg(test)]
mod tests;
