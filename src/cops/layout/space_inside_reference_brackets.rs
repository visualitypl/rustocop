//! `Layout/SpaceInsideReferenceBrackets` — fixed `no_space` for indexing.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct SpaceInsideReferenceBrackets;
impl Cop for SpaceInsideReferenceBrackets {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideReferenceBrackets"
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
        let delimiters = match node {
            Node::CallNode { .. } => {
                let n = node.as_call_node().unwrap();
                if !matches!(n.name().as_slice(), b"[]" | b"[]=") || n.call_operator_loc().is_some()
                {
                    return;
                }
                (n.opening_loc(), n.closing_loc())
            }
            Node::IndexAndWriteNode { .. } => {
                let n = node.as_index_and_write_node().unwrap();
                (Some(n.opening_loc()), Some(n.closing_loc()))
            }
            Node::IndexOrWriteNode { .. } => {
                let n = node.as_index_or_write_node().unwrap();
                (Some(n.opening_loc()), Some(n.closing_loc()))
            }
            Node::IndexOperatorWriteNode { .. } => {
                let n = node.as_index_operator_write_node().unwrap();
                (Some(n.opening_loc()), Some(n.closing_loc()))
            }
            Node::IndexTargetNode { .. } => {
                let n = node.as_index_target_node().unwrap();
                (Some(n.opening_loc()), Some(n.closing_loc()))
            }
            _ => return,
        };
        super::bracket_spacing::check(
            self,
            node,
            cx,
            file,
            super::bracket_spacing::offsets(delimiters),
            super::bracket_spacing::Kind::Reference,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/space_inside_reference_brackets_spec.rb`
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
            .filter(|o| o.cop == SpaceInsideReferenceBrackets.name())
            .collect()
    }

    #[test]
    fn pinned_correction_examples_and_write_forms() {
        for (before, after) in [
            (
                "a[ \"foo\"] = b[\"something\"]",
                "a[\"foo\"] = b[\"something\"]",
            ),
            (
                "a[\"foo\" ] = b[\"something\"]",
                "a[\"foo\"] = b[\"something\"]",
            ),
            (
                "a[\"foo\"] = b[\"something\" ]",
                "a[\"foo\"] = b[\"something\"]",
            ),
            ("a[:key][ \"key\"]", "a[:key][\"key\"]"),
            ("a[1][:key   ]", "a[1][:key]"),
            ("a[:key][3][ :key]", "a[:key][3][:key]"),
            ("a[var][\"key\", 3][:key ]", "a[var][\"key\", 3][:key]"),
            ("b[ 89  ]", "b[89]"),
            ("a[ :key][\"foo\"  ][   0 ]", "a[:key][\"foo\"][0]"),
            (
                "record[ options[:attribute] ]",
                "record[options[:attribute]]",
            ),
            ("foo[ bar[0][1] ]", "foo[bar[0][1]]"),
            (
                "j[ \"pop\"] = [89, nil, \"\"    ]",
                "j[\"pop\"] = [89, nil, \"\"    ]",
            ),
            ("a[ [ 1, 2 ] ]", "a[[ 1, 2 ]]"),
            ("a[ 1, 2..3, *indexes ]", "a[1, 2..3, *indexes]"),
            ("def f(*); a[ * ]; end", "def f(*); a[*]; end"),
            ("def f(**); a[ ** ]; end", "def f(**); a[**]; end"),
            ("def f(&); a[ & ]; end", "def f(&); a[&]; end"),
            ("a[ ..3 ]; a[ 3.. ]; a[ 1...3 ]", "a[..3]; a[3..]; a[1...3]"),
            (
                "a[ <<~H ]\n  [ literal ]\nH\n",
                "a[<<~H]\n  [ literal ]\nH\n",
            ),
            ("a[\t1\t]", "a[1]"),
            ("\u{feff}a[ \"é\" ]\r\n", "\u{feff}a[\"é\"]\r\n"),
            ("a[\r\n \t]\r\n", "a[]\r\n"),
            ("\"a[ x ] #{a[ 1 ]}\"", "\"a[ x ] #{a[1]}\""),
            ("%W[a[ x ]#{a[ 1 ]}]", "%W[a[ x ]#{a[1]}]"),
            (
                "x = <<~H\n  a[ x ]#{a[ 1 ]}\nH\n",
                "x = <<~H\n  a[ x ]#{a[1]}\nH\n",
            ),
            ("a[ 1 ]\n__END__\na[ x ]\n", "a[1]\n__END__\na[ x ]\n"),
        ] {
            assert_correction(&SpaceInsideReferenceBrackets, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
        for op in ["=", "+=", "||=", "&&="] {
            assert_correction(
                &SpaceInsideReferenceBrackets,
                &format!("a[ 1 ] {op} 2"),
                &format!("a[1] {op} 2"),
            );
        }
        assert_correction(
            &SpaceInsideReferenceBrackets,
            "a[ 1 ], b[ 2 ] = 3, 4",
            "a[1], b[2] = 3, 4",
        );
        let found = offenses("\u{feff}a[ \"é\" ]\r\n");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 3), (1, 7)]
        );
    }

    #[test]
    fn pinned_multiline_write_rhs_is_only_part_of_plain_setter_sends() {
        assert!(offenses("a[ 1 ] =\n 2").is_empty());
        for operator in ["+=", "||=", "&&="] {
            assert_correction(
                &SpaceInsideReferenceBrackets,
                &format!("a[ 1 ] {operator}\n 2"),
                &format!("a[1] {operator}\n 2"),
            );
        }
        assert_correction(
            &SpaceInsideReferenceBrackets,
            "a[ 1 ], b =\n 1, 2",
            "a[1], b =\n 1, 2",
        );
    }

    #[test]
    fn pinned_and_synthetic_no_offense_lookalikes_and_multiline_sends() {
        for source in [
            "",
            "a[]",
            "a[1]\nb[index, 2]\nc[\"foo\"]\nd[:bar]\ne[]",
            "a[1] = 2\nb[345] = [ 678, var, \"\", nil]\nc[\"foo\"] = \"qux\"\nd[:bar] = var\ne[] = foo",
            "a = [1, 2 ]\nb = [ 3, 4]\nc = [5, 6]\nd = [ 7, 8 ]",
            "a[[ 1, 2 ]]",
            "foo[\n  bar\n]",
            "a[ 1\n]",
            "a[\n 1 ]",
            "a[ 1 ] =\n  2",
            "a\n .b[ 1 ]",
            "def Vector.[](*array)\nend",
            "subject.[](0)",
            "subject.[]=(0, 1)",
            "subject&.[]( 0 )",
            "a[\n # comment\n]",
            "a[\x0b1\u{c}]",
            "x in [ a, b ]",
            "x in A[ a, b ]",
            "def f(...); g(...); end",
            "def f(*); a[*]; end",
            "# a[ x ]\n",
            "=begin\na[ x ]\n=end\n",
            "__END__\na[ x ]\n",
            "'a[ x ]'",
            "\"a[ x ]\"",
            "/a[ x ]/",
            "%q[a[ x ]]",
            "%w[a[ x ]]",
            "%i[a[ x ]]",
            "x = <<~H\na[ x ]\nH\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn unsafe_empty_multiline_and_literal_delimiter_fixes_are_withheld() {
        for source in [
            "a[\n]\np __LINE__\n",
            "a[\n] # rubocop:disable Style\n",
            "x = <<~H\n  #{a[\n]}\nH\n",
            "a[%q x ]",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&SpaceInsideReferenceBrackets, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
            assert_correction(&SpaceInsideReferenceBrackets, source, source);
        }
        assert_correction(&SpaceInsideReferenceBrackets, "a[ %q x  ]", "a[%q x ]");
    }

    #[test]
    fn pinned_empty_continuations_and_literal_blocks() {
        for (before, after) in [("a[ \\\n ]", "a[]"), ("a[ 1 ] {\n x\n}", "a[1] {\n x\n}")] {
            assert_correction(&SpaceInsideReferenceBrackets, before, after);
        }
    }

    #[test]
    fn pinned_exact_nonempty_and_empty_offenses() {
        let before = "a[  :key]\nb[:key ]\nfoo[ ]\na[     ]\na[\n]\n";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 3), (2, 7), (3, 4), (4, 2), (5, 2)]
        );
        assert_eq!(found[4].end_line, 6);
        for (i, o) in found.iter().enumerate() {
            assert_eq!(
                o.message,
                if i < 2 {
                    "Do not use space inside reference brackets."
                } else {
                    "Do not use space inside empty reference brackets."
                }
            );
            assert_eq!(o.severity, crate::offense::Severity::Convention);
        }
        assert_correction(
            &SpaceInsideReferenceBrackets,
            before,
            "a[:key]\nb[:key]\nfoo[]\na[]\na[]\n",
        );
    }
}
