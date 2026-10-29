//! `Style/Lambda`: fixed `line_count_dependent` syntax.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct Lambda;
impl Cop for Lambda {
    fn name(&self) -> &'static str {
        "Style/Lambda"
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
        let (selector, closing, literal) = if let Some(lambda) = node.as_lambda_node() {
            if in_pattern(node, cx) || (command_argument(cx) && implicit_rescue(lambda.body())) {
                return;
            }
            (lambda.operator_loc(), lambda.closing_loc(), true)
        } else if let Some(call) = node.as_call_node() {
            if !lambda_call(node) {
                return;
            }
            let block = call.block().and_then(|b| b.as_block_node()).unwrap();
            (call.message_loc().unwrap(), block.closing_loc(), false)
        } else {
            return;
        };
        let multiline =
            file.line_of(selector.start_offset()) != file.line_of(closing.end_offset() - 1);
        if multiline != literal {
            return;
        }
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: if literal {
                "Use the `lambda` method for multiline lambdas."
            } else {
                "Use the `-> { ... }` lambda literal syntax for single line lambdas."
            }
            .into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, cx, file))
                .flatten(),
        });
    }
}

fn lambda_call(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.name().as_slice() == b"lambda"
            && call.receiver().is_none()
            && call.arguments().is_none()
            && call.opening_loc().is_none()
            && call.block().is_some_and(|b| b.as_block_node().is_some())
    })
}

/// RuboCop's direct unparenthesized argument/pair rule, not arbitrary ancestry.
fn command_argument(cx: &NodeContext<'_, '_>) -> bool {
    let mut parents = cx.ancestors();
    let mut parent = parents.next();
    if parent.is_some_and(|p| p.as_assoc_node().is_some()) {
        parent = parents.next();
        if parent.is_some_and(|p| p.as_keyword_hash_node().is_some()) {
            parent = parents.next();
        }
    }
    if parent.is_some_and(|p| p.as_arguments_node().is_some()) {
        return parents
            .next()
            .and_then(Node::as_call_node)
            .is_some_and(|c| c.opening_loc().is_none());
    }
    false
}
fn implicit_rescue(body: Option<Node<'_>>) -> bool {
    body.and_then(|b| b.as_begin_node()).is_some_and(|b| {
        b.begin_keyword_loc().is_none()
            && (b.rescue_clause().is_some() || b.ensure_clause().is_some())
    })
}
pub(super) fn in_pattern(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut child = node;
    for parent in cx.ancestors() {
        if matches!(
            parent,
            Node::PinnedExpressionNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
        ) {
            return false;
        }
        let predicate = parent
            .as_if_node()
            .map(|n| n.predicate())
            .or_else(|| parent.as_unless_node().map(|n| n.predicate()));
        if predicate.is_some_and(|p| p.location().start_offset() == child.location().start_offset())
        {
            return false;
        }
        let pattern = parent
            .as_in_node()
            .map(|n| n.pattern())
            .or_else(|| parent.as_match_predicate_node().map(|n| n.pattern()))
            .or_else(|| parent.as_match_required_node().map(|n| n.pattern()));
        if let Some(pattern) = pattern {
            return pattern.location().start_offset() == child.location().start_offset();
        }
        child = parent;
    }
    false
}

/// Only move a horizontal header. Bodies, comments, heredoc payloads and every
/// physical line remain in place. A single atomic span owns paired delimiters.
fn correction(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    if !file
        .lambda_facts(root)
        .proven
        .contains(&node.location().start_offset())
        || file.class_method_bindings(root).observes_sources
    {
        return None;
    }
    let edit = if let Some(lambda) = node.as_lambda_node() {
        let start = lambda.operator_loc().start_offset();
        let opening = lambda.opening_loc();
        let header = start..opening.end_offset();
        if file.bytes[header.clone()]
            .iter()
            .any(|b| matches!(b, b'\n' | b'\r'))
            || super::correction::contains_comment(file, header)
        {
            return None;
        }
        let args = parameter_string(lambda.parameters(), true, root, file)?;
        let braces = opening.as_slice() == b"{" || command_argument(cx);
        // Any other argument path (splats, super/yield, operators, loop
        // predicates, etc.) needs a stronger binding proof than this rule.
        if binding_uncertain(node, cx) {
            return None;
        }
        let mut replacement = format!("lambda {}", if braces { "{" } else { "do" });
        if !args.is_empty() {
            replacement.push_str(&format!(" |{args}|"));
        }
        let mut edits = vec![Correction {
            range: start..opening.end_offset(),
            replacement,
        }];
        if braces && opening.as_slice() == b"do" {
            let closing = lambda.closing_loc();
            edits.push(Correction {
                range: closing.start_offset()..closing.end_offset(),
                replacement: "}".into(),
            });
        }
        super::correction::combine(file, edits)?
    } else {
        let call = node.as_call_node()?;
        let block = call.block()?.as_block_node()?;
        let selector = call.message_loc()?;
        let args = parameter_string(block.parameters(), false, root, file)?;
        let mut edits = vec![Correction {
            range: selector.start_offset()..selector.end_offset(),
            replacement: if args.is_empty() {
                "->".into()
            } else {
                format!("->({args})")
            },
        }];
        if let Some(params) = block
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
        {
            let right = params.closing_loc()?;
            let range = block.opening_loc().end_offset()..right.end_offset();
            if file.bytes[range.clone()]
                .iter()
                .any(|b| matches!(b, b'\n' | b'\r'))
                || super::correction::contains_comment(file, range.clone())
            {
                return None;
            }
            // Explicit || must stay explicit: deleting it enables implicit
            // numbered/it parameters on modern Ruby. Empty arrows use ().
            if args.is_empty() {
                edits[0].replacement = "->()".into();
            }
            edits.push(Correction {
                range,
                replacement: String::new(),
            });
        }
        super::correction::combine(file, edits)?
    };
    super::correction::preserves_data_positions(file, cx, &edit).then_some(edit)
}

fn binding_uncertain(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    // A do/end method block can bind differently from an arrow. Accept only
    // direct call arguments handled above, or value containers/assignments.
    let mut child = node;
    for parent in cx.ancestors() {
        match parent {
            Node::ArgumentsNode { .. } | Node::AssocNode { .. } | Node::KeywordHashNode { .. } => {}
            Node::CallNode { .. } => {
                return child.as_arguments_node().is_none();
            }
            Node::ArrayNode { .. }
            | Node::HashNode { .. }
            | Node::ParenthesesNode { .. }
            | Node::LocalVariableWriteNode { .. }
            | Node::StatementsNode { .. }
            | Node::ProgramNode { .. } => return false,
            _ => return true,
        }
        child = parent;
    }
    false
}

fn parameter_string(
    parameters: Option<Node<'_>>,
    to_method: bool,
    root: &Node<'_>,
    file: &SourceFile,
) -> Option<String> {
    let Some(parameters) = parameters else {
        return Some(String::new());
    };
    let Some(block) = parameters.as_block_parameters_node() else {
        // Numbered and Ruby 3.4 it parameters remain implicit, in their own scope.
        return Some(String::new());
    };
    if file.contains_heredoc(root, &parameters)
        || super::correction::contains_comment(
            file,
            block.location().start_offset()..block.location().end_offset(),
        )
    {
        return None;
    }
    let mut nodes = Vec::new();
    if let Some(p) = block.parameters() {
        nodes.extend(p.requireds().iter());
        nodes.extend(p.optionals().iter());
        nodes.extend(p.rest());
        nodes.extend(p.posts().iter());
        nodes.extend(p.keywords().iter());
        nodes.extend(p.keyword_rest());
        nodes.extend(p.block().map(|b| b.as_node()));
    }
    // Prism stores parameter categories in Ruby's grammatical source order.
    let mut strings = Vec::new();
    for n in &nodes {
        if n.as_implicit_rest_node().is_some() {
            return None;
        }
        let loc = n.location();
        let text = std::str::from_utf8(loc.as_slice()).ok()?;
        if text.contains(['\n', '\r']) {
            return None;
        }
        let default = n
            .as_optional_parameter_node()
            .map(|p| p.value())
            .or_else(|| n.as_optional_keyword_parameter_node().map(|p| p.value()));
        let wrap = default.filter(|v| to_method && needs_default_parens(v));
        strings.push(if let Some(value) = wrap {
            let v = value.location();
            let start = v.start_offset() - loc.start_offset();
            let end = v.end_offset() - loc.start_offset();
            format!("{}({}){}", &text[..start], &text[start..end], &text[end..])
        } else {
            text.into()
        });
    }
    // A block's trailing comma has destructuring semantics, and cannot simply
    // disappear when moved to arrow syntax. Keep the offense without a fix.
    if !to_method
        && block.locals().is_empty()
        && let (Some(last), Some(close)) = (nodes.last(), block.closing_loc())
        && file.bytes[last.location().end_offset()..close.start_offset()].contains(&b',')
    {
        return None;
    }
    let mut result = strings.join(", ");
    let locals: Vec<_> = block
        .locals()
        .iter()
        .map(|n| {
            std::str::from_utf8(n.location().as_slice())
                .ok()
                .map(str::to_owned)
        })
        .collect::<Option<_>>()?;
    if !locals.is_empty() {
        result.push_str("; ");
        result.push_str(&locals.join(", "));
    }
    Some(result)
}
fn needs_default_parens(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::RangeNode { .. }
            | Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::AndNode { .. }
            | Node::OrNode { .. }
    ) || node.as_call_node().is_some_and(|c| {
        c.name().as_slice() != b"[]" && crate::cops::ast::is_operator_method(c.name().as_slice())
    })
}

/// A deliberately closed, eager top-level prefix: no arbitrary call, method,
/// class, refinement, attr/alias/undef/eval or inherited runtime context may
/// precede a proven lookup. Arguments evaluate before dispatch; literal lambda
/// bodies/defaults are deferred and never establish proof for nested candidates.
/// One lazy main-AST walk, no per-offense reparsing or sibling/subtree scans.
#[derive(Default)]
pub(crate) struct Facts {
    pub(super) proven: std::collections::HashSet<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Index {
            facts: Facts,
            stopped: bool,
            deferred: usize,
            stack: Vec<(bool, bool)>,
            begin: bool,
        }
        impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Index {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                self.begin |= node.as_pre_execution_node().is_some();
                let scope = matches!(
                    node,
                    Node::LambdaNode { .. }
                        | Node::BlockNode { .. }
                        | Node::DefNode { .. }
                        | Node::ClassNode { .. }
                        | Node::ModuleNode { .. }
                        | Node::SingletonClassNode { .. }
                );
                let method = lambda_call(&node);
                let unknown_call = node.as_call_node().is_some() && !method;
                if self.deferred == 0 && !self.stopped {
                    if node.as_lambda_node().is_some() || method {
                        self.facts.proven.insert(node.location().start_offset());
                    } else if !matches!(
                        node,
                        Node::ProgramNode { .. }
                            | Node::StatementsNode { .. }
                            | Node::LocalVariableWriteNode { .. }
                            | Node::LocalVariableReadNode { .. }
                            | Node::ParenthesesNode { .. }
                            | Node::ArrayNode { .. }
                            | Node::HashNode { .. }
                            | Node::KeywordHashNode { .. }
                            | Node::AssocNode { .. }
                            | Node::ArgumentsNode { .. }
                            | Node::CallNode { .. }
                            | Node::BlockNode { .. }
                            | Node::IntegerNode { .. }
                            | Node::FloatNode { .. }
                            | Node::StringNode { .. }
                            | Node::SymbolNode { .. }
                            | Node::NilNode { .. }
                            | Node::TrueNode { .. }
                            | Node::FalseNode { .. }
                    ) {
                        self.stopped = true;
                    }
                }
                self.stack.push((scope, unknown_call && self.deferred == 0));
                self.deferred += usize::from(scope);
            }
            fn leave(&mut self) {
                let (scope, unknown) = self.stack.pop().unwrap();
                self.deferred -= usize::from(scope);
                self.stopped |= unknown;
            }
        }
        let mut index = Index::default();
        crate::cops::walk::walk(&mut index, root);
        if index.begin {
            index.facts.proven.clear();
        }
        index.facts
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/lambda_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/Lambda")
            .collect()
    }
    #[test]
    fn exact_fixed_policy_positions_and_messages() {
        for source in [
            "f = lambda { |x| x }",
            "f = lambda { x }",
            "f = lambda { _1 }",
            "f = lambda { it }",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 5, 1));
            assert_eq!(
                found[0].message,
                "Use the `-> { ... }` lambda literal syntax for single line lambdas."
            );
        }
        for source in [
            "f = ->(x) do\n x\nend",
            "f = -> {\n _1\n}",
            "f = -> {\n it\n}",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 5, 1));
            assert_eq!(
                found[0].message,
                "Use the `lambda` method for multiline lambdas."
            );
        }
    }
    #[test]
    fn no_offense_lookalikes_and_pinned_rescue_exemptions() {
        for source in [
            "l = lambda do |x|\n x\nend",
            "l = lambda do\n _1\nend",
            "l = lambda do\n it\nend",
            "lambda = ->(x) { x }; lambda.(1)",
            "-> { _1 }",
            "-> { it }",
            "l = lambda.test",
            "self.lambda { 1 }",
            "Kernel.lambda { 1 }",
            "lambda() { 1 }",
            "proc { 1 }",
            "Proc.new { 1 }",
            "foo&.bar do |_|\n baz\nend",
            "do_something :foo, ->(x) do\n x.bar\nrescue\n nil\nend",
            "do_something :foo, ->(x) do\n x.bar\nensure\n x.baz\nend",
            "case value\nin -> {\n _1.positive?\n}\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn multiline_pattern_guards_and_pins_are_expressions_not_patterns() {
        for source in [
            "case value\nin x if -> {\n 1\n}.call\nend",
            "case value\nin x unless -> {\n 1\n}.call\nend",
            "case value\nin ^(-> {\n _1\n})\nend",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "value in -> {\n _1\n}",
            "case v\nin [String, Hash | -> {\n _1\n}]\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn all_23_pinned_correction_pairs() {
        let pairs: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/lambda_corrections.json"
        ))
        .unwrap();
        assert_eq!(pairs.as_array().unwrap().len(), 23);
        for pair in pairs.as_array().unwrap() {
            crate::cops::testing::assert_correction(
                &Lambda,
                pair["before"].as_str().unwrap(),
                pair["after"].as_str().unwrap(),
            );
        }
    }
    fn unchanged(source: &str) {
        let found = crate::cops::testing::correction_offenses(&Lambda, source);
        assert!(!found.is_empty(), "{source}");
        assert!(
            found.iter().all(|o| o.correction.is_none()),
            "{source}: {found:?}"
        );
    }
    #[test]
    fn closed_prefix_rejects_custom_inherited_refined_removed_and_dynamic_lookup() {
        for prefix in [
            "def lambda(&b); proc(&b); end\n",
            "def self.lambda(&b); proc(&b); end\n",
            "alias lambda proc\n",
            "undef lambda\n",
            "private :lambda\n",
            "public :lambda\n",
            "define_method(:lambda) { |&b| proc(&b) }\n",
            "define_method(\"lambda\") { |&b| proc(&b) }\n",
            "send(:define_method, name) { |&b| proc(&b) }\n",
            "singleton_class.attr_reader(:lambda)\n",
            "attr :lambda\n",
            "attr_accessor(\"lambda\")\n",
            "attr_reader(name)\n",
            "remove_method :lambda\n",
            "undef_method(\"lambda\")\n",
            "class Object; def lambda(&b); proc(&b); end; end\n",
            "module R; refine Object do; def lambda(&b); proc(&b); end; end; end; using R\n",
            "require 'other'\n",
            "eval(code)\n",
            "unknown\n",
        ] {
            for candidate in ["f = lambda { |x| x }\n", "f = ->(x) do\n x\nend\n"] {
                unchanged(&format!("{prefix}{candidate}"));
            }
        }
        for source in [
            "class C < Parent; def f; lambda { |x| x }; end; end",
            "class C < BasicObject; def f; -> do\n 1\nend; end; end",
            "obj.instance_eval { lambda { |x| x } }",
            "define_method(:f) { -> do\n 1\nend }",
            "f = lambda { |x| x }; BEGIN { def lambda(&b); proc(&b); end }",
            "f = -> do\n 1\nend; BEGIN { def lambda(&b); proc(&b); end }",
            "f = -> { lambda { _1 } }",
        ] {
            unchanged(source);
        }
        let nested =
            crate::cops::testing::correction_offenses(&Lambda, "f = -> { -> do\n 1\nend }");
        assert_eq!(nested.len(), 2);
        assert!(nested[0].correction.is_some());
        assert!(nested[1].correction.is_none());
    }
    #[test]
    fn parameter_defaults_destructuring_shadow_locals_and_explicit_empty_scope() {
        for (before, after) in [
            (
                "f = lambda { |x; y| y = x; y }",
                "f = ->(x; y) { y = x; y }",
            ),
            (
                "f = ->(x; y) do\n y = x; y\nend",
                "f = lambda do |x; y|\n y = x; y\nend",
            ),
            ("f = lambda { || 42 }", "f = ->() { 42 }"),
            (
                "f = lambda { |(a,b), *xs, flag: true, **kw, &blk| [a,b,xs,flag,kw,blk] }",
                "f = ->((a,b), *xs, flag: true, **kw, &blk) { [a,b,xs,flag,kw,blk] }",
            ),
            (
                "f = ->(x = y + z) do\n x\nend",
                "f = lambda do |x = (y + z)|\n x\nend",
            ),
            (
                "f = ->(x = ..y, key: y..) do\n x\nend",
                "f = lambda do |x = (..y), key: (y..)|\n x\nend",
            ),
            (
                "f = ->(x = y ? z : w) do\n x\nend",
                "f = lambda do |x = (y ? z : w)|\n x\nend",
            ),
            (
                "f = ->(x = !y) do\n x\nend",
                "f = lambda do |x = (!y)|\n x\nend",
            ),
            (
                "f = ->(x = unless y then z else w end) do\n x\nend",
                "f = lambda do |x = (unless y then z else w end)|\n x\nend",
            ),
            (
                "f = ->(x = y[1], key: (..y)) do\n x\nend",
                "f = lambda do |x = y[1], key: (..y)|\n x\nend",
            ),
        ] {
            crate::cops::testing::assert_correction(&Lambda, before, after);
        }
        unchanged("f = lambda { |x,| x }");
    }
    #[test]
    fn literal_values_header_lines_comments_and_heredoc_defaults_never_move() {
        for source in [
            "f = ->(\n x\n) do\n x\nend",
            "f = ->(x = 'a\nb') do\n x\nend",
            "f = ->(x = <<~TEXT) do\n a\nTEXT\n x\nend",
            "f = ->(x # header\n) do\n x\nend",
        ] {
            unchanged(source);
        }
        for (before, after) in [
            (
                "f = ->(x = __LINE__) do # stays\n [x, __LINE__, 'do } |']\nend # end\np __LINE__\n",
                "f = lambda do |x = __LINE__| # stays\n [x, __LINE__, 'do } |']\nend # end\np __LINE__\n",
            ),
            (
                "f = lambda { |x = 'a,b;|', y: /[|,]/| [x,y] }",
                "f = ->(x = 'a,b;|', y: /[|,]/) { [x,y] }",
            ),
            (
                "f = -> do\n <<~TEXT\n  } end | # literal\nTEXT\nend\n",
                "f = lambda do\n <<~TEXT\n  } end | # literal\nTEXT\nend\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&Lambda, before, after);
        }
    }
    #[test]
    fn observers_data_and_unproven_argument_evaluation_withhold_individually() {
        for source in [
            "f = lambda { 1 }; p DATA.pos\n__END__\nraw\n",
            "f = -> do\n 1\nend; p Object.const_get(:DATA).pos\n__END__\nraw\n",
            "f = lambda { 1 }; p f.source_location",
            "f = lambda { 1 }; p File.read(__FILE__)",
            "f = lambda { 1 }; def delegate(*args, &block); target(*args, &block); end",
            "consume(unknown, -> do\n 1\nend)",
            "receiver.consume(-> do\n 1\nend)",
            "consume(*[-> do\n 1\nend])",
        ] {
            unchanged(source);
        }
        let source = "a = lambda { 1 }; unknown; b = lambda { 2 }";
        let found = crate::cops::testing::correction_offenses(&Lambda, source);
        assert_eq!(found.len(), 2);
        assert!(found[0].correction.is_some());
        assert!(found[1].correction.is_none());
    }
    #[test]
    fn argument_and_keyword_association_detection() {
        for (source, col) in [
            (
                "has_many :kittens, -> do\n where(cats: Cat.young.where_values_hash)\nend, source: cats",
                20,
            ),
            ("do_something :foo, ->(x) do\n x.bar rescue nil\nend", 20),
            (
                "do_something(:foo, ->(x) do\n x.bar\nrescue\n nil\nend)",
                20,
            ),
            (
                "has_many opt: -> do\n where(cats: Cat.young.where_values_hash)\nend",
                15,
            ),
            ("has_many(opt: -> do\n x\nend)", 15),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
        }
    }
}
