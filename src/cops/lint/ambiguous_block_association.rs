//! `Lint/AmbiguousBlockAssociation` — flags blocks that bind to a different
//! call than they appear to, in calls without parentheses:
//!
//! - `some_method a { |x| x }`: the braces bind to `a` (or to a `super`
//!   in that place), not `some_method`. Reported on the outer call, unless
//!   the block is a lambda or proc (`lambda`, `proc`, `Proc.new`), the inner
//!   call has arguments, or the outer call is an operator (`==`, `[]`) or a
//!   setter (`obj.attr = …`).
//! - `render json: data.map do |x| … end`: the `do` block binds to `render`,
//!   so `map` gets none. Reported on the inner call when it is a common
//!   block-taking enumerable method, takes no arguments, and ends the outer
//!   call's arguments.
//!
//! Matches RuboCop's semantics with its default (empty) `AllowedMethods` and
//! `AllowedPatterns`.

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{is_global_const, is_operator_method, source_end},
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};

/// RuboCop's `wrap_in_parentheses`.
fn wrap_in_parentheses(
    call: &ruby_prism::CallNode<'_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let arguments = crate::cops::ast::call_arguments(call);
    let selector_end = call.message_loc()?.end_offset();
    let first = arguments.first()?.location().start_offset();
    let last = arguments.last()?.location().end_offset();
    super::super::style::correction::combine(
        file,
        vec![
            crate::offense::Correction {
                range: selector_end..first,
                replacement: "(".into(),
            },
            crate::offense::Correction {
                range: last..last,
                replacement: ")".into(),
            },
        ],
    )
}

/// The `Lint/AmbiguousBlockAssociation` cop.
pub struct AmbiguousBlockAssociation;

/// Methods that are pointless without the block a `do … end` steals.
const BLOCK_METHODS: [&[u8]; 20] = [
    b"map",
    b"collect",
    b"flat_map",
    b"collect_concat",
    b"select",
    b"filter",
    b"find_all",
    b"reject",
    b"find",
    b"detect",
    b"each",
    b"each_with_object",
    b"each_with_index",
    b"reduce",
    b"inject",
    b"sort_by",
    b"min_by",
    b"max_by",
    b"group_by",
    b"filter_map",
];

impl Cop for AmbiguousBlockAssociation {
    fn name(&self) -> &'static str {
        "Lint/AmbiguousBlockAssociation"
    }

    fn safe_fixes(&self) -> bool {
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
        let Some(call) = node.as_call_node() else {
            return;
        };
        if let Some(param) = brace_block_argument(&call) {
            let (line, col) = file.line_col(node.location().start_offset());
            out.push(Offense {
                correction: cx
                    .corrections_requested()
                    .then(|| wrap_in_parentheses(&call, file))
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Parenthesize the param `{}` to make sure that the block will be \
                     associated with the `{}` method call.",
                    reference_source(param.source, file),
                    reference_source(param.method, file),
                ),
                line,
                end_line: file.line_of(source_end(node).saturating_sub(1)),
                col,
            });
        }
        if let Some(inner) = blockless_enumerable_argument(&call) {
            let (line, col) = file.line_col(inner.location().start_offset());
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "`{}` is called without a block because the `do` block binds to `{}`. \
                     Use braces or extract to a variable.",
                    reference_source(inner.name().as_slice(), file),
                    reference_source(call.name().as_slice(), file),
                ),
                line,
                end_line: file.line_of(source_end(&inner.as_node()).saturating_sub(1)),
                col,
            });
        }
    }
}

/// Source-buffer text quoted by the two existing call-message cops. Binary
/// bytes are escaped explicitly, not silently replaced in UTF8 diagnostics.
pub(super) fn reference_source(bytes: &[u8], file: &SourceFile) -> String {
    file.match_pattern_source(bytes)
        .utf8_chunks()
        .map(|chunk| {
            let mut text = chunk.valid().to_owned();
            for b in chunk.invalid() {
                text.push_str(&format!("\\x{b:02X}"));
            }
            text
        })
        .collect::<String>()
        .replace("\r\n", "\n")
}

/// An argument that carries a literal block.
struct BlockParam<'pr> {
    /// The argument's source, block included (`a { … }`).
    source: &'pr [u8],
    /// The source of the call the block belongs to (`a`).
    method: &'pr [u8],
}

/// The last argument of an unparenthesized `call` when it is a call (or
/// `super`) with a literal block and no arguments of its own (`a { … }`),
/// and the outer call is not one that RuboCop accepts.
fn brace_block_argument<'pr>(call: &CallNode<'pr>) -> Option<BlockParam<'pr>> {
    if call.opening_loc().is_some() {
        return None;
    }
    // A block-pass is RuboCop's last argument when present.
    if call
        .block()
        .is_some_and(|block| block.as_block_argument_node().is_some())
    {
        return None;
    }
    if call.equal_loc().is_some() || is_operator_method(call.name().as_slice()) {
        return None;
    }
    let last = call.arguments()?.arguments().iter().last()?;
    let block = match &last {
        Node::CallNode { .. } => {
            let inner = last.as_call_node().unwrap();
            if inner.arguments().is_some() || is_lambda_or_proc(&inner) {
                return None;
            }
            inner.block()?.as_block_node()?
        }
        Node::SuperNode { .. } => {
            let inner = last.as_super_node().unwrap();
            if inner.arguments().is_some() {
                return None;
            }
            inner.block()?.as_block_node()?
        }
        Node::ForwardingSuperNode { .. } => last.as_forwarding_super_node().unwrap().block()?,
        _ => return None,
    };
    let location = last.location();
    let source = location.as_slice();
    let before_block = &source[..block.location().start_offset() - location.start_offset()];
    Some(BlockParam {
        source,
        method: trim_trailing_gap(before_block),
    })
}

/// `source` without trailing whitespace and line continuations (`\`
/// followed by a newline), which can separate a call from its block.
fn trim_trailing_gap(mut source: &[u8]) -> &[u8] {
    loop {
        let trimmed = source.trim_ascii_end();
        let gap = &source[trimmed.len()..];
        match trimmed.strip_suffix(b"\\") {
            Some(rest) if gap.starts_with(b"\n") || gap.starts_with(b"\r\n") => source = rest,
            _ => return trimmed,
        }
    }
}

/// RuboCop's `lambda_or_proc?` for a call carrying a literal block:
/// `lambda { }` (any receiver), `proc { }`, `Proc.new { }`.
fn is_lambda_or_proc(call: &CallNode<'_>) -> bool {
    match call.name().as_slice() {
        b"lambda" => true,
        b"proc" => call.receiver().is_none() && ordinary_block(call),
        b"new" => {
            ordinary_block(call)
                && call
                    .receiver()
                    .is_some_and(|receiver| is_global_const(&receiver, &["Proc"]))
        }
        _ => false,
    }
}

fn ordinary_block(call: &CallNode<'_>) -> bool {
    call.block()
        .and_then(|b| b.as_block_node())
        .is_some_and(|b| {
            !matches!(
                b.parameters(),
                Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
            )
        })
}

/// For an unparenthesized call (not `&.`) with a `do … end` block, the
/// block-taking enumerable call, without arguments or block, that ends its
/// arguments (`data.map` in `render json: data.map do … end`).
fn blockless_enumerable_argument<'pr>(call: &CallNode<'pr>) -> Option<CallNode<'pr>> {
    let block = call.block()?.as_block_node()?;
    if block.opening_loc().as_slice() != b"do"
        || call.is_safe_navigation()
        || call.opening_loc().is_some()
    {
        return None;
    }
    let arguments = call.arguments()?;
    let last = arguments.arguments().iter().last()?;
    let mut finder = EnumerableFinder {
        end: arguments.location().end_offset(),
        found: None,
    };
    walk(&mut finder, &last);
    finder.found
}

/// Finds, in pre-order, the first enumerable call that ends at `end`.
struct EnumerableFinder<'pr> {
    end: usize,
    found: Option<CallNode<'pr>>,
}

impl<'pr> NodeVisitor<'pr> for EnumerableFinder<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if self.found.is_some() {
            return;
        }
        let Some(node) = node.as_call_node() else {
            return;
        };
        if node.as_node().location().end_offset() == self.end
            && node.arguments().is_none()
            && node.block().is_none()
            && BLOCK_METHODS.contains(&node.name().as_slice())
        {
            self.found = node.as_node().as_call_node();
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/ambiguous_block_association_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! empty `AllowedMethods` and `AllowedPatterns`.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/AmbiguousBlockAssociation")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    fn single(source: &str) -> (u32, u32, String) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "one offense for {source:?}: {found:?}");
        (found[0].line, found[0].col, found[0].message.clone())
    }

    fn param_message(param: &str, method: &str) -> String {
        format!(
            "Parenthesize the param `{param}` to make sure that the block will be associated \
             with the `{method}` method call."
        )
    }

    fn do_end_message(inner: &str, outer: &str) -> String {
        format!(
            "`{inner}` is called without a block because the `do` block binds to `{outer}`. \
             Use braces or extract to a variable."
        )
    }

    #[test]
    fn accepts_unambiguous_blocks() {
        for source in [
            "foo == bar { baz a }",
            "foo ->(a) { bar a }",
            "some_method(a) { |el| puts el }",
            "some_method(a) { puts _1 }",
            "some_method(a) do;puts a;end",
            "some_method a do;puts \"dev\";end",
            "some_method a do |e|;puts e;end",
            "Foo.bar(a) { |el| puts el }",
            "env ENV.fetch(\"ENV\") { \"dev\" }",
            "env(ENV.fetch(\"ENV\") { \"dev\" })",
            "{ f: \"b\"}.fetch(:a) do |e|;puts e;end",
            "Hash[some_method(a) { |el| el }]",
            "foo = lambda do |diagnostic|;end",
            "Proc.new { puts \"proc\" }",
            "expect { order.save }.to(change { orders.size })",
            "scope :active, -> { where(status: \"active\") }",
            "scope :active, proc { where(status: \"active\") }",
            "scope :active, Proc.new { where(status: \"active\") }",
            "assert_equal posts.find { |p| p.title == \"Foo\" }, results.first",
            "assert_equal(posts.find { |p| p.title == \"Foo\" }, results.first)",
            "foo[bar { |x| x }]",
            "obj.attr = bar { |x| x }",
            "assert_equal(results.first, posts.find { |p| p.title == \"Foo\" })",
            "allow(cop).to receive(:on_int) { raise RuntimeError }",
            "allow(cop).to(receive(:on_int) { raise RuntimeError })",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn proc_exemption_is_only_for_ordinary_blocks() {
        for receiver in ["proc", "Proc.new"] {
            assert!(offenses(&format!("foo {receiver} {{ x }}")).is_empty());
            for body in ["_1", "it"] {
                let param = format!("{receiver} {{ {body} }}");
                assert_eq!(
                    single(&format!("foo {param}")),
                    (1, 1, param_message(&param, receiver))
                );
            }
        }
    }

    #[test]
    fn flags_brace_block_param_without_receiver() {
        assert_eq!(
            single("some_method a { |el| puts el }\n"),
            (1, 1, param_message("a { |el| puts el }", "a"))
        );
        assert_eq!(
            single("some_method a { puts _1 }\n"),
            (1, 1, param_message("a { puts _1 }", "a"))
        );
        assert_eq!(
            single("some_method a { puts it }\n"),
            (1, 1, param_message("a { puts it }", "a"))
        );
    }

    #[test]
    fn flags_brace_block_param_with_receiver() {
        assert_eq!(
            single("Foo.some_method a { |el| puts el }\n"),
            (1, 1, param_message("a { |el| puts el }", "a"))
        );
        assert_eq!(
            single("Foo&.some_method a { |el| puts el }\n"),
            (1, 1, param_message("a { |el| puts el }", "a"))
        );
    }

    #[test]
    fn directives_only_overlap_the_outer_call_not_its_literal_block() {
        let source = "foo bar { baz } do\n  work\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
        for source in [
            "foo bar { baz } do\n  work\nend # rubocop:disable Lint/AmbiguousBlockAssociation\n",
            "foo bar { baz } do\n  work # rubocop:disable Lint/AmbiguousBlockAssociation\nend\n",
        ] {
            assert_eq!(positions(source), [(1, 1)], "{source:?}");
        }
        // The argument's own block is still part of the outer send.
        assert!(offenses(
            "foo bar {\n  baz\n} do # rubocop:disable Lint/AmbiguousBlockAssociation\n  work\nend\n"
        ).is_empty());
    }

    #[test]
    fn flags_rspec_change_matcher() {
        assert_eq!(
            single("expect { order.expire }.to change { order.events }\n"),
            (1, 1, param_message("change { order.events }", "change"))
        );
        assert_eq!(
            single("expect { order.expire }.to update { order.events }\n"),
            (1, 1, param_message("update { order.events }", "update"))
        );
    }

    #[test]
    fn flags_param_inside_index_and_assignment() {
        assert_eq!(
            single("Hash[some_method a { |el| el }]\n"),
            (1, 6, param_message("a { |el| el }", "a"))
        );
        assert_eq!(
            single("foo = some_method a { |el| puts el }\n"),
            (1, 7, param_message("a { |el| puts el }", "a"))
        );
    }

    #[test]
    fn flags_do_end_block_stolen_from_enumerable() {
        assert_eq!(
            single("render json: queries.map do |q|\n  q.to_h\nend\n"),
            (1, 14, do_end_message("map", "render"))
        );
        assert_eq!(
            single("foo bar.select do |x|\n  x\nend\n"),
            (1, 5, do_end_message("select", "foo"))
        );
        assert_eq!(
            single("foo bar.each do |x|\n  x\nend\n"),
            (1, 5, do_end_message("each", "foo"))
        );
        assert_eq!(
            single("foo bar.filter_map do |x|\n  x\nend\n"),
            (1, 5, do_end_message("filter_map", "foo"))
        );
        assert_eq!(
            single("foo bar&.map do |x|\n  x\nend\n"),
            (1, 5, do_end_message("map", "foo"))
        );
    }

    #[test]
    fn accepts_do_end_blocks_bound_as_intended() {
        for source in [
            "bar.map do |x|\n  x\nend\n",
            "render(json: queries.map) do |q|\n  q\nend\n",
            "render json: queries.map(&:to_s) do |q|\n  q\nend\n",
            "render json: queries.map { |q| q } do |q|\n  q\nend\n",
            "foo bar.something do |x|\n  x\nend\n",
            "foo bar.map.to_a do |x|\n  x\nend\n",
            "foo bar(baz.map) do |x|\n  x\nend\n",
            "foo(bar.map { |x| x })\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    // The examples below were checked against RuboCop 1.92.

    #[test]
    fn reports_call_source_without_block_in_message() {
        assert_eq!(
            single("foo x.a() { 1 }\n"),
            (1, 1, param_message("x.a() { 1 }", "x.a()"))
        );
        assert_eq!(
            single("foo a {}, b {}\n"),
            (1, 1, param_message("b {}", "b"))
        );
        assert_eq!(
            single("foo a.b { 1 }\n"),
            (1, 1, param_message("a.b { 1 }", "a.b"))
        );
        // Only a receiverless `proc` is a proc.
        assert_eq!(
            single("foo x.proc { 1 }\n"),
            (1, 1, param_message("x.proc { 1 }", "x.proc"))
        );
    }

    #[test]
    fn strips_line_continuation_from_method_in_message() {
        assert_eq!(
            single("a.foo b \\\n{ 1 }\n"),
            (1, 1, param_message("b \\\n{ 1 }", "b"))
        );
        assert_eq!(
            single("a.foo b \\\r\n  \\\n{ 1 }\n"),
            (1, 1, param_message("b \\\n  \\\n{ 1 }", "b"))
        );
    }

    #[test]
    fn flags_brace_block_on_argumentless_super() {
        assert_eq!(
            single("def m\n  foo super { 1 }\nend\n"),
            (2, 3, param_message("super { 1 }", "super"))
        );
        assert_eq!(
            single("def m\n  foo super() { 1 }\nend\n"),
            (2, 3, param_message("super() { 1 }", "super()"))
        );
        assert!(offenses("def m\n  foo super(a) { 1 }\nend\n").is_empty());
        assert!(offenses("def m\n  foo bar, super do 1 end\nend\n").is_empty());
    }

    #[test]
    fn flags_more_do_end_shapes() {
        assert_eq!(positions("foo x = bar.map do\nend\n"), vec![(1, 9)]);
        assert_eq!(positions("foo a, bar.map() do\nend\n"), vec![(1, 8)]);
        assert_eq!(positions("x.foo -bar.map do\nend\n"), vec![(1, 8)]);
        assert_eq!(positions("foo bar.map do end.baz\n"), vec![(1, 5)]);
    }

    #[test]
    fn accepts_more_lookalikes() {
        for source in [
            "foo a { 1 }, &b\n",
            "foo a: b { 1 }\n",
            "foo *a { 1 }\n",
            "foo obj.lambda { 1 }\n",
            "foo ::Proc.new { 1 }\n",
            "foo[1] = bar { 1 }\n",
            "foo + bar { 1 }\n",
            "!foo { 1 }\n",
            "super a { 1 }\n",
            "x&.foo bar.map do\nend\n",
            "super bar.map do\nend\n",
            "foo bar.map, baz do\nend\n",
            "foo bar.inject(0) do\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
