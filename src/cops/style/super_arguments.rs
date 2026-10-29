//! Identical explicit and implicit super forwarding, with separate Ruby channels.
use crate::{
    cops::{
        Cop, NodeContext,
        variables::ScopeAnalysis,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Node, SuperNode};
use std::collections::HashSet;
pub struct SuperArguments;
impl Cop for SuperArguments {
    fn name(&self) -> &'static str {
        "Style/SuperArguments"
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
        let Some(call) = node.as_super_node() else {
            return;
        };
        let mut definition = None;
        for ancestor in cx.ancestors() {
            // A block may be delegated to define_method. Implicit super would
            // raise there. Prism puts the call's own block below the call.
            if matches!(
                ancestor,
                Node::BlockNode { .. }
                    | Node::LambdaNode { .. }
                    | Node::ClassNode { .. }
                    | Node::ModuleNode { .. }
                    | Node::SingletonClassNode { .. }
            ) {
                return;
            }
            if let Some(def) = ancestor.as_def_node() {
                definition = Some(def);
                break;
            }
        }
        let Some(def) = definition else { return };
        // The receiver of a singleton definition evaluates in the outer scope.
        if def.body().is_none_or(|body| {
            !std::iter::once(node).chain(cx.ancestors()).any(|a| {
                a.location().start_offset() == body.location().start_offset()
                    && a.location().end_offset() == body.location().end_offset()
                    && std::mem::discriminant(a) == std::mem::discriminant(&body)
            })
        }) {
            return;
        }
        let mut params = Vec::new();
        if let Some(p) = def.parameters() {
            params.extend(p.requireds().iter());
            params.extend(p.optionals().iter());
            params.extend(p.rest());
            params.extend(p.posts().iter());
            params.extend(p.keywords().iter());
            params.extend(p.keyword_rest());
            params.extend(p.block().map(|b| b.as_node()));
        }
        let inline = call.block().is_some_and(|b| b.as_block_node().is_some());
        let has_block = params
            .last()
            .is_some_and(|p| p.as_block_parameter_node().is_some());
        let mut args = Vec::new();
        if let Some(arguments) = call.arguments() {
            for arg in arguments.arguments().iter() {
                if let Some(hash) = arg.as_keyword_hash_node() {
                    args.extend(hash.elements().iter());
                } else {
                    args.push(arg);
                }
            }
        }
        if let Some(block) = call
            .block()
            .filter(|b| b.as_block_argument_node().is_some())
        {
            args.push(block);
        }
        let expected = params.len() - usize::from(inline && has_block);
        if expected != args.len() {
            return;
        }
        for (param, arg) in params.iter().take(expected).zip(&args) {
            if !identical(param, arg) {
                return;
            }
            if param.as_block_parameter_node().is_some() {
                let root = cx.ancestors().next_back().unwrap_or(node);
                if file
                    .super_facts(root)
                    .rebound_blocks
                    .contains(&def.location().start_offset())
                {
                    return;
                }
            }
        }
        let start = call.keyword_loc().start_offset();
        let end = source_end(&call);
        let (line, col) = file.line_col(start);
        // Never consume physical lines or embedded comments. That would change
        // __LINE__, source locations and DATA positions or discard directives.
        let text = &file.bytes[start..end];
        let correction = (cx.corrections_requested()
            && keyword_order_preserved(&params)
            && bare_super_parse_preserved(file, &call, end)
            // Implicit super uses the original block, not a block parameter
            // mutated through Binding or eval.
            && !(has_block
                && !inline
                && file
                    .super_facts(cx.ancestors().next_back().unwrap_or(node))
                    .observes_block_mutation)
            && !text.contains(&b'\n')
            && !super::correction::contains_comment(file, start..end)
            && {
                let proof = file.class_method_bindings(cx.ancestors().next_back().unwrap_or(node));
                !proof.observes_data && !proof.observes_program_name
            })
        .then(|| Correction {
            range: start..end,
            replacement: "super".into(),
        });
        let message = if inline && has_block {
            "Call `super` without arguments and parentheses when all positional and keyword arguments are forwarded."
        } else {
            "Call `super` without arguments and parentheses when the signature is identical."
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: message.into(),
            correction,
        });
    }
}

// Prove a boundary, rather than blacklist a few operators: bare super can
// consume operands, `<<` can become a heredoc, and removing `)` can join a tight
// keyword (including `do`) to `super`. Never skip explicit line continuations.
// Only inspect adjacent horizontal trivia and a bounded token prefix; block
// ownership comes from the main AST. No reparses or per-candidate file walks.
fn bare_super_parse_preserved(file: &SourceFile, call: &SuperNode<'_>, end: usize) -> bool {
    let rest = &file.bytes[end..];
    let trivia = rest
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t' | b'\r'))
        .count();
    let tail = &rest[trivia..];
    match tail.first() {
        None | Some(b'\n' | b'#' | b';' | b',' | b')' | b']' | b'}') => true,
        // A method chain, but not a range/forwarding token.
        Some(b'.') => tail.get(1) != Some(&b'.'),
        Some(b'{') => call
            .block()
            .and_then(|b| b.as_block_node())
            .is_some_and(|b| b.opening_loc().start_offset() == end + trivia),
        Some(b'd') if trivia > 0 => call
            .block()
            .and_then(|b| b.as_block_node())
            .is_some_and(|b| {
                b.opening_loc().start_offset() == end + trivia
                    && b.opening_loc().as_slice() == b"do"
            }),
        _ if trivia > 0 => [b"if".as_slice(), b"unless", b"rescue", b"and", b"or"]
            .iter()
            .any(|keyword| {
                tail.starts_with(keyword)
                    && tail
                        .get(keyword.len())
                        .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_' && b.is_ascii())
            }),
        _ => false,
    }
}

// Implicit super builds rest keywords first, then required and optional
// keyword slots. Explicit forwarding follows source order instead. Preserve
// observable Hash insertion order, not just values and Ruby 3 keyword channels.
fn keyword_order_preserved(params: &[Node<'_>]) -> bool {
    let mut optional = false;
    let mut named = false;
    for param in params {
        if param.as_optional_keyword_parameter_node().is_some() {
            optional = true;
            named = true;
        }
        if param.as_required_keyword_parameter_node().is_some() {
            if optional {
                return false;
            }
            named = true;
        }
        if named && param.as_keyword_rest_parameter_node().is_some() {
            return false;
        }
    }
    true
}

fn identical(param: &Node<'_>, arg: &Node<'_>) -> bool {
    let same = |name: &[u8], value: &Node<'_>| {
        value
            .as_local_variable_read_node()
            .is_some_and(|v| v.name().as_slice() == name)
    };
    if let Some(p) = param.as_required_parameter_node() {
        return same(p.name().as_slice(), arg);
    }
    if let Some(p) = param.as_optional_parameter_node() {
        return same(p.name().as_slice(), arg);
    }
    if let Some(p) = param.as_rest_parameter_node() {
        return arg
            .as_splat_node()
            .is_some_and(|a| match (p.name(), a.expression()) {
                (None, None) => true,
                (Some(n), Some(v)) => same(n.as_slice(), &v),
                _ => false,
            });
    }
    if let Some(p) = param.as_keyword_rest_parameter_node() {
        return arg
            .as_assoc_splat_node()
            .is_some_and(|a| match (p.name(), a.value()) {
                (None, None) => true,
                (Some(n), Some(v)) => same(n.as_slice(), &v),
                _ => false,
            });
    }
    if let Some(p) = param.as_block_parameter_node() {
        return arg
            .as_block_argument_node()
            .is_some_and(|a| match (p.name(), a.expression()) {
                (None, None) => true,
                (Some(n), Some(v)) => same(n.as_slice(), &v),
                _ => false,
            });
    }
    let keyword = param
        .as_required_keyword_parameter_node()
        .map(|p| p.name())
        .or_else(|| param.as_optional_keyword_parameter_node().map(|p| p.name()));
    if let Some(name) = keyword {
        return arg.as_assoc_node().is_some_and(|a| {
            a.key()
                .as_symbol_node()
                .is_some_and(|key| key.unescaped() == name.as_slice())
                && same(
                    name.as_slice(),
                    &a.value()
                        .as_implicit_node()
                        .map_or_else(|| a.value(), |v| v.value()),
                )
        });
    }
    param.as_forwarding_parameter_node().is_some() && arg.as_forwarding_arguments_node().is_some()
}

/// Built at most once from the main parse. ScopeAnalysis distinguishes captured
/// writes from shadowed block/lambda locals, pattern bindings and nested defs.
#[derive(Default)]
pub(crate) struct Facts {
    rebound_blocks: HashSet<usize>,
    observes_block_mutation: bool,
    pub(super) observes_parameters: bool,
    pub(super) implicit_super_scopes: HashSet<usize>,
    owners: Vec<Option<usize>>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn of(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        if let Some(program) = root.as_program_node() {
            let analysis = ScopeAnalysis::of(&program);
            for id in analysis.scopes_in_exit_order() {
                let scope = analysis.scope(id);
                let Some(def) = scope.node.as_def_node() else {
                    continue;
                };
                let Some(block) = def
                    .parameters()
                    .and_then(|p| p.block())
                    .and_then(|p| p.name())
                else {
                    continue;
                };
                if scope.variables().map(|id| analysis.variable(id)).any(|v| {
                    v.name == block.as_slice()
                        && (!v.assignments.is_empty() || v.has_pattern_bindings)
                }) {
                    facts.rebound_blocks.insert(def.location().start_offset());
                }
            }
        }
        walk(&mut facts, root);
        facts
    }
}
impl<'pr> NodeVisitor<'pr> for Facts {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        let owner = match node {
            Node::DefNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. } => {
                Some(node.location().start_offset())
            }
            _ => None,
        };
        self.owners.push(owner);
        if node.as_forwarding_super_node().is_some() {
            self.implicit_super_scopes
                .extend(self.owners.iter().flatten());
        }
        // Reordering changes parameter/local reflection. Recognize indirect
        // observation too, including symbol-to-proc and dynamic dispatch.
        self.observes_parameters |= observes(&node, parameter_observer);
        self.observes_block_mutation |= observes(&node, block_mutator);
    }
    fn leave(&mut self) {
        self.owners.pop();
    }
}
// Whole-file and cached: a Binding may escape its defining scope, or mutation
// may be dispatched indirectly. Keep diagnostics while withholding uncertain
// block forwarding edits; syntactic reassignment is still handled by scopes.
fn block_mutator(name: &[u8]) -> bool {
    matches!(
        name,
        b"binding"
            | b"eval"
            | b"class_eval"
            | b"module_eval"
            | b"instance_eval"
            | b"local_variable_set"
            | b"send"
            | b"__send__"
            | b"public_send"
            | b"set_trace_func"
    )
}
fn observes(node: &Node<'_>, observer: fn(&[u8]) -> bool) -> bool {
    node.as_call_node()
        .is_some_and(|c| observer(c.name().as_slice()))
        || node
            .as_symbol_node()
            .is_some_and(|s| observer(s.unescaped()))
        || node
            .as_def_node()
            .is_some_and(|d| observer(d.name().as_slice()))
        || node
            .as_call_or_write_node()
            .is_some_and(|c| observer(c.read_name().as_slice()))
        || node
            .as_call_and_write_node()
            .is_some_and(|c| observer(c.read_name().as_slice()))
        || node
            .as_call_operator_write_node()
            .is_some_and(|c| observer(c.read_name().as_slice()))
        || matches!(node, Node::SourceFileNode { .. })
        || node
            .as_constant_read_node()
            .is_some_and(|n| matches!(n.name().as_slice(), b"TracePoint" | b"RubyVM"))
        || node.as_constant_path_node().is_some_and(|n| {
            n.name()
                .is_some_and(|n| matches!(n.as_slice(), b"TracePoint" | b"RubyVM"))
        })
}
fn parameter_observer(name: &[u8]) -> bool {
    super::arguments_forwarding::observer(name) || name == b"call"
}
/// Prism includes literal blocks; only the invocation is replaced/highlighted.
pub(super) fn source_end(call: &SuperNode<'_>) -> usize {
    let mut end = call
        .rparen_loc()
        .map_or(call.keyword_loc().end_offset(), |l| l.end_offset());
    if let Some(args) = call.arguments() {
        end = end.max(args.location().end_offset());
    }
    if let Some(block) = call
        .block()
        .filter(|b| b.as_block_argument_node().is_some())
    {
        end = end.max(block.location().end_offset());
    }
    end
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop super_arguments_spec.rb (MIT, Copyright (c)
    //! 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn all_58_applicable_pinned_correction_calls() {
        // RuboCop MIT examples at 97f30c4; fixture lines refer to the exact
        // expect_correction call, including executed shared examples.
        #[derive(serde::Deserialize)]
        struct Pair {
            cop: String,
            line: usize,
            before: String,
            after: String,
        }
        let pairs: Vec<Pair> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/super_arguments_keywords_corrections.json"
        ))
        .unwrap();
        assert_eq!(pairs.len(), 58);
        let mut withheld = 0;
        for pair in pairs {
            let cop: &dyn Cop = match pair.cop.as_str() {
                "Style/SuperArguments" => &SuperArguments,
                "Style/SuperWithArgsParentheses" => &super::super::SuperWithArgsParentheses,
                "Style/KeywordParametersOrder" => &super::super::KeywordParametersOrder,
                _ => panic!("unknown fixture cop"),
            };
            let found = correction_offenses(cop, &pair.before);
            assert!(!found.is_empty(), "{}:{}", pair.cop, pair.line);
            if pair.cop == "Style/SuperArguments" && pair.before.contains("method(a:, **kwargs)") {
                withheld += 1;
                assert!(found.iter().all(|o| o.correction.is_none()));
                assert_correction(cop, &pair.before, &pair.before);
            } else {
                assert_correction(cop, &pair.before, &pair.after);
            }
        }
        assert_eq!(withheld, 2);
    }
    #[test]
    fn registered_default_pipeline_messages() {
        for (source, cop, line, col) in [
            ("def f(a)\n  super(a)\nend\n", "Style/SuperArguments", 2, 3),
            ("super value\n", "Style/SuperWithArgsParentheses", 1, 1),
            ("def f(a: 1, b:); end", "Style/KeywordParametersOrder", 1, 7),
        ] {
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == cop)
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (line, col));
        }
    }
    #[test]
    fn keyword_hash_insertion_order_is_part_of_correction_safety() {
        for (params, args) in [
            ("a: 1, b:", "a: a, b: b"),
            ("a:, **k", "a: a, **k"),
            ("a: 1, **k", "a: a, **k"),
        ] {
            let source = format!("def f({params}); super({args}); end");
            let found = correction_offenses(&SuperArguments, &source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{source}");
        }
        for (params, args) in [
            ("a:, b: 1", "a: a, b: b"),
            ("a:, b:", "a: a, b: b"),
            ("a: 1, b: 2", "a: a, b: b"),
            ("**k", "**k"),
        ] {
            let source = format!("def f({params}); super({args}); end");
            assert_correction(
                &SuperArguments,
                &source,
                &format!("def f({params}); super; end"),
            );
        }
    }
    #[test]
    fn distinct_channels_and_dynamic_scopes() {
        for (params, args) in [
            ("a, b", "a"),
            ("a, b", "b, a"),
            ("a:, b:", "b: b, a: a"),
            ("a, b", "a, b: b"),
            ("a, b:", "a, b"),
            ("&blk", "&other_blk"),
            ("a:", "{ a: a }"),
            ("a:, b:", "a: a, b: c"),
            ("...", "a, ..."),
            ("a, ...", "b, ..."),
            ("a, **kwargs", "a: a, **kwargs"),
            ("(a, b)", "a, b"),
            ("x, (a, *b)", "x, a, *b"),
            ("*, **", "*"),
            ("*, **", "**"),
            ("a", ""),
            ("a, &blk", "a"),
            ("a, &", "a"),
            ("**nil", ""),
            ("a:", "**{a: a}"),
            ("a:", "a: side_effect(a)"),
            ("a:", "a: a, **{}"),
        ] {
            let source = format!("def method({params}); super({args}); end");
            assert!(
                correction_offenses(&SuperArguments, &source).is_empty(),
                "{source}"
            );
        }
        for source in [
            "super(a)",
            "describe 'example' do; subject { super() }; end",
            "def f(a); delegate_to_define_method { super(a) }; end",
            "def f(a); -> { super(a) }; end",
            "def f(a); define_singleton_method(:f) { |a| super(a) }; end",
            "def f(a); class << self; def f(b); super(a); end; end; end",
            "def f(a); super { x }; end",
        ] {
            assert!(
                correction_offenses(&SuperArguments, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn inline_blocks_chains_nested_definitions_and_rebinding() {
        for (before, after) in [
            (
                "def f(a, b); super(a, b) { x }; end",
                "def f(a, b); super { x }; end",
            ),
            (
                "def f(a, &blk); super(a) { _1 }; end",
                "def f(a, &blk); super { _1 }; end",
            ),
            (
                "def f(a); super(a).foo { x }; end",
                "def f(a); super.foo { x }; end",
            ),
            (
                "def f(a); def g(b:); super(b: b); end; super(a); end",
                "def f(a); def g(b:); super; end; super; end",
            ),
            (
                "def f(a); a = 2; super(a); end",
                "def f(a); a = 2; super; end",
            ),
            (
                "def f(options, &blk); options[:key] ||= default; super(options, &blk); end",
                "def f(options, &blk); options[:key] ||= default; super; end",
            ),
            (
                "def f(&blk); tap { |blk| blk = nil }; super(&blk); end",
                "def f(&blk); tap { |blk| blk = nil }; super; end",
            ),
            (
                "def f(&blk); ->(blk) { blk = nil }; super(&blk); end",
                "def f(&blk); ->(blk) { blk = nil }; super; end",
            ),
            (
                "def f(&blk); def g; blk = nil; end; super(&blk); end",
                "def f(&blk); def g; blk = nil; end; super; end",
            ),
        ] {
            assert_correction(&SuperArguments, before, after);
        }
        let found = correction_offenses(&SuperArguments, "def f(a, &blk); super(a) { x }; end");
        assert_eq!(
            found[0].message,
            "Call `super` without arguments and parentheses when all positional and keyword arguments are forwarded."
        );
        for assignment in [
            "blk = proc {}",
            "blk ||= proc {}",
            "blk &&= proc {}",
            "blk += other",
            "blk, x = values",
            "[1].each { blk = nil }",
            "-> { blk = nil }",
            "case value; in [blk]; end",
            "if foo; blk = nil if bar; end",
        ] {
            let source = format!("def f(&blk); {assignment}; super(&blk); end");
            assert!(
                correction_offenses(&SuperArguments, &source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn expression_continuations_withhold_edits_not_diagnostics() {
        for suffix in [
            " +1", " -1", " [0]", "[0]", " *items", " **opts", " &mask", " /2", " %2", " ?1:2",
        ] {
            let source = format!("def f(a); super(a){suffix}; end");
            let found = correction_offenses(&SuperArguments, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 11));
            assert!(found[0].correction.is_none(), "{source}");
            assert_correction(&SuperArguments, &source, &source);
        }
        for (before, after) in [
            ("def f(a); (super(a)) +1; end", "def f(a); (super) +1; end"),
            (
                "def f(a); super(a).first; end",
                "def f(a); super.first; end",
            ),
        ] {
            assert_correction(&SuperArguments, before, after);
        }
    }
    #[test]
    fn lexical_surroundings_withhold_exact_bytes_in_main_ast_contexts() {
        for receiver in ["", "self."] {
            for newline in ["\n", "\r\n"] {
                for expression in [
                    "super(a)and true",
                    "super(a)if true",
                    "super(a)unless false",
                    "super(a)rescue 1",
                    "super(a)or 1",
                    "super(a) \\\n +1",
                    "super(a) <<value",
                    "super(a)do;end",
                ] {
                    let source = format!(
                        "class C < P\n def {receiver}f(a)\n  value=1\n  {expression}\n end\n def {receiver}safe(a); super(a); end\nend\n"
                    ).replace('\n', newline);
                    let found = correction_offenses(&SuperArguments, &source);
                    assert_eq!(found.len(), 2, "{source}");
                    assert_eq!((found[0].line, found[0].col), (4, 3));
                    assert_eq!(
                        found[0].message,
                        "Call `super` without arguments and parentheses when the signature is identical."
                    );
                    assert!(found[0].correction.is_none(), "{source}");
                    assert!(found[1].correction.is_some(), "{source}");
                    assert_correction(
                        &SuperArguments,
                        &source,
                        &source.replace("safe(a); super(a)", "safe(a); super"),
                    );
                }
            }
        }
    }
    #[test]
    fn delimited_main_ast_contexts_and_inline_block_ownership_remain_correctable() {
        for receiver in ["", "self."] {
            for newline in ["\n", "\r\n"] {
                for (before, after) in [
                    ("(super(a)) +1", "(super) +1"),
                    ("[super(a)]", "[super]"),
                    ("identity(super(a))", "identity(super)"),
                    ("super(a).next", "super.next"),
                    ("super(a) if true", "super if true"),
                    ("super(a) unless false", "super unless false"),
                    ("super(a) rescue 1", "super rescue 1"),
                    ("super(a) and true", "super and true"),
                    ("super(a) or 1", "super or 1"),
                    ("super(a) { :inline }and true", "super { :inline }and true"),
                    ("super(a){ :inline } <<value", "super{ :inline } <<value"),
                    ("super(a) do; :inline; end", "super do; :inline; end"),
                ] {
                    let source = format!("def {receiver}f(a)\n value=1\n {before}\nend\n")
                        .replace('\n', newline);
                    let expected = source.replace(before, after);
                    assert_correction(&SuperArguments, &source, &expected);
                }
            }
        }
        // This `do` belongs to the loop, not a literal block on super. Do not
        // infer block ownership from the following token alone.
        for keyword in ["while", "until"] {
            let source = format!("def f(a); {keyword} super(a) do; break; end; end");
            let found = correction_offenses(&SuperArguments, &source);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, 17));
            assert!(found[0].correction.is_none());
            assert_correction(&SuperArguments, &source, &source);
        }
    }
    #[test]
    fn binding_and_eval_withhold_block_edits_not_diagnostics() {
        for observer in [
            "binding.local_variable_set(:blk, replacement)",
            "eval('blk = replacement')",
            "send(:binding)",
            "alias snapshot binding; snapshot",
            "TracePoint.new(:line) {}",
        ] {
            let source = format!("def f(&blk); {observer}; super(&blk); end");
            let found = correction_offenses(&SuperArguments, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
            assert_correction(&SuperArguments, &source, &source);
        }
        for (before, after) in [
            (
                "def f(a); binding; super(a); end",
                "def f(a); binding; super; end",
            ),
            (
                "def f(&blk); binding; super() { :inline }; end",
                "def f(&blk); binding; super { :inline }; end",
            ),
            (
                "def f(&blk); blk.call; super(&blk); end",
                "def f(&blk); blk.call; super; end",
            ),
        ] {
            assert_correction(&SuperArguments, before, after);
        }
    }
    #[test]
    fn data_byte_position_observations_withhold_length_changing_edits() {
        for observer in [
            "p DATA.pos",
            "p Object.const_get(:DATA).pos",
            "p DATA.method(:pos).call",
        ] {
            let source = format!("def f(a); super(a); end\n{observer}\n__END__\npayload\n");
            let found = correction_offenses(&SuperArguments, &source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn preserves_multiline_forwarding_comments_and_inline_blocks() {
        for source in [
            "def f(a); super(\n a\n); end",
            "def f(a); super(a # keep\n); end",
        ] {
            let found = correction_offenses(&SuperArguments, source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
        }
        assert_correction(
            &SuperArguments,
            "def f(a); super(a) do\n puts __LINE__\nend; end",
            "def f(a); super do\n puts __LINE__\nend; end",
        );
    }
    #[test]
    fn pinned_forwarding_channels() {
        for (params, args) in [
            ("", ""),
            ("a", "a"),
            ("a, b", "a, b"),
            ("a, b, c = 1", "a, b, c"),
            ("a, b:", "a, b: b"),
            ("a, b: 1", "a, b: b"),
            ("a = 1, b: 2", "a, b: b"),
            ("&blk", "&blk"),
            ("*args", "*args"),
            ("**kwargs", "**kwargs"),
            ("*args, **kwargs", "*args, **kwargs"),
            ("*args, **kwargs, &blk", "*args, **kwargs, &blk"),
            ("a:, **kwargs", "a: a, **kwargs"),
            ("...", "..."),
            ("a, ...", "a, ..."),
            ("a:", "a:"),
            ("&", "&"),
            ("*", "*"),
            ("**", "**"),
        ] {
            for receiver in ["", "self."] {
                let before = format!("def {receiver}method({params})\n  super({args})\nend\n");
                let after = format!("def {receiver}method({params})\n  super\nend\n");
                let found = correction_offenses(&SuperArguments, &before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                assert_eq!(
                    found[0].message,
                    "Call `super` without arguments and parentheses when the signature is identical."
                );
                // Both singleton/instance pinned `a:, **kwargs` examples are
                // individually withheld: zsuper inserts rest keys first.
                if params == "a:, **kwargs" {
                    assert!(found[0].correction.is_none());
                    assert_correction(&SuperArguments, &before, &before);
                } else {
                    assert_correction(&SuperArguments, &before, &after);
                }
            }
        }
    }
}
