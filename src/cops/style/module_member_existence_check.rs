//! `Style/ModuleMemberExistenceCheck` — prefer direct Module predicates.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};

pub struct ModuleMemberExistenceCheck;
impl Cop for ModuleMemberExistenceCheck {
    fn name(&self) -> &'static str {
        "Style/ModuleMemberExistenceCheck"
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
        let Some((inner, argument, inherit, predicate)) = inclusion(&call) else {
            return;
        };
        let Some(selector) = inner.message_loc() else {
            return;
        };
        let mut replacement = format!(
            "{predicate}({}",
            String::from_utf8_lossy(argument.location().as_slice())
        );
        if let Some(inherit) = &inherit
            && inherit.as_true_node().is_none()
        {
            replacement.push_str(", ");
            replacement.push_str(&String::from_utf8_lossy(inherit.location().as_slice()));
        }
        replacement.push(')');
        let start = selector.start_offset();
        let end = ast::source_end(node);
        let range = start..end;
        // A replacement is UTF-8 text: other bytes cannot be carried over.
        let correction = (cx.corrections_requested()
            && std::str::from_utf8(&file.bytes[range.clone()]).is_ok())
        .then(|| Correction {
            range,
            replacement: replacement.clone(),
        });
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!("Use `{replacement}` instead."),
            correction,
        });
    }
}

fn replacement_method(name: &[u8]) -> Option<&'static str> {
    Some(match name {
        b"class_variables" => "class_variable_defined?",
        b"instance_methods" => "method_defined?",
        b"private_instance_methods" => "private_method_defined?",
        b"protected_instance_methods" => "protected_method_defined?",
        b"public_instance_methods" => "public_method_defined?",
        _ => return None,
    })
}
fn simple(node: &Node<'_>) -> bool {
    !matches!(
        node,
        Node::SplatNode { .. }
            | Node::HashNode { .. }
            | Node::KeywordHashNode { .. }
            | Node::BlockArgumentNode { .. }
    )
}
fn inclusion<'pr>(
    call: &CallNode<'pr>,
) -> Option<(CallNode<'pr>, Node<'pr>, Option<Node<'pr>>, &'static str)> {
    if !matches!(call.name().as_slice(), b"include?" | b"member?") {
        return None;
    }
    let args = ast::call_arguments(call);
    let [argument] = args.as_slice() else {
        return None;
    };
    if !simple(argument) {
        return None;
    }
    let inner = call.receiver()?.as_call_node()?;
    let predicate = replacement_method(inner.name().as_slice())?;
    if inner.block().is_some_and(|b| b.as_block_node().is_some()) {
        return None;
    }
    let inner_args = ast::call_arguments(&inner);
    if inner_args.len() > usize::from(inner.name().as_slice() != b"class_variables")
        || !inner_args.iter().all(simple)
    {
        return None;
    }
    Some((
        inner,
        args.into_iter().next().unwrap(),
        inner_args.into_iter().next(),
        predicate,
    ))
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/module_member_existence_check_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn corrects_as_rubocop() {
        crate::cops::testing::assert_correction(
            &super::ModuleMemberExistenceCheck,
            "x.instance_methods(false).include?(method)\n",
            "x.method_defined?(method, false)\n",
        );
    }
    const METHODS: [(&str, &str); 5] = [
        ("class_variables", "class_variable_defined?"),
        ("instance_methods", "method_defined?"),
        ("private_instance_methods", "private_method_defined?"),
        ("protected_instance_methods", "protected_method_defined?"),
        ("public_instance_methods", "public_method_defined?"),
    ];
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == ModuleMemberExistenceCheck.name())
            .collect()
    }
    #[test]
    fn pinned_no_offenses_and_lookalikes() {
        for (array, _) in METHODS {
            for source in [
                format!("x.{array}(true, false).include?(method)"),
                format!("x.{array}.include?(foo, bar)"),
                format!("x.{array}.include?(*foo)"),
                format!("x.{array}.include?(**foo)"),
                format!("x.{array}.include?(&foo)"),
                format!("x.{array}(*foo).include?(method)"),
                format!("x.{array}({{}}).include?(method)"),
                format!("x.{array}.include?({{}})"),
                format!("x.{array} {{ 1 }}.include?(:foo)"),
                format!("x.{array}.include?"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for source in [
            "x.class_variables(false).include?(:@@foo)",
            "x.constants.include?(:String)",
            "x.methods.include?(:foo)",
            "'x.instance_methods.include?(:foo)' # comment",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn canonical_builtin_symbol_corrections() {
        for (array, predicate) in METHODS {
            let symbol = if array == "class_variables" {
                ":@@foo"
            } else {
                ":size"
            };
            for receiver in ["Array", "::Array"] {
                for join in [".", "&.", "::"] {
                    for membership in ["include?", "member?"] {
                        let before = format!("{receiver}{join}{array}{join}{membership}({symbol})");
                        let after = format!("{receiver}{join}{predicate}({symbol})");
                        assert_correction(&ModuleMemberExistenceCheck, &before, &after);
                    }
                }
            }
            assert_correction(
                &ModuleMemberExistenceCheck,
                &format!("::Array.{array}.include? {symbol}"),
                &format!("::Array.{predicate}({symbol})"),
            );
            if array != "class_variables" {
                assert_correction(
                    &ModuleMemberExistenceCheck,
                    &format!("::Array.{array}.member? :\"si\\x7ae\""),
                    &format!("::Array.{predicate}(:\"si\\x7ae\")"),
                );
                for (inherit, extra) in [("true", ""), ("false", ", false")] {
                    assert_correction(
                        &ModuleMemberExistenceCheck,
                        &format!("p Array.{array}({inherit}).include?({symbol})"),
                        &format!("p Array.{predicate}({symbol}{extra})"),
                    );
                }
            }
        }
    }
}
