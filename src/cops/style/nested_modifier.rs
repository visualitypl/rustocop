//! `Style/NestedModifier` — nested postfix conditionals.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node};
pub struct NestedModifier;
impl Cop for NestedModifier {
    fn name(&self) -> &'static str {
        "Style/NestedModifier"
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
        let Some(inner) = Modifier::of(node) else {
            return;
        };
        let mut ancestors = cx.ancestors().filter(|n| !transparent(n));
        let Some(parent) = ancestors.next() else {
            return;
        };
        let Some(outer) = Modifier::of(parent) else {
            return;
        };
        // RuboCop ignores the entire subtree of the first (outermost) pair.
        let mut previous_modifier = true;
        for ancestor in ancestors {
            let modifier = Modifier::of(ancestor).is_some();
            if modifier && previous_modifier {
                return;
            }
            previous_modifier = modifier;
        }
        let (line, col) = file.line_col(inner.keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Avoid using nested modifiers.".into(),
            correction: cx
                .corrections_requested()
                .then(|| rewrite(&inner, &outer))
                .flatten(),
        });
    }
}
fn transparent(n: &Node<'_>) -> bool {
    n.as_statements_node().is_some_and(|s| s.body().len() == 1)
}
struct Modifier<'pr> {
    keyword: Location<'pr>,
    condition: Node<'pr>,
    conditional: bool,
}
impl<'pr> Modifier<'pr> {
    fn of(n: &Node<'pr>) -> Option<Self> {
        if let Some(c) = n.as_if_node() {
            if c.end_keyword_loc().is_some() {
                return None;
            }
            return Some(Self {
                keyword: c.if_keyword_loc()?,
                condition: c.predicate(),
                conditional: true,
            });
        }
        if let Some(c) = n.as_unless_node() {
            if c.end_keyword_loc().is_some() {
                return None;
            }
            return Some(Self {
                keyword: c.keyword_loc(),
                condition: c.predicate(),
                conditional: true,
            });
        }
        if let Some(c) = n.as_while_node() {
            if c.closing_loc().is_some() || c.is_begin_modifier() {
                return None;
            }
            return Some(Self {
                keyword: c.keyword_loc(),
                condition: c.predicate(),
                conditional: false,
            });
        }
        let c = n.as_until_node()?;
        if c.closing_loc().is_some() || c.is_begin_modifier() {
            return None;
        }
        Some(Self {
            keyword: c.keyword_loc(),
            condition: c.predicate(),
            conditional: false,
        })
    }
}
/// RuboCop's corrector: one modifier, the outer keyword's, joining both
/// conditions with `&&` (`if`) or `||` (`unless`); the inner condition is
/// negated when the keywords differ.
fn rewrite(inner: &Modifier<'_>, outer: &Modifier<'_>) -> Option<Correction> {
    if !inner.conditional || !outer.conditional {
        return None;
    }
    let is_if = outer.keyword.as_slice() == b"if";
    let operator = if is_if { "&&" } else { "||" };
    let mut left = operand_source(&outer.condition)?;
    if (outer.condition.as_or_node().is_some() && is_if) || low_precedence(&outer.condition) {
        left = format!("({left})");
    }
    let negated = inner.keyword.as_slice() != outer.keyword.as_slice();
    let mut right = operand_source(&inner.condition)?;
    let c = &inner.condition;
    let operator_keyword = c.as_and_node().is_some() || c.as_or_node().is_some();
    let comparison = c.as_call_node().is_some_and(|call| {
        matches!(
            call.name().as_slice(),
            b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<"
        )
    });
    if (negated && operator_keyword) || c.as_or_node().is_some() || low_precedence(c) || comparison
    {
        right = format!("({right})");
    }
    if negated {
        right = format!("!{right}");
    }
    Some(Correction {
        range: inner.keyword.start_offset()..outer.condition.location().end_offset(),
        replacement: format!(
            "{} {left} {operator} {right}",
            std::str::from_utf8(outer.keyword.as_slice()).ok()?
        ),
    })
}

/// RuboCop's `operand_source`: a call with arguments and no operator gets
/// them in parentheses.
fn operand_source(n: &Node<'_>) -> Option<String> {
    let text = |n: &Node<'_>| {
        std::str::from_utf8(n.location().as_slice())
            .ok()
            .map(str::to_owned)
    };
    let Some(call) = n.as_call_node().filter(|c| {
        !c.is_safe_navigation()
            && !c.block().is_some_and(|b| b.as_block_node().is_some())
            && c.arguments().is_some()
            && !super::super::layout::multiline_expression::operator_method(c.name().as_slice())
    }) else {
        return text(n);
    };
    let mut source = String::new();
    if let Some(receiver) = call.receiver() {
        source.push_str(&text(&receiver)?);
        source.push('.');
    }
    source.push_str(std::str::from_utf8(call.name().as_slice()).ok()?);
    let mut arguments = ast::call_arguments(&call)
        .iter()
        .map(text)
        .collect::<Option<Vec<_>>>()?;
    if let Some(block) = call.block() {
        arguments.push(text(&block)?);
    }
    source.push_str(&format!("({})", arguments.join(", ")));
    Some(source)
}

/// RuboCop's `low_precedence?`.
fn low_precedence(n: &Node<'_>) -> bool {
    match n {
        Node::AndNode { .. } => n.as_and_node().unwrap().operator_loc().as_slice() == b"and",
        Node::OrNode { .. } => n.as_or_node().unwrap().operator_loc().as_slice() == b"or",
        Node::IfNode { .. } => n.as_if_node().unwrap().if_keyword_loc().is_none(),
        Node::CallNode { .. } => {
            let c = n.as_call_node().unwrap();
            c.message_loc().is_some_and(|m| m.as_slice() == b"not")
                || (c.is_safe_navigation()
                    && c.arguments().is_some()
                    && c.opening_loc().is_none()
                    && !super::super::layout::multiline_expression::operator_method(
                        c.name().as_slice(),
                    ))
        }
        Node::YieldNode { .. } => {
            let y = n.as_yield_node().unwrap();
            y.arguments().is_some() && y.lparen_loc().is_none()
        }
        Node::SuperNode { .. } => {
            let s = n.as_super_node().unwrap();
            s.arguments().is_some() && s.lparen_loc().is_none()
        }
        _ => super::redundant_parentheses::is_assignment(n),
    }
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! Ported from RuboCop nested_modifier_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    #[test]
    fn post_test_loops_are_not_basic_modifiers() {
        for source in [
            "begin; work; end while a if b\n",
            "work if (begin; work; end until a)\n",
            "begin; work if a; end until b\n",
        ] {
            assert!(
                crate::pipeline::lint_str_with(&[&NestedModifier], source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn non_nested_lookalikes() {
        for source in [
            "work if a && b\n",
            "if a\n work if b\nend\n",
            "work if (b if a)\n",
            "work rescue fallback\n",
            "work while a\n",
            "begin\n work\nend while a\n",
            "a ? b : c\n",
        ] {
            assert!(
                crate::pipeline::lint_str_with(&[&NestedModifier], source).is_empty(),
                "{source}"
            );
        }
    }
}
