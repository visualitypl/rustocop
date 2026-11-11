//! `Layout/MultilineArrayBraceLayout` — the closing bracket of a multi-line
//! array literal sits on the last element's line exactly when the opening
//! bracket sits on the first element's line.
//!
//! A port of RuboCop's `on_array` (97f30c4, MIT) at the fixed
//! `EnforcedStyle: symmetrical`. Every Prism `ArrayNode` with an opening
//! bracket (bracketed and percent arrays; the implicit arrays of assignments
//! and `rescue` lists have none), its elements as the children. The shared
//! `MultilineLiteralBraceLayout` projection (`multiline_literal_brace_layout`)
//! decides the findings and the guarded canonical edit.
//!
//! Upstream's `autocorrect_incompatible_with` (`MultilineHashBraceLayout`,
//! `MultilineMethodCallBraceLayout`, `Style/ConcatArrayLiterals`) only orders
//! RuboCop's correction rounds. It is not declared: rustocop withholds a
//! cop's edits whenever a declared companion is selected, which with the
//! default-on `MultilineHashBraceLayout` would withhold every edit. The edit
//! instead starts at the last element's last character, so a nested hash's
//! (or array's) brace edit always overlaps it and one `--fix` pass keeps one
//! of the two; the `Style/ConcatArrayLiterals` correction is unsafe and never
//! applied.
use ruby_prism::Node;

use super::multiline_literal_brace_layout::{List, Literal, Messages, check_brace_layout};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct MultilineArrayBraceLayout;

const MESSAGES: Messages = Messages {
    same_line: "The closing array brace must be on the same line as the last array element when \
                the opening brace is on the same line as the first array element.",
    new_line: "The closing array brace must be on the line after the last array element when the \
               opening brace is on a separate line from the first array element.",
};

impl Cop for MultilineArrayBraceLayout {
    fn name(&self) -> &'static str {
        "Layout/MultilineArrayBraceLayout"
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
        let Some(array) = node.as_array_node() else {
            return;
        };
        let children: Vec<_> = array.elements().iter().collect();
        let literal = Literal {
            node,
            opening: array.opening_loc().map(|l| l.start_offset()),
            closing: array
                .closing_loc()
                .map(|l| l.start_offset()..l.end_offset()),
            children: &children,
            same_kind_closing: |n| {
                n.as_array_node()
                    .and_then(|a| a.closing_loc())
                    .map(|l| l.start_offset()..l.end_offset())
            },
            scan: std::slice::from_ref(node),
            list: List::Literal,
        };
        check_brace_layout(self, &MESSAGES, &literal, cx, file, out);
    }
}

#[cfg(test)]
mod tests;
