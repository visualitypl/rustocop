//! `Style/NumberedParametersLimit` — at most one distinct numbered name.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NumberedParametersLimit;
impl Cop for NumberedParametersLimit {
    fn name(&self) -> &'static str {
        "Style/NumberedParametersLimit"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(block) = super::implicit_parameters::block(node) else {
            return;
        };
        if !block
            .parameters
            .is_some_and(|p| p.as_numbered_parameters_node().is_some())
        {
            return;
        }
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        let count = file.numbered_limit_facts(root).counts[&node.location().start_offset()];
        if count > 1 {
            super::implicit_parameters::report(
                self,
                node,
                cx,
                file,
                out,
                &format!("Avoid using more than 1 numbered parameter; {count} detected."),
            );
        }
    }
}

/// This is RuboCop's *syntactic* descendant-name set, not maximum arity or
/// VariableForce binding IDs. Fold nine-bit sets bottom-up once for the entire
/// main AST, including captured reads in ordinary nested blocks. RuboCop's
/// numblock includes its owning call, so finalize ordinary blocks at call-owner
/// exit (including super, receivers and arguments), and arrows at LambdaNode exit.
/// No per-block descendant walks (or reparses), even with many nested scopes.
#[derive(Default)]
pub(crate) struct Facts {
    counts: std::collections::HashMap<usize, u32>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Index {
            facts: Facts,
            stack: Vec<(Option<usize>, u16)>,
        }
        impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Index {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                let mask = node.as_local_variable_read_node().map_or(0, |read| {
                    match read.location().as_slice() {
                        [b'_', n @ b'1'..=b'9'] => 1 << (n - b'1'),
                        _ => 0,
                    }
                });
                let block = if let Some(call) = node.as_call_node() {
                    call.block()
                } else if let Some(call) = node.as_super_node() {
                    call.block()
                } else if let Some(call) = node.as_forwarding_super_node() {
                    call.block().map(|b| b.as_node())
                } else {
                    node.as_lambda_node().map(|_| node)
                };
                let offset = block
                    .filter(|b| {
                        super::implicit_parameters::block(b).is_some_and(|b| {
                            b.parameters
                                .is_some_and(|p| p.as_numbered_parameters_node().is_some())
                        })
                    })
                    .map(|b| b.location().start_offset());
                self.stack.push((offset, mask));
            }
            fn leave(&mut self) {
                let (offset, mask) = self.stack.pop().unwrap();
                if let Some(offset) = offset {
                    self.facts.counts.insert(offset, mask.count_ones());
                }
                if let Some((_, parent)) = self.stack.last_mut() {
                    *parent |= mask;
                }
            }
        }
        let mut index = Index::default();
        crate::cops::walk::walk(&mut index, root);
        index.facts
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/numbered_parameters_limit_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        assert_eq!(ruby_prism::parse(s.as_bytes()).errors().count(), 0, "{s}");
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == NumberedParametersLimit.name())
            .collect()
    }
    #[test]
    fn pinned_distinct_names_not_maximum_or_use_count() {
        for s in [
            "foo { do_something(_1, _2, _3, _4, _5) }",
            "foo do\n do_something(_1, _2, _3, _4, _5)\nend",
        ] {
            let f = offenses(s);
            assert_eq!(f.len(), 1, "{s}");
            assert_eq!((f[0].line, f[0].col), (1, 1));
            assert_eq!(
                f[0].message,
                "Avoid using more than 1 numbered parameter; 5 detected."
            );
            assert!(
                correction_offenses(&NumberedParametersLimit, s)
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
        assert!(!NumberedParametersLimit.safe_fixes());
    }
    #[test]
    fn pinned_negatives_and_it_explicit_comment_literal_lookalikes() {
        for s in [
            "foo { |a,b,c,d,e,f,g| do_something(a,b,c,d,e,f,g) }",
            "foo { do_something(_1) }",
            "foo { do_something(_1, _1) }",
            "foo { do_something(_9) }",
            "_lvar = 42\nfoo { do_something(_2, _lvar) }",
            "foo { it }; foo { |a,b| [a,b] }",
            "foo { '_1 _2' } # _3 _4",
            "foo { obj._1 + obj._2 }",
            "foo { _1 + _1 }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn lambda_proc_nested_captures_and_block_association() {
        for (s, positions) in [
            ("-> { [_2, _9, _2] }", vec![(1, 1)]),
            (
                "lambda { [_1, _2] }; proc { [_2, _3] }; Proc.new { [_8, _9] }",
                vec![(1, 1), (1, 22), (1, 41)],
            ),
            ("foo { inner { [_1, _2] } }", vec![(1, 7)]),
            ("foo { |x| inner { [_1, _2, x] } }", vec![(1, 11)]),
            ("consume foo(1) do\n [_1, _2]\nend", vec![(1, 1)]),
            ("obj&.foo { [_1, _2] }.bar", vec![(1, 1)]),
        ] {
            let f = offenses(s);
            assert_eq!(
                f.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{s}"
            );
        }
    }
    #[test]
    fn numbered_blocks_include_owning_call_receiver_and_arguments() {
        for (source, count) in [
            ("(-> { _1 }).call { _2 }", 2),
            ("foo(-> { _1 }) { [_2, _3] }", 3),
            ("(-> { _1 })&.call(-> { _3 }) { _2 }", 3),
            ("foo(-> { _1 }) { _1 }", 1),
            ("(-> { _1 }).call(-> { _2 }) { |x| x }", 1),
            ("foo(-> { _1 }, -> { _2 }) { it }", 1),
            ("super(-> { _1 }) { _2 }", 2),
            ("super { [_1, _2] }", 2),
            ("super { _1 }", 1),
        ] {
            let found = offenses(source);
            if count == 1 {
                assert!(found.is_empty(), "{source}");
            } else {
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 1), "{source}");
                assert_eq!(
                    found[0].message,
                    format!("Avoid using more than 1 numbered parameter; {count} detected."),
                    "{source}"
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn fixed_ruby27_gate_and_fixed_34_applicability() {
        use crate::cops::policy::{FIXED_RUBY_VERSION, RubyVersion};
        let versions = NumberedParametersLimit.ruby_versions();
        assert_eq!(versions.minimum, Some(RubyVersion(2, 7)));
        assert!(!versions.supports(RubyVersion(2, 6)));
        assert!(versions.supports(FIXED_RUBY_VERSION));
    }
}
