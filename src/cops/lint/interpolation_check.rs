//! `Lint/InterpolationCheck` — interpolation-looking string literals.
//!
//! Detection only: changing quotes can execute previously inert Ruby. The fixed
//! policy permits ONLY the pinned literal's syntax-validation parse.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct InterpolationCheck;
impl Cop for InterpolationCheck {
    fn name(&self) -> &'static str {
        "Lint/InterpolationCheck"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let delimited = if let Some(s) = node.as_string_node() {
            // Other str spellings are unchanged by conversion (still str),
            // or have a percent quote delimiter replaced by a mismatched one.
            // Multiline non-single-quoted dstr callbacks are excluded upstream.
            s.opening_loc().is_some_and(|o| o.as_slice() == b"'") && s.closing_loc().is_some()
        } else if let Some(s) = node.as_interpolated_string_node() {
            // Includes multiline single-quoted dstr, but not interpolation or
            // concatenation containers (whose children are visited separately).
            s.opening_loc().is_some_and(|o| o.as_slice() == b"'") && s.closing_loc().is_some()
        } else {
            false
        };
        if !delimited || is_heredoc(node) || cx.ancestors().any(is_heredoc) {
            return;
        }
        if cx.parent().is_some_and(|p| {
            matches!(
                p,
                Node::InterpolatedRegularExpressionNode { .. }
                    | Node::InterpolatedMatchLastLineNode { .. }
            )
        }) {
            return;
        }
        // The negative lookbehind tests exactly the preceding character, NOT
        // backslash parity; dot does not cross a physical newline.
        let raw = node.location().as_slice();
        if !valid_converted_literal(raw, file) {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(), severity: self.severity(), line, col,
            end_line: file.last_line_of(node.location()),
            message: "Interpolation in single quoted string detected. Use double quoted strings if you need interpolation.".into(),
            correction: None,
        });
    }
}

fn is_heredoc(node: &Node<'_>) -> bool {
    let opening = match node {
        Node::StringNode { .. } => node.as_string_node().unwrap().opening_loc(),
        Node::InterpolatedStringNode { .. } => {
            node.as_interpolated_string_node().unwrap().opening_loc()
        }
        Node::XStringNode { .. } => Some(node.as_x_string_node().unwrap().opening_loc()),
        Node::InterpolatedXStringNode { .. } => {
            Some(node.as_interpolated_x_string_node().unwrap().opening_loc())
        }
        _ => None,
    };
    opening.is_some_and(|o| o.as_slice().starts_with(b"<<"))
}

fn looks_like_interpolation(raw: &[u8]) -> bool {
    raw.split(|&b| b == b'\n').any(|line| {
        let Some(end) = line.iter().rposition(|&b| b == b'}') else {
            return false;
        };
        line[..end]
            .windows(2)
            .enumerate()
            .any(|(i, pair)| pair == b"#{" && (i == 0 || line[i - 1] != b'\\'))
    })
}

fn valid_converted_literal(raw: &[u8], file: &SourceFile) -> bool {
    // Pinned node.source is the source buffer's encoding-aware projection, not
    // a lossy UTF-8 conversion. Reuse its existing codec/normalization tables.
    let projected = file.match_pattern_source(raw);
    // Parser::Source::Buffer#raw_source= performs one gsub("\r\n", "\n")
    // on the ORIGINAL source. Do not collapse a CR chain to a fixed point.
    let raw = reference_buffer_source(&projected);
    // Test characters AFTER projection: Shift_JIS's 表 ends in byte 0x5c,
    // which is not a backslash character before a following interpolation.
    if !looks_like_interpolation(&raw) {
        return false;
    }
    let percent = raw.contains(&b'"');
    let mut converted = Vec::with_capacity(raw.len() + 1);
    if raw.first() == Some(&b'\'') {
        converted.extend_from_slice(if percent { b"%{" } else { b"\"" });
    } else {
        converted.push(raw[0]);
    }
    converted.extend_from_slice(&raw[1..raw.len() - 1]);
    if raw.last() == Some(&b'\'') {
        converted.push(if percent { b'}' } else { b'"' });
    } else {
        converted.push(raw[raw.len() - 1]);
    }
    // Like Base#parse, this literal gets no original-file encoding header:
    // the buffer projected legacy encodings above; non-UTF8 binary payloads
    // can therefore fail converted syntax even when the main source is valid.
    // Fixed Ruby3.4, dedicated C stack, recursion/AST-depth guards and bounded
    // destruction are the SAME entrypoint as the main parse. No evaluation,
    // candidate cops, corrections, public knobs or arbitrary size cutoffs.
    // Base#parse creates a SECOND source buffer for the converted literal.
    // CRCRLF therefore loses one CR in each buffer. Only source bytes undergo
    // this substitution, never Prism's decoded/escape-produced payload.
    let converted = reference_buffer_source(&converted);
    let parsed = crate::parser::parse(&converted);
    // Prism::Translation::Parser uses partial_script:true. In pinned Prism1.9
    // that ONLY suppresses parse_yield and parse_block_exit (break/next/redo),
    // not retry or grammar errors. Match those four diagnostics locally rather
    // than weakening the main parser or exposing another parser configuration.
    if parsed.depth_error().is_some()
        || parsed.errors().any(|e| {
            !matches!(
                e.message().as_ref(),
                "Invalid yield" | "Invalid break" | "Invalid next" | "Invalid redo"
            )
        })
    {
        return false;
    }
    let program = parsed.node().as_program_node().unwrap();
    let mut body = program.statements().body().iter();
    let Some(root) = body.next() else {
        return false;
    };
    if body.next().is_some() {
        return false;
    }
    if root.as_interpolated_string_node().is_some() {
        return true;
    }
    let Some(string) = root.as_string_node() else {
        return false;
    };
    // The translation adapter builds zero-child dstr for empty percent strings,
    // and composes zero/multiple static parts into dstr as well. A converted
    // percent literal can close BEFORE the apparent interpolation, leaving it
    // in a Ruby comment: require the reference AST type, not active interpolation.
    let content = string.content_loc();
    if string.unescaped().is_empty() {
        return content.as_slice().contains(&b'\n')
            || string
                .opening_loc()
                .is_some_and(|o| o.as_slice().starts_with(b"%"));
    }
    if !content.as_slice().contains(&b'\n') {
        return false;
    }
    static_multiline_is_dstr(content.as_slice(), string.unescaped())
}

/// One nonrecursive source-buffer CRLF substitution, local to this cop's two
/// reference buffers. Bare CR and all escape-produced bytes stay unchanged.
fn reference_buffer_source(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !bytes.windows(2).any(|pair| pair == b"\r\n") {
        return std::borrow::Cow::Borrowed(bytes);
    }
    std::borrow::Cow::Owned(
        bytes
            .iter()
            .enumerate()
            .filter_map(|(i, &byte)| {
                (byte != b'\r' || bytes.get(i + 1) != Some(&b'\n')).then_some(byte)
            })
            .collect(),
    )
}

/// Type-only port of pinned Prism1.9's
/// `Compiler#string_nodes_from_line_continuations` for interpolating strings
/// (MIT License, Copyright 2022-present Shopify Inc.).
/// The adapter indexes decoded `.lines` using boundaries derived from physical
/// lines and literal `\\n` escapes, NOT from decoded physical-source parts. This
/// intentionally includes its odd interactions with Unicode/control newlines
/// and continuations. Zero or multiple emitted str children compose to dstr.
/// Keep this local: other literal consumers need their existing payload mapping.
fn static_multiline_is_dstr(raw: &[u8], decoded: &[u8]) -> bool {
    // Ruby String#lines omits a trailing empty line and returns [] for "".
    let decoded_lines = decoded.iter().filter(|&&b| b == b'\n').count()
        + usize::from(!decoded.is_empty() && decoded.last() != Some(&b'\n'));
    let mut next_boundary = 0;
    let mut escaped_newlines = 0;
    let mut emitted = 0;
    let mut lines = raw.split_inclusive(|&b| b == b'\n').peekable();
    while let Some(line) = lines.next() {
        // Equivalent to scan(/(\\*)n/), counting only odd backslash runs.
        let mut slashes = 0;
        let mut count = 0;
        for &byte in line {
            if byte == b'\\' {
                slashes += 1;
            } else {
                count += usize::from(byte == b'n' && slashes % 2 == 1);
                slashes = 0;
            }
        }
        if !line.ends_with(b"\n") {
            count = count.saturating_sub(1);
        }
        escaped_newlines += count;
        let before_newline = line.strip_suffix(b"\n").unwrap_or(line);
        let before_newline = before_newline.strip_suffix(b"\r").unwrap_or(before_newline);
        let continuation = line.ends_with(b"\n")
            && before_newline
                .iter()
                .rev()
                .take_while(|&&b| b == b'\\')
                .count()
                % 2
                == 1;
        if continuation && lines.peek().is_some() {
            continue;
        }
        next_boundary += escaped_newlines + 1;
        escaped_newlines = 0;
        if next_boundary <= decoded_lines {
            emitted += 1;
        } else {
            // Later boundaries only increase; the adapter never flushes a
            // decoded tail that has no corresponding do_next_tokens entry.
            break;
        }
    }
    emitted != 1
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/lint/interpolation_check_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/InterpolationCheck")
            .collect()
    }

    #[test]
    fn flags_single_quotes_split_and_multiline_at_literal_start() {
        for (source, line, col, end_line) in [
            ("'foo #{bar}'", 1, 1, 1),
            ("'foo #{bar} }'", 1, 1, 1),
            ("'foo \"#{bar}\"'", 1, 1, 1),
            ("'x' \\\n  'foo #{bar}'", 2, 3, 2),
            ("\"x\" \\\n  'foo #{bar}'", 2, 3, 2),
            (
                "foo = 'something with #{interpolation}\nspanning lines'",
                1,
                7,
                2,
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                "Interpolation in single quoted string detected. Use double quoted strings if you need interpolation."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn ignores_all_pinned_lookalikes() {
        for source in [
            "hello = \"foo #{bar}\"",
            "foo = \"bar '#{baz}' qux\"",
            r"/\#{20}/",
            r#""\#{msg}""#,
            r#"foo = "\xff""#,
            r#"foo = "alpha #{variable} beta \#{gamma}\" delta""#,
            r##"%w("#{a}-foo")"##,
            "'#{%<expression>s}'",
            "'Text `A(\"#{%<base>s}/%<path>s\")` and `B` with C.'",
            "'a \"b\" } #{c}'",
            "foo = <<~TEXT\n  something with #{interpolation}\nTEXT\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn recognizes_empty_percent_dstr_before_a_converted_comment() {
        // Pinned conversion yields %{}#{v}"}: the remainder is a Ruby comment,
        // and the empty percent literal is a zero-child Parser dstr, not str.
        assert_eq!(offenses("'}#{v}\"'").len(), 1);
        assert_eq!(offenses("'}#{v + }\"'").len(), 1);
        assert_eq!(offenses("'a\nb}#{v}\"'").len(), 1);
        assert_eq!(offenses("'\\\n}#{v}\"'").len(), 1);
        assert!(offenses("'a\n}#{v}\"'").is_empty());
        assert!(offenses("'a\\\n}#{v}\"'").is_empty());
    }

    #[test]
    fn static_multiline_escaped_newlines_can_emit_zero_or_multiple_parts() {
        for content in [
            "\\n\\\n",
            "\\n\\\r\n",
            "\\u{a}a\n\\\n",
            "\\u{a a}a\n\\\n",
            "\\cJa\n\\\n",
        ] {
            let source = format!("'{content}}}#{{v}}\"'");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(found[0].end_line, source.lines().count() as u32);
            assert_eq!(
                found[0].message,
                "Interpolation in single quoted string detected. Use double quoted strings if you need interpolation."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn static_multiline_continuation_can_emit_only_one_part() {
        for content in ["a\n\\n\\\n", "a\n\\n\\\r\n"] {
            let source = format!("'{content}}}#{{v}}\"'");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn repeated_crlf_original_and_converted_buffers_emit_zero_or_one_child() {
        // Exact source IDs 694/695/725/726. Both source buffers perform
        // ONE nonrecursive CRLF -> LF substitution, before Prism decodes text.
        // These are physical CRCRLF bytes, not Ruby escape-produced newlines.
        for source in ["'\\\r\r\n}#{v}\"'\n", "'\\n\\\r\r\n}#{v}\"'\n"] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 2));
            assert_eq!(
                found[0].message,
                "Interpolation in single quoted string detected. Use double quoted strings if you need interpolation."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn repeated_crlf_prefix_can_leave_only_one_emitted_child() {
        for source in [
            "'a\r\r\n\\\r\r\n}#{v}\"'\n",
            "'a\r\r\n\\n\\\r\r\n}#{v}\"'\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn each_buffer_pass_removes_one_physical_cr_only() {
        for (raw, once, twice) in [
            ("\r", "\r", "\r"),
            ("\n", "\n", "\n"),
            ("\r\n", "\n", "\n"),
            ("\r\r\n", "\r\n", "\n"),
            ("\r\r\r\n", "\r\r\n", "\r\n"),
            (
                "\\r\\n\\u{d a}\\cM\\cJ",
                "\\r\\n\\u{d a}\\cM\\cJ",
                "\\r\\n\\u{d a}\\cM\\cJ",
            ),
        ] {
            let projected = super::reference_buffer_source(raw.as_bytes());
            assert_eq!(projected, once.as_bytes());
            assert_eq!(super::reference_buffer_source(&projected), twice.as_bytes());
        }
    }

    #[test]
    fn respects_reference_partial_script_but_not_retry_or_grammar_errors() {
        for body in [
            "yield", "break", "next", "redo", "yield 1", "break 1", "next 1",
        ] {
            assert_eq!(offenses(&format!("'#{{{body}}}'")).len(), 1, "{body}");
        }
        for body in ["retry", "yield(", "break +", "redo 1", "class", "one +"] {
            assert!(offenses(&format!("'#{{{body}}}'")).is_empty(), "{body}");
        }
        // This rule's validation-only partial-script compatibility never
        // changes the ordinary main-source parser gate.
        assert!(
            crate::parser::parse(b"\"#{yield}\"")
                .errors()
                .next()
                .is_some()
        );
    }

    #[test]
    fn long_and_many_literals_have_no_arbitrary_detection_cutoff() {
        let long = format!("'{}#{{value}}'", "text ".repeat(50_000));
        assert_eq!(offenses(&long).len(), 1);
        assert_eq!(offenses(&"'#{value}'\n".repeat(200)).len(), 200);
    }

    #[test]
    fn converted_literal_resource_rejection_does_not_recurse_or_execute() {
        let source = format!("'#{{{}1{}}}'", "[".repeat(1500), "]".repeat(1500));
        // The original is a shallow inert string; only its validation tree
        // exceeds the existing AST budget. Rejection uses guarded destruction.
        assert!(
            crate::parser::parse(source.as_bytes())
                .depth_error()
                .is_none()
        );
        assert!(offenses(&source).is_empty());
    }

    #[test]
    fn validates_converted_ruby_grammar_without_executing_it() {
        for source in [
            "'#{one + two}'",
            "'#{`never execute`}'",
            "'#{exit!}'",
            "'#{class C; end}'",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "'#{one + }'",
            "'#{class}'",
            "'#{foo(}'",
            "'#{a\nb}'",
            "'\\#{ignored}'",
            "%q(#{value})",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
