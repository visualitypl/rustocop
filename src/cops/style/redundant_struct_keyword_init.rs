//! `Style/RedundantStructKeywordInit` — opt-in, Ruby >= 3.2; no unsafe edits.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantStructKeywordInit;
impl Cop for RedundantStructKeywordInit {
    fn name(&self) -> &'static str {
        "Style/RedundantStructKeywordInit"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"new"
            || !call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Struct"]))
        {
            return;
        }
        let arguments = ast::call_arguments(&call);
        let Some(last) = arguments.last() else {
            return;
        };
        let elements = if let Some(hash) = last.as_keyword_hash_node() {
            hash.elements()
        } else if let Some(hash) = last.as_hash_node() {
            hash.elements()
        } else {
            return;
        };
        let pairs: Vec<_> = elements
            .iter()
            .filter_map(|n| n.as_assoc_node())
            .filter(|pair| {
                pair.key()
                    .as_symbol_node()
                    .is_some_and(|s| s.unescaped() == b"keyword_init")
            })
            .collect();
        if pairs
            .iter()
            .any(|pair| pair.value().as_false_node().is_some())
        {
            return;
        }
        for pair in pairs {
            let value = pair.value();
            if !matches!(value, Node::TrueNode { .. } | Node::NilNode { .. }) {
                continue;
            }
            let value_source = String::from_utf8_lossy(value.location().as_slice());
            let (line, col) = file.line_col(pair.location().start_offset());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                correction: None,
                message: format!("Remove the redundant `keyword_init: {value_source}`."),
                line,
                col,
                end_line: file.last_line_of(pair.location()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_struct_keyword_init_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;

    #[test]
    fn pinned_keyword_pairs_and_locations_without_unsafe_corrections() {
        for (source, expected) in [
            ("Struct.new(:foo, keyword_init: nil)", vec![(18, "nil")]),
            ("Struct.new(:foo, keyword_init: true)", vec![(18, "true")]),
            (
                "Struct.new(a: 1, keyword_init: nil, b: 2)",
                vec![(18, "nil")],
            ),
            ("Struct.new(keyword_init: true, a: 1)", vec![(12, "true")]),
            ("Struct.new(keyword_init: true)", vec![(12, "true")]),
            ("Struct&.new(:foo, keyword_init: true)", vec![(19, "true")]),
            ("::Struct.new(:foo, keyword_init: true)", vec![(20, "true")]),
            (
                "Struct.new(:foo, keyword_init: true, keyword_init: true)",
                vec![(18, "true"), (38, "true")],
            ),
            (
                "Struct.new(:foo, keyword_init: nil, keyword_init: true)",
                vec![(18, "nil"), (37, "true")],
            ),
            (
                "Struct.new(:foo, { :keyword_init => true })",
                vec![(20, "true")],
            ),
            (
                "Struct.new(:foo, **opts, keyword_init: nil) { action }",
                vec![(26, "nil")],
            ),
        ] {
            let found = correction_offenses(&RedundantStructKeywordInit, source);
            assert_eq!(found.len(), expected.len(), "{source}");
            for (o, (col, value)) in found.iter().zip(expected) {
                assert_eq!((o.line, o.col, o.end_line), (1, col, 1), "{source}");
                assert_eq!(
                    o.message,
                    format!("Remove the redundant `keyword_init: {value}`.")
                );
                assert!(o.correction.is_none());
            }
        }
    }

    #[test]
    fn ignores_false_anywhere_in_the_final_hash_and_nonliteral_lookalikes() {
        for source in [
            "Struct.new(:foo, keyword_init: false)",
            "Struct.new(:foo)",
            "Struct.new(:foo, keyword_init: false, keyword_init: true)",
            "Struct.new(:foo, keyword_init: true, keyword_init: false)",
            "Struct.new(:foo, keyword_init: true, keyword_init: nil, keyword_init: false)",
            "Struct.new(:foo, keyword_init: value)",
            "Struct.new(:foo, keyword_init: 1)",
            "Struct.new(:foo, keyword_init: (true))",
            "Struct.new(:foo, keyword_init: !false)",
            "Struct.new(:foo, 'keyword_init' => true)",
            "Struct.new(:foo, other: true)",
            "Struct.new(:foo, **{keyword_init: true})",
            "Struct.new(:foo, {keyword_init: true}, {})",
            "Struct.new(:foo, keyword_init: true, &block)",
            "Struct.new(:foo, {keyword_init: true}, arg)",
            "Other::Struct.new(keyword_init: true)",
            "obj.new(keyword_init: true)",
            "Struct.define(keyword_init: true)",
            "Struct = Class.new; Struct",
            "keyword_init = true; keyword_init",
        ] {
            assert!(
                correction_offenses(&RedundantStructKeywordInit, source).is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn multiline_symbol_spelling_and_value_source_are_preserved_in_diagnostics() {
        let found = correction_offenses(
            &RedundantStructKeywordInit,
            "Struct.new(\n  :foo,\n  :\"keyword_init\" =>\n    true\n)\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 3, 4));
        assert_eq!(
            found[0].message,
            "Remove the redundant `keyword_init: true`."
        );
    }
}
