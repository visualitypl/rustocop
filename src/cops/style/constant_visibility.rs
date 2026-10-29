//! `Style/ConstantVisibility` — explicit declarations, fixed IgnoreModules=false.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashSet;

pub struct ConstantVisibility;
impl Cop for ConstantVisibility {
    fn name(&self) -> &'static str {
        "Style/ConstantVisibility"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (children, sibling) = if let Some(statements) = node.as_statements_node() {
            // Only transparent :begin (statement lists/parentheses) edges lead
            // to class/module scope. Other enclosing nodes are boundaries.
            let scope = cx.ancestors().find(|ancestor| {
                !matches!(
                    ancestor,
                    Node::StatementsNode { .. } | Node::ParenthesesNode { .. }
                )
            });
            if !scope.is_some_and(|s| matches!(s, Node::ClassNode { .. } | Node::ModuleNode { .. }))
            {
                return;
            }
            let children: Vec<_> = statements.body().iter().collect();
            // A sole body expression has the class as its Parser parent;
            // a direct superclass send is then a sibling declaration too.
            let superclass = (children.len() == 1
                && cx.parent().is_some_and(|p| p.as_class_node().is_some()))
            .then(|| {
                scope
                    .and_then(Node::as_class_node)
                    .and_then(|c| c.superclass())
            })
            .flatten();
            (children, superclass)
        } else {
            // Unparenthesized superclass assignments have no StatementsNode.
            // Their direct Parser siblings include only a sole body expression,
            // not the children of a multi-expression body or parentheses.
            if !matches!(
                node,
                Node::ConstantWriteNode { .. } | Node::ConstantPathWriteNode { .. }
            ) {
                return;
            }
            let Some(class) = cx.parent().and_then(Node::as_class_node) else {
                return;
            };
            let sibling = class.body().and_then(|body| {
                let statements = body.as_statements_node()?;
                (statements.body().len() == 1)
                    .then(|| statements.body().iter().next())
                    .flatten()
            });
            (class.superclass().into_iter().collect(), sibling)
        };
        let mut declarations = HashSet::new();
        let mut dynamic = false;
        // Collect direct sibling sends ONCE per body, not once per assignment.
        for child in children.iter().chain(sibling.iter()) {
            let Some(call) = child.as_call_node() else {
                continue;
            };
            if call.receiver().is_some()
                || !matches!(
                    call.name().as_slice(),
                    b"public_constant" | b"private_constant"
                )
                || call.is_safe_navigation()
            {
                continue;
            }
            // A literal block makes this a :block, not a direct :send sibling.
            if call.block().is_some_and(|b| b.as_block_node().is_some()) {
                continue;
            }
            let mut args = ast::call_arguments(&call);
            if let Some(splat) = args.first().and_then(Node::as_splat_node) {
                if let Some(array) = splat.expression().and_then(|n| n.as_array_node()) {
                    args = array.elements().iter().collect();
                } else {
                    dynamic = true;
                }
            }
            for argument in args {
                if let Some(name) = argument
                    .as_symbol_node()
                    .map(|n| n.unescaped().to_vec())
                    .or_else(|| argument.as_string_node().map(|n| n.unescaped().to_vec()))
                {
                    declarations.insert(name);
                }
            }
        }
        if dynamic {
            return;
        }
        for child in children {
            let name = if let Some(write) = child.as_constant_write_node() {
                write.name().as_slice().to_vec()
            } else if let Some(write) = child.as_constant_path_write_node() {
                write
                    .target()
                    .name()
                    .map(|n| n.as_slice().to_vec())
                    .unwrap_or_default()
            } else {
                continue;
            };
            if declarations.contains(&name) {
                continue;
            }
            let location = child.location();
            let (line, col) = file.line_col(location.start_offset());
            out.push(Offense {
                cop: self.name(), severity: self.severity(), line, col,
                end_line: file.last_line_of(location), correction: None,
                message: format!("Explicitly make `{}` public or private using either `#public_constant` or `#private_constant`.", String::from_utf8_lossy(&name)),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/constant_visibility_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        // lint_str honors default-off; enable through a real source directive.
        lint_str(&format!(
            "# rustocop:enable Style/ConstantVisibility\n{source}"
        ))
        .into_iter()
        .filter(|o| o.cop == ConstantVisibility.name())
        .collect()
    }
    #[test]
    fn pinned_offenses_and_exact_messages() {
        for (prefix, value, suffix, name) in [
            ("class Foo", "42", "", "BAR"),
            ("class Foo", "42", "private_constant", "BAR"),
            ("class Foo", "42", "private_constant 42", "BAR"),
            ("class Foo\n include Bar", "42", "", "BAR"),
            (
                "class Foo\n include Bar",
                "42",
                "private_constant :FOO",
                "BAR",
            ),
            ("module Foo", "42", "", "BAR"),
            ("module Foo", "Class.new()", "", "MyClass"),
            ("class Foo", "Module.new()", "", "MyClass"),
            ("class Foo", "Struct.new()", "", "MyClass"),
            ("class Foo", "42", "private_constant(*%i[BAZ])", "BAR"),
        ] {
            let source = format!("{prefix}\n  {name} = {value}\n  {suffix}\nend");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            let line = 2 + prefix.lines().count() as u32;
            assert_eq!((found[0].line, found[0].col), (line, 3));
            assert_eq!(
                found[0].message,
                format!(
                    "Explicitly make `{name}` public or private using either `#public_constant` or `#private_constant`."
                )
            );
            assert!(
                correction_offenses(&ConstantVisibility, &source)[0]
                    .correction
                    .is_none()
            );
        }
    }
    #[test]
    fn pinned_visibility_arguments() {
        for declaration in [
            "private_constant :BAR, :BAZ",
            "public_constant :BAR, :BAZ",
            "private_constant 'BAR', 'BAZ'",
            "private_constant *['BAR', 'BAZ']",
            "private_constant(*%i[BAR BAZ])",
            "private_constant(*constants(false))",
        ] {
            let source = format!("class Foo\n BAR = 42\n BAZ = 43\n {declaration}\nend");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for declaration in [
            "private_constant ['BAR', 'BAZ']",
            "self.private_constant :BAR, :BAZ",
            "other.private_constant :BAR, :BAZ",
            "private_constant 42, 1.2",
        ] {
            let source = format!("class Foo\n BAR = 42\n BAZ = 43\n {declaration}\nend");
            let found = offenses(&source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                [(3, 2), (4, 2)],
                "{source}"
            );
        }
    }
    #[test]
    fn lexical_qualified_reopened_and_singleton_scopes() {
        for source in [
            "BAR = 42",
            "::BAR = 42",
            "Foo::BAR = 42",
            "class << self; BAR = 42; end",
            "Class.new { BAR = 42 }",
            "module Foo; tap { BAR = 42 }; end",
            "module Foo; if true; BAR = 42; end; end",
            "module Foo; begin; BAR = 42; end; end",
            "module Foo; BAR ||= 42; BAR &&= 43; BAR += 1; end",
            "module Foo; A, B = 1, 2; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "module Foo; BAR = 42; end",
            "class Foo::Bar; BAR = 42; end",
            "module Foo; ::BAR = 42; end",
            "module Foo; Other::BAR = 42; end",
            "module Foo; (BAR = 42); end",
            "module Foo; module Inner; BAR = 42; end; private_constant :BAR; end",
            "module Foo; BAR = 42; end; module Foo; private_constant :BAR; end",
            "module Foo; include M; extend N; private; BAR = 42; end",
            "class Foo < (private_constant(:BAR)); BAR = 42; end",
            "class Foo < private_constant(:BAR); BAR = 42; unrelated; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "module Foo; Other::BAR = 42; private_constant :BAR; end",
            "module Foo; private_constant :BAR; BAR = 42; end",
            "module Foo; BAR = 42; private_constant :\"B\\x41R\"; end",
            "module Foo; (BAR = 42; private_constant :BAR); end",
            // A sole body expression has the class as its Parser parent, so
            // a direct superclass send is a sibling declaration too.
            "class Foo < private_constant(:BAR); BAR = 42; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn superclass_constant_writes_and_sibling_visibility() {
        for (source, col, name) in [
            ("class Foo < BAR = Object; end", 13, "BAR"),
            ("class Foo < Other::BAR = Object; end", 13, "BAR"),
            ("class Foo < ::BAR = Object; end", 13, "BAR"),
            (
                "class Foo < BAR = Object; private_constant :OTHER; end",
                13,
                "BAR",
            ),
            // Multiple body expressions form a separate sibling list.
            (
                "class Foo < BAR = Object; private_constant :BAR; unrelated; end",
                13,
                "BAR",
            ),
            (
                "class Foo < BAR = Object; (private_constant :BAR); end",
                13,
                "BAR",
            ),
            (
                "class Foo < (BAR = Object); private_constant :BAR; end",
                14,
                "BAR",
            ),
            (
                "class Foo < BAR = Object; self.private_constant :BAR; end",
                13,
                "BAR",
            ),
            (
                "class Foo < BAR = Object; private_constant(:BAR) {}; end",
                13,
                "BAR",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, col), "{source}");
            assert_eq!(
                found[0].message,
                format!(
                    "Explicitly make `{name}` public or private using either `#public_constant` or `#private_constant`."
                )
            );
            assert!(found[0].correction.is_none());
        }
        for source in [
            "class Foo < BAR = Object; private_constant :BAR; end",
            "class Foo < Other::BAR = Object; public_constant 'BAR'; end",
            "class Foo < ::BAR = Object; private_constant(*%i[BAR]); end",
            "class Foo < BAR = Object; private_constant(*names); end",
            "class Foo < (BAR ||= Object); end",
            "class Foo < build(BAR = Object); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn visibility_scan_is_linear_in_wide_bodies() {
        let mut source = String::from("module Foo\n");
        for n in 0..4000 {
            source.push_str(&format!(" C{n} = {n}\n private_constant :C{n}\n"));
        }
        source.push_str("end\n");
        assert!(offenses(&source).is_empty());
    }
}
