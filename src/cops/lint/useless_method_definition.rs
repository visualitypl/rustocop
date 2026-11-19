//! `Lint/UselessMethodDefinition` — methods that only delegate to super.

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Node, ParametersNode};

pub struct UselessMethodDefinition;
impl Cop for UselessMethodDefinition {
    fn name(&self) -> &'static str {
        "Lint/UselessMethodDefinition"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        // Arguments is transparent in the reference AST; parentheses and
        // statement sequences are not. csend is deliberately NOT send here.
        let parent = cx.ancestors().find(|n| n.as_arguments_node().is_none());
        if let Some(call) = parent.and_then(Node::as_call_node)
            && !call.is_safe_navigation()
            && !(call.receiver().is_none()
                && matches!(
                    call.name().as_slice(),
                    b"public" | b"protected" | b"private" | b"module_function"
                )
                && {
                    let ancestors: Vec<_> = cx
                        .ancestors()
                        .skip_while(|n| n.as_arguments_node().is_some())
                        .skip(1)
                        .collect();
                    super::useless_constant_scoping::method_modifier_scope(
                        &call.as_node(),
                        &ancestors,
                        file,
                    )
                })
        {
            return;
        }
        let parameters = def.parameters().map(|p| parameters(&p)).unwrap_or_default();
        if parameters.iter().any(|p| {
            matches!(
                p,
                Node::RestParameterNode { .. }
                    | Node::OptionalParameterNode { .. }
                    | Node::OptionalKeywordParameterNode { .. }
                    | Node::KeywordRestParameterNode { .. }
            )
        }) {
            return;
        }
        let Some(body) = def
            .body()
            .and_then(|n| n.as_statements_node())
            .filter(|s| s.body().len() == 1)
            .and_then(|s| s.body().iter().next())
        else {
            return;
        };
        let delegating = if let Some(call) = body.as_forwarding_super_node() {
            call.block().is_none()
        } else if let Some(call) = body.as_super_node() {
            if call.block().is_some_and(|b| b.as_block_node().is_some()) {
                return;
            }
            let mut arguments: Vec<_> = call
                .arguments()
                .map(|a| a.arguments().iter().collect())
                .unwrap_or_default();
            arguments.extend(call.block());
            arguments.len() == parameters.len()
                && arguments
                    .iter()
                    .zip(&parameters)
                    .all(|(a, p)| source(a, file) == source(p, file))
        } else {
            false
        };
        if !delegating {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Useless method definition detected.".into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            // The definition goes, with a call it is the argument of.
            correction: cx.corrections_requested().then(|| {
                let target = cx
                    .ancestors()
                    .find(|n| n.as_arguments_node().is_none())
                    .filter(|p| p.as_call_node().is_some_and(|c| !c.is_safe_navigation()))
                    .map_or_else(|| node.location(), |p| p.location());
                crate::offense::Correction {
                    range: target.start_offset()..target.end_offset(),
                    replacement: String::new(),
                }
            }),
        });
    }
}

fn parameters<'pr>(p: &ParametersNode<'pr>) -> Vec<Node<'pr>> {
    p.requireds()
        .iter()
        .chain(p.optionals().iter())
        .chain(p.rest())
        .chain(p.posts().iter())
        .chain(p.keywords().iter())
        .chain(p.keyword_rest())
        .chain(p.block().map(|b| b.as_node()))
        .collect()
}

fn source(node: &Node<'_>, file: &SourceFile) -> Vec<u8> {
    let source = file.match_pattern_source(node.location().as_slice());
    let mut bytes = source.iter().copied().peekable();
    let mut result = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte != b'\r' || bytes.peek() != Some(&b'\n') {
            result.push(byte);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_method_definition_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessMethodDefinition")
            .collect()
    }
    #[test]
    fn pinned_delegation_and_modifiers() {
        for (prefix, col) in [
            ("", 1),
            ("public ", 8),
            ("protected ", 11),
            ("private ", 9),
            ("module_function ", 17),
        ] {
            for (head, body) in [
                ("f", "super"),
                ("self.f", "super"),
                ("f(arg)", "super(arg)"),
                ("self.f(arg)", "super(arg)"),
                ("f", "super()"),
            ] {
                let source = format!("{prefix}def {head}\n {body}\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (1, col, 3)
                );
                assert_eq!(found[0].message, "Useless method definition detected.");
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "def initialize(a,b); end",
            "def initialize(a); # comment\nend",
            "def self.initialize; end",
            "def f; super; work; end",
            "def f(a); super(b); end",
            "def f(a,b); super(b,a); end",
            "def f(a,b); super(); end",
            "def f(*args); super; end",
            "def f(x=1); super; end",
            "def f(x: 1); super; end",
            "def f(**options); super; end",
            "do_something def f; super; end",
            "def non_constructor; # comment\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn modifier_must_be_in_actual_macro_scope() {
        for (source, n) in [
            ("def outer; private def f; super; end; end", 0),
            ("consume(private def f;super;end)", 0),
            ("x = private def f;super;end", 0),
            ("while flag; private def f;super;end;end", 0),
            ("if private def f;super;end; nil;end", 0),
            ("class C; private def f;super;end;end", 1),
            ("proc { private def f;super;end }", 1),
            ("if flag; private def f;super;end;end", 1),
            ("Class.new(private def f;super;end)", 1),
            ("consume(private(def f;super;end)) { nil }", 0),
        ] {
            assert_eq!(offenses(source).len(), n, "{source}");
        }
    }

    #[test]
    fn exact_sources_not_semantic_argument_equivalence() {
        for (source, n) in [
            ("def f(k:); super; end", 1),
            ("def f(k:); super(k:); end", 1),
            ("def f(k:); super(k: k); end", 0),
            ("def f(&b); super(&b); end", 1),
            ("def f(&b); super; end", 1),
            ("def f(a); super((a)); end", 0),
            ("def f(...); super; end", 1),
            ("def f(...); super(...); end", 1),
            ("def f(**nil); super; end", 1),
            ("def f(*); super; end", 0),
            ("def f; (super); end", 0),
            ("def f; begin; super; end; end", 0),
            ("def f; return super; end", 0),
            ("def f; super {}; end", 0),
            ("object.private def f; super; end", 0),
            ("object&.private def f; super; end", 1),
            ("wrap(def f; super; end)", 0),
            ("wrap((def f; super; end))", 1),
        ] {
            assert_eq!(offenses(source).len(), n, "{source}");
        }
    }
}
