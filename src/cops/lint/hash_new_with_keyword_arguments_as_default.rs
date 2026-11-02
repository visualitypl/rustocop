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
                .then(|| closed_method_reference(node, argument, cx, file))
                .flatten(),
        });
    }
}

// Ordinary bracing repairs Ruby3.4 errors and changes keyword dispatch/default
// values, so it is not semantics-preserving. A closed consumed defined? call
// reference never invokes new or evaluates its arguments. Both sides return
// "method". Keep this proof independent of the broader detector: no delayed
// execution, bindings, calls/observers, DATA, blocks, dynamic values, comments
// inside the edit, or heredocs. Shared main-parse source/encoding/position/
// code-object/heap and atomic guards remain authoritative.
fn closed_method_reference(
    node: &Node<'_>,
    argument: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let call = node.as_call_node()?;
    if call.block().is_some() {
        return None;
    }
    let receiver = call.receiver()?;
    let receiver = match receiver.location().as_slice() {
        b"Hash" => "Hash",
        b"::Hash" => "::Hash",
        _ => return None,
    };
    let arg = argument.location().as_slice();
    if !matches!(
        arg,
        b"key: :value" | b"capacity: 42, key: :value" | b":key => :value"
    ) {
        return None;
    }
    let arg = std::str::from_utf8(arg).ok()?;
    if node.location().as_slice() != format!("{receiver}.new({arg})").as_bytes() {
        return None;
    }
    let mut ancestors = cx.ancestors();
    ancestors.next()?.as_defined_node()?;
    let array = ancestors.next()?.as_array_node()?;
    let statements = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some() || array.elements().len() != 1 || statements.body().len() != 1 {
        return None;
    }
    let loc = argument.location();
    Some(crate::offense::Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: format!("{{{arg}}}"),
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
    fn uncertain_edits_retain_detections() {
        for source in [
            "Hash.new(key: :value)",
            "[defined?(Hash.new(key: effect))]",
            "[defined?(Hash.new(key: :value))]; File.read(__FILE__)",
            "[defined?(Hash.new(key: :value))]; __LINE__",
            "[defined?(Hash.new(key: :value))]\n__END__\ndata\n",
            "def f; [defined?(Hash.new(key: :value))]; end",
            "[defined?(Hash.new(key: # keep\n:value))]",
            "[defined?(Hash.new(key: <<~DOC))]\ntext\nDOC\n",
            "[defined?(Hash.new(key: :value))]; ObjectSpace.each_object {}",
            "[defined?(Hash.new(key: :value))]; RubyVM::InstructionSequence.compile('1')",
        ] {
            let found = crate::cops::testing::correction_offenses(
                &super::HashNewWithKeywordArgumentsAsDefault,
                source,
            );
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
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
