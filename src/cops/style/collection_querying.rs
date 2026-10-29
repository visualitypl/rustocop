//! `Style/CollectionQuerying` — fixed ActiveSupportExtensionsEnabled=false.
//! Unsafe/detection-only: predicates can short circuit, change equality, or be absent.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct CollectionQuerying;
impl Cop for CollectionQuerying {
    fn name(&self) -> &'static str {
        "Style/CollectionQuerying"
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
        if call.is_safe_navigation()
            || !matches!(
                call.name().as_slice(),
                b"positive?" | b"zero?" | b">" | b"!=" | b"=="
            )
        {
            return;
        }
        let Some(count) = call.receiver().and_then(|r| r.as_call_node()) else {
            return;
        };
        if count.name().as_slice() != b"count" || count.receiver().is_none() {
            return;
        }
        // Literal block wrappers contain an argumentless send; block-pass is
        // an argument in the upstream AST. Positional/keyword operands never match.
        if count.arguments().is_some_and(|a| !a.arguments().is_empty()) {
            return;
        }
        let args = ast::call_arguments(&call);
        let value = match args.as_slice() {
            [] => None,
            [arg] => {
                let Some(int) = arg.as_integer_node() else {
                    return;
                };
                match int.value().try_into() {
                    Ok(0_i32) => Some(0),
                    Ok(1_i32) => Some(1),
                    _ => return,
                }
            }
            _ => return,
        };
        let block = count.block().is_some();
        let replacement = match (call.name().as_slice(), value, block) {
            (b"positive?", None, true) | (b">" | b"!=", Some(0), true) => "any?",
            (b"zero?", None, true) | (b"==", Some(0), true) => "none?",
            (b"==", Some(1), true) => "one?",
            (b"positive?", None, false) | (b">" | b"!=", Some(0), false) => "!empty?",
            (b"zero?", None, false) | (b"==", Some(0), false) => "empty?",
            _ => return,
        };
        // RuboCop's send has a synthetic block parent when this call owns a
        // literal block. Parenthesized receivers are not a direct send edge.
        if replacement == "!empty?"
            && !call.block().is_some_and(|b| b.as_block_node().is_some())
            && cx.parent().is_some_and(|p| direct_dotted_receiver(p, node))
        {
            return;
        }
        let Some(selector) = count.message_loc() else {
            return;
        };
        let start = selector.start_offset();
        let end = ast::source_end(node);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `{replacement}` instead."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}
fn direct_dotted_receiver(parent: &Node<'_>, node: &Node<'_>) -> bool {
    // Prism folds the synthetic attribute sends on assignment LHSs into
    // write/target nodes. Inspect only their receiver, never their RHS. Index
    // assignments have no dotted send and remain eligible, as do grouped calls.
    macro_rules! parts {
        ($access:ident) => {{
            let call = parent.$access().unwrap();
            (call.call_operator_loc().is_some(), call.receiver())
        }};
    }
    let (dotted, receiver) = match parent {
        Node::CallNode { .. } => parts!(as_call_node),
        Node::CallAndWriteNode { .. } => parts!(as_call_and_write_node),
        Node::CallOrWriteNode { .. } => parts!(as_call_or_write_node),
        Node::CallOperatorWriteNode { .. } => parts!(as_call_operator_write_node),
        Node::CallTargetNode { .. } => {
            (true, Some(parent.as_call_target_node().unwrap().receiver()))
        }
        _ => return false,
    };
    dotted
        && receiver.is_some_and(|r| {
            r.location().start_offset() == node.location().start_offset()
                && r.location().end_offset() == node.location().end_offset()
        })
}

#[cfg(test)]
mod tests {
    //! RuboCop collection_querying_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == CollectionQuerying.name())
            .collect()
    }
    #[test]
    fn all_comparators_blocks_and_truthiness_aware_replacements() {
        for (suffix, empty, predicate) in [
            (".positive?", "!empty?", "any?"),
            (" > 0", "!empty?", "any?"),
            (" != 0", "!empty?", "any?"),
            (".zero?", "empty?", "none?"),
            (" == 0", "empty?", "none?"),
        ] {
            for receiver in ["x.", "x&.", "factory().", "x.lazy.", "(1...1)."] {
                for block in [
                    "",
                    "(&:foo?)",
                    "(&block)",
                    "(&nil)",
                    " { |item| item.foo? }",
                    " { _1.foo? }",
                    " { it.foo? }",
                    " { |a,b=1| break true }",
                ] {
                    let s = format!("# header\n  {receiver}count{block}{suffix}");
                    let o = offenses(&s);
                    assert_eq!(o.len(), 1, "{s}");
                    assert_eq!(
                        (o[0].line, o[0].col, o[0].end_line),
                        (2, (3 + receiver.len()) as u32, 2)
                    );
                    assert_eq!(
                        o[0].message,
                        format!(
                            "Use `{}` instead.",
                            if block.is_empty() { empty } else { predicate }
                        )
                    );
                    assert!(
                        correction_offenses(&CollectionQuerying, &s)[0]
                            .correction
                            .is_none()
                    );
                }
            }
        }
        for s in [
            "x.count(&:foo?) == 1",
            "x.count { it.foo? } == 0x1",
            "x.count(&b) == +1",
        ] {
            assert_eq!(offenses(s)[0].message, "Use `one?` instead.");
        }
    }
    #[test]
    fn equality_arguments_navigation_and_non_enumerable_lookalikes() {
        for s in [
            "x.count == 1",
            "x.count > 1",
            "x.count(&:foo?) > 1",
            "x.count(a).positive?",
            "x.count(foo: bar).positive?",
            "x.count({foo: bar}).positive?",
            "x.count(*args).zero?",
            "x.count(a, &b) == 0",
            "x.count(a) { it }.positive?",
            "x.count&.positive?",
            "count.positive?",
            "count(&:foo?).zero?",
            "x.size.positive?",
            "x.length.zero?",
            "x.count",
            "x.count < 1",
            "0 == x.count",
            "x.count == 0.0",
            "x.count == 0r",
            "x.count == value",
            "x.count == 9999999999999999999999999999999",
            "x.count.positive?(a)",
            "x.count.zero?(&b)",
            "x.count.positive?.to_s",
            "x.count.positive?&.to_s",
            "(x.count).positive?",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn chained_parenthesized_negated_and_operator_receivers() {
        for s in [
            "(x.count > 0).to_s",
            "!x.count.positive?",
            "not x.count.positive?",
            "x.count.positive? | true",
            "x.count.zero?.to_s",
            "x.count(&:foo?).positive?.to_s",
            "x.count.positive? { effect }",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        let o = offenses("x\n  .count\n  .positive?");
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 4, 3));
    }
    #[test]
    fn synthetic_assignment_sends_obey_the_direct_dotted_receiver_exemption() {
        for tail in [
            ".foo += 1",
            ".foo ||= 1",
            ".foo &&= 1",
            ".foo, other = values",
        ] {
            let source = format!("x.count.positive?{tail}");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        // An assignment RHS, grouped receiver or [] send is not the same edge.
        // Only the !empty? suggestion is suppressed; block predicates and zero?
        // retain their findings even on these synthetic assignment receivers.
        for source in [
            "other.foo += x.count.positive?",
            "other.foo ||= x.count.positive?",
            "other.foo &&= x.count.positive?",
            "other.foo, a = x.count.positive?, 1",
            "(x.count.positive?).foo += 1",
            "x.count.positive?[0] += 1",
            "x.count.positive?[0], other = values",
            "x.count.zero?.foo += 1",
            "x.count(&:foo?).positive?.foo ||= 1",
            "x.count.positive? { effect }.foo += 1",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].line, 1);
            assert_eq!(found[0].col as usize, source.find("count").unwrap() + 1);
            assert_eq!(found[0].end_line, 1);
            let replacement = if source.contains("zero?") {
                "empty?"
            } else if source.contains("&:foo?") {
                "any?"
            } else {
                "!empty?"
            };
            assert_eq!(found[0].message, format!("Use `{replacement}` instead."));
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn bounded_many_candidates_and_argument_work() {
        assert_eq!(
            offenses(&"x.count(&:foo?).positive?\nx.count.zero?\n".repeat(3000)).len(),
            6000
        );
        let args = "0,".repeat(6000);
        assert!(offenses(&format!("x.count({args}0).zero?")).is_empty());
    }
}
