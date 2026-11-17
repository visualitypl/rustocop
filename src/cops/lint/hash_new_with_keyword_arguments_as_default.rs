//! `Lint/HashNewWithKeywordArgumentsAsDefault` — unbraced Hash defaults.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct HashNewWithKeywordArgumentsAsDefault;
impl Cop for HashNewWithKeywordArgumentsAsDefault {
    fn name(&self) -> &'static str {
        "Lint/HashNewWithKeywordArgumentsAsDefault"
    }
    fn safe_fixes(&self) -> bool {
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
        if call.is_safe_navigation()
            || call.name().as_slice() != b"new"
            || !call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Hash"]))
        {
            return;
        }
        let arguments = ast::call_arguments(&call);
        let [argument] = arguments.as_slice() else {
            return;
        };
        let Some(hash) = argument.as_keyword_hash_node() else {
            return;
        };
        // Upstream counts only pairs, not keyword splats, for this exemption.
        let mut pairs = hash.elements().iter().filter_map(|n| n.as_assoc_node());
        if let Some(pair) = pairs.next()
            && pairs.next().is_none()
            && pair
                .key()
                .as_symbol_node()
                .is_some_and(|s| s.unescaped() == b"capacity")
        {
            return;
        }
        let loc = hash.location();
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(loc.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1).max(loc.start_offset())),
            message: "Use a hash literal instead of keyword arguments.".into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(argument))
                .flatten(),
        });
    }
}

/// RuboCop's corrector: wrap the keywords in braces.
fn correction(argument: &Node<'_>) -> Option<crate::offense::Correction> {
    let loc = argument.location();
    Some(crate::offense::Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: format!("{{{}}}", std::str::from_utf8(loc.as_slice()).ok()?),
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/hash_new_with_keyword_arguments_as_default_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/HashNewWithKeywordArgumentsAsDefault")
            .collect()
    }
    #[test]
    fn pinned_unbraced_defaults() {
        for (call, col) in [
            ("Hash.new(key: :value)", 10),
            ("Hash.new(capacity: 42, key: :value)", 10),
            ("::Hash.new(key: :value)", 12),
            ("Hash.new(:key => :value)", 10),
            ("Hash.new(key => 'value')", 10),
        ] {
            let source = format!("nil\n  {call}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, col + 2, 2)
            );
            assert_eq!(
                found[0].message,
                "Use a hash literal instead of keyword arguments."
            );
        }
    }
    #[test]
    fn canonical_bracing_only_in_closed_consumed_method_references() {
        for receiver in ["Hash", "::Hash"] {
            for argument in ["key: :value", "capacity: 42, key: :value", ":key => :value"] {
                let before = format!("[defined?({receiver}.new({argument}))]\n");
                let after = format!("[defined?({receiver}.new({{{argument}}}))]\n");
                crate::cops::testing::assert_correction(
                    &super::HashNewWithKeywordArgumentsAsDefault,
                    &before,
                    &after,
                );
            }
        }
    }
    #[test]
    fn pinned_legal_capacity_and_lookalikes() {
        for source in [
            "Hash.new({key: :value})",
            "Hash.new(capacity: 42)",
            "Hash.new",
            "Foo.new(key: :value)",
            "Hash.new(42)",
            "Hash&.new(key: :value)",
            "Other::Hash.new(key: :value)",
            "Hash.new(1, key: :value)",
            "Hash.new(key: :value, &block)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
