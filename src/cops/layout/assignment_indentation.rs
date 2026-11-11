//! `Layout/AssignmentIndentation` — first RHS alignment at fixed width two.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct AssignmentIndentation;
impl Cop for AssignmentIndentation {
    fn name(&self) -> &'static str {
        "Layout/AssignmentIndentation"
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
        let parent = cx
            .ancestors()
            .find(|parent| parent.as_arguments_node().is_none());
        let Some(alignment) = projection(node, parent, file) else {
            return;
        };
        let (line, col) = file.line_col(alignment.expression.start);
        out.push(Offense {
            correction: (cx.corrections_requested()
                && !enclosing_shift(cx, file, alignment.expression.start))
            .then(|| alignment.correction(file, cx.ancestors().next_back().unwrap_or(node)))
            .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message: "Indent the first line of the right-hand-side of a multi-line assignment."
                .into(),
            line,
            col,
            end_line: file.line_of(
                alignment
                    .expression
                    .end
                    .saturating_sub(1)
                    .max(alignment.expression.start),
            ),
        });
    }
}
fn projection(
    node: &Node<'_>,
    parent: Option<&Node<'_>>,
    file: &SourceFile,
) -> Option<super::alignment::Alignment> {
    let (operator, rhs) = assignment(node)?;
    let location = rhs.location();
    if file.line_of(operator.start_offset()) == file.line_of(location.start_offset())
        || !super::alignment::starts_line(file, location.start_offset())
    {
        return None;
    }
    // The pinned helper recurses but returns its immediate same-line assignment
    // parent (not the recursive result). ArgumentsNode is transparent upstream.
    let base = parent
        .filter(|parent| {
            is_assignment(parent)
                && file.line_of(parent.location().start_offset())
                    == file.line_of(node.location().start_offset())
        })
        .unwrap_or(node);
    let expected = super::alignment::display_column(file, base.location().start_offset())
        + super::alignment::INDENT;
    super::alignment::Alignment::new(
        file,
        location.start_offset()..location.end_offset(),
        expected,
    )
}

/// Reserve outer shifts, including withheld ones, before scanning RHS lines.
/// Only ancestry is consulted: no root/subtree walk or nested line rescans.
fn enclosing_shift(cx: &NodeContext<'_, '_>, file: &SourceFile, offset: usize) -> bool {
    let mut parent = None;
    for node in cx.ancestors().rev() {
        if projection(node, parent, file).is_some_and(|a| a.expression.contains(&offset)) {
            return true;
        }
        if node.as_arguments_node().is_none() {
            parent = Some(node);
        }
    }
    false
}

/// Ancestry uses AST assignment identity, not CheckAssignment callback dispatch.
/// A safe-navigation setter is an assignment parent even though the pinned cop
/// has no on_csend callback. Ordinary calls/comparisons remain non-assignments.
fn is_assignment(node: &Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        call.equal_loc().is_some()
    } else {
        assignment(node).is_some()
    }
}

/// Concrete CheckAssignment callbacks, excluding ordinary sends/comparisons
/// which have no assignment operator. Prism keeps setter `=` separately.
fn assignment<'pr>(node: &Node<'pr>) -> Option<(ruby_prism::Location<'pr>, Node<'pr>)> {
    macro_rules! write {
        ($method:ident) => {{
            let n = node.$method()?;
            (n.operator_loc(), n.value())
        }};
    }
    macro_rules! op_write {
        ($method:ident) => {{
            let n = node.$method()?;
            (n.binary_operator_loc(), n.value())
        }};
    }
    Some(match node {
        Node::LocalVariableWriteNode { .. } => write!(as_local_variable_write_node),
        Node::InstanceVariableWriteNode { .. } => write!(as_instance_variable_write_node),
        Node::ClassVariableWriteNode { .. } => write!(as_class_variable_write_node),
        Node::GlobalVariableWriteNode { .. } => write!(as_global_variable_write_node),
        Node::ConstantWriteNode { .. } => write!(as_constant_write_node),
        Node::ConstantPathWriteNode { .. } => write!(as_constant_path_write_node),
        Node::MultiWriteNode { .. } => write!(as_multi_write_node),
        Node::LocalVariableOrWriteNode { .. } => write!(as_local_variable_or_write_node),
        Node::InstanceVariableOrWriteNode { .. } => write!(as_instance_variable_or_write_node),
        Node::ClassVariableOrWriteNode { .. } => write!(as_class_variable_or_write_node),
        Node::GlobalVariableOrWriteNode { .. } => write!(as_global_variable_or_write_node),
        Node::ConstantOrWriteNode { .. } => write!(as_constant_or_write_node),
        Node::ConstantPathOrWriteNode { .. } => write!(as_constant_path_or_write_node),
        Node::LocalVariableAndWriteNode { .. } => write!(as_local_variable_and_write_node),
        Node::InstanceVariableAndWriteNode { .. } => write!(as_instance_variable_and_write_node),
        Node::ClassVariableAndWriteNode { .. } => write!(as_class_variable_and_write_node),
        Node::GlobalVariableAndWriteNode { .. } => write!(as_global_variable_and_write_node),
        Node::ConstantAndWriteNode { .. } => write!(as_constant_and_write_node),
        Node::ConstantPathAndWriteNode { .. } => write!(as_constant_path_and_write_node),
        Node::LocalVariableOperatorWriteNode { .. } => {
            op_write!(as_local_variable_operator_write_node)
        }
        Node::InstanceVariableOperatorWriteNode { .. } => {
            op_write!(as_instance_variable_operator_write_node)
        }
        Node::ClassVariableOperatorWriteNode { .. } => {
            op_write!(as_class_variable_operator_write_node)
        }
        Node::GlobalVariableOperatorWriteNode { .. } => {
            op_write!(as_global_variable_operator_write_node)
        }
        Node::ConstantOperatorWriteNode { .. } => op_write!(as_constant_operator_write_node),
        Node::ConstantPathOperatorWriteNode { .. } => {
            op_write!(as_constant_path_operator_write_node)
        }
        Node::CallOrWriteNode { .. } => write!(as_call_or_write_node),
        Node::CallAndWriteNode { .. } => write!(as_call_and_write_node),
        Node::CallOperatorWriteNode { .. } => op_write!(as_call_operator_write_node),
        Node::IndexOrWriteNode { .. } => write!(as_index_or_write_node),
        Node::IndexAndWriteNode { .. } => write!(as_index_and_write_node),
        Node::IndexOperatorWriteNode { .. } => op_write!(as_index_operator_write_node),
        Node::CallNode { .. } => {
            let n = node.as_call_node()?;
            if n.is_safe_navigation() {
                return None;
            }
            (n.equal_loc()?, n.arguments()?.arguments().iter().last()?)
        }
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's assignment_indentation_spec.rb at 97f30c4
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == AssignmentIndentation.name())
            .collect()
    }
    #[test]
    fn pinned_rhs_and_multiple_assignment_corrections() {
        for (before, after, line, col) in [
            ("a =\nif b ; end\n", "a =\n  if b ; end\n", 2, 1),
            ("a,\nb =\nif b ; end\n", "a,\nb =\n  if b ; end\n", 3, 1),
            ("foo = bar =\nbaz = ''\n", "foo = bar =\n  baz = ''\n", 2, 1),
            (
                "foo =\n  bar =\n  baz = 42\n",
                "foo =\n  bar =\n    baz = 42\n",
                3,
                3,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert_eq!(
                found[0].message,
                "Indent the first line of the right-hand-side of a multi-line assignment."
            );
            assert_correction(&AssignmentIndentation, before, after);
            assert!(offenses(after).is_empty());
        }
    }
    #[test]
    fn safe_navigation_setter_parents_anchor_without_receiving_callbacks() {
        for (lhs, expected) in [
            ("x&.y = a =", 2),
            ("  x&.y = a =", 4),
            ("x&.y = @a =", 2),
            ("x&.y = a +=", 2),
            ("x&.y = a = b =", 9),
            ("x&.y =\na =", 2),
            ("x&.y(a =", 7),
            ("x.y = a =", 2),
            ("x&.y ||= a =", 2),
        ] {
            let correct = format!(
                "{lhs}\n{}1{}\n",
                " ".repeat(expected),
                if lhs == "x&.y(a =" { ")" } else { "" }
            );
            assert!(offenses(&correct).is_empty(), "{correct:?}");
            for indent in [0, expected - 1, expected + 1, expected + 7] {
                let before = format!(
                    "{lhs}\n{}1{}\n",
                    " ".repeat(indent),
                    if lhs == "x&.y(a =" { ")" } else { "" }
                );
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before:?}: {found:?}");
                let line = lhs.lines().count() as u32 + 1;
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (line, indent as u32 + 1, line)
                );
                assert_eq!(
                    found[0].message,
                    "Indent the first line of the right-hand-side of a multi-line assignment."
                );
                assert_correction(&AssignmentIndentation, &before, &correct);
            }
        }
        // The csend itself still has no CheckAssignment callback upstream.
        for source in ["x&.y =\n1\n", "x&.y =\n         1\n"] {
            assert!(offenses(source).is_empty(), "{source:?}");
            assert!(
                crate::cops::testing::correction_offenses(&AssignmentIndentation, source)
                    .is_empty()
            );
        }
    }
    #[test]
    fn shifts_whole_rhs_but_not_literal_bytes_or_line_numbers() {
        for (before, after) in [
            ("a =\n'foo'\n", "a =\n  'foo'\n"),
            (
                "a =\nfoo do\n  bar\nend\n",
                "a =\n  foo do\n    bar\n  end\n",
            ),
            ("a =\nif b\n  x\nend\n", "a =\n  if b\n    x\n  end\n"),
            ("a =\n\"a\nraw\"\n", "a =\n  \"a\nraw\"\n"),
            (
                "a =\nbegin\n  x = <<~H\n raw\nH\n  __LINE__\nend\n",
                "a =\n  begin\n    x = <<~H\n raw\nH\n    __LINE__\n  end\n",
            ),
        ] {
            assert_correction(&AssignmentIndentation, before, after);
        }
    }
    #[test]
    fn negative_shifts_with_unshiftable_clauses_retain_findings_without_partial_edits() {
        use crate::cops::testing::correction_offenses;
        // A partial negative shift can make the opening keyword accidentally
        // align with an untouched end, deleting a default/-w compiler warning.
        let before = "a =\n          if true\n  1\n  end\np a\n";
        let found = correction_offenses(&AssignmentIndentation, before);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        assert_correction(&AssignmentIndentation, before, before);
    }
    #[test]
    fn negative_shifts_with_tabs_retain_findings_without_edits() {
        use crate::cops::testing::correction_offenses;
        for before in [
            "a =\n  \tif true\n  \t  1\n  \tend\np a\n",
            "a =\n    if true\n\t1\n    end\np a\n",
            "a =\n    if true\n      1\n\tend\np a\n",
        ] {
            let found = correction_offenses(&AssignmentIndentation, before);
            assert_eq!(found.len(), 1, "{before:?}");
            assert!(found[0].correction.is_none(), "{before:?}");
            assert_correction(&AssignmentIndentation, before, before);
        }
    }
    #[test]
    fn pinned_scalar_display_width_not_terminal_emoji_ligatures() {
        for (word, width) in [
            ("❤️", 1),
            ("❤︎", 1),
            ("👨‍👩‍👧‍👦", 8),
            ("👩🏽‍💻", 6),
            ("🇵🇱", 2),
            ("1️⃣", 1),
            ("e\u{301}", 1),
            ("\u{2e3b}", 3),
            ("\u{9be}", 1),
            ("a\u{8}", 0),
        ] {
            let expected = width + 8;
            let correct = format!("f '{word}', a =\n{}b\n", " ".repeat(expected));
            assert!(offenses(&correct).is_empty(), "{correct:?}");
            for indent in [0, expected - 1, expected + 1] {
                let before = format!("f '{word}', a =\n{}b\n", " ".repeat(indent));
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before:?}");
                assert_eq!((found[0].line, found[0].col), (2, indent as u32 + 1));
                assert_eq!(
                    found[0].message,
                    "Indent the first line of the right-hand-side of a multi-line assignment."
                );
                assert_correction(&AssignmentIndentation, &before, &correct);
            }
        }
    }
    #[test]
    fn nested_shifts_reserve_outer_span_and_wide_siblings_stay_independent() {
        use crate::cops::testing::correction_offenses;
        let nested = "a =\n".repeat(48) + "1\n";
        let found = correction_offenses(&AssignmentIndentation, &nested);
        assert_eq!(found.len(), 48);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
        let wide = "a =\n1\n".repeat(500);
        let found = correction_offenses(&AssignmentIndentation, &wide);
        assert_eq!(found.len(), 500);
        assert!(found.iter().all(|o| o.correction.is_some()));
    }
    #[test]
    fn pinned_correct_inline_fullwidth_and_comparison() {
        for source in [
            "a = if b\n      foo\n    end\n",
            "a =\n  if b ; end\n",
            "f 'Ｒｕｂｙ', a =\n                b\n",
            "a ===\nif b ; end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
