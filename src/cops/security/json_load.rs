//! `Security/JSONLoad` — flags `JSON.load` and `JSON.restore`, which can
//! create arbitrary Ruby objects (`json_class`); `JSON.parse` cannot.
//!
//! Matches RuboCop's semantics: a plain call (not `&.`) to `load` or
//! `restore` on `JSON` or `::JSON` is reported, at the method name, when
//!
//! - it has exactly two arguments and the second is a hash (before json
//!   2.17.0 that is the `proc` argument, so options in it are ignored), or
//! - its last argument does not contain a `create_additions:` pair anywhere
//!   (RuboCop searches the argument's whole subtree).
//!
//! `JSON.load` without arguments is not reported. One deviation: a block
//! argument (`&blk`) is ignored, where RuboCop's AST takes it for the last
//! argument, so `JSON.load(s, nil, create_additions: false, &blk)` passes
//! (as does the broken `JSON.load(&blk)`).

use ruby_prism::{AssocNode, Node, Visit};

use crate::{
    cops::{Cop, NodeContext, ast::is_global_const},
    offense::Offense,
    source::SourceFile,
};

/// The `Security/JSONLoad` cop.
pub struct JsonLoad;

impl Cop for JsonLoad {
    fn name(&self) -> &'static str {
        "Security/JSONLoad"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let method = call.name().as_slice();
        if !matches!(method, b"load" | b"restore")
            || call.is_safe_navigation()
            || !call
                .receiver()
                .is_some_and(|receiver| is_global_const(&receiver, &["JSON"]))
        {
            return;
        }
        let arguments: Vec<Node<'pr>> = call
            .arguments()
            .map(|arguments| arguments.arguments().iter().collect())
            .unwrap_or_default();
        let insecure = match arguments.as_slice() {
            [] => false,
            [_, options] if is_hash(options) => true,
            [.., last] => !has_create_additions_pair(last),
        };
        if !insecure {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: selector.start_offset()..selector.end_offset(),
                    replacement: "parse".into(),
                }),
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Prefer `JSON.parse` over `JSON.{}`.",
                String::from_utf8_lossy(method)
            ),
            line,
            end_line: line,
            col,
        });
    }
}

/// A hash literal, braced or not (both `hash` in RuboCop's AST).
fn is_hash(node: &Node<'_>) -> bool {
    matches!(node, Node::HashNode { .. } | Node::KeywordHashNode { .. })
}

/// Whether `node` or any node inside it is a `create_additions: value` pair
/// with a symbol key.
fn has_create_additions_pair(node: &Node<'_>) -> bool {
    let mut search = PairSearch { found: false };
    search.visit(node);
    search.found
}

/// Looks for a `create_additions:` pair.
struct PairSearch {
    found: bool,
}

impl<'pr> Visit<'pr> for PairSearch {
    fn visit_assoc_node(&mut self, node: &AssocNode<'pr>) {
        if node
            .key()
            .as_symbol_node()
            .is_some_and(|key| key.unescaped() == b"create_additions")
        {
            self.found = true;
        }
        ruby_prism::visit_assoc_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/security/json_load_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::pipeline::lint_str;

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Security/JSONLoad")
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn at(line: u32, col: u32, method: &str) -> (u32, u32, String) {
        (
            line,
            col,
            format!("Prefer `JSON.parse` over `JSON.{method}`."),
        )
    }

    #[test]
    fn flags_json_load() {
        assert_eq!(
            found("JSON.load(arg)\n::JSON.load(arg)\n"),
            [at(1, 6, "load"), at(2, 8, "load")]
        );
    }

    #[test]
    fn flags_json_restore() {
        assert_eq!(
            found("JSON.restore(arg)\n::JSON.restore(arg)\n"),
            [at(1, 6, "restore"), at(2, 8, "restore")]
        );
    }

    #[test]
    fn flags_create_additions_passed_as_the_second_argument() {
        // Before json 2.17.0 a second-argument hash is the `proc` argument,
        // so its `create_additions` is ignored.
        assert_eq!(
            found(
                "JSON.load(arg, create_additions: false)\n\
                 ::JSON.load(arg, { create_additions: false })\n\
                 JSON.load(arg, create_additions: true)\n\
                 JSON.load(arg, { allow_nan: false, create_additions: true })\n\
                 JSON.restore(arg, create_additions: false)\n"
            ),
            [
                at(1, 6, "load"),
                at(2, 8, "load"),
                at(3, 6, "load"),
                at(4, 6, "load"),
                at(5, 6, "restore"),
            ]
        );
    }

    #[test]
    fn flags_an_unrelated_option() {
        assert_eq!(
            found("JSON.load(arg, max_nesting: 1)\n::JSON.load(arg, max_nesting: 1)\n"),
            [at(1, 6, "load"), at(2, 8, "load")]
        );
    }

    #[test]
    fn flags_options_without_a_literal_create_additions_key() {
        for source in [
            "JSON.load(arg, nil, \"create_additions\" => true)\n",
            "JSON.load(arg, nil, **opts)\n",
            "JSON.load(arg, **opts)\n",
            "JSON.load(arg, nil, opts)\n",
            "JSON.load(arg, opts)\n",
            "JSON.load(*args)\n",
            "JSON.load(arg) { |x| x }\n",
            "JSON.load(arg, &blk)\n",
        ] {
            assert_eq!(found(source), [at(1, 6, "load")], "should flag {source:?}");
        }
        assert_eq!(
            found("data = JSON\n  .load(io)\n"),
            [at(2, 4, "load")],
            "reported at the method name"
        );
    }

    #[test]
    fn accepts_create_additions_passed_as_the_third_argument() {
        let source = "\
JSON.load(arg, nil, create_additions: false)
::JSON.load(arg, nil, create_additions: true)
JSON.load(arg, proc {}, create_additions: false, max_nesting: 19)
::JSON.load(arg, proc {}, create_additions: false, max_nesting: 19)
JSON.load(arg, -> {}, { max_nesting: 19, create_additions: false, allow_nan: true })
::JSON.load(arg, -> {}, { max_nesting: 19, create_additions: false, allow_nan: true })
";
        assert!(found(source).is_empty());
    }

    #[test]
    fn accepts_other_spellings_of_the_create_additions_key() {
        for source in [
            "JSON.load(arg, nil, \"create_additions\": true)\n",
            "JSON.load(arg, nil, :create_additions => false)\n",
            "JSON.load(arg, nil, create_additions:)\n",
            "JSON.load(arg, nil, { create_additions: false }) { |x| x }\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn accepts_a_create_additions_pair_anywhere_in_the_last_argument() {
        // RuboCop searches the last argument's whole subtree.
        for source in [
            "JSON.load(foo(create_additions: true))\n",
            "JSON.load(arg, nil, max_nesting: { create_additions: false })\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn accepts_an_explicit_create_additions_with_a_block_argument() {
        // RuboCop counts `&blk` as the last argument and flags this; the
        // option is passed explicitly, so rustocop does not.
        assert!(found("JSON.load(arg, nil, create_additions: true, &blk)\n").is_empty());
        assert!(found("JSON.load(&blk)\n").is_empty());
        assert_eq!(
            found("JSON.load(arg, { create_additions: true }, &blk)\n"),
            [at(1, 6, "load")]
        );
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "SomeModule::JSON.load(arg)\nSomeModule::JSON.restore(arg)\n",
            "JSON.parse(arg)\n::JSON.parse(arg)\n",
            "JSON.dump(arg)\n::JSON.dump(arg)\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "JSON.load\n",
            "JSON&.load(arg)\n",
            "json.load(arg)\n",
            "load(arg)\n",
            "JSON.unsafe_load(arg)\n",
            "JSON.load_file(path)\n",
            "Oj.load(arg)\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
