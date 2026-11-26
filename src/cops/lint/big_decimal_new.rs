//! `Lint/BigDecimalNew` — deprecated BigDecimal constructor calls.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct BigDecimalNew;
impl Cop for BigDecimalNew {
    fn name(&self) -> &'static str {
        "Lint/BigDecimalNew"
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
        // Prism fuses the zero-argument getter send into a compound write.
        // Its corresponding RuboCop callback still reports `new`, not `new=`.
        macro_rules! getter {
            ($access:ident) => {{
                let call = node.$access().unwrap();
                if call.read_name().as_slice() != b"new" || call.is_safe_navigation() {
                    return;
                }
                (call.receiver(), call.message_loc())
            }};
        }
        let (receiver, selector) = match node {
            Node::CallAndWriteNode { .. } => getter!(as_call_and_write_node),
            Node::CallOrWriteNode { .. } => getter!(as_call_or_write_node),
            Node::CallOperatorWriteNode { .. } => getter!(as_call_operator_write_node),
            _ => {
                let Some(call) = node.as_call_node() else {
                    return;
                };
                if call.name().as_slice() != b"new" || call.is_safe_navigation() {
                    return;
                }
                (call.receiver(), call.message_loc())
            }
        };
        if !receiver.is_some_and(|receiver| ast::is_global_const(&receiver, &["BigDecimal"])) {
            return;
        }
        let Some(selector) = selector else { return };
        let (line, col) = file.reference_line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(selector.end_offset() - 1),
            message: "`BigDecimal.new()` is deprecated. Use `BigDecimal()` instead.".into(),
            correction: cx
                .corrections_requested()
                .then(|| {
                    let call = node.as_call_node()?;
                    let receiver = call.receiver()?;
                    // From the `::` (or the constant) to the selector's end, the
                    // constant name kept.
                    let start = receiver.location().start_offset();
                    let name = b"BigDecimal".len();
                    let constant_end = receiver.location().end_offset();
                    let mut edits = vec![crate::offense::Correction {
                        range: constant_end..selector.end_offset(),
                        replacement: String::new(),
                    }];
                    if constant_end - start > name {
                        edits.push(crate::offense::Correction {
                            range: start..constant_end - name,
                            replacement: String::new(),
                        });
                    }
                    crate::cops::style::correction::combine(file, edits)
                })
                .flatten(),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/big_decimal_new_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/BigDecimalNew")
            .collect()
    }

    #[test]
    fn pinned_constructors_report_the_selector() {
        for (source, col) in [
            ("BigDecimal.new(123.456, 3)\n", 12),
            ("::BigDecimal.new(123.456, 3)\n", 14),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert_eq!(
                found[0].message,
                "`BigDecimal.new()` is deprecated. Use `BigDecimal()` instead."
            );
        }
    }

    #[test]
    fn declared_utf8_mac_selector_columns_match_the_composed_reference_buffer() {
        for (encoding, col) in [("UTF8-MAC", 17), ("UTF-8", 18)] {
            for eol in ["\n", "\r\n"] {
                for (receiver, extra) in [("BigDecimal", 0), ("::BigDecimal", 2)] {
                    for suffix in ["(1)", " ||= value", " &&= value", " += value"] {
                        let source = format!(
                            "# coding: {encoding}{eol}\"e\u{301}\"; {receiver}.new{suffix}{eol}"
                        );
                        let found = offenses(&source);
                        assert_eq!(found.len(), 1, "{source}");
                        assert_eq!(
                            (found[0].line, found[0].col, found[0].end_line),
                            (2, col + extra, 2),
                            "{source}"
                        );
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn closed_real_factory_wrapper_has_a_canonical_edit() {
        for receiver in ["BigDecimal", "::BigDecimal"] {
            for literal in ["'1.25'", "\"0\"", "'1234567890123456.0000125'"] {
                let prefix = "require 'bigdecimal'\ndef BigDecimal.new(value); ::Kernel.BigDecimal(value); end\n";
                let before = format!("{prefix}[{receiver}.new({literal})]\n");
                let after = format!("{prefix}[BigDecimal({literal})]\n");
                crate::cops::testing::assert_correction(&super::BigDecimalNew, &before, &after);
            }
        }
    }

    #[test]
    fn factory_and_receiver_lookalikes_are_clean() {
        for source in [
            "BigDecimal(123.456, 3)",
            "Other::BigDecimal.new(1)",
            "::Other::BigDecimal.new(1)",
            "BigDecimal&.new(1)",
            "(BigDecimal).new(1)",
            "object.BigDecimal.new(1)",
            "BigDecimal.allocate",
            "big_decimal.new(1)",
            "BigDecimal::Nested.new(1)",
            "'BigDecimal.new(1)'",
            "# BigDecimal.new(1)\n",
            "def BigDecimal.new(x); x; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
