//! `Style/MapIntoArray` — each/push array construction (unsafe).
use crate::{
    cops::{
        Cop, NodeContext, ast,
        variables::{ReferenceKind, ScopeAnalysis, VariableId},
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::{collections::HashMap, ops::Range};
pub struct MapIntoArray;
impl Cop for MapIntoArray {
    fn name(&self) -> &'static str {
        "Style/MapIntoArray"
    }
    fn unsafe_fixes(&self) -> bool {
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
        let Some(each) = node.as_call_node() else {
            return;
        };
        if each.name().as_slice() != b"each"
            || each.is_safe_navigation()
            || !ast::call_arguments(&each).is_empty()
            || each.receiver().is_none_or(|n| n.as_self_node().is_some())
        {
            return;
        }
        let Some(block) = each.block().and_then(|n| n.as_block_node()) else {
            return;
        };
        let Some(body) = block.body().and_then(|n| n.as_statements_node()) else {
            return;
        };
        if body.body().len() != 1 {
            return;
        }
        let Some(push) = body.body().iter().next().and_then(|n| n.as_call_node()) else {
            return;
        };
        if !matches!(push.name().as_slice(), b"<<" | b"push" | b"append")
            || push.is_safe_navigation()
            || push.block().is_some_and(|n| n.as_block_node().is_some())
        {
            return;
        }
        let args = ast::call_arguments(&push);
        let [argument] = args.as_slice() else { return };
        if matches!(
            argument,
            Node::SplatNode { .. }
                | Node::ForwardingArgumentsNode { .. }
                | Node::BlockArgumentNode { .. }
        ) || sole_forwarded_keywords(argument)
        {
            return;
        }
        let Some(dest) = push
            .receiver()
            .and_then(|n| n.as_local_variable_read_node())
        else {
            return;
        };
        let Some(parent) = cx.parent().and_then(|n| n.as_statements_node()) else {
            return;
        };
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        let facts = file.map_into_array_facts(root);
        let Some(var) = facts
            .references
            .get(&dest.location().start_offset())
            .and_then(|id| facts.variables.get(id))
        else {
            return;
        };
        let tap = parent.body().len() == 1 && empty_tap(cx, &var.declaration);
        let mut assignment_range = None;
        if !tap {
            let start = each.location().start_offset();
            let end = each.location().end_offset();
            let before = var.assignments.partition_point(|a| a.range.end < start);
            let Some(assignment) = before.checked_sub(1).map(|i| &var.assignments[i]) else {
                return;
            };
            if !assignment.empty || assignment.parent != Some(key(&parent.as_node())) {
                return;
            }
            let from = assignment.range.start;
            let refs = var.reference_ranges.partition_point(|r| r.start < from)
                ..var.reference_ranges.partition_point(|r| r.start < end);
            if refs.len() != 1 || var.reference_ranges[refs.start].end > end {
                return;
            }
            let writes = var.assignment_starts.partition_point(|s| *s < from)
                ..var.assignment_starts.partition_point(|s| *s < end);
            if writes.len() != 1 {
                return;
            }
            assignment_range = Some(assignment.range.clone());
        }
        let correction = (cx.corrections_requested() && !return_value_used(node, cx))
            .then(|| {
                correction(
                    &each,
                    &push,
                    argument,
                    dest.name().as_slice(),
                    &parent,
                    assignment_range,
                    cx,
                    file,
                )
            })
            .flatten();
        let (line, col) = file.line_col(each.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `map` instead of `each` to map elements into an array.".into(),
            line,
            col,
            end_line: file.last_line_of(each.location()),
            correction,
        });
    }
}

/// Upstream's `return_value_used?`, over the parser parents of Prism nodes:
/// a statement list of several is a `begin`, of one is transparent.
fn return_value_used(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut child = (node.location().start_offset(), node.location().end_offset());
    let mut ancestors = cx.ancestors();
    while let Some(parent) = ancestors.next() {
        match parent {
            Node::StatementsNode { .. } => {
                let body: Vec<_> = parent.as_statements_node().unwrap().body().iter().collect();
                if body.len() > 1 && body.last().is_some_and(|last| key(last) != child) {
                    return false;
                }
            }
            Node::ParenthesesNode { .. } => {}
            Node::ProgramNode { .. } => return false,
            Node::BeginNode { .. } => {
                let begin = parent.as_begin_node().unwrap();
                if begin.rescue_clause().is_some() {
                    return true;
                }
                if begin.ensure_clause().is_some() {
                    return false;
                }
            }
            Node::EnsureNode { .. } | Node::ForNode { .. } => return false,
            Node::DefNode { .. } => {
                let name = parent.as_def_node().unwrap().name();
                let name = name.as_slice();
                let setter =
                    name.ends_with(b"=") && !matches!(name, b"==" | b"!=" | b"<=" | b">=" | b"===");
                return !(name == b"initialize" || setter);
            }
            Node::BlockNode { .. } => {
                return !ancestors
                    .next()
                    .and_then(|n| n.as_call_node())
                    .is_some_and(|c| matches!(c.name().as_slice(), b"each" | b"tap"));
            }
            _ => return true,
        }
        child = key(parent);
    }
    false
}

/// Upstream's corrector: `each` becomes `map`, the empty-array assignment
/// (or `tap`) goes, the push becomes its argument, and the result is
/// assigned, a trailing read of the variable removed.
#[allow(clippy::too_many_arguments)]
fn correction(
    each: &ruby_prism::CallNode<'_>,
    push: &ruby_prism::CallNode<'_>,
    argument: &Node<'_>,
    name: &[u8],
    parent: &ruby_prism::StatementsNode<'_>,
    assignment: Option<Range<usize>>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    use crate::offense::{space_left, space_right, space_right_blank};
    let bytes = &file.bytes;
    let span = |l: &ruby_prism::Location<'_>| l.start_offset()..l.end_offset();
    let start = each.location().start_offset();
    let mut edits = vec![(span(&each.message_loc()?), "map".to_owned())];
    match assignment {
        Some(asgn) => {
            let end = space_right_blank(bytes, space_right(bytes, asgn.end));
            edits.push((asgn.start..end, String::new()));
        }
        None => {
            // [].tap { |dest| ... }: the tap through the `each`, and the
            // block's end with the blanks before it.
            let mut ancestors = cx.ancestors();
            ancestors.next();
            let block = ancestors.next()?.as_block_node()?;
            let tap = ancestors.next()?;
            edits.push((tap.location().start_offset()..start, String::new()));
            let close = block.closing_loc();
            edits.push((
                space_left(bytes, close.start_offset())..close.end_offset(),
                String::new(),
            ));
        }
    }
    // correct_push_node
    let (push_loc, arg_loc) = (push.location(), argument.location());
    if argument.as_keyword_hash_node().is_some() {
        edits.push((
            arg_loc.start_offset()..arg_loc.start_offset(),
            "{ ".to_owned(),
        ));
        edits.push((arg_loc.end_offset()..arg_loc.end_offset(), " }".to_owned()));
    }
    edits.push((
        push_loc.start_offset()..arg_loc.start_offset(),
        String::new(),
    ));
    edits.push((arg_loc.end_offset()..push_loc.end_offset(), String::new()));
    // correct_return_value_handling
    let body: Vec<_> = parent.body().iter().collect();
    let index = body
        .iter()
        .position(|n| n.location().start_offset() == start)?;
    if let Some(next) = body.get(index + 1)
        && next
            .as_local_variable_read_node()
            .is_some_and(|r| r.name().as_slice() == name)
    {
        let loc = next.location();
        edits.push((
            space_left(bytes, loc.start_offset())..loc.end_offset(),
            String::new(),
        ));
    }
    edits.push((
        start..start,
        format!("{} = ", String::from_utf8_lossy(name)),
    ));
    crate::offense::Correction::from_edits(bytes, edits)
}
fn empty_tap(cx: &NodeContext<'_, '_>, declaration: &Range<usize>) -> bool {
    let mut ancestors = cx.ancestors();
    ancestors.next(); // sole statement
    let Some(block) = ancestors.next().and_then(|n| n.as_block_node()) else {
        return false;
    };
    let Some(tap) = ancestors.next().and_then(|n| n.as_call_node()) else {
        return false;
    };
    if tap.name().as_slice() != b"tap"
        || tap.is_safe_navigation()
        || !ast::call_arguments(&tap).is_empty()
        || !tap.receiver().is_some_and(|n| empty_literal(&n))
    {
        return false;
    }
    let Some(p) = block
        .parameters()
        .and_then(|n| n.as_block_parameters_node())
    else {
        return false;
    };
    if !p.locals().is_empty() {
        return false;
    }
    let Some(p) = p.parameters() else {
        return false;
    };
    if p.requireds().len() != 1
        || !p.optionals().is_empty()
        || p.rest()
            .is_some_and(|r| r.as_implicit_rest_node().is_none())
        || !p.posts().is_empty()
        || !p.keywords().is_empty()
        || p.keyword_rest().is_some()
        || p.block().is_some()
    {
        return false;
    }
    p.requireds()
        .iter()
        .next()
        .and_then(|n| n.as_required_parameter_node())
        .is_some_and(|n| {
            n.location().start_offset() == declaration.start
                && n.location().end_offset() == declaration.end
        })
}
fn sole_forwarded_keywords(node: &Node<'_>) -> bool {
    let elements = if let Some(hash) = node.as_hash_node() {
        hash.elements()
    } else if let Some(hash) = node.as_keyword_hash_node() {
        hash.elements()
    } else {
        return false;
    };
    elements.len() == 1
        && elements
            .iter()
            .next()
            .and_then(|n| n.as_assoc_splat_node())
            .is_some_and(|n| n.value().is_none())
}
fn key(node: &Node<'_>) -> (usize, usize) {
    (node.location().start_offset(), node.location().end_offset())
}
fn empty_literal(node: &Node<'_>) -> bool {
    node.as_array_node()
        .is_some_and(|n| n.elements().is_empty())
}
fn empty_array(node: &Node<'_>) -> bool {
    if empty_literal(node) {
        return true;
    }
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if call.is_safe_navigation() || call.block().is_some() {
        return false;
    }
    let args = ast::call_arguments(&call);
    if call.receiver().is_none() {
        return call.name().as_slice() == b"Array" && args.len() == 1 && empty_literal(&args[0]);
    }
    if !call
        .receiver()
        .is_some_and(|n| ast::is_global_const(&n, &["Array"]))
    {
        return false;
    }
    match call.name().as_slice() {
        b"[]" => args.is_empty(),
        b"new" => args.is_empty() || (args.len() == 1 && empty_literal(&args[0])),
        _ => false,
    }
}
struct Assignment {
    range: Range<usize>,
    parent: Option<(usize, usize)>,
    empty: bool,
}
struct Destination {
    declaration: Range<usize>,
    assignments: Vec<Assignment>,
    assignment_starts: Vec<usize>,
    reference_ranges: Vec<Range<usize>>,
}
/// Lifetime-free destination facts. Built once, only after a syntactic candidate;
/// queries use the actual lexical identity plus binary source-range counts.
#[derive(Default)]
pub(crate) struct Facts {
    references: HashMap<usize, VariableId>,
    variables: HashMap<VariableId, Destination>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let Some(program) = root.as_program_node() else {
            return Self::default();
        };
        struct Parents {
            stack: Vec<(usize, usize)>,
            parents: HashMap<(usize, usize), (usize, usize)>,
            visits: usize,
        }
        impl NodeVisitor<'_> for Parents {
            fn enter(&mut self, node: Node<'_>) {
                self.visits += 1;
                if crate::cops::variables::is_local_assignment(&node)
                    && let Some(p) = self.stack.last()
                {
                    self.parents.insert(key(&node), *p);
                }
                self.stack.push(key(&node));
            }
            fn leave(&mut self) {
                self.stack.pop();
            }
        }
        let mut parents = Parents {
            stack: vec![],
            parents: HashMap::new(),
            visits: 0,
        };
        walk(&mut parents, root);
        let analysis = ScopeAnalysis::of(&program);
        let mut out = Self::default();
        #[cfg(test)]
        {
            out.visits = parents.visits;
        }
        for scope in analysis.scopes_in_exit_order() {
            for id in analysis.scope(scope).variables() {
                let v = analysis.variable(id);
                let mut assignments: Vec<_> = v
                    .assignments
                    .iter()
                    .map(|id| {
                        let n = &analysis.assignment(*id).node;
                        Assignment {
                            range: n.location().start_offset()..n.location().end_offset(),
                            parent: parents.parents.get(&key(n)).copied(),
                            empty: n
                                .as_local_variable_write_node()
                                .is_some_and(|n| empty_array(&n.value())),
                        }
                    })
                    .collect();
                let mut assignment_starts: Vec<_> =
                    assignments.iter().map(|a| a.range.start).collect();
                assignment_starts.sort_unstable();
                assignments
                    .sort_unstable_by_key(|a| (a.range.end, std::cmp::Reverse(a.range.start)));
                let mut reference_ranges: Vec<_> = v
                    .references
                    .iter()
                    .map(|r| {
                        let r = analysis.reference(*r);
                        if r.kind == ReferenceKind::Read {
                            out.references.insert(r.range.start, id);
                        }
                        r.range.clone()
                    })
                    .collect();
                reference_ranges.sort_unstable_by_key(|r| r.start);
                out.variables.insert(
                    id,
                    Destination {
                        declaration: v.declaration_range.clone(),
                        assignments,
                        assignment_starts,
                        reference_ranges,
                    },
                );
            }
        }
        out
    }
}

pub(crate) static MAP_INTO_ARRAY_FACTS: crate::source::Fact<Facts> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn map_into_array_facts(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        MAP_INTO_ARRAY_FACTS.get(self, || Facts::build(root))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop map_into_array_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/MapIntoArray")
            .collect()
    }
    #[test]
    fn initializers_pushes_and_block_parameters() {
        for init in [
            "[]",
            "Array.new",
            "Array[]",
            "Array([])",
            "Array.new([])",
            "::Array.new",
            "::Array[]",
        ] {
            for push in [
                "dest << e*2",
                "dest.push(e*2)",
                "dest.append(e: 2)",
                "dest.push({e:2})",
                "dest << if cond; e; else; 2; end",
                "dest << begin; foo; e; end",
            ] {
                let s = format!("dest = {init}\nsrc.each {{ |e| {push} }}\ndest\n");
                let found = offenses(&s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col), (2, 1));
                assert_eq!(
                    found[0].message,
                    "Use `map` instead of `each` to map elements into an array."
                );
                assert!(found[0].correction.is_none());
            }
        }
        for body in [
            "dest << _1*2",
            "dest << it*2",
            "dest << true",
            "dest << [1,2]",
        ] {
            assert_eq!(
                offenses(&format!("dest=[]\nsrc.each {{ {body} }}")).len(),
                1
            );
        }
    }
    #[test]
    fn scopes_siblings_reinitialization_and_nested_mapping() {
        for template in [
            "%s",
            "a = (%s)",
            "begin; %s; end",
            "def foo; %s; end",
            "def initialize; %s; end",
            "for i in foo; %s; end",
            "each { %s }",
            "block { _1; %s }",
        ] {
            let s = template.replace("%s", "dest=[]\nother\nsrc.each { |e| dest << e }\ndest");
            assert_eq!(offenses(&s).len(), 1, "{s}");
        }
        assert_eq!(
            offenses("dest=[]; use(dest); dest=[]; src.each { |e| dest << e }").len(),
            1
        );
        let s = "dest=[]\nsrc.each do |e|\n  dest << (dest2=[]; src.each { |e| dest2 << e }; dest2)\nend";
        assert_eq!(offenses(s).len(), 2);
    }
    #[test]
    fn unsafe_constant_shadowing_and_tap_reference_policy_still_detect() {
        for s in [
            "class Container; Array=Custom; dest=Array.new; src.each { |e| dest << e }; end",
            "[].tap { |dest| src.each { |e| dest << transform(e,dest) } }",
            "[].tap { |dest| src.each { |x,y| dest << [x,y] } }",
            "dest=[]; src.lazy.each { |x,y| dest << [x,y] }",
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none());
        }
        assert!(offenses("dest=[]; binding; src.each { |e| dest << e }").is_empty());
    }
    #[test]
    fn sole_empty_array_tap_and_shadowed_destinations() {
        for s in [
            "[].tap { |dest| src.each { |e| dest << e } }",
            "dest=[]; [].tap { |dest| src.each { dest << _1 } }",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "[].tap { |dest| mutate(dest); src.each { |e| dest << e } }",
            "[1].tap { |dest| src.each { |e| dest << e } }",
            "{}.tap { |dest| src.each { |e| dest << e } }",
            "[].tap { |dest, other| src.each { |e| dest << e } }",
            "dest=[]; src.each { |dest| dest << 1 }",
            "dest=[]; src.each { |e; dest| dest << e }",
            "dest=[]; def foo; src.each { |e| dest << e }; end",
            "dest=[]; foo { |dest| src.each { |e| dest << e } }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn nested_assignments_choose_the_last_completed_write_and_tap_allows_trailing_comma() {
        for s in [
            "dest=[]; dest=(dest=[]); src.each { |x| dest << x }",
            "dest = dest = []; src.each { |x| dest << x }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        assert_eq!(
            offenses("[].tap { |dest,| src.each { |x| dest << x } }").len(),
            1
        );
    }
    #[test]
    fn anonymous_keyword_forwarding_is_exempt_only_as_the_entire_hash() {
        for args in ["**", "{**}"] {
            let s = format!("def f(**); dest=[]; src.each {{ |e| dest.push({args}) }}; end");
            assert!(offenses(&s).is_empty(), "{s}");
        }
        for args in ["foo: 1, **", "**, foo: 1", "{foo: 1, **}"] {
            let s = format!("def f(**); dest=[]; src.each {{ |e| dest.push({args}) }}; end");
            assert_eq!(offenses(&s).len(), 1, "{s}");
        }
    }
    #[test]
    fn a_literal_push_block_is_not_a_single_send_body() {
        assert!(offenses("dest=[]; src.each { |x| dest.push(x) {} }").is_empty());
    }
    #[test]
    fn rejects_mutations_non_sibling_initializers_and_argument_forwarding() {
        for s in [
            "@dest=[]; src.each { |e| @dest << e }",
            "foo; src.each { _1 << bar }",
            "foo; src.each { it << bar }",
            "dest=[]; src.each(':') { |e| dest << e }",
            "dest=[]; each { |e| dest << e }",
            "dest=[]; self.each { |e| dest << e }",
            "[dest=[], src.each { |e| dest << e }]",
            "dest=[]; if cond; src.each { |e| dest << e }; end",
            "dest=[]; dest << 0; src.each { |e| dest << e }",
            "dest=[]; (dest << 1).each { |e| dest << e }",
            "dest=[]; src.each { |e| dest << dest.size }",
            "dest=[]; src.each { |e| dest.push(*e) }",
            "dest=[]; src.each { |e| dest.push(&method(:foo)) }",
            "dest=[]; src.each { |e| dest.push(e, e) }",
            "dest=[]; src.each { |e| dest << e; puts e }",
            "dest=[]; src.each { |e| next dest << e }",
            "dest=[]; src.each { |e| break dest << e }",
            "dest=[]; src.each { |e| dest&.push(e) }",
            "dest=[]; src&.each { |e| dest << e }",
            "def foo(&); dest=[]; src.each { |e| dest.push(&) }; end",
            "def foo(*); dest=[]; src.each { |e| dest.push(*) }; end",
            "def foo(**); dest=[]; src.each { |e| dest.push(**) }; end",
            "def foo(...); dest=[]; src.each { |e| dest.push(...) }; end",
            "dest=Array.new(1); src.each { |e| dest << e }",
            "dest=Other::Array.new; src.each { |e| dest << e }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
