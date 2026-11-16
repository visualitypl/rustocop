//! `Style/CombinableDefined` — redundant immediate namespaces in defined? chains.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashSet;
pub struct CombinableDefined;
fn correct(
    node: &Node<'_>,
    terms: &[Node<'_>],
    subjects: &[Node<'_>],
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    // Only the maximal connected chain owns the correction, which removes
    // every redundant term as RuboCop's offenses on the chain do.
    if cx.parent().is_some_and(|p| p.as_and_node().is_some()) || subjects.len() != terms.len() {
        return None;
    }
    let range = node.location().start_offset()..node.location().end_offset();
    let namespaces: HashSet<_> = subjects
        .iter()
        .filter_map(namespace)
        .map(|n| ast::fingerprint(&n))
        .collect();
    let removed: Vec<_> = terms
        .iter()
        .zip(subjects)
        .filter(|(_, subject)| namespaces.contains(&ast::fingerprint(subject)))
        .map(|(term, _)| term.location().start_offset())
        .collect();
    let replacement = prune(node, &removed, file)??;
    Some(Correction { range, replacement })
}
// Prune the syntax tree, not a textual operator list: mixed `and`/`&&` binding
// determines which operator survives when a middle term disappears.
fn prune(node: &Node<'_>, removed: &[usize], file: &SourceFile) -> Option<Option<String>> {
    if let Some(n) = node.as_and_node() {
        let left = n.left();
        let right = n.right();
        let lhs = prune(&left, removed, file)?;
        let rhs = prune(&right, removed, file)?;
        Some(match (lhs, rhs) {
            (Some(lhs), Some(rhs)) => Some(format!(
                "{lhs}{}{rhs}",
                std::str::from_utf8(
                    &file.bytes[left.location().end_offset()..right.location().start_offset()]
                )
                .ok()?
            )),
            (lhs, None) => lhs,
            (None, rhs) => rhs,
        })
    } else if removed.contains(&node.location().start_offset()) {
        Some(None)
    } else {
        Some(Some(
            std::str::from_utf8(node.location().as_slice()).ok()?.into(),
        ))
    }
}
impl Cop for CombinableDefined {
    fn name(&self) -> &'static str {
        "Style/CombinableDefined"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_and_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_and_node().is_none() {
            return;
        }
        let mut terms = Vec::new();
        if !and_terms(node, &mut terms) {
            return;
        }
        let subjects: Vec<_> = terms
            .iter()
            .map(|n| n.as_defined_node().unwrap().value())
            .filter(|n| ast::is_const(n) || n.as_call_node().is_some())
            .collect();
        // Detection follows RuboCop's value-based AST equality. Keep the
        // correction proof's stricter fingerprints above unchanged.
        let namespaces: HashSet<_> = subjects
            .iter()
            .filter_map(namespace)
            .map(|n| ast::fingerprint_with_integer_source_lines(&n, file))
            .collect();
        if !subjects.iter().any(|subject| {
            namespaces.contains(&ast::fingerprint_with_integer_source_lines(subject, file))
        }) {
            return;
        }
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(loc.end_offset() - 1),
            message: "Combine nested `defined?` calls.".into(),
            correction: if cx.corrections_requested() {
                correct(node, &terms, &subjects, cx, file)
            } else {
                None
            },
        });
    }
}
fn and_terms<'pr>(node: &Node<'pr>, out: &mut Vec<Node<'pr>>) -> bool {
    struct Terms<'a, 'pr> {
        out: &'a mut Vec<Node<'pr>>,
        and_stack: Vec<bool>,
        valid: bool,
    }
    impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Terms<'_, 'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            let is_and = node.as_and_node().is_some();
            if self.and_stack.last() == Some(&true) && !is_and {
                if let Some(n) = node.as_defined_node() {
                    self.out.push(n.as_node());
                } else {
                    self.valid = false;
                }
            }
            self.and_stack.push(is_and);
        }
        fn leave(&mut self) {
            self.and_stack.pop();
        }
    }
    let mut terms = Terms {
        out,
        and_stack: Vec::new(),
        valid: true,
    };
    crate::cops::walk::walk(&mut terms, node);
    terms.valid
}
fn namespace<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    if let Some(n) = node.as_constant_path_node() {
        n.parent()
    } else {
        node.as_call_node().and_then(|n| n.receiver())
    }
}

#[cfg(test)]
mod tests {
    //! Ported/adapted RuboCop style/combinable_defined_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn closed_constant_chains_correct_in_one_pass() {
        let prefix = "module Foo; module Bar; Baz=1; end; Quux=2; end; ";
        for (s, after) in [
            ("defined?(Foo) && defined?(Foo::Bar)", "defined?(Foo::Bar)"),
            ("defined?(Foo::Bar) and defined?(Foo)", "defined?(Foo::Bar)"),
            (
                "defined?(::Foo) && defined?(::Foo::Bar)",
                "defined?(::Foo::Bar)",
            ),
            (
                "defined?(Foo) && defined?(Foo::Bar) and defined?(Foo::Bar::Baz)",
                "defined?(Foo::Bar::Baz)",
            ),
            (
                "defined?(Foo) && defined?(Foo::Bar) and defined?(Foo::Quux)",
                "defined?(Foo::Bar) and defined?(Foo::Quux)",
            ),
        ] {
            assert_correction(
                &CombinableDefined,
                &format!("{prefix}{s}"),
                &format!("{prefix}{after}"),
            );
        }
    }
    #[test]
    fn nested_conjunctions_inside_subjects_participate_in_the_upstream_term_filter() {
        let s = "defined?(foo(a && b)) && defined?(foo(a && b).bar)";
        assert!(correction_offenses(&CombinableDefined, s).is_empty());
    }
    #[test]
    fn namespace_shape_and_non_defined_terms_are_not_conflated() {
        for s in [
            "defined?(Foo)",
            "defined?(Foo) && defined?(::Foo)",
            "defined?(Foo) && defined?(Foo::Bar::Baz)",
            "defined?(Foo::Bar) && defined?(Foo::Baz)",
            "defined?(Foo) && !defined?(Foo::Bar)",
            "defined?(Foo) && other && defined?(Foo::Bar)",
            "foo=1; defined?(foo) && defined?(foo.bar)",
        ] {
            assert!(correction_offenses(&CombinableDefined, s).is_empty(), "{s}");
        }
    }
}
