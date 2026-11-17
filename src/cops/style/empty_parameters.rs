//! Shared explicit-empty-parameter syntax (not implicit numbered/it parameters).
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{self, NodeVisitor},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{BlockParametersNode, Node};
use std::collections::HashSet;

pub(super) fn empty(parameters: Option<Node<'_>>) -> Option<BlockParametersNode<'_>> {
    let parameters = parameters?.as_block_parameters_node()?;
    (parameters.parameters().is_none()
        && parameters.locals().is_empty()
        && parameters.opening_loc().is_some()
        && parameters.closing_loc().is_some())
    .then_some(parameters)
}

pub(super) fn report(
    cop: &dyn Cop,
    parameters: &BlockParametersNode<'_>,
    start: usize,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
    message: &str,
) {
    let (line, col) = file.line_col(parameters.location().start_offset());
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        line,
        col,
        end_line: file.last_line_of(parameters.location()),
        message: message.into(),
        correction: cx
            .corrections_requested()
            .then(|| correction(parameters, start, cx, file))
            .flatten(),
    });
}

fn correction(
    parameters: &BlockParametersNode<'_>,
    start: usize,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    // The parameter delimiters identify the scope in the correction-only index.
    let root = cx.ancestors().next_back()?;
    let left = parameters.opening_loc()?;
    let right = parameters.closing_loc()?;
    let range = start..right.end_offset();
    // Never delete comments, continuation lines, CRLFs or literal bytes. Keeping
    // physical lines also preserves heredocs, directives and executable __LINE__.
    if !file.bytes[start..left.start_offset()]
        .iter()
        .all(|b| matches!(b, b' ' | b'\t'))
        || !file.bytes[left.end_offset()..right.start_offset()]
            .iter()
            .all(|b| matches!(b, b' ' | b'\t'))
    {
        return None;
    }
    if file
        .empty_parameter_facts(root)
        .implicit_it
        .contains(&left.start_offset())
    {
        return None;
    }
    // `do||value` must become `do value`, not the identifier `dovalue`.
    let separator = file.bytes.get(start.wrapping_sub(1)) == Some(&b'o')
        && file
            .bytes
            .get(range.end)
            .is_some_and(|b| !b.is_ascii_whitespace());
    let edit = Correction {
        range,
        replacement: if separator { " " } else { "" }.into(),
    };
    Some(edit)
}

/// One complete main-AST pass shared by both consumers. Unbound `it` reads in
/// the scope whose parameters disappear are unsafe (and normally syntax errors
/// at 3.4 already). Captured locals are LocalVariableReadNodes. Nested blocks
/// and closed lexical scopes are barriers; no descendant rescans or reparses.
#[derive(Default)]
pub(crate) struct Facts {
    implicit_it: HashSet<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Index {
            facts: Facts,
            scopes: Vec<Option<usize>>,
            entered_scope: Vec<bool>,
        }
        impl<'pr> NodeVisitor<'pr> for Index {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                let block = node.as_block_node();
                let lambda = node.as_lambda_node();
                let scope = block.is_some()
                    || lambda.is_some()
                    || matches!(
                        node,
                        Node::ProgramNode { .. }
                            | Node::DefNode { .. }
                            | Node::ClassNode { .. }
                            | Node::ModuleNode { .. }
                            | Node::SingletonClassNode { .. }
                    );
                if scope {
                    let candidate = if let Some(block) = block {
                        empty(block.parameters())
                            .and_then(|p| p.opening_loc())
                            .map(|l| l.start_offset())
                    } else if let Some(lambda) = lambda {
                        empty(lambda.parameters())
                            .and_then(|p| p.opening_loc())
                            .map(|l| l.start_offset())
                    } else {
                        None
                    };
                    self.scopes.push(candidate);
                }
                self.entered_scope.push(scope);
                if (node.as_it_local_variable_read_node().is_some()
                    || node.as_call_node().is_some_and(|call| {
                        call.is_variable_call()
                            && call.receiver().is_none()
                            && call.name().as_slice() == b"it"
                    }))
                    && let Some(Some(key)) = self.scopes.last()
                {
                    self.facts.implicit_it.insert(*key);
                }
            }
            fn leave(&mut self) {
                if self.entered_scope.pop() == Some(true) {
                    self.scopes.pop();
                }
            }
        }
        let mut index = Index::default();
        walk::walk(&mut index, root);
        index.facts
    }
}

#[cfg(test)]
mod tests {
    use crate::cops::{
        Cop,
        style::{EmptyBlockParameter, EmptyLambdaParameter},
        testing::{assert_correction, correction_offenses},
    };

    fn cases() -> [(&'static dyn Cop, &'static str, &'static str); 2] {
        [
            (&EmptyBlockParameter, "proc { ||", "proc {"),
            (&EmptyLambdaParameter, "->() {", "-> {"),
        ]
    }
    #[test]
    fn invalid_unbound_it_is_rejected_and_defensive_main_ast_facts_mark_its_scope() {
        for (cop, prefix, _) in cases() {
            for body in [
                "it",
                "(it)",
                "defined?(it)",
                "[it]",
                "it.to_s",
                "\"#{it}\"",
                "false ? it : 1",
            ] {
                let source = format!("{prefix} {body} }}");
                let parsed = ruby_prism::parse(source.as_bytes());
                assert!(parsed.errors().count() > 0, "{source}");
                let facts = super::Facts::build(&parsed.node());
                assert_eq!(facts.implicit_it.len(), 1, "{source}");
                assert!(correction_offenses(cop, &source).is_empty());
                let naive = source.replace(" ||", "").replace("->()", "->");
                // Naive removal enables Ruby 3.4 implicit it instead of rejecting it.
                let ast = format!("{:?}", ruby_prism::parse(naive.as_bytes()).node());
                assert!(
                    ast.contains("ItParametersNode") && ast.contains("ItLocalVariableReadNode"),
                    "{naive}: {ast}"
                );
            }
        }
    }
    #[test]
    fn explicit_calls_bound_locals_and_nested_scope_barriers_are_safe() {
        for (cop, before, after) in cases() {
            for body in [
                "it()",
                "self.it",
                "it(1)",
                "it { 1 }",
                "it = 1; it",
                "inner { it }",
                "inner { _1 }",
                "inner { |x| it() }",
                "def f; it; end",
                "inner { |;it| it = 1; it }",
            ] {
                assert_correction(
                    cop,
                    &format!("{before} {body} }}"),
                    &format!("{after} {body} }}"),
                );
            }
            for prefix in ["it = 42; ", "outer { |it| "] {
                let suffix = if prefix.contains("outer") { " }" } else { "" };
                assert_correction(
                    cop,
                    &format!("{prefix}{before} it }}{suffix}"),
                    &format!("{prefix}{after} it }}{suffix}"),
                );
            }
        }
    }
    #[test]
    fn nested_uncertain_scope_does_not_block_safe_outer_or_sibling() {
        assert_correction(
            &EmptyBlockParameter,
            "outer { || inner { | # keep\n| 1 }; sibling { || 1 } }",
            "outer { inner { | # keep\n| 1 }; sibling { 1 } }",
        );
        assert_correction(
            &EmptyLambdaParameter,
            "->() { [->( # keep\n) { 1 }, ->() { 1 }] }",
            "-> { [->( # keep\n) { 1 }, -> { 1 }] }",
        );
    }
    #[test]
    fn token_separation_empty_bodies_and_horizontal_delimiters() {
        for (before, after) in [
            ("a do||value end", "a do value end"),
            ("a do||end", "a do end"),
            ("a{||value}", "a{value}"),
            ("a{||}", "a{}"),
            ("a {  |  | value }", "a { value }"),
            ("a do\t| |\nend", "a do\nend"),
            ("obj&.a { || 1 }", "obj&.a { 1 }"),
            (
                "def f; super(1) { || 2 }; end",
                "def f; super(1) { 2 }; end",
            ),
        ] {
            assert_correction(&EmptyBlockParameter, before, after);
        }
        for (before, after) in [
            ("->(){}", "->{}"),
            ("->()do 1 end", "->do 1 end"),
            ("->  (  ) { 1 }", "-> { 1 }"),
            ("->() do end", "-> do end"),
        ] {
            assert_correction(&EmptyLambdaParameter, before, after);
        }
    }
    #[test]
    fn multiline_or_commented_parameter_ranges_are_diagnostic_only() {
        for (cop, sources) in [
            (
                &EmptyBlockParameter as &dyn Cop,
                vec![
                    "a { |\n| __LINE__ }",
                    "a { | # keep\n| 1 }",
                    "a { # keep\n|| 1 }",
                    "a do\r\n|| 1 end",
                ],
            ),
            (
                &EmptyLambdaParameter as &dyn Cop,
                vec![
                    "->(\n) { __LINE__ }",
                    "->( # keep\n) { 1 }",
                    "->(\r\n) { 1 }",
                ],
            ),
        ] {
            for source in sources {
                let found = correction_offenses(cop, source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn comments_heredocs_and_line_values_outside_edit_survive() {
        for (cop, prefix, after) in cases() {
            for body in [
                "# retain\n __LINE__",
                "<<~TEXT\n  café #{__LINE__}\nTEXT\n",
                "x = <<~TEXT\n  it || ()\nTEXT\n x",
            ] {
                assert_correction(
                    cop,
                    &format!("{prefix} {body} }}\np __LINE__\n"),
                    &format!("{after} {body} }}\np __LINE__\n"),
                );
            }
        }
    }
    #[test]
    fn unicode_and_multiline_highlights_use_exact_parameter_locations() {
        use crate::pipeline::lint_str;
        for (source, cop, position, end_line, message) in [
            (
                "# header\né { || 1 }\n",
                "Style/EmptyBlockParameter",
                (2, 5),
                2,
                "Omit pipes for the empty block parameters.",
            ),
            (
                "# header\nf = -> (\n) { 1 }\n",
                "Style/EmptyLambdaParameter",
                (2, 8),
                3,
                "Omit parentheses for the empty lambda parameters.",
            ),
            (
                "a { |\n| 1 }\n",
                "Style/EmptyBlockParameter",
                (1, 5),
                2,
                "Omit pipes for the empty block parameters.",
            ),
        ] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == cop)
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), position);
            assert_eq!(found[0].end_line, end_line);
            assert_eq!(found[0].message, message);
        }
    }
}
