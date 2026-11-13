//! `Style/RedundantInitialize` — fixed pinned policy.
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantInitialize;
impl Cop for RedundantInitialize {
    fn name(&self) -> &'static str {
        "Style/RedundantInitialize"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        if def.name().as_slice() != b"initialize" || def.receiver().is_some() {
            return;
        }
        let loc = def.location();
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        let end = file.initialize_comments(root).end(node);
        if file.has_comment_in_lines(file.line_of(loc.start_offset())..file.line_of(end)) {
            return;
        }
        let mut names = Vec::new();
        if let Some(p) = def.parameters() {
            if !p.optionals().is_empty()
                || p.rest().is_some()
                || !p.posts().is_empty()
                || !p.keywords().is_empty()
                || p.keyword_rest().is_some()
                || p.block().is_some()
            {
                return;
            }
            for arg in p.requireds().iter() {
                let Some(arg) = arg.as_required_parameter_node() else {
                    return;
                };
                names.push(arg.name());
            }
        }
        let empty = def.body().is_none();
        if empty {
            if !names.is_empty() {
                return;
            }
        } else {
            let body = def.body().unwrap();
            let Some(statements) = body.as_statements_node() else {
                return;
            };
            let nodes: Vec<_> = statements.body().iter().collect();
            let [only] = nodes.as_slice() else { return };
            if let Some(s) = only.as_forwarding_super_node() {
                if s.block().is_some() {
                    return;
                }
            } else {
                let Some(s) = only.as_super_node() else {
                    return;
                };
                if s.block().is_some() {
                    return;
                }
                let args: Vec<_> = s
                    .arguments()
                    .map(|a| a.arguments().iter().collect())
                    .unwrap_or_default();
                if args.len() != names.len()
                    || !args.iter().zip(&names).all(|(arg, name)| {
                        arg.as_local_variable_read_node()
                            .is_some_and(|r| r.name().as_slice() == name.as_slice())
                    })
                {
                    return;
                }
            }
        }
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: if empty {
                "Remove unnecessary empty `initialize` method."
            } else {
                "Remove unnecessary `initialize` method."
            }
            .into(),
            correction: None,
        });
    }
}
/// CommentsHelp's sibling/parent end lines, indexed once instead of scanning
/// all siblings for each initializer. Half-open line windows match the pin.
#[derive(Default)]
pub(crate) struct Comments {
    ends: std::collections::HashMap<usize, usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
fn parent_comment_end(node: &Node<'_>) -> usize {
    macro_rules! optional_end {
        ($($access:ident),*) => {$(
            if let Some(parent) = node.$access() {
                return parent.end_keyword_loc().map_or(parent.location().start_offset(), |loc| loc.start_offset());
            }
        )*};
    }
    optional_end!(as_def_node, as_begin_node, as_if_node, as_unless_node);
    macro_rules! required_end {
        ($($access:ident),*) => {$(
            if let Some(parent) = node.$access() {
                return parent.end_keyword_loc().start_offset();
            }
        )*};
    }
    required_end!(
        as_class_node,
        as_module_node,
        as_singleton_class_node,
        as_case_node,
        as_case_match_node,
        as_for_node
    );
    if let Some(parent) = node.as_while_node() {
        return parent
            .closing_loc()
            .map_or(parent.location().start_offset(), |loc| loc.start_offset());
    }
    if let Some(parent) = node.as_until_node() {
        return parent
            .closing_loc()
            .map_or(parent.location().start_offset(), |loc| loc.start_offset());
    }
    if let Some(parent) = node.as_block_node() {
        return parent.closing_loc().start_offset();
    }
    if let Some(parent) = node.as_lambda_node() {
        return parent.closing_loc().start_offset();
    }
    node.location().start_offset()
}
impl Comments {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        struct Build<'pr> {
            facts: Comments,
            stack: Vec<Node<'pr>>,
            next: std::collections::HashMap<usize, usize>,
        }
        impl<'pr> NodeVisitor<'pr> for Build<'pr> {
            fn enter(&mut self, n: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                if let Some(s) = n.as_statements_node() {
                    let mut previous: Option<usize> = None;
                    for child in s.body().iter() {
                        let start = child.location().start_offset();
                        if let Some(previous) = previous {
                            self.next.insert(previous, start);
                        }
                        previous = Some(start);
                    }
                }
                if let Some(d) = n.as_def_node() {
                    let start = d.location().start_offset();
                    let fallback = d
                        .end_keyword_loc()
                        .map_or(d.location().end_offset(), |l| l.start_offset());
                    let end = self.next.get(&start).copied().unwrap_or_else(|| {
                        for (index, parent) in self.stack.iter().enumerate().rev() {
                            if let Some(statements) = parent.as_statements_node() {
                                if statements.body().len() > 1 {
                                    // Explicit begin/end owns the statement group
                                    // in the reference AST. Synthetic groups have
                                    // no end token and use their first line.
                                    if let Some(begin) = index
                                        .checked_sub(1)
                                        .and_then(|i| self.stack[i].as_begin_node())
                                        .filter(|b| b.begin_keyword_loc().is_some())
                                    {
                                        return parent_comment_end(&begin.as_node());
                                    }
                                    return statements.location().start_offset();
                                }
                                continue;
                            }
                            if parent.as_program_node().is_some() {
                                continue;
                            }
                            return parent_comment_end(parent);
                        }
                        fallback
                    });
                    self.facts.ends.insert(start, end);
                }
                self.stack.push(n);
            }
            fn leave(&mut self) {
                self.stack.pop();
            }
        }
        let mut b = Build {
            facts: Self::default(),
            stack: Vec::new(),
            next: std::collections::HashMap::new(),
        };
        walk::walk(&mut b, root);
        b.facts
    }
    fn end(&self, node: &Node<'_>) -> usize {
        self.ends
            .get(&node.location().start_offset())
            .copied()
            .unwrap_or(node.location().end_offset())
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_initialize_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    #[test]
    fn explicit_begin_retains_trailing_comments_after_the_final_initializer() {
        for source in [
            "begin\ndef f;1;end\ndef initialize;end\n# deliberate override\nend\n",
            "begin\ndef f;1;end\ndef initialize;end # deliberate override\nend\n",
            "begin\ndef initialize;end\n# deliberate override\ndef f;1;end\nend\n",
        ] {
            assert!(
                crate::cops::testing::correction_offenses(&RedundantInitialize, source).is_empty(),
                "{source}"
            );
        }
        for source in [
            "begin\ndef f;1;end\ndef initialize;end\nend\n",
            // A synthetic multi-statement group has no enclosing end token
            // in the reference AST, unlike an explicit begin/end container.
            "class C\ndef f;1;end\ndef initialize;end\n# outside its comment window\nend\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&RedundantInitialize, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 1));
            assert_eq!(
                found[0].message,
                "Remove unnecessary empty `initialize` method."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn trailing_and_header_comments_use_reference_sibling_and_parent_boundaries() {
        for source in [
            "class C\n def initialize; end # deliberate override\nend\n",
            "class C\n def initialize; end\n # deliberate override\nend\n",
            "def initialize; end\n# attached to the initializer\ndef next_method; end\n",
        ] {
            assert!(
                crate::cops::testing::correction_offenses(&RedundantInitialize, source).is_empty(),
                "{source}"
            );
        }
        // With no AST parent/sibling the top-level end line is exclusive.
        assert_eq!(
            crate::cops::testing::correction_offenses(
                &RedundantInitialize,
                "def initialize; end # outside its comment window\n"
            )
            .len(),
            1
        );
    }
    #[test]
    fn side_effects_arity_keywords_comments_and_singleton_lookalikes() {
        for source in [
            "def initialize; super { side_effect }; end",
            "def initialize; super() { side_effect }; end",
            "def initialize; super; side_effect; end",
            "def initialize; side_effect; super; end",
            "def initialize(x = side_effect); super; end",
            "def initialize(x: side_effect); super; end",
            "def initialize(x:); super(x: x); end",
            "def initialize(&b); super(&b); end",
            "def initialize(*args); super(*args); end",
            "def initialize(**kwargs); super(**kwargs); end",
            "def initialize(...); super; end",
            "def initialize(a,b); super(b,a); end",
            "def initialize(a); super(nil); end",
            "def initialize(a); end",
            "def self.initialize; end",
            "def initialize\n # deliberate override\nend",
        ] {
            assert!(
                crate::cops::testing::correction_offenses(&RedundantInitialize, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn exact_position_and_all_unsafe_edits_withheld() {
        for (source, message) in [
            (
                "class C\n  def initialize; end\nend",
                "Remove unnecessary empty `initialize` method.",
            ),
            (
                "class C\n  def initialize(a); super(a); end\nend",
                "Remove unnecessary `initialize` method.",
            ),
        ] {
            let found = crate::cops::testing::correction_offenses(&RedundantInitialize, source);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(found[0].message, message);
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn all_pinned_diagnostics_and_messages() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_initialize_pinned.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let found = crate::cops::testing::correction_offenses(
                &RedundantInitialize,
                case["source"].as_str().unwrap(),
            );
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(serde_json::json!(positions), case["positions"], "{case}");
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            assert_eq!(serde_json::json!(messages), case["messages"], "{case}");
        }
    }
}
