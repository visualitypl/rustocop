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
            .then(|| correction(&call, &unpack))
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

/// RuboCop's corrector: `unpack1`, without the first-element access.
fn correction(call: &CallNode<'_>, unpack: &CallNode<'_>) -> Option<Correction> {
    let selector = unpack.message_loc()?;
    let retained = std::str::from_utf8(
        &unpack.location().as_slice()[selector.end_offset() - unpack.location().start_offset()..],
    )
    .ok()?;
    Some(Correction {
        range: selector.start_offset()..ast::source_end(&call.as_node()),
        replacement: format!("unpack1{retained}"),
    })
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
