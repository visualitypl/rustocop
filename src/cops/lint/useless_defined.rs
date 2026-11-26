//! `Lint/UselessDefined` — literal string/symbol operands of `defined?`.

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct UselessDefined;
impl Cop for UselessDefined {
    fn name(&self) -> &'static str {
        "Lint/UselessDefined"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_defined_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(defined) = node.as_defined_node() else {
            return;
        };
        let kind = match defined.value() {
            Node::StringNode { .. }
            | Node::InterpolatedStringNode { .. }
            | Node::SourceFileNode { .. } => "string",
            Node::SymbolNode { .. } | Node::InterpolatedSymbolNode { .. } => "symbol",
            _ => return,
        };
        let (line, col) = file.reference_line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Calling `defined?` with a {kind} argument will always return a truthy value."
            ),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_defined_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessDefined")
            .collect()
    }
    #[test]
    fn pinned_literals_and_synthetic_file_string() {
        for (argument, kind) in [
            ("\"FooBar\"", "string"),
            ("\"Foo#{bar}\"", "string"),
            (":FooBar", "symbol"),
            (":\"Foo#{bar}\"", "symbol"),
            ("__FILE__", "string"),
            ("?a", "string"),
        ] {
            let source = format!("# example\n  defined?({argument})\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!(
                    "Calling `defined?` with a {kind} argument will always return a truthy value."
                )
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_and_grouped_lookalikes() {
        for argument in [
            "FooBar",
            "foo_bar",
            ":foo.to_proc",
            "(\"FooBar\")",
            "begin; :foo; end",
            "1",
            "nil",
            "__ENCODING__",
            "[\"x\"]",
            "/x/",
            "`x`",
        ] {
            assert!(
                offenses(&format!("defined?({argument})")).is_empty(),
                "{argument}"
            );
        }
    }
}
