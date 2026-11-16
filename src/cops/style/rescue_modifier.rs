//! `Style/RescueModifier` — prefer normal begin/rescue syntax.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RescueModifier;
impl Cop for RescueModifier {
    fn name(&self) -> &'static str {
        "Style/RescueModifier"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/MethodCallWithArgsParentheses"]
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_rescue_modifier_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(rescue) = node.as_rescue_modifier_node() else {
            return;
        };
        let loc = rescue.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1)),
            message: "Avoid using `rescue` in its modifier form.".into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, cx, file))
                .flatten(),
        });
    }
}

/// RuboCop's corrector: the modifier becomes a `begin`/`rescue`/`end`
/// block at the column of the expression, an enclosing pair of
/// parentheses is removed, and an unbracketed array gets brackets.
fn correction(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    use crate::offense::Correction;
    let rescue = node.as_rescue_modifier_node()?;
    let operation = rescue.expression();
    let fallback = rescue.rescue_expression();
    let heredoc_end = |n: &Node<'_>| {
        let mut bodies = Vec::new();
        super::file_write::heredoc_bodies(n, &mut bodies);
        bodies.iter().map(|b| b.end).max()
    };
    // The fallback's heredoc body would be left behind, and an enclosing
    // modifier whose fallback this is moves it already.
    let start = node.location().start_offset();
    if heredoc_end(&fallback).is_some()
        || cx.ancestors().any(|a| {
            a.as_rescue_modifier_node().is_some_and(|r| {
                let f = r.rescue_expression().location();
                f.start_offset() <= start && node.location().end_offset() <= f.end_offset()
            })
        })
    {
        return None;
    }
    let parentheses = cx
        .parent()
        .filter(|p| p.as_statements_node().is_some())
        .and_then(|_| cx.ancestors().nth(1))
        .and_then(|p| p.as_parentheses_node());
    let column = file.line_col(start).1 as usize - 1;
    let shift = usize::from(parentheses.is_some());
    let offset = " ".repeat(column.saturating_sub(shift));
    let indentation = " ".repeat((column + 2).saturating_sub(shift));
    let text = |n: &Node<'_>| {
        std::str::from_utf8(n.location().as_slice())
            .ok()
            .map(str::to_owned)
    };
    let op = operation.location();
    let insert = |at: usize, text: String| Correction {
        range: at..at,
        replacement: text,
    };
    let mut edits = vec![insert(op.start_offset(), format!("begin\n{indentation}"))];
    let unbracketed = operation
        .as_array_node()
        .is_some_and(|a| a.opening_loc().is_none());
    if unbracketed {
        edits.push(insert(op.start_offset(), "[".into()));
        edits.push(insert(op.end_offset(), "]".into()));
    }
    edits.push(Correction {
        range: op.end_offset()..node.location().end_offset(),
        replacement: String::new(),
    });
    edits.push(insert(
        heredoc_end(&operation).unwrap_or(op.end_offset()),
        format!(
            "\n{offset}rescue\n{indentation}{}\n{offset}end",
            text(&fallback)?
        ),
    ));
    if let Some(parentheses) = parentheses {
        let bytes = &file.bytes;
        let open = parentheses.opening_loc();
        let mut right = open.end_offset();
        while bytes.get(right).is_some_and(u8::is_ascii_whitespace) {
            right += 1;
        }
        edits.push(Correction {
            range: open.start_offset()..right,
            replacement: String::new(),
        });
        let close = parentheses.closing_loc();
        let mut left = close.start_offset();
        while left > 0 && matches!(bytes[left - 1], b' ' | b'\t') {
            left -= 1;
        }
        while left > 0 && bytes[left - 1] == b'\n' {
            left -= 1;
        }
        edits.push(Correction {
            range: left..close.end_offset(),
            replacement: String::new(),
        });
    }
    super::correction::combine(file, edits)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/rescue_modifier_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;

    #[test]
    fn canonical_expansion_for_closed_standalone_literals() {
        for (before, after) in [
            ("1 rescue nil", "begin\n  1\nrescue\n  nil\nend"),
            (
                "(false rescue true)\n",
                "begin\n  false\nrescue\n  true\nend\n",
            ),
            (
                "  \"ok\" rescue nil\n",
                "  begin\n    \"ok\"\n  rescue\n    nil\n  end\n",
            ),
        ] {
            assert_correction(&RescueModifier, before, after);
        }
    }
}
