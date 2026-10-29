//! `Style/TallyMethod` — manual counting advice, unsafe and detection-only.
use super::symbol_proc::{local, parameter_names, single_body};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct TallyMethod;
impl Cop for TallyMethod {
    fn name(&self) -> &'static str {
        "Style/TallyMethod"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let (report, message) = match call.name().as_slice() {
            b"each_with_object" if accumulation(&call) => {
                (call, "Use `tally` instead of `each_with_object`.")
            }
            b"transform_values" if counting(&call) => {
                let Some(group) = call
                    .receiver()
                    .and_then(|n| n.as_call_node())
                    .filter(identity_group)
                else {
                    return;
                };
                (
                    group,
                    "Use `tally` instead of `group_by` and `transform_values`.",
                )
            }
            _ => return,
        };
        let Some(loc) = report.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: message.to_owned(),
            line,
            col,
            end_line: file.last_line_of(loc),
            correction: None,
        });
    }
}
fn no_arguments(call: &CallNode<'_>) -> bool {
    call.arguments().is_none_or(|a| a.arguments().is_empty())
}
fn symbol_block(call: &CallNode<'_>, accept: impl FnOnce(&[u8]) -> bool) -> bool {
    no_arguments(call)
        && call
            .block()
            .and_then(|n| n.as_block_argument_node())
            .and_then(|n| n.expression())
            .and_then(|n| n.as_symbol_node())
            .is_some_and(|n| accept(n.unescaped()))
}
fn identity_group(call: &CallNode<'_>) -> bool {
    if call.name().as_slice() != b"group_by" {
        return false;
    }
    if symbol_block(call, |name| name == b"itself") {
        return true;
    }
    if !no_arguments(call) {
        return false;
    }
    let Some(block) = call.block().and_then(|n| n.as_block_node()) else {
        return false;
    };
    let Some([param]) = parameter_names::<1>(block.parameters(), true) else {
        return false;
    };
    single_body(block.body()).is_some_and(|n| local(&n, param))
}
fn counting(call: &CallNode<'_>) -> bool {
    if symbol_block(call, count_method) {
        return true;
    }
    if !no_arguments(call) {
        return false;
    }
    let Some(block) = call.block().and_then(|n| n.as_block_node()) else {
        return false;
    };
    let Some([param]) = parameter_names::<1>(block.parameters(), true) else {
        return false;
    };
    let Some(body) = single_body(block.body()).and_then(|n| n.as_call_node()) else {
        return false;
    };
    !body.is_safe_navigation()
        && body.block().is_none()
        && no_arguments(&body)
        && count_method(body.name().as_slice())
        && body.receiver().is_some_and(|n| local(&n, param))
}
fn count_method(name: &[u8]) -> bool {
    matches!(name, b"count" | b"size" | b"length")
}
fn accumulation(call: &CallNode<'_>) -> bool {
    let args = ast::call_arguments(call);
    let [arg] = args.as_slice() else { return false };
    let Some(hash) = arg.as_call_node().filter(|c| {
        !c.is_safe_navigation()
            && c.block().is_none()
            && c.name().as_slice() == b"new"
            && c.receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Hash"]))
    }) else {
        return false;
    };
    let values = ast::call_arguments(&hash);
    if values.len() != 1 || !integer(&values[0], 0) {
        return false;
    }
    let Some(block) = call.block().and_then(|n| n.as_block_node()) else {
        return false;
    };
    let Some([elem, memo]) = parameter_names::<2>(block.parameters(), true) else {
        return false;
    };
    let Some(assign) = single_body(block.body()).and_then(|n| n.as_index_operator_write_node())
    else {
        return false;
    };
    if assign.is_safe_navigation()
        || assign.block().is_some()
        || assign.binary_operator().as_slice() != b"+"
        || !integer(&assign.value(), 1)
        || !assign.receiver().is_some_and(|r| local(&r, memo))
    {
        return false;
    }
    let Some(args) = assign.arguments() else {
        return false;
    };
    args.arguments().len() == 1
        && args
            .arguments()
            .iter()
            .next()
            .is_some_and(|n| local(&n, elem))
}
fn integer(node: &Node<'_>, value: i32) -> bool {
    node.as_integer_node().is_some_and(|n| {
        let actual: Option<i32> = n.value().try_into().ok();
        actual == Some(value)
    })
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop tally_method_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/TallyMethod")
            .collect()
    }
    #[test]
    fn each_with_object_reports_selector_without_correction() {
        for (s, col) in [
            (
                "array.each_with_object(Hash.new(0)) { |item, counts| counts[item] += 1 }",
                7,
            ),
            (
                "array&.each_with_object(Hash.new(0)) { |item, counts| counts[item] += 1 }",
                8,
            ),
            ("each_with_object(::Hash.new(0)) { |x,h| h[x] += 1 }", 1),
            ("array.each_with_object(Hash.new(0)) { _2[_1] += 1 }", 7),
            (
                "array.each_with_object(Hash.new(0)) do |item, counts|\n  counts[item] += 1\nend",
                7,
            ),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, col, 1));
            assert_eq!(o[0].message, "Use `tally` instead of `each_with_object`.");
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn group_by_identity_and_transform_counting_matrix() {
        for group in [
            "group_by(&:itself)",
            "group_by { |x| x }",
            "group_by { _1 }",
            "group_by { it }",
        ] {
            for method in ["count", "size", "length"] {
                for transform in [
                    format!("transform_values(&:{method})"),
                    format!("transform_values {{ |v| v.{method} }}"),
                    format!("transform_values {{ _1.{method} }}"),
                    format!("transform_values {{ it.{method} }}"),
                ] {
                    for (receiver, col) in [("array.", 7), ("array&.", 8), ("", 1)] {
                        let s = format!("{receiver}{group}.{transform}");
                        let o = offenses(&s);
                        assert_eq!(o.len(), 1, "{s}");
                        assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, col, 1), "{s}");
                        assert_eq!(
                            o[0].message,
                            "Use `tally` instead of `group_by` and `transform_values`."
                        );
                        assert!(o[0].correction.is_none());
                    }
                }
            }
        }
    }
    #[test]
    fn group_by_lookalikes_and_control_flow() {
        for s in [
            "array.group_by(&:name).transform_values(&:count)",
            "array.group_by(&:itself).transform_values(&:sum)",
            "array.group_by { |x| x.name }.transform_values(&:count)",
            "array.group_by { |x| x }.transform_values { |v| v.count * 2 }",
            "array.group_by { |x| x }.transform_values { |v| v.count(1) }",
            "array.group_by { |x| x }.transform_values { |v| v&.count }",
            "array.group_by { |x| x }.transform_values { |v| v.count { effect } }",
            "array.group_by { |x| next x }.transform_values(&:count)",
            "array.group_by { |x| x }.transform_values { |v| break v.count }",
            "array.group_by { |x| x }.transform_values { |v| return v.count }",
            "array.group_by(foo) { |x| x }.transform_values(&:count)",
            "array.group_by(&:itself).transform_values(foo) { |v| v.count }",
            "array.group_by { |x,y| x }.transform_values(&:count)",
            "array.group_by { _2 }.transform_values(&:count)",
            "array.group_by { it }.transform_values { _2.count }",
            "x = 1; array.group_by { |_x| x }.transform_values(&:count)",
            "array.group_by { |x; y| x }.transform_values(&:count)",
            "array.group_by(&:itself).transform_values { |v; x| v.count }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn accumulation_lookalikes_preserve_hash_and_binding_semantics() {
        for s in [
            "array.each_with_object(Hash.new(0)) { |item, counts| counts[item] += 2 }",
            "array.each_with_object(Hash.new(0)) { |item, counts| counts[item.to_s] += 1 }",
            "array.each_with_object({}) { |item, counts| counts[item] = true }",
            "array.each_with_object(Hash.new(0)) { |x,h| h[x] += 1; puts x }",
            "array.each_with_object(Hash.new(0)) { _2[_1.to_s] += 1 }",
            "array.each_with_object(Hash.new(1)) { |x,h| h[x] += 1 }",
            "array.each_with_object(Other::Hash.new(0)) { |x,h| h[x] += 1 }",
            "array.each_with_object(Hash.new(0)) { |h,x| h[x] += 1 }",
            "array.each_with_object(Hash.new(0)) { |x,h,z| h[x] += 1 }",
            "array.each_with_object(Hash.new(0)) { |x,h| next h[x] += 1 }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
