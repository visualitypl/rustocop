//! `Layout/ArrayAlignment` — fixed alignment with the first element.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ArrayAlignment;
impl Cop for ArrayAlignment {
    fn name(&self) -> &'static str {
        "Layout/ArrayAlignment"
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
        let Some(items) = elements(node, cx.parent()) else {
            return;
        };
        let base = super::alignment::display_column(file, items[0].location().start_offset());
        let reserved = cx.corrections_requested() && enclosing_shift(node, cx, file);
        for item in misaligned(file, &items, base) {
            let loc = item.location();
            let Some(alignment) =
                super::alignment::Alignment::new(file, loc.start_offset()..loc.end_offset(), base)
            else {
                continue;
            };
            let (line, col) = file.line_col(loc.start_offset());
            out.push(Offense {
                correction: (cx.corrections_requested() && !reserved)
                    .then(|| alignment.correction(file, cx.ancestors().next_back().unwrap_or(node)))
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message: "Align the elements of an array literal if they span more than one line."
                    .into(),
                line,
                col,
                end_line: file.last_line_of(loc),
            });
        }
    }
}
/// RuboCop's `array` nodes with at least two children: literal and implicit
/// arrays (never a multiple-assignment value) and, through the parser
/// translation, a `rescue` clause's exception list.
fn elements<'pr>(node: &Node<'pr>, parent: Option<&Node<'pr>>) -> Option<Vec<Node<'pr>>> {
    let items: Vec<_> = if let Some(array) = node.as_array_node() {
        if parent.is_some_and(|p| p.as_multi_write_node().is_some()) {
            return None;
        }
        array.elements().iter().collect()
    } else {
        node.as_rescue_node()?.exceptions().iter().collect()
    };
    (items.len() >= 2).then_some(items)
}
/// Upstream `each_bad_alignment`: an item that begins its line (and so a
/// later line than the previous item) at another display column than `base`.
fn misaligned<'a, 'pr>(
    file: &'a SourceFile,
    items: &'a [Node<'pr>],
    base: usize,
) -> impl Iterator<Item = &'a Node<'pr>> + 'a {
    items.iter().filter(move |item| {
        let start = item.location().start_offset();
        super::alignment::starts_line(file, start)
            && super::alignment::display_column(file, start) != base
    })
}
/// Upstream defers an element contained in an earlier offense of this cop
/// (`within?`); those can only be misaligned items of enclosing arrays.
/// Follow the concrete ancestor edges from NodeContext, never the root, and
/// measure only the element on the path (no per-ancestor element scans).
fn enclosing_shift(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    let mut child = node.location();
    let mut ancestors = cx.ancestors().peekable();
    while let Some(parent) = ancestors.next() {
        let first = if let Some(array) = parent.as_array_node() {
            // Every child of an array is one of its elements.
            let masgn = ancestors
                .peek()
                .is_some_and(|p| p.as_multi_write_node().is_some());
            (!masgn && array.elements().len() >= 2)
                .then(|| array.elements().iter().next())
                .flatten()
        } else if let Some(rescue) = parent.as_rescue_node() {
            let exceptions = rescue.exceptions();
            let member = exceptions.iter().any(|e| {
                let loc = e.location();
                (loc.start_offset(), loc.end_offset()) == (child.start_offset(), child.end_offset())
            });
            (member && exceptions.len() >= 2)
                .then(|| exceptions.iter().next())
                .flatten()
        } else {
            None
        };
        if let Some(first) = first {
            let start = child.start_offset();
            if super::alignment::starts_line(file, start)
                && super::alignment::display_column(file, start)
                    != super::alignment::display_column(file, first.location().start_offset())
            {
                return true;
            }
        }
        child = parent.location();
    }
    false
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop array_alignment_spec.rb at 97f30c4
    //! (MIT, Copyright (c) 2012–26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    const MSG: &str = "Align the elements of an array literal if they span more than one line.";
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == ArrayAlignment.name())
            .collect()
    }
    fn positions(source: &str) -> Vec<(u32, u32, u32)> {
        let found = offenses(source);
        assert!(found.iter().all(|o| o.message == MSG), "{source}");
        found.iter().map(|o| (o.line, o.col, o.end_line)).collect()
    }
    #[test]
    fn registers_and_corrects_misaligned_elements() {
        let source = "array = [a,\n   b,\n  c,\n   d]\n";
        assert_eq!(positions(source), [(2, 4, 2), (3, 3, 3), (4, 4, 4)]);
        assert_correction(
            &ArrayAlignment,
            source,
            "array = [a,\n         b,\n         c,\n         d]\n",
        );
    }
    #[test]
    fn misaligned_elements_with_the_first_element_on_a_new_row() {
        let source = "array = [\n  a,\n   b,\n  c,\n   d\n]\n";
        assert_eq!(positions(source), [(3, 4, 3), (5, 4, 5)]);
        assert_correction(
            &ArrayAlignment,
            source,
            "array = [\n  a,\n  b,\n  c,\n  d\n]\n",
        );
    }
    #[test]
    fn nested_arrays_reserve_the_outer_shift_for_one_pass() {
        for (source, fixed, expected) in [
            (
                "[:l1,\n  [:l2,\n    [:l3,\n     [:l4]]]]\n",
                "[:l1,\n [:l2,\n   [:l3,\n    [:l4]]]]\n",
                [(2, 3, 4), (3, 5, 4)],
            ),
            (
                "[:l1,\n[:l2,\n  [:l3,\n   [:l4]]]]\n",
                "[:l1,\n [:l2,\n   [:l3,\n    [:l4]]]]\n",
                [(2, 1, 4), (3, 3, 4)],
            ),
        ] {
            assert_eq!(positions(source), expected);
            let found = correction_offenses(&ArrayAlignment, source);
            assert_eq!(found.len(), 2);
            assert!(found[0].correction.is_some() && found[1].correction.is_none());
            assert_correction(&ArrayAlignment, source, fixed);
        }
    }
    #[test]
    fn heredoc_bodies_are_not_indented_by_corrections() {
        let source = "var = [\n       { :type => 'something',\n         :sql => <<EOF\nSelect something\nfrom atable\nEOF\n       },\n      { :type => 'something',\n        :sql => <<EOF\nSelect something\nfrom atable\nEOF\n      }\n]\n";
        assert_eq!(positions(source), [(8, 7, 13)]);
        assert_correction(
            &ArrayAlignment,
            source,
            "var = [\n       { :type => 'something',\n         :sql => <<EOF\nSelect something\nfrom atable\nEOF\n       },\n       { :type => 'something',\n         :sql => <<EOF\nSelect something\nfrom atable\nEOF\n       }\n]\n",
        );
    }
    #[test]
    fn accepted_pinned_shapes() {
        for source in [
            "array = [a,\n         b,\n         c,\n         d]\n",
            "array = [ a, b ]\n",
            "array = [ a, b,\n          c, d ]\n",
            "puts 'Ｒｕｂｙ', [ a,\n                   b ]\n",
            "array = [\n  a,\n  b,\n  c,\n  d\n]\n",
            "thing, foo =\n  1, 2\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn implicit_percent_splat_and_rescue_lists_are_arrays() {
        for (before, after, expected) in [
            (
                "var =\n     first,\n    second\n",
                "var =\n     first,\n     second\n",
                vec![(3, 5, 3)],
            ),
            ("x.y = 1,\n  2\n", "x.y = 1,\n      2\n", vec![(2, 3, 2)]),
            ("A = *a,\n  b\n", "A = *a,\n    b\n", vec![(2, 3, 2)]),
            (
                "begin\nrescue ArgumentError,\n  TypeError => e\nend\n",
                "begin\nrescue ArgumentError,\n       TypeError => e\nend\n",
                vec![(3, 3, 3)],
            ),
            (
                "[1,\n   *rest,\n **opts]\n",
                "[1,\n *rest,\n **opts]\n",
                vec![(2, 4, 2)],
            ),
        ] {
            assert_eq!(positions(before), expected, "{before}");
            assert_correction(&ArrayAlignment, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
    }
    #[test]
    fn percent_array_words_are_reported_but_their_literal_interior_is_not_edited() {
        // The shared alignment seam protects whole percent-literal interiors.
        let source = "%w[one\n  two\n    three]\n";
        assert_eq!(positions(source), [(2, 3, 2), (3, 5, 3)]);
        let found = correction_offenses(&ArrayAlignment, source);
        assert!(found.iter().all(|o| o.correction.is_none()));
        assert_eq!(positions("%i[one\n    two]\n"), [(2, 5, 2)]);
        assert!(offenses("%w[one\n   two]\n").is_empty());
    }
    #[test]
    fn multiple_assignment_values_single_elements_and_lookalikes_are_ignored() {
        for source in [
            "a, b = [1,\n  2]\n",
            "a, (b, c) = 1,\n  2\n",
            "x = *a\n",
            "[a,\n b].each { }\n",
            "[\n  1\n]\n",
            "f(1,\n  2)\n",
            "case x\nin [a,\n  b]\nend\n",
            "case x\nwhen a,\n  b\nend\n",
            "begin\nrescue A\nend\n",
            "x rescue y\n",
            "[a, b,\n c].size\n",
            "[a, foo(b,\n  c)]\n",
            "[a, <<~X,\n  body\nX\n b]\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn sibling_and_inner_offenses_outside_a_reserved_shift_are_corrected() {
        let source = "x = [[1,\n  2],\n  [3,\n4]]\n";
        let found = correction_offenses(&ArrayAlignment, source);
        assert_eq!(
            found
                .iter()
                .map(|o| (o.line, o.col, o.correction.is_some()))
                .collect::<Vec<_>>(),
            [(2, 3, true), (3, 3, true), (4, 1, false)]
        );
        assert_correction(
            &ArrayAlignment,
            source,
            "x = [[1,\n      2],\n     [3,\n   4]]\n",
        );
    }
    #[test]
    fn tabs_and_vertical_tabs_use_display_columns_and_guard_negative_shifts() {
        // The reference display width of a tab is one column.
        assert!(offenses("[a,\n\tb]\n").is_empty());
        let source = "[a,\n\t\tb]\n";
        assert_eq!(positions(source), [(2, 3, 2)]);
        assert!(
            correction_offenses(&ArrayAlignment, source)[0]
                .correction
                .is_none()
        );
        assert_correction(&ArrayAlignment, "x = [1,\n\tb]\n", "x = [1,\n\t    b]\n");
        assert_eq!(positions("[a,\n\u{b}b]\n"), [(2, 2, 2)]);
        assert_correction(&ArrayAlignment, "[a,\n\u{b}b]\n", "[a,\n\u{b} b]\n");
    }
    #[test]
    fn closed_source_observers_hold_and_other_readers_do_not() {
        let source = "x = [1,\n  2]\np __FILE__\n";
        let found = correction_offenses(&ArrayAlignment, source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        assert_correction(
            &ArrayAlignment,
            "x = [1,\n  2]\np File.read('x.rb')\n",
            "x = [1,\n     2]\np File.read('x.rb')\n",
        );
    }
    #[test]
    fn wide_and_deep_arrays_measure_only_the_enclosing_path() {
        let wide = format!("x = [\n{}]\n", "  [1,\n    2],\n".repeat(3000));
        let found = correction_offenses(&ArrayAlignment, &wide);
        assert_eq!(found.len(), 3000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        let deep = "[a,\n".repeat(24) + "b" + &"]".repeat(24) + "\n";
        let found = correction_offenses(&ArrayAlignment, &deep);
        assert_eq!(found.len(), 24);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
    }
}
