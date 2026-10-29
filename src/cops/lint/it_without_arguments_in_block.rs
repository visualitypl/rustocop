//! `Lint/ItWithoutArgumentsInBlock` — legacy bare `it` calls in blocks.
//! The maximum Ruby 3.3 gate is mandatory: Ruby 3.4's implicit `it` is valid.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The legacy `Lint/ItWithoutArgumentsInBlock` cop.
pub struct ItWithoutArgumentsInBlock;

impl Cop for ItWithoutArgumentsInBlock {
    fn name(&self) -> &'static str {
        "Lint/ItWithoutArgumentsInBlock"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let bare_it = if let Some(call) = node.as_call_node() {
            call.name().as_slice() == b"it"
                && call.receiver().is_none()
                && call.arguments().is_none()
                && call.opening_loc().is_none()
                && call.block().is_none()
        } else {
            // The identical bare spelling in Prism's fixed-3.4 AST. This is
            // only observable via the direct matcher seam: production selection
            // excludes the cop before dispatch, including source force-enables.
            node.as_it_local_variable_read_node().is_some()
        };
        if !bare_it {
            return;
        }
        // RuboCop wraps a send AND its arguments in :block, whereas Prism
        // makes the attached block a sibling of the arguments. Include that
        // wrapper even when `it` is in the receiver/arguments, not its body.
        // :block includes lambdas but not numbered-parameter blocks.
        let Some(parameters) = cx.ancestors().find_map(|ancestor| {
            let parameters = if let Some(block) = ancestor.as_block_node() {
                block.parameters()
            } else if let Some(call) = ancestor.as_call_node() {
                call.block()?.as_block_node()?.parameters()
            } else {
                ancestor.as_lambda_node()?.parameters()
            };
            if parameters
                .as_ref()
                .is_some_and(|p| p.as_numbered_parameters_node().is_some())
            {
                return None;
            }
            Some(parameters)
        }) else {
            return;
        };
        if parameters.is_some_and(|p| p.as_it_parameters_node().is_none()) {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message:
                "`it` calls without arguments will refer to the first block param in Ruby 3.4; \
                      use `it()` or `self.it`."
                    .into(),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's it_without_arguments_in_block_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::cops::run_cops;

    // Direct matcher seam, NOT a legacy parser or a supported public target.
    // Prism represents unshadowed bare `it` as ItLocalVariableRead at 3.4.
    fn legacy_shape(source: &str) -> Vec<Offense> {
        let parsed = crate::parser::parse(source.as_bytes());
        let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        let mut out = vec![];
        assert!(run_cops(&[&ItWithoutArgumentsInBlock], &parsed, &file, &mut out).is_empty());
        out
    }

    #[test]
    fn legacy_bare_it_positions_and_message() {
        for (source, line, col) in [
            ("0.times { it }", 1, 11),
            ("0.times do\n  it\n  it = 1\n  it\nend", 2, 3),
            ("0.times { consume(it) }", 1, 19),
            ("0.times { def foo; it; end }", 1, 20),
            ("-> { it }", 1, 6),
        ] {
            let out = legacy_shape(source);
            assert_eq!(out.len(), 1, "{source}");
            assert_eq!((out[0].line, out[0].col), (line, col), "{source}");
            assert_eq!(
                out[0].message,
                "`it` calls without arguments will refer to the first block param in Ruby 3.4; use `it()` or `self.it`."
            );
            assert!(out[0].correction.is_none());
        }
    }

    #[test]
    fn legacy_call_attached_block_ancestry() {
        let out = legacy_shape("consume(it) {}");
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].line, out[0].col), (1, 9));
        assert_eq!(
            out[0].message,
            "`it` calls without arguments will refer to the first block param in Ruby 3.4; use `it()` or `self.it`."
        );
        assert!(legacy_shape("outer { consume(it) { |x| } }").is_empty());
    }

    #[test]
    fn all_pinned_legacy_no_offense_lookalikes() {
        for source in [
            "0.times { it(42) }",
            "0.times { it { do_something } }",
            "0.times { it() }",
            "0.times { self.it }",
            "0.times do\n it(42)\n it = 1\n it\nend",
            "0.times do\n it { do_something }\n it = 1\n it\nend",
            "0.times do\n it()\n it = 1\n it\nend",
            "0.times do\n self.it\n it = 1\n it\nend",
            "if false\n it\nend",
            "def foo\n it\nend",
            "0.times { || it }",
            "0.times { |_n| it }",
            "0.times do\n it = 1\n it\nend",
            "it = 1\n0.times { it }",
            "0.times {}",
            "outer { inner { |x| it } }",
            "outer { |x| inner { || it } }",
            "->() { it }",
            "it",
            "object.it",
            "0.times { it(&block) }",
        ] {
            assert!(legacy_shape(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn every_pinned_legacy_spec_and_source_shape_matches_the_reference() {
        let rows: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/legacy_lint.json")).unwrap();
        let mut count = 0;
        for row in rows.as_array().unwrap() {
            if row["cop"] != ItWithoutArgumentsInBlock.name() || row["legacy_version"] == 3.4 {
                continue;
            }
            let out = legacy_shape(row["source"].as_str().unwrap());
            let actual: Vec<_> = out
                .iter()
                .map(|o| serde_json::json!([o.line, o.col]))
                .collect();
            assert_eq!(serde_json::json!(actual), row["legacy_positions"], "{row}");
            assert_eq!(
                serde_json::json!(out.iter().map(|o| &o.message).collect::<Vec<_>>()),
                row["legacy_messages"],
                "{row}"
            );
            assert!(out.iter().all(|o| o.correction.is_none()));
            count += 1;
        }
        assert_eq!(count, 20);
    }

    #[test]
    fn fixed_ruby34_accepts_both_pinned_modern_examples() {
        for source in ["0.times { it }", "0.times do\n it\n it = 1\n it\nend"] {
            assert!(
                crate::pipeline::lint_str(source)
                    .iter()
                    .all(|o| o.cop != ItWithoutArgumentsInBlock.name())
            );
        }
    }
}
