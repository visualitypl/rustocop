//! `Lint/SafeNavigationChain` — ordinary calls immediately after safe navigation.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct SafeNavigationChain;
impl Cop for SafeNavigationChain {
    fn name(&self) -> &'static str {
        "Lint/SafeNavigationChain"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (receiver, dot, end) = if let Some(call) = node.as_call_node() {
            if call.is_safe_navigation()
                || nil_method(call.name().as_slice())
                || matches!(call.name().as_slice(), b"+@" | b"-@")
            {
                return;
            }
            let Some(receiver) = call.receiver() else {
                return;
            };
            (receiver, call.call_operator_loc(), ast::source_end(node))
        } else {
            let Some(parts) = assignment_send(node) else {
                return;
            };
            parts
        };
        if !safe_receiver(&receiver)
            || (node.as_call_node().is_some() && guarded(node, &receiver, cx, file))
        {
            return;
        }
        let start = dot.map_or(receiver.location().end_offset(), |l| l.start_offset());
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not chain ordinary method call after safe navigation operator.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}

// Prism fuses the getter send in compound assignment and the setter send
// in multiple-assignment targets. Neither has the enclosing AND/if as parent.
fn assignment_send<'pr>(
    node: &Node<'pr>,
) -> Option<(Node<'pr>, Option<ruby_prism::Location<'pr>>, usize)> {
    macro_rules! attribute {
        ($access:ident) => {{
            let n = node.$access()?;
            if n.is_safe_navigation() || nil_method(n.read_name().as_slice()) {
                return None;
            }
            Some((
                n.receiver()?,
                n.call_operator_loc(),
                n.message_loc()?.end_offset(),
            ))
        }};
    }
    macro_rules! index {
        ($access:ident) => {{
            let n = node.$access()?;
            if n.is_safe_navigation() {
                return None;
            }
            Some((
                n.receiver()?,
                n.call_operator_loc(),
                n.closing_loc().end_offset(),
            ))
        }};
    }
    match node {
        Node::CallAndWriteNode { .. } => attribute!(as_call_and_write_node),
        Node::CallOrWriteNode { .. } => attribute!(as_call_or_write_node),
        Node::CallOperatorWriteNode { .. } => attribute!(as_call_operator_write_node),
        Node::IndexAndWriteNode { .. } => index!(as_index_and_write_node),
        Node::IndexOrWriteNode { .. } => index!(as_index_or_write_node),
        Node::IndexOperatorWriteNode { .. } => index!(as_index_operator_write_node),
        Node::CallTargetNode { .. } => {
            let n = node.as_call_target_node()?;
            if n.is_safe_navigation() || nil_method(n.name().as_slice()) {
                return None;
            }
            Some((
                n.receiver(),
                Some(n.call_operator_loc()),
                n.message_loc().end_offset(),
            ))
        }
        Node::IndexTargetNode { .. } => {
            let n = node.as_index_target_node()?;
            Some((n.receiver(), None, n.closing_loc().end_offset()))
        }
        _ => None,
    }
}

// Exactly one parser `begin` wrapper is accepted, not arbitrary unwrapping.
// Prism attaches ordinary/numbered/it blocks to their call, so all three
// upstream any_block shapes are already represented by the CallNode.
fn safe_receiver(node: &Node<'_>) -> bool {
    if node.as_call_node().is_some_and(|c| c.is_safe_navigation()) {
        return true;
    }
    node.as_parentheses_node()
        .and_then(|p| p.body())
        .and_then(|b| b.as_statements_node())
        .is_some_and(|s| {
            s.body().len() == 1
                && s.body().iter().next().is_some_and(|n| {
                    n.as_call_node().is_some_and(|c| {
                        c.is_safe_navigation()
                            && !c.block().is_some_and(|b| b.as_block_node().is_some())
                    })
                })
        })
}

// The upstream exceptions inspect ONLY the immediate parser parent. A
// one-statement branch is transparent in Prism; a multi-statement branch,
// parentheses and a call's synthetic block parent remain real boundaries.
fn guarded(
    node: &Node<'_>,
    receiver: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> bool {
    if node
        .as_call_node()
        .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()))
    {
        return false;
    }
    let mut ancestors = cx.ancestors();
    let Some(mut parent) = ancestors.next() else {
        return false;
    };
    if let Some(statements) = parent.as_statements_node() {
        if statements.body().len() != 1 {
            return false;
        }
        let Some(next) = ancestors.next() else {
            return false;
        };
        parent = next;
    }
    if let Some(and) = parent.as_and_node() {
        return same_node(&and.right(), node)
            && and
                .left()
                .as_call_node()
                .and_then(|c| c.receiver())
                .is_some_and(|left| same_structure(&left, receiver, file));
    }
    if let Some(branch) = parent.as_if_node() {
        return branch
            .statements()
            .is_some_and(|s| s.body().len() == 1 && s.body().iter().any(|n| same_node(&n, node)))
            && same_structure(&branch.predicate(), receiver, file);
    }
    if let Some(branch) = parent.as_unless_node() {
        return branch
            .statements()
            .is_some_and(|s| s.body().len() == 1 && s.body().iter().any(|n| same_node(&n, node)))
            && same_structure(&branch.predicate(), receiver, file);
    }
    false
}
fn same_node(a: &Node<'_>, b: &Node<'_>) -> bool {
    a.location().start_offset() == b.location().start_offset()
        && a.location().end_offset() == b.location().end_offset()
}
fn same_structure(a: &Node<'_>, b: &Node<'_>, file: &SourceFile) -> bool {
    // Detection-only literal tokens preserve Ruby Complex#== across numeric
    // component kinds; ordinary fingerprints remain strict for correction proofs.
    ast::fingerprint_for_conditional_scope(a, file)
        .source_buffer_scope_eq(&ast::fingerprint_for_conditional_scope(b, file), file)
}

// NilMethods = the pinned runtime's nil.methods, stdlib to_d and the fixed
// six AllowedMethods. No receiver type inference or user method settings.
fn nil_method(name: &[u8]) -> bool {
    super::redundant_safe_navigation::nil_method(name)
        || matches!(
            name,
            b"to_d" | b"present?" | b"blank?" | b"presence" | b"try" | b"try!" | b"in?"
        )
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/safe_navigation_chain_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/SafeNavigationChain")
            .collect()
    }
    #[test]
    fn accepts_fixed_nil_methods_unary_and_ordinary_lookalikes() {
        for source in [
            "x.foo.bar",
            "x&.foo&.bar",
            "x.foo&.bar",
            "x&.foo || bar",
            "x&.foo && bar",
            "x&.foo | bar",
            "x&.foo & bar",
            "+x&.foo",
            "-x&.foo",
            "x&.foo = bar",
            "x&.foo += bar",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for method in [
            "nil?", "to_d", "present?", "blank?", "presence", "try", "try!", "in?", "to_json",
            "to_yaml",
        ] {
            assert!(offenses(&format!("x&.foo.{method}")).is_empty(), "{method}");
        }
        assert_eq!(offenses("x&.foo.foo").len(), 1); // custom AllowedMethods is not policy
    }
    #[test]
    fn immediate_parent_exceptions_not_general_flow_inference() {
        for source in [
            "x&.foo&.bar && x&.foo.baz",
            "x&.foo ? x&.foo.bar : other",
            "if x&.foo; x&.foo.bar; end",
            "unless x&.foo; x&.foo.bar; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "x&.foo&.bar || x&.foo.baz",
            "x&.foo&.bar && y&.foo.baz",
            "x&.foo ? other : x&.foo.bar",
            "if x&.foo; other; x&.foo.bar; end",
            "if x&.foo; x&.foo.bar {}; end",
            "x&.foo&.bar && (x&.foo.baz)",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn parent_exceptions_compare_complex_numeric_values() {
        for (a, b, equal) in [
            ("1i", "1.0i", true),
            ("1.0i", "1ri", true),
            ("0.5ri", "0.5i", true),
            ("-1i", "-1.0i", true),
            ("0i", "-0.0i", true),
            ("[1i]", "[1.0i]", true),
            ("1i", "2.0i", false),
            ("1ri", "1.5i", false),
            ("-1i", "1.0i", false),
            ("[1i]", "[2.0i]", false),
        ] {
            for (a, b) in [(a, b), (b, a)] {
                for source in [
                    format!("x&.foo({a})&.bar && x&.foo({b}).baz"),
                    format!("x&.foo({a}) ? x&.foo({b}).bar : other"),
                    format!("if x&.foo({a}); x&.foo({b}).bar; end"),
                    format!("unless x&.foo({a}); x&.foo({b}).bar; end"),
                ] {
                    let found = offenses(&source);
                    assert_eq!(found.len(), usize::from(!equal), "{source}");
                    if let Some(offense) = found.first() {
                        assert_eq!(offense.line, 1);
                        assert_eq!(offense.col, source.rfind('.').unwrap() as u32 + 1);
                        assert_eq!(
                            offense.message,
                            "Do not chain ordinary method call after safe navigation operator."
                        );
                        assert!(offense.correction.is_none());
                    }
                }
            }
        }
    }
    #[test]
    fn single_grouping_and_all_three_block_receiver_shapes() {
        for source in [
            "(x&.foo).bar",
            "x&.select { |x| foo(x) }.bar",
            "x&.select { foo(_1) }.bar",
            "x&.select { foo(it) }.bar",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].col, source.rfind('.').unwrap() as u32 + 1);
        }
        for source in [
            "((x&.foo)).bar",
            "(x&.foo; x&.foo).bar",
            "(x&.foo { it }).bar",
            "begin; x&.foo; end.bar",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn many_siblings_and_typed_block_children_keep_exact_findings() {
        let source = "each { x&.foo.bar }\n".repeat(6000);
        let found = crate::cops::testing::correction_offenses(&super::SafeNavigationChain, &source);
        assert_eq!(found.len(), 6000);
        for (index, offense) in found.iter().enumerate() {
            assert_eq!(
                (offense.line, offense.col, offense.end_line),
                (index as u32 + 1, 14, index as u32 + 1)
            );
            assert!(offense.correction.is_none());
        }
    }
    #[test]
    fn nil_snapshot_matches_native_ruby34_public_methods() {
        let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
            return;
        };
        // Authored introspection only; no fixture/source evaluation.
        let out = std::process::Command::new(ruby)
            .args(["-e", "puts nil.methods.sort"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert_eq!(text.lines().count(), 62);
        for name in text.lines() {
            assert!(super::nil_method(name.as_bytes()), "{name}");
            assert!(offenses(&format!("x&.foo.{name}")).is_empty(), "{name}");
        }
    }
    #[test]
    fn ordinary_chain_location_message_and_no_edits() {
        let found = offenses("something\nx&.foo.bar.baz\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 7, 2));
        assert_eq!(
            found[0].message,
            "Do not chain ordinary method call after safe navigation operator."
        );
        assert!(found[0].correction.is_none());
    }
}
