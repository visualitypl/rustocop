//! `Style/LambdaCall` — fixed explicit `call` spelling.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct LambdaCall;
impl Cop for LambdaCall {
    fn name(&self) -> &'static str {
        "Style/LambdaCall"
    }
    fn safe_fixes(&self) -> bool {
        true
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
        if !implicit_call(node) {
            return;
        }
        let start = call.location().start_offset();
        let end = ast::source_end(node);
        let range = start..end;
        // Match upstream's deliberate comment/heredoc detection exemptions.
        if super::correction::contains_comment(file, range.clone()) {
            return;
        }
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        if ast::call_arguments(&call)
            .iter()
            .any(|arg| file.contains_heredoc(root, arg))
        {
            return;
        }
        let Some(open) = call.opening_loc() else {
            return;
        };
        let Some(close) = call.closing_loc() else {
            return;
        };
        // A full send span intentionally gives the outer call priority for a
        // chained sugar call, matching upstream's one-pass ignored-node rule.
        // Unlike rebuilding arguments, retain every literal and physical line.
        let mut bytes = file.bytes[start..open.start_offset()].to_vec();
        bytes.extend_from_slice(b"call");
        let empty =
            call.arguments().is_none() && call.block().is_none_or(|b| b.as_block_node().is_some());
        let interior = &file.bytes[open.end_offset()..close.start_offset()];
        if !empty
            || interior.iter().any(|b| matches!(b, b'\n' | b'\r'))
            || !can_omit_empty_parens(file, close.end_offset())
        {
            bytes.extend_from_slice(&file.bytes[open.start_offset()..end]);
        } else {
            bytes.extend_from_slice(&file.bytes[close.end_offset()..end]);
        }
        let replacement = String::from_utf8_lossy(&bytes).into_owned();
        let (line, col) = file.line_col(start);
        let edit = Correction {
            range,
            replacement: replacement.clone(),
        };
        let correction = (cx.corrections_requested()
            && std::str::from_utf8(&bytes).is_ok()
            && !file
                .lambda_call_facts(root)
                .suppressed
                .contains(&(start, end)))
        .then_some(edit);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!(
                "Prefer the use of `{replacement}` over `{}`.",
                String::from_utf8_lossy(&file.bytes[start..end])
            ),
            correction,
        });
    }
}
// A tight RHS/operator/index can otherwise become a command argument, and an
// adjacent keyword can merge into `call`. Keep () unless the local terminator
// proves omission harmless; never reparse each candidate or move a newline.
fn can_omit_empty_parens(file: &SourceFile, end: usize) -> bool {
    file.bytes[end..]
        .iter()
        .find(|b| !matches!(b, b' ' | b'\t'))
        .is_none_or(|b| {
            matches!(
                b,
                b'\n' | b'\r' | b';' | b',' | b')' | b']' | b'}' | b'.' | b'{' | b'#'
            )
        })
}

fn implicit_call(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.receiver().is_some()
            && call.name().as_slice() == b"call"
            && call.message_loc().is_none()
    })
}

/// Outermost sugar sends own the one-pass correction. Cache ownership in one
/// complete main-AST walk, rather than scanning a deep chain's ancestors for
/// every call. Literal blocks outside a send span are independent candidates.
#[derive(Default)]
pub(crate) struct Facts {
    suppressed: std::collections::HashSet<(usize, usize)>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Index {
            facts: Facts,
            end: usize,
            stack: Vec<usize>,
        }
        impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Index {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                self.stack.push(self.end);
                if implicit_call(&node) {
                    let end = ast::source_end(&node);
                    if self.end >= end {
                        self.facts
                            .suppressed
                            .insert((node.location().start_offset(), end));
                    }
                    self.end = self.end.max(end);
                }
            }
            fn leave(&mut self) {
                self.end = self.stack.pop().unwrap();
            }
        }
        let mut index = Index::default();
        crate::cops::walk::walk(&mut index, root);
        index.facts
    }
}

/// Correction-only ownership of nested sugar sends, indexed in linear work.
pub(crate) static LAMBDA_CALL_FACTS: crate::source::Fact<Facts> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn lambda_call_facts(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        LAMBDA_CALL_FACTS.get(self, || Facts::build(root))
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/lambda_call_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn pinned_diagnostics_and_corrections() {
        for (before, after) in [
            ("x.(a, b)\n", "x.call(a, b)\n"),
            ("x&.(a, b)\n", "x&.call(a, b)\n"),
            ("x.(a, b).(c)\n", "x.(a, b).call(c)\n"),
            ("x&.(a, b)&.(c)\n", "x&.(a, b)&.call(c)\n"),
            ("x.()\n", "x.call\n"),
            ("x.call(a, b)\nx.(a, b)\n", "x.call(a, b)\nx.call(a, b)\n"),
            (
                "x&.call(a, b)\nx&.(a, b)\n",
                "x&.call(a, b)\nx&.call(a, b)\n",
            ),
            (
                "x.call(a, b)\nx.(a, b)\nx.(a, b)\n",
                "x.call(a, b)\nx.call(a, b)\nx.call(a, b)\n",
            ),
            (
                "x&.call(a, b)\nx&.(a, b)\nx&.(a, b)\n",
                "x&.call(a, b)\nx&.call(a, b)\nx&.call(a, b)\n",
            ),
        ] {
            assert_correction(&LambdaCall, before, after);
        }
        let found: Vec<_> = lint_str("\n  x.(a, b)\n")
            .into_iter()
            .filter(|o| o.cop == LambdaCall.name())
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(
            found[0].message,
            "Prefer the use of `x.call(a, b)` over `x.(a, b)`."
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn pinned_negatives_and_dispatch_lookalikes() {
        for source in [
            "x.(<<~TEXT)\n text\nTEXT\n",
            "x.(a: <<~TEXT.chomp)\n text\nTEXT\n",
            "x.(\n a, # keep\n b\n)",
            "x.call(a, b)",
            "x&.call",
            "call(a, b)",
            "x[a, b]",
            "x.[](a)",
            "Proc.new { 1 }",
            "'x.()' # x.()",
        ] {
            assert!(
                !lint_str(source).iter().any(|o| o.cop == LambdaCall.name()),
                "{source}"
            );
        }
    }
    #[test]
    fn empty_calls_keep_delimiters_when_omission_changes_token_binding() {
        for (before, after) in [
            ("x.() +2", "x.call() +2"),
            ("x.() -2", "x.call() -2"),
            ("x.() *xs", "x.call() *xs"),
            ("x.() [2]", "x.call() [2]"),
            ("x.() ?true : false", "x.call() ?true : false"),
            ("x.()and true", "x.call()and true"),
            ("x.()do 1 end", "x.call()do 1 end"),
        ] {
            assert_correction(&LambdaCall, before, after);
        }
    }

    #[test]
    fn nested_ownership_excludes_literal_block_bodies_and_is_single_pass() {
        for (before, after) in [
            ("x.() { y.() }", "x.call { y.call }"),
            ("x.(y.())", "x.call(y.())"),
            ("x.(a: y.())", "x.call(a: y.())"),
            ("(x.() { y.() }).()", "(x.() { y.() }).call"),
            // An exempt outer send still owns its uncertain descendants, whose
            // offenses remain visible but are individually not corrected.
            ("x.(y.(), # keep\n z)", "x.(y.(), # keep\n z)"),
        ] {
            assert_correction(&LambdaCall, before, after);
        }
    }
}
