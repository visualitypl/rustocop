//! `Layout/FirstArrayElementIndentation` — fixed `special_inside_parentheses`:
//! the first element of a multi-line bracketed or percent array is indented
//! two columns from the parent hash key, the first position after a `(` on
//! the bracket's line, or the start of that line; a `]` that begins its line
//! is aligned with the same base.
use super::{
    element_indentation::column,
    multiline_element_indentation::{Base, indent_base, left_parenthesis},
};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct FirstArrayElementIndentation;
impl Cop for FirstArrayElementIndentation {
    fn name(&self) -> &'static str {
        "Layout/FirstArrayElementIndentation"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_array_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(array) = node.as_array_node() else {
            return;
        };
        let (Some(open), Some(close)) = (array.opening_loc(), array.closing_loc()) else {
            return;
        };
        let left = open.start_offset();
        let ancestors: Vec<&Node<'pr>> = cx.ancestors().collect();
        let paren = left_parenthesis(file, node, left, &ancestors);
        let first = array.elements().iter().next();
        let root = || cx.ancestors().next_back().unwrap_or(node);
        if let Some(first) = &first {
            let loc = first.location();
            if file.line_of(loc.start_offset()) == file.line_of(left) {
                return;
            }
            let (base, kind) = indent_base(file, left, true, &ancestors, paren);
            let delta =
                (base + super::alignment::INDENT) as i64 - column(file, loc.start_offset()) as i64;
            if delta != 0 {
                let (line, col) = file.line_col(loc.start_offset());
                out.push(Offense {
                    correction: cx
                        .corrections_requested()
                        .then(|| {
                            super::alignment::Alignment {
                                expression: loc.start_offset()..loc.end_offset(),
                                delta,
                            }
                            .correction(file, root())
                        })
                        .flatten(),
                    cop: self.name(),
                    severity: self.severity(),
                    message: format!(
                        "Use 2 spaces for indentation in an array, relative to {}.",
                        match kind {
                            Base::AfterLeftParenthesis =>
                                "the first position after the preceding left parenthesis",
                            Base::ParentHashKey => "the parent hash key",
                            Base::StartOfLine =>
                                "the start of the line where the left square bracket is",
                        }
                    ),
                    line,
                    col,
                    end_line: file.last_line_of(loc),
                });
            }
        }
        // `check_right_bracket`, independent of the first element's offense.
        let right = close.start_offset();
        if !super::alignment::starts_line(file, right) {
            return;
        }
        let (expected, kind) = indent_base(file, left, first.is_some(), &ancestors, paren);
        let delta = expected as i64 - column(file, right) as i64;
        if delta == 0 {
            return;
        }
        let (line, col) = file.line_col(right);
        out.push(Offense {
            correction: cx
                .corrections_requested()
                .then(|| {
                    super::alignment::Alignment {
                        expression: right..close.end_offset(),
                        delta,
                    }
                    .correction(file, root())
                })
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message: match kind {
                Base::AfterLeftParenthesis => {
                    "Indent the right bracket the same as the first position after the \
                     preceding left parenthesis."
                }
                Base::ParentHashKey => "Indent the right bracket the same as the parent hash key.",
                Base::StartOfLine => {
                    "Indent the right bracket the same as the start of the line where the left \
                     bracket is."
                }
            }
            .into(),
            line,
            col,
            end_line: line,
        });
    }
}
#[cfg(test)]
mod tests;
