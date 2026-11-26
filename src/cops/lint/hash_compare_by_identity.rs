//! `Lint/HashCompareByIdentity` — object IDs used as collection keys.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct HashCompareByIdentity;
impl Cop for HashCompareByIdentity {
    fn name(&self) -> &'static str {
        "Lint/HashCompareByIdentity"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Prism fuses index getter sends into compound writes and targets.
        macro_rules! index {
            ($access:ident) => {{
                let call = node.$access().unwrap();
                (
                    call.arguments().and_then(|a| a.arguments().iter().next()),
                    call.closing_loc().end_offset(),
                )
            }};
        }
        let (key, end) = match node {
            Node::IndexAndWriteNode { .. } => index!(as_index_and_write_node),
            Node::IndexOrWriteNode { .. } => index!(as_index_or_write_node),
            Node::IndexOperatorWriteNode { .. } => index!(as_index_operator_write_node),
            Node::IndexTargetNode { .. } => index!(as_index_target_node),
            _ => {
                let Some(call) = node.as_call_node() else {
                    return;
                };
                if !matches!(
                    call.name().as_slice(),
                    b"key?" | b"has_key?" | b"fetch" | b"[]" | b"[]="
                ) {
                    return;
                }
                (
                    ast::call_arguments(&call).into_iter().next(),
                    ast::source_end(node),
                )
            }
        };
        let Some(key) = key.and_then(|n| n.as_call_node()) else {
            return;
        };
        if key.is_safe_navigation()
            || key.name().as_slice() != b"object_id"
            || !ast::call_arguments(&key).is_empty()
            || key.block().is_some()
        {
            return;
        }
        let start = node.location().start_offset();
        let (line, col) = file.reference_line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: "Use `Hash#compare_by_identity` instead of using `object_id` for keys.".into(),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/hash_compare_by_identity_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/HashCompareByIdentity")
            .collect()
    }
    #[test]
    fn pinned_key_methods_and_safe_navigation() {
        for call in [
            "hash.key?(foo.object_id)",
            "hash.has_key?(foo.object_id)",
            "hash[foo.object_id] = bar",
            "hash[foo.object_id]",
            "hash.fetch(foo.object_id, 42)",
            "hash.key?(object_id)",
            "hash&.key?(object_id)",
        ] {
            let source = format!("nil\n  {call}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                "Use `Hash#compare_by_identity` instead of using `object_id` for keys."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn no_type_inference_or_nested_key_search() {
        for source in [
            "hash.key?(foo)",
            "hash.key?(foo&.object_id)",
            "hash.key?(foo.object_id(1))",
            "hash.fetch(x, foo.object_id)",
            "hash[[foo.object_id]]",
            "hash.keys(foo.object_id)",
            "object_id = 1; hash.key?(object_id)",
            "hash[foo.__id__]",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
