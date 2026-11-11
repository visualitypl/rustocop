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
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(selector.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(selector.end_offset() - 1),
            message: "`BigDecimal.new()` is deprecated. Use `BigDecimal()` instead.".into(),
            correction: cx
                .corrections_requested()
                .then(|| closed_factory_wrapper(node, cx, file))
                .flatten(),
        });
    }
}

/// Native BigDecimal.new is removed. Only this closed compatibility definition
/// establishes the original constructor as a genuine forwarding call to the same
/// real C factory as the replacement. Keep that definition unchanged. Its one
/// required formal is read exactly once; no project binding/alias inference or
/// local-variable rewrite is involved. A short plain decimal string cannot raise
/// the conversion/type/precision errors whose backtraces would expose the removed
/// Ruby frame. No other statements, delayed code, hooks or observers are admitted.
/// The inherited lazy main-parse source/position/code-object/heap guards still
/// filter every edit. Exact spelling excludes discarded comments and heredocs.
fn closed_factory_wrapper(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let call = node.as_call_node()?;
    if call.block().is_some() {
        return None;
    }
    let receiver = call.receiver()?;
    let receiver = match receiver.location().as_slice() {
        b"BigDecimal" => "BigDecimal",
        b"::BigDecimal" => "::BigDecimal",
        _ => return None,
    };
    let arguments = ast::call_arguments(&call);
    let [argument] = arguments.as_slice() else {
        return None;
    };
    let string = argument.as_string_node()?;
    let literal = argument.location().as_slice();
    if literal.len() < 3
        || !matches!(literal[0], b'\'' | b'"')
        || literal.last() != literal.first()
        || string.unescaped() != &literal[1..literal.len() - 1]
    {
        return None;
    }
    let mut parts = string.unescaped().split(|b| *b == b'.');
    let digits =
        |part: &[u8]| (1..=16).contains(&part.len()) && part.iter().all(u8::is_ascii_digit);
    if !digits(parts.next()?)
        || parts.next().is_some_and(|part| !digits(part))
        || parts.next().is_some()
    {
        return None;
    }
    let literal = std::str::from_utf8(literal).ok()?;
    if node.location().as_slice() != format!("{receiver}.new({literal})").as_bytes() {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let array = ancestors.next()?.as_array_node()?;
    if array.elements().len() != 1 {
        return None;
    }
    let statements = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some() || statements.body().len() != 3 {
        return None;
    }
    let body = statements.body();
    let mut nodes = body.iter();
    let require = nodes.next()?.as_call_node()?;
    if !matches!(
        require.location().as_slice(),
        b"require 'bigdecimal'" | b"require \"bigdecimal\""
    ) {
        return None;
    }
    let definition = nodes.next()?.as_def_node()?;
    if definition.location().as_slice()
        != b"def BigDecimal.new(value); ::Kernel.BigDecimal(value); end"
        || nodes.next()?.location().start_offset() != array.location().start_offset()
    {
        return None;
    }
    Some(crate::offense::Correction {
        range: node.location().start_offset()..node.location().end_offset(),
        replacement: format!("BigDecimal({literal})"),
    })
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
    fn all_pinned_source_and_control_fixtures() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deprecated_constructors.json"
        ))
        .unwrap();
        for row in rows.iter().filter(|r| r["cop"] == "Lint/BigDecimalNew") {
            let source = row["source"].as_str().unwrap();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            let found = offenses(source);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
            let found = crate::cops::testing::correction_offenses(&super::BigDecimalNew, source);
            assert!(found.iter().all(|o| o.correction.is_none()), "{row}");
        }
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
    fn uncertain_factory_binding_formals_values_and_observers_keep_findings() {
        let safe = "require 'bigdecimal'\ndef BigDecimal.new(value); ::Kernel.BigDecimal(value); end\n[BigDecimal.new('1.25')]\n";
        let mut sources = vec![
            "BigDecimal.new('1.25')".to_owned(),
            "require 'bigdecimal'\n[BigDecimal.new('1.25')]".to_owned(),
        ];
        for (from, to) in [
            ("require 'bigdecimal'", "require 'other'"),
            ("::Kernel.BigDecimal(value)", ":constant"),
            ("::Kernel.BigDecimal(value)", "BigDecimal(value)"),
            ("::Kernel.BigDecimal(value)", "::Kernel.BigDecimal('2')"),
            ("new(value)", "new(value = '2')"),
            ("new(value)", "new(*value)"),
            ("new(value)", "new(other)"),
            ("'1.25'", "'bad'"),
            ("'1.25'", "'1e999999999999999999'"),
            ("'1.25'", "'-1.25'"),
            ("'1.25'", "'12345678901234567'"),
            ("'1.25'", "\"1\\x2e25\""),
            ("'1.25'", "value"),
            ("'1.25'", "1.25"),
            (".new('1.25')", ".new('1.25', 4)"),
            (".new('1.25')", ".new('1.25', &nil)"),
            (".new('1.25')", ".new('1.25') { work }"),
            (".new('1.25')", ".new( # keep\n'1.25')"),
            ("[BigDecimal.new('1.25')]", "[BigDecimal.new('1.25'), 1]"),
            ("[BigDecimal.new('1.25')]", "BigDecimal.new('1.25')"),
        ] {
            sources.push(safe.replace(from, to));
        }
        for observer in [
            "GC.stat",
            "ObjectSpace.each_object {}",
            "TracePoint.new(:call) {}.enable",
            "File.read(__FILE__)",
            "RubyVM::InstructionSequence.compile_file(__FILE__)",
            "p __LINE__",
            "DATA.read\n__END__\ndata\n",
            "def BigDecimal(value); :other; end",
            "BigDecimal = Object",
            "module Kernel; def BigDecimal(value); :other; end; end",
        ] {
            if !observer.contains("__END__") {
                sources.push(format!("{observer}\n{safe}"));
            }
            sources.push(format!("{safe}{observer}\n"));
        }
        sources.push(format!("def f; {safe}end"));
        for source in sources {
            let found = crate::cops::testing::correction_offenses(&super::BigDecimalNew, &source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }

    #[test]
    fn many_candidate_siblings_keep_findings_without_root_rescans() {
        let source =
            "require 'bigdecimal'\ndef BigDecimal.new(value); ::Kernel.BigDecimal(value); end\n"
                .to_owned()
                + &"[BigDecimal.new('1.25')]\n".repeat(6000);
        let found = crate::cops::testing::correction_offenses(&super::BigDecimalNew, &source);
        assert_eq!(found.len(), 6000);
        assert!(found.iter().all(|o| o.correction.is_none()));
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
