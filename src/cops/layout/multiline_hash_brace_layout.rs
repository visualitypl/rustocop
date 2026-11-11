//! `Layout/MultilineHashBraceLayout` — the closing brace of a multi-line
//! hash literal sits on the last element's line exactly when the opening
//! brace sits on the first element's line.
//!
//! A port of RuboCop's `on_hash` (97f30c4, MIT) at the fixed
//! `EnforcedStyle: symmetrical`. Every Prism `HashNode` (always braced; the
//! brace-less keyword hashes of calls and arrays are `KeywordHashNode`s, the
//! parser's implicit hashes), its pairs and `**` splats as the children. The
//! shared `MultilineLiteralBraceLayout` projection
//! (`multiline_literal_brace_layout`) decides the findings and the guarded
//! canonical edit.
//!
//! Upstream's `autocorrect_incompatible_with`
//! (`MultilineMethodCallBraceLayout`, `Style/HashAsLastArrayItem`,
//! `Style/RedundantDoubleSplatHashBraces`) only orders RuboCop's correction
//! rounds. It is not declared: rustocop withholds a cop's edits whenever a
//! declared companion is selected, and both Style peers are default-on. Each
//! of their edits is one contiguous replacement of the whole hash (or
//! keyword hash), so it always overlaps this cop's edit, which starts at the
//! last element's last character; one `--fix` pass keeps one of the two.
use ruby_prism::Node;

use super::multiline_literal_brace_layout::{List, Literal, Messages, check_brace_layout};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct MultilineHashBraceLayout;

const MESSAGES: Messages = Messages {
    same_line: "Closing hash brace must be on the same line as the last hash element when opening \
                brace is on the same line as the first hash element.",
    new_line: "Closing hash brace must be on the line after the last hash element when opening \
               brace is on a separate line from the first hash element.",
};

impl Cop for MultilineHashBraceLayout {
    fn name(&self) -> &'static str {
        "Layout/MultilineHashBraceLayout"
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
        let children: Vec<_> = hash.elements().iter().collect();
        let literal = Literal {
            node,
            opening: Some(hash.opening_loc().start_offset()),
            closing: Some(hash.closing_loc()).map(|l| l.start_offset()..l.end_offset()),
            children: &children,
            same_kind_closing: |n| {
                n.as_hash_node()
                    .map(|h| h.closing_loc())
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
