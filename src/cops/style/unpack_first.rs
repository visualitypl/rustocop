//! `Style/UnpackFirst` — prefer the first-value String unpacking API.

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense, single_line},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};

pub struct UnpackFirst;
impl Cop for UnpackFirst {
    fn name(&self) -> &'static str {
        "Style/UnpackFirst"
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
        let Some(call) = node.as_call_node() else {
            return;
        };
        let Some((unpack, format)) = unpack_and_first(&call) else {
            return;
        };
        let Some(selector) = unpack.message_loc() else {
            return;
        };
        let start = selector.start_offset();
        let end = ast::source_end(node);
        let current = single_line(&file.bytes[start..end]);
        let correction = cx
            .corrections_requested()
            .then(|| correction(&call, &unpack, cx, file))
            .flatten();
        let format = single_line(format.location().as_slice());
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `unpack1({format})` instead of `{current}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction,
        });
    }
}

fn unpack_and_first<'pr>(call: &CallNode<'pr>) -> Option<(CallNode<'pr>, Node<'pr>)> {
    let args = ast::call_arguments(call);
    let matches = match call.name().as_slice() {
        b"first" => args.is_empty(),
        b"[]" | b"slice" | b"at" => {
            matches!(args.as_slice(), [n] if n.as_integer_node().is_some_and(|i| i.value().to_u32_digits().1.iter().all(|d| *d == 0)))
        }
        _ => false,
    };
    if !matches {
        return None;
    }
    let unpack = call.receiver()?.as_call_node()?;
    if unpack.name().as_slice() != b"unpack"
        || unpack.receiver().is_none()
        || unpack
            .block()
            .is_some_and(|block| block.as_block_argument_node().is_none())
    {
        return None;
    }
    let mut args = ast::call_arguments(&unpack);
    if args.len() != 1 {
        return None;
    }
    Some((unpack, args.pop()?))
}

/// Closed primitive API identities for the shared lazy dispatch index. Never
/// infer types from locals/conversions. Full unpack must have just ONE output
/// directive: unpack1 can ignore errors in trailing directives or UTF-8 bytes.
/// Deliberately omit Unicode, pointer, conversion and multi-value formats.
pub(super) fn primitive_api(call: &CallNode<'_>) -> bool {
    if call.block().is_some() {
        return false;
    }
    if !matches!(call.name().as_slice(), b"unpack" | b"unpack1") {
        return unpack_and_first(call).is_some_and(|(unpack, _)| primitive_api(&unpack));
    }
    if !call
        .receiver()
        .is_some_and(|r| r.as_string_node().is_some())
    {
        return false;
    }
    let args = ast::call_arguments(call);
    let [format] = args.as_slice() else {
        return false;
    };
    let Some(format) = format.as_string_node() else {
        return false;
    };
    let bytes = format.unescaped();
    let Some((&directive, count)) = bytes.split_first() else {
        return false;
    };
    match directive {
        b'a' | b'A' | b'Z' | b'b' | b'B' | b'h' | b'H' => {
            count.is_empty()
                || count == b"*"
                || (count.len() <= 6 && count.iter().all(u8::is_ascii_digit))
        }
        b'C' | b'c' => count.is_empty() || count == b"1",
        _ => false,
    }
}
fn correction(
    call: &CallNode<'_>,
    unpack: &CallNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if !primitive_api(call) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.format_string_facts(root);
    let selector = unpack.message_loc()?;
    let end = ast::source_end(&call.as_node());
    let range = selector.start_offset()..end;
    if !facts.core_before(call.location().start_offset())
        || !facts.closed(&unpack.receiver()?)
        || file.contains_heredoc(root, &call.as_node())
        || file.heredoc_pending_at(root, call.location().start_offset())
        || file.bytes[range.clone()].contains(&b'\n')
        || super::correction::contains_comment(file, range.clone())
    {
        return None;
    }
    // Only the selector and final access are changed. Preserve literal bytes,
    // parentheses and evaluation ownership without constructing/reparsing Ruby.
    let retained =
        std::str::from_utf8(&file.bytes[selector.end_offset()..unpack.location().end_offset()])
            .ok()?;
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range,
            replacement: format!("unpack1{retained}"),
        },
    )
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop unpack_first_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/UnpackFirst")
            .collect()
    }
    #[test]
    fn all_nine_pinned_offenses_have_exact_positions_and_messages() {
        for (source, col, format, current) in [
            ("x.unpack('h*').first", 3, "'h*'", "unpack('h*').first"),
            ("x&.unpack('h*').first", 4, "'h*'", "unpack('h*').first"),
            ("x&.unpack('h*')&.first", 4, "'h*'", "unpack('h*')&.first"),
            ("''.unpack(y)[0]", 4, "y", "unpack(y)[0]"),
            ("''.unpack(y).[](0)", 4, "y", "unpack(y).[](0)"),
            ("''.unpack(y).slice(0)", 4, "y", "unpack(y).slice(0)"),
            ("''&.unpack(y).slice(0)", 5, "y", "unpack(y).slice(0)"),
            ("''&.unpack(y)&.slice(0)", 5, "y", "unpack(y)&.slice(0)"),
            ("''.unpack(y).at(0)", 4, "y", "unpack(y).at(0)"),
        ] {
            let found = offenses(&format!("# header\n  {source}\n"));
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, col + 2, 2)
            );
            assert_eq!(
                found[0].message,
                format!("Use `unpack1({format})` instead of `{current}`.")
            );
        }
    }
    #[test]
    fn canonical_closed_formats_preserve_exact_receiver_and_format_tokens() {
        use super::UnpackFirst;
        use crate::cops::testing::assert_correction;
        for receiver in ["'abc'", "\"é\\0\\xFF\"", "%q(abc)", "?x", "''"] {
            for format in [
                "'h*'",
                "'H2'",
                "'a*'",
                "'A'",
                "'Z0'",
                "'b8'",
                "'B*'",
                "'C'",
                "'c1'",
                "\"\\x68*\"",
            ] {
                for dot in [".", "&."] {
                    for suffix in [
                        ".first",
                        "&.first()",
                        "[0]",
                        ".[](0x0)",
                        ".slice(0)",
                        "&.at(0)",
                    ] {
                        let source = format!("{receiver}{dot}unpack({format}){suffix}");
                        assert_correction(
                            &UnpackFirst,
                            &source,
                            &format!("{receiver}{dot}unpack1({format})"),
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn each_pinned_correction_is_withheld_for_unknown_receiver_or_format() {
        use super::UnpackFirst;
        use crate::cops::testing::assert_correction;
        for source in [
            "x.unpack('h*').first",
            "x&.unpack('h*').first",
            "x&.unpack('h*')&.first",
            "''.unpack(y)[0]",
            "''.unpack(y).[](0)",
            "''.unpack(y).slice(0)",
            "''&.unpack(y).slice(0)",
            "''&.unpack(y)&.slice(0)",
            "''.unpack(y).at(0)",
        ] {
            assert_correction(&UnpackFirst, source, source);
        }
    }
    #[test]
    fn full_unpack_tail_errors_utf8_pointer_formats_and_uncertain_arity_never_fix() {
        use super::UnpackFirst;
        use crate::cops::testing::{assert_correction, correction_offenses};
        // p/P remain AST-only: never execute pointer-dereference formats.
        for format in [
            "'C? ' ",
            "'h*? '",
            "'U*'",
            "'U'",
            "'C2'",
            "'C*'",
            "'p'",
            "'P'",
            "'@1C'",
            "'x'",
            "'m'",
            "''",
            "'h99999999999999999999'",
            "'h* C'",
            "format",
            "*formats",
            "offset: 0",
            "\"#{effect}\"",
        ] {
            let source = format!("'abc'.unpack({format}).first");
            assert_eq!(
                correction_offenses(&UnpackFirst, &source).len(),
                1,
                "{source}"
            );
            assert_correction(&UnpackFirst, &source, &source);
        }
        for source in [
            "nil&.unpack('h*').first",
            "'abc'.to_s.unpack('h*').first",
            "`never execute`.unpack('h*').first",
            "'abc'.unpack('h*').first { effect }",
            "'abc'.unpack('h*'). # keep\n first",
            "'abc'.unpack('h*') \\\n .first",
        ] {
            assert_correction(&UnpackFirst, source, source);
        }
    }
    #[test]
    fn syntax_detection_includes_unknown_splat_keyword_and_outer_block_shapes() {
        for (source, format, current) in [
            (
                "x.unpack(*formats).first",
                "*formats",
                "unpack(*formats).first",
            ),
            (
                "x.unpack(**options).at(0)",
                "**options",
                "unpack(**options).at(0)",
            ),
            (
                "x.unpack(offset: 0).slice(0)",
                "offset: 0",
                "unpack(offset: 0).slice(0)",
            ),
            ("x.unpack(y).first { effect }", "y", "unpack(y).first"),
            ("x.unpack(y)[-0]", "y", "unpack(y)[-0]"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 3, 1));
            assert_eq!(
                found[0].message,
                format!("Use `unpack1({format})` instead of `{current}`.")
            );
        }
    }
    #[test]
    fn sole_block_pass_arguments_report_exactly_but_never_correct() {
        use super::UnpackFirst;
        use crate::cops::testing::assert_correction;
        for (source, col, argument, current) in [
            ("x.unpack(&b).first", 3, "&b", "unpack(&b).first"),
            ("x&.unpack(&nil)&.at(0)", 4, "&nil", "unpack(&nil)&.at(0)"),
            ("'abc'.unpack(&:foo)[0]", 7, "&:foo", "unpack(&:foo)[0]"),
            (
                "'abc'.unpack(&effect).slice(0)",
                7,
                "&effect",
                "unpack(&effect).slice(0)",
            ),
            (
                "def probe(&); x.unpack(&).slice(0); end",
                17,
                "&",
                "unpack(&).slice(0)",
            ),
        ] {
            let source = format!("# header\n  {source}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, col + 2, 2)
            );
            assert_eq!(
                found[0].message,
                format!("Use `unpack1({argument})` instead of `{current}`.")
            );
            assert_correction(&UnpackFirst, &source, &source);
        }
    }
    #[test]
    fn canonical_edits_preserve_argument_binding_control_flow_and_receiver_continuations() {
        use super::UnpackFirst;
        use crate::cops::testing::assert_correction;
        for (source, after) in [
            (
                "consume('abc'.unpack('h*').first, 2)",
                "consume('abc'.unpack1('h*'), 2)",
            ),
            // The shared eager proof deliberately does not model conditional
            // or rescue execution. Keep those offenses, withholding edits.
            (
                "'abc'.unpack('h*').first || fallback",
                "'abc'.unpack('h*').first || fallback",
            ),
            (
                "'abc'.unpack('h*').first rescue fallback",
                "'abc'.unpack('h*').first rescue fallback",
            ),
            (
                "'abc' \\\n .unpack('h*').first",
                "'abc' \\\n .unpack1('h*')",
            ),
            ("'abc'.unpack('h*').first.to_s", "'abc'.unpack1('h*').to_s"),
        ] {
            assert_correction(&UnpackFirst, source, after);
        }
    }
    #[test]
    fn pinned_negatives_and_arity_receiver_block_lookalikes() {
        for source in [
            "x.unpack1(y)",
            "''.unpack('h*')[1]",
            "x.unpack(y).first(1)",
            "x.unpack(y).slice(0, 1)",
            "x.unpack(y)[0, 1]",
            "x.unpack(y).at(-1)",
            "x.unpack(y).first(&block)",
            "x.unpack(y, z).first",
            "x.unpack.first",
            "unpack(y).first",
            "x.unpack(y) { effect }.first",
            "x.unpack { effect }.first",
            "x.unpack do effect end.first",
            "x.unpack(y, &block).first",
            "(x.unpack(y)).first",
            "x.unpack!(y).first",
            "x.unpack(y).fetch(0)",
            "# x.unpack(y).first\n'unpack.first'",
            "def unpack; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
