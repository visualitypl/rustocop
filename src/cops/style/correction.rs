//! Atomic corrections assembled from several edits to one source snapshot.

use crate::{offense::Correction, source::SourceFile};

/// Comment rewrites must not activate, deactivate or change a magic header.
/// Conservatively protect these spellings even outside Ruby's header positions.
pub(crate) fn contains_magic_comment_key(bytes: &[u8]) -> bool {
    // Prism/Ruby normalize ASCII case and hyphens in magic-comment keys.
    static MAGIC: std::sync::LazyLock<regex::bytes::Regex> = std::sync::LazyLock::new(|| {
        regex::bytes::Regex::new(concat!(
            r"(?i-u:coding|encoding|frozen[_-]string[_-]literal|",
            r"warn[_-]indent|shareable[_-]constant[_-]value)",
        ))
        .expect("fixed magic-comment keys")
    });
    MAGIC.is_match(bytes)
}

/// Ordered main-parse comment ranges allow bounded local span queries.
pub(super) fn contains_comment(file: &SourceFile, range: std::ops::Range<usize>) -> bool {
    let comments = file.comments();
    let first = comments.partition_point(|c| c.end <= range.start);
    comments.get(first).is_some_and(|c| c.start < range.end)
}

/// A trailing comment can become an active Ruby header when moved above a
/// declaration. Protect shebangs and magic spellings at every destination,
/// including indented/noninitial lines, rather than guessing header activation.
/// Call only on a main-parse comment slice, never unrelated source/literal text.
pub(super) fn comment_relocation_safe(bytes: &[u8]) -> bool {
    !bytes.starts_with(b"#!") && !contains_magic_comment_key(bytes)
}

/// Shared correction-only proof for method newline edits. Main-parse facts and
/// the hardened observer/DATA index are lazy and cached once per source file.
pub(super) fn method_newline_safe(
    node: &ruby_prism::Node<'_>,
    cx: &crate::cops::NodeContext<'_, '_>,
    file: &SourceFile,
    range: std::ops::Range<usize>,
) -> bool {
    let Some(root) = cx.ancestors().next_back() else {
        return false;
    };
    !(range.start < file.line_value_boundary()
        || file.heredoc_pending_at(root, range.start)
        || cx.ancestors().any(|n| n.as_def_node().is_some())
        || file.contains_heredoc(root, node))
}

/// First offset after the last textual `__LINE__` start. Compute once when
/// checking many newline deletions, rather than repeatedly scanning the suffix.
pub(crate) fn line_value_boundary(file: &SourceFile) -> usize {
    file.line_value_boundary()
}

/// A cop offense holds one contiguous edit. Include untouched interior bytes
/// when a syntax change requires several coordinated edits, so overlaps and
/// disabled lines defer the *whole* correction rather than half of it.
pub(crate) fn combine(file: &SourceFile, mut edits: Vec<Correction>) -> Option<Correction> {
    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    let start = edits.first()?.range.start;
    let end = edits.iter().map(|edit| edit.range.end).max()?;
    let mut replacement = Vec::new();
    let mut cursor = start;
    for edit in edits {
        if edit.range.start < cursor || edit.range.end > file.bytes.len() {
            return None;
        }
        replacement.extend_from_slice(&file.bytes[cursor..edit.range.start]);
        replacement.extend_from_slice(edit.replacement.as_bytes());
        cursor = edit.range.end;
    }
    replacement.extend_from_slice(&file.bytes[cursor..end]);
    Some(Correction {
        range: start..end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

/// RuboCop's `range_by_whole_lines(range, include_final_newline: true)`.
pub(crate) fn whole_lines(file: &SourceFile, r: std::ops::Range<usize>) -> std::ops::Range<usize> {
    file.line_start(file.line_of(r.start))
        ..file.line_start(file.line_of(r.end.saturating_sub(1).max(r.start)) + 1)
}

/// RuboCop's `Util#to_string_literal`: single quotes, unless the value
/// needs an escape that only double quotes can spell (`inspect` then).
pub(crate) fn to_string_literal(value: &[u8], file: &SourceFile) -> Option<String> {
    let mut inspected = Vec::new();
    crate::cops::lint::duplicate_match_pattern::inspect_source(value, file, &mut inspected);
    let escaped = replace(&inspected[1..inspected.len() - 1], b"\\\"", b"\"");
    let text = if double_quotes_required(&escaped) && std::str::from_utf8(value).is_ok() {
        inspected
    } else {
        let mut text = vec![b'\''];
        text.extend(replace(&replace(value, b"\\", b"\\\\"), b"\\\"", b"\""));
        text.push(b'\'');
        text
    };
    String::from_utf8(text).ok()
}

/// RuboCop's `trim_string_interpolation_escape_character`: `\#{x}` becomes
/// `#{x}`.
pub(crate) fn trim_interpolation_escape(text: &str) -> String {
    use std::sync::LazyLock;
    static ESCAPED: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\\#\{(.*?)\}").unwrap());
    ESCAPED.replace_all(text, "#{$1}").into_owned()
}

/// RuboCop's `double_quotes_required?`: a `'`, or a backslash escape other
/// than `\\` and `\"`.
fn double_quotes_required(text: &[u8]) -> bool {
    if text.contains(&b'\'') {
        return true;
    }
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'\\' {
            let run = text[i..].iter().take_while(|&&b| b == b'\\').count();
            if run % 2 == 1 && text.get(i + run) != Some(&b'"') {
                return true;
            }
            i += run;
        } else {
            i += 1;
        }
    }
    false
}

fn replace(text: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(text[i]);
            i += 1;
        }
    }
    out
}

/// RuboCop's `build_bracketed_array_with_appropriate_whitespace`: the
/// whitespace before the first element, between the first two (used for
/// every separator) and after the last.
pub(crate) fn bracketed_array(
    array: &ruby_prism::ArrayNode<'_>,
    words: &[String],
    file: &SourceFile,
) -> Option<String> {
    let elements: Vec<_> = array.elements().iter().collect();
    let (Some(first), Some(last)) = (elements.first(), elements.last()) else {
        return Some("[]".into());
    };
    let text = |range: std::ops::Range<usize>| std::str::from_utf8(&file.bytes[range]).ok();
    let leading = text(array.opening_loc()?.end_offset()..first.location().start_offset())?;
    let between = match elements.get(1) {
        Some(second) => text(first.location().end_offset()..second.location().start_offset())?,
        None => " ",
    };
    let trailing = text(last.location().end_offset()..array.closing_loc()?.start_offset())?;
    Some(format!(
        "[{leading}{}{trailing}]",
        words.join(&format!(",{between}"))
    ))
}

/// RuboCop AST's value_used? adapted to Prism's transparent statement lists.
/// Unknown contexts conservatively consume the value. Ancestor depth is bounded
/// by the pipeline; no descendant scans or reparsing are needed per offense.
pub(crate) fn value_used<'a, 'pr: 'a>(
    node: &ruby_prism::Node<'pr>,
    ancestors: impl IntoIterator<Item = &'a ruby_prism::Node<'pr>>,
) -> bool {
    let span = |n: &ruby_prism::Node<'_>| (n.location().start_offset(), n.location().end_offset());
    let mut child = span(node);
    for parent in ancestors {
        if let Some(statements) = parent.as_statements_node() {
            if !statements
                .body()
                .last()
                .is_some_and(|last| span(&last) == child)
            {
                return false;
            }
        } else if parent.as_program_node().is_some() {
            return false;
        } else if let Some(n) = parent.as_if_node() {
            if span(&n.predicate()) == child {
                return true;
            }
        } else if let Some(n) = parent.as_unless_node() {
            if span(&n.predicate()) == child {
                return true;
            }
        } else if let Some(n) = parent.as_while_node() {
            return span(&n.predicate()) == child;
        } else if let Some(n) = parent.as_until_node() {
            return span(&n.predicate()) == child;
        } else if let Some(n) = parent.as_for_node() {
            if span(&n.collection()) == child {
                return true;
            }
        } else if let Some(n) = parent.as_case_node() {
            if n.predicate().is_some_and(|p| span(&p) == child) {
                return true;
            }
        } else if let Some(n) = parent.as_begin_node() {
            if n.rescue_clause().is_some() || n.ensure_clause().is_some() {
                return true;
            }
        } else if !matches!(
            parent,
            ruby_prism::Node::ParenthesesNode { .. }
                | ruby_prism::Node::EmbeddedStatementsNode { .. }
                | ruby_prism::Node::ElseNode { .. }
                | ruby_prism::Node::WhenNode { .. }
                | ruby_prism::Node::ArrayNode { .. }
                | ruby_prism::Node::HashNode { .. }
                | ruby_prism::Node::KeywordHashNode { .. }
                | ruby_prism::Node::AssocNode { .. }
                | ruby_prism::Node::DefinedNode { .. }
                | ruby_prism::Node::InterpolatedStringNode { .. }
                | ruby_prism::Node::InterpolatedSymbolNode { .. }
                | ruby_prism::Node::InterpolatedRegularExpressionNode { .. }
                | ruby_prism::Node::InterpolatedXStringNode { .. }
                | ruby_prism::Node::RangeNode { .. }
                | ruby_prism::Node::FlipFlopNode { .. }
        ) {
            return true;
        }
        child = span(parent);
    }
    false
}
