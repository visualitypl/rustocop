//! `Layout/SpaceInsideArrayLiteralBrackets` — spaces inside the brackets of
//! array literals and patterns. `enforced_style` is RuboCop's
//! `EnforcedStyle`: `no_space` (the default), `space`, or `compact` (spaces,
//! except between the brackets of nested arrays: `[[ 1 ]]`).
//! `enforced_style_for_empty_brackets` is `EnforcedStyleForEmptyBrackets`:
//! `no_space` (the default, `[]`) or `space` (`[ ]`).
use crate::{
    cops::{
        Cop, NodeContext,
        settings::{Kind, Setting, Values},
    },
    offense::{Correction, Offense},
    source::{Fact, SourceFile, tokens::Token},
};
use ruby_prism::Node;

#[derive(Default)]
pub struct SpaceInsideArrayLiteralBrackets {
    style: Style,
    /// `enforced_style_for_empty_brackets = "space"`.
    empty_space: bool,
}

/// `enforced_style`.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Style {
    #[default]
    NoSpace,
    Space,
    Compact,
}

impl Cop for SpaceInsideArrayLiteralBrackets {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideArrayLiteralBrackets"
    }
    fn settings(&self) -> &'static [Setting] {
        const ENFORCED_STYLE: Setting = Setting {
            name: "enforced_style",
            kind: Kind::Choice(&["no_space", "space", "compact"]),
        };
        const ENFORCED_STYLE_FOR_EMPTY_BRACKETS: Setting = Setting {
            name: "enforced_style_for_empty_brackets",
            kind: Kind::Choice(&["no_space", "space"]),
        };
        &[ENFORCED_STYLE, ENFORCED_STYLE_FOR_EMPTY_BRACKETS]
    }
    fn configure(&self, values: &Values) -> Option<Box<dyn Cop>> {
        let style = match values.choice("enforced_style") {
            Some("space") => Style::Space,
            Some("compact") => Style::Compact,
            Some(_) => Style::NoSpace,
            None => self.style,
        };
        let empty_space = match values.choice("enforced_style_for_empty_brackets") {
            Some(style) => style == "space",
            None => self.empty_space,
        };
        Some(Box::new(SpaceInsideArrayLiteralBrackets {
            style,
            empty_space,
        }))
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
        if self.style != Style::NoSpace
            && let Some((left, right)) = delimiters
            && !super::bracket_spacing::empty_gap(&file.bytes[left + 1..right])
        {
            let brackets = Brackets {
                node,
                left,
                right,
                skip_left,
                skip_right,
            };
            self.check_spaced(&brackets, cx, file, out);
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
                empty_space: self.empty_space,
            },
            out,
        );
    }
}

/// The brackets of one array (or of the constant pattern it is in) that
/// a non-empty `space` or `compact` check looks at. `skip_left` and
/// `skip_right`: a side that an enclosing pattern reports.
struct Brackets<'a, 'pr> {
    node: &'a Node<'pr>,
    left: usize,
    right: usize,
    skip_left: bool,
    skip_right: bool,
}

const USE: &str = "Use space inside array brackets.";
const DO_NOT_USE: &str = "Do not use space inside array brackets.";

/// `processed_source.sorted_tokens`: the indexes of the parser tokens, in a
/// stable sort by start.
static SORTED_TOKENS: Fact<Vec<usize>> = Fact::new();

impl SpaceInsideArrayLiteralBrackets {
    /// RuboCop's `space_offenses` and `compact_offenses`, on the tokens
    /// within the node, with the corrector of `autocorrect`: one correction
    /// for both brackets, on the first offense.
    fn check_spaced(
        &self,
        brackets: &Brackets<'_, '_>,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let all = file.parser_tokens();
        let order = SORTED_TOKENS.get(file, || {
            let mut order: Vec<usize> = (0..all.len()).collect();
            order.sort_by_key(|&i| all[i].start);
            order
        });
        // `tokens_within(node)`.
        let location = brackets.node.location();
        let first = order.partition_point(|&i| all[i].start < location.start_offset());
        let last = order.partition_point(|&i| all[i].end < location.end_offset());
        let Some(tokens) = order.get(first..=last.min(order.len().saturating_sub(1))) else {
            return;
        };
        let tokens: Vec<&Token> = tokens.iter().map(|&i| &all[i]).collect();
        let (Some(l), Some(r)) = (
            tokens.iter().position(|t| t.start == brackets.left),
            tokens.iter().position(|t| t.start == brackets.right),
        ) else {
            return;
        };
        let bytes = &file.bytes;
        let (left, right) = (brackets.left, brackets.right);
        let at = |offset: usize| bytes.get(offset).copied().unwrap_or(0);
        // `extra_space?`, then `space_after?` / `space_before?`.
        let extra_after_left = blank(at(left + 1));
        let extra_before_right = blank(at(right - 1));
        let space_after_left = whitespace(at(left + 1));
        let space_before_right = whitespace(at(right - 1));
        let single_line = file.line_of(location.start_offset()) == file.last_line_of(location);
        // `next_to_newline?`: the token after `[` is on another line.
        let start_ok = tokens
            .get(l + 1)
            .is_some_and(|next| next.line != tokens[l].line);
        // `end_has_own_line?`: nothing but whitespace before `]` on its line.
        let line_start = file.line_start(file.line_of(right));
        let own_line = bytes[line_start..right].iter().all(|&b| whitespace(b));
        let end_ok = !single_line && own_line;
        // `multi_dimensional_array?`: the nearest token inward, past newline
        // tokens, is a bracket of the same side.
        let nested_left = tokens[l + 1..]
            .iter()
            .find(|t| !t.is_newline())
            .is_some_and(|t| matches!(t.kind, "tLBRACK" | "tLBRACK2"));
        let nested_right = tokens[..r]
            .iter()
            .rev()
            .find(|t| !t.is_newline())
            .is_some_and(|t| t.kind == "tRBRACK");
        let sides = Sides {
            space_after_left,
            space_before_right,
            nested_left,
            nested_right,
            kept_apart: own_line && !nested_left && !first_child_on_line(brackets.node, file, left),
        };

        let mut found: Vec<(std::ops::Range<usize>, &str)> = Vec::new();
        if self.style == Style::Space {
            if !(extra_after_left || start_ok) {
                found.push((left..left + 1, USE));
            }
            if !(extra_before_right || end_ok) {
                found.push((right..right + 1, USE));
            }
        } else {
            // `compact_offense`: the blanks after `[` (before `]`), which are
            // none when a line break follows (precedes) the bracket.
            if nested_left && space_after_left {
                let end = left + 1 + bytes[left + 1..].iter().take_while(|&&b| blank(b)).count();
                found.push((left + 1..end, DO_NOT_USE));
            } else if !nested_left && !(extra_after_left || start_ok) {
                found.push((left..left + 1, USE));
            }
            if !sides.kept_apart {
                if nested_right && space_before_right {
                    let start = right
                        - bytes[..right]
                            .iter()
                            .rev()
                            .take_while(|&&b| blank(b))
                            .count();
                    found.push((start..right, DO_NOT_USE));
                } else if !nested_right && !(extra_before_right || end_ok) {
                    found.push((right..right + 1, USE));
                }
            }
        }
        // An enclosing pattern reports a side that it shares.
        found.retain(|(range, _)| {
            let on_left = range.start <= left + 1;
            !(on_left && brackets.skip_left || !on_left && brackets.skip_right)
        });
        let mut correction = cx
            .corrections_requested()
            .then(|| self.correction(file, left, right, &sides))
            .flatten();
        for (range, message) in found {
            let (line, col) = file.line_col(range.start);
            out.push(Offense {
                correction: correction.take(),
                cop: self.name(),
                severity: self.severity(),
                message: message.to_owned(),
                line,
                end_line: file.line_of(range.end.max(range.start + 1) - 1),
                col,
            });
        }
    }

    /// RuboCop's `autocorrect` for the `space` (`SpaceCorrector.add_space`)
    /// and `compact` (`compact_corrections`) styles.
    fn correction(
        &self,
        file: &SourceFile,
        left: usize,
        right: usize,
        sides: &Sides,
    ) -> Option<Correction> {
        let bytes = &file.bytes;
        // `side_space_range(include_newlines: true)`.
        let gap = |b: u8| matches!(b, b' ' | b'\t' | b'\n');
        let compact = self.style == Style::Compact;
        let mut edits = Vec::new();
        if compact && sides.nested_left {
            let end = left + 1 + bytes[left + 1..].iter().take_while(|&&b| gap(b)).count();
            edits.push((left + 1..end, String::new()));
        } else if !sides.space_after_left {
            edits.push((left + 1..left + 1, " ".to_owned()));
        }
        if compact && sides.kept_apart {
            // The closing bracket stays on its own line.
        } else if compact && sides.nested_right {
            let start = right - bytes[..right].iter().rev().take_while(|&&b| gap(b)).count();
            edits.push((start..right, String::new()));
        } else if !sides.space_before_right {
            edits.push((right..right, " ".to_owned()));
        }
        Correction::from_edits(bytes, edits)
    }
}

/// What the corrector of the `space` and `compact` styles reads about the
/// two brackets.
struct Sides {
    /// `space_after?` of `[` and `space_before?` of `]`: any whitespace.
    space_after_left: bool,
    space_before_right: bool,
    /// `multi_dimensional_array?` on each side.
    nested_left: bool,
    nested_right: bool,
    /// `closing_bracket_kept_apart?`: in the `compact` style, a `]` on its
    /// own line stays there unless `[` is on the line of the first element
    /// or opens a nested array.
    kept_apart: bool,
}

/// RuboCop's `SINGLE_SPACE_REGEXP`.
fn blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t')
}

/// Ruby's `\s`.
fn whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

/// `same_line?(left, node.children.first)`: false without a first child.
fn first_child_on_line(node: &Node<'_>, file: &SourceFile, left: usize) -> bool {
    first_child(node).is_some_and(|child| file.line_of(child) == file.line_of(left))
}

/// The start of `node.children.first` in the parser gem: the first element
/// of an array or array pattern, the constant of a constant pattern.
fn first_child(node: &Node<'_>) -> Option<usize> {
    let start = |n: Node<'_>| n.location().start_offset();
    match node {
        Node::ArrayNode { .. } => node
            .as_array_node()
            .unwrap()
            .elements()
            .iter()
            .next()
            .map(start),
        Node::ArrayPatternNode { .. } => {
            let n = node.as_array_pattern_node().unwrap();
            if let Some(constant) = n.constant() {
                return Some(start(constant));
            }
            n.requireds()
                .iter()
                .chain(n.rest())
                .chain(n.posts().iter())
                .map(start)
                .min()
        }
        Node::FindPatternNode { .. } => node.as_find_pattern_node().unwrap().constant().map(start),
        Node::HashPatternNode { .. } => node.as_hash_pattern_node().unwrap().constant().map(start),
        _ => None,
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
            .filter(|o| o.cop == SpaceInsideArrayLiteralBrackets::default().name())
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
            assert_correction(&SpaceInsideArrayLiteralBrackets::default(), before, after);
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
            assert_correction(&SpaceInsideArrayLiteralBrackets::default(), before, after);
        }
    }

    #[test]
    fn newline_removal_is_withheld_for_later_line_values() {
        for source in [
            "x = [\n]\np __LINE__\n",
            "x = [\n]\ndef f; __LINE__; end\n",
            "x = <<~H\n  #{[\n]}\nH\n",
        ] {
            let found = crate::cops::testing::correction_offenses(
                &SpaceInsideArrayLiteralBrackets::default(),
                source,
            );
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
            assert_correction(&SpaceInsideArrayLiteralBrackets::default(), source, source);
        }
        for source in [
            "x = [\n] # rubocop:disable Style\n",
            "x = [\n] # rustocop:disable-next Style\n1\n",
        ] {
            assert_correction(
                &SpaceInsideArrayLiteralBrackets::default(),
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
            assert_correction(&SpaceInsideArrayLiteralBrackets::default(), before, after);
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
            assert_correction(&SpaceInsideArrayLiteralBrackets::default(), before, after);
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
            &SpaceInsideArrayLiteralBrackets::default(),
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
            assert_correction(&SpaceInsideArrayLiteralBrackets::default(), before, after);
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
            assert_correction(&SpaceInsideArrayLiteralBrackets::default(), before, after);
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
                crate::cops::testing::correction_offenses(
                    &SpaceInsideArrayLiteralBrackets::default(),
                    source
                )
                .is_empty()
            );
        }
        assert_correction(
            &SpaceInsideArrayLiteralBrackets::default(),
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
            &SpaceInsideArrayLiteralBrackets::default(),
            before,
            "[2, 3, 4]\n[b, c, d]\na = []\na = []\na = []\n",
        );
    }
}
