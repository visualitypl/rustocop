//! `Lint/LambdaWithoutLiteralBlock` — lambda calls without a literal block.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct LambdaWithoutLiteralBlock;
impl Cop for LambdaWithoutLiteralBlock {
    fn name(&self) -> &'static str {
        "Lint/LambdaWithoutLiteralBlock"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"lambda"
            || call.is_safe_navigation()
            || ordinary_block_parent(&call, cx)
        {
            return;
        }
        let args = crate::cops::ast::call_arguments(&call);
        if args.is_empty() {
            return;
        }
        if call.receiver().is_none()
            && args.len() == 1
            && args[0]
                .as_block_argument_node()
                .and_then(|b| b.expression())
                .is_some_and(|e| reference_symbol(&e, file))
        {
            return;
        }
        let start = node.location().start_offset();
        let end = crate::cops::ast::source_end(node);
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message:
                "lambda without a literal block is deprecated; use the proc without lambda instead."
                    .into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction: cx
                .corrections_requested()
                .then(|| inactive_first_argument(node, cx, file))
                .flatten(),
        });
    }
}

// The pinned matcher needs sym, not dsym. Port only the static SymbolNode
// token-emission projection from Prism 1.9's visit_symbol_node and
// string_nodes_from_line_continuations (MIT, Shopify Inc.). Parser's builder
// collapses exactly ONE emitted str token, even if its payload is empty.
fn reference_symbol(node: &Node<'_>, file: &SourceFile) -> bool {
    let Some(symbol) = node.as_symbol_node() else {
        return false;
    };
    if symbol.closing_loc().is_none() {
        return true;
    }
    let raw = symbol.value_loc();
    let raw = raw.as_ref().map_or(b"".as_slice(), |loc| loc.as_slice());
    // Project source glyphs BEFORE looking for ASCII escapes (UTF8-MAC can
    // compose n + tilde into ñ; legacy trail bytes need not be backslashes).
    let escaped = file.match_pattern_source(raw);
    if escaped.is_empty() {
        return false;
    }
    if !escaped.contains(&b'\n') {
        return true;
    }
    // Parser::Source::Buffer removes exactly one CR from every CRLF before
    // Prism sees the source. A remaining CR still participates in grouping.
    let escaped: Vec<_> = escaped
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| (b != b'\r' || escaped.get(i + 1) != Some(&b'\n')).then_some(b))
        .collect();
    let single = symbol
        .opening_loc()
        .is_some_and(|loc| loc.as_slice().ends_with(b"'") || loc.as_slice().starts_with(b"%s"));
    if single {
        // Noninterpolating symbols emit once per escaped.lines entry, NOT per
        // decoded line, and never glue backslash continuations.
        return escaped.split_inclusive(|&b| b == b'\n').count() == 1;
    }
    // Reuse ONLY payload projection/alignment: preserve main-parse decoded
    // escapes, projecting source glyphs rather than normalizing escape results.
    // Its part boundaries are deliberately irrelevant to emitted token count.
    let mut payload =
        crate::cops::ast::require_literal::parts(raw, symbol.unescaped(), false, file);
    project_buffer_continuations(raw, &mut payload, file);
    let cooked_lines = ruby_line_count(payload.iter().flatten().copied());
    symbol_string_tokens(&escaped, cooked_lines) == 1
}

// Buffer normalization followed by Prism's CRLF handling normally removes only
// CR bytes, which cannot change String#lines cardinality. The exception is a
// direct \\ CR CR LF: it becomes a continuation, removing its cooked CR LF.
// Align those deletions to the existing helper's physical payload boundaries,
// NOT its token count. Skip whole escaped characters/control operands so a
// legacy trail backslash or \\c\\ CR CR LF cannot masquerade as a continuation.
// No escape values are decoded here; all other cooked bytes stay main-parse bytes.
pub(super) fn project_buffer_continuations(raw: &[u8], payload: &mut [Vec<u8>], file: &SourceFile) {
    if !raw.windows(4).any(|s| s == b"\\\r\r\n") {
        return;
    }
    let (mut r, mut part) = (0, 0);
    while r < raw.len() {
        if raw[r] == b'\\' {
            if raw[r..].starts_with(b"\\\r\r\n")
                && let Some(bytes) = payload.get_mut(part)
                && bytes.ends_with(b"\r\n")
            {
                bytes.truncate(bytes.len() - 2);
            }
            r += 1;
            // Control/meta prefixes consume their operand as part of the same
            // escape. A controlled physical LF is not a payload-part boundary.
            loop {
                let prefix = match raw.get(r..) {
                    Some([b'c', ..]) => 1,
                    Some([b'C' | b'M', b'-', ..]) => 2,
                    _ => break,
                };
                r += prefix;
                if raw.get(r) != Some(&b'\\') {
                    break;
                }
                r += 1;
            }
            r += if raw.get(r..).is_some_and(|s| s.starts_with(b"\r\n")) {
                2
            } else {
                file.encoded_character_width(&raw[r.min(raw.len())..])
                    .max(1)
            };
        } else if raw[r..].starts_with(b"\r\n") || raw[r] == b'\n' {
            r += if raw[r] == b'\r' { 2 } else { 1 };
            part += 1;
        } else {
            r += file.encoded_character_width(&raw[r..]).max(1);
        }
    }
}

// Ruby String#lines: no entry for empty strings or after a terminal LF.
fn ruby_line_count(bytes: impl Iterator<Item = u8>) -> usize {
    let mut count = 0;
    let mut last = None;
    for byte in bytes {
        count += usize::from(byte == b'\n');
        last = Some(byte);
    }
    count + usize::from(last.is_some_and(|b| b != b'\n'))
}

// The buffer-projected source may still contain CRLF (original CR CR LF).
// Strip its optional CR exactly like the pinned /\\r?\\n$/ grouping.
fn symbol_string_tokens(escaped: &[u8], cooked_lines: usize) -> usize {
    let mut slots = 0;
    let mut group_slots = 1;
    let mut emitted = 0;
    let mut lines = escaped.split_inclusive(|&b| b == b'\n').peekable();
    while let Some(line) = lines.next() {
        // Exactly Ruby scan(/(\\*)n/): count odd runs before ASCII n only,
        // including runs in longer escape spellings, not Unicode/hex/octal LF.
        let mut backslashes = 0;
        let mut escaped_newlines = 0;
        for &byte in line {
            if byte == b'\\' {
                backslashes += 1;
            } else {
                escaped_newlines += usize::from(byte == b'n' && backslashes % 2 == 1);
                backslashes = 0;
            }
        }
        if !line.ends_with(b"\n") && escaped_newlines > 0 {
            escaped_newlines -= 1;
        }
        group_slots += escaped_newlines;
        let before_lf = line.strip_suffix(b"\n").unwrap_or(line);
        let before_lf = before_lf.strip_suffix(b"\r").unwrap_or(before_lf);
        let continuation = line.ends_with(b"\n")
            && before_lf.iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1;
        if !continuation || lines.peek().is_none() {
            // normalized_lengths/do_next_tokens contain group_slots slots with
            // only the LAST flagged. The cooked iterator can stop before that
            // flag; leftover nonempty payload then emits NO token.
            slots += group_slots;
            emitted += usize::from(slots <= cooked_lines);
            group_slots = 1;
        }
    }
    emitted
}

// Prism nests a literal block under its call; RuboCop wraps the send in
// block/numblock/itblock. ONLY ordinary block parents are exempt, including
// a sole ungrouped body expression (but not an implicit multi-statement begin).
fn ordinary_block_parent(call: &ruby_prism::CallNode<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let ordinary = |params: Option<Node<'_>>| {
        params.is_none_or(|p| {
            p.as_numbered_parameters_node().is_none() && p.as_it_parameters_node().is_none()
        })
    };
    if let Some(block) = call.block().and_then(|b| b.as_block_node()) {
        return ordinary(block.parameters());
    }
    let mut ancestors = cx.ancestors();
    let Some(body) = ancestors.next().and_then(|n| n.as_statements_node()) else {
        return false;
    };
    if body.body().len() != 1 {
        return false;
    }
    let Some(parent) = ancestors.next() else {
        return false;
    };
    if let Some(block) = parent.as_block_node() {
        return ordinary(block.parameters());
    }
    parent
        .as_lambda_node()
        .is_some_and(|lambda| ordinary(lambda.parameters()))
}

// Dropping lambda dispatch, to_proc conversion or extra arguments is not safe
// in reachable code. Even a builtin call can change warnings, allocation and
// proc identity/lambda-ness. Prove only one immediate dead expression in an
// otherwise closed top-level false branch. The sole positional argument is
// a bare send: no bindings, definitions, literals, parameters or compiler effects.
// Unknown block passes remain withheld by the shared selector-acquisition guards;
// literal procs remain withheld because their block positions change. Exact
// spelling excludes comments, heredocs and line movement. Shared main-parse
// source/encoding/position/code-object/heap guards remain authoritative.
fn inactive_first_argument(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let call = node.as_call_node()?;
    if call.receiver().is_some() || call.block().is_some() {
        return None;
    }
    let arguments = call.arguments()?.arguments();
    if arguments.len() != 1 {
        return None;
    }
    let expression = arguments.iter().next()?;
    let bare = expression.as_call_node()?;
    let name = bare.name().as_slice();
    if bare.receiver().is_some()
        || bare.arguments().is_some()
        || bare.block().is_some()
        || name.is_empty()
        || !name.iter().all(|b| b.is_ascii_alphabetic() || *b == b'_')
        || expression.location().as_slice() != name
    {
        return None;
    }
    let replacement = std::str::from_utf8(name).ok()?.to_owned();
    if node.location().as_slice() != format!("lambda({replacement})").as_bytes() {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let body = ancestors.next()?.as_statements_node()?;
    let conditional = ancestors.next()?.as_if_node()?;
    let top = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some()
        || body.body().len() != 1
        || top.body().len() != 1
        || conditional.predicate().as_false_node().is_none()
        || conditional.subsequent().is_some()
        || conditional.end_keyword_loc().is_none()
        || conditional.if_keyword_loc()?.as_slice() != b"if"
    {
        return None;
    }
    Some(crate::offense::Correction {
        range: node.location().start_offset()..node.location().end_offset(),
        replacement,
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/lambda_without_literal_block_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/LambdaWithoutLiteralBlock")
            .collect()
    }
    #[test]
    fn pinned_offenses() {
        for source in [
            "lambda(&proc { do_something })",
            "lambda(&Proc.new { do_something })",
            "lambda(&pr)",
            "lambda(&obj&.foo)",
        ] {
            let found = offenses(&format!("nil\n  {source}\n"));
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                "lambda without a literal block is deprecated; use the proc without lambda instead."
            );
        }
    }
    #[test]
    fn canonical_first_argument_rewrite_in_a_closed_inactive_envelope() {
        crate::cops::testing::assert_correction(
            &super::LambdaWithoutLiteralBlock,
            "if false\n  lambda(unreachable)\nend\n",
            "if false\n  unreachable\nend\n",
        );
    }
    #[test]
    fn complete_original_byte_reference_matrix() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/lambda_without_literal_block.json"
        ))
        .unwrap();
        let mismatches = std::sync::Mutex::new(vec![]);
        parallel_rows!(for row in rows => {
            let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
            let Ok(source) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let mut got: Vec<_> = offenses(source)
                .into_iter()
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            got.sort();
            let want: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            if got != want {
                mismatches.lock().unwrap().push(format!("{source:?}: {got:?} != {want:?}"));
            }
        });
        let mismatches = mismatches.into_inner().unwrap();
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }
    #[test]
    fn empty_decoded_symbol_tokens_with_physical_continuations() {
        for newline in ["\n", "\r\n"] {
            for payload in [
                format!("\\u{{}}\\{newline}"),
                format!("\\{newline}\\u{{}}"),
                format!("\\u{{}}\\{newline}\\u{{ }}"),
            ] {
                let source = format!("lambda(&:\"{payload}\")");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source:?}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 2));
                assert_eq!(
                    found[0].message,
                    "lambda without a literal block is deprecated; use the proc without lambda instead."
                );
                for nonempty in [format!("a{payload}"), format!("{payload}a")] {
                    let source = format!("lambda(&:\"{nonempty}\")");
                    assert!(offenses(&source).is_empty(), "{source:?}");
                }
            }
        }
    }
    #[test]
    fn composed_newlines_count_emitted_tokens_not_payload_parts() {
        for nl in ["\n", "\r\n"] {
            for prefix in ["", "a"] {
                for (payload, count) in [
                    (format!("{prefix}\\n\\{nl}"), 1),
                    (format!("{prefix}{nl}\\n\\{nl}"), 0),
                    (format!("{prefix}\\u000a\\{nl}"), 0),
                    (format!("{prefix}\\x0a\\{nl}"), 0),
                    (format!("{prefix}\\012\\{nl}"), 0),
                    (format!("{prefix}\\n\\{nl}a"), 0),
                    (format!("{prefix}\\n\\{nl}\\u{{}}"), 1),
                ] {
                    let source = format!("lambda(&:\"{payload}\")");
                    let found = offenses(&source);
                    assert_eq!(found.len(), count, "{source:?}");
                    if count == 1 {
                        assert_eq!((found[0].line, found[0].col), (1, 1));
                    }
                }
            }
        }
    }
    #[test]
    fn buffer_crlf_projection_precedes_symbol_parsing() {
        for (payload, count) in [("\\\r\r\n", 1), ("\\\r\r\nn", 0)] {
            let source = format!("lambda(&:\"{payload}\")\n");
            let found = offenses(&source);
            assert_eq!(found.len(), count, "{source:?}");
            if count == 1 {
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 2));
                assert_eq!(
                    found[0].message,
                    "lambda without a literal block is deprecated; use the proc without lambda instead."
                );
            }
        }
    }
    #[test]
    fn pinned_lookalikes() {
        for source in [
            "lambda { do_something }",
            "lambda.call",
            "lambda(&:do_something)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
