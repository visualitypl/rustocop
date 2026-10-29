//! `Style/TrailingUnderscoreVariable` — fixed named-underscore allowance.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::ops::Range;
pub struct TrailingUnderscoreVariable;
impl Cop for TrailingUnderscoreVariable {
    fn name(&self) -> &'static str {
        "Style/TrailingUnderscoreVariable"
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
        if node.as_multi_write_node().is_none() {
            return;
        }
        let mut findings = Vec::new();
        let locals_only = collect(node, &mut findings);
        for finding in findings {
            let preferred = deletion_range(&finding, file);
            let mut good = file.bytes[node.location().start_offset()..preferred.start].to_vec();
            good.extend_from_slice(finding.replacement.as_bytes());
            good.extend_from_slice(&file.bytes[preferred.end..node.location().end_offset()]);
            if finding.replacement == "[" {
                good.push(b']');
            }
            let (line, col) = file.line_col(finding.range.start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Do not use trailing `_`s in parallel assignment. Prefer `{}`.",
                    String::from_utf8_lossy(&good)
                ),
                line,
                col,
                end_line: file.line_of(finding.range.end.saturating_sub(1)),
                correction: cx
                    .corrections_requested()
                    .then(|| correction(node, &finding, locals_only, cx, file))
                    .flatten(),
            });
        }
    }
}

struct Finding {
    range: Range<usize>,
    replacement: String,
    removed: Vec<usize>,
    // Removing an entire group loses a potentially effectful to_ary conversion.
    discard: bool,
    first_target: usize,
}

fn targets<'pr>(node: &Node<'pr>) -> (Vec<Node<'pr>>, Option<usize>) {
    let (left, rest, right, close) = if let Some(n) = node.as_multi_write_node() {
        (n.lefts(), n.rest(), n.rights(), n.rparen_loc())
    } else {
        let n = node.as_multi_target_node().unwrap();
        (n.lefts(), n.rest(), n.rights(), n.rparen_loc())
    };
    (
        left.iter()
            .chain(rest.filter(|n| n.as_implicit_rest_node().is_none()))
            .chain(right.iter())
            .collect(),
        close.map(|l| l.start_offset()),
    )
}
fn underscore(node: &Node<'_>) -> bool {
    node.as_local_variable_target_node()
        .is_some_and(|n| n.name().as_slice() == b"_")
        || node
            .as_splat_node()
            .and_then(|n| n.expression())
            .is_some_and(|n| underscore(&n))
}
fn collect(node: &Node<'_>, out: &mut Vec<Finding>) -> bool {
    let (vars, close) = targets(node);
    let mut locals_only = true;
    for child in &vars {
        locals_only &= if child.as_multi_target_node().is_some() {
            collect(child, out)
        } else {
            local_target(child)
        };
    }
    let first = vars
        .iter()
        .rposition(|n| !underscore(n))
        .map_or(0, |i| i + 1);
    if first == vars.len() {
        return locals_only;
    }
    // The pinned cop locates the first candidate by reverse structural equality,
    // not source identity. Repeated `_` targets can therefore put a splat before
    // the matching target even when it occurs inside the removable suffix.
    // Only bare `_` / `*_` candidates participate, so comparisons are bounded.
    let last_equal = vars
        .iter()
        .rposition(|n| underscore(n) && crate::cops::ast::same_node(n, &vars[first]))
        .unwrap();
    if vars[..last_equal]
        .iter()
        .any(|n| n.as_splat_node().is_some())
    {
        return locals_only;
    }
    let (range, replacement) = if first == 0 {
        if let Some(write) = node.as_multi_write_node() {
            let value = write.value();
            (
                node.location().start_offset()..value.location().start_offset(),
                if value
                    .as_array_node()
                    .is_some_and(|a| a.opening_loc().is_none())
                {
                    "["
                } else {
                    ""
                },
            )
        } else {
            (
                node.location().start_offset()..node.location().end_offset(),
                "_",
            )
        }
    } else if let Some(close) = close {
        (vars[first].location().start_offset() - 1..close, "")
    } else {
        (
            vars[first].location().start_offset()
                ..node
                    .as_multi_write_node()
                    .unwrap()
                    .operator_loc()
                    .start_offset(),
            "",
        )
    };
    out.push(Finding {
        range,
        replacement: replacement.into(),
        discard: first == 0,
        first_target: vars[first].location().start_offset(),
        removed: vars[first..]
            .iter()
            .map(|n| {
                n.as_splat_node().and_then(|s| s.expression()).map_or_else(
                    || n.location().start_offset(),
                    |n| n.location().start_offset(),
                )
            })
            .collect(),
    });
    locals_only
}
fn local_target(node: &Node<'_>) -> bool {
    node.as_local_variable_target_node().is_some()
        || node
            .as_splat_node()
            .is_some_and(|s| s.expression().is_none_or(|n| local_target(&n)))
}

fn correction(
    node: &Node<'_>,
    finding: &Finding,
    locals_only: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if !locals_only || finding.replacement == "_" {
        return None;
    }
    if finding.discard
        && node
            .as_multi_write_node()?
            .value()
            .as_array_node()
            .is_none()
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.trailing_underscore_facts(root);
    if facts.metadata || !finding.removed.iter().all(|p| facts.unobserved.contains(p)) {
        return None;
    }
    let proof = file.class_method_bindings(root);
    if proof.observes_sources || proof.observes_data {
        return None;
    }
    let range = deletion_range(finding, file);
    let old = &file.bytes[range.clone()];
    // Never delete comments, physical lines, escaped newlines or deferred text.
    // The replacement is only ASCII target punctuation/whitespace.
    if old
        .iter()
        .any(|b| !matches!(b, b' ' | b'\t' | b'_' | b'*' | b',' | b'(' | b')' | b'='))
        || super::correction::contains_comment(file, range.clone())
    {
        return None;
    }
    let mut edits = vec![Correction {
        range,
        replacement: finding.replacement.clone(),
    }];
    if finding.replacement == "[" {
        if file.contains_heredoc(root, node) {
            return None;
        }
        edits.push(Correction {
            range: node.location().end_offset()..node.location().end_offset(),
            replacement: "]".into(),
        });
    }
    super::correction::combine(file, edits)
}

// Retain the pinned diagnostic range, but neither suggest nor apply deletion
// of the comma in `(a,_)`: it owns the remaining forced destructuring.
fn deletion_range(finding: &Finding, file: &SourceFile) -> Range<usize> {
    let mut range = finding.range.clone();
    if !finding.discard && range.start < finding.first_target && file.bytes[range.start] == b',' {
        range.start += 1;
    }
    range
}

#[cfg(test)]
thread_local! {
    static FACT_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Owned, correction-only binding facts: one VariableForce analysis and one
/// complete main-tree metadata walk, never a scan/reparse per assignment.
#[derive(Default)]
pub(crate) struct Facts {
    unobserved: std::collections::HashSet<usize>,
    metadata: bool,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        use crate::cops::{
            variables::{AssignmentKind, ScopeAnalysis, scope_owns_child},
            walk::{NodeVisitor, walk},
        };
        #[cfg(test)]
        FACT_BUILDS.with(|n| n.set(n.get() + 1));
        let mut facts = Self::default();
        let Some(program) = root.as_program_node() else {
            return facts;
        };
        let analysis = ScopeAnalysis::of(&program);
        for scope in analysis.scopes_in_exit_order() {
            for id in analysis.scope(scope).variables() {
                let var = analysis.variable(id);
                if var.name == b"_"
                    && !var.is_parameter()
                    && !var.is_referenced()
                    && !var.captured_by_block
                    && !var.has_pattern_bindings
                    && var
                        .assignments
                        .iter()
                        .all(|id| analysis.assignment(*id).kind == AssignmentKind::Multiple)
                {
                    facts.unobserved.extend(
                        var.assignments
                            .iter()
                            .map(|id| analysis.assignment(*id).name_range.start),
                    );
                }
            }
        }
        fn metadata(name: &[u8]) -> bool {
            matches!(
                name,
                b"local_variables"
                    | b"local_variable_get"
                    | b"local_variable_set"
                    | b"local_variable_defined?"
                    | b"Binding"
            )
        }
        struct Frame<'pr> {
            node: Node<'pr>,
            active: usize,
            body: usize,
        }
        struct Index<'a, 'pr> {
            facts: &'a mut Facts,
            frames: Vec<Frame<'pr>>,
            parents: Vec<usize>,
            owners: std::collections::HashMap<usize, usize>,
            blocked: std::collections::HashSet<usize>,
        }
        impl<'pr> NodeVisitor<'pr> for Index<'_, 'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                let active = self.frames.last().map_or(0, |p| {
                    if scope_owns_child(&p.node, &node) {
                        p.body
                    } else {
                        p.active
                    }
                });
                let body = if matches!(
                    node,
                    Node::DefNode { .. }
                        | Node::BlockNode { .. }
                        | Node::LambdaNode { .. }
                        | Node::ClassNode { .. }
                        | Node::ModuleNode { .. }
                        | Node::SingletonClassNode { .. }
                ) {
                    self.parents.push(active);
                    self.parents.len() - 1
                } else {
                    active
                };
                // Resolve the parser's lexical depth at structural scope edges,
                // including deferred receiver/header interpolations. Keep this
                // independent capture barrier even though VariableForce now
                // declares parallel targets before visiting their RHS.
                macro_rules! local {
                    ($access:ident, $read:expr) => {
                        if let Some(n) = node.$access()
                            && n.name().as_slice() == b"_"
                        {
                            let mut owner = active;
                            for _ in 0..n.depth() {
                                owner = self.parents[owner];
                            }
                            self.owners.insert(n.location().start_offset(), owner);
                            if $read || n.depth() > 0 {
                                self.blocked.insert(owner);
                            }
                        }
                    };
                }
                local!(as_local_variable_read_node, true);
                local!(as_local_variable_target_node, false);
                local!(as_local_variable_write_node, true);
                local!(as_local_variable_and_write_node, true);
                local!(as_local_variable_or_write_node, true);
                local!(as_local_variable_operator_write_node, true);
                self.facts.metadata |= node
                    .as_call_node()
                    .is_some_and(|n| metadata(n.name().as_slice()))
                    || node
                        .as_symbol_node()
                        .is_some_and(|n| metadata(n.unescaped()))
                    || node
                        .as_constant_read_node()
                        .is_some_and(|n| metadata(n.name().as_slice()))
                    || node
                        .as_constant_path_node()
                        .is_some_and(|n| n.name().is_some_and(|n| metadata(n.as_slice())))
                    || node
                        .as_call_or_write_node()
                        .is_some_and(|n| metadata(n.read_name().as_slice()))
                    || node
                        .as_call_and_write_node()
                        .is_some_and(|n| metadata(n.read_name().as_slice()))
                    || node
                        .as_call_operator_write_node()
                        .is_some_and(|n| metadata(n.read_name().as_slice()));
                self.frames.push(Frame { node, active, body });
            }
            fn leave(&mut self) {
                self.frames.pop();
            }
        }
        let mut index = Index {
            facts: &mut facts,
            frames: Vec::new(),
            parents: vec![0],
            owners: std::collections::HashMap::new(),
            blocked: std::collections::HashSet::new(),
        };
        walk(&mut index, root);
        index.facts.unobserved.retain(|p| {
            index
                .owners
                .get(p)
                .is_some_and(|owner| !index.blocked.contains(owner))
        });
        facts
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/trailing_underscore_variable_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/TrailingUnderscoreVariable")
            .collect()
    }
    #[test]
    fn corrections_are_lazy_cached_and_bounded() {
        super::FACT_BUILDS.with(|n| n.set(0));
        let source = "a, _ = rhs\n".repeat(400);
        assert_eq!(offenses(&source).len(), 400);
        super::FACT_BUILDS.with(|n| assert_eq!(n.get(), 0));
        let found =
            crate::cops::testing::correction_offenses(&super::TrailingUnderscoreVariable, &source);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 400);
        super::FACT_BUILDS.with(|n| assert_eq!(n.get(), 1));
        let file = crate::source::SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let first = file.trailing_underscore_facts(&root);
        assert!(first.visits < 400 * 8, "{}", first.visits);
        assert!(std::ptr::eq(first, file.trailing_underscore_facts(&root)));
    }
    #[test]
    fn nested_offense_messages_apply_one_change_to_the_assignment() {
        let found = offenses("a, (_, (b, _), *_) = foo()");
        assert_eq!(found.len(), 2);
        for (o, col, good) in [
            (&found[0], 11, "a, (_, (b,), *_) = foo()"),
            (&found[1], 15, "a, (_, (b, _),) = foo()"),
        ] {
            assert_eq!((o.line, o.col, o.end_line), (1, col, 1));
            assert_eq!(
                o.message,
                format!("Do not use trailing `_`s in parallel assignment. Prefer `{good}`.")
            );
        }
    }
    #[test]
    fn comments_newlines_and_continuations_are_not_deleted() {
        for before in [
            "a, _ # keep\n = rhs",
            "a, _ , # comment\n _ = rhs",
            "a, _ \\\n = rhs",
        ] {
            let found = crate::cops::testing::correction_offenses(
                &super::TrailingUnderscoreVariable,
                before,
            );
            assert!(!found.is_empty(), "{before}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{before}");
        }
        crate::cops::testing::assert_correction(
            &super::TrailingUnderscoreVariable,
            "a, # keep\n b, _ = rhs # tail\np __LINE__\n",
            "a, # keep\n b, = rhs # tail\np __LINE__\n",
        );
    }
    #[test]
    fn local_reads_and_writes_in_every_scope_stay_bound() {
        for source in [
            "a, _ = rhs; {_:}",
            "a, _ = rhs; _ ||= 1",
            "a, _ = rhs; _ += 1",
            "a, _ = rhs; 1.times { _ = 2 }",
            "1.times { _ = 2 }; a, _ = rhs; -> { _ }",
            "def f(_); a, _ = rhs; end",
            "f { |_; x| a, _ = rhs }",
            "_ = old; a, _ = [-> { _ }, 2]",
            "a, _ = rhs; _ => x",
            "a, _ = rhs; p :local_variables",
            "a, _ = rhs; ::Binding",
            "a, _ = rhs; obj.local_variables ||= []",
            "a, _ = rhs; obj.local_variables += []",
            "a, _ = [_, 2]",
            "a, _ = [defined?(_), 2]",
            "a, _ = [-> { _ ||= 1 }, 2]",
            "a, _ = [-> { _ += 1 }, 2]",
            "def f; a, _ = rhs; -> { _ }; end",
            "class C; a, _ = rhs; -> { _ }; end",
            "module M; a, _ = rhs; binding; end",
            "class << obj; a, _ = rhs; local_variables; end",
        ] {
            let found = crate::cops::testing::correction_offenses(
                &super::TrailingUnderscoreVariable,
                source,
            );
            assert!(!found.is_empty(), "{source}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{source}: {found:?}"
            );
        }
    }
    #[test]
    fn unrelated_closed_scopes_and_deferred_headers_are_structural() {
        crate::cops::testing::assert_correction(
            &super::TrailingUnderscoreVariable,
            "_ = 9; def f; a, _ = rhs; a; end; p _",
            "_ = 9; def f; a, = rhs; a; end; p _",
        );
        let source = "a, _ = [def (build(<<~DOC)).f; end, 2]\n#{_}\nDOC\n";
        let found =
            crate::cops::testing::correction_offenses(&super::TrailingUnderscoreVariable, source);
        assert!(!found.is_empty());
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
    #[test]
    fn pinned_exact_diagnostics() {
        for (source, col, good) in [
            ("a, b, _ = foo()", 7, "a, b, = foo()"),
            ("a, _, _ = foo()", 4, "a, = foo()"),
            ("a, *_ = foo()", 4, "a, = foo()"),
            ("a, _, = foo()", 4, "a, = foo()"),
            ("_, = foo()", 1, "foo()"),
            ("_, b, _ = foo()", 7, "_, b, = foo()"),
            ("a, *_, _, _ = foo()", 4, "a, = foo()"),
            ("a, (b, _) = foo()", 7, "a, (b,) = foo()"),
            ("(a,_)=rhs", 3, "(a,)=rhs"),
            ("a, (_, _) = foo", 4, "a, _ = foo"),
            ("_, _ = 1, 2, 3", 1, "[1, 2, 3]"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                format!("Do not use trailing `_`s in parallel assignment. Prefer `{good}`.")
            );
        }
    }
    #[test]
    fn canonical_partial_corrections_preserve_parallel_assignment() {
        for (before, after) in [
            ("a, b, _ = foo()", "a, b, = foo()"),
            ("a, _, _ = foo()", "a, = foo()"),
            ("a, *_ = foo()", "a, = foo()"),
            ("a, _, = foo()", "a, = foo()"),
            ("_, b, _ = foo()", "_, b, = foo()"),
            ("a, *_, _, _ = foo()", "a, = foo()"),
            ("a, (b, _) = foo()", "a, (b,) = foo()"),
            ("a, (_, (b, _), *_) = foo()", "a, (_, (b,),) = foo()"),
            ("(a, b, _) = foo()", "(a, b,) = foo()"),
            ("(a, _, _,) = foo()", "(a,) = foo()"),
            ("a,_=rhs", "a,=rhs"),
            ("(a,_)=rhs", "(a,)=rhs"),
        ] {
            crate::cops::testing::assert_correction(
                &super::TrailingUnderscoreVariable,
                before,
                after,
            );
        }
    }
    #[test]
    fn canonical_all_underscore_array_values() {
        for (before, after) in [
            ("_, _ = 1, 2, 3", "[1, 2, 3]"),
            ("_, _ = *foo", "[*foo]"),
            ("_, _ = [1, 2]", "[1, 2]"),
            ("(_, _, _,) = [1, 2]", "[1, 2]"),
            ("(_,) = [1, 2]", "[1, 2]"),
            ("f((_, _ = [1, 2]))", "f(([1, 2]))"),
        ] {
            crate::cops::testing::assert_correction(
                &super::TrailingUnderscoreVariable,
                before,
                after,
            );
        }
    }
    #[test]
    fn uncertain_conversions_and_observable_locals_keep_offenses_without_edits() {
        for before in [
            "_, _ = x, -> { _, _ = y, z }",
            "a, _ = [-> { _ }, 2]",
            "a, _ = [-> { _ = 2 }, 3]",
            "_, = foo()",
            "(_, _, _,) = foo()",
            "(_,) = foo()",
            "a, (_, _) = foo",
            "a, (b, (_, _)) = foo",
            "a, (_, (_, _), b) = foo",
            "a, (_, (_,),) = foo",
            "a, *b, (_, _) = foo",
            "a, _ = rhs; p _",
            "a, _ = rhs; p defined?(_)",
            "a, _ = rhs; -> { _ }",
            "a, _ = rhs; binding",
            "binding; a, _ = rhs",
            "a, _ = rhs; local_variables",
            "a, _ = rhs; eval(code)",
            "a, _ = rhs; p DATA.pos\n__END__\nraw\n",
            "_ = old; a, _ = rhs",
            "a, _ = rhs; _ = new_value",
            "obj.a, _ = rhs",
            "A, _ = rhs",
            "@a, _ = rhs",
        ] {
            let found = crate::cops::testing::correction_offenses(
                &super::TrailingUnderscoreVariable,
                before,
            );
            assert!(!found.is_empty(), "{before}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{before}: {found:?}"
            );
        }
    }
    #[test]
    fn qualified_observer_constants_keep_offenses_without_edits() {
        for observer in [
            "RubyVM",
            "::RubyVM",
            "Object::RubyVM",
            "::Object::RubyVM",
            "TracePoint",
            "::TracePoint",
            "Object::TracePoint",
            "Other::RubyVM",
        ] {
            let source = format!("{observer}; a, _ = [1, 2]");
            let found = crate::cops::testing::correction_offenses(
                &super::TrailingUnderscoreVariable,
                &source,
            );
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}: {found:?}");
        }
    }
    #[test]
    fn repeated_underscore_targets_before_and_after_splats_match_pin() {
        for source in [
            "_, *_, _ = [1, 2, 3]",
            "(_, *_, _) = [1, 2, 3]",
            "a, _, *_, _ = [1, 2, 3, 4]",
            "a, (_, *_, _) = [1, [2, 3, 4]]",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // A splat after the last equal bare target does not suppress the offense.
        assert_eq!(offenses("_, *_ = [1, 2]").len(), 1);
        assert_eq!(offenses("a, _, *_ = [1, 2, 3]").len(), 1);
    }
    #[test]
    fn pinned_no_offenses_and_syntax_lookalikes() {
        for source in [
            "a, b, c = foo()",
            "_, a, b = foo()",
            "*a, b, _ = foo()",
            "a, *b, _ = foo()",
            "_, *b, _ = *foo",
            "a, *b, _, _ = foo()",
            "a, *b, _c, _d = foo()",
            "a, *b, _c = foo()",
            "a, (_, b) = foo()",
            "a, (_, (b,), c, (d, e),) = foo()",
            "a, b, _c = foo()",
            "a, *_b = foo()",
            "a, *_b, _ = foo()",
            "a, *_b, _, _ = foo()",
            "_ = foo()",
            "f { |a, _| a }",
            "def f(a, _); end",
            "f(a, _)",
            "[a, _]; {a: _}",
            "for a, _ in xs; end",
            "x => [a, _]",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
