//! `Style/ClassMethods` — use lexical self for matching direct singleton defs.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ClassMethods;
impl Cop for ClassMethods {
    fn name(&self) -> &'static str {
        "Style/ClassMethods"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((name, body)) = declaration(node) else {
            return;
        };
        // A dynamic/qualified header can be large. Fingerprint it once, not
        // once per direct method (including mismatching short receivers).
        let name_key = ast::fingerprint(&name);
        for child in elements(body) {
            let Some(def) = child.as_def_node() else {
                continue;
            };
            let Some(original_receiver) = def.receiver() else {
                continue;
            };
            let receiver = ungroup(def.receiver().unwrap());
            if std::mem::discriminant(&name) != std::mem::discriminant(&receiver)
                || name_key != ast::fingerprint(&receiver)
            {
                continue;
            }
            let loc = receiver
                .as_constant_path_node()
                .map(|n| n.name_loc())
                .unwrap_or(receiver.location());
            let (line, col) = file.line_col(loc.start_offset());
            let method = String::from_utf8_lossy(def.name().as_slice()).into_owned();
            let class = String::from_utf8_lossy(name.location().as_slice());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.last_line_of(loc),
                message: format!("Use `self.{method}` instead of `{class}.{method}`."),
                correction: cx.corrections_requested().then(|| Correction {
                    range: original_receiver.location().start_offset()
                        ..original_receiver.location().end_offset(),
                    replacement: "self".into(),
                }),
            });
        }
    }
}

// The pinned AST drops the receiver's syntactic parentheses, but retains
// additional grouping as begin nodes. Strip exactly one layer, not all groups.
fn ungroup(node: Node<'_>) -> Node<'_> {
    let Some(body) = node.as_parentheses_node().and_then(|group| group.body()) else {
        return node;
    };
    if let Some(statements) = body.as_statements_node() {
        if statements.body().len() != 1 {
            return node;
        }
        statements.body().first().unwrap()
    } else {
        body
    }
}

pub(super) fn declaration<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Option<Node<'pr>>)> {
    node.as_class_node()
        .map(|n| (n.constant_path(), n.body()))
        .or_else(|| node.as_module_node().map(|n| (n.constant_path(), n.body())))
}

// Parser collapses an implicit singleton statement list; parentheses produce
// a begin node. Unwrap just that one group, never rescue/explicit kwbegin.
pub(super) fn elements<'pr>(body: Option<Node<'pr>>) -> Vec<Node<'pr>> {
    let Some(body) = body else {
        return Vec::new();
    };
    let nodes: Vec<_> = body
        .as_statements_node()
        .map(|n| n.body().iter().collect())
        .unwrap_or_else(|| vec![body]);
    if let [only] = nodes.as_slice()
        && let Some(group) = only.as_parentheses_node()
    {
        return group
            .body()
            .map(|body| {
                body.as_statements_node()
                    .map(|n| n.body().iter().collect())
                    .unwrap_or_else(|| vec![body])
            })
            .unwrap_or_default();
    }
    nodes
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/class_methods_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_class_and_module_receiver_locations() {
        for kind in ["class", "module"] {
            let source =
                format!("{kind} Test\n  def Test.some_method\n    do_something\n  end\nend\n");
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == ClassMethods.name())
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 7, 2));
            assert_eq!(
                found[0].message,
                "Use `self.some_method` instead of `Test.some_method`."
            );
        }
    }
    #[test]
    fn pinned_lookalikes_and_non_direct_defs() {
        for source in [
            "module Test; def self.some_method; end; end",
            "class Test; X = Something.new; def X.some_method; end; end",
            "def Test.some_method; end",
            "class Test; if condition; def Test.foo; end; end; end",
            "class Test; class << self; def Test.foo; end; end; end",
            "class Test; private_class_method def Test.foo; end; end",
            "module Outer; class Test; def Outer.foo; end; end; end",
            "class A::Test; def Test.foo; end; end",
            "class Test; def (::Test).foo; end; end",
        ] {
            assert!(
                correction_offenses(&ClassMethods, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn receiver_parentheses_match_pinned_ast_semantics() {
        for kind in ["class", "module"] {
            for receiver in ["A", "A::B"] {
                let source = format!("{kind} {receiver}; def ({receiver}).foo; end; end");
                let found = correction_offenses(&ClassMethods, &source);
                assert_eq!(found.len(), 1, "{source}");
                let col = source.find("def (").unwrap()
                    + 6
                    + receiver.rfind("::").map_or(0, |offset| offset + 2);
                assert_eq!(
                    (found[0].line, found[0].col),
                    (1, u32::try_from(col).unwrap()),
                    "{source}"
                );
                for receiver in [format!("(({receiver}))"), format!("((({receiver})))")] {
                    let name = if receiver.contains("::") { "A::B" } else { "A" };
                    let source = format!("{kind} {name}; def {receiver}.foo; end; end");
                    assert!(
                        correction_offenses(&ClassMethods, &source).is_empty(),
                        "{source}"
                    );
                }
            }
        }
    }
    #[test]
    fn qualified_nested_reopened_singleton_and_shadowed_receivers_still_report() {
        for (source, col, receiver) in [
            ("class A::Test; def (A::Test).foo; end; end", 24, "A::Test"),
            (
                "module M; class Test; def Test.foo; end; end; end",
                27,
                "Test",
            ),
            (
                "class Test; end; class Test; def Test.foo; end; end",
                34,
                "Test",
            ),
            (
                "class << obj; class Test; def Test.foo; end; end; end",
                31,
                "Test",
            ),
            (
                "class Test; Test = other; def Test.foo; end; end",
                31,
                "Test",
            ),
            ("class Test; (def Test.foo; end); end", 18, "Test"),
        ] {
            let found = correction_offenses(&ClassMethods, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(
                found[0].message,
                format!("Use `self.foo` instead of `{receiver}.foo`.")
            );
        }
    }
    #[test]
    fn pinned_canonical_corrections() {
        for kind in ["class", "module"] {
            let source =
                format!("{kind} Test\n  def Test.some_method\n    do_something\n  end\nend\n");
            assert_correction(
                &ClassMethods,
                &source,
                &source.replace("def Test.", "def self."),
            );
        }
    }
}
