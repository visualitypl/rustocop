//! `Style/ModuleFunction` — fixed `module_function`, detection only.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ModuleFunction;
impl Cop for ModuleFunction {
    fn name(&self) -> &'static str {
        "Style/ModuleFunction"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_module_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(module) = node.as_module_node() else {
            return;
        };
        let children = body_elements(module.body());
        if children.iter().any(|child| {
            child.as_call_node().is_some_and(|call| {
                call.receiver().is_none()
                    && call.name().as_slice() == b"private"
                    && !call
                        .block()
                        .is_some_and(|block| block.as_block_node().is_some())
            })
        }) {
            return;
        }
        for child in children {
            let Some(call) = child.as_call_node() else {
                continue;
            };
            let args = crate::cops::ast::call_arguments(&call);
            if call.receiver().is_some()
                || call.name().as_slice() != b"extend"
                || call.block().is_some()
                || !matches!(args.as_slice(), [arg] if arg.as_self_node().is_some())
            {
                continue;
            }
            let (line, col) = file.line_col(child.location().start_offset());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Use `module_function` instead of `extend self`.".into(),
                line,
                col,
                end_line: file.last_line_of(child.location()),
                correction: cx
                    .corrections_requested()
                    .then(|| crate::offense::Correction {
                        range: child.location().start_offset()..child.location().end_offset(),
                        replacement: "module_function".into(),
                    }),
            });
        }
    }
}

// Match RuboCop's one direct :begin layer, not recursively nested control flow.
pub(super) fn body_elements<'pr>(body: Option<Node<'pr>>) -> Vec<Node<'pr>> {
    let Some(body) = body else { return Vec::new() };
    let children: Vec<_> = body
        .as_statements_node()
        .map(|s| s.body().iter().collect())
        .unwrap_or_else(|| vec![body]);
    if let [only] = children.as_slice()
        && let Some(group) = only.as_parentheses_node()
    {
        return group
            .body()
            .and_then(|body| body.as_statements_node())
            .map(|s| s.body().iter().collect())
            .unwrap_or_default();
    }
    children
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/module_function_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/ModuleFunction")
            .collect()
    }
    #[test]
    fn flags_extend_self_only_in_direct_module_body() {
        for body in [
            "extend self",
            "extend self\ndef test; end",
            "extend(self)",
            "(extend self)",
        ] {
            let found = offenses(&format!("module Test\n  {body}\nend\n"));
            assert_eq!(found.len(), 1, "{body}");
            assert_eq!(
                (found[0].line, found[0].col),
                (2, if body.starts_with('(') { 4 } else { 3 })
            );
            assert_eq!(
                found[0].message,
                "Use `module_function` instead of `extend self`."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn private_directives_and_lookalikes() {
        for body in [
            "extend self\nprivate",
            "private :test\nextend self",
            "private(&block)\nextend self",
            "extend self\nprivate def test; end",
            "module_function",
            "module_function :test",
            "self.extend self",
            "extend self, Other",
            "extend Other",
            "def f; extend self; end",
            "if condition; extend self; end",
            "class << self; extend self; end",
            "extend(self) {}",
        ] {
            assert!(
                offenses(&format!("module Test\n{body}\nend")).is_empty(),
                "{body}"
            );
        }
        assert!(offenses("class Test; extend self; end").is_empty());
    }
    #[test]
    fn nested_qualified_and_reopened_modules_have_independent_visibility() {
        let s = "module Outer\nprivate\nmodule Inner\nextend self\nend\nend\nmodule Outer::Inner\nextend self\nprivate\nend\nmodule Outer::Inner\nextend self\nend\n";
        let found = offenses(s);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(4, 1), (12, 1)]
        );
        assert_eq!(
            offenses("module M; extend self; def f; private; end; end").len(),
            1
        );
    }
}
