//! The two adjacency cops share Prism's real concatenation children. Ordinary
//! interpolation fragments and heredoc bodies are not adjacent quoted literals.
use crate::{offense::Correction, source::SourceFile};
use ruby_prism::Node;

pub(super) fn children<'pr>(node: &Node<'pr>) -> Option<Vec<Node<'pr>>> {
    let string = node.as_interpolated_string_node()?;
    string
        .opening_loc()
        .is_none()
        .then(|| string.parts().iter().collect())
}

/// Pinned parser's direct empty str child (not a zero-child multiline dstr).
pub(super) fn empty(node: &Node<'_>) -> bool {
    node.as_string_node()
        .is_some_and(|s| s.unescaped().is_empty() && !s.content_loc().as_slice().contains(&b'\n'))
}

pub(super) fn display(node: &Node<'_>, file: &SourceFile) -> String {
    let raw = node.location().as_slice();
    if !raw.contains(&b'\n') {
        return String::from_utf8_lossy(&file.match_pattern_source(raw)).replace("\r\n", "\n");
    }
    // Pinned str_content recursively visits ALL AST children, not just the
    // outer string's fragments. Project str descendants in this literal's
    // subtree only, including Parser's synthetic leaves, without evaluating
    // interpolation (especially xstrings) or reparsing source.
    let mut projection = StrContent {
        file,
        cooked: Vec::new(),
        parents: Vec::new(),
    };
    crate::cops::walk::walk(&mut projection, node);
    let cooked = projection.cooked;
    let mut inspected = Vec::new();
    if file.match_pattern_binary() {
        super::duplicate_match_pattern::inspect_source(&cooked, file, &mut inspected);
    } else {
        // Numeric escapes can yield invalid UTF-8 even though the projected
        // source buffer is valid. Inspect those cooked bytes as \\xNN, not as
        // source glyphs or lossy replacement characters. Keep prior source-
        // identity consumers of inspect_source unchanged.
        inspected.push(b'"');
        for chunk in cooked.utf8_chunks() {
            let mut valid = Vec::new();
            super::duplicate_match_pattern::inspect_source(
                chunk.valid().as_bytes(),
                file,
                &mut valid,
            );
            inspected.extend_from_slice(&valid[1..valid.len() - 1]);
            for byte in chunk.invalid() {
                inspected.extend_from_slice(format!("\\x{byte:02X}").as_bytes());
            }
        }
        inspected.push(b'"');
    }
    String::from_utf8_lossy(&inspected).into_owned()
}

struct StrContent<'file, 'pr> {
    file: &'file SourceFile,
    cooked: Vec<u8>,
    parents: Vec<Node<'pr>>,
}
impl StrContent<'_, '_> {
    fn literal(&mut self, raw: &[u8], value: &[u8], single: bool) {
        for part in crate::cops::ast::require_literal::parts(raw, value, single, self.file) {
            self.cooked.extend(part);
        }
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for StrContent<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if let Some(s) = node.as_string_node() {
            let regex_fragment = self.parents.last().is_some_and(|p| {
                matches!(
                    p,
                    Node::InterpolatedRegularExpressionNode { .. }
                        | Node::InterpolatedMatchLastLineNode { .. }
                )
            });
            if regex_fragment {
                // Regex payload keeps regexp escapes (\\d, \\xNN, etc.), unlike
                // cooked string escapes. Prism already supplied that payload.
                self.cooked
                    .extend_from_slice(&self.file.match_pattern_source(s.unescaped()));
            } else {
                let single = s.opening_loc().is_some_and(|o| {
                    o.as_slice().ends_with(b"'") || o.as_slice().starts_with(b"%q")
                }) || self
                    .parents
                    .last()
                    .and_then(Node::as_array_node)
                    .and_then(|a| a.opening_loc())
                    .is_some_and(|o| o.as_slice().starts_with(b"%w"));
                self.literal(s.content_loc().as_slice(), s.unescaped(), single);
            }
        } else if let Some(s) = node.as_regular_expression_node() {
            self.cooked
                .extend_from_slice(&self.file.match_pattern_source(s.unescaped()));
        } else if let Some(s) = node.as_match_last_line_node() {
            self.cooked
                .extend_from_slice(&self.file.match_pattern_source(s.unescaped()));
        } else if let Some(s) = node.as_x_string_node() {
            self.literal(
                s.content_loc().as_slice(),
                s.unescaped(),
                s.opening_loc().as_slice().ends_with(b"'"),
            );
        } else if let Some(s) = node.as_symbol_node() {
            // A static quoted symbol usually collapses to sym (no str leaf).
            // Multiple physical payload parts instead form dsym, even without
            // interpolation. Bare/word-array symbols do not take that path.
            if s.closing_loc().is_some()
                && let Some(raw) = s.value_loc()
            {
                let single = s.opening_loc().is_some_and(|o| {
                    o.as_slice().ends_with(b"'") || o.as_slice().starts_with(b"%s")
                });
                let parts = crate::cops::ast::require_literal::parts(
                    raw.as_slice(),
                    s.unescaped(),
                    single,
                    self.file,
                );
                if parts.len() > 1 {
                    self.cooked.extend(parts.into_iter().flatten());
                }
            }
        } else if node.as_source_file_node().is_some() {
            self.cooked
                .extend_from_slice(self.file.source_path().as_os_str().as_encoded_bytes());
        }
        // SourceLineNode/SourceEncodingNode are NOT str leaves. Dynamic
        // symbols' literal StringNode children are visited normally.
        self.parents.push(node);
    }
    fn leave(&mut self) {
        self.parents.pop();
    }
}

/// Prove a static, ordinary-encoding literal concatenation. No dynamic string
/// coercion, String#+, changed allocation dispatch, escaped encoding directives,
/// or removed line/comment bytes. Global source/code-object/heap/position proofs
/// are applied by run_cops. Uncertain individual edits retain their findings.
pub(super) fn static_parts(parts: &[Node<'_>], file: &SourceFile) -> bool {
    file.data_start().is_none()
        && parts.iter().all(|part| {
            part.as_string_node().is_some_and(|s| {
                s.opening_loc().is_some_and(|open| {
                    matches!(open.as_slice(), b"\"" | b"'")
                        && s.closing_loc()
                            .is_some_and(|close| close.as_slice() == open.as_slice())
                }) && !s.content_loc().as_slice().contains(&b'\\')
            })
        })
}

/// Caller proves static_parts once per concatenation, not once per pair.
pub(super) fn removal(remove: &[&Node<'_>], file: &SourceFile) -> Option<Correction> {
    if remove.is_empty()
        || remove
            .iter()
            .any(|n| !matches!(n.location().as_slice(), b"\"\"" | b"''"))
    {
        return None;
    }
    let start = remove.first()?.location().start_offset();
    let end = remove.last()?.location().end_offset();
    let mut replacement = Vec::new();
    let mut cursor = start;
    for part in remove {
        replacement.extend_from_slice(file.bytes.get(cursor..part.location().start_offset())?);
        cursor = part.location().end_offset();
    }
    Some(Correction {
        range: start..end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}
