//! `Style/EvenOdd` — parity comparisons, with conservative Integer-only fixes.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EvenOdd;
impl Cop for EvenOdd {
    fn name(&self) -> &'static str {
        "Style/EvenOdd"
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
        let Some((base, method)) = candidate(node, Some(file)) else {
            return;
        };
        let loc = node.location();
        let correction = if cx.corrections_requested() {
            correct(node, &base, method, cx, file)
        } else {
            None
        };
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            message: format!("Replace with `Integer#{method}?`."),
            correction,
        });
    }
}

/// One RuboCop `begin` grouping, not arbitrary nested parentheses or statements.
pub(super) fn single_group(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(p) = node.as_parentheses_node() {
        let s = p.body()?.as_statements_node()?;
        (s.body().len() == 1)
            .then(|| s.body().iter().next())
            .flatten()
    } else {
        Some(node)
    }
}
pub(super) fn integer(node: &Node<'_>, file: Option<&SourceFile>) -> Option<i32> {
    // Match the adapter's literal folding only for detection. A separated
    // minus is real Ruby dispatch and must not broaden the correction proof.
    if file.is_some()
        && let Some(literal) = ast::negated_numeric_literal(node)
    {
        let value: i32 = literal.as_integer_node()?.value().try_into().ok()?;
        return value.checked_neg();
    }
    if node.as_source_line_node().is_some() {
        return file?
            .line_of(node.location().start_offset())
            .try_into()
            .ok();
    }
    node.as_integer_node()?.value().try_into().ok()
}
fn remainder<'pr>(node: Node<'pr>, file: Option<&SourceFile>) -> Option<Node<'pr>> {
    let call = single_group(node)?.as_call_node()?;
    if call.name().as_slice() != b"%" || call.is_safe_navigation() || call.block().is_some() {
        return None;
    }
    let args = ast::call_arguments(&call);
    (args.len() == 1 && integer(&args[0], file) == Some(2))
        .then(|| call.receiver())
        .flatten()
}
fn candidate<'pr>(
    node: &Node<'pr>,
    file: Option<&SourceFile>,
) -> Option<(Node<'pr>, &'static str)> {
    let call = node.as_call_node()?;
    if call.is_safe_navigation() {
        return None;
    }
    let equal = match call.name().as_slice() {
        b"==" => true,
        b"!=" => false,
        _ => return None,
    };
    let args = ast::call_arguments(&call);
    if args.len() != 1 {
        return None;
    }
    let value = integer(&args[0], file)?;
    if !matches!(value, 0 | 1) {
        return None;
    }
    Some((
        remainder(call.receiver()?, file)?,
        if equal == (value == 0) { "even" } else { "odd" },
    ))
}
fn small_integer(node: &Node<'_>) -> bool {
    let value = if let Some(p) = node.as_parentheses_node() {
        single_group(p.as_node()).and_then(|n| integer(&n, None))
    } else {
        integer(node, None)
    };
    value.is_some_and(|n| (-(1 << 30)..(1 << 30)).contains(&n))
}
fn correct(
    node: &Node<'_>,
    base: &Node<'_>,
    method: &str,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if !small_integer(base) || candidate(node, None).is_none() {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.even_odd_safety(root);
    let range = node.location().start_offset()..ast::source_end(node);
    if facts.changed
        || facts.first_effect.is_some_and(|end| end <= range.start)
        || super::correction::contains_comment(file, range.clone())
        || super::correction::touches_directive(file, range.clone())
        || file.heredoc_pending_at(root, range.start)
    {
        return None;
    }
    let source = std::str::from_utf8(base.location().as_slice()).ok()?;
    // Negative/explicitly signed literal tokens must remain the method receiver,
    // not unary operators applied to the predicate's Boolean result.
    let source = if source.starts_with(['-', '+']) {
        format!("({source})")
    } else {
        source.into()
    };
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range,
            replacement: format!("{source}.{method}?"),
        },
    )
}

/// Lazy, one-walk closed-core dispatch proof. Never infer a local/call's type.
#[derive(Default)]
pub(crate) struct Safety {
    changed: bool,
    first_effect: Option<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        crate::cops::walk::walk(&mut facts, root);
        facts
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Safety {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        if let Some(call) = node.as_call_node() {
            let trusted = candidate(&node, None).is_some_and(|(base, _)| small_integer(&base))
                || remainder(call.as_node(), None).is_some_and(|base| small_integer(&base))
                || (matches!(call.name().as_slice(), b"even?" | b"odd?")
                    && call.receiver().is_some_and(|n| small_integer(&n))
                    && call.arguments().is_none()
                    && call.block().is_none()
                    && !call.is_safe_navigation());
            if !trusted || call.block().is_some() {
                self.changed |= call.block().is_some();
                let end = call.location().end_offset();
                self.first_effect = Some(self.first_effect.map_or(end, |old| old.min(end)));
            }
        } else {
            self.changed |= !matches!(
                node,
                Node::ProgramNode { .. }
                    | Node::StatementsNode { .. }
                    | Node::ArgumentsNode { .. }
                    | Node::IntegerNode { .. }
                    | Node::ParenthesesNode { .. }
                    | Node::ArrayNode { .. }
                    | Node::LocalVariableReadNode { .. }
                    | Node::LocalVariableWriteNode { .. }
                    | Node::SourceLineNode { .. }
                    | Node::TrueNode { .. }
                    | Node::FalseNode { .. }
                    | Node::NilNode { .. }
            );
        }
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/even_odd_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/EvenOdd")
            .collect()
    }
    #[test]
    fn pinned_parity_matrix() {
        for receiver in [
            "x", "y", "x.y", "x(y)", "a * b", "-a", "a[0]", "x._(y)", "(x._(y))", "1",
        ] {
            for (op, value, method) in [
                ("==", 0, "even"),
                ("!=", 0, "odd"),
                ("==", 1, "odd"),
                ("!=", 1, "even"),
            ] {
                for grouped in [false, true] {
                    let rem = format!("{receiver} % 2");
                    let rem = if grouped { format!("({rem})") } else { rem };
                    let s = format!("if {rem} {op} {value}\nend\n");
                    let found = offenses(&s);
                    assert_eq!(found.len(), 1, "{s}");
                    assert_eq!((found[0].line, found[0].col), (1, 4));
                    assert_eq!(
                        found[0].message,
                        format!("Replace with `Integer#{method}?`.")
                    );
                }
            }
        }
    }
    #[test]
    fn exact_literal_values_not_spellings() {
        for s in ["x % 0x2 == 0b0", "x % 02 != 0x1", "x % +2 == -0"] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "x % 2 == 2",
            "x % 3 == 0",
            "x % 3 != 0",
            "x % 2.0 == 0",
            "x % 2r == 0",
            "x % 2i == 0",
            "x % 2 == 0.0",
            "x % 2 == 1r",
            "x % 2 == 0i",
            "0 == x % 2",
            "((x % 2)) == 0",
            "(x % 2)&.==(0)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn spaced_and_multiline_minus_are_detection_only() {
        for s in [
            "x % 2 == - 0",
            "x % - -2 == 0",
            "x % -\n-2 == 0",
            "x % 2 == -\n0",
            "x % 2 == - +0",
            "2 % 2 == - 0",
            "2 % - -2 == 0",
            "- 2 % 2 == 0",
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(found[0].message, "Replace with `Integer#even?`.");
            assert!(
                correction_offenses(&EvenOdd, s)[0].correction.is_none(),
                "{s}"
            );
        }
        for s in [
            "x % + 2 == 0",
            "x % - - 2 == 0",
            "x % 2 == - - 0",
            "x % 2 == - + 0",
            "x % 2 == 0.-@",
            "x % 2 == -(0)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn line_macros_are_exact_integer_values_for_detection() {
        for s in [
            "\nx % __LINE__ == 0",
            "x % 2 == __LINE__",
            "\n2 % __LINE__ == 0",
            "2 % 2 == __LINE__",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
            let found = correction_offenses(&EvenOdd, s);
            assert!(found[0].correction.is_none(), "uncertain macro edit: {s}");
        }
        assert!(offenses("\nx % 2 == __LINE__").is_empty());
    }
    #[test]
    fn literal_integer_corrections() {
        for (s, after) in [
            ("1 % 2 != 1", "1.even?"),
            ("0x10 % 02 == 0", "0x10.even?"),
            ("-3 % 2 == 1", "(-3).odd?"),
            ("(+2 % 2) != 0", "(+2).odd?"),
            ("(0) % 2 != 1", "(0).even?"),
        ] {
            assert_correction(&EvenOdd, s, after);
        }
    }
    #[test]
    fn uncertain_numeric_receivers_and_dispatch_remain_reported() {
        for s in [
            "x % 2 == 0",
            "x.y % 2 == 1",
            "a * b % 2 == 0",
            "-a % 2 == 0",
            "a[0] % 2 == 0",
            "1.0 % 2 == 0",
            "1r % 2 == 1",
            "1i % 2 == 1",
            "9999999999999999999999999 % 2 == 1",
            "class Integer; def even?; false; end; end; 2 % 2 == 0",
            "patch; 2 % 2 == 0",
            "def f; 2 % 2 == 0; end",
            "(2 % #keep\n 2) == 0",
        ] {
            let found = correction_offenses(&EvenOdd, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
}
