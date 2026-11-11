//! `Layout/FirstHashElementIndentation` — fixed `special_inside_parentheses`
//! with `HashAlignment`'s fixed `key` styles: the first pair of a multi-line
//! braced hash that begins its line is indented two columns from the parent
//! hash key, the first position after a `(` on the brace's line, or the start
//! of that line; a `}` that begins its line is aligned with the same base.
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
pub struct FirstHashElementIndentation;
impl Cop for FirstHashElementIndentation {
    fn name(&self) -> &'static str {
        "Layout/FirstHashElementIndentation"
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
        let Some(hash) = node.as_hash_node() else {
            return;
        };
        let (open, close) = (hash.opening_loc(), hash.closing_loc());
        let left = open.start_offset();
        let ancestors: Vec<&Node<'pr>> = cx.ancestors().collect();
        let paren = left_parenthesis(file, node, left, &ancestors);
        // `hash_node.pairs.first`: keyword splats are not pairs.
        let first = hash.elements().iter().find_map(|e| e.as_assoc_node());
        let root = || cx.ancestors().next_back().unwrap_or(node);
        if let Some(pair) = &first {
            let loc = pair.location();
            if file.line_of(loc.start_offset()) == file.line_of(left) {
                return;
            }
            let (base, kind) = indent_base(file, left, true, &ancestors, paren);
            let delta =
                (base + super::alignment::INDENT) as i64 - column(file, loc.start_offset()) as i64;
            if super::alignment::starts_line(file, loc.start_offset()) && delta != 0 {
                // A value starting below its key moves only the key's line.
                let key_line = file.line_of(pair.key().location().start_offset());
                let end = if file.line_of(pair.value().location().start_offset()) <= key_line {
                    loc.end_offset()
                } else {
                    file.line_start(key_line) + file.line_text(key_line).len()
                };
                let (line, col) = file.line_col(loc.start_offset());
                out.push(Offense {
                    correction: cx
                        .corrections_requested()
                        .then(|| {
                            super::alignment::Alignment {
                                expression: loc.start_offset()..end,
                                delta,
                            }
                            .correction(file, root())
                        })
                        .flatten(),
                    cop: self.name(),
                    severity: self.severity(),
                    message: format!(
                        "Use 2 spaces for indentation in a hash, relative to {}.",
                        match kind {
                            Base::AfterLeftParenthesis =>
                                "the first position after the preceding left parenthesis",
                            Base::ParentHashKey => "the parent hash key",
                            Base::StartOfLine =>
                                "the start of the line where the left curly brace is",
                        }
                    ),
                    line,
                    col,
                    end_line: file.last_line_of(loc),
                });
            }
        }
        // `check_right_brace`, independent of the first pair's offense.
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
                    "Indent the right brace the same as the first position after the \
                     preceding left parenthesis."
                }
                Base::ParentHashKey => "Indent the right brace the same as the parent hash key.",
                Base::StartOfLine => {
                    "Indent the right brace the same as the start of the line where the left \
                     brace is."
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
