//! `Layout/SpaceInsideArrayLiteralBrackets` — fixed `no_space` for literals and patterns.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct SpaceInsideArrayLiteralBrackets;
impl Cop for SpaceInsideArrayLiteralBrackets {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideArrayLiteralBrackets"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        matches!(
            node,
            Node::ArrayNode { .. }
                | Node::ArrayPatternNode { .. }
                | Node::FindPatternNode { .. }
                | Node::HashPatternNode { .. }
        )
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !matches!(
            node,
            Node::ArrayNode { .. }
                | Node::ArrayPatternNode { .. }
                | Node::FindPatternNode { .. }
                | Node::HashPatternNode { .. }
        ) {
            return;
        }
        // RuboCop redirects a nested array to its nearest const_pattern.
        // Constant array patterns report themselves; constant hash/find patterns
        // participate only when they contain an array pattern or literal to redirect.
        if !constant_pattern(node) && cx.ancestors().any(constant_pattern) {
            return;
        }
        let delimiters = match node {
            Node::ArrayNode { .. } => {
                let n = node.as_array_node().unwrap();
                (n.opening_loc(), n.closing_loc())
            }
            Node::ArrayPatternNode { .. } => {
                let n = node.as_array_pattern_node().unwrap();
                (n.opening_loc(), n.closing_loc())
            }
            Node::FindPatternNode { .. } if constant_pattern(node) && contains_array(node) => {
                let n = node.as_find_pattern_node().unwrap();
                (n.opening_loc(), n.closing_loc())
            }
            Node::HashPatternNode { .. } if constant_pattern(node) && contains_array(node) => {
                let n = node.as_hash_pattern_node().unwrap();
                (n.opening_loc(), n.closing_loc())
            }
            _ => return,
        };
        let delimiters = if constant_pattern(node) {
            constant_brackets(node, cx, file)
        } else {
            super::bracket_spacing::offsets(delimiters)
        };
        // Constant patterns can share either delimiter with an enclosing
        // check, not just the entire pair. Report each offending side once.
        let (mut skip_left, mut skip_right) = (false, false);
        if let Some((left, right)) = delimiters {
            for ancestor in cx.ancestors().filter(|n| constant_pattern(n)) {
                let Some((outer_left, outer_right)) = constant_brackets(ancestor, cx, file) else {
                    continue;
                };
                // Test shared bounds before scanning the ancestor's pattern:
                // unrelated sibling constants must not each rescan that region.
                // Farther ancestors can only widen these token bounds.
                if left != outer_left && right != outer_right {
                    break;
                }
                if ancestor.as_array_pattern_node().is_some() || contains_array(ancestor) {
                    skip_left = left == outer_left;
                    skip_right = right == outer_right;
                    break;
                }
            }
        }
        if skip_left && skip_right {
            return;
        }
        super::bracket_spacing::check(
            self,
            node,
            cx,
            file,
            delimiters,
            super::bracket_spacing::Kind::Array {
                skip_left,
                skip_right,
            },
            out,
        );
    }
}

/// Pinned constant patterns use the first/last square-bracket tokens, even
/// when the constant's own delimiters are parentheses around a nested array.
fn constant_brackets(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<(usize, usize)> {
    let root = cx.ancestors().next_back().unwrap_or(node);
    let brackets = file.square_brackets(root);
    let start = brackets.partition_point(|&offset| offset < node.location().start_offset());
    let end = brackets.partition_point(|&offset| offset < node.location().end_offset());
    let tokens = &brackets[start..end];
    tokens
        .first()
        .zip(tokens.last())
        .and_then(|(&left, &right)| {
            (file.bytes[left] == b'[' && file.bytes[right] == b']').then_some((left, right))
        })
}

fn constant_pattern(node: &Node<'_>) -> bool {
    node.as_array_pattern_node()
        .is_some_and(|n| n.constant().is_some())
        || node
            .as_hash_pattern_node()
            .is_some_and(|n| n.constant().is_some())
        || node
            .as_find_pattern_node()
            .is_some_and(|n| n.constant().is_some())
}

/// Find array patterns and square literals (including in pinned expressions),
/// stopping at nested constant boundaries. Each edge is checked at most once
/// across all constant hash/find patterns.
fn contains_array(node: &Node<'_>) -> bool {
    use ruby_prism::{ArrayNode, ArrayPatternNode, FindPatternNode, HashPatternNode, Visit};
    struct Search {
        root: usize,
        found: bool,
    }
    impl<'pr> Visit<'pr> for Search {
        fn visit_array_node(&mut self, node: &ArrayNode<'pr>) {
            if node.opening_loc().is_some_and(|loc| loc.as_slice() == b"[") {
                self.found = true;
            } else if !self.found {
                ruby_prism::visit_array_node(self, node);
            }
        }
        fn visit_array_pattern_node(&mut self, node: &ArrayPatternNode<'pr>) {
            self.found |= node.constant().is_none();
        }
        fn visit_hash_pattern_node(&mut self, node: &HashPatternNode<'pr>) {
            if !self.found
                && (node.constant().is_none() || node.location().start_offset() == self.root)
            {
                ruby_prism::visit_hash_pattern_node(self, node);
            }
        }
        fn visit_find_pattern_node(&mut self, node: &FindPatternNode<'pr>) {
            if !self.found
                && (node.constant().is_none() || node.location().start_offset() == self.root)
            {
                ruby_prism::visit_find_pattern_node(self, node);
            }
        }
    }
    let mut search = Search {
        root: node.location().start_offset(),
        found: false,
    };
    search.visit(node);
    search.found
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/space_inside_array_literal_brackets_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == SpaceInsideArrayLiteralBrackets.name())
            .collect()
    }

    #[test]
    fn pinned_correction_examples_and_nested_literals() {
        for (before, after) in [
            ("['qux', 'baz'  ] - ['baz']", "['qux', 'baz'] - ['baz']"),
            (
                "['ok',\n 'still good',\n 'not good' ]",
                "['ok',\n 'still good',\n 'not good']",
            ),
            ("[:good,\n :bad  ].compact", "[:good,\n :bad].compact"),
            ("[2,3,4] - [ 3,4]", "[2,3,4] - [3,4]"),
            ("array = [ a, [ b, c ]]", "array = [a, [b, c]]"),
            ("[ [ 1 ], [ 2 ] ]", "[[1], [2]]"),
            (
                "[ *items, 1..3, (..4), (5...) ]",
                "[*items, 1..3, (..4), (5...)]",
            ),
            ("def f(*); [ * ]; end", "def f(*); [*]; end"),
            (
                "def f(...); g([ 1 ], ...); end",
                "def f(...); g([1], ...); end",
            ),
            (
                "case foo\nin [ bar, baz ]\nend",
                "case foo\nin [bar, baz]\nend",
            ),
            ("foo in [ bar, baz ]", "foo in [bar, baz]"),
            ("foo => [ bar, baz ]", "foo => [bar, baz]"),
            ("case value\nin ADT[ ]\nend", "case value\nin ADT[]\nend"),
            (
                "case value\nin ADT[*head, ADT[ ]]\nend",
                "case value\nin ADT[*head, ADT[]]\nend",
            ),
            (
                "case value\nin ADT[ *head, tail ]\nend",
                "case value\nin ADT[*head, tail]\nend",
            ),
            (
                "case value\nin ADT[ *head, ADT[ *headhead, tail ] ]\nend",
                "case value\nin ADT[*head, ADT[*headhead, tail]]\nend",
            ),
            (
                "ActiveRecord::Base.connection.execute(<<-SQL, [self.class.to_s ]).first[\"count\"]\n  SELECT COUNT(widgets.id) FROM widgets\n  WHERE widget_type = $1\nSQL\n",
                "ActiveRecord::Base.connection.execute(<<-SQL, [self.class.to_s]).first[\"count\"]\n  SELECT COUNT(widgets.id) FROM widgets\n  WHERE widget_type = $1\nSQL\n",
            ),
            ("[ <<~H ]\n  [ literal ]\nH\n", "[<<~H]\n  [ literal ]\nH\n"),
            ("\u{feff}[ \"é\" ]\r\n", "\u{feff}[\"é\"]\r\n"),
            ("x = [\r\n \t]\r\n", "x = []\r\n"),
            ("[ ?[ , ?] ]", "[?[ , ?]]"),
        ] {
            assert_correction(&SpaceInsideArrayLiteralBrackets, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
        let found = offenses("\u{feff}[ \"é\" ]\r\n");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 2), (1, 6)]
        );
    }

    #[test]
    fn pinned_and_synthetic_no_offense_lookalikes() {
        for source in [
            "",
            "[]",
            "[1, 2, 3]",
            "[foo, bar]",
            "[\"qux\", \"baz\"]",
            "[[1, 2], [3, 4]]",
            "[{ foo: 1 }, { bar: 2}]",
            "a[1]\nb[ 3]\nc[ foo ]\nd[index, 2]",
            "[1, 2, 3][0]",
            "[foo, bar][ 1]",
            "[\"qux\", \"baz\"][ -1 ]",
            "[[1, 2], [3, 4]][1 ]",
            "[2,3,4] + [5,6,7]",
            "stuff = [\n  a,\n  b\n]",
            "def do_stuff\n  a = [\n    1, 2\n    ]\nend",
            "a = [\n  1, 2, nil\n    ].compact",
            "a =\n\t[\n\t1, 2, nil\n\t].compact",
            "['Encoding:',\n '  Enabled: false']",
            "a = [ # Comment\n     1, 2\n    ]",
            "[\n# comment\n]",
            "[\x0b1\u{c}]",
            "def Vector.[](*array)\nend",
            "subject.[](0)",
            "case ary\nin a, b, c, d\nend",
            "case foo\nin [bar, baz]\nend",
            "case value\nin ADT[*head, ADT[*headhead, tail]]\nend",
            "def f(...); g(...); end",
            "def f(*); [*]; end",
            "def f(**); g(**); end",
            "# [ x ]\n",
            "=begin\n[ x ]\n=end\n",
            "__END__\n[ x ]\n",
            "'[ x ]'",
            "\"[ x ]\"",
            "/[ x ]/",
            "%q[ x ]",
            "%Q[ x ]",
            "%w[ x ]",
            "%W[ x ]",
            "%i[ x ]",
            "%I[ x ]",
            "%r[ x ]",
            "%x[ x ]",
            "%s[ x ]",
            ":\"[ x ]\"",
            "?[",
            "?]",
            "x = <<~H\n[ x ]\nH\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn literal_text_is_untouched_but_interpolation_contains_code() {
        for (before, after) in [
            ("\"[ x ] #{[ 1 ]}\"", "\"[ x ] #{[1]}\""),
            ("%W[[ x ]#{[ 1 ]}]", "%W[[ x ]#{[1]}]"),
            ("%I[[ x ]#{[ 1 ]}]", "%I[[ x ]#{[1]}]"),
            ("%r[[ x ]#{[ 1 ]}]", "%r[[ x ]#{[1]}]"),
            ("%x[[ x ]#{[ 1 ]}]", "%x[[ x ]#{[1]}]"),
            (
                "x = <<~H\n  [ x ]#{[ 1 ]}\nH\n",
                "x = <<~H\n  [ x ]#{[1]}\nH\n",
            ),
            ("[ 1 ]\n__END__\n[ x ]\n", "[1]\n__END__\n[ x ]\n"),
        ] {
            assert_eq!(offenses(before).len(), 2, "{before}");
            assert_correction(&SpaceInsideArrayLiteralBrackets, before, after);
        }
    }

    #[test]
    fn newline_removal_is_withheld_for_later_line_values() {
        for source in [
            "x = [\n]\np __LINE__\n",
            "x = [\n]\ndef f; __LINE__; end\n",
            "x = <<~H\n  #{[\n]}\nH\n",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&SpaceInsideArrayLiteralBrackets, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
            assert_correction(&SpaceInsideArrayLiteralBrackets, source, source);
        }
        for source in [
            "x = [\n] # rubocop:disable Style\n",
            "x = [\n] # rustocop:disable-next Style\n1\n",
        ] {
            assert_correction(
                &SpaceInsideArrayLiteralBrackets,
                source,
                &source.replacen("[\n]", "[]", 1),
            );
        }
    }

    #[test]
    fn pinned_comment_tokens_and_whitespace_literal_delimiters() {
        assert!(offenses("[ \n # Comment\n 1\n]").is_empty());
        for (before, after, columns) in [
            ("[ %q x  ]", "[%q x ]", vec![2, 7]),
            ("[%q x ]", "[%q x ]", vec![6]),
        ] {
            assert_eq!(
                offenses(before).iter().map(|o| o.col).collect::<Vec<_>>(),
                columns
            );
            assert_correction(&SpaceInsideArrayLiteralBrackets, before, after);
        }
    }

    #[test]
    fn pinned_additional_pattern_and_line_break_classifications() {
        // Pinned on_array_pattern does not include find patterns, and redirects
        // nested array patterns to their nearest constant-pattern brackets.
        for source in ["x in [ *, 1, * ]", "x in A[ a: b ]", "x in A( a, b )"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for (before, after, columns) in [
            ("[ \n 1 ]", "[\n 1]", vec![(1, 2), (2, 3)]),
            ("[ \\\n ]", "[]", vec![(1, 1)]),
            ("x in A( B[ a ] )", "x in A( B[a] )", vec![(1, 11), (1, 13)]),
            (
                "x in A[ [ a, b ] ]",
                "x in A[[ a, b ]]",
                vec![(1, 8), (1, 17)],
            ),
            (
                "x in A[ a: [ b ] ]",
                "x in A[a: [ b ]]",
                vec![(1, 8), (1, 17)],
            ),
            (
                "x in A( [ a, b ] )",
                "x in A( [a, b] )",
                vec![(1, 10), (1, 15)],
            ),
        ] {
            assert_eq!(
                offenses(before)
                    .iter()
                    .map(|o| (o.line, o.col))
                    .collect::<Vec<_>>(),
                columns,
                "{before}"
            );
            assert_correction(&SpaceInsideArrayLiteralBrackets, before, after);
        }
    }

    #[test]
    fn pinned_literals_redirect_to_enclosing_constant_patterns() {
        let before = "x in A(a: ^([ 1 ]))";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 14), (1, 16)]
        );
        assert!(
            found
                .iter()
                .all(|o| o.message == "Do not use space inside array brackets.")
        );
        assert_correction(
            &SpaceInsideArrayLiteralBrackets,
            before,
            "x in A(a: ^([1]))",
        );
        assert!(offenses("x in A(a: ^(%w[ a ]))").is_empty());
    }

    #[test]
    fn pinned_bracket_method_names_are_not_delimiters() {
        for (before, after, columns) in [
            (
                "x in A(^(obj.[](1)), [ a ])",
                "x in A(^(obj.[](1)), [a])",
                vec![23, 25],
            ),
            (
                "x in A(^(obj.[]=(1, 2)), [ a ])",
                "x in A(^(obj.[]=(1, 2)), [a])",
                vec![27, 29],
            ),
            (
                "x in A(^(def obj.[](i); end), [ a ])",
                "x in A(^(def obj.[](i); end), [a])",
                vec![32, 34],
            ),
            (
                "x in A(^(def []=(i, v); end), [ a ])",
                "x in A(^(def []=(i, v); end), [a])",
                vec![32, 34],
            ),
        ] {
            assert_eq!(
                offenses(before).iter().map(|o| o.col).collect::<Vec<_>>(),
                columns,
                "{before}"
            );
            assert_correction(&SpaceInsideArrayLiteralBrackets, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
    }

    #[test]
    fn pinned_overlapping_constant_patterns_report_each_side_once() {
        for (before, after, columns) in [
            (
                "x in A([ a ], B[ b ])",
                "x in A([a ], B[b])",
                vec![9, 17, 19],
            ),
            (
                "x in A(B[ a ], [ b ])",
                "x in A(B[a], [ b])",
                vec![10, 12, 19],
            ),
            ("x in A(B(a: C[ d ]))", "x in A(B(a: C[d]))", vec![15, 17]),
        ] {
            assert_eq!(
                offenses(before).iter().map(|o| o.col).collect::<Vec<_>>(),
                columns,
                "{before}"
            );
            assert_correction(&SpaceInsideArrayLiteralBrackets, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
    }

    #[test]
    fn pinned_block_comment_exempts_opening_gap() {
        for source in [
            "[ \n=begin\ncomment\n=end\n 1\n]\n",
            "[ \r\n=begin\r\ncomment\r\n=end\r\n 1\r\n]\r\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
            assert!(
                crate::cops::testing::correction_offenses(&SpaceInsideArrayLiteralBrackets, source)
                    .is_empty()
            );
        }
        assert_correction(
            &SpaceInsideArrayLiteralBrackets,
            "[ \n=begin\ncomment\n=end\n 1 ]\n",
            "[ \n=begin\ncomment\n=end\n 1]\n",
        );
    }

    #[test]
    fn pinned_exact_nonempty_and_empty_offenses() {
        let before = "[ 2, 3, 4]\n[b, c, d   ]\na = [ ]\na = [     ]\na = [\n]\n";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 2), (2, 9), (3, 5), (4, 5), (5, 5)]
        );
        assert_eq!(found[4].end_line, 6);
        for (i, o) in found.iter().enumerate() {
            assert_eq!(
                o.message,
                if i < 2 {
                    "Do not use space inside array brackets."
                } else {
                    "Do not use space inside empty array brackets."
                }
            );
            assert_eq!(o.severity, crate::offense::Severity::Convention);
        }
        assert_correction(
            &SpaceInsideArrayLiteralBrackets,
            before,
            "[2, 3, 4]\n[b, c, d]\na = []\na = []\na = []\n",
        );
    }
}
