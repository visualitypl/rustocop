//! Correction-only, main-parse lexical identity proof shared by the Naming rename cops.
//!
//! Rename an entire VariableForce identity, never source-text lookalikes. A
//! preferred-name declaration/call anywhere in its open lexical component is
//! a conservative capture barrier, including later block parameters/defaults.
//! Rescue reassignments need value/branch splitting, so retain those findings
//! rather than guessing where an exception value stops flowing.
use crate::{
    cops::{
        variables::{
            AssignmentKind, Declaration, ParameterKind, ReferenceKind, ScopeAnalysis, ScopeId,
            ScopeKind, scope_owns_child,
        },
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::{collections::HashMap, ops::Range};

#[cfg(test)]
thread_local! { pub(super) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(Default)]
pub(crate) struct Facts {
    renames: HashMap<usize, Rename>,
    shorthand: HashMap<usize, usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
struct Rename {
    name: Vec<u8>,
    preferred: &'static str,
    ranges: Vec<Range<usize>>,
}
fn mask(name: &[u8]) -> u8 {
    match name {
        b"other" => 1,
        b"e" => 2,
        b"_e" => 4,
        _ => 0,
    }
}
fn component(analysis: &ScopeAnalysis<'_>, mut scope: ScopeId) -> usize {
    while analysis.scope(scope).kind.is_block() {
        let Some(parent) = analysis.scope(scope).parent else {
            break;
        };
        scope = parent;
    }
    let scope = analysis.scope(scope);
    if scope.kind == ScopeKind::TopLevel {
        0
    } else {
        scope.node.location().start_offset() + 1
    }
}
fn metadata(name: &[u8]) -> bool {
    matches!(
        name,
        b"binding"
            | b"Binding"
            | b"local_variables"
            | b"local_variable_get"
            | b"local_variable_set"
            | b"local_variable_defined?"
            | b"parameters"
            | b"eval"
            | b"method"
            | b"public_method"
            | b"singleton_method"
            | b"instance_method"
            | b"public_instance_method"
            | b"TracePoint"
    )
}
struct Frame<'pr> {
    node: Node<'pr>,
    active: usize,
    body: usize,
}
#[derive(Default)]
struct Index<'pr> {
    frames: Vec<Frame<'pr>>,
    occupied: HashMap<usize, u8>,
    rescue_ranges: HashMap<usize, Range<usize>>,
    shorthand: HashMap<usize, usize>,
    metadata: bool,
    visits: usize,
}
impl<'pr> NodeVisitor<'pr> for Index<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        self.visits += 1;
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
                | Node::ClassNode { .. }
                | Node::ModuleNode { .. }
                | Node::SingletonClassNode { .. }
        ) {
            node.location().start_offset() + 1
        } else {
            active
        };
        if let Some(call) = node.as_call_node() {
            self.metadata |= metadata(call.name().as_slice());
            if call.receiver().is_none() {
                *self.occupied.entry(active).or_default() |= mask(call.name().as_slice());
            }
        }
        macro_rules! getter {
            ($access:ident) => {
                if let Some(n) = node.$access() {
                    self.metadata |= metadata(n.read_name().as_slice());
                }
            };
        }
        getter!(as_call_or_write_node);
        getter!(as_call_and_write_node);
        getter!(as_call_operator_write_node);
        self.metadata |= node
            .as_symbol_node()
            .is_some_and(|n| metadata(n.unescaped()))
            || node
                .as_string_node()
                .is_some_and(|n| metadata(n.unescaped()))
            || node
                .as_constant_read_node()
                .is_some_and(|n| metadata(n.name().as_slice()))
            || node
                .as_constant_path_node()
                .is_some_and(|n| n.name().is_some_and(|n| metadata(n.as_slice())));
        if let Some(rescue) = node.as_rescue_node()
            && let Some(target) = rescue.reference()
        {
            let end = rescue
                .subsequent()
                .map_or(node.location().end_offset(), |n| {
                    n.location().start_offset()
                });
            self.rescue_ranges.insert(
                target.location().start_offset(),
                node.location().start_offset()..end,
            );
        }
        if let Some(pair) = node.as_assoc_node()
            && let Some(implicit) = pair.value().as_implicit_node()
        {
            self.shorthand.insert(
                implicit.value().location().start_offset(),
                pair.location().end_offset(),
            );
        }
        self.frames.push(Frame { node, active, body });
    }
    fn leave(&mut self) {
        self.frames.pop();
    }
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[cfg(test)]
        BUILDS.with(|n| n.set(n.get() + 1));
        let mut result = Self::default();
        let Some(program) = root.as_program_node() else {
            return result;
        };
        let analysis = ScopeAnalysis::of(&program);
        let mut index = Index::default();
        walk(&mut index, root);
        #[cfg(test)]
        {
            result.visits = index.visits;
        }
        let mut rescue_counts = HashMap::<(usize, u8), usize>::new();
        for scope in analysis.scopes_in_exit_order() {
            let owner = component(&analysis, scope);
            for id in analysis.scope(scope).variables() {
                let var = analysis.variable(id);
                *index.occupied.entry(owner).or_default() |= mask(var.name);
                if var
                    .assignments
                    .iter()
                    .any(|a| analysis.assignment(*a).kind == AssignmentKind::Rescue)
                {
                    let preferred = if var.name.starts_with(b"_") {
                        b"_e".as_slice()
                    } else {
                        b"e"
                    };
                    *rescue_counts.entry((owner, mask(preferred))).or_default() += 1;
                }
            }
        }
        if index.metadata {
            return result;
        }
        for scope in analysis.scopes_in_exit_order() {
            let owner = component(&analysis, scope);
            for id in analysis.scope(scope).variables() {
                let var = analysis.variable(id);
                let rescue = !var.assignments.is_empty()
                    && !var.is_parameter()
                    && var
                        .assignments
                        .iter()
                        .all(|a| analysis.assignment(*a).kind == AssignmentKind::Rescue);
                let parameter = analysis.scope(scope).kind == ScopeKind::Def
                    && var.declaration == Declaration::Parameter(ParameterKind::Required);
                if !(rescue || parameter)
                    || var.has_pattern_bindings
                    || analysis.may_be_dynamically_mutated(id)
                {
                    continue;
                }
                let preferred = if parameter {
                    "other"
                } else if var.name.starts_with(b"_") {
                    "_e"
                } else {
                    "e"
                };
                if index.occupied.get(&owner).copied().unwrap_or(0) & mask(preferred.as_bytes())
                    != 0
                {
                    continue;
                }
                let mut ranges = Vec::new();
                let mut keys = Vec::new();
                if parameter {
                    ranges.push(var.declaration_range.clone());
                    keys.push(var.declaration_range.start);
                }
                for a in &var.assignments {
                    let a = analysis.assignment(*a);
                    ranges.push(a.name_range.clone());
                    if rescue {
                        keys.push(a.name_range.start);
                    }
                }
                // Distinct handler identities may only merge when their uses
                // remain inside their own clauses and none escapes in a closure.
                let merging = rescue
                    && rescue_counts
                        .get(&(owner, mask(preferred.as_bytes())))
                        .copied()
                        .unwrap_or(0)
                        > 1;
                if merging {
                    let mut spans: Vec<_> = keys
                        .iter()
                        .filter_map(|key| index.rescue_ranges.get(key).cloned())
                        .collect();
                    spans.sort_by_key(|r| r.start);
                    let mut union: Vec<Range<usize>> = Vec::new();
                    for span in spans {
                        if let Some(last) = union.last_mut()
                            && span.start <= last.end
                        {
                            last.end = last.end.max(span.end);
                        } else {
                            union.push(span);
                        }
                    }
                    if var.captured_by_block
                        || var.references.iter().any(|r| {
                            let at = analysis.reference(*r).range.start;
                            let i = union.partition_point(|r| r.start <= at);
                            i == 0 || !union[i - 1].contains(&at)
                        })
                    {
                        continue;
                    }
                }
                let mut valid = true;
                for r in &var.references {
                    let r = analysis.reference(*r);
                    match r.kind {
                        ReferenceKind::Read => ranges.push(r.range.clone()),
                        // The explicit operator target is already an assignment;
                        // its reference range covers the entire RHS expression.
                        ReferenceKind::OperatorAssignment | ReferenceKind::ZeroAritySuper => {}
                        ReferenceKind::Binding => valid = false,
                    }
                }
                if !valid {
                    continue;
                }
                ranges.sort_by_key(|r| (r.start, r.end));
                ranges.dedup();
                // Emit one atomic correction per identity, on its first target.
                // Other targets are covered by that edit; duplicating a whole
                // scope-sized replacement at each target would be quadratic.
                if let Some(&first) = keys.first() {
                    result.renames.insert(
                        first,
                        Rename {
                            name: var.name.to_vec(),
                            preferred,
                            ranges,
                        },
                    );
                }
            }
        }
        result.shorthand = index.shorthand;
        result
    }

    pub(super) fn correction(
        &self,
        file: &SourceFile,
        start: usize,
        preferred: &str,
    ) -> Option<Correction> {
        let rename = self.renames.get(&start)?;
        if rename.preferred != preferred {
            return None;
        }
        let mut edits = Vec::new();
        for range in &rename.ranges {
            let old = file.bytes.get(range.clone())?;
            if let Some(&end) = self.shorthand.get(&range.start) {
                // Prism's synthetic implicit-value location can include the
                // label colon. Preserve that key and insert only its value.
                if old.strip_suffix(b":").unwrap_or(old) != rename.name {
                    return None;
                }
                edits.push(Correction {
                    range: end..end,
                    replacement: format!(" {preferred}"),
                });
            } else {
                if old != rename.name {
                    return None;
                }
                edits.push(Correction {
                    range: range.clone(),
                    replacement: preferred.into(),
                });
            }
        }
        crate::cops::style::correction::combine(file, edits)
    }
}

pub(super) fn attach(
    out: &mut [Offense],
    cx: &crate::cops::NodeContext<'_, '_>,
    file: &SourceFile,
    start: usize,
    preferred: &str,
) {
    if cx.corrections_requested()
        && let Some(root) = cx.ancestors().next_back()
        && let Some(offense) = out.last_mut()
    {
        offense.correction = file
            .local_rename_facts(root)
            .correction(file, start, preferred);
    }
}

pub(crate) static LOCAL_RENAME_FACTS: crate::source::Fact<Facts> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn local_rename_facts(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        LOCAL_RENAME_FACTS.get(self, || Facts::build(root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cops::{
        naming::BinaryOperatorParameterName,
        testing::{assert_correction, correction_offenses},
    };
    #[test]
    fn correction_index_is_lazy_shared_and_bounded() {
        BUILDS.with(|n| n.set(0));
        let source = "class Example; def +(arg); f { arg }; end; end\n".repeat(400);
        let found = crate::pipeline::lint_str_with(&[&BinaryOperatorParameterName], &source);
        assert_eq!(found.len(), 400);
        BUILDS.with(|n| assert_eq!(n.get(), 0));
        let found = correction_offenses(&BinaryOperatorParameterName, &source);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 400);
        BUILDS.with(|n| assert_eq!(n.get(), 1));
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        let root = parsed.node();
        let facts = file.local_rename_facts(&root);
        assert!(facts.visits < 400 * 20, "{}", facts.visits);
        assert!(std::ptr::eq(facts, file.local_rename_facts(&root)));
    }
    #[test]
    fn closed_scopes_shadowed_blocks_and_default_captures_keep_their_owners() {
        let before = "e=1; other=2\ndef +(arg)\n def inner(arg); arg; end\n def self.inner(arg); arg; end\n f { |arg| arg }; f { |;arg| arg=1 }\n ->(v=arg) { arg = v; arg }\nend\n";
        let after = "e=1; other=2\ndef +(other)\n def inner(arg); arg; end\n def self.inner(arg); arg; end\n f { |arg| arg }; f { |;arg| arg=1 }\n ->(v=other) { other = v; other }\nend\n";
        assert_correction(&BinaryOperatorParameterName, before, after);
    }
    #[test]
    fn canonical_rewrite_preserves_variableforce_scope_edges_and_reference_kinds() {
        let before = "def +(arg); result=arg; f { |arg| result+=arg }; ->(value=arg) { arg=value; arg }; end";
        let after = "def +(other); result=other; f { |arg| result+=arg }; ->(value=other) { other=value; other }; end";
        let graph = |source: &str| {
            let parsed = crate::parser::parse(source.as_bytes());
            let a = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            a.scopes_in_exit_order()
                .map(|s| {
                    let scope = a.scope(s);
                    let vars: Vec<_> = scope
                        .variables()
                        .map(|v| {
                            let v = a.variable(v);
                            let name = if scope
                                .node
                                .as_def_node()
                                .is_some_and(|n| n.name().as_slice() == b"+")
                                && v.name == b"arg"
                            {
                                b"other".to_vec()
                            } else {
                                v.name.to_vec()
                            };
                            (
                                name,
                                v.declaration,
                                v.captured_by_block,
                                v.assignments
                                    .iter()
                                    .map(|id| a.assignment(*id).kind)
                                    .collect::<Vec<_>>(),
                                v.references
                                    .iter()
                                    .map(|id| a.reference(*id).kind)
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect();
                    (scope.kind, scope.parent, vars)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(graph(before), graph(after));
    }
    #[test]
    fn writes_for_targets_and_zero_arity_super_are_not_read_expression_ranges() {
        assert_correction(
            &BinaryOperatorParameterName,
            "def +(arg); arg ||= 1; arg &&= 2; arg += 3; arg, x = [4, 5]; for arg in [6]; end; super; arg; end",
            "def +(other); other ||= 1; other &&= 2; other += 3; other, x = [4, 5]; for other in [6]; end; super; other; end",
        );
    }
}
