//! `Metrics/CyclomaticComplexity` — methods with more than 7 independent
//! paths.
//!
//! Upstream counts one plus every parser `if` (also `unless`, `elsif`,
//! modifiers and ternaries), `while`/`until` (not the `begin ... end while`
//! forms), `for`, `rescue` construct, `when`, `in`, `&&`/`and`, `||`/`or`,
//! `||=` and `&&=` in a method body (nested definitions included), plus every
//! `block` or `block_pass` on a known iterating method and every `&.` call
//! except a repeated one on the same untouched local variable. The pinned
//! defaults are `Max: 7` and no allowed method names or patterns. The whole
//! method is reported; there is no correction.

use ruby_prism::Node;

use super::method_complexity::{Compound, Event, Method, Order, RepeatedCsend, events};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// Pinned `Max`.
const MAX: usize = 7;

/// The `Metrics/CyclomaticComplexity` cop.
pub struct CyclomaticComplexity;

impl Cop for CyclomaticComplexity {
    fn name(&self) -> &'static str {
        "Metrics/CyclomaticComplexity"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(method) = Method::of(node) else {
            return;
        };
        let complexity = complexity(&method.body, cyclomatic_score);
        if complexity > MAX {
            let message = format!(
                "Cyclomatic complexity for `{}` is too high. [{complexity}/{MAX}]",
                method.name
            );
            method.report(self, message, file, out);
        }
    }
}

/// `MethodComplexity#complexity`: one plus the score of every counted node
/// of `body` in pre-order (`each_node`), resetting the repeated `&.`
/// discount at every `lvasgn`.
pub(super) fn complexity(
    body: &Node<'_>,
    score: impl Fn(&Event<'_, '_>, &mut RepeatedCsend) -> usize,
) -> usize {
    let mut total = 1;
    let mut csend = RepeatedCsend::default();
    events(body, Order::Pre, |event| {
        if let Event::Lvasgn(name) = event {
            csend.reset(name);
        } else {
            total += score(&event, &mut csend);
        }
    });
    total
}

/// `CyclomaticComplexity#complexity_score_for`.
pub(super) fn cyclomatic_score(event: &Event<'_, '_>, csend: &mut RepeatedCsend) -> usize {
    match event {
        Event::If { .. }
        | Event::Loop
        | Event::For
        | Event::IteratingBlock
        | Event::Rescue
        | Event::When
        | Event::In
        | Event::And
        | Event::Or
        | Event::Compound {
            kind: Compound::Or | Compound::And,
            ..
        } => 1,
        Event::Send(send) if send.csend => usize::from(!csend.discount(send)),
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
