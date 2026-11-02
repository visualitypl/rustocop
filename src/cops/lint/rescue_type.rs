//! `Lint/RescueType` — literal exception filters that cannot match exceptions.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RescueType;
impl Cop for RescueType {
    fn name(&self) -> &'static str {
        "Lint/RescueType"
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
        let Some(rescue) = node.as_rescue_node() else {
            return;
        };
        let exceptions: Vec<_> = rescue.exceptions().iter().collect();
        let invalid: Vec<_> = exceptions
            .iter()
            .filter(|n| invalid_type(n, file))
            .collect();
        if invalid.is_empty() {
            return;
        }
        let names = invalid
            .iter()
            .map(|n| {
                let raw = file.match_pattern_source(n.location().as_slice());
                raw.utf8_chunks()
                    .map(|c| {
                        let mut s = c.valid().to_owned();
                        for b in c.invalid() {
                            s.push_str(&format!("\\x{b:02X}"));
                        }
                        s
                    })
                    .collect::<String>()
                    .replace("\r\n", "\n")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            rescue.keyword_loc().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(), severity: self.severity(), line, col,
            end_line: file.line_of(exceptions.last().unwrap().location().end_offset().saturating_sub(1)),
            message: format!("Rescuing from `{names}` will raise a `TypeError` instead of catching the actual exception."),
            correction: cx.corrections_requested().then(|| live_correction(node, cx, file)).flatten(),
        });
    }
}
/// A closed, live version of the reviewed witness, not exception type flow.
/// The sole top-level begin has one standard raise and a single standard puts
/// handler. No other executable syntax can replace raise, StandardError or ===,
/// install hooks/refinements, expose bindings, or observe changed source/objects.
/// Never generalize this to arbitrary protected calls or effectful type lists.
/// The pipeline's shared source/position/VM/heap guards still filter this edit.
fn live_correction(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let rescue = node.as_rescue_node()?;
    let exceptions: Vec<_> = rescue.exceptions().iter().collect();
    let [valid, invalid] = exceptions.as_slice() else {
        return None;
    };
    if valid.location().as_slice() != b"StandardError"
        || valid.as_constant_read_node().is_none()
        || invalid.as_nil_node().is_none()
        || rescue.reference().is_some()
        || rescue.subsequent().is_some()
    {
        return None;
    }
    let mut parents = cx.ancestors();
    let begin = parents.next()?.as_begin_node()?;
    let top = parents.next()?.as_statements_node()?;
    parents.next()?.as_program_node()?;
    if parents.next().is_some()
        || top.body().len() != 1
        || begin.begin_keyword_loc().is_none()
        || begin.else_clause().is_some()
        || begin.ensure_clause().is_some()
    {
        return None;
    }
    let protected = begin.statements()?;
    let handler = rescue.statements()?;
    if protected.body().len() != 1 || handler.body().len() != 1 {
        return None;
    }
    let raise = protected.body().iter().next()?.as_call_node()?;
    let puts = handler.body().iter().next()?.as_call_node()?;
    for (call, name) in [(&raise, b"raise".as_slice()), (&puts, b"puts".as_slice())] {
        if call.name().as_slice() != name
            || call.receiver().is_some()
            || call.block().is_some()
            || call.arguments()?.arguments().len() != 1
        {
            return None;
        }
    }
    let raised = raise.arguments()?.arguments().iter().next()?;
    let printed = puts.arguments()?.arguments().iter().next()?;
    if raised.as_constant_read_node().is_none()
        || raised.location().as_slice() != b"StandardError"
        || printed.as_symbol_node().is_none()
        || printed.location().as_slice() != b":caught"
    {
        return None;
    }
    let start = rescue.keyword_loc().end_offset();
    let end = invalid.location().end_offset();
    // Canonical upstream replacement is keyword.end..exceptions.end, not just
    // a comma token deletion. Refuse comments, continuations or physical lines.
    let removed = file.bytes.get(start..end)?;
    if removed
        .iter()
        .any(|b| !b.is_ascii_alphabetic() && !matches!(b, b' ' | b'\t' | b','))
    {
        return None;
    }
    Some(crate::offense::Correction {
        range: start..end,
        replacement: " StandardError".into(),
    })
}
fn invalid_type(node: &Node<'_>, file: &SourceFile) -> bool {
    matches!(
        node,
        Node::ArrayNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::InterpolatedStringNode { .. }
            | Node::FalseNode { .. }
            | Node::FloatNode { .. }
            | Node::HashNode { .. }
            | Node::NilNode { .. }
            | Node::IntegerNode { .. }
            | Node::RationalNode { .. }
            | Node::StringNode { .. }
            | Node::TrueNode { .. }
            | Node::SourceLineNode { .. }
            | Node::SourceFileNode { .. }
    ) || crate::cops::ast::is_reference_symbol(node, file)
        || crate::cops::ast::negated_numeric_literal(node).is_some()
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/rescue_type_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/RescueType")
            .collect()
    }
    #[test]
    fn pinned_invalid_taxonomy_all_shared_contexts() {
        for literal in [
            "nil",
            "'string'",
            "\"#{string}\"",
            "0",
            "0.0",
            "[]",
            "{}",
            ":symbol",
            "true",
            "false",
            "1r",
            "1i",
            "-1",
            "-1.0",
            "-1r",
            "-1i",
            "--1",
            "__LINE__",
            "__FILE__",
        ] {
            for (header, ensure) in [
                ("begin", ""),
                ("begin", "ensure\n  baz\n"),
                ("def foobar", ""),
                ("def foobar", "ensure\n  baz\n"),
            ] {
                for types in [
                    literal.to_string(),
                    format!("{literal}, StandardError"),
                    format!("StandardError, {literal}"),
                ] {
                    let source = format!("{header}\n  foo\nrescue {types}\n  bar\n{ensure}end\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
                    assert_eq!(
                        found[0].message,
                        format!(
                            "Rescuing from `{literal}` will raise a `TypeError` instead of catching the actual exception."
                        )
                    );
                }
            }
        }
    }
    #[test]
    fn reviewed_live_witness_canonical_edit_and_uncertain_holds() {
        let before =
            "begin\n  raise StandardError\nrescue StandardError, nil\n  puts :caught\nend\n";
        let after = before.replace(", nil", "");
        crate::cops::testing::assert_correction(&RescueType, before, &after);
        for s in [
            before.replace("raise StandardError", "foo"),
            before.replace("raise StandardError", "raise Exception"),
            before.replace("StandardError, nil", "nil, StandardError"),
            before.replace("StandardError, nil", "StandardError, effect(), nil"),
            before.replace("puts :caught", "effect()"),
            format!("def raise(*); end\n{before}"),
            format!("StandardError = Exception\n{before}"),
            format!("class Module; def ===(*); false; end; end\n{before}"),
            format!("TracePoint.new(:raise) {{}}.enable\n{before}"),
            format!("{before}p binding\n"),
            format!("{before}p RubyVM::InstructionSequence.of(method(:foo))\n"),
            format!("{before}p ObjectSpace.count_objects\n"),
            format!("{before}File.read(__FILE__)\n"),
            format!("{before}__END__\npayload\n"),
            before.replace(", nil", ", # retained\n nil"),
            before.replace(", nil", ",\n nil"),
        ] {
            let found = crate::cops::testing::correction_offenses(&RescueType, &s);
            assert!(!found.is_empty(), "{s}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{s}");
        }
    }
    #[test]
    fn pinned_valid_and_dynamic_lookalikes() {
        for s in [
            "foo rescue nil",
            "begin; foo; rescue; bar; end",
            "def foobar; foo; rescue NameError; bar; end",
            "def foobar; foo; rescue; bar; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for expr in [
            "NameError",
            "::NameError",
            "Other::Error",
            "error",
            "self.error",
            "*errors",
            ":\"#{name}\"",
            "/error/",
            "%r(#{x})",
            "`never executed`",
            "proc { nil }",
            "(nil)",
            "---1",
            ":''",
            "__ENCODING__",
        ] {
            let s = format!("begin; foo; rescue {expr}; bar; end");
            assert!(offenses(&s).is_empty(), "{s}");
        }
    }
}
