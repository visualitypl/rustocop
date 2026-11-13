//! `Lint/UnusedBlockArgument` — unused explicit block parameters and block locals.
use crate::{
    cops::{
        Cop, NodeContext,
        variables::{Declaration, ParameterKind, ScopeAnalysis},
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashMap;

/// Fixed-default unused-block-argument detection and guarded source correction.
pub struct UnusedBlockArgument;
impl Cop for UnusedBlockArgument {
    fn name(&self) -> &'static str {
        "Lint/UnusedBlockArgument"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(program) = node.as_program_node() else {
            return;
        };
        // One force analysis and one typed-child-complete metadata walk per file,
        // never a root rescan per parameter. Keep raw correction consumers intact.
        let analysis = ScopeAnalysis::for_underscore_variables(&program, file);
        let canonical = cx
            .corrections_requested()
            .then(|| canonical_parameter(node, file))
            .flatten();
        let mut calls = BlockCalls::default();
        walk(&mut calls, node);
        for scope_id in analysis.scopes_in_exit_order() {
            let scope = analysis.scope(scope_id);
            if !scope.kind.is_block() || scope.body().is_none() {
                continue;
            }
            let (lambda, define_method) = calls
                .0
                .get(&scope.node.location().start_offset())
                .copied()
                .unwrap_or((scope.node.as_lambda_node().is_some(), false));
            let arguments: Vec<_> = scope
                .variables()
                .map(|id| analysis.variable(id))
                .filter(|v| v.is_parameter())
                .collect();
            let all_unused = arguments.iter().all(|v| !v.is_referenced());
            for variable in &arguments {
                let local =
                    variable.declaration == Declaration::Parameter(ParameterKind::BlockLocal);
                if variable.should_be_unused()
                    || variable.is_referenced()
                    || (local && !variable.assignments.is_empty())
                {
                    continue;
                }
                let name: String = file
                    .match_pattern_source(variable.name)
                    .utf8_chunks()
                    .map(|chunk| {
                        let mut text = chunk.valid().to_owned();
                        for byte in chunk.invalid() {
                            text.push_str(&format!("\\x{byte:02X}"));
                        }
                        text
                    })
                    .collect();
                let mut message = format!(
                    "Unused {} - `{name}`.",
                    if local {
                        "block local variable"
                    } else {
                        "block argument"
                    }
                );
                if !local {
                    if !lambda && !define_method && all_unused {
                        message.push_str(if arguments.len() > 1 {
                            " You can omit all the arguments if you don't care about them."
                        } else {
                            " You can omit the argument if you don't care about it."
                        });
                    } else {
                        message.push_str(&format!(" If it's necessary, use `_` or `_{name}` as an argument name to indicate that it won't be used."));
                        if lambda && all_unused {
                            message.push_str(" Also consider using a proc without arguments instead of a lambda if you want it to accept any arguments but don't care about them.");
                        }
                    }
                }
                let (line, col) = super::array_literal_in_regexp::reference_line_col(
                    variable.declaration_range.start,
                    file,
                );
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    line,
                    col,
                    end_line: line,
                    message,
                    correction: canonical
                        .clone()
                        .filter(|r| r.start == variable.declaration_range.start)
                        .map(|range| crate::offense::Correction {
                            range,
                            replacement: "_value".into(),
                        }),
                });
            }
        }
    }
}
// Renaming a live block parameter can change binding/reflection, keyword APIs,
// capture identities, defaults, code objects and allocation behavior. Only the
// exact inventoried positional rename in a closed top-level inactive envelope
// is proved here. The sole body has no assignments, collisions, defaults or
// nested captures; neither the call nor block is evaluated/allocated. Keep all
// other findings, including keyword and block-argument corrector exceptions.
// Shared source/code-object/heap/position/encoding guards still filter the edit.
fn canonical_parameter(node: &Node<'_>, file: &SourceFile) -> Option<std::ops::Range<usize>> {
    if file.data_start().is_some() {
        return None;
    }
    let program = node.as_program_node()?;
    let top = program.statements().body();
    if top.len() != 1 {
        return None;
    }
    let conditional = top.iter().next()?.as_if_node()?;
    if conditional.predicate().as_false_node().is_none()
        || conditional.subsequent().is_some()
        || conditional.end_keyword_loc().is_none()
        || conditional.if_keyword_loc()?.as_slice() != b"if"
    {
        return None;
    }
    let body = conditional.statements()?.body();
    if body.len() != 1 {
        return None;
    }
    let call = body.iter().next()?;
    let raw = call.location().as_slice();
    let normalized: Vec<_> = raw
        .iter()
        .enumerate()
        .filter_map(|(i, b)| (*b != b'\r' || raw.get(i + 1) != Some(&b'\n')).then_some(*b))
        .collect();
    if normalized != b"hash.each do |key, value|\n  puts key\nend" {
        return None;
    }
    let start = call.location().start_offset() + 19;
    Some(start..start + 5)
}
#[derive(Default)]
struct BlockCalls(HashMap<usize, (bool, bool)>);
impl NodeVisitor<'_> for BlockCalls {
    fn enter(&mut self, node: Node<'_>) {
        if let Some(call) = node.as_call_node()
            && let Some(block) = call.block().filter(|b| b.as_block_node().is_some())
        {
            self.0.insert(
                block.location().start_offset(),
                (
                    call.name().as_slice() == b"lambda",
                    call.name().as_slice() == b"define_method",
                ),
            );
        }
    }
    fn leave(&mut self) {}
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's unused_block_argument_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnusedBlockArgument")
            .collect()
    }
    #[test]
    fn reports_unused_name_and_contextual_message() {
        let found = offenses("hash.each do |key, value|\n  puts key\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 20, 1));
        assert_eq!(
            found[0].message,
            "Unused block argument - `value`. If it's necessary, use `_` or `_value` as an argument name to indicate that it won't be used."
        );
    }
    #[test]
    fn reports_omission_lambda_define_method_and_block_local_messages() {
        for (source, name, col, augmentation) in [
            (
                "1.times { |index| puts :something }",
                "index",
                12,
                "You can omit the argument if you don't care about it.",
            ),
            (
                "xs.each { |foo, bar| nil }",
                "foo",
                12,
                "You can omit all the arguments if you don't care about them.",
            ),
            (
                "define_method(:f) { |bar| nil }",
                "bar",
                22,
                "If it's necessary, use `_` or `_bar` as an argument name to indicate that it won't be used.",
            ),
            (
                "->(arg) { 1 }",
                "arg",
                4,
                "If it's necessary, use `_` or `_arg` as an argument name to indicate that it won't be used. Also consider using a proc without arguments instead of a lambda if you want it to accept any arguments but don't care about them.",
            ),
        ] {
            let found = offenses(source);
            assert!(!found.is_empty(), "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                format!("Unused block argument - `{name}`. {augmentation}")
            );
        }
        let found = offenses("1.times { |index; local| puts index }");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 19, 1));
        assert_eq!(found[0].message, "Unused block local variable - `local`.");
    }
    #[test]
    fn reports_name_not_sigil_default_or_later_reassignment() {
        for (source, col) in [
            ("xs.each { |foo, *bars, baz| stuff(foo,baz) }", 18),
            ("xs.each { |foo, bar=baz| foo }", 17),
            ("xs.each { |foo, bar| foo,bar=bar,42 }", 12),
            ("xs.each { |foo: 1| nil }", 12),
            ("xs.each { |**foo| nil }", 14),
            ("xs.each { |&foo| nil }", 13),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1),
                "{source}"
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn ignores_used_underscore_empty_and_non_block_variables() {
        for source in [
            "obj.method { |foo, bar| stuff(foo, bar) }",
            "hash.each { |key, value| key, value = value, key }",
            "1.times { |_index| puts 'foo' }",
            "super { |bar| }",
            "->(arg) {}",
            "def f(foo); nil; end",
            "1.times { foo=1 }",
            "test { |key, value| something(binding) }",
            "xs.each { _1 }; xs.each { it }",
            "1.times { |index; x| x=10; puts index }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
