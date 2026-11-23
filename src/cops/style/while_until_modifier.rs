//! `Style/WhileUntilModifier` — single-expression pre-test loops.
use super::if_unless_modifier::{
    body_source, compression_binding_safe, correction_parens, first_comment, needs_parens,
    semantic_parent,
};
use crate::{
    cops::{
        Companions, Cop, NodeContext, variables,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node, StatementsNode};

/// Prism's pre-test, modifier and post-test loop shapes, shared by the loop cops.
pub(super) struct Loop<'pr> {
    pub keyword: Location<'pr>,
    pub end: Option<Location<'pr>>,
    pub do_keyword: Option<Location<'pr>>,
    pub predicate: Node<'pr>,
    pub body: Option<StatementsNode<'pr>>,
    pub post_test: bool,
}
impl<'pr> Loop<'pr> {
    pub fn of(node: &Node<'pr>) -> Option<Self> {
        if let Some(n) = node.as_while_node() {
            Some(Self {
                keyword: n.keyword_loc(),
                end: n.closing_loc(),
                do_keyword: n.do_keyword_loc(),
                predicate: n.predicate(),
                body: n.statements(),
                post_test: n.is_begin_modifier(),
            })
        } else {
            node.as_until_node().map(|n| Self {
                keyword: n.keyword_loc(),
                end: n.closing_loc(),
                do_keyword: n.do_keyword_loc(),
                predicate: n.predicate(),
                body: n.statements(),
                post_test: n.is_begin_modifier(),
            })
        }
    }
    pub fn word(&self) -> &'static str {
        if self.keyword.as_slice() == b"while" {
            "while"
        } else {
            "until"
        }
    }
    pub fn single_body(&self) -> Option<Node<'pr>> {
        self.body
            .as_ref()
            .filter(|s| s.body().len() == 1)?
            .body()
            .first()
    }
}

pub struct WhileUntilModifier {
    max: Option<usize>,
}
impl Default for WhileUntilModifier {
    fn default() -> Self {
        Self { max: Some(120) }
    }
}
impl Cop for WhileUntilModifier {
    fn name(&self) -> &'static str {
        "Style/WhileUntilModifier"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn with_companions(&self, companions: &Companions) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            max: companions.line_length_limit,
        }))
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        matches!(node, Node::WhileNode { .. } | Node::UntilNode { .. })
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(c) = Loop::of(node) else { return };
        let Some(end) = c.end.as_ref().filter(|_| !c.post_test) else {
            return;
        };
        let Some(body) = c.single_body() else { return };
        if matches!(
            body,
            Node::IfNode { .. }
                | Node::UnlessNode { .. }
                | Node::WhileNode { .. }
                | Node::UntilNode { .. }
                | Node::CaseNode { .. }
                | Node::CaseMatchNode { .. }
                | Node::BeginNode { .. }
                | Node::ParenthesesNode { .. }
        ) {
            return;
        }
        let start_line = file.line_of(c.keyword.start_offset());
        let end_line = file.line_of(end.start_offset());
        if (start_line..=end_line)
            .filter(|&line| !file.line_text(line).iter().all(u8::is_ascii_whitespace))
            .take(4)
            .count()
            > 3
            || file.has_comment_in_lines(end_line..end_line + 1)
            || file.has_comment_in_lines(
                file.line_of(body.location().start_offset())
                    ..file.last_line_of(body.location()) + 1,
            )
        {
            return;
        }
        #[derive(Default)]
        struct Assignments {
            found: bool,
            targets_are_assignments: Vec<bool>,
        }
        impl<'pr> NodeVisitor<'pr> for Assignments {
            fn enter(&mut self, n: Node<'pr>) {
                self.found |= variables::is_local_assignment(&n)
                    && (n.as_local_variable_target_node().is_none()
                        || self
                            .targets_are_assignments
                            .last()
                            .copied()
                            .unwrap_or(false));
                // Named regexp captures and pattern bindings are not RuboCop
                // lvasgn nodes; destructuring, for and rescue targets are.
                self.targets_are_assignments.push(matches!(
                    n,
                    Node::MultiWriteNode { .. }
                        | Node::MultiTargetNode { .. }
                        | Node::ForNode { .. }
                        | Node::RescueNode { .. }
                ));
            }
            fn leave(&mut self) {
                self.targets_are_assignments.pop();
            }
        }
        let mut assignments = Assignments::default();
        walk(&mut assignments, &c.predicate);
        if assignments.found {
            return;
        }
        let parens = semantic_parent(cx).is_some_and(needs_parens);
        if self.render(&c, &body, parens, file).is_none() {
            return;
        }
        let correction = if cx.corrections_requested() {
            self.rewrite(node, &c, &body, cx, file)
        } else {
            None
        };
        let (line, col) = file.line_col(c.keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: format!(
                "Favor modifier `{}` usage when having a single-line body.",
                c.word()
            ),
            correction,
        });
    }
}
impl WhileUntilModifier {
    fn render(
        &self,
        c: &Loop<'_>,
        body: &Node<'_>,
        parens: bool,
        file: &SourceFile,
    ) -> Option<Vec<u8>> {
        let end = c.end.as_ref()?;
        let line = file.line_of(c.keyword.start_offset());
        let prefix = &file.bytes[file.line_start(line)..c.keyword.start_offset()];
        let suffix = &file.line_text(file.line_of(end.start_offset()))
            [end.end_offset() - file.line_start(file.line_of(end.start_offset()))..];
        let comment = first_comment(file, line);
        if comment.is_some() && !suffix.is_empty() {
            return None;
        }
        let mut result = Vec::new();
        if parens {
            result.push(b'(');
        }
        result.extend_from_slice(&body_source(body));
        result.push(b' ');
        result.extend_from_slice(c.word().as_bytes());
        result.push(b' ');
        result.extend_from_slice(c.predicate.location().as_slice());
        if parens {
            result.push(b')');
        }
        if let Some(comment) = comment {
            result.push(b' ');
            result.extend_from_slice(comment.strip_suffix(b"\r").unwrap_or(comment));
        }
        if self.max.is_some_and(|max| {
            crate::cops::layout::line_length::display_width(prefix)
                + crate::cops::layout::line_length::display_width(&result)
                + crate::cops::layout::line_length::display_width(suffix)
                > max
        }) {
            return None;
        }
        Some(result)
    }
    fn rewrite(
        &self,
        node: &Node<'_>,
        c: &Loop<'_>,
        body: &Node<'_>,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
    ) -> Option<Correction> {
        let loc = node.location();
        let range = loc.start_offset()..loc.end_offset();
        let root = cx.ancestors().next_back()?;
        if range.len() > REWRITE_BUDGET
            || file.contains_heredoc(root, node)
            || file.heredoc_pending_at(root, range.start)
        {
            return None;
        }
        #[cfg(test)]
        REWRITE_BYTES.with(|v| v.set(v.get() + range.len()));
        if !compression_binding_safe(body, &c.predicate) {
            return None;
        }
        let parens = correction_parens(cx)?;
        let replacement = String::from_utf8(self.render(c, body, parens, file)?).ok()?;
        // A newline embedded in a literal or a continuation must not be moved.
        if c.predicate.location().as_slice().contains(&b'\n')
            || body.location().as_slice().contains(&b'\n')
        {
            return None;
        }
        let rendered_body = body_source(body);
        // Unsupported wrappers around omitted hash values and endless ranges
        // need an operand boundary that the shared renderer cannot establish.
        if rendered_body.ends_with(b":") || rendered_body.ends_with(b"..") {
            return None;
        }
        let comment = first_comment(file, file.line_of(c.keyword.start_offset()));
        let comments = file.comments();
        let first = comments.partition_point(|r| r.end <= range.start);
        if comment.is_some_and(|c| !super::correction::comment_relocation_safe(c))
            || comments[first..]
                .iter()
                .take_while(|r| r.start < range.end)
                .any(|r| Some(&file.bytes[r.clone()]) != comment)
        {
            return None;
        }
        Some(Correction { range, replacement })
    }
}
const REWRITE_BUDGET: usize = 16 * 1024;
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! Ported from RuboCop while_until_modifier_spec.rb and condition_modifier_cop.rb
    //! (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn positions_messages_and_canonical_corrections() {
        for word in ["while", "until"] {
            let s = format!("  {word} cond\n    work\n  end\n");
            let o: Vec<_> = lint_str(&s)
                .into_iter()
                .filter(|o| o.cop == "Style/WhileUntilModifier")
                .collect();
            assert_eq!(o.len(), 1);
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 3, 1));
            assert_eq!(
                o[0].message,
                format!("Favor modifier `{word}` usage when having a single-line body.")
            );
            for (before, after) in [
                (
                    format!("{word} cond\n work\nend\n"),
                    format!("work {word} cond\n"),
                ),
                (
                    format!("x = {word} cond\n work\nend\n"),
                    format!("x = (work {word} cond)\n"),
                ),
                (
                    format!("def f = {word} cond\n work\nend\n"),
                    format!("def f = (work {word} cond)\n"),
                ),
                (
                    format!("{word} cond # ordinary\n work\nend\n"),
                    format!("work {word} cond # ordinary\n"),
                ),
                (
                    format!("{word} cond; x = 0; end\n"),
                    format!("x = 0 {word} cond\n"),
                ),
            ] {
                assert_correction(&WhileUntilModifier::default(), &before, &after);
            }
        }
    }
    #[test]
    fn for_loop_bodies_report_without_uncertain_corrections() {
        for word in ["while", "until"] {
            for body in ["for x in xs; end", "for x in xs; work(x); end"] {
                let source = format!("{word} cond\n {body}\nend\n");
                let offenses: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == "Style/WhileUntilModifier")
                    .collect();
                assert_eq!(offenses.len(), 1, "{source}");
                assert_eq!((offenses[0].line, offenses[0].col), (1, 1));
                assert_eq!(
                    offenses[0].message,
                    format!("Favor modifier `{word}` usage when having a single-line body.")
                );
                let offenses = correction_offenses(&WhileUntilModifier::default(), &source);
                assert_eq!(offenses.len(), 1, "{source}");
                assert!(offenses[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn ineligible_lookalikes_and_post_test_loops() {
        for word in ["while", "until"] {
            for body in [
                "work if test",
                "work unless test",
                "test ? a : b",
                "work while test",
                "work until test",
                "case x; when 1; work; end",
                "case x; in 1; work; end",
                "begin work end",
                "(work)",
                "work; other",
                "work # comment",
            ] {
                let s = format!("{word} cond\n {body}\nend\n");
                assert!(
                    lint_str(&s)
                        .iter()
                        .all(|o| o.cop != "Style/WhileUntilModifier"),
                    "{s}"
                );
            }
            for s in [
                format!("begin\n work\nend {word} cond"),
                format!("work {word} cond"),
                format!("{word} (x = cond)\n work\nend"),
                format!("{word} (x, y = cond)\n work\nend"),
                format!("{word} cond\nend"),
                format!("{word} cond\n work\nend # comment"),
            ] {
                assert!(
                    lint_str(&s)
                        .iter()
                        .all(|o| o.cop != "Style/WhileUntilModifier"),
                    "{s}"
                );
            }
        }
    }
    #[test]
    fn length_policy_and_disabled_line_length() {
        let s = "while condition\n  work\nend\n";
        assert!(correction_offenses(&WhileUntilModifier { max: Some(10) }, s).is_empty());
        assert_correction(
            &WhileUntilModifier { max: None },
            s,
            "work while condition\n",
        );
        assert_correction(
            &WhileUntilModifier { max: Some(20) },
            s,
            "work while condition\n",
        );
    }
    #[test]
    fn correction_work_is_lazy_and_bounded_per_candidate() {
        let source = "while cond\n work\nend\n".repeat(2000);
        REWRITE_BYTES.with(|v| v.set(0));
        assert_eq!(
            lint_str(&source)
                .iter()
                .filter(|o| o.cop == "Style/WhileUntilModifier")
                .count(),
            2000
        );
        REWRITE_BYTES.with(|v| assert_eq!(v.get(), 0));
        let o = correction_offenses(&WhileUntilModifier::default(), &source);
        assert_eq!(o.len(), 2000);
        REWRITE_BYTES.with(|v| assert_eq!(v.get(), source.len() - 2000));
        let large = format!("while cond\n \"{}\"\nend\n", "x".repeat(REWRITE_BUDGET));
        REWRITE_BYTES.with(|v| v.set(0));
        let o = correction_offenses(&WhileUntilModifier { max: None }, &large);
        assert_eq!(o.len(), 1);
        assert!(o[0].correction.is_none());
        REWRITE_BYTES.with(|v| assert_eq!(v.get(), 0));
    }
    #[test]
    fn declaration_only_bodies_keep_their_loop_guard() {
        for body in ["alias old_name new_name", "alias $a $b", "undef old_name"] {
            let s = format!("while cond\n {body}\nend\n");
            assert_correction(
                &WhileUntilModifier::default(),
                &s,
                &format!("{body} while cond\n"),
            );
        }
    }
}
