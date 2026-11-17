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
                    let text = std::str::from_utf8(loc.as_slice()).ok()?;
                    let start = loc.start_offset();
                    let single = file.line_of(start) == file.last_line_of(hash.location());
                    let same_line =
                        file.line_of(node.location().start_offset()) == file.line_of(start);
                    let replacement = if single || same_line {
                        format!("{{{text}}}")
                    } else {
                        let line_start = file.line_start(file.line_of(start));
                        let indent = " ".repeat(
                            std::str::from_utf8(&file.bytes[line_start..start])
                                .ok()?
                                .chars()
                                .count(),
                        );
                        format!("{{\n{indent}{text}\n{indent}}}")
                    };
                    Some(crate::offense::Correction {
                        range: start..loc.end_offset(),
                        replacement,
                    })
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
