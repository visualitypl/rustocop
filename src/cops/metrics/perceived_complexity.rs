//! `Metrics/PerceivedComplexity` — methods whose perceived complexity
//! exceeds 8.
//!
//! Upstream inherits `Metrics/CyclomaticComplexity`'s count, except that:
//! an `if` with an `else` clause (or a first `elsif`) counts 2 and an
//! `elsif` 1; `when` and `in` do not count by themselves; a `case` with a
//! subject counts `0.8 + 0.2` per `when` and non-empty `else`, rounded, and
//! one without counts each `when` and a non-empty `else`; a `case ... in`
//! counts 0.2 per `in` whose unguarded pattern is a literal or a constant,
//! 1 per other `in` and 0.2 for an `else`, rounded. Rounding is Ruby's
//! (half away from zero, after `Array#sum`'s compensated float sum). The
//! pinned defaults are `Max: 8` and no allowed method names or patterns.
//! The whole method is reported; there is no correction.

use ruby_prism::{CaseMatchNode, CaseNode, Node};

use super::{
    cyclomatic_complexity::{complexity, cyclomatic_score},
    method_complexity::{Event, Method, RepeatedCsend, in_pattern},
};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The default `Max`.
const MAX: usize = 8;

/// The `Metrics/PerceivedComplexity` cop.
pub struct PerceivedComplexity {
    /// The value of `Max`.
    max: usize,
}

impl Default for PerceivedComplexity {
    fn default() -> Self {
        PerceivedComplexity { max: MAX }
    }
}

impl Cop for PerceivedComplexity {
    fn name(&self) -> &'static str {
        "Metrics/PerceivedComplexity"
    }

    fn max_setting(&self) -> Option<&'static str> {
        Some("max")
    }

    fn with_max(&self, max: usize) -> Option<Box<dyn Cop>> {
        Some(Box::new(PerceivedComplexity { max }))
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let max = self.max;
        let Some(method) = Method::of(node) else {
            return;
        };
        let complexity = complexity(&method.body, perceived_score);
        if complexity > max {
            let message = format!(
                "Perceived complexity for `{}` is too high. [{complexity}/{max}]",
                method.name
            );
            method.report(self, message, file, out);
        }
    }
}

/// `PerceivedComplexity#complexity_score_for`.
pub(super) fn perceived_score(event: &Event<'_, '_>, csend: &mut RepeatedCsend) -> usize {
    match event {
        Event::Case(case) => case_score(case),
        Event::CaseMatch(case) => case_match_score(case),
        Event::If {
            else_clause, elsif, ..
        } => {
            if *else_clause && !*elsif {
                2
            } else {
                1
            }
        }
        Event::When | Event::In => 0,
        _ => cyclomatic_score(event, csend),
    }
}

/// A `case`/`when`: each branch counts without a subject; with one, the
/// `case` counts 0.8 and each branch 0.2. The `else` branch counts only
/// with a body (the parser's `else_branch`).
fn case_score(case: &CaseNode<'_>) -> usize {
    let else_body = case.else_clause().is_some_and(|e| e.statements().is_some());
    let branches = case.conditions().len() + usize::from(else_body);
    if case.predicate().is_none() {
        branches
    } else {
        ruby_round((branches as f64 * 0.2) + 0.8)
    }
}

/// A `case`/`in`: a simple branch counts 0.2, any other 1, and an `else`
/// (even an empty one, the parser's `empty_else`) 0.2.
fn case_match_score(case: &CaseMatchNode<'_>) -> usize {
    let branches = case
        .conditions()
        .iter()
        .map(|branch| branch.as_in_node().is_some_and(|b| simple_in_pattern(&b)));
    let mut score = ruby_array_sum(branches);
    if case.else_clause().is_some() {
        score = Score::Float(score.to_f64() + 0.2);
    }
    match score {
        Score::Int(n) => n,
        Score::Float(f) => ruby_round(f),
    }
}

/// `simple_in_pattern?`: no guard, and a literal (`int`, `str`, `sym`,
/// ranges, regexps, `%w` arrays, `nil`, `true`, ...) or a constant pattern.
fn simple_in_pattern(branch: &ruby_prism::InNode<'_>) -> bool {
    let (pattern, guard) = in_pattern(branch);
    guard.is_none()
        && matches!(
            pattern,
            Node::IntegerNode { .. }
                | Node::FloatNode { .. }
                | Node::RationalNode { .. }
                | Node::ImaginaryNode { .. }
                | Node::StringNode { .. }
                | Node::InterpolatedStringNode { .. }
                | Node::XStringNode { .. }
                | Node::InterpolatedXStringNode { .. }
                | Node::SymbolNode { .. }
                | Node::InterpolatedSymbolNode { .. }
                | Node::ArrayNode { .. }
                | Node::HashNode { .. }
                | Node::RegularExpressionNode { .. }
                | Node::InterpolatedRegularExpressionNode { .. }
                | Node::RangeNode { .. }
                | Node::TrueNode { .. }
                | Node::FalseNode { .. }
                | Node::NilNode { .. }
                // `__FILE__` and `__LINE__` are parser `str`/`int` literals,
                // `__ENCODING__` the constant `Encoding::UTF_8`.
                | Node::SourceFileNode { .. }
                | Node::SourceLineNode { .. }
                | Node::SourceEncodingNode { .. }
                | Node::ConstantReadNode { .. }
                | Node::ConstantPathNode { .. }
        )
}

/// A Ruby `Integer` or `Float` partial score.
#[derive(Clone, Copy)]
enum Score {
    Int(usize),
    Float(f64),
}

impl Score {
    fn to_f64(self) -> f64 {
        match self {
            Score::Int(n) => n as f64,
            Score::Float(f) => f,
        }
    }
}

/// Ruby's `Array#sum` of `simple ? 0.2 : 1`: integers add exactly until the
/// first float, then the Kahan-Babuska compensated sum continues from there.
fn ruby_array_sum(simple: impl Iterator<Item = bool>) -> Score {
    let mut integer = 0;
    // The running float sum and its compensation, from the first float on.
    let mut float: Option<(f64, f64)> = None;
    for simple in simple {
        let x = if simple { 0.2 } else { 1.0 };
        let (f, c) = match float {
            None if !simple => {
                integer += 1;
                continue;
            }
            None => (integer as f64, 0.0),
            Some(sum) => sum,
        };
        let t = f + x;
        let c = if f.abs() >= x.abs() {
            c + ((f - t) + x)
        } else {
            c + ((x - t) + f)
        };
        float = Some((t, c));
    }
    match float {
        Some((f, c)) => Score::Float(f + c),
        None => Score::Int(integer),
    }
}

/// Ruby's `Float#round` (half away from zero) of a non-negative score.
fn ruby_round(x: f64) -> usize {
    x.round() as usize
}

#[cfg(test)]
mod tests;
