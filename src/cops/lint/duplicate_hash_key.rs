//! `Lint/DuplicateHashKey` — flags a key repeated in a hash literal.
//!
//! Matches RuboCop's semantics: only keys that are (recursively) basic
//! literals or constants are compared, structurally, so `'a'`, `"a"` collide
//! and so do `:a` and `a:`. Braceless keyword arguments count as hash
//! literals. Every repetition after the first is flagged.

use std::collections::HashSet;

use ruby_prism::{Node, NodeList};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{fingerprint, is_const, is_recursive_basic_literal},
    },
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/DuplicateHashKey` cop.
pub struct DuplicateHashKey;

impl Cop for DuplicateHashKey {
    fn name(&self) -> &'static str {
        "Lint/DuplicateHashKey"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let elements: NodeList<'pr> = if let Some(hash) = node.as_hash_node() {
            hash.elements()
        } else if let Some(hash) = node.as_keyword_hash_node() {
            hash.elements()
        } else {
            return;
        };
        let keys: Vec<Node<'pr>> = elements
            .iter()
            .filter_map(|element| element.as_assoc_node().map(|pair| pair.key()))
            .filter(|key| is_recursive_basic_literal(key) || is_const(key))
            .collect();
        if keys.len() < 2 {
            return;
        }
        let mut seen = HashSet::new();
        for key in keys {
            if seen.insert(fingerprint(&key)) {
                continue;
            }
            let (line, col) = file.line_col(key.location().start_offset());
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: "Duplicated key in hash literal.".to_string(),
                line,
                end_line: file.last_line_of(key.location()),
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/duplicate_hash_key_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateHashKey")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_duplicated_string_key_with_message() {
        let found = offenses("hash = { 'otherkey' => 'value', 'key' => 'value', 'key' => 'hi' }\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 51));
        assert_eq!(found[0].message, "Duplicated key in hash literal.");
    }

    #[test]
    fn flags_duplicated_constant_key() {
        assert_eq!(
            positions("hash = { 'otherkey' => 'value', KEY => 'value', KEY => 'hi' }\n"),
            vec![(1, 49)]
        );
    }

    #[test]
    fn flags_two_duplicated_keys() {
        assert_eq!(
            positions("hash = { fruit: 'apple', veg: 'kale', veg: 'cuke', fruit: 'orange' }\n"),
            vec![(1, 39), (1, 52)]
        );
    }

    #[test]
    fn flags_key_duplicated_three_times_twice() {
        assert_eq!(
            positions("hash = { 1 => 2, 1 => 3, 1 => 4 }\n"),
            vec![(1, 18), (1, 26)]
        );
    }

    #[test]
    fn accepts_distinct_keys() {
        assert!(
            offenses("hash = { ['one', 'two'] => ['hello, bye'], ['two'] => ['yes, no'] }\n")
                .is_empty()
        );
    }

    #[test]
    fn flags_duplicated_literal_keys() {
        for key in [
            "!true",
            "\"#{2}\"",
            "(1)",
            "(false && true)",
            "(false <=> true)",
            "(false or true)",
            "[1, 2, 3]",
            "{ :a => 1, :b => 2 }",
            "{ a: 1, b: 2 }",
            "/./",
            "%r{abx}ixo",
            "1.0",
            "1",
            "false",
            "nil",
            "'str'",
            "(42..)",
        ] {
            let source = format!("hash = {{ {key} => 1, {key} => 4}}\n");
            let col = 10 + key.chars().count() as u32 + 7;
            assert_eq!(positions(&source), vec![(1, col)], "should flag {key}");
        }
    }

    #[test]
    fn accepts_duplicated_non_literal_keys() {
        for key in [
            "\"#{some_method_call}\"",
            "(x && false)",
            "(x == false)",
            "(x or false)",
            "[some_method_call]",
            "{ :sym => some_method_call }",
            "{ some_method_call => :sym }",
            "/.#{some_method_call}/",
            "%r{abx#{foo}}ixo",
            "some_method_call",
            "some_method_call(x, y)",
        ] {
            let source = format!("hash = {{ {key} => 1, {key} => 4}}\n");
            assert!(offenses(&source).is_empty(), "should not flag {key}");
        }
    }

    #[test]
    fn flags_keys_across_spellings_and_keyword_arguments() {
        assert_eq!(positions("{ a: 1, :a => 2 }\n"), vec![(1, 9)]);
        assert_eq!(positions("{ 'a' => 1, \"a\" => 2 }\n"), vec![(1, 13)]);
        assert_eq!(positions("foo(a: 1, a: 2)\n"), vec![(1, 11)]);
        assert_eq!(positions("{\n  a: 1,\n  a: 2\n}\n"), vec![(3, 3)]);
        assert_eq!(
            positions("{ __ENCODING__ => 1, __ENCODING__ => 2 }\n"),
            vec![(1, 22)]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "{ a: 1, b: 1 }\n",
            "{ 1 => :a, 1.0 => :a }\n",
            "{ :a => 1, 'a' => 2 }\n",
            "{ a: 1, **opts, b: 2 }\n",
            "{ a: 1 }\n{ a: 2 }\n",
            "{ a: { b: 1 }, b: { b: 2 } }\n",
            "{ x => 1, x => 2 }\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
