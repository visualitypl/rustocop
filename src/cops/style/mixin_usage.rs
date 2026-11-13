//! `Style/MixinUsage` — bare constant mixins at the top level.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct MixinUsage;
impl Cop for MixinUsage {
    fn name(&self) -> &'static str {
        "Style/MixinUsage"
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
        let args = crate::cops::ast::call_arguments(&call);
        if !mixin_call(&call)
            || args.is_empty()
            || !args.iter().all(|arg| {
                matches!(
                    arg,
                    Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. }
                )
            })
        {
            return;
        }
        if !cx.ancestors().all(|parent| match parent {
            Node::ProgramNode { .. }
            | Node::StatementsNode { .. }
            | Node::ParenthesesNode { .. }
            | Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::ElseNode { .. } => true,
            Node::BeginNode { .. } => parent
                .as_begin_node()
                .is_some_and(|b| b.rescue_clause().is_none() && b.ensure_clause().is_none()),
            Node::DefNode { .. } => parent.as_def_node().is_some_and(|d| d.receiver().is_none()),
            _ => false,
        }) {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "`{}` is used at the top level. Use inside `class` or `module`.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}

pub(super) fn mixin_call(call: &ruby_prism::CallNode<'_>) -> bool {
    call.receiver().is_none()
        && !call
            .block()
            .is_some_and(|block| block.as_block_node().is_some())
        && matches!(call.name().as_slice(), b"include" | b"extend" | b"prepend")
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/mixin_usage_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/MixinUsage")
            .collect()
    }
    #[test]
    fn top_level_constants_and_transparent_scopes() {
        for method in ["include", "extend", "prepend"] {
            for (prefix, suffix, line, col) in [
                ("", "", 1, 1),
                ("class C; end\n", "", 2, 1),
                ("", "\nclass C; end", 1, 1),
                ("def foo\n  ", "\nend", 2, 3),
                ("begin\n  ", "\nend", 2, 3),
                ("if x\n  ", "\nend", 2, 3),
                ("(", ")", 1, 2),
                ("x ? (", ") : nil", 1, 6),
            ] {
                let s = format!("{prefix}{method} M1::M2, ::M3{suffix}");
                let found = offenses(&s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col), (line, col), "{s}");
                assert_eq!(
                    found[0].message,
                    format!("`{method}` is used at the top level. Use inside `class` or `module`.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn ignores_receivers_nonconstants_and_nontransparent_scopes() {
        for method in ["include", "extend", "prepend"] {
            for s in [
                format!("Foo.{method} M"),
                format!("self.{method} M"),
                method.to_string(),
                format!("{method} M, value"),
                format!("{method} *M"),
                format!("{method} M, &block"),
                format!("{method} M {{}}"),
                format!("class C; {method} M; end"),
                format!("module M1; class C; {method} M; end; end"),
                format!("class << self; {method} M; end"),
                format!("Class.new {{ {method} M }}"),
                format!("klass.class_eval {{ {method} M if defined?(M) }}"),
                format!("def self.foo; {method} M; end"),
                format!("class X; def foo; {method} M; end; end"),
                format!("do_something({method}(M))"),
                format!("while x; {method} M; end"),
                format!("x && {method}(M)"),
                format!("value = {method}(M)"),
            ] {
                assert!(offenses(&s).is_empty(), "{s}");
            }
        }
    }
}
