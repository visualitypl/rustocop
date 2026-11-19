//! `Style/RedundantEach` — redundant enumerator iteration; unsafe correction.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node};

// Compound assignment nodes contain a synthetic read-side send. Keep its
// receiver/selector separate from the assignment value, just as RuboCop does.
struct ReadCall<'pr> {
    name: &'pr [u8],
    receiver: Option<Node<'pr>>,
    block: Option<Node<'pr>>,
    selector: Option<Location<'pr>>,
    operator: Option<Location<'pr>>,
}
fn read_call<'pr>(node: &Node<'pr>) -> Option<ReadCall<'pr>> {
    macro_rules! write_call {
        ($access:ident) => {{
            let call = node.$access().unwrap();
            ReadCall {
                name: call.read_name().as_slice(),
                receiver: call.receiver(),
                block: None,
                selector: call.message_loc(),
                operator: call.call_operator_loc(),
            }
        }};
    }
    Some(match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            ReadCall {
                name: call.name().as_slice(),
                receiver: call.receiver(),
                block: call.block(),
                selector: call.message_loc(),
                operator: call.call_operator_loc(),
            }
        }
        Node::CallOrWriteNode { .. } => write_call!(as_call_or_write_node),
        Node::CallAndWriteNode { .. } => write_call!(as_call_and_write_node),
        Node::CallOperatorWriteNode { .. } => write_call!(as_call_operator_write_node),
        _ => return None,
    })
}

pub struct RedundantEach;
impl Cop for RedundantEach {
    fn name(&self) -> &'static str {
        "Style/RedundantEach"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = read_call(node) else {
            return;
        };
        let name = call.name;
        if !matches!(name, b"each" | b"each_with_index" | b"each_with_object")
            || call
                .block
                .as_ref()
                .is_some_and(|n| n.as_block_argument_node().is_some())
        {
            return;
        }
        // Prism attaches a literal block to its call. RuboCop instead wraps
        // that send in a block node, which must not be treated as an enumerator.
        let chained = name == b"each"
            && call.block.is_none()
            && cx.parent().and_then(read_call).is_some_and(|parent| {
                parent.receiver.is_some_and(|n| {
                    n.location().start_offset() == node.location().start_offset()
                        && n.location().end_offset() == node.location().end_offset()
                }) && matches!(
                    parent.name,
                    b"each" | b"each_with_index" | b"each_with_object" | b"reverse_each"
                )
            });
        let previous = call
            .receiver
            .and_then(|n| read_call(&n))
            .is_some_and(|prev| {
                prev.block.is_none()
                    && (prev.name == b"reverse_each"
                        || (name != b"each" && prev.name.starts_with(b"each_")))
            });
        if !chained && !previous {
            return;
        }
        let Some(selector) = call.selector else {
            return;
        };
        let parent_call = if call.block.is_some_and(|n| n.as_block_node().is_some()) {
            None
        } else {
            cx.parent().and_then(read_call)
        };
        let start = if name == b"each" && parent_call.is_none() {
            call.operator
                .map_or(selector.start_offset(), |l| l.start_offset())
        } else {
            selector.start_offset()
        };
        let (line, col) = file.line_col(start);
        let end = if name == b"each" {
            parent_call
                .as_ref()
                .and_then(|p| p.operator.as_ref())
                .map_or(selector.end_offset(), |l| l.end_offset())
        } else {
            selector.end_offset()
        };
        // An `each` goes, and a redundant `each_with_*` next to it (the
        // chained call, else the receiver) becomes `each.with_*`; the other
        // methods become `with_*`.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let selector_edit = |name: &[u8], selector: &Location<'_>| {
                    let replacement = match name {
                        b"each_with_index" => "each.with_index",
                        b"each_with_object" => "each.with_object",
                        _ => return None,
                    };
                    Some((
                        selector.start_offset()..selector.end_offset(),
                        replacement.to_owned(),
                    ))
                };
                let mut edits = Vec::new();
                match name {
                    b"each" => {
                        edits.push((start..end, String::new()));
                        let redundant = if chained {
                            cx.parent().and_then(read_call)
                        } else {
                            read_call(node)
                                .and_then(|c| c.receiver)
                                .and_then(|n| read_call(&n))
                        };
                        if let Some(redundant) = redundant
                            && let Some(selector) = &redundant.selector
                            && let Some(edit) = selector_edit(redundant.name, selector)
                        {
                            edits.push(edit);
                        }
                    }
                    _ => edits.push((
                        selector.start_offset()..selector.end_offset(),
                        if name == b"each_with_index" {
                            "with_index"
                        } else {
                            "with_object"
                        }
                        .into(),
                    )),
                }
                Correction::from_edits(&file.bytes, edits)
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: match name {
                b"each_with_index" => "Use `with_index` to remove redundant `each`.",
                b"each_with_object" => "Use `with_object` to remove redundant `each`.",
                _ => "Remove redundant `each`.",
            }
            .into(),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop redundant_each_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantEach")
            .collect()
    }
    #[test]
    fn pinned_chains_and_exact_ranges() {
        for (s, col, message) in [
            (
                "array.each.each { |v| do_something(v) }",
                7,
                "Remove redundant `each`.",
            ),
            ("array&.each&.each { _1 }", 8, "Remove redundant `each`."),
            (
                "context.reverse_each.each { it }",
                21,
                "Remove redundant `each`.",
            ),
            (
                "context.each.reverse_each { it }",
                9,
                "Remove redundant `each`.",
            ),
            (
                "array.each.each_with_index { |v,i| v }",
                7,
                "Remove redundant `each`.",
            ),
            (
                "context.each_child_node.each_with_index.any? {}",
                25,
                "Use `with_index` to remove redundant `each`.",
            ),
            (
                "scope_stack.reverse_each.each_with_object([]) {}",
                26,
                "Use `with_object` to remove redundant `each`.",
            ),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{s}");
            assert_eq!(found[0].message, message);
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn compound_assignment_reads_and_parent_chains() {
        for op in ["||=", "&&=", "+="] {
            for (chain, col, message) in [
                ("xs.each.each", 4, "Remove redundant `each`."),
                ("xs.each.reverse_each", 4, "Remove redundant `each`."),
                ("xs.reverse_each.each", 16, "Remove redundant `each`."),
                (
                    "xs.reverse_each.each_with_index",
                    17,
                    "Use `with_index` to remove redundant `each`.",
                ),
                (
                    "xs.each_child.each_with_object",
                    15,
                    "Use `with_object` to remove redundant `each`.",
                ),
                ("xs&.each&.each", 5, "Remove redundant `each`."),
            ] {
                let source = format!("xs = [1]\n{chain} {op} []\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, col), "{source}");
                assert_eq!(found[0].message, message, "{source}");
                assert!(found[0].correction.is_none());
            }
            for chain in [
                "xs.each",
                "xs.each.with_index",
                "xs.each_with_index.reverse_each",
                "xs.each { it }.each",
                "xs.reverse_each(&:foo).each",
            ] {
                let source = format!("xs = [1]\n{chain} {op} []\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
            // A write's RHS is not its read-call receiver.
            let source = format!("xs.each {op} xs.each\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn captures_control_flow_lazy_and_enumerator_arguments_remain_unedited() {
        for block in [
            " { |x,y; seen| seen = [x,y] }",
            " { |value| break value }",
            " { |x| return x }",
            " { |x| next x }",
            " { |*xs| xs }",
        ] {
            let s = format!("side_effect().lazy.each.each{block}");
            let found = offenses(&s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none());
        }
        for s in [
            "xs.each(1).each {}",
            "xs.each_child(1,2).each_with_object([]) {}",
            "xs.reverse_each.each.each {}",
        ] {
            let found = offenses(s);
            assert!(!found.is_empty(), "{s}");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
        // The reference range builder crashes on this native-valid argument
        // context (a bare parent's dot is nil); our detector remains useful.
        assert_eq!(offenses("consume(xs.reverse_each.each)").len(), 1);
    }
    #[test]
    fn block_bearing_enumeration_and_nonadjacent_lookalikes() {
        for s in [
            "array.each { |v| v }",
            "array.each",
            "array.each.with_index {}",
            "array.each.with_object([]) {}",
            "node.each_ancestor(:def).each {}",
            "array.each_with_index.reverse_each {}",
            "array.each_char.map(&:to_i).reverse.each_with_index {}",
            "array.reverse_each { _1 }.each { it }",
            "array.each { _1 }.reverse_each { it }",
            "array.each { _1 }.each_with_index { it }",
            "array.each_foo { _1 }.each_with_object([]) { it }",
            "[foo.each].each",
            "array.each(&:foo).each {}",
            "array.each(foo, &:bar).each {}",
            "array.reverse_each(&callback).each {}",
            "array.reverse_each.each(&callback)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
