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
    cops::{
        Cop, NodeContext,
        ast::{same_node, same_optional_node},
    },
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/SelfAssignment` cop.
pub struct SelfAssignment;

/// Whether `value` reads the variable written by a `name =`-style node of the
/// given variable kind.
fn reads_variable(kind: VariableKind, name: &[u8], value: &Node<'_>) -> bool {
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
        _ => return false,
    };
    read.as_slice() == name
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
fn same_constant_path(target: &ConstantPathNode<'_>, value: &Node<'_>) -> bool {
    let Some(value) = value.as_constant_path_node() else {
        return false;
    };
    let same_name = match (target.name(), value.name()) {
        (Some(a), Some(b)) => a.as_slice() == b.as_slice(),
        _ => false,
    };
    same_name && same_optional_node(target.parent().as_ref(), value.parent().as_ref())
}

/// `a, b = a, b` / `a, b = [a, b]`: every target is a variable re-assigned
/// from the read at the same position.
fn multiple_self_assignment(node: &Node<'_>) -> bool {
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
                Some((kind, name)) => reads_variable(kind, name, value),
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
) -> Option<bool> {
    let call = value.as_call_node()?;
    let value_arguments = call_arguments(&call);
    Some(
        call.block().is_none()
            && call.name().as_slice() == name
            && same_optional_node(receiver, call.receiver().as_ref())
            && value_arguments.len() == arguments.len()
            && arguments
                .iter()
                .zip(value_arguments.iter())
                .all(|(a, b)| same_node(a, b)),
    )
}

/// A call-like key may return different values on each call.
fn has_call_key(arguments: &[Node<'_>]) -> bool {
    arguments
        .iter()
        .any(|argument| argument.as_call_node().is_some())
}

/// `foo.bar ||= foo.bar` / `foo.bar &&= foo.bar`.
fn attribute_or_and_self_assignment(node: &Node<'_>) -> bool {
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
        && reads_same(&value, name.as_slice(), receiver.as_ref(), &[]).unwrap_or(false)
}

/// `hash['k'] ||= hash['k']` / `hash['k'] &&= hash['k']`.
fn index_or_and_self_assignment(node: &Node<'_>) -> bool {
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
    !has_call_key(&arguments)
        && value
            .as_call_node()
            .is_some_and(|call| call.is_safe_navigation() == safe_navigation)
        && reads_same(&value, b"[]", receiver.as_ref(), &arguments).unwrap_or(false)
}

/// `foo['k'] = foo['k']` and `foo.bar = foo.bar` (including `&.`).
fn call_self_assignment(node: &Node<'_>) -> bool {
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
        return !has_call_key(keys)
            && reads_same(value, b"[]", call.receiver().as_ref(), keys).unwrap_or(false);
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
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let self_assigned = if let Some((kind, name, value)) = variable_write(node) {
            reads_variable(kind, name, &value)
        } else if let Some((target, value)) = constant_path_write(node) {
            same_constant_path(&target, &value)
        } else {
            multiple_self_assignment(node)
                || attribute_or_and_self_assignment(node)
                || index_or_and_self_assignment(node)
                || call_self_assignment(node)
        };
        if !self_assigned {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
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
