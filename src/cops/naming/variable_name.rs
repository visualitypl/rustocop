//! `Naming/VariableName` — flags variable names not written in snake_case.
//!
//! Matches RuboCop's semantics at its defaults (`EnforcedStyle:
//! snake_case`, no allowed or forbidden identifiers): checked are local
//! variable assignments (`=`, `||=`, `+=`, multiple assignment, `rescue =>`,
//! `for`), local variable reads, instance and class variable assignments
//! (not reads), and every named method, block and lambda parameter. Names
//! bound by pattern matching (`in [x]`, `=> x`) or by named regexp captures,
//! block-local variables (`|a; b|`) and globals are not checked (their reads
//! are, where they are locals). The offense is reported at the name, sigil
//! included for instance and class variables.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

use super::format::is_snake_case;

/// The `Naming/VariableName` cop.
pub struct VariableName;

impl Cop for VariableName {
    fn name(&self) -> &'static str {
        "Naming/VariableName"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut check = |(name, offset): Named<'pr>| {
            if is_snake_case(&String::from_utf8_lossy(name)) {
                return;
            }
            let (line, col) = file.line_col(offset);
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: "Use snake_case for variable names.".to_string(),
                line,
                end_line: line,
                col,
            });
        };
        if let Some(name) = variable_name(node) {
            check(name);
            return;
        }
        // A local variable target is an assignment (RuboCop's `lvasgn`) in
        // multiple assignment, `rescue =>` and `for`, but a pattern binding
        // (`match_var`) or a named capture elsewhere: judge it from the
        // nodes that make it an assignment.
        match node {
            Node::MultiWriteNode { .. } => {
                let write = node.as_multi_write_node().unwrap();
                let targets = write
                    .lefts()
                    .iter()
                    .chain(write.rest())
                    .chain(write.rights().iter());
                targets.filter_map(local_target).for_each(check);
            }
            Node::MultiTargetNode { .. } => {
                let target = node.as_multi_target_node().unwrap();
                let targets = target
                    .lefts()
                    .iter()
                    .chain(target.rest())
                    .chain(target.rights().iter());
                targets.filter_map(local_target).for_each(check);
            }
            Node::RescueNode { .. } => {
                if let Some(name) = node
                    .as_rescue_node()
                    .unwrap()
                    .reference()
                    .and_then(local_target)
                {
                    check(name);
                }
            }
            Node::ForNode { .. } => {
                if let Some(name) = local_target(node.as_for_node().unwrap().index()) {
                    check(name);
                }
            }
            _ => {}
        }
    }
}

/// A variable's name, and the offset RuboCop reports it at
/// (`node.loc.name`).
type Named<'pr> = (&'pr [u8], usize);

/// `$node.$cast()`'s name, at the start of its location `$loc`.
macro_rules! named {
    ($node:expr, $cast:ident, $loc:ident) => {{
        let typed = $node.$cast()?;
        (typed.name().as_slice(), typed.$loc().start_offset())
    }};
}

/// The variable `node` assigns, reads or binds as a parameter; `None` for
/// anything else, including local variable targets, which depend on their
/// parent. The name comes from the node, not its source: a keyword
/// parameter's name location includes the colon.
pub(super) fn variable_name<'pr>(node: &Node<'pr>) -> Option<Named<'pr>> {
    Some(match node {
        Node::LocalVariableWriteNode { .. } => {
            named!(node, as_local_variable_write_node, name_loc)
        }
        Node::LocalVariableOrWriteNode { .. } => {
            named!(node, as_local_variable_or_write_node, name_loc)
        }
        Node::LocalVariableAndWriteNode { .. } => {
            named!(node, as_local_variable_and_write_node, name_loc)
        }
        Node::LocalVariableOperatorWriteNode { .. } => {
            named!(node, as_local_variable_operator_write_node, name_loc)
        }
        Node::LocalVariableReadNode { .. } => named!(node, as_local_variable_read_node, location),
        Node::InstanceVariableWriteNode { .. } => {
            named!(node, as_instance_variable_write_node, name_loc)
        }
        Node::InstanceVariableOrWriteNode { .. } => {
            named!(node, as_instance_variable_or_write_node, name_loc)
        }
        Node::InstanceVariableAndWriteNode { .. } => {
            named!(node, as_instance_variable_and_write_node, name_loc)
        }
        Node::InstanceVariableOperatorWriteNode { .. } => {
            named!(node, as_instance_variable_operator_write_node, name_loc)
        }
        Node::InstanceVariableTargetNode { .. } => {
            named!(node, as_instance_variable_target_node, location)
        }
        Node::ClassVariableWriteNode { .. } => {
            named!(node, as_class_variable_write_node, name_loc)
        }
        Node::ClassVariableOrWriteNode { .. } => {
            named!(node, as_class_variable_or_write_node, name_loc)
        }
        Node::ClassVariableAndWriteNode { .. } => {
            named!(node, as_class_variable_and_write_node, name_loc)
        }
        Node::ClassVariableOperatorWriteNode { .. } => {
            named!(node, as_class_variable_operator_write_node, name_loc)
        }
        Node::ClassVariableTargetNode { .. } => {
            named!(node, as_class_variable_target_node, location)
        }
        Node::RequiredParameterNode { .. } => named!(node, as_required_parameter_node, location),
        Node::OptionalParameterNode { .. } => named!(node, as_optional_parameter_node, name_loc),
        Node::RequiredKeywordParameterNode { .. } => {
            named!(node, as_required_keyword_parameter_node, name_loc)
        }
        Node::OptionalKeywordParameterNode { .. } => {
            named!(node, as_optional_keyword_parameter_node, name_loc)
        }
        // Anonymous `*`, `**` and `&` have no name.
        Node::RestParameterNode { .. } => {
            let rest = node.as_rest_parameter_node()?;
            (rest.name()?.as_slice(), rest.name_loc()?.start_offset())
        }
        Node::KeywordRestParameterNode { .. } => {
            let rest = node.as_keyword_rest_parameter_node()?;
            (rest.name()?.as_slice(), rest.name_loc()?.start_offset())
        }
        Node::BlockParameterNode { .. } => {
            let block = node.as_block_parameter_node()?;
            (block.name()?.as_slice(), block.name_loc()?.start_offset())
        }
        _ => return None,
    })
}

/// The name of a local variable target (`a` in `a, b = …`), looking through
/// a splat (`*a`).
fn local_target<'pr>(node: Node<'pr>) -> Option<Named<'pr>> {
    let node = match node.as_splat_node() {
        Some(splat) => splat.expression()?,
        None => node,
    };
    Some(named!(node, as_local_variable_target_node, location))
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/naming/variable_name_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! `snake_case` style without allowed or forbidden identifiers.

    use crate::pipeline::lint_str;

    const MESSAGE: &str = "Use snake_case for variable names.";

    fn found(source: &str) -> Vec<(u32, u32)> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/VariableName")
            .map(|o| {
                assert_eq!(o.message, MESSAGE);
                (o.line, o.col)
            })
            .collect()
    }

    #[test]
    fn flags_rubocop_spec_assignments() {
        assert_eq!(found("myLocal = 1\n"), [(1, 1)]);
        assert_eq!(found("my_local = 1\nmyLocal = 1\n"), [(2, 1)]);
        assert_eq!(found("@myAttribute = 3\n"), [(1, 1)]);
        assert_eq!(found("@@myAttr = 2\n"), [(1, 1)]);
        assert_eq!(found("_myLocal = 1\n"), [(1, 1)]);
    }

    #[test]
    fn flags_rubocop_spec_parameters() {
        assert_eq!(found("def method(funnyArg); end\n"), [(1, 12)]);
        assert_eq!(found("def foo(optArg = 1); end\n"), [(1, 9)]);
        assert_eq!(found("def foo(*restArg); end\n"), [(1, 10)]);
        assert_eq!(found("def foo(kwArg: 1); end\n"), [(1, 9)]);
        assert_eq!(found("def foo(**kwRest); end\n"), [(1, 11)]);
        assert_eq!(found("def foo(&blockArg); end\n"), [(1, 10)]);
    }

    #[test]
    fn judges_keyword_parameters_without_their_colon() {
        for source in [
            "def urlsafe_encode64(bin, padding: true); end\n",
            "def foo(kw_arg:, other_kw: 1, **rest); end\n",
            "foo { |kw_arg:, other_kw: 1| }\n",
            "->(kw_arg:) {}\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
        assert_eq!(found("def foo(kwArg:); end\n"), [(1, 9)]);
    }

    #[test]
    fn flags_reads_of_local_variables() {
        assert_eq!(
            found("firstArg = 'foo'\nsecondArg = 'foo'\n\ndo_something(firstArg, secondArg)\n"),
            [(1, 1), (2, 1), (4, 14), (4, 24)]
        );
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "$MY_GLOBAL = 0\n",
            "MY_CONSTANT = 0\n",
            "Paren = Struct.new :left, :right, :kind\n",
            "self[:a] = b\n",
            "_ = 1\n",
            "i = 1\n",
            "léo = 1\n",
            "$firstArg = :foo\n",
            "def firstArg\nend\n",
            ":firstArg\n",
            "firstArg()\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn flags_every_kind_of_local_assignment() {
        assert_eq!(found("myF ||= 1\n"), [(1, 1)]);
        assert_eq!(found("myF &&= 1\n"), [(1, 1)]);
        assert_eq!(found("myF += 1\n"), [(1, 1)]);
        assert_eq!(found("a, myB = 1, 2\n"), [(1, 4)]);
        assert_eq!(found("a, *myB = 1, 2\n"), [(1, 5)]);
        assert_eq!(found("*a, myB = 1, 2\n"), [(1, 5)]);
        assert_eq!(found("a, (b, myC) = 1, [2, 3]\n"), [(1, 8)]);
        assert_eq!(found("for myD in x; end\n"), [(1, 5)]);
        assert_eq!(found("for a, myD in x; end\n"), [(1, 8)]);
        assert_eq!(found("begin\nrescue => myE\nend\n"), [(2, 11)]);
    }

    #[test]
    fn flags_instance_and_class_variable_assignments() {
        assert_eq!(found("@a, @myC = 1, 2\n"), [(1, 5)]);
        assert_eq!(found("@myG += 1\n"), [(1, 1)]);
        assert_eq!(found("@myG ||= 1\n"), [(1, 1)]);
        assert_eq!(found("@@myH &&= 1\n"), [(1, 1)]);
        assert_eq!(found("@@a, @@myH = 1, 2\n"), [(1, 6)]);
        assert_eq!(found("begin\nrescue => @myE\nend\n"), [(2, 11)]);
    }

    #[test]
    fn flags_block_and_lambda_parameters() {
        assert_eq!(found("foo { |(a, myK)| }\n"), [(1, 12)]);
        assert_eq!(found("->(myL) {}\n"), [(1, 4)]);
        assert_eq!(found("foo { |myM| }\n"), [(1, 8)]);
        assert_eq!(
            found("foo { |*myN, **myO, &myP| }\n"),
            [(1, 9), (1, 16), (1, 22)]
        );
        assert_eq!(found("foo { |a, myB = 1, myC:| }\n"), [(1, 11), (1, 20)]);
        assert_eq!(found("def m(a, (b, myC)); end\n"), [(1, 14)]);
    }

    #[test]
    fn flags_reads_in_every_position() {
        assert_eq!(found("myVar = 1\nfoo(myVar:)\n"), [(1, 1), (2, 5)]);
        assert_eq!(found("myVar = 1\n\"#{myVar}\"\n"), [(1, 1), (2, 4)]);
        assert_eq!(
            found("myVar = 1\ncase 1\nin ^myVar then 1\nend\n"),
            [(1, 1), (3, 5)]
        );
    }

    #[test]
    fn names_bound_by_patterns_and_captures_are_only_checked_when_read() {
        assert_eq!(found("case 1\nin [myA] then myA\nend\n"), [(2, 15)]);
        assert_eq!(found("foo { |x; myV| myV }\n"), [(1, 16)]);
        for source in [
            "case 1\nin [a, *myRest] then 1\nend\n",
            "case 1\nin {myKey:} then 1\nend\n",
            "case 1\nin {key: myValue} then 1\nend\n",
            "case 1\nin Integer => myInt then 1\nend\n",
            "case 1\nin [*, myX, *] then 1\nend\n",
            "case 1\nin {a: 1, **myRest} then 1\nend\n",
            "x => {myQ:}\n",
            "x in [myQ]\n",
            "/(?<myCap>.)/ =~ s\n",
            "foo { |x; myV| }\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "@myIvar\n",
            "@@myCvar\n",
            "$myGlobal = 1\n",
            "foo.myAttr = 1\n",
            "foo(myArg: 1)\n",
            "def foo(*, **, &); end\n",
            "def foo(...); end\n",
            "foo { _1 }\nbar { it }\n",
            "FooBar = 1\n",
            "alias myAlias foo\n",
            "@foo_bar = 1\n@@foo2 = 2\nfoo_bar ||= 3\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
