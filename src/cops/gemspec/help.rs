//! The pinned GemspecHelp's structural predicates, not gem type inference.
//! It matches the FIRST explicit specification parameter name throughout the
//! source, plus local `_1`/`it` unconditionally. Duplicate grouping separately
//! uses the nearest matching explicit specification. These are intentionally
//! name-based upstream semantics, not lexical binding or runtime analysis.
use crate::{
    cops::{
        ast,
        walk::{NodeVisitor, walk},
    },
    source::SourceFile,
};
use ruby_prism::{BlockNode, CallNode, Node};

pub(super) struct Assignment<'pr> {
    pub node: Node<'pr>,
    pub attribute: Vec<u8>,
    pub index: Option<Node<'pr>>,
    pub specification: Option<usize>,
}

/// `(block (send (const (const {cbase nil?} :Gem) :Specification) :new) ...)`:
/// the literal block of an argument-less `Gem::Specification.new` send.
pub(super) fn specification_block<'pr>(call: &CallNode<'pr>) -> Option<BlockNode<'pr>> {
    if call.is_safe_navigation()
        || call.name().as_slice() != b"new"
        || !ast::call_arguments(call).is_empty()
    {
        return None;
    }
    let receiver = call.receiver()?;
    let path = receiver.as_constant_path_node()?;
    if path.name()?.as_slice() != b"Specification"
        || !ast::is_global_const(&path.parent()?, &["Gem"])
    {
        return None;
    }
    call.block()?.as_block_node()
}

/// `gem_specification?`: the block variable of a matching constructor block
/// with exactly one plain parameter, `(args (arg $_))`.
pub(super) fn specification(call: &CallNode<'_>) -> Option<Vec<u8>> {
    let block = specification_block(call)?;
    let params = block.parameters()?.as_block_parameters_node()?;
    if !params.locals().is_empty() {
        return None;
    }
    let params = params.parameters()?;
    if params.requireds().len() != 1
        || !params.optionals().is_empty()
        || !params.posts().is_empty()
        || params
            .rest()
            .is_some_and(|rest| rest.as_implicit_rest_node().is_none())
        || !params.keywords().is_empty()
        || params.keyword_rest().is_some()
        || params.block().is_some()
    {
        return None;
    }
    Some(
        params
            .requireds()
            .iter()
            .next()?
            .as_required_parameter_node()?
            .name()
            .as_slice()
            .to_vec(),
    )
}

fn local_name<'pr>(
    node: &Node<'pr>,
    projected: &std::collections::HashSet<(usize, usize)>,
) -> Option<&'pr [u8]> {
    if let Some(call) = node.as_call_node()
        && projected.contains(&(call.location().start_offset(), call.location().end_offset()))
    {
        return Some(call.name().as_slice());
    }
    if let Some(local) = node.as_local_variable_read_node() {
        return Some(local.name().as_slice());
    }
    node.as_it_local_variable_read_node().map(|_| &b"it"[..])
}
fn assignment_method(name: &[u8]) -> bool {
    name.ends_with(b"=") && !matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=")
}
struct Collector<'pr> {
    first: Option<Vec<u8>>,
    projected: std::collections::HashSet<(usize, usize)>,
    // Each complete-walk frame carries the nearest matching specification.
    scopes: Vec<Option<usize>>,
    specifications: Vec<Node<'pr>>,
    ancestors: Vec<(usize, usize)>,
    projected_subtrees: std::collections::HashSet<(usize, usize)>,
    candidates: Vec<(Vec<u8>, Assignment<'pr>)>,
}
impl<'pr> NodeVisitor<'pr> for Collector<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let location = node.location();
        let span = (location.start_offset(), location.end_offset());
        if self.projected.contains(&span)
            || node.as_local_variable_read_node().is_some()
            || node.as_it_local_variable_read_node().is_some()
            || node.as_regular_expression_node().is_some()
            || node.as_interpolated_regular_expression_node().is_some()
            || node.as_match_last_line_node().is_some()
            || node.as_interpolated_match_last_line_node().is_some()
            || node
                .as_begin_node()
                .is_some_and(|n| n.rescue_clause().is_some())
        {
            // Compound literal fast paths must not hide projected reads,
            // pinned lvar(:it) identity, regexp source composition before
            // escape interpretation, or rescue child-slot normalization.
            // Reuse this complete walk's ancestor spans.
            self.projected_subtrees.extend(&self.ancestors);
            self.projected_subtrees.insert(span);
        }
        self.ancestors.push(span);
        let mut scope = self.scopes.last().copied().flatten();
        if let Some(call) = node.as_call_node() {
            if let Some(name) = specification(&call) {
                if self.first.is_none() {
                    self.first = Some(name);
                }
                scope = Some(self.specifications.len());
                self.specifications.push(call.as_node());
            }
            self.scopes.push(scope);
            if call.is_safe_navigation() {
                return;
            }
            let Some(receiver) = call.receiver() else {
                return;
            };
            if let Some(name) = local_name(&receiver, &self.projected) {
                if assignment_method(call.name().as_slice()) {
                    self.candidates.push((
                        name.to_vec(),
                        Assignment {
                            attribute: call.name().as_slice().to_vec(),
                            node: call.as_node(),
                            index: None,
                            specification: scope,
                        },
                    ));
                }
                return;
            }
            if call.name().as_slice() != b"[]=" {
                return;
            }
            let Some(receiver) = receiver.as_call_node() else {
                return;
            };
            if receiver.is_safe_navigation()
                || !ast::call_arguments(&receiver).is_empty()
                || receiver.block().is_some()
            {
                return;
            }
            let Some(local) = receiver.receiver() else {
                return;
            };
            let Some(name) = local_name(&local, &self.projected) else {
                return;
            };
            let mut args = ast::call_arguments(&call);
            if args.len() != 2 {
                return;
            }
            let key = args.remove(0);
            if !ast::is_literal(&key) {
                return;
            }
            self.candidates.push((
                name.to_vec(),
                Assignment {
                    attribute: receiver.name().as_slice().to_vec(),
                    node: call.as_node(),
                    index: Some(key),
                    specification: scope,
                },
            ));
        } else {
            self.scopes.push(scope);
            let target = if let Some(target) = node.as_call_target_node() {
                (!target.is_safe_navigation())
                    .then(|| (target.receiver(), target.name().as_slice().to_vec()))
            } else if let Some(target) = node.as_index_target_node() {
                (!target.is_safe_navigation()).then(|| (target.receiver(), b"[]=".to_vec()))
            } else {
                None
            };
            if let Some((receiver, attribute)) = target
                && let Some(name) = local_name(&receiver, &self.projected)
            {
                self.candidates.push((
                    name.to_vec(),
                    Assignment {
                        node,
                        attribute,
                        index: None,
                        specification: scope,
                    },
                ));
            }
        }
    }
    fn leave(&mut self) {
        self.ancestors.pop();
        self.scopes.pop();
    }
}
pub(super) struct Assignments<'pr> {
    pub nodes: Vec<Assignment<'pr>>,
    pub key_locals: std::collections::HashSet<(usize, usize)>,
    // Assignment scope IDs index these nearest enclosing blocks. Only the
    // duplicate consumer needs their structural keys; collect each once here.
    pub specifications: Vec<Node<'pr>>,
}
/// One bounded complete traversal per investigation, never a per-node root scan.
pub(super) fn assignments<'pr>(root: &Node<'pr>, file: &SourceFile) -> Assignments<'pr> {
    let mut projected = std::collections::HashSet::new();
    if file.match_pattern_composes_identifiers()
        && let Some(program) = root.as_program_node()
    {
        // Reuse the existing opt-in VariableForce source-buffer projection;
        // closed scopes and declaration order remain the shared analyzer's job.
        let analysis =
            super::super::variables::ScopeAnalysis::for_useless_assignments(&program, file);
        for id in analysis.scopes_in_exit_order() {
            for id in analysis.scope(id).variables() {
                for id in &analysis.variable(id).references {
                    let reference = analysis.reference(*id);
                    if reference.is_explicit() {
                        projected.insert((reference.range.start, reference.range.end));
                    }
                }
            }
        }
    }
    let mut collector = Collector {
        first: None,
        projected,
        scopes: vec![],
        specifications: vec![],
        ancestors: vec![],
        projected_subtrees: Default::default(),
        candidates: vec![],
    };
    walk(&mut collector, root);
    let first = collector
        .first
        .map(|name| file.match_pattern_source(&name).into_owned());
    let nodes = collector
        .candidates
        .into_iter()
        .filter_map(|(name, mut assignment)| {
            let name = file.match_pattern_source(&name);
            if !matches!(name.as_ref(), b"_1" | b"it") && first.as_deref() != Some(name.as_ref()) {
                return None;
            }
            assignment.attribute = file
                .match_pattern_source(&assignment.attribute)
                .into_owned();
            Some(assignment)
        })
        .collect();
    Assignments {
        nodes,
        key_locals: collector.projected_subtrees,
        specifications: collector.specifications,
    }
}
