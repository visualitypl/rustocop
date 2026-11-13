//! `Lint/SharedMutableDefault` — Hash defaults that share a mutable collection.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/SharedMutableDefault` cop (detection only).
pub struct SharedMutableDefault;

const MESSAGE: &str = "Do not create a Hash with a mutable default value \
                      as the default value can accidentally be changed.";

impl Cop for SharedMutableDefault {
    fn name(&self) -> &'static str {
        "Lint/SharedMutableDefault"
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
        if call.is_safe_navigation()
            || call.name().as_slice() != b"new"
            || !call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Hash"]))
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let matches = match args.as_slice() {
            [default] => mutable_default(default) && !capacity_argument(default),
            [default, capacity] => mutable_default(default) && capacity_argument(capacity),
            _ => false,
        };
        if !matches {
            return;
        }
        let (line, col) = file.line_col(call.location().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: MESSAGE.into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
        });
    }
}

fn mutable_default(node: &Node<'_>) -> bool {
    if matches!(
        node,
        Node::ArrayNode { .. } | Node::HashNode { .. } | Node::KeywordHashNode { .. }
    ) {
        return true;
    }
    node.as_call_node().is_some_and(|call| {
        !call.is_safe_navigation()
            && call.name().as_slice() == b"new"
            && call.block().is_none()
            && ast::call_arguments(&call).is_empty()
            && call.receiver().is_some_and(|r| {
                ast::is_global_const(&r, &["Array"]) || ast::is_global_const(&r, &["Hash"])
            })
    })
}

// Exactly one symbol-keyed pair, whether braced or keyword-style. A string key,
// splat or additional keyword is not the pinned capacity exception.
fn capacity_argument(node: &Node<'_>) -> bool {
    let pairs = if let Some(hash) = node.as_hash_node() {
        hash.elements()
    } else if let Some(hash) = node.as_keyword_hash_node() {
        hash.elements()
    } else {
        return false;
    };
    if pairs.len() != 1 {
        return false;
    }
    pairs
        .iter()
        .next()
        .and_then(|pair| pair.as_assoc_node())
        .is_some_and(|pair| {
            pair.key()
                .as_symbol_node()
                .is_some_and(|key| key.unescaped() == b"capacity")
        })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/lint/shared_mutable_default_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/SharedMutableDefault")
            .collect()
    }

    #[test]
    fn flags_literal_and_empty_constructor_defaults_with_optional_capacity() {
        for source in [
            "Hash.new([])",
            "Hash.new Array.new",
            "Hash.new({})",
            "Hash.new(Hash.new)",
            "Hash.new([], capacity: 42)",
            "Hash.new(Array.new, capacity: 42)",
            "Hash.new(Hash.new, capacity: 42)",
            "Hash.new({}, capacity: 42)",
            "::Hash.new(::Array.new)",
            "Hash.new(a: 1)",
            "Hash.new(**opts)",
        ] {
            let found = offenses(&format!("# header\n  {source}\n"));
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                "Do not create a Hash with a mutable default value as the default value can accidentally be changed."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn ignores_unrelated_immutable_frozen_and_block_defaults() {
        for source in [
            "[]",
            "{}",
            "Array.new",
            "Hash.new",
            "Hash.new { |h, k| h[k] = [] }",
            "Hash.new { |h, k| h[k] = {} }",
            "Hash.new { 0 }",
            "Hash.new { Array.new }",
            "Hash.new { Hash.new }",
            "Hash.new { {} }",
            "Hash.new { [] }",
            "Hash.new(0)",
            "Hash.new(false)",
            "Hash.new(true)",
            "Hash.new(nil)",
            "Hash.new(BigDecimal(0))",
            "Hash.new(BigDecimal(0.0))",
            "Hash.new(0.0)",
            "Hash.new(0.0.to_d)",
            "Hash.new([].freeze)",
            "Hash.new({}.freeze)",
            "Hash.new(Array.new.freeze)",
            "Hash.new(Hash.new.freeze)",
            "Hash.new(capacity: 42)",
            "Hash.new(0, capacity: 42)",
            "Hash.new('')",
            "Hash.new(String.new)",
            "Hash.new(Array.new(3))",
            "Hash.new(Array.new {})",
            "Hash.new(([]))",
            "Hash.new([], unknown: 1)",
            "Hash.new([], capacity: 1, unknown: 2)",
            "Hash.new([], {})",
            "Other::Hash.new([])",
            "Hash&.new([])",
            "Hash.new(Other::Array.new)",
            "Hash.new(Array&.new)",
            "Hash.new([], &block)",
            "Hash.new(*[[]])",
            "default = []; Hash.new(default)",
            "Hash.[]([])",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn highlights_send_not_literal_block_and_visits_typed_children() {
        let found = offenses("Hash.new(\n  []\n) do |h, k|\n  Hash.new({})\nend\n");
        assert_eq!(
            found
                .iter()
                .map(|o| (o.line, o.col, o.end_line))
                .collect::<Vec<_>>(),
            [(1, 1, 3), (4, 3, 4)]
        );
    }
}
