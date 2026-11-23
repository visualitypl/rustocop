//! `Naming/BlockForwarding` — fixed anonymous block forwarding (Ruby >=3.1).
use crate::{
    cops::{
        Cop, NodeContext,
        variables::{ScopeAnalysis, scope_owns_child},
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::{HashMap, HashSet};
pub struct BlockForwarding;
type Span = (usize, usize);
fn span(node: &Node<'_>) -> Span {
    (node.location().start_offset(), node.location().end_offset())
}
impl Cop for BlockForwarding {
    fn name(&self) -> &'static str {
        "Naming/BlockForwarding"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &[
            "Lint/AmbiguousOperator",
            "Style/ArgumentsForwarding",
            "Style/EndlessMethod",
            "Style/ExplicitBlockArgument",
            "Style/MethodDefParentheses",
        ]
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
        // Detection follows the pin's syntactic descendant/source tests in
        // one complete index walk. Structural intervals, not byte offsets,
        // own defaults/heredocs. Binding analysis is correction-only, once.
        let mut index = Index::default();
        walk(&mut index, node);
        let mut bindings = None;
        let mut safety = None;
        let mut reported = HashSet::new();
        for def in &index.defs {
            let method = def.location().start_offset();
            let Some(parameters) = def.parameters() else {
                continue;
            };
            let Some(parameter) = parameters.block() else {
                continue;
            };
            if parameter.name().is_none() {
                continue;
            }
            // Pinned matcher retains its historical keyword-parameter exception
            // even at the fixed 3.4 target; **kwargs alone is not a keyword arg.
            if index.keyword_methods.contains(&method) {
                continue;
            }
            let source = parameter.location().as_slice();
            let ranges = &index.methods[&method];
            // Deliberately includes shadowed locals and nested closed scopes.
            // Upstream compares the raw signature minus '&', not its name.
            if index.uses_local(source.get(1..).unwrap_or_default(), ranges.body) {
                continue;
            }
            let forwards = index.matching_forwards(source, ranges.subtree);
            let first = out.len();
            for (start, end) in
                std::iter::once(span(&parameter.as_node())).chain(forwards.iter().map(|f| f.span))
            {
                // Multiple enclosing methods can match one descendant pass;
                // upstream's offense set reports that source location once.
                if !reported.insert((start, end)) {
                    continue;
                }
                let (line, col) = file.line_col(start);
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Use anonymous block forwarding.".into(),
                    line,
                    col,
                    end_line: file.line_of(end.saturating_sub(1)),
                    correction: None,
                });
            }
            // The bundle stands for every offense of the method (upstream
            // corrects each separately); a partly disabled group stays as is.
            if cx.corrections_requested()
                && out.len() > first
                && out[first..]
                    .iter()
                    .all(|o| file.cop_enabled_for_lines(self.name(), o.line, o.end_line))
            {
                let (analysis, scopes) = bindings.get_or_insert_with(|| {
                    let analysis = ScopeAnalysis::of(&program);
                    let scopes: HashMap<_, _> = analysis
                        .scopes_in_exit_order()
                        .filter_map(|id| {
                            let scope = analysis.scope(id);
                            scope.node.as_def_node().map(|_| (span(&scope.node).0, id))
                        })
                        .collect();
                    (analysis, scopes)
                });
                let safety = safety.get_or_insert_with(|| file.forwarding_safety(node, analysis));
                if !safety.observes_bindings
                    && !index.shadowed.contains(&method)
                    && let Some(scope) = scopes.get(&method)
                    && let Some(proven) =
                        proven_forwards(analysis.scope(*scope), analysis, &index, file, source)
                {
                    // A syntactic match may belong to a different parameter or
                    // even be a method call. Never rewrite those identities.
                    out[first].correction = rewrite(def, &proven, file);
                }
            }
        }
    }
}

struct Forward {
    span: Span,
    parens: Option<Option<(usize, usize, usize)>>,
}
struct Frame<'pr> {
    node: Node<'pr>,
    active: Option<usize>,
    body: Option<usize>,
    enters_parameters: bool,
    ordinal: usize,
    body_range: Span,
    pattern: bool,
}
// Half-open traversal intervals. Per-name/source vectors are ordered by these
// ordinals, so each method query is logarithmic plus its matching findings.
struct MethodRanges {
    subtree: Span,
    body: Span,
}
#[derive(Default)]
struct Index<'pr> {
    frames: Vec<Frame<'pr>>,
    forwards: HashMap<Span, Forward>,
    forward_reads: HashMap<Span, Span>,
    forward_sources: HashMap<&'pr [u8], Vec<(usize, Span)>>,
    local_uses: HashMap<&'pr [u8], Vec<usize>>,
    methods: HashMap<usize, MethodRanges>,
    defs: Vec<ruby_prism::DefNode<'pr>>,
    ordinal: usize,
    shadowed: HashSet<usize>,
    parameter_owners: Vec<usize>,
    keyword_methods: HashSet<usize>,
}
impl Index<'_> {
    fn uses_local(&self, name: &[u8], (start, end): Span) -> bool {
        self.local_uses.get(name).is_some_and(|uses| {
            uses.get(uses.partition_point(|&n| n < start))
                .is_some_and(|&n| n < end)
        })
    }
    fn matching_forwards(&self, source: &[u8], (start, end): Span) -> Vec<&Forward> {
        let Some(uses) = self.forward_sources.get(source) else {
            return Vec::new();
        };
        let first = uses.partition_point(|&(n, _)| n < start);
        let last = uses.partition_point(|&(n, _)| n < end);
        let mut found: Vec<_> = uses[first..last]
            .iter()
            .map(|(_, range)| &self.forwards[range])
            .collect();
        found.sort_by_key(|f| f.span);
        found
    }
}

// Corrections cover every explicit read of exactly this method parameter.
// A raw-spelling mismatch remains diagnostic-only: removing just the signature
// would strand its read, while editing a nonmatching pass exceeds the finding.
fn proven_forwards<'a>(
    scope: &crate::cops::variables::Scope<'_>,
    analysis: &ScopeAnalysis<'_>,
    index: &'a Index<'_>,
    file: &SourceFile,
    source: &[u8],
) -> Option<Vec<&'a Forward>> {
    let name = scope.node.as_def_node()?.parameters()?.block()?.name()?;
    let variable = scope
        .variables()
        .map(|v| analysis.variable(v))
        .find(|v| v.name == name.as_slice())?;
    if !variable.assignments.is_empty() || variable.has_pattern_bindings {
        return None;
    }
    let mut forwards = Vec::new();
    for r in &variable.references {
        let r = analysis.reference(*r);
        if !r.is_explicit() {
            continue;
        }
        let range = index.forward_reads.get(&(r.range.start, r.range.end))?;
        let forward = &index.forwards[range];
        if file.bytes.get(range.0..range.1)? != source {
            return None;
        }
        forwards.push(forward);
    }
    forwards.sort_by_key(|f| f.span);
    Some(forwards)
}

impl<'pr> NodeVisitor<'pr> for Index<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let ordinal = self.ordinal;
        self.ordinal += 1;
        // Same syntactic lvasgn projection as the shared variable helpers:
        // pattern/capture targets are match_var, real writes remain lvasgn.
        let assignment_targets = matches!(
            node,
            Node::MultiWriteNode { .. } | Node::ForNode { .. } | Node::RescueNode { .. }
        );
        let pattern = !assignment_targets
            && (self.frames.last().is_some_and(|p| p.pattern)
                || matches!(
                    node,
                    Node::MatchWriteNode { .. }
                        | Node::MatchRequiredNode { .. }
                        | Node::MatchPredicateNode { .. }
                        | Node::InNode { .. }
                ));
        let name = match node {
            Node::LocalVariableReadNode { .. } => Some(
                node.as_local_variable_read_node()
                    .unwrap()
                    .name()
                    .as_slice(),
            ),
            Node::ItLocalVariableReadNode { .. } => Some(b"it".as_slice()),
            Node::LocalVariableWriteNode { .. } => Some(
                node.as_local_variable_write_node()
                    .unwrap()
                    .name()
                    .as_slice(),
            ),
            Node::LocalVariableTargetNode { .. } if !pattern => Some(
                node.as_local_variable_target_node()
                    .unwrap()
                    .name()
                    .as_slice(),
            ),
            Node::LocalVariableAndWriteNode { .. } => Some(
                node.as_local_variable_and_write_node()
                    .unwrap()
                    .name()
                    .as_slice(),
            ),
            Node::LocalVariableOrWriteNode { .. } => Some(
                node.as_local_variable_or_write_node()
                    .unwrap()
                    .name()
                    .as_slice(),
            ),
            Node::LocalVariableOperatorWriteNode { .. } => Some(
                node.as_local_variable_operator_write_node()
                    .unwrap()
                    .name()
                    .as_slice(),
            ),
            _ => None,
        };
        if let Some(name) = name
            && !self
                .frames
                .last()
                .is_some_and(|p| p.node.as_block_argument_node().is_some())
        {
            self.local_uses.entry(name).or_default().push(ordinal);
        }
        // Upstream's keyword exception covers the entire parameter subtree,
        // including lambdas/default expressions, but not a method's body.
        let parameter_owner = node.as_parameters_node().and_then(|_| {
            self.frames
                .last()?
                .node
                .as_def_node()
                .map(|d| d.location().start_offset())
        });
        if let Some(owner) = parameter_owner {
            self.parameter_owners.push(owner);
        }
        if matches!(
            node,
            Node::RequiredKeywordParameterNode { .. } | Node::OptionalKeywordParameterNode { .. }
        ) {
            self.keyword_methods.extend(&self.parameter_owners);
        }
        let active = self.frames.last().and_then(|p| {
            if scope_owns_child(&p.node, &node) {
                p.body
            } else {
                p.active
            }
        });
        let body = match node {
            Node::DefNode { .. } => Some(span(&node).0),
            Node::ClassNode { .. } | Node::ModuleNode { .. } | Node::SingletonClassNode { .. } => {
                None
            }
            _ => active,
        };
        let parameters = node
            .as_block_node()
            .and_then(|b| b.parameters())
            .or_else(|| node.as_lambda_node().and_then(|b| b.parameters()))
            .and_then(|p| p.as_block_parameters_node())
            .and_then(|p| p.parameters());
        if parameters.is_some_and(|p| p.block().is_some())
            && let Some(owner) = active
        {
            self.shadowed.insert(owner);
        }
        if let Some(block) = node.as_block_argument_node()
            && let Some(parent) = self.frames.last()
        {
            let range = span(&node);
            let parens = delimiters(&parent.node, range);
            self.forwards.insert(
                range,
                Forward {
                    span: range,
                    parens,
                },
            );
            self.forward_sources
                .entry(node.location().as_slice())
                .or_default()
                .push((ordinal, range));
            if let Some(value) = block
                .expression()
                .filter(|e| e.as_local_variable_read_node().is_some())
            {
                self.forward_reads.insert(span(&value), range);
            }
        }
        self.frames.push(Frame {
            node,
            active,
            body,
            enters_parameters: parameter_owner.is_some(),
            ordinal,
            body_range: (0, 0),
            pattern,
        });
    }
    fn leave(&mut self) {
        let frame = self.frames.pop().unwrap();
        if frame.enters_parameters {
            self.parameter_owners.pop();
        }
        let range = (frame.ordinal, self.ordinal);
        if let Some(def) = frame.node.as_def_node() {
            self.defs.push(def);
            self.methods.insert(
                span(&frame.node).0,
                MethodRanges {
                    subtree: range,
                    body: frame.body_range,
                },
            );
        }
        if let Some(parent) = self.frames.last_mut()
            && let Some(body) = parent.node.as_def_node().and_then(|d| d.body())
            && span(&body) == span(&frame.node)
        {
            parent.body_range = range;
        }
    }
}

// Outer None means an unsupported owner; inner None means existing delimiters.
fn delimiters(parent: &Node<'_>, block: Span) -> Option<Option<(usize, usize, usize)>> {
    let (selector, arguments) = if let Some(call) = parent.as_call_node() {
        if call.opening_loc().is_some() {
            return Some(None);
        }
        (call.message_loc()?.end_offset(), call.arguments())
    } else if let Some(call) = parent.as_super_node() {
        if call.lparen_loc().is_some() {
            return Some(None);
        }
        (call.keyword_loc().end_offset(), call.arguments())
    } else {
        return matches!(
            parent,
            Node::IndexTargetNode { .. }
                | Node::IndexAndWriteNode { .. }
                | Node::IndexOrWriteNode { .. }
                | Node::IndexOperatorWriteNode { .. }
        )
        .then_some(None);
    };
    let first = arguments
        .and_then(|a| a.arguments().iter().next())
        .map_or(block.0, |a| span(&a).0);
    Some(Some((selector, first, block.1)))
}

fn rewrite(
    def: &ruby_prism::DefNode<'_>,
    forwards: &[&Forward],
    file: &SourceFile,
) -> Option<Correction> {
    use crate::cops::style::{arguments_forwarding::add_parens, correction::combine};
    let parameters = def.parameters()?;
    let mut edits = Vec::new();
    for (start, end) in
        std::iter::once(span(&parameters.block()?.as_node())).chain(forwards.iter().map(|f| f.span))
    {
        // Never discard comments, continuation/newline trivia or encodings.
        let raw = file.bytes.get(start..end)?;
        if !raw.starts_with(b"&")
            || raw
                .iter()
                .any(|b| matches!(b, b'#' | b'\n' | b'\r' | b'\\'))
        {
            return None;
        }
        edits.push(Correction {
            range: start..end,
            replacement: "&".into(),
        });
    }
    if def.lparen_loc().is_none() {
        add_parens(
            def.name_loc().end_offset(),
            parameters.location().start_offset(),
            parameters.location().end_offset(),
            file,
            &mut edits,
        )?;
    }
    for forward in forwards {
        if let Some((selector, first, end)) = forward.parens? {
            add_parens(selector, first, end, file, &mut edits)?;
        }
    }
    // A single envelope per method makes the signature and every captured use
    // indivisible under directives, overlap selection and single-pass fixing.
    combine(file, edits)
}

#[cfg(test)]
mod tests {
    //! Ported RuboCop examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str_with};
    #[test]
    fn fixed_version_gate_is_real_and_not_bypassed_by_selection() {
        use crate::cops::policy::{DEFAULT_RUBY_VERSION, RubyVersion};
        let versions = BlockForwarding.ruby_versions();
        assert!(!versions.supports(RubyVersion(3, 0)));
        assert!(versions.supports(RubyVersion(3, 1)));
        assert!(versions.supports(DEFAULT_RUBY_VERSION));
    }

    #[test]
    fn canonical_explicit_and_singleton_forwarding() {
        for receiver in ["", "self."] {
            let before = format!(
                "def {receiver}foo(&callback)\n  {receiver}bar(&callback)\n  {receiver}baz(qux, &callback)\nend\n"
            );
            let found = lint_str_with(&[&BlockForwarding], &before);
            let shift = if receiver.is_empty() { 0 } else { 5 };
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                [(1, 9 + shift), (2, 7 + shift), (3, 12 + shift)]
            );
            assert!(
                found
                    .iter()
                    .all(|o| o.message == "Use anonymous block forwarding.")
            );
            assert_correction(&BlockForwarding, &before, &before.replace("&callback", "&"));
            let edits = crate::cops::testing::correction_offenses(&BlockForwarding, &before);
            assert_eq!(edits.iter().filter(|o| o.correction.is_some()).count(), 1);
        }
    }
    #[test]
    fn empty_yield_and_command_forms() {
        for (before, after) in [
            ("def foo(&block)\nend\n", "def foo(&)\nend\n"),
            (
                "def foo(&block)\n  yield\nend\n",
                "def foo(&)\n  yield\nend\n",
            ),
            (
                "def foo(& callback); bar(& callback); end\n",
                "def foo(&); bar(&); end\n",
            ),
            (
                "def foo arg, &block\n  bar &block\n  baz qux, &block\nend\n",
                "def foo(arg, &)\n  bar(&)\n  baz(qux, &)\nend\n",
            ),
        ] {
            assert_correction(&BlockForwarding, before, after);
        }
    }
    #[test]
    fn ruby34_captures_and_shadowed_bindings_keep_structural_owners() {
        for body in [
            "items.each { bar(&callback) }",
            "items.each { bar(&callback); baz(_1) }",
            "items.each { bar(&callback); baz(it) }",
            "-> { bar(&callback) }.call",
            "super",
            "super(&callback)",
            "super &callback",
            "target[1, &callback]",
        ] {
            let before = format!("def foo(&callback); {body}; end\n");
            let mut after = before.replace("&callback", "&");
            after = after.replace("super &", "super(&)");
            assert_correction(&BlockForwarding, &before, &after);
        }
        for (before, after) in [
            (
                "def foo(&callback); def inner(&callback); bar(&callback); end; end\n",
                "def foo(&); def inner(&); bar(&); end; end\n",
            ),
            (
                "def foo(&callback); consume(<<~TEXT)\n#{bar(&callback)}\nTEXT\nend\n",
                "def foo(&); consume(<<~TEXT)\n#{bar(&)}\nTEXT\nend\n",
            ),
            (
                "def foo(&callback); def (factory(&callback)).inner; end; end\n",
                "def foo(&); def (factory(&)).inner; end; end\n",
            ),
            (
                "def outer(&callback)\n def (make(<<~TXT)).inner; end\n#{bar(&callback)}\nTXT\nend\n",
                "def outer(&)\n def (make(<<~TXT)).inner; end\n#{bar(&)}\nTXT\nend\n",
            ),
            (
                "def outer(&callback)\n def inner(&callback); consume(<<~TXT); end\n#{bar(&callback)}\nTXT\nend\n",
                "def outer(&)\n def inner(&); consume(<<~TXT); end\n#{bar(&)}\nTXT\nend\n",
            ),
        ] {
            assert_correction(&BlockForwarding, before, after);
        }
    }
    #[test]
    fn uncertain_defaults_channels_trivia_and_observers_keep_diagnostics() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "def foo(arg = -> { bar(&callback) }, &callback); bar(&callback); end\n",
            "def foo(&callback); items.each { |callback| bar(&callback) }; end\n",
            "def foo(&callback); items.each { |&other| bar(&callback) }; end\n",
            "def foo(&callback); ->(&other) { bar(&callback) }; end\n",
            "def foo(&callback); bar(& # keep\n callback); end\n",
            "def foo(&callback); bar(&callback); p local_variables; end\n",
            "def foo(&callback); bar(&callback); eval('callback'); end\n",
            "def foo(&callback); bar(&callback); end\np method(:foo).parameters\n",
            "def foo(&callback); bar(&callback); end\np DATA.pos\n__END__\nraw\n",
            "def foo(&callback); things.map(&:parameters); end\n",
            "def foo(&block); :value => block; bar(&block); end",
            "def foo(&callback); baz(& callback); end",
            "def foo(&callback); baz(&\n callback); end",
            "def foo(& callback); callback; end",
            "def foo(other, &callback); bar(&other).each(&callback); end\n",
        ] {
            let found = correction_offenses(&BlockForwarding, source);
            assert!(!found.is_empty(), "{source}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{source}: {found:?}"
            );
        }
    }
    #[test]
    fn pinned_syntactic_scope_and_exact_source_differential_regressions() {
        // Independent source-only pin differential: body names are syntactic,
        // even across shadowing/closed scopes; passes compare original source.
        for (source, positions) in [
            (
                "def foo(&callback); xs.each { |callback| callback }; bar(&callback); end",
                vec![],
            ),
            (
                "def foo(&callback); xs.each { |;callback| callback=1; callback }; bar(&callback); end",
                vec![],
            ),
            (
                "def foo(&callback); def inner(callback); callback; end; bar(&callback); end",
                vec![],
            ),
            (
                "def foo(&callback); def inner; callback=1; end; bar(&callback); end",
                vec![],
            ),
            (
                "def outer(&callback); def inner(&other); foo(&callback); end; bar(&callback); end",
                vec![(1, 11), (1, 33), (1, 46), (1, 67)],
            ),
            (
                "def foo(&callback); xs.each { |callback| bar(&callback) }; end",
                vec![(1, 9), (1, 46)],
            ),
            (
                "def foo(&callback); def inner(&callback); bar(&callback); end; end",
                vec![(1, 9), (1, 31), (1, 47)],
            ),
            ("def foo(&callback); baz(& callback); end", vec![(1, 9)]),
            ("def foo(&callback); baz(&\n callback); end", vec![(1, 9)]),
            ("def foo(&callback); baz(&(callback)); end", vec![]),
            (
                "def foo(&callback); alias value callback; bar(&callback); end",
                vec![(1, 9), (1, 47)],
            ),
        ] {
            let mut found = lint_str_with(&[&BlockForwarding], source);
            found.sort_by_key(|o| (o.line, o.col));
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{source}"
            );
            assert!(
                found
                    .iter()
                    .all(|o| o.message == "Use anonymous block forwarding.")
            );
        }
    }

    #[test]
    fn pinned_syntactic_matrix_includes_defaults_patterns_and_heredoc_ownership() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns05_syntax.json")).unwrap();
        assert_eq!(rows.len(), 47);
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let mut found = lint_str_with(&[&BlockForwarding], source);
            found.sort_by_key(|o| (o.line, o.col));
            let positions: Vec<(u32, u32)> = crate::fixture::value(&row["positions"]);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{row}"
            );
            assert!(
                found
                    .iter()
                    .all(|o| o.message == "Use anonymous block forwarding.")
            );
        }
    }

    #[test]
    fn syntactic_index_visits_once_and_queries_only_structural_subtrees() {
        struct Count(usize);
        impl<'pr> NodeVisitor<'pr> for Count {
            fn enter(&mut self, _: Node<'pr>) {
                self.0 += 1;
            }
        }
        let source =
            "def outer(&callback); def inner(callback); callback; end; bar(&callback); end\n"
                .repeat(1000);
        let parsed = crate::parser::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let mut index = Index::default();
        walk(&mut index, &parsed.node());
        let mut count = Count(0);
        walk(&mut count, &parsed.node());
        assert_eq!(index.ordinal, count.0);
        assert!(count.0 < 1000 * 30);
        assert_eq!(index.methods.len(), 2000);
        assert_eq!(index.local_uses[b"callback".as_slice()].len(), 1000);
        for def in &index.defs {
            let ranges = &index.methods[&def.location().start_offset()];
            assert!(index.uses_local(b"callback", ranges.body));
            assert_eq!(
                index.matching_forwards(b"&callback", ranges.subtree).len(),
                usize::from(def.name().as_slice() == b"outer")
            );
        }
    }

    #[test]
    fn variables_keywords_and_anonymous_lookalikes() {
        for source in [
            "def foo(&); bar(&); end",
            "def foo; end",
            "def foo(arg); end",
            "def foo(&block); block; end",
            "def foo(&block); bar(&block) if block; end",
            "def foo(&block); block.call; end",
            "def foo(k:, &block); bar(&block); end",
            "def foo(k: v, &block); bar(&block); end",
            "def foo(value = ->(key:) {}, &callback); bar(&callback); end",
            "def foo(value = ->(key: 1) {}, &callback); bar(&callback); end",
            "def foo(&block); block ||= -> {}; bar(&block); end",
            "def foo(&block); block = proc {} unless block_given?; bar(&block); end",
        ] {
            assert!(
                lint_str_with(&[&BlockForwarding], source).is_empty(),
                "{source}"
            );
        }
    }
}
