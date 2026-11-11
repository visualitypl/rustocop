//! `Lint/SelfAssignment` — flags assignments of a value to itself:
//! `foo = foo`, `@a ||= @a`, `Foo = Foo`, `a, b = a, b`, `obj.attr = obj.attr`,
//! `hash['k'] = hash['k']`.
//!
//! Matches RuboCop's semantics with its default `AllowRBSInlineAnnotation:
//! false` (rustocop has no cop parameters): `#: Type` comments do not exempt
//! an assignment. Key assignments whose key is a method call (`hash[foo] =
//! hash[foo]`) are allowed, since the call may return different values.

use ruby_prism::{CallNode, ConstantPathNode, Node};

use crate::{
    cops::{Cop, NodeContext, ast::fingerprint_for_self_assignment},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/SelfAssignment` cop.
pub struct SelfAssignment;

/// Whether `value` reads the variable written by a `name =`-style node of the
/// given variable kind.
fn reads_variable(kind: VariableKind, name: &[u8], value: &Node<'_>, file: &SourceFile) -> bool {
    let read = match (kind, value) {
        (VariableKind::Local, Node::LocalVariableReadNode { .. }) => {
            value.as_local_variable_read_node().unwrap().name()
        }
        (VariableKind::Instance, Node::InstanceVariableReadNode { .. }) => {
            value.as_instance_variable_read_node().unwrap().name()
        }
        (VariableKind::Class, Node::ClassVariableReadNode { .. }) => {
            value.as_class_variable_read_node().unwrap().name()
        }
        (VariableKind::Global, Node::GlobalVariableReadNode { .. }) => {
            value.as_global_variable_read_node().unwrap().name()
        }
        (VariableKind::Constant, Node::ConstantReadNode { .. }) => {
            value.as_constant_read_node().unwrap().name()
        }
        (VariableKind::Local, Node::CallNode { .. })
            if file.match_pattern_composes_identifiers() =>
        {
            let call = value.as_call_node().unwrap();
            if !call.is_variable_call()
                || call.receiver().is_some()
                || call.arguments().is_some()
                || call.block().is_some()
            {
                return false;
            }
            call.name()
        }
        _ => return false,
    };
    same_name(read.as_slice(), name, file)
}

// Detection-only projection: never change shared AST equality/correction proofs.
fn same_name(a: &[u8], b: &[u8], file: &SourceFile) -> bool {
    a == b || file.match_pattern_source(a) == file.match_pattern_source(b)
}

struct Comparison<'a, 'pr> {
    file: &'a SourceFile,
    node: &'a Node<'pr>,
    cx: NodeContext<'a, 'pr>,
    locals: std::cell::OnceCell<Option<std::collections::HashSet<(usize, usize)>>>,
}

impl std::ops::Deref for Comparison<'_, '_> {
    type Target = SourceFile;
    fn deref(&self) -> &SourceFile {
        self.file
    }
}

impl Comparison<'_, '_> {
    fn projected_locals(&self) -> Option<&std::collections::HashSet<(usize, usize)>> {
        self.locals
            .get_or_init(|| {
                self.match_pattern_composes_identifiers()
                    .then(|| {
                        super::non_atomic_file_operation::projected_local_spans(
                            self.node, &self.cx, self.file,
                        )
                    })
                    .flatten()
            })
            .as_ref()
    }
}

fn same_node(a: &Node<'_>, b: &Node<'_>, file: &Comparison<'_, '_>) -> bool {
    let locals = file.projected_locals();
    fingerprint_for_self_assignment(a, file.file, locals).source_buffer_scope_eq(
        &fingerprint_for_self_assignment(b, file.file, locals),
        file.file,
    )
}

fn same_optional_node(
    a: Option<&Node<'_>>,
    b: Option<&Node<'_>>,
    file: &Comparison<'_, '_>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_node(a, b, file),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum VariableKind {
    Local,
    Instance,
    Class,
    Global,
    Constant,
}

/// The variable kind and name written by a simple (`=`, `||=`, `&&=`)
/// variable or constant write.
fn variable_write<'pr>(node: &Node<'pr>) -> Option<(VariableKind, &'pr [u8], Node<'pr>)> {
    use VariableKind::*;
    macro_rules! write {
        ($cast:ident, $kind:expr) => {{
            let write = node.$cast().unwrap();
            Some(($kind, write.name().as_slice(), write.value()))
        }};
    }
    match node {
        Node::LocalVariableWriteNode { .. } => write!(as_local_variable_write_node, Local),
        Node::LocalVariableOrWriteNode { .. } => write!(as_local_variable_or_write_node, Local),
        Node::LocalVariableAndWriteNode { .. } => write!(as_local_variable_and_write_node, Local),
        Node::InstanceVariableWriteNode { .. } => write!(as_instance_variable_write_node, Instance),
        Node::InstanceVariableOrWriteNode { .. } => {
            write!(as_instance_variable_or_write_node, Instance)
        }
        Node::InstanceVariableAndWriteNode { .. } => {
            write!(as_instance_variable_and_write_node, Instance)
        }
        Node::ClassVariableWriteNode { .. } => write!(as_class_variable_write_node, Class),
        Node::ClassVariableOrWriteNode { .. } => write!(as_class_variable_or_write_node, Class),
        Node::ClassVariableAndWriteNode { .. } => write!(as_class_variable_and_write_node, Class),
        Node::GlobalVariableWriteNode { .. } => write!(as_global_variable_write_node, Global),
        Node::GlobalVariableOrWriteNode { .. } => write!(as_global_variable_or_write_node, Global),
        Node::GlobalVariableAndWriteNode { .. } => {
            write!(as_global_variable_and_write_node, Global)
        }
        Node::ConstantWriteNode { .. } => write!(as_constant_write_node, Constant),
        Node::ConstantOrWriteNode { .. } => write!(as_constant_or_write_node, Constant),
        Node::ConstantAndWriteNode { .. } => write!(as_constant_and_write_node, Constant),
        _ => None,
    }
}

/// The target path and value of a `A::B =`, `||=` or `&&=` write.
fn constant_path_write<'pr>(node: &Node<'pr>) -> Option<(ConstantPathNode<'pr>, Node<'pr>)> {
    if let Some(write) = node.as_constant_path_write_node() {
        Some((write.target(), write.value()))
    } else if let Some(write) = node.as_constant_path_or_write_node() {
        Some((write.target(), write.value()))
    } else {
        node.as_constant_path_and_write_node()
            .map(|write| (write.target(), write.value()))
    }
}

/// `A::B = A::B`: same namespace (both `::` or structurally equal) and name.
fn same_constant_path(
    target: &ConstantPathNode<'_>,
    value: &Node<'_>,
    file: &Comparison<'_, '_>,
) -> bool {
    let Some(value) = value.as_constant_path_node() else {
        return false;
    };
    let same_name = match (target.name(), value.name()) {
        (Some(a), Some(b)) => same_name(a.as_slice(), b.as_slice(), file),
        _ => false,
    };
    same_name && same_optional_node(target.parent().as_ref(), value.parent().as_ref(), file)
}

/// `a, b = a, b` / `a, b = [a, b]`: every target is a variable re-assigned
/// from the read at the same position.
fn multiple_self_assignment(node: &Node<'_>, file: &Comparison<'_, '_>) -> bool {
    let Some(write) = node.as_multi_write_node() else {
        return false;
    };
    let Some(values) = write.value().as_array_node() else {
        return false;
    };
    let mut targets: Vec<Node<'_>> = write.lefts().iter().collect();
    if let Some(rest) = write.rest() {
        // A trailing comma (`a, = ...`) leaves no target in RuboCop's AST.
        if rest.as_implicit_rest_node().is_none() {
            targets.push(rest);
        }
    }
    targets.extend(write.rights().iter());
    let values: Vec<Node<'_>> = values.elements().iter().collect();
    targets.len() == values.len()
        && targets
            .iter()
            .zip(values.iter())
            .all(|(target, value)| match target_variable(target) {
                Some((kind, name)) => reads_variable(kind, name, value, file),
                None => false,
            })
}

/// The variable kind and name of a multiple-assignment target.
fn target_variable<'pr>(node: &Node<'pr>) -> Option<(VariableKind, &'pr [u8])> {
    match node {
        Node::LocalVariableTargetNode { .. } => Some((
            VariableKind::Local,
            node.as_local_variable_target_node()
                .unwrap()
                .name()
                .as_slice(),
        )),
        Node::InstanceVariableTargetNode { .. } => Some((
            VariableKind::Instance,
            node.as_instance_variable_target_node()
                .unwrap()
                .name()
                .as_slice(),
        )),
        Node::ClassVariableTargetNode { .. } => Some((
            VariableKind::Class,
            node.as_class_variable_target_node()
                .unwrap()
                .name()
                .as_slice(),
        )),
        Node::GlobalVariableTargetNode { .. } => Some((
            VariableKind::Global,
            node.as_global_variable_target_node()
                .unwrap()
                .name()
                .as_slice(),
        )),
        _ => None,
    }
}

/// The arguments of a call, as a list.
fn call_arguments<'pr>(call: &CallNode<'pr>) -> Vec<Node<'pr>> {
    call.arguments()
        .map(|arguments| arguments.arguments().iter().collect())
        .unwrap_or_default()
}

/// Whether `value` is a plain call (no block) named `name` on a receiver
/// structurally equal to `receiver`, with arguments equal to `arguments`.
fn reads_same(
    value: &Node<'_>,
    name: &[u8],
    receiver: Option<&Node<'_>>,
    arguments: &[Node<'_>],
    file: &Comparison<'_, '_>,
) -> Option<bool> {
    let call = value.as_call_node()?;
    let value_arguments = call_arguments(&call);
    Some(
        call.block().is_none()
            && same_name(call.name().as_slice(), name, file)
            && same_optional_node(receiver, call.receiver().as_ref(), file)
            && value_arguments.len() == arguments.len()
            && arguments
                .iter()
                .zip(value_arguments.iter())
                .all(|(a, b)| same_node(a, b, file)),
    )
}

/// A call-like key may return different values on each call.
fn has_call_key(arguments: &[Node<'_>], file: &Comparison<'_, '_>) -> bool {
    arguments.iter().any(|argument| {
        argument.as_call_node().is_some_and(|call| {
            if call.is_variable_call()
                && file.projected_locals().is_some_and(|locals| {
                    let loc = argument.location();
                    locals.contains(&(loc.start_offset(), loc.end_offset()))
                })
            {
                return false;
            }
            // A literal block wraps its send in RuboCop's AST: it is not a
            // call_type? key, unlike an ordinary send/csend or block pass.
            call.block()
                .is_none_or(|block| block.as_block_node().is_none())
        })
    })
}

/// `foo.bar ||= foo.bar` / `foo.bar &&= foo.bar`.
fn attribute_or_and_self_assignment(node: &Node<'_>, file: &Comparison<'_, '_>) -> bool {
    let (receiver, name, safe_navigation, value) = if let Some(write) = node.as_call_or_write_node()
    {
        (
            write.receiver(),
            write.read_name(),
            write.is_safe_navigation(),
            write.value(),
        )
    } else if let Some(write) = node.as_call_and_write_node() {
        (
            write.receiver(),
            write.read_name(),
            write.is_safe_navigation(),
            write.value(),
        )
    } else {
        return false;
    };
    value
        .as_call_node()
        .is_some_and(|call| call.is_safe_navigation() == safe_navigation)
        && reads_same(&value, name.as_slice(), receiver.as_ref(), &[], file).unwrap_or(false)
}

/// `hash['k'] ||= hash['k']` / `hash['k'] &&= hash['k']`.
fn index_or_and_self_assignment(node: &Node<'_>, file: &Comparison<'_, '_>) -> bool {
    let (receiver, arguments, block, safe_navigation, value) =
        if let Some(write) = node.as_index_or_write_node() {
            (
                write.receiver(),
                write.arguments(),
                write.block(),
                write.is_safe_navigation(),
                write.value(),
            )
        } else if let Some(write) = node.as_index_and_write_node() {
            (
                write.receiver(),
                write.arguments(),
                write.block(),
                write.is_safe_navigation(),
                write.value(),
            )
        } else {
            return false;
        };
    if block.is_some() {
        return false;
    }
    let arguments: Vec<Node<'_>> = arguments
        .map(|arguments| arguments.arguments().iter().collect())
        .unwrap_or_default();
    !has_call_key(&arguments, file)
        && value
            .as_call_node()
            .is_some_and(|call| call.is_safe_navigation() == safe_navigation)
        && reads_same(&value, b"[]", receiver.as_ref(), &arguments, file).unwrap_or(false)
}

/// `foo['k'] = foo['k']` and `foo.bar = foo.bar` (including `&.`).
fn call_self_assignment(node: &Node<'_>, file: &Comparison<'_, '_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    let name = call.name().as_slice();
    // Filter on the name before collecting arguments: this runs for every
    // call in the file.
    if name == b"[]=" {
        let arguments = call_arguments(&call);
        let Some((value, keys)) = arguments.split_last() else {
            return false;
        };
        return !has_call_key(keys, file)
            && reads_same(value, b"[]", call.receiver().as_ref(), keys, file).unwrap_or(false);
    }
    let is_assignment_method =
        name.ends_with(b"=") && !matches!(name, b"==" | b"!=" | b"<=" | b">=" | b"===");
    if !is_assignment_method {
        return false;
    }
    let arguments = call_arguments(&call);
    let [value] = arguments.as_slice() else {
        return false;
    };
    reads_same(
        value,
        &name[..name.len() - 1],
        call.receiver().as_ref(),
        &[],
        file,
    )
    .unwrap_or(false)
}

impl Cop for SelfAssignment {
    fn name(&self) -> &'static str {
        "Lint/SelfAssignment"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let file = &Comparison {
            file,
            node,
            cx: *cx,
            locals: std::cell::OnceCell::new(),
        };
        let self_assigned = if let Some((kind, name, value)) = variable_write(node) {
            reads_variable(kind, name, &value, file)
        } else if let Some((target, value)) = constant_path_write(node) {
            same_constant_path(&target, &value, file)
        } else {
            multiple_self_assignment(node, file)
                || attribute_or_and_self_assignment(node, file)
                || index_or_and_self_assignment(node, file)
                || call_self_assignment(node, file)
        };
        if !self_assigned {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Self-assignment detected.".to_string(),
            line,
            end_line: file.line_of(crate::cops::ast::source_end(node).saturating_sub(1)),
            col,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/self_assignment_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/SelfAssignment")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    fn assert_flags(source: &str) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "should flag {source:?}");
        assert_eq!((found[0].line, found[0].col), (1, 1), "{source:?}");
        assert_eq!(found[0].message, "Self-assignment detected.");
    }

    fn assert_accepts(source: &str) {
        assert!(offenses(source).is_empty(), "should not flag {source:?}");
    }

    #[test]
    fn mixed_mac_variable_names_match_only_in_the_reference_buffer() {
        for body in ["@é = @e\u{301}", "é = e\u{301}", "Foo::É = Foo::E\u{301}"] {
            let found = offenses(&format!("# coding: UTF8-MAC\n{body}\n"));
            assert_eq!(found.len(), 1, "{body}");
            assert_eq!((found[0].line, found[0].col), (2, 1));
            assert_eq!(found[0].message, "Self-assignment detected.");
            assert_accepts(&format!("# coding: UTF-8\n{body}\n"));
        }
    }

    #[test]
    fn projected_locals_in_receivers_and_keys_keep_lexical_scope_and_call_exemptions() {
        for (body, col) in [
            ("é = obj; é.x = e\u{301}.x", 10),
            ("é = 1; obj[e\u{301}] = obj[é]", 8),
            ("é = 1; obj[e\u{301}] ||= obj[é]", 8),
        ] {
            assert_eq!(
                positions(&format!("# coding: UTF8-MAC\n{body}\n")),
                vec![(2, col)],
                "{body}"
            );
            assert_accepts(&format!("# coding: UTF-8\n{body}\n"));
        }
        for body in [
            "é = obj; def f; obj[e\u{301}] = obj[é]; end",
            "obj[e\u{301}] = obj[(é = 1; é)]",
            "obj[e\u{301}] = obj[e\u{301}]",
        ] {
            assert_accepts(&format!("# coding: UTF8-MAC\n{body}\n"));
        }
    }

    #[test]
    fn flags_variable_self_assignment() {
        for source in ["foo = foo", "@foo = @foo", "@@foo = @@foo", "$foo = $foo"] {
            assert_flags(source);
        }
    }

    #[test]
    fn accepts_variable_assignment() {
        for source in ["foo = bar", "@foo = @bar", "@@foo = @@bar", "$foo = $bar"] {
            assert_accepts(source);
        }
    }

    #[test]
    fn flags_constant_self_assignment() {
        assert_flags("Foo = Foo");
        assert_flags("Foo::Bar = Foo::Bar");
        assert_flags("::Foo = ::Foo");
    }

    #[test]
    fn accepts_constant_from_another_scope() {
        assert_accepts("Foo = ::Foo");
        assert_accepts("Foo ||= ::Foo");
        assert_accepts("Foo::Bar = Baz::Bar");
        assert_accepts("Foo::Bar = Bar");
    }

    #[test]
    fn flags_multiple_self_assignment() {
        assert_flags("foo, bar = foo, bar");
        assert_flags("foo, bar = [foo, bar]");
        assert_flags("@a, $b = @a, $b");
    }

    #[test]
    fn accepts_multiple_assignment() {
        assert_accepts("foo, bar = bar, foo");
        assert_accepts("foo, bar = *something");
        assert_accepts("foo, bar = something");
        assert_accepts("foo, *bar = foo, bar");
        assert_accepts("foo, bar = foo, bar, baz");
        assert_accepts("foo.a, bar = foo.a, bar");
    }

    #[test]
    fn flags_or_and_and_variable_self_assignment() {
        for source in [
            "foo ||= foo",
            "foo &&= foo",
            "Foo ||= Foo",
            "Foo &&= Foo",
            "@foo ||= @foo",
            "@@foo ||= @@foo",
            "$foo &&= $foo",
            "Foo::Bar ||= Foo::Bar",
        ] {
            assert_flags(source);
        }
    }

    #[test]
    fn accepts_or_and_and_variable_assignment() {
        assert_accepts("foo ||= bar");
        assert_accepts("foo &&= bar");
    }

    #[test]
    fn flags_or_and_and_attribute_and_key_self_assignment() {
        assert_flags("foo.bar ||= foo.bar");
        assert_flags("foo.bar &&= foo.bar");
        assert_flags("hash['foo'] ||= hash['foo']");
        assert_flags("hash['foo'] &&= hash['foo']");
    }

    #[test]
    fn accepts_or_attribute_and_key_assignment_that_differs() {
        assert_accepts("foo.bar ||= foo.baz");
        assert_accepts("hash['foo'] ||= hash['bar']");
        assert_accepts("hash[foo] ||= hash[foo]");
        assert_accepts("foo&.bar ||= foo.bar");
        assert_accepts("foo.bar ||= other.bar");
    }

    #[test]
    fn flags_attribute_self_assignment() {
        assert_flags("foo.bar = foo.bar");
        assert_flags("foo&.bar = foo&.bar");
    }

    #[test]
    fn accepts_attribute_assignment_that_differs() {
        for source in [
            "foo.bar = foo.baz",
            "bar.foo = baz.foo",
            "foo.bar = foo.bar + 1",
            "foo.bar = foo.bar(arg)",
            "foo.bar = true",
            "foo.bar = foo.bar { x }",
            "self.foo = foo",
        ] {
            assert_accepts(source);
        }
    }

    #[test]
    fn flags_key_self_assignment_with_same_keys() {
        for source in [
            "foo[\"bar\"] = foo[\"bar\"]",
            "foo[1] = foo[1]",
            "foo[1.2] = foo[1.2]",
            "foo[Foo] = foo[Foo]",
            "foo[:bar] = foo[:bar]",
            "foo[@var] = foo[@var]",
            "foo[@@var] = foo[@@var]",
            "foo[$var] = foo[$var]",
            "foo&.[]=(\"bar\", foo[\"bar\"])",
            "matrix[1, 2] = matrix[1, 2]",
            "singleton[] = singleton[]",
        ] {
            assert_flags(source);
        }
        assert_eq!(positions("var = 1\nfoo[var] = foo[var]\n"), vec![(2, 1)]);
    }

    // Node#== is
    // detection identity, not a proof that rewriting either side is safe.
    fn assert_detection_pair(a: &str, b: &str, equal: bool) {
        for encoding in ["UTF8-MAC", "UTF-8"] {
            for op in ["=", "||=", "&&="] {
                for (a, b) in [(a, b), (b, a)] {
                    for (left, right) in [
                        (format!("obj[{a}]"), format!("obj[{b}]")),
                        (format!("obj[[{a}]]"), format!("obj[[{b}]]")),
                        (format!("obj[{a}].x"), format!("obj[{b}].x")),
                        (format!("obj[{{key: {a}}}]"), format!("obj[{{key: {b}}}]")),
                        (format!("obj[\"#{{{a}}}\"]"), format!("obj[\"#{{{b}}}\"]")),
                    ] {
                        let source = format!("# coding: {encoding}\n{left} {op} {right}\n");
                        let found = offenses(&source);
                        assert_eq!(found.len(), usize::from(equal), "{source}");
                        if equal {
                            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 2));
                            assert_eq!(found[0].message, "Self-assignment detected.");
                            assert!(found[0].correction.is_none());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn operator_recovery_quoted_labels_keep_static_symbol_identity() {
        assert_detection_pair(r#"{"": 1}"#, r#"{:"" => 1}"#, false);
        assert_detection_pair(r#"{"": 1}"#, r#"{:"\u{}" => 1}"#, true);
        assert_detection_pair(r#"{'': 1}"#, r#"{:"\u{}" => 1}"#, true);
        assert_detection_pair(r#"{"a": 1}"#, r#"{:a => 1}"#, true);
        assert_detection_pair(r#"{"a": 1}"#, r#"{:b => 1}"#, false);
    }

    #[test]
    fn operator_recovery_signed_zero_is_equal_only_for_detection() {
        assert_detection_pair("[0.0]", "[-0.0]", true);
        assert_detection_pair("[0.0]", "[0]", false);
        assert_detection_pair("[1.0]", "[-1.0]", false);
    }

    #[test]
    fn operator_recovery_rational_payloads_are_not_encoded_source() {
        assert_detection_pair("186657005489253r", "142714875718083r", false);
        assert_detection_pair("1r", "1.0r", true);
        assert_detection_pair("1r", "1.0000000000000001r", false);
    }

    #[test]
    fn operator_recovery_conditional_subtrees_compare_parser_child_slots() {
        assert_detection_pair("[(x ? 1 : 2)]", "[(if x; 1; else; 2; end)]", true);
        assert_detection_pair("[(x ? 1 : 2)]", "[(x ? 2 : 1)]", false);
        assert_detection_pair("[(x ? 1 : 2)]", "[(y ? 1 : 2)]", false);
    }

    #[test]
    fn operator_recovery_regexp_options_are_sorted_sets() {
        assert_detection_pair("/a/im", "/a/mi", true);
        assert_detection_pair("/a/im", "/a/iim", true);
        assert_detection_pair("/a/im", "/a/i", false);
        assert_detection_pair("/a/im", "/b/mi", false);
        assert_detection_pair("/a#{x}/im", "/a#{x}/mi", true);
        assert_detection_pair("/a#{x}/im", "/a#{y}/mi", false);
    }

    #[test]
    fn source_composition_preserves_distinct_binary_numeric_keys() {
        for (a, b) in [
            ("186657005489253", "142714875718083"),
            ("1.0414461810403506", "1.0316890681957467"),
        ] {
            for encoding in ["UTF8-MAC", "UTF-8"] {
                for op in ["=", "||=", "&&="] {
                    for (a, b) in [(a, b), (b, a)] {
                        for (left, right) in [
                            (format!("obj[{a}]"), format!("obj[{b}]")),
                            (format!("obj[[{a}]]"), format!("obj[[{b}]]")),
                            (format!("obj[{a}].x"), format!("obj[{b}].x")),
                            (format!("obj[{{key: {a}}}]"), format!("obj[{{key: {b}}}]")),
                            (format!("obj[\"#{{{a}}}\"]"), format!("obj[\"#{{{b}}}\"]")),
                        ] {
                            assert_accepts(&format!("# coding: {encoding}\n{left} {op} {right}\n"));
                        }
                        let source = format!("# coding: {encoding}\nobj[{a}] {op} obj[{a}]\n");
                        let found = offenses(&source);
                        assert_eq!(found.len(), 1, "{source}");
                        assert_eq!((found[0].line, found[0].col), (2, 1));
                        assert_eq!(found[0].message, "Self-assignment detected.");
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn source_composition_does_not_compose_escape_produced_payloads() {
        for op in ["=", "||=", "&&="] {
            for (a, b) in [
                (r#""é""#, r#""\u0065\u0301""#),
                ("\"e\u{301}\"", r#""\u0065\u0301""#),
                (r#""\u00e9""#, r#""\u0065\u0301""#),
            ] {
                for (a, b) in [(a, b), (b, a)] {
                    for (left, right) in [
                        (format!("obj[{a}]"), format!("obj[{b}]")),
                        (format!("obj[[{a}]]"), format!("obj[[{b}]]")),
                        (format!("obj[{a}].x"), format!("obj[{b}].x")),
                    ] {
                        let source = format!("# coding: UTF8-MAC\n{left} {op} {right}\n");
                        assert!(offenses(&source).is_empty(), "{source}");
                    }
                }
            }
            for key in [r#""é""#, r#""\u0065\u0301""#, "%q()", "%Q()"] {
                let source = format!("# coding: UTF8-MAC\nobj[{key}] {op} obj[{key}]\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 2));
                assert_eq!(found[0].message, "Self-assignment detected.");
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn empty_percent_dstr_keys_do_not_equal_quoted_str_keys() {
        for op in ["=", "||=", "&&="] {
            for a in ["%q()", "%Q()"] {
                for b in [r#""""#, r#""\u{}""#] {
                    for (a, b) in [(a, b), (b, a)] {
                        for (left, right) in [
                            (format!("obj[{a}]"), format!("obj[{b}]")),
                            (format!("obj[[{a}]]"), format!("obj[[{b}]]")),
                            (format!("obj[{a}].x"), format!("obj[{b}].x")),
                        ] {
                            let source = format!("{left} {op} {right}\n");
                            assert!(offenses(&source).is_empty(), "{source}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn literal_block_keys_are_not_plain_call_keys() {
        for source in [
            "obj[foo {}] = obj[foo {}]",
            "obj[foo { _1 }] ||= obj[foo { _1 }]",
            "obj[foo { it }] &&= obj[foo { it }]",
        ] {
            assert_flags(source);
        }
        for source in ["obj[foo] = obj[foo]", "obj[foo(&b)] = obj[foo(&b)]"] {
            assert_accepts(source);
        }
    }

    #[test]
    fn accepts_key_assignment_that_differs() {
        for source in [
            "foo[\"bar\"] = foo[\"baz\"]",
            "foo[1] = foo[2]",
            "foo[1.2] = foo[2.2]",
            "foo[Foo] = foo[Bar]",
            "foo[:foo] = foo[:bar]",
            "foo[@var1] = foo[@var2]",
            "foo[@@var1] = foo[@@var2]",
            "foo[$var1] = foo[$var2]",
            "foo[bar] = foo[bar]",
            "bar[\"foo\"] = baz[\"foo\"]",
            "foo[\"bar\"] = foo[\"bar\"] + 1",
            "matrix[1, 2] = matrix[1, 3]",
            "matrix[1, foo] = matrix[1, foo]",
            "foo.[]=",
        ] {
            assert_accepts(source);
        }
        assert!(offenses("var1 = 1\nvar2 = 2\nfoo[var1] = foo[var2]\n").is_empty());
    }

    #[test]
    fn flags_despite_comments_including_rbs_annotations() {
        // AllowRBSInlineAnnotation is fixed at RuboCop's default (false).
        let statements = [
            "foo = foo",
            "@foo = @foo",
            "@@foo = @@foo",
            "$foo = $foo",
            "Foo = Foo",
            "foo, bar = foo, bar",
            "foo ||= foo",
            "foo &&= foo",
            "foo.bar = foo.bar",
            "foo&.bar = foo&.bar",
            "foo[\"bar\"] = foo[\"bar\"]",
            "foo&.[]=(\"bar\", foo[\"bar\"])",
        ];
        for comment in ["#: Integer", "# comment", ""] {
            let source: String = statements
                .iter()
                .map(|statement| format!("{statement} {comment}\n"))
                .collect();
            let expected: Vec<(u32, u32)> = (1..=statements.len() as u32)
                .map(|line| (line, 1))
                .collect();
            assert_eq!(positions(&source), expected, "with {comment:?}");
        }
    }

    #[test]
    fn reports_at_assignment_start_when_nested() {
        assert_eq!(positions("def m\n  @a = @a\nend\n"), vec![(2, 3)]);
        assert_eq!(positions("x = (y = y)\n"), vec![(1, 6)]);
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "foo += foo",
            "foo = foo + 1",
            "foo = (foo)",
            "foo = @foo",
            "@foo = foo",
            "foo == foo",
            "foo.bar == foo.bar",
            "foo = self.foo",
        ] {
            assert_accepts(source);
        }
    }
}
