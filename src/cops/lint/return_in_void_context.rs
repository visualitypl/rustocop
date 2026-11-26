//! `Lint/ReturnInVoidContext` — values returned by constructors and setters.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Offense, single_line},
    source::SourceFile,
};

pub struct ReturnInVoidContext;

impl Cop for ReturnInVoidContext {
    fn name(&self) -> &'static str {
        "Lint/ReturnInVoidContext"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_return_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(ret) = node.as_return_node() else {
            return;
        };
        // RuboCop checks for AST descendants, not a non-nil value. An empty
        // parenthesized expression still counts; a bare return does not.
        if ret
            .arguments()
            .is_none_or(|args| args.arguments().is_empty())
        {
            return;
        }
        let Some(def) = cx.ancestors().find_map(Node::as_def_node) else {
            return;
        };
        let name = def.name().as_slice();
        let assignment =
            name.ends_with(b"=") && !matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=");
        if !(assignment || (def.receiver().is_none() && name == b"initialize")) {
            return;
        }
        // Search ALL block ancestors, even beyond the nearest def. RuboCop's
        // block wraps its send as well as its body, so a scope-changing call
        // with a literal block also excludes returns in its receiver/arguments.
        if cx.ancestors().any(|ancestor| {
            ancestor.as_lambda_node().is_some()
                || ancestor.as_call_node().is_some_and(|call| {
                    matches!(
                        call.name().as_slice(),
                        b"lambda" | b"define_method" | b"define_singleton_method"
                    ) && call.block().is_some_and(|b| b.as_block_node().is_some())
                })
        }) {
            return;
        }
        let loc = ret.keyword_loc();
        let (line, col) = file.reference_line_col(loc.start_offset());
        let method = single_line(&file.match_pattern_source(def.name_loc().as_slice()));
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: format!("Do not return a value in `{method}`."),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/return_in_void_context_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/ReturnInVoidContext")
            .collect()
    }

    #[test]
    fn pinned_constructor_and_setter_keyword_findings() {
        for (header, method) in [("initialize", "initialize"), ("foo=(bar)", "foo=")] {
            for value in [":qux if bar?", "nil", "42", "(\n    :qux\n  )"] {
                let source = format!("class A\n def {header}\n  return {value}\n end\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 3, 3));
                assert_eq!(
                    found[0].message,
                    format!("Do not return a value in `{method}`.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn pinned_blocks_numblocks_itblocks_and_procs_keep_findings() {
        for block in ["foo", "proc", "Proc.new"] {
            let source = format!("def initialize\n {block} do\n  return :qux\n end\nend\n");
            assert!(!offenses(&source).is_empty(), "{source}");
        }
        for parameter in ["_1", "it"] {
            let source =
                format!("def initialize\n foo do\n  {parameter}\n  return :qux\n end\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (4, 3));
        }
    }

    #[test]
    fn setters_include_singleton_and_index_but_not_comparison_operators() {
        for (header, method) in [
            ("self.foo=(v)", "foo="),
            ("[]=(k, v)", "[]="),
            ("self.[]=(k, v)", "[]="),
            ("é=(v)", "é="),
        ] {
            let source = format!("def {header}; return (); end");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                found[0].message,
                format!("Do not return a value in `{method}`.")
            );
        }
        for header in [
            "==(v)", "===(v)", "!=(v)", "<=(v)", ">=(v)", "<=>(v)", "=~(v)",
        ] {
            let source = format!("def {header}; return 1; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        let found = offenses("def initialize = (return 1)");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 19, 1));
    }

    #[test]
    fn scope_changing_blocks_above_nearest_def_still_exclude() {
        for call in [
            "lambda",
            "->",
            "self.define_method(:f)",
            "o&.define_singleton_method(:f)",
        ] {
            for header in ["initialize", "self.foo=(v)"] {
                let source = format!("{call} {{ def {header}; return 1; end }}");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        let source = "proc { def initialize; return 1; end }";
        assert_eq!(offenses(source).len(), 1);
    }

    #[test]
    fn synthetic_block_ancestors_cover_call_arguments_not_only_block_bodies() {
        for call in ["lambda", "self.define_method", "o&.define_singleton_method"] {
            let source = format!("{call}(def initialize; return 1; end) {{}}");
            assert!(offenses(&source).is_empty(), "{source}");
            // A block-pass isn't an any_block AST ancestor. A method definition
            // is a valid argument value; a direct return there is parser-fatal.
            let source = format!("{call}(def initialize; return 1; end, &block)");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
    }

    #[test]
    fn pinned_no_value_and_nonvoid_lookalikes() {
        for source in [
            "def initialize; return if bar?; end",
            "def initialize; foo { return if bar? }; end",
            "def foo=(bar); return; end",
            "def bar; foo; return :qux if bar?; foo; end",
            "def self.initialize; foo; return :qux if bar?; foo; end",
            "return if true",
            "return 1",
            "def initialize; def foo; return bar; end; end",
            "def initialize; def self.foo; return bar; end; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_scope_changing_blocks_with_or_without_receivers() {
        for method in [
            "lambda",
            "define_method",
            "self.define_method",
            "define_singleton_method",
            "->",
        ] {
            let args = if method.contains("define") {
                "(:foo)"
            } else {
                ""
            };
            let source = format!("def initialize; {method}{args} {{ return bar }}; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
}
