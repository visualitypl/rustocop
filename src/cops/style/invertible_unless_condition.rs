//! `Style/InvertibleUnlessCondition` — fixed inverse predicates (unsafe, opt-in).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct InvertibleUnlessCondition;
impl Cop for InvertibleUnlessCondition {
    fn name(&self) -> &'static str {
        "Style/InvertibleUnlessCondition"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_unless_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(n) = node.as_unless_node() else {
            return;
        };
        let condition = n.predicate();
        let mut preferred = String::new();
        if invert(&condition, file, &mut preferred).is_none() {
            return;
        }
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: format!(
                "Prefer `if {preferred}` over `unless {}`.",
                String::from_utf8_lossy(condition.location().as_slice())
            ),
            correction: None,
        });
    }
}
/// Decode only diagnostic text; classification and locations use source bytes.
fn source<'pr>(node: &Node<'pr>) -> std::borrow::Cow<'pr, str> {
    String::from_utf8_lossy(node.location().as_slice())
}
fn inverse(method: &[u8]) -> Option<&'static str> {
    Some(match method {
        b"!=" => "==",
        b">" => "<=",
        b"<=" => ">",
        b"<" => ">=",
        b">=" => "<",
        b"!~" => "=~",
        b"zero?" => "nonzero?",
        b"nonzero?" => "zero?",
        b"any?" => "none?",
        b"none?" => "any?",
        b"even?" => "odd?",
        b"odd?" => "even?",
        _ => return None,
    })
}
/// One append-only rendering traversal; no per-node parse or recursive strings.
fn invert(n: &Node<'_>, file: &SourceFile, out: &mut String) -> Option<()> {
    if let Some(p) = n.as_parentheses_node() {
        let statements = p.body()?.as_statements_node()?;
        if statements.body().len() != 1 {
            return None;
        }
        out.push('(');
        invert(&statements.body().iter().next()?, file, out)?;
        out.push(')');
    } else if n.as_and_node().is_some() || n.as_or_node().is_some() {
        let (left, right, op, is_or) = if let Some(n) = n.as_and_node() {
            (n.left(), n.right(), n.operator_loc(), false)
        } else {
            let n = n.as_or_node()?;
            (n.left(), n.right(), n.operator_loc(), true)
        };
        for (i, operand) in [left, right].iter().enumerate() {
            if i == 1 {
                out.push_str(match op.as_slice() {
                    b"&&" => " || ",
                    b"||" => " && ",
                    b"and" => " or ",
                    _ => " and ",
                });
            }
            let wrap = is_or && operand.as_and_node().is_some();
            if wrap {
                out.push('(');
            }
            invert(operand, file, out)?;
            if wrap {
                out.push(')');
            }
        }
    } else {
        let call = n.as_call_node()?;
        // Upstream :send excludes safe-navigation and literal blocks.
        if call.is_safe_navigation() || call.block().is_some_and(|b| b.as_block_node().is_some()) {
            return None;
        }
        let method = call.name().as_slice();
        if method == b"!" {
            out.push_str(&source(&call.receiver()?));
            return Some(());
        }
        let inverse = inverse(method)?;
        let args = ast::call_arguments(&call);
        if method == b"<"
            && let Some(arg) = args.first()
            // Encoding literals are represented as constants upstream, but
            // cannot name a CamelCase class in an inheritance comparison.
            && matches!(arg, Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. })
        {
            let name = if let Some(n) = arg.as_constant_read_node() {
                n.name().as_slice()
            } else {
                arg.as_constant_path_node()?.name()?.as_slice()
            };
            if file.identifier_uppercase_changes(name) {
                return None;
            }
        }
        let receiver = call.receiver();
        if let Some(receiver) = receiver {
            out.push_str(&source(&receiver));
            if !ast::is_operator_method(method) {
                out.push('.');
            }
        }
        if ast::is_operator_method(method) {
            out.push(' ');
        }
        out.push_str(inverse);
        if !args.is_empty() {
            let parens = !ast::is_operator_method(method) && call.opening_loc().is_some();
            out.push(if parens { '(' } else { ' ' });
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&source(arg));
            }
            if parens {
                out.push(')');
            }
        }
    }
    Some(())
}
#[cfg(test)]
mod tests {
    //! RuboCop spec examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;

    fn byte_offenses(source: &[u8]) -> Vec<Offense> {
        let path = std::path::Path::new("a.rb");
        let result = crate::pipeline::lint_one(
            path,
            path,
            Some(source),
            &[&InvertibleUnlessCondition],
            &crate::config::ResolvedConfig::default(),
        );
        assert!(
            result.errors.is_empty() && result.internal_errors.is_empty(),
            "{result:?}"
        );
        result.result.offenses
    }

    #[test]
    fn caseless_lowercase_constants_remain_invertible() {
        for (encoding, name) in [
            ("UTF-8", "Aª".as_bytes()),
            ("UTF-8", "NS::Aª".as_bytes()),
            ("ISO-8859-1", b"A\xaa".as_slice()),
            ("Windows-1252", b"NS::A\xaa".as_slice()),
        ] {
            for (prefix, suffix, tail) in [
                ("  work unless x < ", "\n", ""),
                ("  unless x < ", "\n    work\n  end\n", ""),
                ("  work unless x.odd? || (x < ", ")\n", ")"),
            ] {
                let mut source = format!("# encoding: {encoding}\n{prefix}").into_bytes();
                source.extend_from_slice(name);
                source.extend_from_slice(suffix.as_bytes());
                let found = byte_offenses(&source);
                assert_eq!(found.len(), 1, "{source:?}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                let name = String::from_utf8_lossy(name);
                let (condition, preferred) = if tail.is_empty() {
                    (format!("x < {name}"), format!("x >= {name}"))
                } else {
                    (
                        format!("x.odd? || (x < {name})"),
                        format!("x.even? && (x >= {name})"),
                    )
                };
                assert_eq!(
                    found[0].message,
                    format!("Prefer `if {preferred}` over `unless {condition}`.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn titlecase_constants_are_inheritance_checks() {
        for name in ["Aǅ", "NS::Aǅ", "::Camel::Aǅ", "Aǅ::CamelCase"] {
            for condition in [
                format!("x < {name}"),
                format!("x.odd? && x < {name}"),
                format!("x.odd? || (x < {name})"),
            ] {
                for source in [
                    format!("work unless {condition}\n"),
                    format!("unless {condition}\n  work\nend\n"),
                ] {
                    assert!(byte_offenses(source.as_bytes()).is_empty(), "{source}");
                }
            }
        }
    }

    #[test]
    fn legacy_encoded_class_names_are_not_inverted_even_in_later_operands() {
        for encoding in ["ISO-8859-1", "Windows-1252"] {
            for name in [
                b"\xc9\xe9".as_slice(),
                b"NS::\xc9\xe9",
                b"::Camel::\xc9\xe9",
                b"\xc9\xc9::CamelCase",
            ] {
                for (prefix, suffix) in [
                    ("work unless x < ", "\n"),
                    ("  unless x < ", "\n    work\n  end\n"),
                    ("work unless x < ", " && x.odd?\n"),
                    ("work unless x.odd? && x < ", "\n"),
                    ("work unless x.odd? || (x < ", ")\n"),
                    ("work unless x.odd? and x < ", "\n"),
                    ("work unless x.odd? or x < ", "\n"),
                ] {
                    let mut source = format!("# encoding: {encoding}\n{prefix}").into_bytes();
                    source.extend_from_slice(name);
                    source.extend_from_slice(suffix.as_bytes());
                    assert!(byte_offenses(&source).is_empty(), "{source:?}");
                }
            }
        }
    }

    #[test]
    fn legacy_uppercase_constants_remain_invertible_with_complete_diagnostics() {
        for name in [
            b"\xc9".as_slice(),
            b"\xc9\xc9",
            b"NS::\xc9\xc9",
            b"::Camel::\xc9\xc9",
            b"\xc9\xe9::\xc9\xc9",
        ] {
            let display = String::from_utf8_lossy(name);
            for (prefix, suffix, preferred) in [
                ("work unless x < ", "\n", format!("x >= {display}")),
                (
                    "unless x < ",
                    "\n    work\n  end\n",
                    format!("x >= {display}"),
                ),
                (
                    "work unless x.odd? && x < ",
                    "\n",
                    format!("x.even? || x >= {display}"),
                ),
                (
                    "work unless x.odd? || (x < ",
                    ")\n",
                    format!("x.even? && (x >= {display})"),
                ),
            ] {
                let mut source = format!("# encoding: ISO-8859-1\n  {prefix}").into_bytes();
                source.extend_from_slice(name);
                source.extend_from_slice(suffix.as_bytes());
                let found = byte_offenses(&source);
                assert_eq!(found.len(), 1, "{source:?}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                let condition = format!(
                    "{}{}{}",
                    prefix.split_once("unless ").unwrap().1,
                    display,
                    if suffix.starts_with(')') { ")" } else { "" }
                );
                assert_eq!(
                    found[0].message,
                    format!("Prefer `if {preferred}` over `unless {condition}`.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn legacy_receivers_negations_and_arguments_are_rendered_without_dropping_bytes() {
        for (condition, preferred) in [
            (b"!\xe9".as_slice(), "�"),
            (b"\xe9.odd?", "�.even?"),
            (b"\xe9 != \xc9\xc9", "� == ��"),
            (b"any?(\xe9, \xc9\xc9)", "none?(�, ��)"),
            (b"any? \xe9, \xc9\xc9", "none? �, ��"),
            (b"x < \xc9\xc9 && \xe9.odd?", "x >= �� || �.even?"),
            (b"x.odd? || (\xe9 != \xc9\xc9)", "x.even? && (� == ��)"),
        ] {
            // A legacy character before the reported node must not change its
            // byte-based position. Conversion is for the message only.
            let mut source = b"# encoding: ISO-8859-1\n  \"\xe9\"; work unless ".to_vec();
            source.extend_from_slice(condition);
            source.push(b'\n');
            let found = byte_offenses(&source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 8, 2));
            assert_eq!(
                found[0].message,
                format!(
                    "Prefer `if {preferred}` over `unless {}`.",
                    String::from_utf8_lossy(condition)
                )
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn every_fixed_pair_and_implicit_argument_shape() {
        for (condition, preferred) in [
            ("a > b", "a <= b"),
            ("a <= b", "a > b"),
            ("a < LIMIT", "a >= LIMIT"),
            ("a >= b", "a < b"),
            ("a !~ /x/", "a =~ /x/"),
            ("a.zero?", "a.nonzero?"),
            ("a.nonzero?", "a.zero?"),
            ("a.any?", "a.none?"),
            ("a.none?", "a.any?"),
            ("even?", "odd?"),
            ("odd?", "even?"),
            ("any?(value)", "none?(value)"),
            ("none? value", "any? value"),
            ("not value", "value"),
        ] {
            let source = format!("  unless {condition}\n    work\n  end\n");
            let found =
                crate::cops::testing::correction_offenses(&InvertibleUnlessCondition, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 3));
            assert_eq!(
                found[0].message,
                format!("Prefer `if {preferred}` over `unless {condition}`.")
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn encoding_literal_comparisons_are_not_inheritance_checks() {
        for source in [
            "work unless x < __ENCODING__\n",
            "unless x < __ENCODING__\n  work\nend\n",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&InvertibleUnlessCondition, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(
                found[0].message,
                "Prefer `if x >= __ENCODING__` over `unless x < __ENCODING__`."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn fixed_map_excludes_custom_pairs_equality_blocks_and_safe_navigation() {
        for s in [
            "foo unless array.include?(value)\n",
            "foo unless include? value\n",
            "foo unless array.exclude?(value)\n",
            "foo unless x == y\n",
            "foo unless x =~ /y/\n",
            "foo unless x&.odd?\n",
            "foo unless x.any? { work }\n",
            "foo unless x < ::Space::Foo\n",
            "odd? = true; foo unless odd?\n",
            "foo unless (x != y; x.odd?)\n",
            "foo unless begin x != y end\n",
        ] {
            assert!(
                crate::pipeline::lint_str_with(&[&InvertibleUnlessCondition], s).is_empty(),
                "{s}"
            );
        }
        let s = "foo unless !x || x.even? && x.nonzero?\n".repeat(6000);
        assert_eq!(
            crate::pipeline::lint_str_with(&[&InvertibleUnlessCondition], &s).len(),
            6000
        );
    }
    #[test]
    fn all_pinned_diagnostics_and_unsafe_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/invertible_unless_condition_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 14);
        assert_eq!(
            cases.iter().filter(|c| c["correction"].is_string()).count(),
            7
        );
        for c in cases {
            let found = crate::cops::testing::correction_offenses(
                &InvertibleUnlessCondition,
                c["source"].as_str().unwrap(),
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                c["positions"],
                "spec {}",
                c["spec_line"]
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "spec {}",
                c["spec_line"]
            );
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }
}
