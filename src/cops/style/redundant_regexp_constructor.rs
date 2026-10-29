//! `Style/RedundantRegexpConstructor` — regexp literals wrapped in constructors.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantRegexpConstructor;
impl Cop for RedundantRegexpConstructor {
    fn name(&self) -> &'static str {
        "Style/RedundantRegexpConstructor"
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
        let Some(regexp) = candidate(node) else {
            return;
        };
        let call = node.as_call_node().unwrap();
        let loc = node.location();
        let range = loc.start_offset()..ast::source_end(node);
        let (line, col) = file.line_col(loc.start_offset());
        // A literal is frozen; Regexp.new(literal) is an unfrozen copy. Only
        // discarded top-level values have no observable identity/frozen state.
        let mut ancestors = cx.ancestors();
        let discarded = ancestors
            .next()
            .is_some_and(|n| n.as_statements_node().is_some())
            && ancestors
                .next()
                .is_some_and(|n| n.as_program_node().is_some());
        let correction = if cx.corrections_requested()
            && discarded
            && call.block().is_none()
            && !super::correction::contains_comment(file, range.clone())
            && cx
                .ancestors()
                .next_back()
                .is_some_and(|root| !file.redundant_regexp_safety(root).changed)
        {
            std::str::from_utf8(regexp.location().as_slice())
                .ok()
                .and_then(|text| {
                    super::literal_spelling::safe_edit(
                        file,
                        cx,
                        Correction {
                            range: range.clone(),
                            replacement: text.into(),
                        },
                    )
                })
        } else {
            None
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Remove the redundant `Regexp.{}`.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
            line,
            col,
            end_line: file.line_of(range.end.saturating_sub(1)),
            correction,
        });
    }
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let call = node.as_call_node()?;
    if call.is_safe_navigation()
        || !matches!(call.name().as_slice(), b"new" | b"compile")
        || !ast::is_global_const(&call.receiver()?, &["Regexp"])
    {
        return None;
    }
    let args = call.arguments()?;
    if args.arguments().len() != 1 {
        return None;
    }
    let first = args.arguments().iter().next()?;
    matches!(
        first,
        Node::RegularExpressionNode { .. } | Node::InterpolatedRegularExpressionNode { .. }
    )
    .then_some(first)
}
/// Shared correction-only closed-core proof: one cached complete main-AST walk.
/// No deferred scopes, declarations, interpolation calls, or unknown effects.
#[derive(Default)]
pub(crate) struct Safety {
    pub(super) changed: bool,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        crate::cops::walk::walk(&mut facts, root);
        facts
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Safety {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        if let Some(call) = node.as_call_node() {
            self.changed |= call.block().is_some()
                || !(candidate(&node).is_some()
                    || super::redundant_regexp_argument::trusted_call(&call));
        } else if matches!(
            node,
            Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. }
        ) {
            // An unrelated constant lookup can execute an autoload hook before
            // the candidate. Only the built-in receiver has a closed proof.
            self.changed |= !ast::is_global_const(&node, &["Regexp"]);
        } else {
            self.changed |= !matches!(
                node,
                Node::ProgramNode { .. }
                    | Node::StatementsNode { .. }
                    | Node::ArgumentsNode { .. }
                    | Node::StringNode { .. }
                    | Node::RegularExpressionNode { .. }
                    | Node::InterpolatedRegularExpressionNode { .. }
                    | Node::EmbeddedStatementsNode { .. }
                    | Node::LocalVariableReadNode { .. }
                    | Node::LocalVariableWriteNode { .. }
                    | Node::TrueNode { .. }
                    | Node::FalseNode { .. }
                    | Node::NilNode { .. }
                    | Node::SourceLineNode { .. }
            );
        }
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop redundant_regexp_constructor_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantRegexpConstructor")
            .collect()
    }
    #[test]
    fn pinned_diagnostics_delimiters_and_options() {
        for prefix in ["Regexp", "::Regexp"] {
            for method in ["new", "compile"] {
                for pattern in ["/regexp/", "/regexp/i", r"/\A#{regexp}\z/io", "%r{foo/bar}"] {
                    let s = format!("\n  {prefix}.{method}({pattern})");
                    let found = offenses(&s);
                    assert_eq!(found.len(), 1, "{s}");
                    assert_eq!((found[0].line, found[0].col), (2, 3));
                    assert_eq!(
                        found[0].message,
                        format!("Remove the redundant `Regexp.{method}`.")
                    );
                }
            }
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for s in [
            "Regexp.new('regexp')",
            "Regexp.compile('regexp')",
            "Regexp.new('regexp', Regexp::IGNORECASE)",
            "Regexp.new(/x/, Regexp::IGNORECASE)",
            "Other::Regexp.new(/x/)",
            "object.new(/x/)",
            "Regexp&.new(/x/)",
            "/x/",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn canonical_discarded_constructor_corrections() {
        for pattern in ["/regexp/", "/regexp/i", "%r{foo/bar}", r"/\//n"] {
            for prefix in ["Regexp.new", "::Regexp.compile"] {
                assert_correction(
                    &RedundantRegexpConstructor,
                    &format!("{prefix}({pattern})\n"),
                    &format!("{pattern}\n"),
                );
            }
        }
    }
    #[test]
    fn ambiguous_command_frozen_identity_comments_and_shadowing_withhold_only_edits() {
        for s in [
            "p Regexp.new(/ a/)",
            "Regexp.new(/#{Hook}/)",
            "foo&.bar Regexp.new(/=a/)",
            "p Regexp.new(/ a/).source",
            "x = Regexp.new(/a/)",
            "Regexp.new(/a/).frozen?",
            "Regexp = Custom; Regexp.new(/a/)",
            "Regexp.new( # keep\n /a/)",
        ] {
            let found = correction_offenses(&RedundantRegexpConstructor, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
}
