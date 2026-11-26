//! `Lint/RedundantTypeConversion` — redundant fixed-default conversions.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct RedundantTypeConversion;
impl Cop for RedundantTypeConversion {
    fn name(&self) -> &'static str {
        "Lint/RedundantTypeConversion"
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
        let method = call.name().as_slice();
        if !conversion(method)
            || !ast::call_arguments(&call).is_empty()
            || (matches!(method, b"to_h" | b"to_set")
                && (literal_block(&call) || direct_block_body(cx)))
        {
            return;
        }
        let Some(receiver) = call.receiver().map(unwrap_receiver) else {
            return;
        };
        if !literal(&receiver, method)
            && !constructor(&receiver, method)
            && !receiver.as_call_node().is_some_and(|r| {
                !literal_block(&r)
                    && (r.name().as_slice() == method
                        || (method == b"to_s"
                            && matches!(r.name().as_slice(), b"inspect" | b"to_json")))
            })
        {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.reference_line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: format!(
                "Redundant `{}` detected.",
                std::str::from_utf8(method).unwrap()
            ),
            // RuboCop removes from the dot to the closing parenthesis or the selector.
            correction: cx
                .corrections_requested()
                .then(|| {
                    let end = call.closing_loc().unwrap_or(selector);
                    Some(crate::offense::Correction {
                        range: call.call_operator_loc()?.start_offset()..end.end_offset(),
                        replacement: String::new(),
                    })
                })
                .flatten(),
        });
    }
}
fn conversion(method: &[u8]) -> bool {
    matches!(
        method,
        b"to_s"
            | b"to_sym"
            | b"to_i"
            | b"to_f"
            | b"to_d"
            | b"to_r"
            | b"to_c"
            | b"to_a"
            | b"to_h"
            | b"to_set"
    )
}
// RuboCop attaches a sole body expression directly to block/numblock/itblock.
// A multi-statement body has a begin parent instead. Prism adds Statements.
fn direct_block_body(cx: &NodeContext<'_, '_>) -> bool {
    let mut ancestors = cx.ancestors();
    ancestors
        .next()
        .and_then(|n| n.as_statements_node())
        .is_some_and(|body| body.body().len() == 1)
        && ancestors
            .next()
            .is_some_and(|n| matches!(n, Node::BlockNode { .. } | Node::LambdaNode { .. }))
}
fn literal_block(call: &CallNode<'_>) -> bool {
    call.block().is_some_and(|b| b.as_block_node().is_some())
}
// Only parser `begin` wrappers with one child, not explicit kwbegin scopes.
fn unwrap_receiver(mut node: Node<'_>) -> Node<'_> {
    while let Some(group) = node.as_parentheses_node() {
        let Some(body) = group.body().and_then(|b| b.as_statements_node()) else {
            break;
        };
        if body.body().len() != 1 {
            break;
        }
        node = body.body().iter().next().unwrap();
    }
    node
}
fn literal(node: &Node<'_>, method: &[u8]) -> bool {
    let number = ast::negated_numeric_literal(node);
    let node = number.as_ref().unwrap_or(node);
    match method {
        b"to_s" => matches!(
            node,
            Node::StringNode { .. }
                | Node::InterpolatedStringNode { .. }
                | Node::SourceFileNode { .. }
        ),
        b"to_sym" => matches!(
            node,
            Node::SymbolNode { .. } | Node::InterpolatedSymbolNode { .. }
        ),
        b"to_i" => matches!(node, Node::IntegerNode { .. } | Node::SourceLineNode { .. }),
        b"to_f" => node.as_float_node().is_some(),
        b"to_r" => node.as_rational_node().is_some(),
        b"to_c" => node.as_imaginary_node().is_some(),
        b"to_a" => node.as_array_node().is_some(),
        b"to_h" => node.as_hash_node().is_some(),
        _ => false,
    }
}
fn constructor(node: &Node<'_>, method: &[u8]) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if call.is_safe_navigation() {
        return false;
    }
    let name = match method {
        b"to_s" => "String",
        b"to_i" => "Integer",
        b"to_f" => "Float",
        b"to_d" => "BigDecimal",
        b"to_r" => "Rational",
        b"to_c" => "Complex",
        b"to_a" => "Array",
        b"to_h" => "Hash",
        b"to_set" => "Set",
        _ => return false,
    };
    let block = literal_block(&call);
    let class = call
        .receiver()
        .is_some_and(|r| ast::is_global_const(&r, &[name]));
    let matched = if block {
        method == b"to_h"
            && class
            && call.name().as_slice() == b"new"
            && call.arguments().is_none()
            && call
                .block()
                .and_then(|b| b.as_block_node())
                .is_some_and(|b| {
                    b.parameters().is_none_or(|p| {
                        p.as_numbered_parameters_node().is_none()
                            && p.as_it_parameters_node().is_none()
                    })
                })
    } else {
        (class
            && match method {
                b"to_s" => call.name().as_slice() == b"new",
                b"to_a" | b"to_h" | b"to_set" => matches!(call.name().as_slice(), b"new" | b"[]"),
                _ => false,
            })
            || (method != b"to_set"
                && call.name().as_slice() == name.as_bytes()
                && call
                    .receiver()
                    .is_none_or(|r| ast::is_global_const(&r, &["Kernel"])))
    };
    matched && (block || !ast::call_arguments(&call).iter().any(exception_false))
}
fn exception_false(node: &Node<'_>) -> bool {
    let elements: Vec<_> = if let Some(hash) = node.as_keyword_hash_node() {
        hash.elements().iter().collect()
    } else if let Some(hash) = node.as_hash_node() {
        hash.elements().iter().collect()
    } else {
        return false;
    };
    let [element] = elements.as_slice() else {
        return false;
    };
    element.as_assoc_node().is_some_and(|pair| {
        pair.key()
            .as_symbol_node()
            .is_some_and(|s| s.unescaped() == b"exception")
            && pair.value().as_false_node().is_some()
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/redundant_type_conversion_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantTypeConversion")
            .collect()
    }
    #[test]
    fn literals_constructors_and_typed_methods() {
        for (receiver, conversion) in [
            ("'string'", "to_s"),
            ("%Q{#{value}}", "to_s"),
            (":sym", "to_sym"),
            (":\"#{sym}\"", "to_sym"),
            ("42", "to_i"),
            ("42.0", "to_f"),
            ("5r", "to_r"),
            ("5ri", "to_c"),
            ("[]", "to_a"),
            ("{}", "to_h"),
            ("String.new('s')", "to_s"),
            ("::Kernel::Integer('42', 5)", "to_i"),
            ("Float(42)", "to_f"),
            ("BigDecimal(42)", "to_d"),
            ("Rational(3, 8)", "to_r"),
            ("Complex(5, 3)", "to_c"),
            ("::Array[1, 2]", "to_a"),
            ("::Hash.new { |k, v| default }", "to_h"),
            ("Set[1, 2]", "to_set"),
            ("inspect", "to_s"),
            ("foo.to_json", "to_s"),
        ] {
            for dot in [".", "&."] {
                for suffix in ["", "()"] {
                    let s = format!("nil\n  {receiver}{dot}{conversion}{suffix}\n");
                    let found = offenses(&s);
                    assert_eq!(found.len(), 1, "{s}");
                    assert_eq!(
                        (found[0].line, found[0].col, found[0].end_line),
                        (2, (3 + receiver.len() + dot.len()) as u32, 2),
                        "{s}"
                    );
                    assert_eq!(
                        found[0].message,
                        format!("Redundant `{conversion}` detected.")
                    );
                }
            }
        }
    }
    #[test]
    fn all_conversion_chains_and_parentheses() {
        for m in [
            "to_s", "to_sym", "to_i", "to_f", "to_d", "to_r", "to_c", "to_a", "to_h", "to_set",
        ] {
            for receiver in [
                format!("foo.{m}"),
                format!("foo&.{m}(2)"),
                format!("((foo.{m}))"),
            ] {
                let s = format!("{receiver}.{m}.bar");
                assert_eq!(offenses(&s).len(), 1, "{s}");
            }
            for s in [
                m.to_owned(),
                format!("self.{m}"),
                format!("foo.{m}"),
                format!("foo.{m}(2)"),
                format!("{m} = foo; {m}.{m}"),
                format!("foo.to_str.{m}"),
            ] {
                assert!(offenses(&s).is_empty(), "{s}");
            }
        }
    }
    #[test]
    fn arguments_blocks_and_exception_suppression() {
        for s in [
            "'string'.to_s(arg)",
            "[].to_a(&blk)",
            "{}.to_h { |k, v| [v, k] }",
            "Set[1].to_set { _1 }",
            "{}.to_h(&:baz)",
            "Integer(obj, exception: false).to_i",
            "BigDecimal(obj, n, exception: false).to_d",
            "String.new { 's' }.to_s",
            "foo.to_s { 1 }.to_s",
            "-> {}.to_s",
            "proc {}.to_s",
            "Other::String.new.to_s",
            "(1; 2).to_i",
            "(begin; 's'; end).to_s",
            "String&.new.to_s",
            "Hash.new { _1 }.to_h",
            "Hash.new { it }.to_h",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            "'s'.to_s { 1 }",
            "Integer(obj, exception: true).to_i",
            "Integer(obj, exception: false, other: true).to_i",
            "Hash.new.to_h",
            "Hash.new(&blk).to_h",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
    #[test]
    fn immediate_block_body_is_a_real_block_parent_but_sequences_are_not() {
        for expression in ["{}.to_h", "Set.new.to_set", "alpha.to_h.to_h"] {
            for wrap in [
                "foo { %s }",
                "foo { _1; %s }",
                "foo { it; %s }",
                "-> { %s }",
                "proc { %s }",
            ] {
                let s = wrap.replace("%s", expression);
                let want = usize::from(wrap.contains(";"));
                assert_eq!(offenses(&s).len(), want, "{s}");
            }
            let s = format!("foo {{ nil; {expression} }}");
            assert_eq!(offenses(&s).len(), 1, "{s}");
        }
        assert_eq!(offenses("foo { 's'.to_s }").len(), 1);
    }
    #[test]
    fn removes_the_dot_through_the_parentheses() {
        crate::cops::testing::assert_correction(
            &super::RedundantTypeConversion,
            "foo&.to_s(2)&.to_s()\n\"x\".to_s\n",
            "foo&.to_s(2)\n\"x\"\n",
        );
    }
    #[test]
    fn heredoc_selector_and_pseudoliterals() {
        for (s, col) in [
            ("<<~STR.to_s\n  string\nSTR\n", 8),
            ("__FILE__.to_s", 10),
            ("__LINE__.to_i", 10),
            ("(-42).to_i", 7),
            ("(/x/).to_s", 0),
        ] {
            let got = offenses(s);
            if col == 0 {
                assert!(got.is_empty());
            } else {
                assert_eq!(got.len(), 1, "{s}");
                assert_eq!((got[0].line, got[0].col), (1, col));
            }
        }
    }
}
