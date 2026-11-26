//! `Lint/RedundantSafeNavigation` — fixed-default, detection-only safe navigation advice.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct RedundantSafeNavigation;
impl Cop for RedundantSafeNavigation {
    fn name(&self) -> &'static str {
        "Lint/RedundantSafeNavigation"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some(or) = node.as_or_node() {
            if let Some(dot) = conversion_default(&or.left(), &or.right(), file) {
                let (line, col) = file.reference_line_col(dot.start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Redundant safe navigation with default literal detected.".into(),
                    line,
                    col,
                    end_line: file.line_of(or.location().end_offset().saturating_sub(1)),
                    correction: None,
                });
            }
            return;
        }
        let dot = if let Some(call) = node.as_call_node() {
            if !call.is_safe_navigation() {
                return;
            }
            let Some(receiver) = call.receiver() else {
                return;
            };
            if !assumed_instance(&receiver, file) && !guaranteed_instance(&receiver) {
                // A literal block is a real synthetic parent in RuboCop, so
                // this csend is not itself the enclosing condition.
                if !allowed(call.name().as_slice())
                    || call.block().is_some_and(|b| b.as_block_node().is_some())
                    || !boolean_parent(node, cx)
                {
                    return;
                }
                let args = ast::call_arguments(&call);
                if call.name().as_slice() == b"respond_to?"
                    && matches!(args.as_slice(), [arg] if arg.as_symbol_node().is_some_and(|s| nil_method(s.unescaped())))
                {
                    return;
                }
            }
            call.call_operator_loc()
        } else {
            // Prism fuses the getter csend of compound attribute assignment.
            // Its parent is the assignment, never an enclosing condition.
            macro_rules! getter {
                ($access:ident) => {{
                    let n = node.$access().unwrap();
                    if !n.is_safe_navigation() {
                        return;
                    }
                    (n.receiver(), n.call_operator_loc())
                }};
            }
            let (receiver, dot) = match node {
                Node::CallAndWriteNode { .. } => getter!(as_call_and_write_node),
                Node::CallOrWriteNode { .. } => getter!(as_call_or_write_node),
                Node::CallOperatorWriteNode { .. } => getter!(as_call_operator_write_node),
                _ => return,
            };
            if !receiver.is_some_and(|r| assumed_instance(&r, file) || guaranteed_instance(&r)) {
                return;
            }
            dot
        };
        let Some(dot) = dot else { return };
        let (line, col) = file.reference_line_col(dot.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Redundant safe navigation detected, use `.` instead.".into(),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}

fn conversion_default<'pr>(
    left: &Node<'pr>,
    right: &Node<'pr>,
    file: &SourceFile,
) -> Option<ruby_prism::Location<'pr>> {
    let call = left.as_call_node()?;
    if !call.is_safe_navigation() || !ast::call_arguments(&call).is_empty() {
        return None;
    }
    let block = call.block().is_some_and(|b| b.as_block_node().is_some());
    let numeric = ast::negated_numeric_literal(right);
    let number = numeric.as_ref().unwrap_or(right);
    let matches = match call.name().as_slice() {
        b"to_h" => right
            .as_hash_node()
            .is_some_and(|n| n.elements().is_empty()),
        b"to_a" if !block => right
            .as_array_node()
            .is_some_and(|n| n.elements().is_empty()),
        b"to_i" if !block => number
            .as_integer_node()
            .is_some_and(|n| n.value().to_u32_digits().1.iter().all(|&d| d == 0)),
        b"to_f" if !block => number.as_float_node().is_some_and(|n| n.value() == 0.0),
        b"to_s" if !block => empty_string(right, file),
        _ => false,
    };
    matches.then(|| call.call_operator_loc()).flatten()
}
fn empty_string(node: &Node<'_>, file: &SourceFile) -> bool {
    // Never fingerprint interpolation expressions: they cannot be a str
    // default. Static concatenations/empty percent strings/heredocs still need
    // the existing exact Prism-to-parser emitted-part projection, not merely
    // StringNode#unescaped (an empty %q() is a dstr in the pinned adapter).
    if node.as_string_node().is_none()
        && !node
            .as_interpolated_string_node()
            .is_some_and(|s| s.parts().iter().all(|p| p.as_string_node().is_some()))
    {
        return false;
    }
    matches!(super::literal_assignment_in_condition::scope_literal(node, file), Some(super::literal_assignment_in_condition::ScopeLiteral::Str(value)) if value.is_empty())
}
fn guaranteed_instance(node: &Node<'_>) -> bool {
    node.as_call_node()
        .is_some_and(|r| !r.is_safe_navigation() && conversion(r.name().as_slice()))
}
fn conversion(name: &[u8]) -> bool {
    matches!(name, b"to_h" | b"to_a" | b"to_i" | b"to_f" | b"to_s")
}
fn allowed(name: &[u8]) -> bool {
    matches!(
        name,
        b"instance_of?" | b"kind_of?" | b"is_a?" | b"eql?" | b"respond_to?" | b"equal?"
    )
}
fn boolean_parent(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    // Parser attaches send arguments directly to the send; Prism inserts an
    // ArgumentsNode. Skip only that wrapper, not grouping or nested calls.
    let parent = cx.parent().and_then(|parent| {
        if parent.as_arguments_node().is_some() {
            cx.ancestors().nth(1)
        } else {
            Some(parent)
        }
    });
    let Some(parent) = parent else { return false };
    // Prism uses conditional nodes for pattern guards; RuboCop has
    // if_guard/unless_guard, which are not conditional? nodes.
    if (parent.as_if_node().is_some() || parent.as_unless_node().is_some())
        && cx
            .ancestors()
            .nth(1)
            .is_some_and(|n| n.as_in_node().is_some())
    {
        return false;
    }
    let predicate = if let Some(n) = parent.as_if_node() {
        Some(n.predicate())
    } else if let Some(n) = parent.as_unless_node() {
        Some(n.predicate())
    } else if let Some(n) = parent.as_while_node() {
        Some(n.predicate())
    } else if let Some(n) = parent.as_until_node() {
        Some(n.predicate())
    } else if let Some(n) = parent.as_case_node() {
        n.predicate()
    } else {
        parent.as_case_match_node().and_then(|n| n.predicate())
    };
    // Only immediate child edges are relevant; identity needs no subtree
    // fingerprint, ancestor walk, local-variable analysis or speculative flow.
    predicate.is_some_and(|p| {
        p.location().start_offset() == node.location().start_offset()
            && p.location().end_offset() == node.location().end_offset()
    }) || parent.as_and_node().is_some()
        || parent.as_or_node().is_some()
        || parent
            .as_call_node()
            .is_some_and(|c| !c.is_safe_navigation() && c.name().as_slice() == b"!")
}
// Pinned RuboCop's NIL_METHODS (nil.methods at class load, including its JSON/YAML extensions).
// This is NOT AdditionalNilMethods, which only affects nondefault inference.
pub(super) fn nil_method(name: &[u8]) -> bool {
    matches!(
        name,
        b"!" | b"!="
            | b"!~"
            | b"&"
            | b"<=>"
            | b"=="
            | b"==="
            | b"=~"
            | b"^"
            | b"__id__"
            | b"__send__"
            | b"class"
            | b"clone"
            | b"define_singleton_method"
            | b"display"
            | b"dup"
            | b"enum_for"
            | b"eql?"
            | b"equal?"
            | b"extend"
            | b"freeze"
            | b"frozen?"
            | b"hash"
            | b"inspect"
            | b"instance_eval"
            | b"instance_exec"
            | b"instance_of?"
            | b"instance_variable_defined?"
            | b"instance_variable_get"
            | b"instance_variable_set"
            | b"instance_variables"
            | b"is_a?"
            | b"itself"
            | b"kind_of?"
            | b"method"
            | b"methods"
            | b"nil?"
            | b"object_id"
            | b"private_methods"
            | b"protected_methods"
            | b"public_method"
            | b"public_methods"
            | b"public_send"
            | b"rationalize"
            | b"remove_instance_variable"
            | b"respond_to?"
            | b"send"
            | b"singleton_class"
            | b"singleton_method"
            | b"singleton_methods"
            | b"tap"
            | b"then"
            | b"to_a"
            | b"to_c"
            | b"to_enum"
            | b"to_f"
            | b"to_h"
            | b"to_i"
            | b"to_json"
            | b"to_r"
            | b"to_s"
            | b"to_yaml"
            | b"yield_self"
            | b"|"
    )
}
fn assumed_instance(node: &Node<'_>, file: &SourceFile) -> bool {
    if node.as_self_node().is_some() || (ast::is_literal(node) && node.as_nil_node().is_none()) {
        return true;
    }
    let name = if let Some(n) = node.as_constant_read_node() {
        Some(n.name())
    } else {
        node.as_constant_path_node().and_then(|n| n.name())
    };
    name.is_some_and(|name| !snake_case(name.as_slice(), file))
}
fn snake_case(name: &[u8], file: &SourceFile) -> bool {
    let name = file.match_pattern_source(name);
    if file.match_pattern_binary() {
        return name
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_');
    }
    std::str::from_utf8(&name).is_ok_and(|s| {
        s.chars()
            .all(|c| c == '_' || crate::source::unicode_uppercase(c) || decimal_digit(c))
    })
}
// Ruby3.4 / Unicode15.0 POSIX digit ranges, all length10 except the five
// contiguous mathematical digit alphabets. No evolving Rust Unicode classes.
fn decimal_digit(c: char) -> bool {
    const STARTS: &[u32] = &[
        0x30, 0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66,
        0xde6, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90,
        0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0,
        0xff10, 0x104a0, 0x10d30, 0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450, 0x114d0,
        0x11650, 0x116c0, 0x11730, 0x118e0, 0x11950, 0x11c50, 0x11d50, 0x11da0, 0x11f50, 0x16a60,
        0x16ac0, 0x16b50, 0x1d7ce, 0x1e140, 0x1e2f0, 0x1e4f0, 0x1e950, 0x1fbf0,
    ];
    let code = c as u32;
    let i = STARTS.partition_point(|&start| start <= code);
    i > 0 && code - STARTS[i - 1] < if STARTS[i - 1] == 0x1d7ce { 50 } else { 10 }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/redundant_safe_navigation_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantSafeNavigation")
            .collect()
    }
    #[test]
    fn decimal_scalar_class_matches_native_ruby34() {
        let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
            return;
        };
        // Authored bounded predicate probe, not execution of lint input.
        let output = std::process::Command::new(ruby).args(["-e", "puts (0..0x10ffff).reject { |n| (0xd800..0xdfff).cover?(n) }.select { |n| /[[:digit:]]/.match?([n].pack('U')) }.join(',')"]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let actual = (0..=0x10ffff)
            .filter_map(char::from_u32)
            .filter(|&c| super::decimal_digit(c))
            .map(|c| (c as u32).to_string())
            .collect::<Vec<_>>()
            .join(",")
            + "\n";
        assert_eq!(output.stdout, actual.as_bytes());
    }

    #[test]
    fn fixed_allowed_methods_only_in_direct_boolean_contexts() {
        for method in [
            "instance_of?",
            "kind_of?",
            "is_a?",
            "eql?",
            "respond_to?",
            "equal?",
        ] {
            let call = format!("foo&.{method}(:bar)");
            for source in [
                format!("if {call}; x; elsif {call}; y; end"),
                format!("unless {call}; x; end"),
                format!("x if {call}"),
                format!("while {call}; x; end"),
                format!("begin; x; end until {call}"),
                format!("{call} && x"),
                format!("x or {call}"),
                format!("!{call}"),
                format!("{call}.!"),
            ] {
                let found = offenses(&source);
                let n = source.matches("&.").count();
                assert_eq!(found.len(), n, "{source}");
                for o in found {
                    assert_eq!(
                        o.message,
                        "Redundant safe navigation detected, use `.` instead."
                    );
                    assert!(o.correction.is_none());
                }
            }
            for source in [
                call.clone(),
                format!("if true; {call}; end"),
                format!("if ({call}); x; end"),
                format!("if consume({call}); x; end"),
                format!("if {call} {{ x }}; y; end"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for source in [
            "if foo&.bar?; x; end",
            "if foo&.respond_to?(:class); x; end",
            "if foo&.respond_to?(:to_a); x; end",
            "foo&.respond_to?(:bar)&.!",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "if foo&.respond_to?(:class, true); x; end",
            "if foo&.respond_to?('class'); x; end",
            "case foo&.respond_to?(:bar); when true; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn explicit_negation_method_arguments() {
        for (source, col) in [
            ("obj.!(foo&.is_a?(X))", 10),
            ("obj.!(1, foo&.is_a?(X))", 13),
            ("obj.!(foo&.is_a?(X), 1)", 10),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(
                found[0].message,
                "Redundant safe navigation detected, use `.` instead."
            );
            assert!(found[0].correction.is_none());
        }
        for source in [
            "obj&.!(foo&.is_a?(X))",
            "obj.!((foo&.is_a?(X)))",
            "obj.!(foo&.respond_to?(:class))",
            "obj.!(foo&.other(X))",
            "obj.!(consume(foo&.is_a?(X)))",
            "obj.!(foo&.is_a?(X) { x })",
            "obj.consume(foo&.is_a?(X))",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn ordinary_conversion_results_but_not_safe_conversion_results() {
        for method in ["to_h", "to_a", "to_i", "to_f", "to_s"] {
            for block in ["", " { |x| x }", " { _1 }", " { it }"] {
                let source = format!("foo.{method}{block}&.example");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (1, source.find("&.").unwrap() as u32 + 1)
                );
                assert!(offenses(&format!("foo&.{method}{block}&.example")).is_empty());
            }
        }
        assert_eq!(offenses("foo.to_s&.respond_to?(:class)").len(), 1);
        assert!(offenses("(foo.to_s)&.example").is_empty());
    }

    #[test]
    fn conversion_with_exact_default_literals() {
        for (method, default) in [
            ("to_h", "{}"),
            ("to_h { |k,v| [k,v] }", "{}"),
            ("to_h { _1 }", "{}"),
            ("to_h { it }", "{}"),
            ("to_a", "[]"),
            ("to_i", "0"),
            ("to_f", "0.0"),
            ("to_s", "''"),
        ] {
            let source = format!("foo&.{method} || {default}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 4, 1));
            assert_eq!(
                found[0].message,
                "Redundant safe navigation with default literal detected."
            );
            assert!(found[0].correction.is_none());
        }
        for source in [
            "foo&.to_h || {a: 1}",
            "foo&.to_a || [1]",
            "foo&.to_i || 1",
            "foo&.to_f || 1.0",
            "foo&.to_s || 'default'",
            "foo&.to_i || 0.0",
            "foo&.to_f || 0",
            "foo&.to_s(x) || ''",
            "foo&.to_h(&b) || {}",
            "(foo&.to_h) || {}",
            "foo&.to_a {} || []",
            "foo&.to_h || ({})",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let found = offenses("foo&.to_h ||\n  {}\n");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 4, 2));
        // Different ranges/messages at the same dot are distinct upstream offenses.
        let found = offenses("self&.to_s || ''");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn literal_self_and_camel_case_receivers() {
        for receiver in [
            "Const",
            "FOO::Const",
            "bar::Const_name",
            "self",
            "'text'",
            "42",
            "[1, 2, 3]",
            "{k: :v}",
        ] {
            let source = format!("{receiver}&.example\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, receiver.len() as u32 + 1, 1)
            );
            assert_eq!(
                found[0].message,
                "Redundant safe navigation detected, use `.` instead."
            );
            assert!(found[0].correction.is_none());
        }
        for source in [
            "CONST&.example",
            "bar::CONST_NAME&.example",
            "nil&.to_i",
            "foo&.example",
            "(self)&.example",
            "foo.bar; foo&.baz",
            "if foo; foo&.bar; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
