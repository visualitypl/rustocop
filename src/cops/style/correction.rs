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

/// A newline-changing edit must not turn a standalone directive into a trailing
/// one (or vice versa), or move code onto/off an inline directive's line. Check
/// whole touched lines, including comments just outside the replacement range.
pub(crate) fn touches_directive(file: &SourceFile, range: std::ops::Range<usize>) -> bool {
    let start = file.line_start(file.line_of(range.start));
    let end = file.line_start(file.line_of(range.end.saturating_sub(1).max(range.start)) + 1);
    let comments = file.directive_comments();
    let first = comments.partition_point(|comment| comment.end <= start);
    comments
        .get(first)
        .is_some_and(|comment| comment.start < end)
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
    if range.start < file.line_value_boundary()
        || file.heredoc_pending_at(root, range.start)
        || touches_directive(file, range)
        || cx.ancestors().any(|n| n.as_def_node().is_some())
        || file.contains_heredoc(root, node)
    {
        return false;
    }
    let proof = file.class_method_bindings(root);
    !proof.observes_sources && !proof.observes_data
}

/// Conservatively protect executable `__LINE__` values beyond a newline-changing
/// edit, including outside a cop's local reparse scope. Textual matching also
/// withholds a few harmless fixes when the spelling occurs in a comment/literal.
/// Local semantic verification remains responsible for values inside the edit.
pub(super) fn preserves_line_values(file: &SourceFile, correction: &Correction) -> bool {
    let old = &file.bytes[correction.range.clone()];
    memchr::memchr_iter(b'\n', old).count()
        == memchr::memchr_iter(b'\n', correction.replacement.as_bytes()).count()
        || correction.range.start >= line_value_boundary(file)
}

/// DATA's initial byte position changes even when all physical lines survive.
/// Equal-byte edits preserve DATA offsets, but still require the main-script
/// source-read guard. Reuse the one lazy complete main-AST
/// observer index. Any DATA acquisition is a conservative barrier, regardless of
/// selector/alias/dispatch, without per-candidate scans, reparses or value flow.
/// This is correction-only: callers keep the offense and all other safety guards.
pub(super) fn preserves_data_positions(
    file: &SourceFile,
    cx: &crate::cops::NodeContext<'_, '_>,
    correction: &Correction,
) -> bool {
    cx.ancestors().next_back().is_some_and(|root| {
        let proof = file.class_method_bindings(root);
        !proof.observes_program_name
            && (correction.range.len() == correction.replacement.len() || !proof.observes_data)
    })
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
