//! `Metrics/ParameterLists` — fixed limits of five total and three optional parameters.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Node, ParametersNode};

pub struct ParameterLists;
impl Cop for ParameterLists {
    fn name(&self) -> &'static str {
        "Metrics/ParameterLists"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some(def) = node.as_def_node() {
            let Some(params) = def.parameters() else {
                return;
            };
            let optional = params.optionals().len();
            if optional > 3 {
                self.report(
                    node.location(),
                    format!("Method has too many optional parameters. [{optional}/3]"),
                    file,
                    out,
                );
            }
            if def.name().as_slice() == b"initialize" && struct_or_data_initializer(cx) {
                return;
            }
            let count = parameter_count(&params);
            if count > 5 {
                let loc = match (def.lparen_loc(), def.rparen_loc()) {
                    (Some(open), Some(close)) => open.join(&close).unwrap(),
                    _ => params.location(),
                };
                self.report(
                    loc,
                    format!("Avoid parameter lists longer than 5 parameters. [{count}/5]"),
                    file,
                    out,
                );
            }
        } else if let Some(block) = node.as_block_node() {
            let Some(params) = block
                .parameters()
                .and_then(|p| p.as_block_parameters_node())
            else {
                return;
            };
            let exempt = cx
                .parent()
                .and_then(|p| p.as_call_node())
                .is_some_and(|call| {
                    call.name().as_slice() == b"lambda"
                        || (!call.is_safe_navigation()
                            && ast::call_arguments(&call).is_empty()
                            && match call.name().as_slice() {
                                b"proc" => call.receiver().is_none(),
                                b"new" => call
                                    .receiver()
                                    .is_some_and(|r| ast::is_global_const(&r, &["Proc"])),
                                _ => false,
                            })
                });
            if exempt {
                return;
            }
            let count =
                params.parameters().map_or(0, |p| parameter_count(&p)) + params.locals().len();
            if count > 5 {
                self.report(
                    params.location(),
                    format!("Avoid parameter lists longer than 5 parameters. [{count}/5]"),
                    file,
                    out,
                );
            }
        }
    }
}

impl ParameterLists {
    fn report(
        &self,
        loc: ruby_prism::Location<'_>,
        message: String,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message,
            correction: None,
        });
    }
}

fn parameter_count(params: &ParametersNode<'_>) -> usize {
    params.requireds().len()
        + params.optionals().len()
        + params.posts().len()
        + params.keywords().len()
        + usize::from(
            params
                .rest()
                .is_some_and(|p| p.as_implicit_rest_node().is_none()),
        )
        + usize::from(params.keyword_rest().is_some())
}

fn struct_or_data_initializer(cx: &NodeContext<'_, '_>) -> bool {
    let mut ancestors = cx.ancestors();
    // RuboCop elides a single-statement body, but a multi-statement begin is
    // a real parent and intentionally prevents this exemption.
    let Some(statements) = ancestors.next().and_then(|n| n.as_statements_node()) else {
        return false;
    };
    if statements.body().len() != 1 {
        return false;
    }
    let Some(block) = ancestors.next().and_then(|n| n.as_block_node()) else {
        return false;
    };
    if block.parameters().is_some_and(|p| {
        p.as_block_parameters_node()
            .is_none_or(|p| !p.locals().is_empty() || p.parameters().is_some())
    }) {
        return false;
    }
    ancestors
        .next()
        .and_then(|n| n.as_call_node())
        .is_some_and(|call| {
            !call.is_safe_navigation()
                && call
                    .receiver()
                    .is_some_and(|r| match call.name().as_slice() {
                        b"new" => ast::is_global_const(&r, &["Struct"]),
                        b"define" => ast::is_global_const(&r, &["Data"]),
                        _ => false,
                    })
        })
}

#[cfg(test)]
mod tests {
    //! Ported/adapted from RuboCop metrics/parameter_lists_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Spec Max=4 is adapted to fixed Max=5.
    use crate::{offense::Offense, pipeline::lint_str_with};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str_with(&[&super::ParameterLists], source)
    }
    #[test]
    fn total_parameters_and_exact_location_message() {
        let found = offenses("def meth(a, b, c, d, e, f)\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 9, 1));
        assert_eq!(
            found[0].message,
            "Avoid parameter lists longer than 5 parameters. [6/5]"
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn optional_limit_is_independent_and_covers_the_definition() {
        let found = offenses("def foo(a=1, b=2, c=3, d=4)\n  work\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 3));
        assert_eq!(
            found[0].message,
            "Method has too many optional parameters. [4/3]"
        );
        assert!(found[0].correction.is_none());
        assert_eq!(offenses("def foo(a=1,b=2,c=3,d=4,e=5,f=6); end").len(), 2);
    }
    #[test]
    fn fixed_default_no_offense_examples_and_non_parameter_lookalikes() {
        for source in [
            "def meth(a,b,c,d,e); end",
            "def foo(a,b=2,c=3,d=4); end",
            "def foo(a,b,c,d,e,&block); end",
            "def foo; end",
            "proc { |a,b,c,d,e,f| }",
            "->(a,b,c,d,e,f) {}",
            "lambda { |a,b,c,d,e,f| }",
            "call(a,b,c,d,e,f)",
            "# def foo(a,b,c,d,e,f)\n'def foo(a,b,c,d,e,f)'",
            "each { _6 }",
            "each { it }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn keyword_rest_forwarding_destructuring_and_block_counting() {
        for params in [
            "a,b,c,d,e:,f:",
            "a,b,c,d,e:1,f:2",
            "a,b,c,d,*rest,z",
            "a,b,c,d,e,**kw",
            "a,b,c,d,e,**nil",
            "a,b,c,d,e,...",
            "(a,b),c,d,e,f,g",
            "a,b,c,d,*,**,&",
        ] {
            let source = format!("def foo({params}); end");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
        for source in [
            "def self.foo(a,b,c,d,e,f) = nil",
            "def foo a,b,c,d,e,f; end",
            "each { |a,b,c,d,e,f; local| }",
            "each { |a,b,c,d,e,f,| }",
            "each { |a,b,c,d,e; local| }",
            "obj.proc { |a,b,c,d,e,f| }",
            "Kernel.proc { |a,b,c,d,e,f| }",
            "Other::Proc.new { |a,b,c,d,e,f| }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "Proc.new { |a,b,c,d,e,f| }",
            "::Proc.new { |a,b,c,d,e,f| }",
            "obj&.lambda(1) { |a,b,c,d,e,f| }",
            "def foo(a,b,c,d,e, &); end",
            "def foo((a,b,c,d,e,f)); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn struct_data_exemption_has_exact_lexical_parent_and_empty_block_args() {
        for receiver in ["Struct.new", "::Struct.new", "Data.define", "::Data.define"] {
            let source = format!("{receiver}(:a) do\n def initialize(a:,b:,c:,d:,e:,f:); end\nend");
            assert!(offenses(&source).is_empty(), "{source}");
            let optional = format!("{receiver}(:a) do\n def initialize(a=1,b=2,c=3,d=4); end\nend");
            assert_eq!(offenses(&optional).len(), 1, "{optional}");
        }
        for outer in [
            "class Foo",
            "Other::Struct.new do",
            "Struct.define do",
            "Struct.new do |x|",
            "Struct.new do; work",
            "Struct.new do; class Foo",
        ] {
            let end = if outer.ends_with("class Foo") && outer.starts_with("Struct") {
                "end; end"
            } else {
                "end"
            };
            let source = format!("{outer}\n def initialize(a:,b:,c:,d:,e:,f:); end\n{end}");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
    }
    #[test]
    fn multiline_directives_and_crlf_use_offense_span() {
        for prefix in ["rubocop", "rustocop"] {
            let source = format!(
                "def foo(a,b,c,\r\n d,e,f) # {prefix}:disable Metrics/ParameterLists\r\nend\r\n"
            );
            assert!(offenses(&source).is_empty());
            let source = format!("# {prefix}:disable Metrics\ndef foo(a,b,c,d,e,f); end\n");
            assert!(offenses(&source).is_empty());
        }
        let found = offenses("def foo(a,b,c,\r\n d,e,f)\r\nend\r\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 8, 2));
    }
}
