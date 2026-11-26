//! `Lint/UnusedMethodArgument` — fixed-default unused method parameters.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        variables::{Declaration, ParameterKind, ScopeAnalysis, ScopeKind},
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashSet;

/// Checks explicit method arguments using the shared pinned VariableForce view.
pub struct UnusedMethodArgument;
impl Cop for UnusedMethodArgument {
    fn name(&self) -> &'static str {
        "Lint/UnusedMethodArgument"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/ExplicitBlockArgument"]
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(program) = node.as_program_node() else {
            return;
        };
        let analysis = ScopeAnalysis::for_underscore_variables(&program, file);
        // Yield in ANY body descendant exempts &block, including nested defs.
        // Propagate subtree facts once on exit, not a root/body scan per argument.
        let mut yields = Yields::default();
        walk(&mut yields, node);
        for id in analysis.scopes_in_exit_order() {
            let scope = analysis.scope(id);
            if scope.kind != ScopeKind::Def {
                continue;
            }
            let Some(body) = scope.body() else { continue };
            if not_implemented(body) {
                continue;
            }
            let def = scope.node.as_def_node().unwrap();
            let args: Vec<_> = scope
                .variables()
                .map(|v| analysis.variable(v))
                .filter(|v| v.is_parameter())
                .collect();
            let all_unused = args.iter().all(|v| !v.is_referenced());
            for variable in args {
                if variable.should_be_unused()
                    || variable.is_referenced()
                    || (variable.declaration == Declaration::Parameter(ParameterKind::Block)
                        && yields.defs.contains(&scope.node.location().start_offset()))
                {
                    continue;
                }
                let name = reference_name(variable.name, file);
                let mut message = format!("Unused method argument - `{name}`.");
                if !matches!(
                    variable.declaration,
                    Declaration::Parameter(
                        ParameterKind::RequiredKeyword | ParameterKind::OptionalKeyword
                    )
                ) {
                    message.push_str(&format!(" If it's necessary, use `_` or `_{name}` as an argument name to indicate that it won't be used. If it's unnecessary, remove it."));
                }
                if all_unused {
                    let method = reference_name(def.name().as_slice(), file);
                    message.push_str(&format!(" You can also write as `{method}(*)` if you want the method to accept any arguments but don't care about them."));
                }
                let (line, col) = file.reference_line_col(variable.declaration_range.start);
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    line,
                    col,
                    end_line: line,
                    message,
                    correction: cx
                        .corrections_requested()
                        .then(|| correction(variable, file))
                        .flatten(),
                });
            }
        }
    }
}
/// RuboCop's `UnusedArgCorrector`.
fn correction(
    variable: &crate::cops::variables::Variable<'_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let start = variable.declaration_range.start;
    match variable.declaration {
        Declaration::Parameter(ParameterKind::RequiredKeyword | ParameterKind::OptionalKeyword) => {
            None
        }
        Declaration::Parameter(ParameterKind::Block) => {
            let mut begin = start;
            while begin > 0 && file.bytes[begin - 1] != b'&' {
                begin -= 1;
            }
            begin = begin.checked_sub(1)?;
            while begin > 0 && matches!(file.bytes[begin - 1], b' ' | b'\t') {
                begin -= 1;
            }
            while begin > 0 && file.bytes[begin - 1] == b'\n' {
                begin -= 1;
            }
            while begin > 0 && file.bytes[begin - 1] == b',' {
                begin -= 1;
            }
            Some(crate::offense::Correction {
                range: begin..start + variable.name.len(),
                replacement: String::new(),
            })
        }
        _ => Some(crate::offense::Correction {
            range: start..start + variable.name.len(),
            replacement: format!("_{}", std::str::from_utf8(variable.name).ok()?),
        }),
    }
}
fn reference_name(name: &[u8], file: &SourceFile) -> String {
    file.match_pattern_source(name)
        .utf8_chunks()
        .map(|chunk| {
            let mut text = chunk.valid().to_owned();
            for b in chunk.invalid() {
                text.push_str(&format!("\\x{b:02X}"));
            }
            text
        })
        .collect()
}
fn not_implemented(mut body: Node<'_>) -> bool {
    if let Some(statements) = body.as_statements_node() {
        if statements.body().len() != 1 {
            return false;
        }
        body = statements.body().iter().next().unwrap();
    }
    let Some(call) = body.as_call_node() else {
        return false;
    };
    if call.receiver().is_some() || call.block().is_some_and(|b| b.as_block_node().is_some()) {
        return false;
    }
    call.name().as_slice() == b"fail"
        || (call.name().as_slice() == b"raise"
            && ast::call_arguments(&call)
                .first()
                .is_some_and(|arg| ast::is_global_const(arg, &["NotImplementedError"])))
}
#[derive(Default)]
struct Yields<'pr> {
    stack: Vec<(Node<'pr>, bool)>,
    defs: HashSet<usize>,
}
impl<'pr> NodeVisitor<'pr> for Yields<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let found = node.as_yield_node().is_some();
        self.stack.push((node, found));
    }
    fn leave(&mut self) {
        let (node, found) = self.stack.pop().unwrap();
        if found && let Some((parent, parent_found)) = self.stack.last_mut() {
            // Only the direct body edge exempts this def's &block. A yield
            // in its receiver/defaults is not in its body, but still counts
            // as a descendant of an enclosing method's body, as upstream does.
            if let Some(def) = parent.as_def_node()
                && def.body().is_some_and(|body| {
                    body.location().start_offset() == node.location().start_offset()
                        && body.location().end_offset() == node.location().end_offset()
                })
            {
                self.defs.insert(parent.location().start_offset());
            }
            *parent_found = true;
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's unused_method_argument_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnusedMethodArgument")
            .collect()
    }
    #[test]
    fn reports_name_and_exact_contextual_message() {
        let found = offenses("def some_method(foo, bar)\n  puts bar\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 17, 1));
        assert_eq!(
            found[0].message,
            "Unused method argument - `foo`. If it's necessary, use `_` or `_foo` as an argument name to indicate that it won't be used. If it's unnecessary, remove it."
        );
    }
    #[test]
    fn keyword_and_all_unused_messages() {
        let found = offenses("def self.method(foo:, **bar); nil; end");
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].message,
            "Unused method argument - `foo`. You can also write as `method(*)` if you want the method to accept any arguments but don't care about them."
        );
        assert!(found[1].message.contains("use `_` or `_bar`"));
        assert_eq!((found[0].line, found[0].col), (1, 17));
        assert_eq!((found[1].line, found[1].col), (1, 25));
    }
    #[test]
    fn ignores_empty_not_implemented_and_references() {
        for source in [
            "def f(foo); end",
            "def self.f(foo); end",
            "def f(foo); raise NotImplementedError; end",
            "def f(foo); raise ::NotImplementedError, 'TODO'; end",
            "def f(foo); fail; end",
            "def f(foo); fail 'TODO'; end",
            "def f(foo); foo; end",
            "def f(foo); super; end",
            "def f(foo); consume(binding); end",
            "def f(&block); yield; end",
            "def f(&block); xs.each { yield }; end",
            "def f(&block); def g; yield; end; end",
            "def f(_foo); nil; end",
            "xs.each { |foo| nil }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn keeps_real_findings_at_defaults_and_scope_boundaries() {
        for source in [
            "def f(foo); nil; end",
            "def f(foo); super(); end",
            "def f(foo); binding(1); end",
            "def f(foo); def g; binding; end; end",
            "def f(foo); raise OtherError; end",
            "def f(foo); obj.fail; end",
            "def f(foo); raise NotImplementedError; nil; end",
            "def f(foo); begin; raise NotImplementedError; end; end",
            "def f(foo:); nil; end",
            "private def f(foo); nil; end",
            "def self.f(foo); nil; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
}
