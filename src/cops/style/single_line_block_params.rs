//! `Style/SingleLineBlockParams` — fixed reduce/inject names, opt-in.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{BlockParametersNode, Node};
pub struct SingleLineBlockParams;
const TARGETS: [&str; 2] = ["acc", "elem"];
impl Cop for SingleLineBlockParams {
    fn name(&self) -> &'static str {
        "Style/SingleLineBlockParams"
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
        let Some(block) = node.as_block_node() else {
            return;
        };
        let Some(call) = cx.parent().and_then(Node::as_call_node) else {
            return;
        };
        if call.receiver().is_none()
            || !matches!(call.name().as_slice(), b"reduce" | b"inject")
            || file.line_of(block.opening_loc().start_offset())
                != file.line_of(block.closing_loc().start_offset())
        {
            return;
        }
        let Some((parameters, actual, preferred)) = parameter_names(block.parameters()) else {
            return;
        };
        if actual
            .iter()
            .zip(TARGETS.iter())
            .all(|(name, target)| name.trim_start_matches('_') == *target)
            && actual.len() <= 2
        {
            return;
        }
        let (line, col) = file.line_col(parameters.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Name `{}` block params `|{}|`.",
                String::from_utf8_lossy(call.name().as_slice()),
                preferred.join(", ")
            ),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| {
                    let root = cx.ancestors().next_back()?;
                    let proof = file.class_method_bindings(root);
                    if proof.observes_sources || proof.observes_data {
                        return None;
                    }
                    file.block_param_facts(root)
                        .edits
                        .get(&parameters.location().start_offset())
                        .cloned()
                        .flatten()
                })
                .flatten(),
        });
    }
}

fn parameter_names(
    parameters: Option<Node<'_>>,
) -> Option<(BlockParametersNode<'_>, Vec<String>, Vec<String>)> {
    let parameters = parameters?.as_block_parameters_node()?;
    if !parameters.locals().is_empty() {
        return None;
    }
    let args = parameters.parameters()?;
    if !args.optionals().is_empty()
        || args
            .rest()
            .is_some_and(|n| n.as_implicit_rest_node().is_none())
        || !args.posts().is_empty()
        || !args.keywords().is_empty()
        || args.keyword_rest().is_some()
        || args.block().is_some()
        || args.requireds().is_empty()
    {
        return None;
    }
    let actual: Option<Vec<_>> = args
        .requireds()
        .iter()
        .map(|arg| {
            arg.as_required_parameter_node()
                .map(|p| String::from_utf8_lossy(p.name().as_slice()).into_owned())
        })
        .collect();
    let actual = actual?;
    let preferred = actual
        .iter()
        .enumerate()
        .map(|(i, name)| {
            format!(
                "{}{}",
                if name.starts_with('_') { "_" } else { "" },
                TARGETS.get(i).copied().unwrap_or("")
            )
        })
        .collect();
    Some((parameters, actual, preferred))
}

/// A correction-only, owned index. All binding identities and all-use ranges
/// come from VariableForce; source facts are collected in one complete walk.
#[derive(Default)]
pub(crate) struct Facts {
    edits: std::collections::HashMap<usize, Option<Correction>>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        use crate::cops::{
            variables::{
                AssignmentKind, ReferenceKind, ScopeAnalysis, ScopeId, VariableId, scope_owns_child,
            },
            walk::{NodeVisitor, walk},
        };
        use std::collections::{HashMap, HashSet};
        let Some(program) = root.as_program_node() else {
            return Self::default();
        };
        let analysis = ScopeAnalysis::of(&program);
        let mut local: HashMap<ScopeId, HashMap<Vec<u8>, HashSet<VariableId>>> = HashMap::new();
        let mut subtree = HashMap::new();
        let mut by_location = HashMap::new();
        for scope in analysis.scopes_in_exit_order() {
            let data = analysis.scope(scope);
            by_location.insert(
                (
                    data.node.location().start_offset(),
                    data.node.location().end_offset(),
                    std::mem::discriminant(&data.node),
                ),
                scope,
            );
            let names = local.entry(scope).or_default();
            for id in data.variables() {
                let v = analysis.variable(id);
                names.entry(v.name.to_vec()).or_default().insert(id);
            }
            let below: &mut HashMap<Vec<u8>, HashSet<VariableId>> =
                subtree.entry(scope).or_default();
            for (name, ids) in names.iter() {
                below.entry(name.clone()).or_default().extend(ids);
            }
            if data.kind.is_block()
                && let Some(parent) = data.parent
            {
                let below = below.clone();
                let above: &mut HashMap<Vec<u8>, HashSet<VariableId>> =
                    subtree.entry(parent).or_default();
                for (name, ids) in below {
                    above.entry(name).or_default().extend(ids);
                }
            }
        }
        struct Frame<'pr> {
            node: Node<'pr>,
            // Scope evaluating this node, and scope owning its params/body.
            // Headers inherit `active`, including deferred heredoc descendants.
            active: ScopeId,
            body: ScopeId,
        }
        struct Index<'a, 'pr> {
            analysis: &'a ScopeAnalysis<'pr>,
            by_location: &'a HashMap<(usize, usize, std::mem::Discriminant<Node<'pr>>), ScopeId>,
            frames: Vec<Frame<'pr>>,
            bare: HashMap<ScopeId, HashSet<Vec<u8>>>,
            candidates: Vec<(ScopeId, BlockParametersNode<'pr>, Vec<String>, Vec<String>)>,
            hazard: bool,
            #[cfg(test)]
            visits: usize,
        }
        fn metadata(name: &[u8]) -> bool {
            matches!(
                name,
                b"parameters"
                    | b"local_variables"
                    | b"local_variable_get"
                    | b"local_variable_set"
                    | b"local_variable_defined?"
                    | b"define_method"
                    | b"define_singleton_method"
            )
        }
        impl<'pr> NodeVisitor<'pr> for Index<'_, 'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.visits += 1;
                }
                let own = self
                    .by_location
                    .get(&(
                        node.location().start_offset(),
                        node.location().end_offset(),
                        std::mem::discriminant(&node),
                    ))
                    .copied();
                // Decide at the direct child edge, once, then carry the active
                // scope through the complete subtree. Descendant source ranges
                // cannot decide ownership: deferred interpolation may lie past
                // the receiver/header AND its definition's closing delimiter.
                let active = self
                    .frames
                    .last()
                    .map(|parent| {
                        if scope_owns_child(&parent.node, &node) {
                            parent.body
                        } else {
                            parent.active
                        }
                    })
                    .or(own)
                    .expect("ProgramNode opens the root scope");
                let body = own.unwrap_or(active);
                if let Some(b) = node.as_block_node()
                    && let Some((p, a, t)) = parameter_names(b.parameters())
                {
                    self.candidates.push((body, p, a, t));
                }
                self.hazard |= node.as_implicit_node().is_some() // preserve shorthand keys, never silently rename them
                    || node.as_symbol_node().is_some_and(|s| metadata(s.unescaped()))
                    || node.as_call_node().is_some_and(|c| metadata(c.name().as_slice()))
                    || node.as_call_or_write_node().is_some_and(|c| metadata(c.read_name().as_slice()))
                    || node.as_call_and_write_node().is_some_and(|c| metadata(c.read_name().as_slice()))
                    || node.as_call_operator_write_node().is_some_and(|c| metadata(c.read_name().as_slice()))
                    || node.as_def_node().is_some_and(|d| matches!(d.name().as_slice(),b"reduce"|b"inject"));
                if let Some(call) = node.as_call_node()
                    && call.is_variable_call()
                    && call.receiver().is_none()
                {
                    let mut scope = active;
                    loop {
                        self.bare
                            .entry(scope)
                            .or_default()
                            .insert(call.name().as_slice().to_vec());
                        let data = self.analysis.scope(scope);
                        if !data.kind.is_block() {
                            break;
                        }
                        let Some(parent) = data.parent else {
                            break;
                        };
                        scope = parent;
                    }
                }
                self.frames.push(Frame { node, active, body });
            }
            fn leave(&mut self) {
                self.frames.pop();
            }
        }
        let mut index = Index {
            analysis: &analysis,
            by_location: &by_location,
            frames: Vec::new(),
            bare: HashMap::new(),
            candidates: Vec::new(),
            hazard: false,
            #[cfg(test)]
            visits: 0,
        };
        walk(&mut index, root);
        let mut facts = Self::default();
        #[cfg(test)]
        {
            facts.visits = index.visits;
        }
        for (scope, p, actual, preferred) in index.candidates {
            let edit = (|| {
                if index.hazard
                    || p.parameters()?.rest().is_some()
                    || file.bytes[p.location().start_offset()..p.closing_loc()?.start_offset()]
                        .iter()
                        .rfind(|b| !b.is_ascii_whitespace())
                        == Some(&b',')
                    || p.parameters()?.requireds().iter().any(|n| {
                        n.as_required_parameter_node()
                            .is_none_or(|p| std::str::from_utf8(p.name().as_slice()).is_err())
                    })
                    || actual.len() > 2
                    || actual.iter().collect::<HashSet<_>>().len() != actual.len()
                {
                    return None;
                }
                let vars: Vec<_> = actual
                    .iter()
                    .map(|name| {
                        local
                            .get(&scope)?
                            .get(name.as_bytes())?
                            .iter()
                            .next()
                            .copied()
                    })
                    .collect::<Option<_>>()?;
                for target in &preferred {
                    if subtree
                        .get(&scope)
                        .and_then(|m| m.get(target.as_bytes()))
                        .is_some_and(|ids| ids.iter().any(|id| !vars.contains(id)))
                        || index
                            .bare
                            .get(&scope)
                            .is_some_and(|names| names.contains(target.as_bytes()))
                    {
                        return None;
                    }
                    let mut current = scope;
                    while analysis.scope(current).kind.is_block() {
                        current = analysis.scope(current).parent?;
                        if local
                            .get(&current)
                            .and_then(|m| m.get(target.as_bytes()))
                            .is_some()
                        {
                            return None;
                        }
                    }
                }
                let mut edits = vec![Correction {
                    range: p.location().start_offset()..p.location().end_offset(),
                    replacement: format!("|{}|", preferred.join(", ")),
                }];
                for (id, target) in vars.into_iter().zip(preferred) {
                    let variable = analysis.variable(id);
                    if variable.has_pattern_bindings {
                        return None;
                    }
                    for &assignment in &variable.assignments {
                        let assignment = analysis.assignment(assignment);
                        if assignment.kind == AssignmentKind::NamedCapture {
                            return None;
                        }
                        edits.push(Correction {
                            range: assignment.name_range.clone(),
                            replacement: target.clone(),
                        });
                    }
                    for &reference in &variable.references {
                        let reference = analysis.reference(reference);
                        match reference.kind {
                            ReferenceKind::OperatorAssignment => {} // name already covered by the write
                            ReferenceKind::Read => edits.push(Correction {
                                range: reference.range.clone(),
                                replacement: target.clone(),
                            }),
                            _ => return None,
                        }
                    }
                }
                edits.sort_by_key(|e| (e.range.start, e.range.end));
                edits.dedup_by(|a, b| a.range == b.range && a.replacement == b.replacement);
                // Refuse any implicit/non-name range or overlap; atomic all-use edit.
                if edits.iter().skip(1).any(|e| {
                    !actual
                        .iter()
                        .any(|n| file.bytes[e.range.clone()] == *n.as_bytes())
                }) {
                    return None;
                }
                super::correction::combine(file, edits)
            })();
            facts.edits.insert(p.location().start_offset(), edit);
        }
        facts
    }
}
#[cfg(test)]
mod tests {
    //! Fixed-default adaptations of RuboCop style/single_line_block_params_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str_with};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str_with(&[&SingleLineBlockParams], source)
    }
    #[test]
    fn pinned_fixed_policy_corrections_and_positions() {
        for method in ["reduce", "inject"] {
            for suffix in [" {", "{", "(5) {", "(5){", " (5) {"] {
                let before = format!("[0, 1].{method}{suffix} |c, d| c + d }}");
                let after = format!("[0, 1].{method}{suffix} |acc, elem| acc + elem }}");
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (1, before.find('|').unwrap() as u32 + 1)
                );
                assert_eq!(
                    found[0].message,
                    format!("Name `{method}` block params `|acc, elem|`.")
                );
                assert_correction(&SingleLineBlockParams, &before, &after);
            }
        }
        for (before, after) in [
            ("test.reduce { |_x, _y| }", "test.reduce { |_acc, _elem| }"),
            (
                "test.reduce(x) { |acc, b| foo(acc, b) }",
                "test.reduce(x) { |acc, elem| foo(acc, elem) }",
            ),
            (
                "test.reduce(x) { |elem, acc| foo(elem, acc) }",
                "test.reduce(x) { |acc, elem| foo(acc, elem) }",
            ),
            (
                "test.reduce(x) { |b| foo(b) }",
                "test.reduce(x) { |acc| foo(acc) }",
            ),
        ] {
            assert_correction(&SingleLineBlockParams, before, after);
        }
    }
    #[test]
    fn trailing_comma_remains_an_offense_without_changing_destructuring_shape() {
        let source = "xs.reduce { |memo,| memo }";
        let found = crate::cops::testing::correction_offenses(&SingleLineBlockParams, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 13));
        assert!(found[0].correction.is_none());
        assert!(offenses("xs.reduce { |acc,| acc }").is_empty());
    }
    #[test]
    fn multiline_receiver_still_has_single_line_parameters() {
        let before = "xs\n .reduce { |memo,item| memo+item }";
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 12));
        assert_correction(
            &SingleLineBlockParams,
            before,
            "xs\n .reduce { |acc, elem| acc+elem }",
        );
    }
    #[test]
    fn all_uses_follow_variable_ids_not_spelling() {
        for (before, after) in [
            (
                "xs.reduce { |memo,item| memo += item; memo }",
                "xs.reduce { |acc, elem| acc += elem; acc }",
            ),
            (
                "xs.reduce { |memo,item| inner { memo = item }; memo }",
                "xs.reduce { |acc, elem| inner { acc = elem }; acc }",
            ),
            (
                "xs.reduce { |memo,item| inner { |memo| memo }; memo + item }",
                "xs.reduce { |acc, elem| inner { |memo| memo }; acc + elem }",
            ),
            (
                "xs.reduce { |memo,item| ->(item) { memo + item }; item }",
                "xs.reduce { |acc, elem| ->(item) { acc + item }; elem }",
            ),
            (
                "xs.reduce { |memo,item| memo, item = item, memo; memo }",
                "xs.reduce { |acc, elem| acc, elem = elem, acc; acc }",
            ),
            (
                "xs.reduce { |memo,item| begin; memo; rescue => item; memo = item; end }",
                "xs.reduce { |acc, elem| begin; acc; rescue => elem; acc = elem; end }",
            ),
            (
                "xs.reduce { |memo,item| for memo in item; memo; end }",
                "xs.reduce { |acc, elem| for acc in elem; acc; end }",
            ),
            (
                "-> { xs.reduce { |memo,item| memo + item } }",
                "-> { xs.reduce { |acc, elem| acc + elem } }",
            ),
        ] {
            assert_correction(&SingleLineBlockParams, before, after);
        }
    }
    #[test]
    fn collisions_and_reflection_keep_offenses_without_partial_renames() {
        for source in [
            "acc = 7; xs.reduce { |memo,item| memo + item + acc }",
            "acc = 7; xs.reduce { |memo,item| memo + item }",
            "xs.reduce { |memo,item| memo + item + acc }",
            "xs.reduce { |memo,item| inner { |acc| memo + acc }; item }",
            "xs.reduce { |memo,item| inner { acc }; memo + item }",
            "xs.reduce { |memo,item| acc = memo; item }",
            "xs.reduce { |memo,item| binding.local_variable_get(:memo) }",
            "xs.reduce { |memo,item| local_variables }",
            "xs.reduce { |memo,item| defined?(memo); {memo:} }",
            "xs.reduce { |memo,item| item => [memo]; memo }",
            "xs.reduce { |memo,item| /(?<memo>.)/ =~ item; memo }",
            "xs.reduce { |_,_| 1 }",
            "xs.reduce { |a,b,c| a+b+c }",
            "xs.reduce { |memo,item| memo+item }; p callback.parameters",
            "xs.reduce { |memo,item| memo+item }; callback.parameters ||= value",
            "xs.reduce { |memo,item| memo+item }; callback.parameters &&= value",
            "xs.reduce { |memo,item| memo+item }; callback.parameters += value",
            "xs.reduce { |memo,item| def acc.f; end; memo+item }",
            "xs.reduce { |memo,item| class << acc; end; memo+item }",
            "xs.reduce { |memo,item| memo+item }; define_method(:f, &callback)",
        ] {
            let found = crate::cops::testing::correction_offenses(&SingleLineBlockParams, source);
            assert_eq!(found.len(), 1, "{source}: {found:?}");
            assert!(found[0].correction.is_none(), "{source}: {found:?}");
        }
    }
    #[test]
    fn deferred_def_receiver_interpolation_is_an_outer_bare_call_collision() {
        let source = "$seen=[]\ndef acc;10;end\ndef build(s);$seen<<s;Object.new;end\nvalue=[1,2].reduce { |memo,item| def (build(<<~TXT)).tag;end; memo+item }\n#{acc}\nTXT\np [value,$seen]\n";
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        // The interpolation is structurally below the receiver, but physically
        // beyond BOTH the receiver and def. Offset containment cannot own it.
        struct Header<'pr> {
            ancestors: Vec<Node<'pr>>,
            receiver_end: usize,
            def_end: usize,
            bare_start: usize,
            outer_edge: bool,
        }
        impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Header<'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                if let Some(def) = node.as_def_node()
                    && def.name().as_slice() == b"tag"
                {
                    self.receiver_end = def.receiver().unwrap().location().end_offset();
                    self.def_end = def.location().end_offset();
                }
                if let Some(call) = node.as_call_node()
                    && call.name().as_slice() == b"acc"
                {
                    assert!(call.is_variable_call());
                    self.bare_start = call.location().start_offset();
                    let (i, def) = self
                        .ancestors
                        .iter()
                        .enumerate()
                        .find(|(_, n)| {
                            n.as_def_node()
                                .is_some_and(|d| d.name().as_slice() == b"tag")
                        })
                        .unwrap();
                    let child = &self.ancestors[i + 1];
                    self.outer_edge = !crate::cops::variables::scope_owns_child(def, child);
                    assert_eq!(child.location().end_offset(), self.receiver_end);
                }
                self.ancestors.push(node);
            }
            fn leave(&mut self) {
                self.ancestors.pop();
            }
        }
        let mut header = Header {
            ancestors: Vec::new(),
            receiver_end: 0,
            def_end: 0,
            bare_start: 0,
            outer_edge: false,
        };
        crate::cops::walk::walk(&mut header, &parsed.node());
        assert!(header.outer_edge);
        assert!(header.bare_start > header.def_end);
        assert!(header.bare_start > header.receiver_end);
        let found = crate::cops::testing::correction_offenses(&SingleLineBlockParams, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (4, 22));
        assert_eq!(
            found[0].message,
            "Name `reduce` block params `|acc, elem|`."
        );
        assert!(found[0].correction.is_none(), "{found:?}");
        assert_correction(&SingleLineBlockParams, source, source);
    }
    #[test]
    fn all_deferred_definition_headers_keep_outer_interpolation_and_captures() {
        for header in [
            "def (build(<<~TXT)).tag; end",
            "def (build(<<~TXT)).tag(acc); end",
            "def (build(<<~TXT)).tag = 0",
            "class << build(<<~TXT); end",
            "class C < build(<<~TXT); end",
            "class build(<<~TXT)::C; end",
            "module build(<<~TXT)::M; end",
        ] {
            for interpolation in [
                "acc",
                "elem",
                "[0].map { acc }.first",
                "-> { elem }.call",
                "\"#{acc}\"",
            ] {
                let source = format!(
                    "xs.reduce {{ |memo,item| {header}; memo+item }}\n#{{{interpolation}}}\nTXT\n"
                );
                let found =
                    crate::cops::testing::correction_offenses(&SingleLineBlockParams, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}: {found:?}");
                assert_correction(&SingleLineBlockParams, &source, &source);
            }
        }
    }
    #[test]
    fn def_parameters_and_all_definition_bodies_keep_their_own_bare_calls() {
        // These `acc`/`elem` calls belong to closed scopes, even when their
        // heredoc bytes are physically past an enclosing block's closing `}`.
        for inner in [
            "def tag(x=<<~TXT); x; end",
            "def tag = <<~TXT",
            "def tag; <<~TXT; end",
            "class C; $value=<<~TXT; end",
            "module M; $value=<<~TXT; end",
            "class << self; $value=<<~TXT; end",
        ] {
            for interpolation in ["acc", "[0].map { acc }.first", "-> { elem }.call"] {
                let source = format!(
                    "xs.reduce {{ |memo,item| {inner}; memo+item }}\n#{{{interpolation}}}\nTXT\n"
                );
                let after = source
                    .replace("|memo,item|", "|acc, elem|")
                    .replace("memo+item", "acc+elem");
                assert_correction(&SingleLineBlockParams, &source, &after);
            }
        }
        // Receiver/header scope bypass does not leak into params or body.
        assert_correction(
            &SingleLineBlockParams,
            "xs.reduce { |memo,item| def (obj).tag(x=acc); elem; end; memo+item }",
            "xs.reduce { |acc, elem| def (obj).tag(x=acc); elem; end; acc+elem }",
        );
    }
    #[test]
    fn captured_header_variables_and_independent_heredocs_rename_by_identity() {
        for header in [
            "def (build(<<~TXT)).tag(acc); end",
            "class << build(<<~TXT); end",
            "class C < build(<<~TXT); end",
            "class build(<<~TXT)::C; end",
            "module build(<<~TXT)::M; end",
        ] {
            let source = format!(
                "xs.reduce {{ |memo,item| {header}; memo+item }}\n#{{memo}} #{{[0].map {{ item }}.first}} #{{->(memo) {{ memo }}.call(item)}}\nTXT\n"
            );
            let after = source
                .replace("|memo,item|", "|acc, elem|")
                .replace("memo+item", "acc+elem")
                .replace("#{memo}", "#{acc}")
                .replace("map { item }", "map { elem }")
                .replace("call(item)", "call(elem)");
            assert_correction(&SingleLineBlockParams, &source, &after);
        }
        let guarded =
            "xs.reduce { |memo,item| def (build(<<~TXT)).tag; end; memo+item }\n#{acc}\nTXT\n";
        let sibling = "xs.inject { |total,value| <<~SAFE }\n#{total+value}\nSAFE\n";
        let source = format!("{guarded}{sibling}");
        let after = format!(
            "{guarded}{}",
            sibling
                .replace("|total,value|", "|acc, elem|")
                .replace("total+value", "acc+elem")
        );
        let found = crate::cops::testing::correction_offenses(&SingleLineBlockParams, &source);
        assert_eq!(found.len(), 2);
        assert!(found[0].correction.is_none());
        assert!(found[1].correction.is_some());
        assert_correction(&SingleLineBlockParams, &source, &after);
    }
    #[test]
    fn simultaneous_and_nested_heredocs_follow_structural_scope_not_byte_order() {
        for (header, parameter, correctable) in [
            ("#{memo+item}", "#{acc}", true),
            ("#{acc}", "#{elem}", false),
            ("#{<<~INNER}\n#{memo+item}\nINNER", "#{acc}", true),
            ("#{<<~INNER}\n#{acc}\nINNER", "#{elem}", false),
        ] {
            let source = format!(
                "xs.reduce {{ |memo,item| def (build(<<~HEADER)).tag; end; def body(x=<<~PARAM); x; end; memo+item }}\n{header}\nHEADER\n{parameter}\nPARAM\n"
            );
            let after = if correctable {
                source
                    .replace("|memo,item|", "|acc, elem|")
                    .replace("memo+item", "acc+elem")
            } else {
                source.clone()
            };
            let found = crate::cops::testing::correction_offenses(&SingleLineBlockParams, &source);
            assert_eq!(found.len(), 1);
            assert_eq!(
                found[0].correction.is_some(),
                correctable,
                "{source}: {found:?}"
            );
            assert_correction(&SingleLineBlockParams, &source, &after);
        }
    }
    #[test]
    fn pinned_negatives_and_scope_shapes() {
        for source in [
            "test.reduce { |acc, elem| acc+elem }",
            "test.reduce { |acc, _elem| acc }",
            "test.reduce { |__acc| __acc }",
            "test.reduce { |acc| acc }",
            "test.reduce do |c,d|\n c+d\nend",
            "call_method(:reduce) { |a,b| a+b }",
            "test.reduce { |a,(id,_)| a+id }",
            "test.reduce { true }",
            "test.reduce { _1 }",
            "test.reduce { it }",
            "reduce { |x,y| x+y }",
            "->(x,y) { x+y }",
            "proc { |x,y| x+y }",
            "test.reduce { |x=1| x }",
            "test.reduce { |*x| x }",
            "test.reduce { |x:| x }",
            "test.reduce { |**x| x }",
            "test.reduce { |&x| x }",
            "test.reduce { |x; local| x }",
            "test.reduce {|| true }",
        ] {
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
