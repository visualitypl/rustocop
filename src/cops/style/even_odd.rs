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
            correct(node, &base, method)
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
/// RuboCop's corrector: an operator receiver other than `[]` binds looser
/// than the appended call, so it is wrapped in parentheses.
fn correct(node: &Node<'_>, base: &Node<'_>, method: &str) -> Option<Correction> {
    let source = std::str::from_utf8(base.location().as_slice()).ok()?;
    let operator = base.as_call_node().is_some_and(|c| {
        let name = c.name().as_slice();
        name != b"[]" && super::super::layout::multiline_expression::operator_method(name)
    });
    let source = if operator {
        format!("({source})")
    } else {
        source.into()
    };
    Some(Correction {
        range: node.location().start_offset()..ast::source_end(node),
        replacement: format!("{source}.{method}?"),
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/even_odd_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
    #[test]
    fn corrects_as_rubocop() {
        assert_correction(&EvenOdd, "x % 2 == 0\n", "x.even?\n");
        assert_correction(&EvenOdd, "(x - y) % 2 != 0\n", "(x - y).odd?\n");
        assert_correction(&EvenOdd, "-1 % 2 == 1\n", "-1.odd?\n");
    }
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
}
