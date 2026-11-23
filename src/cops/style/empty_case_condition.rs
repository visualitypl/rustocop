//! `Style/EmptyCaseCondition` — prefer an if chain to a subjectless case.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EmptyCaseCondition;
impl Cop for EmptyCaseCondition {
    fn name(&self) -> &'static str {
        "Style/EmptyCaseCondition"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_case_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(n) = node.as_case_node() else { return };
        if n.predicate().is_some() {
            return;
        }
        let parent = cx.ancestors().find(|a| a.as_arguments_node().is_none());
        if parent.is_some_and(|p| {
            matches!(
                p,
                Node::ReturnNode { .. }
                    | Node::BreakNode { .. }
                    | Node::NextNode { .. }
                    | Node::CallNode { .. }
                    | Node::YieldNode { .. }
                    | Node::SuperNode { .. }
            )
        }) {
            return;
        }
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        if file
            .empty_case_return_bodies(root)
            .contains(&n.location().start_offset())
        {
            return;
        }
        let (line, col) = file.line_col(n.case_keyword_loc().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Do not use empty `case` condition, instead use an `if` expression.".into(),
            correction: cx
                .corrections_requested()
                .then(|| correct(&n, cx, file))
                .flatten(),
        });
    }
}
fn correct(
    n: &ruby_prism::CaseNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    let loc = n.location();
    if loc.end_offset() - loc.start_offset() > 16 * 1024
        || file.contains_heredoc(root, &n.as_node())
        || file.heredoc_pending_at(root, loc.start_offset())
    {
        return None;
    }
    let branches: Vec<_> = n
        .conditions()
        .iter()
        .filter_map(|w| w.as_when_node())
        .collect();
    let first = branches.first()?;
    let header = n.case_keyword_loc().start_offset()..first.keyword_loc().end_offset();
    let comments = file.comments();
    let at = comments.partition_point(|c| c.end <= header.start);
    let mut preserved = String::new();
    let newline = if file.bytes[header.clone()].windows(2).any(|w| w == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let line_start = file.line_start(file.line_of(header.start));
    let indent = std::str::from_utf8(&file.bytes[line_start..header.start]).ok()?;
    for comment in comments[at..].iter().take_while(|c| c.start < header.end) {
        if !indent.bytes().all(|b| b == b' ' || b == b'\t')
            || !super::correction::comment_relocation_safe(&file.bytes[comment.clone()])
        {
            return None;
        }
        preserved.push_str(indent);
        preserved.push_str(
            std::str::from_utf8(&file.bytes[comment.clone()])
                .ok()?
                .trim_end_matches(['\r', '\n']),
        );
        preserved.push_str(newline);
    }
    let mut edits = vec![Correction {
        range: header,
        replacement: "if".into(),
    }];
    if !preserved.is_empty() {
        edits.push(Correction {
            range: line_start..line_start,
            replacement: preserved,
        });
    }
    for (i, branch) in branches.iter().enumerate() {
        if i > 0 {
            edits.push(Correction {
                range: branch.keyword_loc().start_offset()..branch.keyword_loc().end_offset(),
                replacement: "elsif".into(),
            });
        }
        let conditions: Vec<_> = branch.conditions().iter().collect();
        if conditions.iter().any(|c| changes_boolean_meaning(c)) {
            return None;
        }
        // Even a single when operand is not necessarily an if predicate:
        // English `not` and command calls can bind across the new header.
        if conditions.len() > 1 || conditions.iter().any(|c| !tight_condition(c, 0)) {
            let first = conditions.first()?.location();
            let last = conditions.last()?.location();
            let range = first.start_offset()..last.end_offset();
            if super::correction::contains_comment(file, range.clone()) {
                return None;
            }
            let mut joined = Vec::new();
            for c in &conditions {
                let text = std::str::from_utf8(c.location().as_slice()).ok()?;
                joined.push(if tight_condition(c, 0) {
                    text.to_owned()
                } else {
                    format!("({text})")
                });
            }
            edits.push(Correction {
                range,
                replacement: joined.join(" || "),
            });
        }
    }
    let edit = super::correction::combine(file, edits)?;
    Some(edit)
}
// Only leave proven tight operands exposed to the inserted `||` / if header.
// In particular Prism represents plain attribute/index setters as CallNodes,
// not WriteNodes. Test assignment flags/equal_loc before call delimiters (even
// `obj.foo=(value)` has assignment syntax). Explicit `obj.[]=(index, value)`
// dispatch is delimited, but command calls and English `not` are not atoms.
// Check exposed receivers/operator operands too.
fn tight_condition(n: &Node<'_>, depth: usize) -> bool {
    if depth == 64 {
        return false;
    }
    if matches!(
        n,
        Node::LocalVariableReadNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::ClassVariableReadNode { .. }
            | Node::GlobalVariableReadNode { .. }
            | Node::ConstantReadNode { .. }
            | Node::ConstantPathNode { .. }
            | Node::SelfNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::StringNode { .. }
            | Node::SymbolNode { .. }
            | Node::ParenthesesNode { .. }
    ) {
        return true;
    }
    let Some(c) = n.as_call_node() else {
        return false;
    };
    if c.is_attribute_write()
        || c.equal_loc().is_some()
        || c.block().is_some()
        || c.message_loc().is_some_and(|l| l.as_slice() == b"not")
        || !c.receiver().is_none_or(|r| tight_condition(&r, depth + 1))
    {
        return false;
    }
    c.opening_loc().is_some()
        || c.arguments().is_none()
        || c.call_operator_loc().is_none()
            && crate::cops::ast::is_operator_method(c.name().as_slice())
            && c.arguments()
                .is_some_and(|a| a.arguments().iter().all(|n| tight_condition(&n, depth + 1)))
}
fn changes_boolean_meaning(n: &Node<'_>) -> bool {
    match n {
        Node::SplatNode { .. }
        | Node::RegularExpressionNode { .. }
        | Node::InterpolatedRegularExpressionNode { .. }
        | Node::RangeNode { .. } => true,
        _ => {
            // Ruby's conditional-context rewrite descends through logical
            // operands, turning regexp values into $_ matches and ranges into
            // flip-flops. Subjectless when tests their ordinary truthiness.
            if let Some(a) = n.as_and_node() {
                return changes_boolean_meaning(&a.left()) || changes_boolean_meaning(&a.right());
            }
            if let Some(o) = n.as_or_node() {
                return changes_boolean_meaning(&o.left()) || changes_boolean_meaning(&o.right());
            }
            if let Some(p) = n.as_parentheses_node() {
                return p.body().is_some_and(|b| changes_boolean_meaning(&b));
            }
            if let Some(s) = n.as_statements_node() {
                return s
                    .body()
                    .iter()
                    .last()
                    .is_some_and(|n| changes_boolean_meaning(&n));
            }
            if let Some(b) = n.as_begin_node() {
                return b
                    .statements()
                    .is_some_and(|s| changes_boolean_meaning(&s.as_node()))
                    || b.rescue_clause().is_some()
                    || b.ensure_clause().is_some();
            }
            false
        }
    }
}

/// Fold descendant returns through actual AST edges, not source containment:
/// heredoc interpolation descendants can lie beyond their parent's location.
/// One bottom-up boolean per node avoids repeated scans of nested case bodies.
pub(crate) fn return_bodies(root: &Node<'_>) -> std::collections::HashSet<usize> {
    enum Kind {
        Case(usize),
        WhenOrElse,
        Other,
    }
    struct Frame {
        kind: Kind,
        has_return: bool,
        body_of: Option<usize>,
    }
    #[derive(Default)]
    struct Returns {
        stack: Vec<Frame>,
        cases: std::collections::HashSet<usize>,
    }
    impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Returns {
        fn enter(&mut self, n: Node<'pr>) {
            #[cfg(test)]
            VISITS.with(|v| v.set(v.get() + 1));
            let body_of = if n.as_statements_node().is_some() && self.stack.len() >= 2 {
                match (
                    &self.stack[self.stack.len() - 2].kind,
                    &self.stack[self.stack.len() - 1].kind,
                ) {
                    (Kind::Case(at), Kind::WhenOrElse) => Some(*at),
                    _ => None,
                }
            } else {
                None
            };
            let kind = if n.as_case_node().is_some() {
                Kind::Case(n.location().start_offset())
            } else if n.as_when_node().is_some() || n.as_else_node().is_some() {
                Kind::WhenOrElse
            } else {
                Kind::Other
            };
            self.stack.push(Frame {
                kind,
                has_return: n.as_return_node().is_some(),
                body_of,
            });
        }
        fn leave(&mut self) {
            let frame = self.stack.pop().expect("balanced walker");
            if frame.has_return {
                if let Some(at) = frame.body_of {
                    self.cases.insert(at);
                }
                if let Some(parent) = self.stack.last_mut() {
                    parent.has_return = true;
                }
            }
        }
    }
    #[cfg(test)]
    BUILDS.with(|v| v.set(v.get() + 1));
    let mut found = Returns::default();
    crate::cops::walk::walk(&mut found, root);
    found.cases
}
#[cfg(test)]
thread_local! {
    static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Detection-only bottom-up return facts for subjectless-case branch guards.
pub(crate) static EMPTY_CASE_RETURN_BODIES: crate::source::Fact<std::collections::HashSet<usize>> =
    crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn empty_case_return_bodies(
        &self,
        root: &ruby_prism::Node<'_>,
    ) -> &std::collections::HashSet<usize> {
        EMPTY_CASE_RETURN_BODIES.get(self, || return_bodies(root))
    }
}

#[cfg(test)]
mod tests {
    //! RuboCop empty_case_condition_spec.rb examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/empty_case_condition_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 30);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/EmptyCaseCondition")
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                c["positions"],
                "{c}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "{c}"
            );
            if let Some(after) = c["correction"].as_str() {
                assert_correction(&EmptyCaseCondition, source, after);
            }
        }
    }
    #[test]
    fn alternatives_precedence_assignments_and_nested_guards_keep_lexical_order() {
        for (before, after) in [
            (
                "v = case\nwhen (x = 1), y ? a : b\n x\nelse\n y\nend\n",
                "v = if (x = 1) || (y ? a : b)\n x\nelse\n y\nend\n",
            ),
            (
                "case\nwhen (a and b), (c or d)\n work\nend",
                "if (a and b) || (c or d)\n work\nend",
            ),
            (
                "case; when a; b; when c then d; else e; end",
                "if a; b; elsif c then d; else e; end",
            ),
            (
                "case\nwhen x\n unless y\n  a\n else\n  b\n end\nend",
                "if x\n unless y\n  a\n else\n  b\n end\nend",
            ),
            (
                "\u{feff}# coding: utf-8\r\ncase # why\r\nwhen true then \"é\"\r\nelse :no\r\nend\r\n__END__\r\npayload\x00\r\n",
                "\u{feff}# coding: utf-8\r\n# why\r\nif true then \"é\"\r\nelse :no\r\nend\r\n__END__\r\npayload\x00\r\n",
            ),
        ] {
            assert_correction(&EmptyCaseCondition, before, after);
        }
    }
    #[test]
    fn setter_alternatives_keep_the_assigned_value() {
        assert_correction(
            &EmptyCaseCondition,
            "a=[]; r = case\nwhen a[0] = false, true\n :yes\nelse\n :no\nend; p [r,a]",
            "a=[]; r = if (a[0] = false) || true\n :yes\nelse\n :no\nend; p [r,a]",
        );
    }
    #[test]
    fn english_not_is_grouped_even_as_the_only_condition() {
        assert_correction(
            &EmptyCaseCondition,
            "r = case\nwhen not false\n :yes\nelse\n :no\nend; p r",
            "r = if (not false)\n :yes\nelse\n :no\nend; p r",
        );
    }
    #[test]
    fn all_assignment_shapes_are_grouped_in_first_later_and_single_operands() {
        for expression in [
            "x = false",
            "@x = false",
            "@@x = false",
            "$x = false",
            "C = false",
            "A::C = false",
            "a[0] = false",
            "obj.value = false",
            "obj&.value = false",
            "x &&= false",
            "x ||= false",
            "x += 1",
            "@x &&= false",
            "@x ||= false",
            "@x += 1",
            "@@x &&= false",
            "@@x ||= false",
            "@@x += 1",
            "$x &&= false",
            "$x ||= false",
            "$x += 1",
            "C &&= false",
            "C ||= false",
            "C += 1",
            "A::C &&= false",
            "A::C ||= false",
            "A::C += 1",
            "a[0] &&= false",
            "a[0] ||= false",
            "a[0] += 1",
            "obj.value &&= false",
            "obj.value ||= false",
            "obj.value += 1",
            "obj&.value &&= false",
            "obj&.value ||= false",
            "obj&.value += 1",
        ] {
            for (operands, expected) in [
                (expression.to_owned(), format!("({expression})")),
                (
                    format!("{expression}, true"),
                    format!("({expression}) || true"),
                ),
                (
                    format!("false, {expression}"),
                    format!("false || ({expression})"),
                ),
            ] {
                assert_correction(
                    &EmptyCaseCondition,
                    &format!("case\nwhen {operands}\n :yes\nend"),
                    &format!("if {expected}\n :yes\nend"),
                );
            }
        }
    }
    #[test]
    fn call_shape_binding_is_proven_instead_of_assuming_every_call_is_atomic() {
        for (expression, expected) in [
            ("probe", "probe"),
            ("probe(false)", "probe(false)"),
            ("probe(*values)", "probe(*values)"),
            ("probe(**keywords)", "probe(**keywords)"),
            ("obj.probe", "obj.probe"),
            ("self.probe", "self.probe"),
            ("::A.probe", "::A.probe"),
            ("obj.probe(false)", "obj.probe(false)"),
            ("obj&.probe(false)", "obj&.probe(false)"),
            ("obj::probe(false)", "obj::probe(false)"),
            ("obj.(false)", "obj.(false)"),
            ("a[0]", "a[0]"),
            ("obj.value=(false)", "(obj.value=(false))"),
            ("obj.[]=(0, false)", "obj.[]=(0, false)"),
            ("obj.+(1)", "obj.+(1)"),
            ("obj.==(false)", "obj.==(false)"),
            ("!false", "!false"),
            ("!probe(false)", "!probe(false)"),
            ("1 == 2", "1 == 2"),
            ("probe { false }", "(probe { false })"),
            ("probe do false end", "(probe do false end)"),
            ("not(false)", "(not(false))"),
            ("defined? probe", "(defined? probe)"),
            ("(false and true)", "(false and true)"),
            ("(false or true)", "(false or true)"),
        ] {
            for (operands, after) in [
                (expression.to_owned(), expected.to_owned()),
                (format!("{expression}, true"), format!("{expected} || true")),
                (
                    format!("false, {expression}"),
                    format!("false || {expected}"),
                ),
            ] {
                assert_correction(
                    &EmptyCaseCondition,
                    &format!("case\nwhen {operands}\n :yes\nend"),
                    &format!("if {after}\n :yes\nend"),
                );
            }
        }
    }
    #[test]
    fn return_index_is_lazy_once_and_bounded_for_wide_and_nested_cases() {
        BUILDS.with(|v| v.set(0));
        VISITS.with(|v| v.set(0));
        crate::pipeline::lint_str("case x; when a; b; end");
        assert_eq!(BUILDS.with(|v| v.get()), 0);
        let source = "case\nwhen x\n work\nend\n".repeat(500);
        let found = correction_offenses(&EmptyCaseCondition, &source);
        assert_eq!(found.len(), 500);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert_eq!(BUILDS.with(|v| v.get()), 1);
        assert!(VISITS.with(|v| v.get()) < 10 * 500 + 2);
        BUILDS.with(|v| v.set(0));
        VISITS.with(|v| v.set(0));
        let source = format!(
            "{}work\n{}",
            "case; when x\n".repeat(15),
            "end\n".repeat(15)
        );
        assert_eq!(correction_offenses(&EmptyCaseCondition, &source).len(), 15);
        assert_eq!(BUILDS.with(|v| v.get()), 1);
        assert!(VISITS.with(|v| v.get()) < 10 * 15 + 2);
    }
    #[test]
    fn lookalikes_supported_parent_shapes_and_return_guards() {
        for source in [
            "case x; when 1; a; end",
            "def f\n case\n when ready\n  puts <<~TEXT\n   #{return 1}\n  TEXT\n end\nend\n",
            "case x; in Integer; a; end",
            "f(case; when a; b; end)",
            "obj&.f(case; when a; b; end)",
            "def f; yield case; when a; b; end; end",
            "def f; super(case; when a; b; end); end",
            "def f; return case; when a; b; end; end",
            "loop { break case; when a; b; end }",
            "loop { next case; when a; b; end }",
            "def f; case; when a; begin; return b; ensure; work; end; end; end",
        ] {
            assert!(
                correction_offenses(&EmptyCaseCondition, source).is_empty(),
                "{source}"
            );
        }
    }
}
