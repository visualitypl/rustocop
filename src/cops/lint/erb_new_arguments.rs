//! `Lint/ErbNewArguments` — ERB's deprecated positional constructor arguments.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ErbNewArguments;
impl Cop for ErbNewArguments {
    fn name(&self) -> &'static str {
        "Lint/ErbNewArguments"
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
        if call.name().as_slice() != b"new"
            || call.is_safe_navigation()
            || !call
                .receiver()
                .is_some_and(|receiver| ast::is_global_const(&receiver, &["ERB"]))
        {
            return;
        }
        // RuboCop counts an explicit &block as a block_pass argument too.
        // Braced and keyword hashes both map to its hash node, at any slot.
        for (i, argument) in ast::call_arguments(&call)
            .iter()
            .skip(1)
            .take(3)
            .enumerate()
        {
            if matches!(
                argument,
                Node::HashNode { .. } | Node::KeywordHashNode { .. }
            ) {
                continue;
            }
            let location = argument.location();
            // Quote the reference source buffer, not UTF8-lossy raw bytes:
            // legacy argument/identifier data must survive in the suggestion.
            // The existing renderer projects declared encodings and explicitly
            // escapes bytes without a Unicode mapping; it never changes edits.
            let value =
                super::ambiguous_block_association::reference_source(location.as_slice(), file);
            let message = match i {
                0 => "Passing safe_level with the 2nd argument of `ERB.new` is deprecated. Do not use it, and specify other arguments as keyword arguments.".into(),
                1 => format!("Passing trim_mode with the 3rd argument of `ERB.new` is deprecated. Use keyword argument like `ERB.new(str, trim_mode: {value})` instead."),
                _ => format!("Passing eoutvar with the 4th argument of `ERB.new` is deprecated. Use keyword argument like `ERB.new(str, eoutvar: {value})` instead."),
            };
            let (line, col) = file.reference_line_col(location.start_offset());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.line_of(location.end_offset() - 1),
                message,
                // One actual arguments-range edit; the other slot's finding is
                // retained until re-lint. Never drop evaluated ordinary args.
                correction: (i == 0 && cx.corrections_requested())
                    .then(|| closed_method_reference(node, cx, file))
                    .flatten(),
            });
        }
    }
}

/// The real stdlib establishes ERB.new on BOTH sides. Under defined? this
/// constructor is not invoked: no deprecation warning, template compilation,
/// argument evaluation, error or evaluation-order change. A single consumed
/// reference in a closed two-statement program excludes namespace mutations,
/// method hooks, source/VM/heap observations and delayed execution. Do not infer
/// this property for ordinary constructors, unresolved APIs or arbitrary args.
/// Exact spelling excludes comment/encoding/heredoc loss; inherited main-parse
/// source/position/code-object/heap guards remain authoritative.
fn closed_method_reference(
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
        b"ERB" => "ERB",
        b"::ERB" => "::ERB",
        _ => return None,
    };
    let arguments = ast::call_arguments(&call);
    let [template, safe_level, trim] = arguments.as_slice() else {
        return None;
    };
    template.as_string_node()?;
    safe_level.as_nil_node()?;
    if !matches!(trim.location().as_slice(), b"'-'" | b"\"-\"") {
        return None;
    }
    let literal = template.location().as_slice();
    if literal.len() < 2
        || !matches!(literal[0], b'\'' | b'"')
        || literal.last() != literal.first()
        || literal.iter().any(|b| !b.is_ascii_graphic() && *b != b' ')
        || literal.contains(&b'\\')
    {
        return None;
    }
    let literal = std::str::from_utf8(literal).ok()?;
    let trim = std::str::from_utf8(trim.location().as_slice()).ok()?;
    if node.location().as_slice() != format!("{receiver}.new({literal}, nil, {trim})").as_bytes() {
        return None;
    }
    let mut ancestors = cx.ancestors();
    ancestors.next()?.as_defined_node()?;
    let array = ancestors.next()?.as_array_node()?;
    if array.elements().len() != 1 {
        return None;
    }
    let statements = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some() || statements.body().len() != 2 {
        return None;
    }
    let body = statements.body();
    let mut nodes = body.iter();
    let require = nodes.next()?.as_call_node()?;
    if !matches!(
        require.location().as_slice(),
        b"require 'erb'" | b"require \"erb\""
    ) || nodes.next()?.location().start_offset() != array.location().start_offset()
    {
        return None;
    }
    let args = call.arguments()?.location();
    Some(crate::offense::Correction {
        range: args.start_offset()..args.end_offset(),
        replacement: format!("{literal}, trim_mode: {trim}"),
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/erb_new_arguments_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/ErbNewArguments")
            .collect()
    }
    #[test]
    fn all_pinned_source_and_control_fixtures() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deprecated_constructors.json"
        ))
        .unwrap();
        for row in rows.iter().filter(|r| r["cop"] == "Lint/ErbNewArguments") {
            let source = row["source"].as_str().unwrap();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            let found = offenses(source);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
            let found = crate::cops::testing::correction_offenses(&super::ErbNewArguments, source);
            assert!(found.iter().all(|o| o.correction.is_none()), "{row}");
        }
    }

    #[test]
    fn lead_legacy_argument_suggestions_preserve_trim_and_eoutvar_data() {
        for (source, col, value) in [
            (
                b"# encoding: ISO-8859-1\nERB.new(\"x\", nil, \"caf\xe9\")\n".as_slice(),
                19,
                "trim_mode: \"café\"",
            ),
            (
                b"# encoding: ISO-8859-1\nERB.new(\"x\", nil, \"-\", \"_out\xe9\")\n",
                24,
                "eoutvar: \"_outé\"",
            ),
        ] {
            let file = crate::source::SourceFile::new("example.rb".into(), source.to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            assert_eq!(parsed.errors().count(), 0);
            file.record_encoding(&parsed);
            let mut found = Vec::new();
            assert!(
                crate::cops::run_cops(&[&super::ErbNewArguments], &parsed, &file, &mut found)
                    .is_empty()
            );
            let offense = found.iter().find(|o| o.col == col).unwrap();
            assert_eq!((offense.line, offense.end_line), (2, 2));
            assert!(offense.message.contains(value), "{offense:?}");
            assert!(!offense.message.contains('\u{fffd}'));
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn encoded_literals_identifiers_multiline_and_binary_fallback_match_reference() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deprecated_constructor_encodings.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 327);
        for row in rows {
            let hex = row["source_hex"].as_str().unwrap();
            let source = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let file = crate::source::SourceFile::new("example.rb".into(), source);
            let parsed = crate::parser::parse(&file.bytes);
            assert_eq!(parsed.errors().count(), 0, "{row}");
            file.record_encoding(&parsed);
            let mut found = Vec::new();
            assert!(
                crate::cops::run_cops(&[&super::ErbNewArguments], &parsed, &file, &mut found)
                    .is_empty()
            );
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            found.sort_by_key(|o| (o.line, o.col));
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
            assert!(found.iter().all(|o| o.correction.is_none()), "{row}");
        }
    }

    #[test]
    fn positional_arguments_have_exact_ranges_and_messages() {
        for prefix in ["", "::"] {
            for suffix in [
                "nil",
                "nil, '-'",
                "nil, '-', '@output_buffer'",
                "nil, '-', '@output_buffer', trim_mode: '-', eoutvar: '@output_buffer'",
                "nil, '-', trim_mode: '-', eoutvar: '@output_buffer'",
                "nil, trim_mode: '-'",
            ] {
                let source = format!("{prefix}ERB.new(str, {suffix})\n");
                let found = offenses(&source);
                let n = if suffix.contains("'@output_buffer', trim")
                    || suffix == "nil, '-', '@output_buffer'"
                {
                    3
                } else if suffix.starts_with("nil, '-'") {
                    2
                } else {
                    1
                };
                assert_eq!(found.len(), n, "{source}");
                for (i, o) in found.iter().enumerate() {
                    let col = [14, 19, 24][i] + prefix.len() as u32;
                    assert_eq!((o.line, o.col, o.end_line), (1, col, 1), "{source}");
                    let msg = match i {
                        0 => {
                            "Passing safe_level with the 2nd argument of `ERB.new` is deprecated. Do not use it, and specify other arguments as keyword arguments."
                        }
                        1 => {
                            "Passing trim_mode with the 3rd argument of `ERB.new` is deprecated. Use keyword argument like `ERB.new(str, trim_mode: '-')` instead."
                        }
                        _ => {
                            "Passing eoutvar with the 4th argument of `ERB.new` is deprecated. Use keyword argument like `ERB.new(str, eoutvar: '@output_buffer')` instead."
                        }
                    };
                    assert_eq!(o.message, msg);
                }
            }
        }
    }
    #[test]
    fn declared_utf8_mac_argument_columns_match_the_composed_reference_buffer() {
        for (encoding, columns) in [("UTF8-MAC", [14, 19, 24]), ("UTF-8", [15, 20, 25])] {
            for eol in ["\n", "\r\n"] {
                for (receiver, extra) in [("ERB", 0), ("::ERB", 2)] {
                    let source = format!(
                        "# coding: {encoding}{eol}{receiver}.new(\"e\u{301}\", nil, \"-\", \"_out\"){eol}"
                    );
                    let found = offenses(&source);
                    assert_eq!(found.len(), 3, "{source}");
                    for (offense, col) in found.iter().zip(columns) {
                        assert_eq!(
                            (offense.line, offense.col, offense.end_line),
                            (2, col + extra, 2),
                            "{source}"
                        );
                        assert!(offense.correction.is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn closed_loaded_method_reference_has_a_canonical_edit() {
        for receiver in ["ERB", "::ERB"] {
            for literal in ["'task-literal'", "\"hello <%= 1 %>\"", "''"] {
                let before =
                    format!("require 'erb'\n[defined?({receiver}.new({literal}, nil, '-'))]\n");
                let after = format!(
                    "require 'erb'\n[defined?({receiver}.new({literal}, trim_mode: '-'))]\n"
                );
                crate::cops::testing::assert_correction(&super::ErbNewArguments, &before, &after);
            }
        }
    }

    #[test]
    fn uncertain_method_references_evaluation_comments_and_observers_keep_findings() {
        let safe = "require 'erb'\n[defined?(ERB.new('task-literal', nil, '-'))]\n";
        let mut sources = Vec::new();
        for (from, to) in [
            ("require 'erb'\n", ""),
            ("require 'erb'", "require 'other'"),
            (
                "defined?(ERB.new('task-literal', nil, '-'))",
                "ERB.new('task-literal', nil, '-')",
            ),
            ("[defined?", "[1, defined?"),
            ("'task-literal'", "unknown"),
            ("'task-literal'", "effect()"),
            ("nil, '-'", "effect(), '-'"),
            ("nil, '-'", "nil, effect()"),
            ("nil, '-'", "nil, '-', '@buf'"),
            ("nil, '-'", "nil, '-', **kwargs"),
            ("nil, '-'", "nil, '-', &block"),
            ("nil, '-'", "nil, # keep\n'-'"),
            ("nil, '-'", "nil, <<~MODE\n-\nMODE\n"),
            ("nil, '-'", "nil, '-' # keep\n"),
            (
                ".new('task-literal', nil, '-')",
                ".new('task-literal', nil, '-') { work }",
            ),
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
            "ERB = Object",
            "def ERB.new(*); :other; end",
        ] {
            if !observer.contains("__END__") {
                sources.push(format!("{observer}\n{safe}"));
            }
            sources.push(format!("{safe}{observer}\n"));
        }
        sources.push(format!("def f; {safe}end"));
        for source in sources {
            let found = crate::cops::testing::correction_offenses(&super::ErbNewArguments, &source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }

    #[test]
    fn many_reference_siblings_keep_findings_without_root_rescans() {
        let source = "require 'erb'\n".to_owned()
            + &"[defined?(ERB.new('task-literal', nil, '-'))]\n".repeat(6000);
        let found = crate::cops::testing::correction_offenses(&super::ErbNewArguments, &source);
        assert_eq!(found.len(), 12000);
        assert!(found.iter().all(|o| o.correction.is_none()));
    }

    #[test]
    fn keyword_hashes_empty_calls_and_receiver_lookalikes_are_clean() {
        for source in [
            "ERB.new(str, trim_mode: '-', eoutvar: '@output_buffer')",
            "ERB.new(str)",
            "ERB.new",
            "ERB.new(str, {})",
            "ERB.new(str, {trim_mode: '-'})",
            "Other::ERB.new(str, nil)",
            "::Other::ERB.new(str, nil)",
            "(ERB).new(str, nil)",
            "ERB&.new(str, nil)",
            "erb.new(str, nil)",
            "ERB.allocate(str, nil)",
            "'ERB.new(str, nil)'",
            "# ERB.new(str, nil)\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
