//! `Lint/RedundantWithIndex` — redundant index block parameter shapes.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{BlockNode, Node};

pub struct RedundantWithIndex;
impl Cop for RedundantWithIndex {
    fn name(&self) -> &'static str {
        "Lint/RedundantWithIndex"
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
        let each = call.name().as_slice() == b"each_with_index";
        if !each && call.name().as_slice() != b"with_index" {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        if !each
            && !receiver
                .as_call_node()
                .is_some_and(|c| c.receiver().is_some())
        {
            return;
        }
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        if !redundant_parameters(&block, true) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(selector.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            message: if each {
                "Use `each` instead of `each_with_index`."
            } else {
                "Remove redundant `with_index`."
            }
            .into(),
            // RuboCop renames `each_with_index` to `each`, and removes
            // `.with_index(...)`.
            correction: cx
                .corrections_requested()
                .then(|| {
                    if each {
                        return Some(Correction {
                            range: selector.start_offset()..selector.end_offset(),
                            replacement: "each".into(),
                        });
                    }
                    let dot = call.call_operator_loc()?;
                    super::super::style::correction::combine(
                        file,
                        vec![
                            Correction {
                                range: dot.start_offset()..dot.end_offset(),
                                replacement: String::new(),
                            },
                            Correction {
                                range: selector.start_offset()..ast::source_end(node),
                                replacement: String::new(),
                            },
                        ],
                    )
                })
                .flatten(),
        });
    }
}

// Pinned (args (arg _)), (args), numblock arity=1 and itblock predicates.
// This is deliberately NOT unused-variable analysis: even an unread second
// parameter precludes a finding. Shadows, destructuring, rest, optional and
// keyword parameters are distinct RuboCop args children. A trailing comma is
// Prism's ImplicitRestNode, not a RuboCop restarg.
pub(super) fn redundant_parameters(block: &BlockNode<'_>, allow_empty: bool) -> bool {
    let Some(parameters) = block.parameters() else {
        return allow_empty;
    };
    if let Some(numbered) = parameters.as_numbered_parameters_node() {
        return numbered.maximum() == 1;
    }
    if parameters.as_it_parameters_node().is_some() {
        return true;
    }
    let Some(parameters) = parameters.as_block_parameters_node() else {
        return false;
    };
    if !parameters.locals().is_empty() {
        return false;
    }
    let Some(p) = parameters.parameters() else {
        return allow_empty;
    };
    p.optionals().is_empty()
        && p.posts().is_empty()
        && p.keywords().is_empty()
        && p.keyword_rest().is_none()
        && p.block().is_none()
        && p.rest().is_none_or(|r| r.as_implicit_rest_node().is_some())
        && ((allow_empty && p.requireds().is_empty())
            || (p.requireds().len() == 1
                && p.requireds()
                    .iter()
                    .next()
                    .unwrap()
                    .as_required_parameter_node()
                    .is_some()))
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/redundant_with_index_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantWithIndex")
            .collect()
    }
    #[test]
    fn pinned_each_and_chained_examples() {
        for (call, col, message) in [
            (
                "ary.each_with_index",
                7,
                "Use `each` instead of `each_with_index`.",
            ),
            (
                "ary&.each_with_index",
                8,
                "Use `each` instead of `each_with_index`.",
            ),
            ("ary.each.with_index", 12, "Remove redundant `with_index`."),
            (
                "ary.each.with_index(1)",
                12,
                "Remove redundant `with_index`.",
            ),
            (
                "ary.each_with_object([]).with_index",
                28,
                "Remove redundant `with_index`.",
            ),
        ] {
            for block in ["{ |v| v }", "{ do_something }", "{ _1 }", "{ it }"] {
                let source = format!("\n  {call} {block}\n");
                let got = offenses(&source);
                assert_eq!(got.len(), 1, "{source}");
                assert_eq!(
                    (got[0].line, got[0].col, got[0].end_line),
                    (2, col, 2),
                    "{source}"
                );
                assert_eq!(got[0].message, message);
            }
        }
    }
    #[test]
    fn exact_shapes_not_semantic_unused_variable_analysis() {
        for source in [
            "ary.each_with_index { |v, i| v }",
            "ary.each_with_index { _2 }",
            "ary.with_index { |v| v }",
            "with_index { |v| v }",
            "with_index { _1 }",
            "with_index { it }",
            "each_with_index { |v| v }",
            "ary.each_with_index(&blk)",
            "ary.each_with_index",
            "ary.each_with_index { |(v)| v }",
            "ary.each_with_index { |v = 1| v }",
            "ary.each_with_index { |*v| v }",
            "ary.each_with_index { |v, *| v }",
            "ary.each_with_index { |v:| v }",
            "ary.each_with_index { |v; x| v }",
            "ary.each_with_index { |&v| v }",
            "ary.each_with_index { |; x| x }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "ary.each_with_index { || nil }",
            "ary.each_with_index { |v,| v }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
}
