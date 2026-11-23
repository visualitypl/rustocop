//! `Style/ArgumentsForwarding`: fixed Ruby 3.4 detection and atomic safe fixes.

use std::collections::HashSet;

use ruby_prism::{Node, Visit};

use crate::{
    cops::{
        Cop, NodeContext, ast,
        variables::{Scope, ScopeAnalysis},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct ArgumentsForwarding;
const ALL: &str = "Use shorthand syntax `...` for arguments forwarding.";
const PARTIAL: [&str; 3] = [
    "Use anonymous positional arguments forwarding (`*`).",
    "Use anonymous keyword arguments forwarding (`**`).",
    "Use anonymous block arguments forwarding (`&`).",
];
type Span = (usize, usize);
fn span(node: &Node<'_>) -> Span {
    (node.location().start_offset(), node.location().end_offset())
}

impl Cop for ArgumentsForwarding {
    fn name(&self) -> &'static str {
        "Style/ArgumentsForwarding"
    }

    fn safe_fixes(&self) -> bool {
        true
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
        // Analyze bindings once, not once per def. Each method body is then
        // visited once, stopping at nested local-variable scope boundaries.
        let analysis = ScopeAnalysis::of(&program);
        let safety = cx
            .corrections_requested()
            .then(|| file.forwarding_safety(node, &analysis));
        for scope in analysis.scopes_in_exit_order() {
            let scope = analysis.scope(scope);
            let Some(def) = scope.node.as_def_node() else {
                continue;
            };
            let (Some(parameters), Some(body)) = (def.parameters(), def.body()) else {
                continue;
            };
            let nodes = [
                parameters.rest(),
                parameters.keyword_rest(),
                parameters.block().map(|b| b.as_node()),
            ];
            if nodes.iter().all(Option::is_none) {
                continue;
            }
            let mut calls = Calls::default();
            calls.visit_parameters_node(&parameters);
            calls.visit(&body);
            let params: Vec<_> = nodes
                .into_iter()
                .enumerate()
                .map(|(kind, node)| Parameter::new(node, kind, scope, &analysis, &calls))
                .collect();
            let classifications: Vec<_> = calls
                .nodes
                .iter()
                .map(|(call, in_block)| classify(call, *in_block, &def, &params))
                .collect();
            let classified: Vec<_> = classifications.iter().flatten().collect();
            if classified.is_empty() {
                continue;
            }
            let forward_all =
                classified.iter().all(|c| c.all)
                    && !calls.nodes.iter().zip(&classifications).any(
                        |((call, _), classification)| {
                            classification.is_none()
                                && arguments(call).iter().any(anonymous_rest_or_keywords)
                        },
                    );
            let first_offense = out.len();
            if forward_all {
                for c in &classified {
                    if let Some(first) = c.first_all {
                        self.report(first, c.end, ALL, file, out);
                    }
                }
                let first = params[0].node.as_ref().or(params[1].node.as_ref()).unwrap();
                self.report(
                    span(first).0,
                    parameters.location().end_offset(),
                    ALL,
                    file,
                    out,
                );
            } else {
                let mut reported = [false; 3];
                for c in &classified {
                    for (kind, forwarded) in c.forwards.iter().enumerate() {
                        let Some(forwarded) = forwarded else { continue };
                        let Some(param) = &params[kind].node else {
                            continue;
                        };
                        if params[kind].anonymous {
                            continue;
                        }
                        if !reported[kind] {
                            let (start, end) = span(param);
                            self.report(start, end, PARTIAL[kind], file, out);
                            reported[kind] = true;
                        }
                        let (start, end) = span(forwarded);
                        self.report(start, end, PARTIAL[kind], file, out);
                    }
                }
            }
            // The bundle stands for every offense of the method. Upstream
            // corrects each separately; a partly disabled group would leave a
            // signature and its uses out of step, so it is not corrected.
            let enabled = out[first_offense..]
                .iter()
                .all(|o| file.cop_enabled_for_lines(self.name(), o.line, o.end_line));
            if out.len() > first_offense
                && enabled
                && safety.as_ref().is_some_and(|s| !s.observes_bindings)
            {
                out[first_offense].correction =
                    rewrite(&def, &calls, &params, &classifications, forward_all, file);
            }
        }
    }
}

impl ArgumentsForwarding {
    fn report(
        &self,
        start: usize,
        end: usize,
        message: &str,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: message.into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}

// The single-pass engine selects individual ranges, not edit groups. Put the
// signature and every use into ONE replacement: overlap/directive filtering can
// then never anonymize a signature while leaving a named use behind.
fn rewrite(
    def: &ruby_prism::DefNode<'_>,
    calls: &Calls<'_>,
    params: &[Parameter<'_>],
    classifications: &[Option<Classification<'_>>],
    all: bool,
    file: &SourceFile,
) -> Option<Correction> {
    if calls.nested_scope || calls.shadowing_channels {
        return None;
    }
    let parameters = def.parameters()?;
    let mut edits = Vec::new();
    let mut changed = [false; 3];
    for c in classifications.iter().flatten() {
        if all {
            edits.push(Correction {
                range: c.first_all?..c.end,
                replacement: "...".into(),
            });
            changed = [true; 3];
        } else {
            for (kind, forwarded) in c.forwards.iter().enumerate() {
                if let Some(node) = forwarded.as_ref().filter(|_| !params[kind].anonymous) {
                    let (start, end) = span(node);
                    edits.push(Correction {
                        range: start..end,
                        replacement: ["*", "**", "&"][kind].into(),
                    });
                    changed[kind] = true;
                }
            }
        }
    }
    if all {
        let start = span(params[0].node.as_ref()?).0;
        edits.push(Correction {
            range: start..parameters.location().end_offset(),
            replacement: "...".into(),
        });
    } else {
        for (kind, changed) in changed.iter().enumerate() {
            if *changed {
                let (start, end) = span(params[kind].node.as_ref()?);
                edits.push(Correction {
                    range: start..end,
                    replacement: ["*", "**", "&"][kind].into(),
                });
            }
        }
    }
    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    // Uses outside classified replacements (e.g. [*args], [*] or {**})
    // cannot survive removal of their named or anonymous parameter binding.
    for (kind, parameter) in params.iter().enumerate() {
        if changed[kind]
            && parameter.reads.iter().any(|&(start, end)| {
                let index = edits.partition_point(|edit| edit.range.start <= start);
                index == 0 || end > edits[index - 1].range.end
            })
        {
            return None;
        }
    }
    // Never discard a comment, continuation or physical line. In particular,
    // multiline complete-forwarding collapses remain diagnostic-only (__LINE__,
    // DATA offsets and CRLF must not move). Partial edits retain their lines.
    if edits.iter().any(|edit| {
        file.bytes[edit.range.clone()]
            .iter()
            .any(|b| matches!(b, b'#' | b'\n' | b'\r' | b'\\'))
    }) {
        return None;
    }
    if def.lparen_loc().is_none() {
        add_parens(
            def.name_loc().end_offset(),
            span(&parameters.as_node()).0,
            parameters.location().end_offset(),
            file,
            &mut edits,
        )?;
    }
    for ((call, _), classification) in calls.nodes.iter().zip(classifications) {
        let Some(classification) = classification else {
            continue;
        };
        if !all
            && classification
                .forwards
                .iter()
                .enumerate()
                .all(|(kind, forwarded)| forwarded.is_none() || !changed[kind])
        {
            continue;
        }
        let args = arguments(call);
        let first = span(args.first()?).0;
        let end = span(args.last()?).1;
        let selector = if let Some(call) = call.as_call_node() {
            if call.opening_loc().is_some() {
                // Complete forwarding is not legal in index argument brackets.
                if all && call.opening_loc()?.as_slice() != b"(" {
                    return None;
                }
                continue;
            }
            let name = call.name();
            let name = name.as_slice();
            let operator = ast::is_operator_method(name);
            if (name.ends_with(b"=") && !operator)
                || (call.call_operator_loc().is_none() && operator)
            {
                continue;
            }
            call.message_loc()?.end_offset()
        } else if let Some(call) = call.as_super_node() {
            if call.lparen_loc().is_some() {
                continue;
            }
            call.keyword_loc().end_offset()
        } else if let Some(call) = call.as_yield_node() {
            if call.lparen_loc().is_some() {
                continue;
            }
            call.keyword_loc().end_offset()
        } else {
            continue; // Index write/target brackets are already delimiters.
        };
        add_parens(selector, first, end, file, &mut edits)?;
    }
    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    // Keep repeated delimiter insertions: equal byte offsets need not mean
    // equal AST ownership. Overlapping nonempty edits are withheld below.
    let start = edits.first()?.range.start;
    let end = edits.last()?.range.end;
    let mut replacement = Vec::new();
    let mut offset = start;
    for edit in edits {
        if edit.range.start < offset {
            return None;
        }
        replacement.extend_from_slice(&file.bytes[offset..edit.range.start]);
        replacement.extend_from_slice(edit.replacement.as_bytes());
        offset = edit.range.end;
    }
    Some(Correction {
        range: start..end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

pub(crate) fn add_parens(
    selector: usize,
    first: usize,
    end: usize,
    file: &SourceFile,
    edits: &mut Vec<Correction>,
) -> Option<()> {
    if !file
        .bytes
        .get(selector..first)?
        .iter()
        .all(|b| matches!(b, b' ' | b'\t'))
    {
        return None;
    }
    edits.push(Correction {
        range: selector..first,
        replacement: "(".into(),
    });
    edits.push(Correction {
        range: end..end,
        replacement: ")".into(),
    });
    Some(())
}

struct Parameter<'pr> {
    node: Option<Node<'pr>>,
    anonymous: bool,
    usable: bool,
    reads: HashSet<Span>,
}
impl<'pr> Parameter<'pr> {
    fn new(
        node: Option<Node<'pr>>,
        kind: usize,
        scope: &Scope<'pr>,
        analysis: &ScopeAnalysis<'pr>,
        calls: &Calls<'pr>,
    ) -> Self {
        // `**nil` and `...` occupy Prism's rest slots but are not rest parameters.
        let node = node.filter(|node| match kind {
            0 => node.as_rest_parameter_node().is_some(),
            1 => node.as_keyword_rest_parameter_node().is_some(),
            _ => node.as_block_parameter_node().is_some(),
        });
        let name = node.as_ref().and_then(|node| match kind {
            0 => node.as_rest_parameter_node().and_then(|p| p.name()),
            1 => node.as_keyword_rest_parameter_node().and_then(|p| p.name()),
            _ => node.as_block_parameter_node().and_then(|p| p.name()),
        });
        let anonymous = node.is_some() && name.is_none();
        let redundant = match name.as_ref().map(|n| n.as_slice()) {
            None => anonymous,
            Some(name) => match kind {
                0 => [b"args".as_slice(), b"arguments"].contains(&name),
                1 => [b"kwargs".as_slice(), b"options", b"opts"].contains(&name),
                _ => [b"blk".as_slice(), b"block", b"proc"].contains(&name),
            },
        };
        let keyword: &[u8] = [b"*".as_slice(), b"**", b"&"][kind];
        let redundant = redundant
            && node.as_ref().is_some_and(|node| {
                node.location().as_slice().strip_prefix(keyword)
                    == Some(name.as_ref().map_or(b"".as_slice(), |name| name.as_slice()))
            });
        let mut reads = if anonymous {
            calls.anonymous_reads[kind].clone()
        } else {
            HashSet::new()
        };
        let mut usable = redundant;
        if let Some(name) = name {
            if let Some(variable) = scope
                .variables()
                .map(|id| analysis.variable(id))
                .find(|v| v.name == name.as_slice())
            {
                usable &= variable.assignments.is_empty() && !variable.has_pattern_bindings;
                for reference in &variable.references {
                    let reference = analysis.reference(*reference);
                    // Bare super forwards implicitly and is left intact. The
                    // upstream matcher only considers explicit local reads.
                    if reference.is_explicit() {
                        let range = (reference.range.start, reference.range.end);
                        usable &= calls.forwarding_reads.contains(&range);
                        reads.insert(range);
                    }
                }
            } else {
                usable = false;
            }
        }
        Self {
            node: node.filter(|_| redundant),
            anonymous,
            usable,
            reads,
        }
    }

    fn matches(&self, value: Option<Node<'_>>) -> bool {
        self.usable
            && if self.anonymous {
                value.is_none()
            } else {
                value.is_some_and(|v| {
                    v.as_local_variable_read_node().is_some() && self.reads.contains(&span(&v))
                })
            }
    }
}

struct Classification<'pr> {
    all: bool,
    first_all: Option<usize>,
    end: usize,
    forwards: [Option<Node<'pr>>; 3],
}

fn classify<'pr>(
    call: &Node<'pr>,
    in_block: bool,
    def: &ruby_prism::DefNode<'pr>,
    params: &[Parameter<'pr>],
) -> Option<Classification<'pr>> {
    let args = arguments(call);
    let mut forwards: [Option<Node<'pr>>; 3] = [None, None, None];
    // Only named splats participate in partial forwarding; anonymous splats
    // participate in the separate all-anonymous pattern below.
    for arg in &args {
        if let Some(splat) = arg.as_splat_node()
            && !params[0].anonymous
            && params[0].matches(splat.expression())
            && forwards[0].is_none()
        {
            forwards[0] = Some(splat.as_node());
        }
        for element in hash_elements(arg) {
            if let Some(splat) = element.as_assoc_splat_node()
                && !params[1].anonymous
                && params[1].matches(splat.value())
                && forwards[1].is_none()
            {
                forwards[1] = Some(splat.as_node());
            }
        }
        if let Some(block) = arg.as_block_argument_node()
            && params[2].matches(block.expression())
            && forwards[2].is_none()
        {
            forwards[2] = Some(block.as_node());
        }
    }
    let parameters = def.parameters().unwrap();
    let all_anonymous = !in_block
        && parameters.posts().is_empty()
        && parameters.keywords().is_empty()
        && def.receiver().is_none()
        && params.iter().all(|p| p.node.is_some() && p.anonymous)
        && call.as_call_node().is_some_and(|c| !c.is_safe_navigation())
        && args.len() >= 3
        && args[args.len() - 3..]
            .iter()
            .enumerate()
            .all(|(i, arg)| match i {
                0 => arg
                    .as_splat_node()
                    .is_some_and(|s| s.expression().is_none()),
                1 => {
                    let elements = keyword_elements(arg);
                    elements.len() == 1
                        && elements[0]
                            .as_assoc_splat_node()
                            .is_some_and(|s| s.value().is_none())
                }
                _ => arg
                    .as_block_argument_node()
                    .is_some_and(|b| b.expression().is_none()),
            });
    if forwards.iter().all(Option::is_none) && !all_anonymous {
        return None;
    }
    let rest_index = forwards[0]
        .as_ref()
        .and_then(|rest| args.iter().position(|a| span(a) == span(rest)));
    let no_post_rest = rest_index.is_none_or(|index| {
        args.get(index + 1).is_none_or(|arg| {
            arg.as_keyword_hash_node().is_some() || arg.as_block_argument_node().is_some()
        })
    });
    let no_extra_keywords = parameters.keywords().is_empty()
        && args.iter().all(|arg| {
            let elements = hash_elements(arg);
            !elements
                .iter()
                .any(|e| forwards[1].as_ref().is_some_and(|f| span(e) == span(f)))
                || elements.len() == 1
        });
    let forwards_keywords = args.iter().any(|arg| {
        keyword_elements(arg).iter().any(|element| {
            forwards[1]
                .as_ref()
                .is_some_and(|f| span(element) == span(f))
        })
    });
    // Unlike leading required/default parameters, post-rest parameters
    // would disappear inside the proposed `...` range. Keep their binding.
    let all = all_anonymous
        || (parameters.posts().is_empty()
            && params.iter().all(|p| p.usable)
            && forwards.iter().all(Option::is_some)
            && no_post_rest
            && no_extra_keywords
            && forwards_keywords);
    let first_all = if all_anonymous {
        Some(span(&args[args.len() - 3]).0)
    } else {
        forwards[0].as_ref().map(|f| span(f).0)
    };
    Some(Classification {
        all,
        first_all,
        end: args.last().map_or(0, |a| span(a).1),
        forwards,
    })
}

#[derive(Default)]
struct Calls<'pr> {
    nodes: Vec<(Node<'pr>, bool)>,
    forwarding_reads: HashSet<Span>,
    anonymous_reads: [HashSet<Span>; 3],
    block_depth: usize,
    nested_scope: bool,
    shadowing_channels: bool,
}
impl Calls<'_> {
    fn allow(&mut self, node: Option<Node<'_>>) {
        if let Some(node) = node.filter(|n| n.as_local_variable_read_node().is_some()) {
            self.forwarding_reads.insert(span(&node));
        }
    }
}
impl<'pr> Visit<'pr> for Calls<'pr> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.nodes.push((node.as_node(), self.block_depth > 0));
        ruby_prism::visit_call_node(self, node);
    }
    fn visit_super_node(&mut self, node: &ruby_prism::SuperNode<'pr>) {
        self.nodes.push((node.as_node(), self.block_depth > 0));
        ruby_prism::visit_super_node(self, node);
    }
    fn visit_yield_node(&mut self, node: &ruby_prism::YieldNode<'pr>) {
        self.nodes.push((node.as_node(), self.block_depth > 0));
        ruby_prism::visit_yield_node(self, node);
    }
    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        self.nodes.push((node.as_node(), self.block_depth > 0));
        ruby_prism::visit_index_operator_write_node(self, node);
    }
    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        self.nodes.push((node.as_node(), self.block_depth > 0));
        ruby_prism::visit_index_or_write_node(self, node);
    }
    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        self.nodes.push((node.as_node(), self.block_depth > 0));
        ruby_prism::visit_index_and_write_node(self, node);
    }
    fn visit_index_target_node(&mut self, node: &ruby_prism::IndexTargetNode<'pr>) {
        self.nodes.push((node.as_node(), self.block_depth > 0));
        ruby_prism::visit_index_target_node(self, node);
    }
    fn visit_splat_node(&mut self, node: &ruby_prism::SplatNode<'pr>) {
        if node.expression().is_none() {
            self.anonymous_reads[0].insert(span(&node.as_node()));
        }
        self.allow(node.expression());
        ruby_prism::visit_splat_node(self, node);
    }
    fn visit_assoc_splat_node(&mut self, node: &ruby_prism::AssocSplatNode<'pr>) {
        if node.value().is_none() {
            self.anonymous_reads[1].insert(span(&node.as_node()));
        }
        self.allow(node.value());
        ruby_prism::visit_assoc_splat_node(self, node);
    }
    fn visit_block_argument_node(&mut self, node: &ruby_prism::BlockArgumentNode<'pr>) {
        if node.expression().is_none() {
            self.anonymous_reads[2].insert(span(&node.as_node()));
        }
        self.allow(node.expression());
        ruby_prism::visit_block_argument_node(self, node);
    }
    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        self.shadowing_channels |= node
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
            .and_then(|p| p.parameters())
            .is_some_and(|p| shadows_channels(&p));
        self.block_depth += 1;
        ruby_prism::visit_block_node(self, node);
        self.block_depth -= 1;
    }
    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        self.shadowing_channels |= node
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
            .and_then(|p| p.parameters())
            .is_some_and(|p| shadows_channels(&p));
        self.block_depth += 1;
        ruby_prism::visit_lambda_node(self, node);
        self.block_depth -= 1;
    }
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        self.nested_scope = true;
        if let Some(receiver) = node.receiver() {
            self.visit(&receiver);
        }
    }
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        self.nested_scope = true;
        self.visit(&node.constant_path());
        if let Some(superclass) = node.superclass() {
            self.visit(&superclass);
        }
    }
    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        self.nested_scope = true;
        self.visit(&node.constant_path());
    }
    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        self.nested_scope = true;
        self.visit(&node.expression());
    }
}

fn shadows_channels(parameters: &ruby_prism::ParametersNode<'_>) -> bool {
    parameters.rest().is_some()
        || parameters.keyword_rest().is_some()
        || parameters.block().is_some()
}

/// Correction-only, one complete main-AST walk. Named locals and parameter
/// metadata can escape without a LocalVariableReadNode (eval, tracing, callback
/// hooks, indirect selectors). Ordinary external dispatch/monkey patches cannot
/// be inferred; visible observers and unknown block-selector dispatch end the
/// proof. Forwarding an unchanged, unwritten method block parameter is allowed.
#[derive(Default)]
pub(crate) struct Safety {
    pub(crate) observes_bindings: bool,
    approved_blocks: HashSet<Span>,
    approved_reads: HashSet<Span>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    pub(crate) fn build(root: &Node<'_>, analysis: &ScopeAnalysis<'_>) -> Self {
        let mut safety = Self::default();
        for id in analysis.scopes_in_exit_order() {
            let scope = analysis.scope(id);
            let Some(parameters) = scope.node.as_def_node().and_then(|d| d.parameters()) else {
                continue;
            };
            let names = [
                parameters
                    .rest()
                    .and_then(|p| p.as_rest_parameter_node())
                    .and_then(|p| p.name()),
                parameters
                    .keyword_rest()
                    .and_then(|p| p.as_keyword_rest_parameter_node())
                    .and_then(|p| p.name()),
                parameters.block().and_then(|p| p.name()),
            ];
            for (kind, name) in names.into_iter().enumerate() {
                let Some(name) = name else { continue };
                if let Some(variable) = scope
                    .variables()
                    .map(|id| analysis.variable(id))
                    .find(|v| v.name == name.as_slice())
                    && variable.assignments.is_empty()
                    && !variable.has_pattern_bindings
                {
                    for id in &variable.references {
                        let range = &analysis.reference(*id).range;
                        let span = (range.start, range.end);
                        safety.approved_reads.insert(span);
                        if kind == 2 {
                            safety.approved_blocks.insert(span);
                        }
                    }
                }
            }
        }
        crate::cops::walk::walk(&mut safety, root);
        safety
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Safety {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        self.observes_bindings |= matches!(node, Node::SourceFileNode { .. })
            || node.as_constant_read_node().is_some_and(|c| {
                matches!(c.name().as_slice(), b"TracePoint" | b"RubyVM" | b"DATA")
            })
            || node.as_constant_path_node().is_some_and(|c| {
                c.name()
                    .is_some_and(|n| matches!(n.as_slice(), b"TracePoint" | b"RubyVM" | b"DATA"))
            })
            || node.as_call_node().is_some_and(|c| {
                observer(c.name().as_slice())
                    || (c.name().as_slice() == b"call"
                        && !c.receiver().is_some_and(|r| r.as_lambda_node().is_some())
                        && !arguments(&c.as_node()).iter().any(|arg| {
                            let approved = |value: Option<Node<'_>>| {
                                value.is_none_or(|v| self.approved_reads.contains(&span(&v)))
                            };
                            arg.as_splat_node()
                                .is_some_and(|s| approved(s.expression()))
                                || keyword_elements(arg).iter().any(|e| {
                                    e.as_assoc_splat_node().is_some_and(|s| approved(s.value()))
                                })
                                || arg
                                    .as_block_argument_node()
                                    .is_some_and(|b| approved(b.expression()))
                                || matches!(arg, Node::ForwardingArgumentsNode { .. })
                        }))
            })
            || node
                .as_call_or_write_node()
                .is_some_and(|c| indirect_observer(c.read_name().as_slice()))
            || node
                .as_call_and_write_node()
                .is_some_and(|c| indirect_observer(c.read_name().as_slice()))
            || node
                .as_call_operator_write_node()
                .is_some_and(|c| indirect_observer(c.read_name().as_slice()))
            || node
                .as_symbol_node()
                .is_some_and(|s| indirect_observer(s.unescaped()))
            || node.as_def_node().is_some_and(|d| {
                matches!(
                    d.name().as_slice(),
                    b"method_added"
                        | b"singleton_method_added"
                        | b"inherited"
                        | b"method_missing"
                        | b"respond_to_missing?"
                )
            })
            || node.as_block_argument_node().is_some_and(|b| {
                b.expression().is_some_and(|e| {
                    e.as_nil_node().is_none() && !self.approved_blocks.contains(&span(&e))
                })
            });
    }
}
fn indirect_observer(name: &[u8]) -> bool {
    observer(name) || name == b"call"
}
pub(super) fn observer(name: &[u8]) -> bool {
    matches!(
        name,
        b"local_variables"
            | b"binding"
            | b"eval"
            | b"class_eval"
            | b"module_eval"
            | b"instance_eval"
            | b"method"
            | b"public_method"
            | b"singleton_method"
            | b"instance_method"
            | b"public_instance_method"
            | b"parameters"
            | b"arity"
            | b"source_location"
            | b"set_trace_func"
            | b"send"
            | b"__send__"
            | b"public_send"
            | b"to_proc"
            | b"enum_for"
            | b"to_enum"
            | b"alias_method"
            | b"const_get"
            | b"load"
            | b"require"
            | b"require_relative"
            | b"method_added"
            | b"singleton_method_added"
            | b"inherited"
            | b"method_missing"
            | b"respond_to_missing?"
    )
}

fn hash_elements<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    if let Some(hash) = node.as_hash_node() {
        hash.elements().iter().collect()
    } else if let Some(hash) = node.as_keyword_hash_node() {
        hash.elements().iter().collect()
    } else {
        vec![]
    }
}
// An explicit positional `{ **kwargs }` must remain positional: only an
// unbraced keyword hash can be replaced as part of complete `...` forwarding.
fn keyword_elements<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    node.as_keyword_hash_node()
        .map(|hash| hash.elements().iter().collect())
        .unwrap_or_default()
}
fn anonymous_rest_or_keywords(node: &Node<'_>) -> bool {
    node.as_splat_node()
        .is_some_and(|s| s.expression().is_none())
        || hash_elements(node)
            .iter()
            .any(|e| e.as_assoc_splat_node().is_some_and(|s| s.value().is_none()))
}
fn arguments<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    let (arguments, block) = if let Some(call) = node.as_call_node() {
        (call.arguments(), call.block())
    } else if let Some(call) = node.as_super_node() {
        (call.arguments(), call.block())
    } else if let Some(call) = node.as_yield_node() {
        (call.arguments(), None)
    } else if let Some(call) = node.as_index_operator_write_node() {
        (call.arguments(), call.block().map(|b| b.as_node()))
    } else if let Some(call) = node.as_index_or_write_node() {
        (call.arguments(), call.block().map(|b| b.as_node()))
    } else if let Some(call) = node.as_index_and_write_node() {
        (call.arguments(), call.block().map(|b| b.as_node()))
    } else if let Some(call) = node.as_index_target_node() {
        (call.arguments(), call.block().map(|b| b.as_node()))
    } else {
        return vec![];
    };
    let mut args: Vec<_> = arguments
        .map(|a| a.arguments().iter().collect())
        .unwrap_or_default();
    if let Some(block) = block.filter(|b| b.as_block_argument_node().is_some()) {
        args.push(block)
    }
    args
}

pub(crate) static FORWARDING_SAFETY: crate::source::Fact<Safety> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn forwarding_safety(
        &self,
        root: &ruby_prism::Node<'_>,
        analysis: &crate::cops::variables::ScopeAnalysis<'_>,
    ) -> &Safety {
        FORWARDING_SAFETY.get(self, || Safety::build(root, analysis))
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/arguments_forwarding_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/ArgumentsForwarding")
            .collect()
    }

    #[test]
    fn every_applicable_pinned_correction_pair_is_accounted_for() {
        // All 88 matching expect_correction calls at the pin, deduplicated to
        // 80 exact pairs. spec_lines identify every original correction call.
        let pairs: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/arguments_forwarding_corrections.json"
        ))
        .unwrap();
        let pairs = pairs.as_array().unwrap();
        assert_eq!(pairs.len(), 80);
        assert_eq!(
            pairs
                .iter()
                .map(|p| p["spec_lines"].as_array().unwrap().len())
                .sum::<usize>(),
            88
        );
        for pair in pairs {
            let before = pair["before"].as_str().unwrap();
            let after = if pair["withheld"].is_null() {
                pair["after"].as_str().unwrap()
            } else {
                before
            };
            crate::cops::testing::assert_correction(&super::ArgumentsForwarding, before, after);
        }
    }

    #[test]
    fn canonical_safe_corrections_are_atomic_and_parenthesize_commands() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "def foo(*args, **kwargs, &block)\n  bar(*args, **kwargs, &block)\nend\n",
                "def foo(...)\n  bar(...)\nend\n",
            ),
            (
                "def foo(*args, **kwargs, &block); -> { bar(*args, **kwargs, &block) }.call; end\n",
                "def foo(...); -> { bar(...) }.call; end\n",
            ),
            (
                "def foo *args, &block\n  bar *args, &block\nend\n",
                "def foo(*, &)\n  bar(*, &)\nend\n",
            ),
            (
                "def foo(**kwargs)\n  bar(**kwargs)\nend\n",
                "def foo(**)\n  bar(**)\nend\n",
            ),
            (
                "def foo(&block)\n  bar(&block)\nend\n",
                "def foo(&)\n  bar(&)\nend\n",
            ),
            (
                "def foo(*, **, &)\n  bar(*, **, &)\nend\n",
                "def foo(...)\n  bar(...)\nend\n",
            ),
        ] {
            assert_correction(&super::ArgumentsForwarding, before, after);
            let found =
                crate::cops::testing::correction_offenses(&super::ArgumentsForwarding, before);
            assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
        }
    }

    #[test]
    fn reflection_and_indirect_observers_withhold_edits_not_diagnostics() {
        use crate::cops::testing::correction_offenses;
        for observer in [
            "local_variables",
            "binding",
            "eval('args')",
            "defined?(eval('args'))",
            "method(:foo).parameters",
            "method(:foo).arity",
            "TracePoint.new {}",
            "set_trace_func(callback)",
            "send(selector)",
            "public_send(selector)",
            "things.each(&callback)",
            "things.map(&:parameters)",
            "selector.to_proc",
            "def method_added(name); inspect; end",
            "alias method_added custom_callback",
            "define_method(:method_added, callback)",
            "alias snapshot binding",
            "p __FILE__",
            "Object.const_get(:RubyVM)",
            "p DATA.pos",
            "callback.call",
            "callback.call ||= other",
            "callback.call(extra: 1)",
            "callback.call(*values)",
            "actions.each { |action| action.call }",
            "metadata ||= callback.call",
        ] {
            let source = format!(
                "def foo(*args, **kwargs, &block); bar(*args, **kwargs, &block); end\n{observer}\n"
            );
            let found = correction_offenses(&super::ArgumentsForwarding, &source);
            assert_eq!(found.len(), 2, "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }

    #[test]
    fn uncovered_reads_and_shadowing_forwarding_channels_are_withheld_atomically() {
        use crate::cops::testing::correction_offenses;
        for body in [
            "bar(*args); [*args]",
            "bar(*args, *args)",
            "bar(*args); items.each { |*other| baz(*args) }",
            "bar(*args); ->(*other) { baz(*args) }",
            "bar(*args); def inner; baz; end",
        ] {
            let source = format!("def foo(*args); {body}; end\n");
            let found = correction_offenses(&super::ArgumentsForwarding, &source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
        for source in [
            "def foo(**kwargs); bar(**kwargs); ->(**other) { baz(**kwargs) }; end\n",
            "def foo(&block); bar(&block); ->(&other) { baz(&block) }; end\n",
        ] {
            let found = correction_offenses(&super::ArgumentsForwarding, source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }

    #[test]
    fn uncovered_anonymous_uses_withhold_complete_forwarding_not_diagnostics() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        for body in [
            "bar(*, **, &); [*]",
            "bar(*, **, &); {**}",
            "bar(*, **, &); baz([*])",
            "bar([*], *, **, &)",
            "bar({**}, *, **, &)",
            "bar(*, **, &); -> { [*] }",
            "bar(*, **, &); -> { {**} }",
        ] {
            let source = format!("def foo(*, **, &); {body}; end\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            assert!(found.iter().all(|o| o.message == super::ALL), "{source}");
            let found = correction_offenses(&super::ArgumentsForwarding, &source);
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
            assert_correction(&super::ArgumentsForwarding, &source, &source);
        }
    }

    #[test]
    fn nested_commands_and_grouped_arguments_keep_all_delimiters() {
        crate::cops::testing::assert_correction(
            &super::ArgumentsForwarding,
            "def foo(*args, &block)\n outer *args, (inner &block)\nend\n",
            "def foo(*, &)\n outer(*, (inner(&)))\nend\n",
        );
    }

    #[test]
    fn operator_commands_are_parenthesized_but_infix_setter_and_index_syntax_is_not() {
        use crate::cops::testing::assert_correction;
        for operator in ["+", "==", "===", "<=", ">=", "!=", "=~", "!~", "[]"] {
            let before = format!("def foo(*args); obj.{operator} *args; end\n");
            let after = format!("def foo(*); obj.{operator}(*); end\n");
            assert_correction(&super::ArgumentsForwarding, &before, &after);
        }
        for call in [
            "obj << {**kwargs}",
            "obj.foo = {**kwargs}",
            "obj[key] = {**kwargs}",
            "obj&.foo = {**kwargs}",
        ] {
            let before = format!("def foo(**kwargs); {call}; end\n");
            assert_correction(
                &super::ArgumentsForwarding,
                &before,
                &before.replace("**kwargs", "**"),
            );
        }
    }

    #[test]
    fn multiline_complete_forwarding_and_comments_are_never_erased() {
        use crate::cops::testing::assert_correction;
        for before in [
            "def foo(*args, # explanation\n **kwargs, &block); bar(*args, **kwargs, &block); end\n",
            "def foo(*args, **kwargs, &block)\n bar(*args,\n **kwargs, &block)\nend\np __LINE__\n",
            "def foo(*args, **kwargs, &block); bar[*args, **kwargs, &block]; end\n",
        ] {
            assert_correction(&super::ArgumentsForwarding, before, before);
        }
        assert_correction(
            &super::ArgumentsForwarding,
            "def foo *args\\\n, &block\n bar(*args, &block)\nend\n",
            "def foo(*\\\n, &)\n bar(*, &)\nend\n",
        );
        let before = "def foo(*args, **kwargs) # signature\n bar(*args, # positional\n **kwargs) # keyword\n bar(\"#{1}\", '*args', /kwargs/, {**kwargs})\nend\np __LINE__\n__END__\n*args **kwargs\n";
        let after = "def foo(*, **) # signature\n bar(*, # positional\n **) # keyword\n bar(\"#{1}\", '*args', /kwargs/, {**})\nend\np __LINE__\n__END__\n*args **kwargs\n";
        assert_correction(&super::ArgumentsForwarding, before, after);
    }

    #[test]
    fn defaults_destructuring_post_rest_and_nested_scopes_keep_bindings() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "def foo(*args, x: bar(*args)); baz(*args); end\n",
                "def foo(*, x: bar(*)); baz(*); end\n",
            ),
            // Before its declaration, kwargs is a method call, not a local.
            (
                "def foo(x = bar(**kwargs), **kwargs); baz(**kwargs); end\n",
                "def foo(x = bar(**kwargs), **); baz(**); end\n",
            ),
            (
                "def foo((x, y), **kwargs); baz(x, y, **kwargs); end\n",
                "def foo((x, y), **); baz(x, y, **); end\n",
            ),
            (
                "def foo(*args, x); bar(*args, x); end\n",
                "def foo(*, x); bar(*, x); end\n",
            ),
        ] {
            assert_correction(&super::ArgumentsForwarding, before, after);
        }
        // Before its declaration, &block is a method call returning an unknown
        // callback, not the forwarded parameter: retain this uncertain edit.
        let default_block = "def foo(x = bar(&block), &block); baz(&block); end\n";
        assert_correction(&super::ArgumentsForwarding, default_block, default_block);
        let before = "def outer(*args)\n bar(*args)\n def inner(*args); baz(*args); end\nend\n";
        assert_correction(
            &super::ArgumentsForwarding,
            before,
            "def outer(*args)\n bar(*args)\n def inner(*); baz(*); end\nend\n",
        );
    }

    #[test]
    fn partial_forwarding_keeps_positional_keyword_and_block_semantics_separate() {
        for (param, message) in [
            (
                "*args",
                "Use anonymous positional arguments forwarding (`*`).",
            ),
            (
                "**kwargs",
                "Use anonymous keyword arguments forwarding (`**`).",
            ),
            ("&block", "Use anonymous block arguments forwarding (`&`)."),
        ] {
            let source = format!("def foo({param})\n  bar({param})\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            for (offense, position) in found.iter().zip([(1, 9), (2, 7)]) {
                assert_eq!((offense.line, offense.col), position);
                assert_eq!(offense.message, message);
            }
        }
    }

    #[test]
    fn forwarding_in_parameter_defaults_is_collected_in_the_method_scope() {
        // Names before their parameter declaration are method calls, not
        // reads of that later keyword-rest/block parameter.
        for (source, expected) in [
            (
                "def foo(*args, x: bar(*args))\n  baz(*args)\nend\n",
                vec![(1, 9, 0), (1, 23, 0), (2, 7, 0)],
            ),
            (
                "def foo(x = bar(**kwargs), **kwargs)\n  baz(**kwargs)\nend\n",
                vec![(1, 28, 1), (2, 7, 1)],
            ),
            (
                "def foo(x = bar(&block), &block)\n  baz(&block)\nend\n",
                vec![(1, 26, 2), (2, 7, 2)],
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), expected.len(), "{source}");
            for (offense, (line, col, kind)) in found.iter().zip(expected) {
                assert_eq!((offense.line, offense.col), (line, col), "{source}");
                assert_eq!(offense.message, super::PARTIAL[kind], "{source}");
            }
        }
        for source in [
            "def foo(*args, x: args.size); baz(*args); end",
            "def foo(*args, x: (args = []; bar(*args))); baz(*args); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn compound_and_multiple_index_assignments_forward_arguments() {
        for param in ["*args", "**kwargs"] {
            let message = super::PARTIAL[usize::from(param == "**kwargs")];
            // Ruby 3.4 forbids keyword arguments in index assignment, but
            // keyword forwarding inside a positional hash remains legal.
            let (index, col) = if param == "**kwargs" {
                ("{**kwargs}", 8)
            } else {
                (param, 7)
            };
            for call in [
                format!("obj[{index}] += 1"),
                format!("obj[{index}] ||= 1"),
                format!("obj[{index}] &&= 1"),
                format!("obj[{index}], other = values"),
            ] {
                let source = format!("def foo({param})\n  {call}\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 2, "{source}");
                for (offense, position) in found.iter().zip([(1, 9), (2, col)]) {
                    assert_eq!((offense.line, offense.col), position, "{source}");
                    assert_eq!(offense.message, message, "{source}");
                }
                let used = source.replace("end\n", "  args.inspect; kwargs.inspect\nend\n");
                assert!(offenses(&used).is_empty(), "{used}");
            }
        }
        for source in [
            "def foo(*args); obj[args] += 1; end",
            "def foo(*args); args = []; obj[*args] ||= 1; end",
            "def foo(*args); obj[*args], args = values; end",
            "def foo(**kwargs); obj[{**kwargs}] &&= kwargs.size; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn positional_hashes_are_preserved_in_partial_forwarding() {
        for (call, positions) in [
            ("bar(*args, {**kwargs}, &block)", [(2, 7), (2, 15), (2, 26)]),
            (
                "bar(*args, {}, **kwargs, &block)",
                [(2, 7), (2, 18), (2, 28)],
            ),
            (
                "bar(*args, {extra: 1}, **kwargs, &block)",
                [(2, 7), (2, 26), (2, 36)],
            ),
        ] {
            let source = format!("def foo(*args, **kwargs, &block)\n  {call}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 6, "{source}");
            let anonymous = source
                .replace("*args", "*")
                .replace("**kwargs", "**")
                .replace("&block", "&");
            assert_eq!(ruby_prism::parse(anonymous.as_bytes()).errors().count(), 0);
            assert!(offenses(&anonymous).is_empty(), "{anonymous}");
            for (kind, (parameter_col, call_position)) in
                [9, 16, 26].into_iter().zip(positions).enumerate()
            {
                for position in [(1, parameter_col), call_position] {
                    let offense = found.iter().find(|o| (o.line, o.col) == position).unwrap();
                    assert_eq!(offense.message, super::PARTIAL[kind], "{source}");
                }
            }
        }
    }

    #[test]
    fn pattern_bindings_prevent_anonymizing_the_written_parameter() {
        for binding in [
            "[7] in [*args]",
            "[7] => [*args]",
            "case [7]; in [*args]; end",
            "items.each { [7] in [*args] }",
            "-> { [7] => [*args] }.call",
        ] {
            let source = format!("def foo(*args)\n  {binding}\n  bar(*args)\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for (binding, kind) in [("{} => kwargs", 1), ("nil => block", 2)] {
            let source = format!(
                "def foo(*args, **kwargs, &block)\n  {binding}\n  bar(*args, **kwargs, &block)\nend\n"
            );
            let found = offenses(&source);
            assert_eq!(found.len(), 4, "{source}");
            assert!(
                found
                    .iter()
                    .all(|o| o.message != super::PARTIAL[kind] && o.message != super::ALL)
            );
        }
        for shadow in [
            "items.each { |args| [7] in [*args] }",
            "items.each { |;args| [7] => [*args] }",
            "def nested; [7] in [*args]; end",
        ] {
            let source = format!("def foo(*args)\n  {shadow}\n  bar(*args)\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            assert!(found.iter().all(|o| o.message == super::PARTIAL[0]));
        }
    }

    #[test]
    fn spaced_parameter_spellings_are_not_in_the_fixed_redundant_name_lists() {
        assert!(
            offenses("def foo(* args, ** kwargs, & block); bar(*args, **kwargs, &block); end")
                .is_empty()
        );
    }

    #[test]
    fn ignores_unforwarded_meaningful_used_or_reassigned_parameters() {
        for source in [
            "def foo(*args, **kwargs, &block); end",
            "def foo(*items, **settings, &callback); bar(*items, **settings, &callback); end",
            "def foo(*args); args; bar(*args); end",
            "def foo(*args); args = []; bar(*args); end",
            "def foo(*args); args += []; bar(*args); end",
            "def foo(*args); args, x = []; bar(*args); end",
            "def foo(**kwargs); kwargs[:x]; bar(**kwargs); end",
            "def foo(&block); block.call; bar(&block); end",
            "def foo(*args); bar(*arguments); end",
            "def foo(...); bar(...); end",
            "def foo(*, **); bar(*, **); end",
            "def foo(*, x, **, &); bar(x, *, **, &); end",
            "def foo(*, k:, **, &); bar(*, **, &); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn forwards_all_anonymous_only_when_every_call_can_keep_all_channels() {
        for (signature, call, positions) in [
            ("foo(*, **, &)", "bar(*, **, &)", [(1, 9), (2, 7)]),
            (
                "foo(m, *, **, &)",
                "bar(self, *, **, &).baz",
                [(1, 12), (2, 13)],
            ),
            (
                "self.foo(*args, **opts, &)",
                "obj&.bar(*args, **opts, &)",
                [(1, 14), (2, 12)],
            ),
        ] {
            let source = format!("def {signature}\n  {call}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            for (o, pos) in found.iter().zip(positions) {
                assert_eq!((o.line, o.col), pos, "{source}");
                assert_eq!(o.message, super::ALL);
            }
        }
        for body in [
            "bar(*, **, &); baz(*)",
            "bar(*, **, &); baz(**)",
            "bar(*, **, &); baz(*, **)",
            "items.each { bar(*, **, &) }",
        ] {
            let source = format!("def foo(*, **, &); {body}; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn leading_arguments_allow_complete_forwarding_but_trailing_values_and_keywords_do_not() {
        for (signature, call, all, count) in [
            (
                "foo(x, y = 42, *args, **kwargs, &block)",
                "bar(effect(), x, y, *args, **kwargs, &block)",
                true,
                2,
            ),
            (
                "foo(*args, **kwargs, &block)",
                "bar(*args, effect(), **kwargs, &block)",
                false,
                6,
            ),
            (
                "foo(*args, first:, **kwargs, &block)",
                "bar(*args, **kwargs, &block)",
                false,
                6,
            ),
            (
                "foo(*args, first: nil, **kwargs, &block)",
                "bar(*args, first: first, **kwargs, &block)",
                false,
                6,
            ),
            (
                "foo(*args, **kwargs, &block)",
                "bar(*args, extra: effect(), **kwargs, &block)",
                false,
                6,
            ),
            (
                "foo(*args, **kwargs, &block)",
                "bar(*args, **kwargs, extra: effect(), &block)",
                false,
                6,
            ),
            ("foo((x, y), **kwargs)", "bar(x, y, **kwargs)", false, 2),
            ("foo(x = 1, *args, y)", "bar(*args, y, effect())", false, 2),
        ] {
            let source = format!("def {signature}\n  {call}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), count, "{source}");
            assert_eq!(
                found.iter().all(|o| o.message == super::ALL),
                all,
                "{source}"
            );
        }
    }

    #[test]
    fn independent_usage_or_writes_only_exempt_the_affected_parameter() {
        for (usage, absent) in [
            ("args.inspect", 0),
            ("args = []", 0),
            ("kwargs = {}", 1),
            ("block.call", 2),
            ("block = nil", 2),
        ] {
            let source = format!(
                "def foo(*args, **kwargs, &block)\n  {usage}\n  bar(*args, **kwargs, &block)\nend\n"
            );
            let found = offenses(&source);
            assert_eq!(found.len(), 4, "{source}");
            assert!(
                !found.iter().any(|o| o.message == super::PARTIAL[absent]),
                "{source}"
            );
        }
    }

    #[test]
    fn post_rest_required_parameters_cannot_be_removed_by_forward_all() {
        let found = offenses(
            "def foo(*args, x, **kwargs, &block)\n  bar(x, *args, **kwargs, &block)\nend\n",
        );
        assert_eq!(found.len(), 6);
        assert!(found.iter().all(|o| o.message != super::ALL));
    }

    #[test]
    fn separate_calls_and_repeated_call_sites_report_the_signature_once() {
        let found = offenses(
            "def foo(*args, **kwargs, &block)\n  bar(first(*args), second(**kwargs), third(&block))\n  bar(*args)\nend\n",
        );
        assert_eq!(found.len(), 7);
        assert_eq!(found.iter().filter(|o| o.line == 1).count(), 3);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 9), (1, 16), (1, 26), (2, 13), (2, 28), (2, 45), (3, 7)]
        );
    }

    #[test]
    fn inherited_super_and_yield_and_index_setter_operator_calls() {
        for call in [
            "super(*args, **kwargs, &block)",
            "obj&.bar(*args, **kwargs, &block)",
            "obj.(*args, **kwargs, &block)",
        ] {
            let source = format!(
                "class C < Parent\n  def foo(*args, **kwargs, &block)\n    {call}\n    super\n  end\nend\n"
            );
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            assert!(found.iter().all(|o| o.message == super::ALL));
        }
        for (param, call) in [
            ("*args", "yield(*args)"),
            ("**kwargs", "yield(**kwargs)"),
            ("*args", "bar[*args]"),
            ("*args", "bar[*args] = value"),
            ("**kwargs", "bar[key] = { x: 1, **kwargs }"),
            ("**kwargs", "obj&.bar = { x: 1, **kwargs }"),
            ("**kwargs", "obj << { x: 1, **kwargs }"),
            ("*args", "obj.+ *args"),
        ] {
            let source = format!("def foo({param})\n  {call}\nend\n");
            assert_eq!(offenses(&source).len(), 2, "{source}");
        }
    }

    #[test]
    fn blocks_capture_method_parameters_at_ruby_34_including_numbered_and_it_blocks() {
        for body in [
            "do_something { bar(*args, **kwargs, &block) }",
            "do_something { bar(*args, **kwargs, &block); baz(_1) }",
            "do_something { bar(*args, **kwargs, &block); baz(it) }",
            "-> { bar(*args, **kwargs, &block) }",
        ] {
            let source = format!("def foo(*args, **kwargs, &block)\n  {body}\nend\n");
            assert_eq!(offenses(&source).len(), 2, "{source}");
        }
        for body in [
            "bar(*args, **kwargs) { |x| x }",
            "items.each { bar(*args, **kwargs) }",
        ] {
            let source = format!("def foo(*args, **kwargs)\n  {body}\nend\n");
            assert_eq!(offenses(&source).len(), 4, "{source}");
        }
    }

    #[test]
    fn binding_and_shadowing_are_lexical_not_name_based() {
        for source in [
            "def foo(*args); items.each { |*args| bar(*args) }; end",
            "def foo(*args); items.each { |;args| bar(*args) }; end",
            "def foo(*args); def nested; args = []; bar(*args); end; end",
            "def foo(*args); class << Object.new; args = []; bar(*args); end; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let source = "def foo(*args)\n  bar(*args)\n  items.each { |*args| args.inspect }\n  def inner(*args)\n    bar(*args)\n  end\nend\n";
        assert_eq!(offenses(source).len(), 4);
        assert_eq!(
            offenses("def foo(*args); items.each { args = []; bar(*args) }; end").len(),
            0
        );
    }

    #[test]
    fn ruby_gate_and_safe_fix_policy_use_the_real_matcher() {
        use crate::{
            config::ResolvedConfig,
            cops::{Cop, Registry, policy::RubyVersion, select_active_for_version},
            pipeline::lint_str_with,
        };
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let only = ["Style/ArgumentsForwarding".to_owned()];
        assert!(super::ArgumentsForwarding.default_enabled());
        assert!(super::ArgumentsForwarding.safe_fixes());
        for (version, count) in [
            (RubyVersion(2, 6), 0),
            (RubyVersion(2, 7), 2),
            (RubyVersion(3, 4), 2),
        ] {
            let active = select_active_for_version(&registry, &config, &only, &[], version);
            assert_eq!(
                lint_str_with(
                    &active,
                    "def foo(*args, **kwargs, &block); bar(*args, **kwargs, &block); end"
                )
                .len(),
                count
            );
        }
    }

    #[test]
    fn fixed_redundant_name_lists_are_exercised_independently() {
        for parameter in [
            "*args",
            "*arguments",
            "**kwargs",
            "**options",
            "**opts",
            "&blk",
            "&block",
            "&proc",
        ] {
            let source = format!("def self.foo {parameter}\n  bar {parameter}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 14));
            assert_eq!((found[1].line, found[1].col), (2, 7));
            let sigil = if parameter.starts_with("**") {
                "**"
            } else if parameter.starts_with('*') {
                "*"
            } else {
                "&"
            };
            crate::cops::testing::assert_correction(
                &super::ArgumentsForwarding,
                &source,
                &format!("def self.foo({sigil})\n  bar({sigil})\nend\n"),
            );
        }
        assert_eq!(offenses("def foo(*args, **nil); bar(*args); end").len(), 2);
        assert_eq!(
            offenses("def foo(*args, &callback); bar(*args, &callback); end").len(),
            2
        );
    }

    #[test]
    fn complete_forwarding_tracks_multiline_ranges_not_the_literal_block() {
        let source = "def foo(\n  *args,\n  **kwargs,\n  &block\n)\n  bar(\n    *args,\n    **kwargs,\n    &block\n  )\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 4));
        assert_eq!((found[1].line, found[1].col, found[1].end_line), (7, 5, 9));
        let found = offenses(
            "def foo(*args, **kwargs)\n  bar(*args, **kwargs) do\n    effect\n  end\nend\n",
        );
        assert_eq!(found.len(), 4);
        assert!(found.iter().all(|o| o.end_line == o.line));
    }

    #[test]
    fn anonymous_partial_clean_methods_do_not_attempt_an_empty_rewrite() {
        for source in [
            "def foo(&); bar(&); end\n",
            "def foo(*, &); bar(*, &); end\n",
            "def foo(*, **, &); bar(*, **, &); baz(*); end\n",
            "def foo(*, **, &); bar(*, **, &); baz({**}); end\n",
        ] {
            assert!(
                crate::cops::testing::correction_offenses(&super::ArgumentsForwarding, source)
                    .is_empty()
            );
        }
    }

    #[test]
    fn correction_proofs_and_envelopes_have_bounded_source_work() {
        use crate::cops::{
            testing::correction_offenses,
            variables::ScopeAnalysis,
            walk::{self, NodeVisitor},
        };
        #[derive(Default)]
        struct Count(usize);
        impl<'pr> NodeVisitor<'pr> for Count {
            fn enter(&mut self, _: ruby_prism::Node<'pr>) {
                self.0 += 1;
            }
        }
        for size in [20, 80, 160] {
            let source: String = (0..size)
                .map(|i| format!("def foo{i}(*args); bar(*args); baz(*args); end\n"))
                .collect();
            let parse = ruby_prism::parse(source.as_bytes());
            let root = parse.node();
            let analysis = ScopeAnalysis::of(&root.as_program_node().unwrap());
            let safety = super::Safety::build(&root, &analysis);
            let mut count = Count::default();
            walk::walk(&mut count, &root);
            assert_eq!(safety.visits, count.0);
            assert!(safety.visits < source.len());
            let found = correction_offenses(&super::ArgumentsForwarding, &source);
            let edits: Vec<_> = found.iter().filter_map(|o| o.correction.as_ref()).collect();
            assert_eq!(edits.len(), size);
            assert!(edits.iter().map(|c| c.range.len()).sum::<usize>() <= source.len());
        }
        let source = format!(
            "{}{}",
            "def foo(*args); bar(*args)\n".repeat(24),
            "end\n".repeat(24)
        );
        let found = correction_offenses(&super::ArgumentsForwarding, &source);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
    }

    #[test]
    fn method_collection_is_bounded_across_nested_and_wide_definitions() {
        use crate::cops::{Cop, variables::ScopeAnalysis};
        use ruby_prism::Visit;
        for nested in [false, true] {
            // Exercise overlapping ranges without relying on enlarged thread
            // stacks for the shared recursive scope analyzer.
            let size = if nested { 24 } else { 100 };
            let mut source = String::new();
            for i in 0..size {
                source.push_str(&format!("def foo{i}(*args, x: baz(*args))\n  bar(*args)\n"));
                if !nested {
                    source.push_str("end\n");
                }
            }
            if nested {
                source.push_str(&"end\n".repeat(size));
            }
            let parse = ruby_prism::parse(source.as_bytes());
            assert_eq!(parse.errors().count(), 0);
            let analysis = ScopeAnalysis::of(&parse.node().as_program_node().unwrap());
            let mut calls = 0;
            let mut reads = 0;
            for id in analysis.scopes_in_exit_order() {
                let scope = analysis.scope(id);
                if let Some(def) = scope.node.as_def_node() {
                    let mut collector = super::Calls::default();
                    collector.visit_parameters_node(&def.parameters().unwrap());
                    collector.visit(&scope.body().unwrap());
                    calls += collector.nodes.len();
                    reads += collector.forwarding_reads.len();
                }
            }
            assert_eq!((calls, reads), (2 * size, 2 * size));
            let active: [&dyn Cop; 1] = [&super::ArgumentsForwarding];
            assert_eq!(
                crate::pipeline::lint_str_with(&active, &source).len(),
                3 * size
            );
        }
    }

    #[test]
    fn complete_forwarding_reports_definition_and_each_call() {
        let found = offenses(
            "def foo(*args, **kwargs, &block)\n  bar(*args, **kwargs, &block)\n  baz(*args, **kwargs, &block)\nend\n",
        );
        assert_eq!(found.len(), 3);
        for (offense, position) in found.iter().zip([(1, 9), (2, 7), (3, 7)]) {
            assert_eq!((offense.line, offense.col), position);
            assert_eq!(
                offense.message,
                "Use shorthand syntax `...` for arguments forwarding."
            );
            assert!(offense.correction.is_none());
        }
    }
}
