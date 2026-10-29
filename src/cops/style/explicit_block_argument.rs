//! `Style/ExplicitBlockArgument`: exact wrapper detection with conservative forwarding proofs.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ExplicitBlockArgument;
const MSG: &str =
    "Consider using explicit block argument in the surrounding method's signature over `yield`.";
impl Cop for ExplicitBlockArgument {
    fn name(&self) -> &'static str {
        "Style/ExplicitBlockArgument"
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
        let block = node.as_block_node();
        let (body, parameters, call) = if let Some(block) = &block {
            let Some(call) = cx.parent().filter(|n| {
                matches!(
                    n,
                    Node::CallNode { .. }
                        | Node::SuperNode { .. }
                        | Node::ForwardingSuperNode { .. }
                )
            }) else {
                return;
            };
            (block.body(), block.parameters(), Some(call))
        } else if let Some(lambda) = node.as_lambda_node() {
            // Prism models arrow lambdas separately; upstream's block matcher
            // includes them. Their Proc semantics are not a forwarding proof.
            (lambda.body(), lambda.parameters(), None)
        } else {
            return;
        };
        let Some(body) = body.and_then(|b| b.as_statements_node()) else {
            return;
        };
        if body.body().len() != 1 {
            return;
        }
        let Some(yield_node) = body.body().first().and_then(|n| n.as_yield_node()) else {
            return;
        };
        let mut params = Vec::new();
        if let Some(parameters) = parameters {
            let Some(parameters) = parameters.as_block_parameters_node() else {
                return;
            };
            if let Some(p) = parameters.parameters() {
                params.extend(p.requireds().iter());
                params.extend(p.optionals().iter());
                params.extend(p.rest());
                params.extend(p.posts().iter());
                params.extend(p.keywords().iter());
                params.extend(p.keyword_rest());
                params.extend(p.block().map(|b| b.as_node()));
            }
            params.extend(parameters.locals().iter());
        }
        let args: Vec<_> = yield_node
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if args.len() != params.len()
            || !args.iter().zip(&params).all(|(arg, param)| {
                let name = param
                    .as_required_parameter_node()
                    .map(|p| p.name())
                    .or_else(|| param.as_optional_parameter_node().map(|p| p.name()))
                    .or_else(|| param.as_rest_parameter_node().and_then(|p| p.name()))
                    .or_else(|| {
                        param
                            .as_keyword_rest_parameter_node()
                            .and_then(|p| p.name())
                    })
                    .or_else(|| param.as_required_keyword_parameter_node().map(|p| p.name()))
                    .or_else(|| param.as_optional_keyword_parameter_node().map(|p| p.name()))
                    .or_else(|| param.as_block_parameter_node().and_then(|p| p.name()))
                    .or_else(|| param.as_block_local_variable_node().map(|p| p.name()));
                name.is_some_and(|name| {
                    arg.as_local_variable_read_node()
                        .is_some_and(|a| a.name().as_slice() == name.as_slice())
                })
            })
        {
            return;
        }
        let Some(def) = cx.ancestors().find_map(|n| n.as_def_node()) else {
            return;
        };
        if def
            .parameters()
            .and_then(|p| p.keyword_rest())
            .is_some_and(|p| p.as_forwarding_parameter_node().is_some())
        {
            return;
        }
        let start = call.unwrap_or(node).location().start_offset();
        let end = node.location().end_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: MSG.into(),
            correction: cx
                .corrections_requested()
                .then(|| {
                    call.zip(block.as_ref())
                        .and_then(|(call, block)| rewrite(call, block, &def, cx, file))
                })
                .flatten(),
        });
    }
}
// Only forward through core Array#each when its single positional yield channel
// is identical and the original method's block-absence behavior is guarded.
// Arbitrary receivers may inspect Proc identity, store it, call with different
// arity/keywords or return an Enumerator. A parser check cannot prove this safe.
fn rewrite(
    call: &Node<'_>,
    block: &ruby_prism::BlockNode<'_>,
    def: &ruby_prism::DefNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    // Do not assume block_given? resolves to Kernel for a singleton or an
    // inherited class/module receiver. Keep those offenses diagnostic-only.
    if def.receiver().is_some()
        || cx.ancestors().filter(|n| n.as_def_node().is_some()).count() != 1
        || cx.ancestors().any(|n| {
            matches!(
                n,
                Node::ClassNode { .. } | Node::ModuleNode { .. } | Node::SingletonClassNode { .. }
            )
        })
    {
        return None;
    }
    let call_node = call.as_call_node()?;
    if call_node.name().as_slice() != b"each"
        || call_node.arguments().is_some()
        || call_node.is_safe_navigation()
    {
        return None;
    }
    let array = call_node.receiver()?.as_array_node()?;
    if !array.elements().iter().all(|n| inert_scalar(&n)) {
        return None;
    }
    let bp = block.parameters()?.as_block_parameters_node()?;
    let parameters = bp.parameters()?;
    let required = parameters.requireds();
    if required.len() != 1
        || !bp.locals().is_empty()
        || !parameters.optionals().is_empty()
        || parameters.rest().is_some()
        || !parameters.posts().is_empty()
        || !parameters.keywords().is_empty()
        || parameters.keyword_rest().is_some()
        || parameters.block().is_some()
    {
        return None;
    }
    let parameter = required.first()?.as_required_parameter_node()?;
    let body = def.body()?.as_statements_node()?;
    if body.body().len() != 1 {
        return None;
    }
    let guard = body.body().first()?.as_if_node()?;
    if guard.subsequent().is_some() {
        return None;
    }
    let predicate = guard.predicate().as_call_node()?;
    if predicate.name().as_slice() != b"block_given?"
        || predicate.receiver().is_some()
        || predicate.arguments().is_some()
        || predicate.block().is_some()
    {
        return None;
    }
    let statements = guard.statements()?;
    if statements.body().len() != 1
        || statements.body().first()?.location().start_offset() != call.location().start_offset()
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.argument_rewrite_facts(root);
    if facts.unsafe_core
        || file.class_method_bindings(root).observes_sources
        || file.class_method_bindings(root).observes_data
    {
        return None;
    }
    let name = facts.block_names.get(&def.location().start_offset())?;
    if parameter.name().as_slice() == name.as_bytes() {
        return None;
    }
    let start = call_node.message_loc()?.end_offset();
    let end = block.location().end_offset();
    if file.bytes[start..end].contains(&b'\n')
        || super::correction::contains_comment(file, start..end)
    {
        return None;
    }
    let mut edits = vec![Correction {
        range: start..end,
        replacement: format!("(&{name})"),
    }];
    if def.parameters().and_then(|p| p.block()).is_none() {
        let start = def
            .lparen_loc()
            .map(|l| l.start_offset())
            .unwrap_or(def.name_loc().end_offset());
        let end = def.rparen_loc().map(|l| l.end_offset()).unwrap_or(start);
        if file.bytes[start..end]
            .iter()
            .any(|b| matches!(b, b'\n' | b'\r' | b'#' | b'\\'))
        {
            return None;
        }
        edits.push(Correction {
            range: start..end,
            replacement: format!("(&{name})"),
        });
    }
    super::correction::combine(file, edits)
}

pub(super) fn inert_scalar(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::IntegerNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
            | Node::SymbolNode { .. }
    )
}

/// One lazy main-AST/core/observer proof shared by both argument-rewriting cops.
/// ScopeAnalysis owns local binding evidence; no per-candidate body scans.
#[derive(Default)]
pub(crate) struct Facts {
    pub(super) unsafe_core: bool,
    block_names: std::collections::HashMap<usize, String>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        if let Some(program) = root.as_program_node() {
            let analysis = crate::cops::variables::ScopeAnalysis::of(&program);
            for id in analysis.scopes_in_exit_order() {
                let scope = analysis.scope(id);
                let Some(def) = scope.node.as_def_node() else {
                    continue;
                };
                let mut name = "block".to_owned();
                let mut existing = false;
                if let Some(p) = def.parameters() {
                    if !p.requireds().is_empty()
                        || !p.optionals().is_empty()
                        || p.rest().is_some()
                        || !p.posts().is_empty()
                        || !p.keywords().is_empty()
                        || p.keyword_rest().is_some()
                    {
                        continue;
                    }
                    if let Some(block) = p.block() {
                        let Some(n) = block.name() else { continue };
                        let Ok(n) = std::str::from_utf8(n.as_slice()) else {
                            continue;
                        };
                        name = n.to_owned();
                        existing = true;
                    }
                }
                let taken = scope.variables().map(|id| analysis.variable(id)).any(|v| {
                    v.name == name.as_bytes()
                        && (!existing || !v.assignments.is_empty() || v.has_pattern_bindings)
                });
                if !taken {
                    facts
                        .block_names
                        .insert(def.location().start_offset(), name);
                }
            }
        }
        crate::cops::walk::walk(&mut facts, root);
        facts
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Facts {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        let dangerous = |name: &[u8]| {
            super::arguments_forwarding::observer(name)
                || matches!(
                    name,
                    b"define_method"
                        | b"define_singleton_method"
                        // Attribute macros generate methods too, including
                        // from strings or computed names. Do not resolve targets.
                        | b"attr"
                        | b"attr_reader"
                        | b"attr_writer"
                        | b"attr_accessor"
                        | b"prepend"
                        | b"include"
                        | b"extend"
                        | b"refine"
                        | b"using"
                        | b"local_variable_set"
                        // Method visibility/export and removal all invalidate
                        // ordinary core dispatch. Do not resolve their targets:
                        // strings, escaped/computed names, splats, receivers and
                        // zero-argument default-visibility forms are barriers too.
                        | b"private"
                        | b"protected"
                        | b"public"
                        | b"private_class_method"
                        | b"public_class_method"
                        | b"module_function"
                        | b"ruby2_keywords"
                        | b"undef_method"
                        | b"remove_method"
                )
        };
        let core = |name: &[u8]| {
            dangerous(name)
                || matches!(
                    name,
                    b"merge" | b"each" | b"block_given?" | b"to_hash" | b"initialize_copy"
                )
        };
        self.unsafe_core |= node
            .as_call_node()
            .is_some_and(|c| dangerous(c.name().as_slice()))
            || node
                .as_def_node()
                .is_some_and(|d| core(d.name().as_slice()))
            || node.as_symbol_node().is_some_and(|s| core(s.unescaped()))
            || matches!(
                node,
                Node::AliasMethodNode { .. }
                    // `undef` permits interpolated symbols: a static symbol
                    // check alone cannot prove that core dispatch survives.
                    | Node::UndefNode { .. }
                    | Node::PreExecutionNode { .. }
            );
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/explicit_block_argument_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/ExplicitBlockArgument")
            .collect()
    }
    #[test]
    fn every_pinned_correction_pair_is_individually_accounted_for() {
        let pairs: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/argument_wrappers_corrections.json"
        ))
        .unwrap();
        let pairs: Vec<_> = pairs
            .iter()
            .filter(|p| p["cop"] == "Style/ExplicitBlockArgument")
            .collect();
        assert_eq!(pairs.len(), 24);
        for p in pairs {
            let before = p["before"].as_str().unwrap();
            assert!(!p["withheld_reason"].as_str().unwrap().is_empty());
            assert!(!offenses(before).is_empty(), "spec line {}", p["line"]);
            crate::cops::testing::assert_correction(&ExplicitBlockArgument, before, before);
            assert!(offenses(p["after"].as_str().unwrap()).is_empty());
        }
    }
    #[test]
    fn exact_yield_wrappers_and_locations() {
        for call in [
            "items.something(first_arg) { |i| yield i }",
            "items.something(first_arg) { |i,j| yield i,j }",
            "items.something(a, b,) { |i| yield i }",
            "array&.each do |row|\n    yield row\n  end",
            "callable.() { |row| yield row }",
            "array.each() { |row| yield row }",
            "foo[] { |row| yield row }",
            "foo.[] { |row| yield row }",
            "3.times { yield }",
            "super { yield }",
            "super() { yield }",
            "foo(x) { yield }",
        ] {
            let source = format!("def m\n  {call}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(found[0].message, MSG);
        }
    }
    #[test]
    fn arrow_lambda_identity_wrappers_report_without_corrections() {
        for wrapper in [
            "->(x) { yield x }",
            "->(x, y) do yield x, y end",
            "-> { yield }",
            "->(x = 1) { yield x }",
            "->(*x) { yield x }",
            "->(x:) { yield x }",
        ] {
            let source = format!("def f\n  {wrapper}\nend\n");
            let found = crate::cops::testing::correction_offenses(&ExplicitBlockArgument, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(found[0].end_line, 2);
            assert_eq!(found[0].message, MSG);
            assert!(found[0].correction.is_none(), "{source}");
        }
        for source in [
            "def f; ->(x, y) { yield y, x }; end",
            "def f; ->(x) { yield x, 1 }; end",
            "def f; ->(x) { work; yield x }; end",
            "def f; ->(*x) { yield *x }; end",
            "def f; ->((x, y)) { yield x, y }; end",
            "def f(...); ->(x) { yield x }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn distinct_parameter_channels_follow_the_pinned_name_matcher() {
        for parameter in ["i = 1", "*i", "i:", "i: 1", "**i", "&i"] {
            let source = format!("def f; foo {{ |{parameter}| yield i }}; end");
            assert_eq!(offenses(&source).len(), 1, "{source}");
            assert!(
                crate::cops::testing::correction_offenses(&ExplicitBlockArgument, &source)
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
        for source in [
            "def f; foo { |*i| yield *i }; end",
            "def f; foo { |i:| yield i: i }; end",
            "def f; foo { |(i,j)| yield i,j }; end",
            "def f; foo { |i; local| yield i }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn signatures_super_and_nested_defs_still_report() {
        for signature in [
            "m(&blk)",
            "self.m(x, y = 42, *args, **options)",
            "m(x,k:1,j:)",
            "m(x, **nil)",
            "m((a,b))",
            "m(*, **)",
            "m(scope, block = nil)",
            "[]",
        ] {
            let found = offenses(&format!("def {signature}\n  super {{ yield }}\nend"));
            assert_eq!(found.len(), 1, "{signature}");
        }
        let source = "def foo\n  def bar.baz\n    qux { |quux| yield quux }\n  end\nend\n";
        let found = offenses(source);
        assert_eq!((found[0].line, found[0].col), (3, 5));
    }
    #[test]
    fn guarded_core_iterator_canonical_edits_are_atomic() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "def f; if block_given?; [1,2].each { |x| yield x }; end; end",
                "def f(&block); if block_given?; [1,2].each(&block); end; end",
            ),
            (
                "def f(); if block_given?; [1].each() { |x| yield x }; end; end",
                "def f(&block); if block_given?; [1].each(&block); end; end",
            ),
            (
                "def f(&blk); if block_given?; [1].each { |x| yield x }; end; end",
                "def f(&blk); if block_given?; [1].each(&blk); end; end",
            ),
        ] {
            assert_correction(&ExplicitBlockArgument, before, after);
        }
    }
    #[test]
    fn guarded_iterator_with_extra_channels_must_not_be_forwarded() {
        use crate::cops::testing::correction_offenses;
        for (params, args) in [
            ("x, y=1", "x,y"),
            ("x, *rest", "x,rest"),
            ("x, key:", "x,key"),
            ("x, **keys", "x,keys"),
            ("x, &blk", "x,blk"),
        ] {
            let source =
                format!("def f; if block_given?; [1].each {{ |{params}| yield {args} }}; end; end");
            let found = correction_offenses(&ExplicitBlockArgument, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn unsafe_wrapper_contexts_remain_reported_but_unedited() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "class Child < External; def f; if block_given?; [1].each { |x| yield x }; end; end; end",
            "def object.f; if block_given?; [1].each { |x| yield x }; end; end",
            "def outer; def f; if block_given?; [1].each { |x| yield x }; end; end; end",
            "def f; [1].each { |x| yield x }; end",
            "def f(&blk); unknown { |x| yield x }; end",
            "def f; if block_given?; [1].map { |x| yield x }; end; end",
            "def f; if block_given?; [1].each { yield }; end; end",
            "def f; if block_given?; object.each { |x| yield x }; end; end",
            "def f(block); if block_given?; [1].each { |x| yield x }; end; end",
            "def f; if block_given?; [1].each { |block| yield block }; end; end",
            "def f(&blk); blk=nil; if block_given?; [1].each { |x| yield x }; end; end",
            "def f; if block_given?; super { yield }; end; end",
            "def f; if block_given?; [effect()].each { |x| yield x }; end; end",
            "p method(:f).parameters; def f; if block_given?; [1].each { |x| yield x }; end; end",
        ] {
            let found = correction_offenses(&ExplicitBlockArgument, source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
    #[test]
    fn visibility_and_export_barriers_ignore_target_spelling_and_call_context() {
        use crate::cops::{
            Cop,
            testing::{assert_correction, correction_offenses},
        };
        for operation in [
            "private",
            "protected",
            "public",
            "private_class_method",
            "public_class_method",
            "module_function",
            "ruby2_keywords",
        ] {
            for (cop, class, method, candidate, line, col) in [
                (
                    &super::super::keyword_arguments_merging::KeywordArgumentsMerging as &dyn Cop,
                    "Hash",
                    "merge",
                    "take(**{}.merge(a: 1))\n",
                    2,
                    8,
                ),
                (
                    &ExplicitBlockArgument as &dyn Cop,
                    "Array",
                    "each",
                    "def f\n  if block_given?; [1].each { |x| yield x }; end\nend\n",
                    3,
                    20,
                ),
            ] {
                for target in [
                    format!(":{method}"),
                    format!("\"{method}\""),
                    format!("\"\\x{:02x}{}\"", method.as_bytes()[0], &method[1..]),
                    format!(":\"\\x{:02x}{}\"", method.as_bytes()[0], &method[1..]),
                    format!("%q({method})"),
                    format!("(\"{method}\".reverse.reverse)"),
                    format!("*%w({method})"),
                    "target".into(),
                    "\"#{target}\"".into(),
                    ":\"#{target}\"".into(),
                    "*targets".into(),
                ] {
                    for prefix in [
                        format!("class {class}; {operation} {target}; end"),
                        format!("{class}.{operation}({target})"),
                        format!("{class}&.{operation} {target}"),
                        format!("class << {class}; {operation}({target}); end"),
                        format!("def later; {class}.{operation}({target}); end"),
                    ] {
                        let source = format!("{prefix}\n{candidate}");
                        assert_eq!(
                            ruby_prism::parse(source.as_bytes()).errors().count(),
                            0,
                            "{source}"
                        );
                        let found = correction_offenses(cop, &source);
                        assert_eq!(found.len(), 1, "{source}");
                        assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
                        assert!(found[0].correction.is_none(), "{source}");
                        assert_correction(cop, &source, &source);
                    }
                }
                // Selector acquisition, symbol-to-proc dispatch and unknown
                // block passes must not evade the same source-local proof.
                let escaped = format!("\\x{:02x}{}", operation.as_bytes()[0], &operation[1..]);
                for prefix in [
                    format!("class {class}; {operation}; end"),
                    format!("{class}.{operation}"),
                    format!("class {class}; alias hidden {operation}; hidden target; end"),
                    format!("targets.map(&:{operation})"),
                    format!("targets.map(&:\"{escaped}\")"),
                    format!("targets.map(&\"{escaped}\".to_sym)"),
                    "targets.map(&selector)".into(),
                    format!("{class}.method(\"{escaped}\").call(target)"),
                    format!("{class}.__send__(\"{escaped}\", target)"),
                    format!("def {operation}(*names); super; end"),
                ] {
                    let source = format!("{prefix}\n{candidate}");
                    let found = correction_offenses(cop, &source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert!(found[0].correction.is_none(), "{source}");
                    assert_correction(cop, &source, &source);
                }
            }
        }
    }
    fn assert_core_mutation_withholds_both_corrections(prefix: &str) {
        use crate::cops::testing::{assert_correction, correction_offenses};
        for (cop, candidate, line, col, message) in [
            (
                &super::super::keyword_arguments_merging::KeywordArgumentsMerging as &dyn Cop,
                "take(**{}.merge(a: 1))\n",
                2,
                8,
                "Provide additional arguments directly rather than using `merge`.",
            ),
            (
                &ExplicitBlockArgument as &dyn Cop,
                "def f\n  if block_given?; [1].each { |x| yield x }; end\nend\n",
                3,
                20,
                MSG,
            ),
        ] {
            let source = format!("{prefix}\n{candidate}");
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            let found = correction_offenses(cop, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
            assert_eq!(found[0].message, message);
            assert!(found[0].correction.is_none(), "{source}");
            assert_correction(cop, &source, &source);
        }
    }
    #[test]
    fn attribute_declarations_are_core_barriers_regardless_of_target_spelling() {
        for operation in ["attr_reader", "attr_accessor", "attr", "attr_writer"] {
            for target in [
                ":merge",
                "\"merge\"",
                "\"\\x6derge\"",
                "\"#{'mer'}ge\"",
                "target",
                "*targets",
            ] {
                for prefix in [
                    format!("class Hash; {operation} {target}; end"),
                    format!("Hash.{operation}({target})"),
                    format!("Array&.{operation}({target})"),
                ] {
                    assert_core_mutation_withholds_both_corrections(&prefix);
                }
            }
        }
    }
    #[test]
    fn undef_statements_are_core_barriers_including_computed_targets() {
        for prefix in [
            "class Hash; undef merge; end",
            "class Hash; undef :\"\\x6derge\"; end",
            "class Hash; undef :\"#{'mer'}ge\"; end",
            "class Array; undef :\"#{'ea'}ch\"; end",
            "class Hash; undef unrelated; end",
        ] {
            assert_core_mutation_withholds_both_corrections(prefix);
        }
    }
    #[test]
    fn no_offense_for_nonidentity_wrappers_or_forwarding() {
        for source in [
            "def m; items.something { |i,j| yield j,i }; end",
            "def m; yield i; end",
            "def m; items.something { |i| work; yield i }; end",
            "def m; items.something { |i,j,k| yield j,k }; end",
            "def m; items.something { |i,j| yield i }; end",
            "render('partial') { yield }",
            "def m(...); bar { |x| yield x }; end",
            "def m(a, ...); super { yield }; end",
            "def m; foo { yield _1 }; end",
            "def m; foo { yield it }; end",
            "def m; foo { |i| yield i, 1 }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
