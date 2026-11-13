//! `Style/ModuleMemberExistenceCheck` — prefer direct Module predicates.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
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
        let correction = (cx.corrections_requested()
            && safe_shape(&call, &inner, &argument, inherit.as_ref())
            && !file
                .comments()
                .get(file.comments().partition_point(|c| c.end <= start))
                .is_some_and(|c| c.start < end)
            && !file.bytes[range.clone()].contains(&b'\n')
            && std::str::from_utf8(&file.bytes[range.clone()]).is_ok()
            && file
                .module_member_bindings(cx.ancestors().last().unwrap_or(node))
                .closed)
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
const CORE_MODULES: &[&str] = &[
    "Array",
    "Hash",
    "String",
    "Object",
    "Module",
    "Class",
    "Enumerable",
    "Integer",
];
fn core_receiver(call: &CallNode<'_>) -> bool {
    let Some(receiver) = call.receiver() else {
        return false;
    };
    CORE_MODULES
        .iter()
        .any(|name| ast::is_global_const(&receiver, &[name]))
}
fn safe_shape(
    call: &CallNode<'_>,
    inner: &CallNode<'_>,
    argument: &Node<'_>,
    inherit: Option<&Node<'_>>,
) -> bool {
    let Some(symbol) = argument.as_symbol_node() else {
        return false;
    };
    let name = symbol.unescaped();
    // Array membership is Symbol-only; predicates also coerce strings and can
    // raise for invalid class variable names. Restrict edits to known-valid names.
    let name = if inner.name().as_slice() == b"class_variables" {
        let Some(name) = name.strip_prefix(b"@@") else {
            return false;
        };
        name
    } else {
        name
    };
    let valid_name = name
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        && name
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'?' | b'!'));
    valid_name
        && core_receiver(inner)
        && call.block().is_none()
        && inner.block().is_none()
        && (!inner.is_safe_navigation() || call.is_safe_navigation())
        && inherit.is_none_or(|n| n.as_true_node().is_some() || n.as_false_node().is_some())
        && (inner.name().as_slice() != b"class_variables"
            || name.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_'))
}

/// Correction-only closed-expression proof. Whitelist a literal observation
/// file, not arbitrary receivers or user code: no definitions, assignments,
/// bindings, refinements, autoload/eval, includes, callbacks or unknown calls.
/// Built once from the complete main AST; external monkey patches are unknowable.
#[derive(Default)]
pub(crate) struct Bindings {
    closed: bool,
    #[cfg(test)]
    pub(crate) visited: usize,
}
impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut result = Self {
            closed: true,
            #[cfg(test)]
            visited: 0,
        };
        walk(&mut result, root);
        result
    }
}
impl<'pr> NodeVisitor<'pr> for Bindings {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visited += 1;
        }
        let allowed = match node {
            Node::ProgramNode { .. }
            | Node::StatementsNode { .. }
            | Node::ArgumentsNode { .. }
            | Node::ParenthesesNode { .. }
            | Node::ArrayNode { .. }
            | Node::SymbolNode { .. }
            | Node::StringNode { .. }
            | Node::IntegerNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
            | Node::SourceLineNode { .. } => true,
            Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. } => CORE_MODULES
                .iter()
                .any(|name| ast::is_global_const(&node, &[name])),
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if let Some((inner, arg, inherit, _)) = inclusion(&call) {
                    safe_shape(&call, &inner, &arg, inherit.as_ref())
                } else if replacement_method(call.name().as_slice()).is_some() {
                    core_receiver(&call)
                        && call.block().is_none()
                        && ast::call_arguments(&call)
                            .iter()
                            .all(|n| n.as_true_node().is_some() || n.as_false_node().is_some())
                } else {
                    call.name().as_slice() == b"p"
                        && call.receiver().is_none()
                        && call.block().is_none()
                }
            }
            _ => false,
        };
        self.closed &= allowed;
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/module_member_existence_check_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
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
    fn pinned_detection_and_individually_withheld_corrections() {
        for (array, predicate) in METHODS {
            for (suffix, argument) in [
                ("", "method"),
                ("(true)", "method"),
                ("(false)", "method, false"),
                ("(inherit)", "method, inherit"),
            ] {
                if array == "class_variables" && !suffix.is_empty() {
                    continue;
                }
                for (before, col) in [
                    (format!("x.{array}{suffix}.include?(method)"), 3),
                    (format!("x.{array}{suffix}.include? method"), 3),
                    (format!("x.{array}{suffix}.member?(method)"), 3),
                    (format!("x&.{array}{suffix}&.include?(method)"), 4),
                    (format!("{array}{suffix}.include?(method)"), 1),
                ] {
                    let found = offenses(&before);
                    assert_eq!(found.len(), 1, "{before}");
                    assert_eq!(
                        (found[0].line, found[0].col, found[0].end_line),
                        (1, col, 1)
                    );
                    assert_eq!(
                        found[0].message,
                        format!("Use `{predicate}({argument})` instead.")
                    );
                    // Unknown dispatch/name types and reordered evaluations are not safe.
                    assert_correction(&ModuleMemberExistenceCheck, &before, &before);
                }
            }
        }
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
    #[test]
    fn correction_proof_is_cached_and_visits_each_node_once() {
        use crate::cops::testing::{Step, tree_steps};
        let source = "Array.instance_methods.include?(:size)\n".repeat(2000);
        let parsed = ruby_prism::parse(source.as_bytes());
        let root = parsed.node();
        let expected = tree_steps(&source)
            .into_iter()
            .filter(|s| matches!(s, Step::Enter(_)))
            .count();
        let file = SourceFile::new("members.rb".into(), source.as_bytes().to_vec());
        let proof = file.module_member_bindings(&root);
        assert!(proof.closed);
        assert_eq!(proof.visited, expected);
        for _ in 0..2000 {
            assert!(std::ptr::eq(proof, file.module_member_bindings(&root)));
        }
    }

    #[test]
    fn retains_unsafe_offenses_without_edits() {
        for source in [
            "Array.instance_methods.include?('size')", // String != Symbol
            "Array.instance_methods.include?(nil)",
            "Array.instance_methods.include?(name)",
            "Array.instance_methods(inherit).include?(:size)",
            "Array.class_variables.include?(:invalid)", // predicate raises
            "Array.instance_methods.include?(:size) { break 1 }",
            "Array.instance_methods(true # keep\n).include?(:size)",
            "Array.instance_methods(\ntrue).include?(:size)",
            "x.instance_methods.include?(:size)",
            "module R; refine Module do; def method_defined?(x); false; end; end; end; using R\nArray.instance_methods.include?(:size)",
            "class Array; def self.method_defined?(x); false; end; end\nArray.instance_methods.include?(:size)",
            "autoload :Array, 'other'; Array.instance_methods.include?(:size)",
            "eval(code); Array.instance_methods.include?(:size)",
            "Array = custom; Array.instance_methods.include?(:size)",
            "Object.new.method(:define_singleton_method).call(:method_defined?) {}; Array.instance_methods.include?(:size)",
            "Array&.instance_methods.include?(:size)", // nil chain mismatch
        ] {
            let found = correction_offenses(&ModuleMemberExistenceCheck, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
}
