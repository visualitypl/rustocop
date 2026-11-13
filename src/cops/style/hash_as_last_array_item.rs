//! `Style/HashAsLastArrayItem` — fixed explicit braces for a trailing array hash.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct HashAsLastArrayItem;
impl Cop for HashAsLastArrayItem {
    fn name(&self) -> &'static str {
        "Style/HashAsLastArrayItem"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_array_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Inspect the array once, not once per hash/pair or by rescanning siblings.
        // A bare final hash means the upstream all-braced-hashes exemption is
        // already false. The penultimate-hash exclusion also covers mixed hashes.
        let Some(array) = node.as_array_node() else {
            return;
        };
        if array.opening_loc().is_none_or(|l| l.as_slice() != b"[") {
            return;
        }
        let elements = array.elements();
        let Some(last) = elements.last() else {
            return;
        };
        let Some(hash) = last.as_keyword_hash_node() else {
            return;
        };
        if hash
            .elements()
            .first()
            .is_some_and(|n| n.as_assoc_splat_node().is_some())
            || elements
                .len()
                .checked_sub(2)
                .and_then(|i| elements.iter().nth(i))
                .is_some_and(|n| n.as_hash_node().is_some() || n.as_keyword_hash_node().is_some())
        {
            return;
        }
        let loc = hash.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Wrap hash in `{` and `}`.".into(),
            line,
            col,
            end_line: file.last_line_of(hash.location()),
            correction: cx
                .corrections_requested()
                .then(|| {
                    // Ruby's shareability compiler treats keyword hashes and
                    // braced HashNodes differently (even for literal values).
                    // Preserve each protected assignment in every non-none mode;
                    // Prism already supplies the pragma/scope/write semantics.
                    // Ancestors are bounded by the pipeline's AST depth budget.
                    if cx
                        .ancestors()
                        .any(|n| matches!(n, Node::ShareableConstantNode { .. }))
                    {
                        return None;
                    }
                    // Keep all original operand/trivia bytes and physical lines. In
                    // particular, never insert upstream's two multiline newlines:
                    // those change __LINE__, error locations and deferred bodies.
                    let text = std::str::from_utf8(loc.as_slice()).ok()?;
                    super::literal_spelling::safe_edit(
                        file,
                        cx,
                        crate::offense::Correction {
                            range: loc.start_offset()..loc.end_offset(),
                            replacement: format!("{{{text}}}"),
                        },
                    )
                })
                .flatten(),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop hash_as_last_array_item_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == HashAsLastArrayItem.name())
            .collect()
    }
    #[test]
    fn shareable_literal_assignment_keeps_offense_but_only_fixes_ordinary_sibling() {
        let before = "# shareable_constant_value: literal\nC=[a:1]\nx=[b:2]\n";
        let found = crate::cops::testing::correction_offenses(&HashAsLastArrayItem, before);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col), (2, 4));
        assert_eq!(found[0].message, "Wrap hash in `{` and `}`.");
        assert!(found[0].correction.is_none());
        assert!(found[1].correction.is_some());
        assert_correction(
            &HashAsLastArrayItem,
            before,
            &before.replace("x=[b:2]", "x=[{b:2}]"),
        );
    }
    #[test]
    fn shareable_modes_cover_all_eight_typed_constant_writes_and_nested_values() {
        for mode in [
            "literal",
            "experimental_copy",
            "experimental_everything",
            "none",
        ] {
            for lhs in ["C", "::C", "M::C", "::M::C"] {
                for op in ["=", "||=", "&&=", "+="] {
                    let s =
                        format!("# shareable_constant_value: {mode}\n{lhs}{op}[a:1]\nx=[b:2]\n");
                    let found = crate::cops::testing::correction_offenses(&HashAsLastArrayItem, &s);
                    assert_eq!(found.len(), 2, "{s}");
                    assert_eq!(found[0].correction.is_some(), mode == "none", "{s}");
                    assert!(found[1].correction.is_some(), "{s}");
                }
            }
            for rhs in ["begin; [a:1]; end", "foo([a:1])", "[a:[nested:1]]"] {
                let s = format!("# shareable_constant_value: {mode}\nC={rhs}\n");
                let found = crate::cops::testing::correction_offenses(&HashAsLastArrayItem, &s);
                assert_eq!(
                    found.len(),
                    if rhs.contains("nested") { 2 } else { 1 },
                    "{s}"
                );
                assert!(
                    found
                        .iter()
                        .all(|o| o.correction.is_some() == (mode == "none")),
                    "{s}"
                );
            }
        }
    }
    #[test]
    fn shareable_scope_resets_multiple_assignment_and_unprotected_controls() {
        let s = "# shareable_constant_value: literal\nC=[a:1]\nmodule M\n C=[a:1]\n # shareable_constant_value: none\n D=[b:2]\nend\nE=[a:1]\n# shareable_constant_value: none\nF=[b:2]\nclass K\n # shareable_constant_value: literal\n C=[a:1]\nend\nG=[b:2]\n";
        assert_correction(&HashAsLastArrayItem, s, &s.replace("[b:2]", "[{b:2}]"));
        for mode in [
            "literal",
            "experimental_copy",
            "experimental_everything",
            "none",
        ] {
            // Multiple-assignment targets are not protected constant writes in
            // Prism/native Ruby; don't infer protection from spelling a constant.
            let s = format!("# shareable_constant_value: {mode}\nA,M::B=[a:1],[b:2]\nx=[c:3]\n");
            let after = s
                .replace("[a:1]", "[{a:1}]")
                .replace("[b:2]", "[{b:2}]")
                .replace("[c:3]", "[{c:3}]");
            assert_correction(&HashAsLastArrayItem, &s, &after);
        }
        for prefix in [
            "",
            "# shareable_constant_value: none\n",
            "# shareable_constant_value: invalid\n",
            "s = '# shareable_constant_value: literal'\n",
        ] {
            let s = format!("{prefix}C=[a:1]\n");
            assert_correction(&HashAsLastArrayItem, &s, &s.replace("[a:1]", "[{a:1}]"));
        }
    }
    #[test]
    fn pinned_diagnostics_and_all_four_correction_calls() {
        for (before, after, line, col, end_line) in [
            (
                "[1, 2, one: 1, two: 2]\n",
                "[1, 2, {one: 1, two: 2}]\n",
                1,
                8,
                1,
            ),
            ("[one: 1, two: 2]\n", "[{one: 1, two: 2}]\n", 1, 2, 1),
            // Physical-line-preserving adaptation: upstream adds two newlines.
            (
                "[\n  one: 1,\n  two: 2\n]\n",
                "[\n  {one: 1,\n  two: 2}\n]\n",
                2,
                3,
                3,
            ),
            ("[one: 1,\n two: 2]\n", "[{one: 1,\n two: 2}]\n", 1, 2, 2),
        ] {
            let o = offenses(before);
            assert_eq!(o.len(), 1, "{before}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (line, col, end_line));
            assert_eq!(o[0].message, "Wrap hash in `{` and `}`.");
            assert_correction(&HashAsLastArrayItem, before, after);
            assert!(offenses(after).is_empty());
        }
    }
    #[test]
    fn significant_operand_bytes_comments_omitted_values_and_later_splats() {
        for (before, after) in [
            ("[a: 1,]", "[{a: 1},]"),
            (
                "[1, \"a\" => value, :a => other]",
                "[1, {\"a\" => value, :a => other}]",
            ),
            (
                "[effect() => value(), **options]",
                "[{effect() => value(), **options}]",
            ),
            ("[one: 1, **nil]", "[{one: 1, **nil}]"),
            ("[one: 1, **{}]", "[{one: 1, **{}}]"),
            ("one = 1; [one:]", "one = 1; [{one:}]"),
            ("[key: %q( foo\\ )]", "[{key: %q( foo\\ )}]"),
            (
                "[\n # before\n key: 1, # pair\n other: 2 # after\n]\n",
                "[\n # before\n {key: 1, # pair\n other: 2} # after\n]\n",
            ),
            (
                "[a: __LINE__,\r\nb: __LINE__]\r\np __LINE__\r\n",
                "[{a: __LINE__,\r\nb: __LINE__}]\r\np __LINE__\r\n",
            ),
            (
                "[1,\n two: <<~DOC]\nhi\nDOC\n",
                "[1,\n {two: <<~DOC}]\nhi\nDOC\n",
            ),
            (
                "x=<<~DOC; y=[a: 1]\nbody\nDOC\n",
                "x=<<~DOC; y=[{a: 1}]\nbody\nDOC\n",
            ),
        ] {
            assert_correction(&HashAsLastArrayItem, before, after);
        }
    }
    #[test]
    fn commands_and_nested_interpolations_are_source_only_and_keep_operand_bytes() {
        for (s, a) in [
            ("[a: `never execute this`]", "[{a: `never execute this`}]"),
            ("[a: %x(never execute)]", "[{a: %x(never execute)}]"),
            (
                "[a: <<~`CMD`]\nnever execute\nCMD\n",
                "[{a: <<~`CMD`}]\nnever execute\nCMD\n",
            ),
            ("[a: /#{[inner: 1]}/o]", "[{a: /#{[inner: 1]}/o}]"),
        ] {
            assert_correction(&HashAsLastArrayItem, s, a);
        }
    }
    #[test]
    fn original_source_columns_include_bom_and_multibyte_prefixes() {
        for (s, line, col) in [
            ("\u{feff}[a: 1]", 1, 2),
            ("\"é\"; [a: 1]", 1, 7),
            ("# header\r\n  [é: 1, a: 2]\r\n", 2, 4),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (line, col), "{s}");
        }
    }
    #[test]
    fn pinned_negatives_and_first_kwsplat_exemption() {
        for s in [
            "[1, 2, { one: 1, two: 2 }]",
            "foo(one: 1, two: 2)",
            "[{ one: 1 }, { two: 2 }]",
            "[1, {}]",
            "[1, **options]",
            "[**nil]",
            "[**{}]",
            "[**options, one: 1]",
            "[{}, one: 1]",
            "[1, {}, one: 1]",
            "cache_store = :redis, {url: value}",
            "a = 1, one: 1",
            "[]",
            "%w[one: two:]",
            "[{one: 1}, 2]",
            "foo(one: 1)",
            "foo[one: 1]",
            "foo([**options])",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
