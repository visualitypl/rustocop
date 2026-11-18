//! `Layout/ArgumentAlignment` — fixed alignment with the first argument.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ArgumentAlignment;
impl Cop for ArgumentAlignment {
    fn name(&self) -> &'static str {
        "Layout/ArgumentAlignment"
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
        let Some(items) = items(node) else { return };
        let base = super::alignment::display_column(file, items[0].location().start_offset());
        let enclosing = cx.corrections_requested() && enclosing_shift(node, cx, file);
        let mut previous = 0;
        for item in items {
            let loc = item.location();
            let line = file.line_of(loc.start_offset());
            let new_line = line > previous;
            previous = line;
            if !new_line || !super::alignment::starts_line(file, loc.start_offset()) {
                continue;
            }
            let Some(alignment) =
                super::alignment::Alignment::new(file, loc.start_offset()..loc.end_offset(), base)
            else {
                continue;
            };
            let (line, col) = file.line_col(loc.start_offset());
            out.push(Offense {
                correction: (cx.corrections_requested() && !enclosing)
                    .then(|| alignment.correction(file, cx.ancestors().next_back().unwrap_or(node)))
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message: "Align the arguments of a method call if they span more than one line."
                    .into(),
                line,
                col,
                end_line: file.last_line_of(loc),
            });
        }
    }
}
/// HashAlignment's fixed defaults use `key` for both delimiters, never
/// `separator`, so enabling or disabling that cop cannot change this fixed
/// projection; do not expose alternative styles here.
fn items<'pr>(node: &Node<'pr>) -> Option<Vec<Node<'pr>>> {
    let (_, flatten) = first_item(node)?;
    let (arguments, block) = argument_list(node)?;
    let mut arguments: Vec<_> = arguments.arguments().iter().collect();
    arguments.extend(block);
    if flatten {
        Some(
            arguments[0]
                .as_keyword_hash_node()?
                .elements()
                .iter()
                .filter(|n| n.as_assoc_node().is_some())
                .collect(),
        )
    } else {
        Some(arguments)
    }
}
/// Reserve containing shifts even when their safety guard withholds the edit.
/// Follow concrete argument edges using NodeContext, not repeated root scans.
/// The ancestor probe does not collect positional siblings; only an initial
/// implicit hash's prefix up to its first two real pairs is inspected.
fn enclosing_shift(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    let mut child = node;
    let mut argument = None;
    let mut pair = None;
    for parent in cx.ancestors() {
        if parent.as_keyword_hash_node().is_some() {
            pair = Some(child);
        }
        if parent.as_arguments_node().is_some() {
            argument = Some(child);
        }
        if let Some((first, flatten)) = first_item(parent) {
            let item = if child.as_arguments_node().is_some() {
                if flatten { pair } else { argument }
            } else if child.as_block_argument_node().is_some() && !flatten {
                Some(child)
            } else {
                None
            };
            if let Some(item) = item {
                let loc = item.location();
                if (!flatten || item.as_assoc_node().is_some())
                    && super::alignment::starts_line(file, loc.start_offset())
                    && super::alignment::display_column(file, loc.start_offset())
                        != super::alignment::display_column(file, first.location().start_offset())
                {
                    return true;
                }
            }
        }
        child = parent;
    }
    false
}
/// First projected item and whether an initial implicit hash is flattened.
fn first_item<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, bool)> {
    let (arguments, block) = argument_list(node)?;
    let list = arguments.arguments();
    let first = list.iter().next()?;
    if let Some(hash) = first.as_keyword_hash_node() {
        let mut pairs = hash
            .elements()
            .iter()
            .filter(|n| n.as_assoc_node().is_some());
        let first_pair = pairs.next()?;
        if list.len() < 2 && block.is_none() && pairs.next().is_none() {
            return None;
        }
        Some((first_pair, true))
    } else if list.len() >= 2 || block.is_some() {
        Some((first, false))
    } else {
        None
    }
}
/// Compound index assignments contain a real synthetic `[]` send upstream;
/// ordinary/index-target setters instead contain `[]=` and remain excluded.
fn argument_list<'pr>(
    node: &Node<'pr>,
) -> Option<(ruby_prism::ArgumentsNode<'pr>, Option<Node<'pr>>)> {
    let (arguments, block) = match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node()?;
            if excluded(&call) {
                return None;
            }
            (call.arguments(), call.block())
        }
        Node::IndexAndWriteNode { .. } => {
            let n = node.as_index_and_write_node()?;
            (n.arguments(), n.block().map(|b| b.as_node()))
        }
        Node::IndexOrWriteNode { .. } => {
            let n = node.as_index_or_write_node()?;
            (n.arguments(), n.block().map(|b| b.as_node()))
        }
        Node::IndexOperatorWriteNode { .. } => {
            let n = node.as_index_operator_write_node()?;
            (n.arguments(), n.block().map(|b| b.as_node()))
        }
        _ => return None,
    };
    Some((
        arguments?,
        block.filter(|b| b.as_block_argument_node().is_some()),
    ))
}
fn excluded(call: &ruby_prism::CallNode<'_>) -> bool {
    let name = call.name();
    name.as_slice() == b"[]="
        || (crate::cops::ast::is_operator_method(name.as_slice())
            && name.as_slice() != b"[]"
            && call.call_operator_loc().is_none()
            && call
                .message_loc()
                .is_some_and(|m| m.start_offset() != call.location().start_offset()))
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop argument_alignment_spec.rb at 97f30c4
    //! (MIT, Copyright (c) 2012–26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == ArgumentAlignment.name())
            .collect()
    }
    #[test]
    fn ruby_vertical_tab_indentation_has_exact_positions_and_aligned_lookalikes() {
        for (before, after, col) in [
            ("f(a,\n\u{b}b)\n", "f(a,\n\u{b}  b)\n", 2),
            ("f(a,\n   \u{b}b)\n", "f(a,\n   \u{b}b)\n", 5),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before:?}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, col, 2)
            );
            assert_eq!(
                found[0].message,
                "Align the arguments of a method call if they span more than one line."
            );
            assert_correction(&ArgumentAlignment, before, after);
            assert_eq!(offenses(after).is_empty(), before != after, "{after:?}");
        }
        for source in [
            "f(a,\n\u{b}  b)\n",
            "f(a,\n \u{b} b)\n",
            "f(\n\u{b}a,\n\u{b}b)\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }
    #[test]
    fn literal_splat_and_keyword_containers_keep_real_canonical_corrections() {
        for (before, after) in [
            ("f(1,\n*[2, 3])\n", "f(1,\n  *[2, 3])\n"),
            (
                "f(a: 1,\n**{k: 3},\nb: 2)\n",
                "f(a: 1,\n**{k: 3},\n  b: 2)\n",
            ),
        ] {
            assert_eq!(offenses(before).len(), 1);
            assert_correction(&ArgumentAlignment, before, after);
            assert!(offenses(after).is_empty());
        }
    }
    #[test]
    fn nested_shifts_reserve_the_outer_expression_but_siblings_remain_independent() {
        let source = "f(a,\ng(b,\nc),\nd)\n";
        let found = crate::cops::testing::correction_offenses(&ArgumentAlignment, source);
        assert_eq!(found.len(), 3);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 2);
        assert_correction(&ArgumentAlignment, source, "f(a,\n  g(b,\n  c),\n  d)\n");
    }
    #[test]
    fn shifts_continuations_and_blocks_but_not_heredoc_or_literal_contents() {
        for (before, after) in [
            ("f(a,\ng do\n  b\nend)\n", "f(a,\n  g do\n    b\n  end)\n"),
            ("f(a,\n\"b\nraw\")\n", "f(a,\n  \"b\nraw\")\n"),
            (
                "f(a,\nbegin\n  x = <<~H\n raw\nH\n  __LINE__\nend)\n",
                "f(a,\n  begin\n    x = <<~H\n raw\nH\n    __LINE__\n  end)\n",
            ),
            ("f(a,\ng(b, \\\n  c))\n", "f(a,\n  g(b, \\\n    c))\n"),
        ] {
            assert_correction(&ArgumentAlignment, before, after);
        }
    }
    #[test]
    fn compound_index_getters_are_real_sends_but_plain_setters_and_targets_are_not() {
        for op in ["||=", "&&=", "+="] {
            let before = format!("o[a,\nb] {op} c\n");
            let found = offenses(&before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 2));
            assert_correction(&ArgumentAlignment, &before, &format!("o[a,\n  b] {op} c\n"));
        }
        for source in ["o[a,\nb] = c\n", "o[a,\nb], c = values\n"] {
            assert!(offenses(source).is_empty());
        }
    }
    #[test]
    fn guarded_negative_tabs_and_document_comments_keep_findings() {
        for source in [
            "f(a,\n  \tb)\n",
            "f(a,\nbegin\n=begin\ncomment\n=end\n  1\nend)\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&ArgumentAlignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
            assert_correction(&ArgumentAlignment, source, source);
        }
    }
    #[test]
    fn lines_indented_less_than_a_negative_shift_stay_as_rubocop_leaves_them() {
        assert_correction(
            &ArgumentAlignment,
            "f(a,\n          if true\n  1\n  end)\n",
            "f(a,\n  if true\n  1\n  end)\n",
        );
    }
    #[test]
    fn wide_and_nested_argument_shifts_do_not_rescan_root_or_nested_expressions() {
        let source = "f(a,\n".repeat(24) + "b" + &")".repeat(24) + "\n";
        let found = crate::cops::testing::correction_offenses(&ArgumentAlignment, &source);
        assert_eq!(found.len(), 24);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
        let wide = "f(a,\nb)\n".repeat(500);
        let found = crate::cops::testing::correction_offenses(&ArgumentAlignment, &wide);
        assert_eq!(found.len(), 500);
        assert!(found.iter().all(|o| o.correction.is_some()));
    }
    #[test]
    fn exclusions_and_no_offense_lookalikes() {
        for source in [
            "Test.config[\"something\"] =\n true\n",
            "x[1,\n2] = 3\n",
            "x&.[]=(1,\n2)\n",
            "foo << {\n         a: 1,\n b: 2\n}\n",
            "f({a: 1,\nb: 2})\n",
            "f(a: 1,\n**kw)\n",
            "f(**kw,\na: 1)\n",
            "f()\n",
            "f(a)\n",
            "f(a, b, c)\n",
            "f(a,\n  b, c)\n",
            "f(\n a,\n b\n)\n",
            "run(collection, :entry_name => label,\n                :paginator => paginator)\n",
            "read(path, { headers: true,\n             converters: :numeric })\n",
            "f 'Ｒｕｂｙ', g(a,\n                b)\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_positional_safe_navigation_and_index_calls() {
        for (before, after, positions) in [
            (
                "function(a,\n  if b then c else d end)\n",
                "function(a,\n         if b then c else d end)\n",
                vec![(2, 3)],
            ),
            (
                "func(a,\n       b,\nc)\n",
                "func(a,\n     b,\n     c)\n",
                vec![(2, 8), (3, 1)],
            ),
            (
                "receiver&.function(a,\n  if b then c else d end)\n",
                "receiver&.function(a,\n                   if b then c else d end)\n",
                vec![(2, 3)],
            ),
            (
                "callable[\n      foo,\n    bar,\n           baz\n]\n",
                "callable[\n      foo,\n      bar,\n      baz\n]\n",
                vec![(3, 5), (4, 12)],
            ),
        ] {
            let found = offenses(before);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{before}"
            );
            assert!(found.iter().all(|o| o.message
                == "Align the arguments of a method call if they span more than one line."));
            assert_correction(&ArgumentAlignment, before, after);
            assert!(offenses(after).is_empty());
        }
    }
}
