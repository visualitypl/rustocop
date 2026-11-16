//! `Style/NegatedIf` — fixed `both` prefix/modifier policy.

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Location, Node};

pub struct NegatedIf;
impl Cop for NegatedIf {
    fn name(&self) -> &'static str {
        "Style/NegatedIf"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_if_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(n) = node.as_if_node() else { return };
        let Some(keyword) = n.if_keyword_loc().filter(|k| k.as_slice() == b"if") else {
            return;
        };
        if n.subsequent().is_some() {
            return;
        }
        check_single(self, node, keyword, n.predicate(), "unless", cx, file, out);
    }
}

// Parentheses are RuboCop begin nodes, but keyword begin is deliberately not
// unwrapped by the single-branch rules. The else rule uses the first child.
pub(super) fn unwrap(mut n: Node<'_>, first: bool) -> Option<Node<'_>> {
    loop {
        let body = if let Some(p) = n.as_parentheses_node() {
            p.body()
        } else if first {
            n.as_begin_node()
                .filter(|b| {
                    b.rescue_clause().is_none()
                        && b.ensure_clause().is_none()
                        && b.else_clause().is_none()
                })
                .and_then(|b| b.statements().map(|s| s.as_node()))
        } else {
            None
        };
        let Some(body) = body else { return Some(n) };
        let Some(s) = body.as_statements_node() else {
            n = body;
            continue;
        };
        n = if first {
            s.body().first()?
        } else {
            s.body().iter().last()?
        };
    }
}
pub(super) fn negation<'pr>(n: &Node<'pr>) -> Option<CallNode<'pr>> {
    n.as_call_node().filter(|c| {
        c.name().as_slice() == b"!"
            && !c.is_safe_navigation()
            && c.receiver().is_some()
            && c.arguments().is_none_or(|a| a.arguments().is_empty())
            && c.block().is_none()
    })
}
#[allow(clippy::too_many_arguments)]
pub(super) fn check_single(
    cop: &dyn Cop,
    node: &Node<'_>,
    keyword: Location<'_>,
    predicate: Node<'_>,
    inverse: &str,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let Some(condition) = unwrap(predicate, false) else {
        return;
    };
    let Some(call) = negation(&condition) else {
        return;
    };
    if call
        .receiver()
        .as_ref()
        .is_some_and(|r| negation(r).is_some())
    {
        return;
    }
    let correction = cx
        .corrections_requested()
        .then(|| {
            // RuboCop's ConditionCorrector: the inverse keyword, and the
            // condition without its negation.
            let receiver = call.receiver()?;
            super::correction::combine(
                file,
                vec![
                    Correction {
                        range: keyword.start_offset()..keyword.end_offset(),
                        replacement: inverse.into(),
                    },
                    Correction {
                        range: call.location().start_offset()..call.location().end_offset(),
                        replacement: text(&receiver)?.into(),
                    },
                ],
            )
        })
        .flatten();
    report(
        cop,
        node,
        file,
        format!(
            "Favor `{inverse}` over `{}` for negative conditions.",
            String::from_utf8_lossy(keyword.as_slice())
        ),
        correction,
        out,
    );
}
pub(super) fn text<'a>(n: &'a Node<'_>) -> Option<&'a str> {
    std::str::from_utf8(n.location().as_slice()).ok()
}
pub(super) fn report(
    cop: &dyn Cop,
    node: &Node<'_>,
    file: &SourceFile,
    message: String,
    correction: Option<Correction>,
    out: &mut Vec<Offense>,
) {
    let (line, col) = file.line_col(node.location().start_offset());
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        line,
        col,
        end_line: file.last_line_of(node.location()),
        message,
        correction,
    });
}

// Use the same complete dispatch proof for a correction and for exempting an
// earlier call from the effect index. Integer#!= can invoke an unknown RHS's ==.
fn closed_primitive(call: &CallNode<'_>) -> bool {
    if call.block().is_some()
        || call.is_safe_navigation()
        || !call.receiver().is_some_and(immediate)
    {
        return false;
    }
    let args = crate::cops::ast::call_arguments(call);
    call.name().as_slice() == b"!" && args.is_empty()
        || call.name().as_slice() == b"!="
            && args.len() == 1
            && args.into_iter().next().is_some_and(immediate)
}

fn immediate(n: Node<'_>) -> bool {
    if let Some(p) = n.as_parentheses_node() {
        return p.body().is_some_and(immediate);
    }
    if let Some(s) = n.as_statements_node() {
        return s.body().len() == 1 && s.body().first().is_some_and(immediate);
    }
    super::case_equality::immediate(&n)
}

/// One lazy main-parse dispatch/declaration index shared by all three cops.
#[derive(Default)]
pub(crate) struct Safety {
    changed: bool,
    effects: Vec<usize>,
    pub(super) declarations: Vec<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    /// Shared proof for a newly inserted immediate-operand negation. Callers
    /// also exclude repeated execution and prove the operand itself is closed.
    pub(super) fn before_effects(&self, at: usize) -> bool {
        !self.changed && self.effects.first().is_none_or(|&end| end > at)
    }
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[cfg(test)]
        BUILDS.with(|n| n.set(n.get() + 1));
        let mut facts = Self::default();
        crate::cops::walk::walk(&mut facts, root);
        facts.effects.sort_unstable();
        if let Some(program) = root.as_program_node() {
            let analysis = crate::cops::variables::ScopeAnalysis::of(&program);
            for scope in analysis.scopes_in_exit_order() {
                for var in analysis.scope(scope).variables() {
                    facts
                        .declarations
                        .push(analysis.variable(var).declaration_range.start);
                }
            }
            facts.declarations.sort_unstable();
        }
        facts
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Safety {
    fn enter(&mut self, n: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
            VISITS.with(|n| n.set(n.get() + 1));
        }
        // A pre-test loop cannot run body effects before its first predicate.
        // Its completion remains a barrier for subsequent candidates. Nested
        // conditional corrections are separately excluded by `trusted`.
        if matches!(n, Node::WhileNode { .. } | Node::UntilNode { .. }) {
            self.effects.push(n.location().end_offset());
            return;
        }
        if let Some(c) = n.as_call_node() {
            if !closed_primitive(&c) {
                self.effects.push(n.location().end_offset());
            }
            self.changed |= c.block().is_some();
            return;
        }
        self.changed |= matches!(
            n,
            Node::DefNode { .. }
                | Node::ClassNode { .. }
                | Node::ModuleNode { .. }
                | Node::SingletonClassNode { .. }
                | Node::PreExecutionNode { .. }
                | Node::PostExecutionNode { .. }
                | Node::AliasMethodNode { .. }
                | Node::UndefNode { .. }
        );
        // Only syntax known not to dispatch implicitly is transparent. Splat
        // conversions, interpolation, hash keys, destructuring, pattern matching,
        // etc. can execute user hooks without a CallNode. Use their start as a
        // conservative barrier: conversions may precede a nested candidate while
        // their enclosing expression/call has not finished yet. A conversion
        // lexically after a candidate does not invalidate that earlier proof.
        if !matches!(
            n,
            Node::ProgramNode { .. }
                | Node::StatementsNode { .. }
                | Node::ArgumentsNode { .. }
                | Node::ParenthesesNode { .. }
                | Node::IfNode { .. }
                | Node::UnlessNode { .. }
                | Node::ElseNode { .. }
                | Node::BeginNode { .. }
                | Node::AndNode { .. }
                | Node::OrNode { .. }
                | Node::LocalVariableReadNode { .. }
                | Node::LocalVariableWriteNode { .. }
                | Node::GlobalVariableReadNode { .. }
                | Node::InstanceVariableReadNode { .. }
                | Node::ClassVariableReadNode { .. }
                | Node::IntegerNode { .. }
                | Node::FloatNode { .. }
                | Node::RationalNode { .. }
                | Node::ImaginaryNode { .. }
                | Node::NilNode { .. }
                | Node::TrueNode { .. }
                | Node::FalseNode { .. }
                | Node::SelfNode { .. }
                | Node::ArrayNode { .. }
                | Node::StringNode { .. }
                | Node::SymbolNode { .. }
                | Node::RegularExpressionNode { .. }
                | Node::SourceLineNode { .. }
                | Node::SourceFileNode { .. }
                | Node::SourceEncodingNode { .. }
        ) {
            self.effects.push(n.location().start_offset());
        }
    }
}

#[cfg(test)]
thread_local! {
    static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop negated_if_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/NegatedIf")
            .collect()
    }
    #[test]
    fn negative_prefix_and_modifier_locations_and_messages() {
        for op in ["!", "not "] {
            let s =
                format!("if {op}a_condition\n  some_method\nend\nsome_method if {op}a_condition\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 2);
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 1, 3));
            assert_eq!((o[1].line, o[1].col), (4, 1));
            assert_eq!(
                o[0].message,
                "Favor `unless` over `if` for negative conditions."
            );
        }
    }
    #[test]
    fn canonical_closed_immediate_corrections_preserve_parentheses_and_modifier_scope() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("if !false\n  work\nend\n", "unless false\n  work\nend\n"),
            (":work if not true\n", ":work unless true\n"),
            (":work if (!nil)\n", ":work unless (nil)\n"),
            ("value = 1 if !false\n", "value = 1 unless false\n"),
        ] {
            assert_correction(&super::NegatedIf, before, after);
        }
    }
    #[test]
    fn ignores_else_elsif_double_partial_and_empty_conditions() {
        for s in [
            "if !x\n a\nelse\n b\nend",
            "if x\n a\nelsif !y\n b\nend",
            "if !!x\n a\nend",
            "a if not x or y",
            "if !x && y\n a\nend",
            "if (); end",
            "!x ? a : b",
            "unless !x; a; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
