//! `Style/KeywordArgumentsMerging`: keyword spreading, not Ruby 2 positional hashes.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct KeywordArgumentsMerging;
impl Cop for KeywordArgumentsMerging {
    fn name(&self) -> &'static str {
        "Style/KeywordArgumentsMerging"
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
        let Some(outer) = node.as_call_node() else {
            return;
        };
        if outer.is_safe_navigation()
            || outer
                .block()
                .is_some_and(|b| b.as_block_argument_node().is_some())
        {
            return;
        }
        let Some(last) = outer.arguments().and_then(|a| a.arguments().iter().last()) else {
            return;
        };
        let elements: Vec<_> = if let Some(h) = last.as_keyword_hash_node() {
            h.elements().iter().collect()
        } else if let Some(h) = last.as_hash_node() {
            h.elements().iter().collect()
        } else {
            return;
        };
        let Some(splat) = elements.first().and_then(|e| e.as_assoc_splat_node()) else {
            return;
        };
        let Some(merge) = splat.value().and_then(|v| v.as_call_node()) else {
            return;
        };
        if merge.name().as_slice() != b"merge"
            || merge.is_safe_navigation()
            || merge.block().is_some()
        {
            return;
        }
        let start = merge.location().start_offset();
        let end = merge.location().end_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: "Provide additional arguments directly rather than using `merge`.".into(),
            correction: cx
                .corrections_requested()
                .then(|| rewrite(&merge, cx, file))
                .flatten(),
        });
    }
}
// A literal receiver cannot be a Hash subclass, carry custom defaults or escape
// into appended argument evaluation. All inputs are fresh symbol-key hashes
// with inert scalar values: no to_hash dispatch, mutation or effect reordering.
fn literal_hash(node: &Node<'_>) -> bool {
    let Some(root) = node
        .as_hash_node()
        .map(|h| h.as_node())
        .or_else(|| node.as_keyword_hash_node().map(|h| h.as_node()))
    else {
        return false;
    };
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        let elements = if let Some(h) = node.as_hash_node() {
            h.elements()
        } else if let Some(h) = node.as_keyword_hash_node() {
            h.elements()
        } else {
            return false;
        };
        for element in elements.iter() {
            if element.as_assoc_node().is_some_and(|a| {
                a.key().as_symbol_node().is_some()
                    && super::explicit_block_argument::inert_scalar(&a.value())
            }) {
                continue;
            }
            let Some(value) = element.as_assoc_splat_node().and_then(|s| s.value()) else {
                return false;
            };
            pending.push(value);
        }
    }
    true
}
fn rewrite(
    merge: &ruby_prism::CallNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let mut segments = Vec::new();
    literal_merge_segments(&merge.as_node(), &mut segments)?;
    let start = merge.location().start_offset();
    let end = merge.location().end_offset();
    if file.bytes[start..end].contains(&b'\n')
        || super::correction::contains_comment(file, start..end)
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.argument_rewrite_facts(root).unsafe_core
        || file.class_method_bindings(root).observes_sources
        || file.class_method_bindings(root).observes_data
    {
        return None;
    }
    Some(Correction {
        range: start..end,
        replacement: segments.join(", "),
    })
}
fn literal_merge_segments(node: &Node<'_>, segments: &mut Vec<String>) -> Option<()> {
    let mut current = node.as_call_node()?.as_node();
    let mut merges = Vec::new();
    loop {
        if literal_hash(&current) {
            segments.push(
                std::str::from_utf8(current.location().as_slice())
                    .ok()?
                    .to_owned(),
            );
            break;
        }
        let merge = current.as_call_node()?;
        if merge.name().as_slice() != b"merge"
            || merge.is_safe_navigation()
            || merge.block().is_some()
        {
            return None;
        }
        current = merge.receiver()?;
        merges.push(merge);
    }
    for merge in merges.into_iter().rev() {
        if let Some(arguments) = merge.arguments() {
            for arg in arguments.arguments().iter() {
                if !literal_hash(&arg) {
                    return None;
                }
                let text = std::str::from_utf8(arg.location().as_slice()).ok()?;
                let text = if arg.as_hash_node().is_some() {
                    text.strip_prefix('{')?.strip_suffix('}')?
                } else {
                    text
                };
                if !text.trim().is_empty() {
                    segments.push(text.to_owned());
                }
            }
        }
    }
    Some(())
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/keyword_arguments_merging_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/KeywordArgumentsMerging")
            .collect()
    }
    #[test]
    fn every_pinned_correction_pair_is_individually_accounted_for() {
        let pairs: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/argument_wrappers_corrections.json"
        ))
        .unwrap();
        let pairs: Vec<_> = pairs
            .iter()
            .filter(|p| p["cop"] == "Style/KeywordArgumentsMerging")
            .collect();
        assert_eq!(pairs.len(), 7);
        for p in pairs {
            let before = p["before"].as_str().unwrap();
            assert!(!p["withheld_reason"].as_str().unwrap().is_empty());
            assert!(!offenses(before).is_empty(), "spec line {}", p["line"]);
            crate::cops::testing::assert_correction(&KeywordArgumentsMerging, before, before);
            assert!(offenses(p["after"].as_str().unwrap()).is_empty());
        }
    }
    #[test]
    fn long_literal_chains_and_spreads_use_iterative_bounded_proofs() {
        for n in [16, 512] {
            let source = format!("{{}}{}", ".merge({v: 1})".repeat(n));
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let node = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .first()
                .unwrap();
            let mut segments = Vec::new();
            assert!(literal_merge_segments(&node, &mut segments).is_some());
            assert_eq!(segments.len(), n + 1);
            assert!(segments.iter().all(|s| s.len() <= 4));
            let source = format!("{}{{}}{}", "{**".repeat(n), "}".repeat(n));
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let node = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .first()
                .unwrap();
            assert!(literal_hash(&node));
        }
    }
    #[test]
    fn pinned_merge_shapes() {
        for expr in [
            "options.merge(y: 1)",
            "options.merge(other_options)",
            "options.merge(y: 1, **other_options, z: 2)",
            "options.merge({ y: 1 })",
            "options.merge(other_options, y: 1, **yet_another_options)",
            "options.merge(other_options, yet_another_options)",
            "options.merge(other_options).merge(more_options)",
        ] {
            let source = format!("# test\nfoo(x, **{expr})\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 10));
            assert_eq!(
                found[0].message,
                "Provide additional arguments directly rather than using `merge`."
            );
        }
    }
    #[test]
    fn literal_canonical_corrections_preserve_order_and_bytes() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("foo(x, **{a: 0}.merge(y: 1))", "foo(x, **{a: 0}, y: 1)"),
            ("foo(**{a: 1}.merge({ b: 2 }))", "foo(**{a: 1},  b: 2 )"),
            (
                "foo(**{a: 1}.merge({b: 2}, {c: 3}))",
                "foo(**{a: 1}, b: 2, c: 3)",
            ),
            (
                "foo(**{a: 1}.merge(a: 2, z: 3))",
                "foo(**{a: 1}, a: 2, z: 3)",
            ),
            (
                "foo(**{a: 1}.merge(b: 2, **{c: 3}, d: 4))",
                "foo(**{a: 1}, b: 2, **{c: 3}, d: 4)",
            ),
            (
                "foo(**{a: 1}.merge({b: 2}, y: 1, **{z: 3}))",
                "foo(**{a: 1}, b: 2, y: 1, **{z: 3})",
            ),
            (
                "foo(**{a: 1}.merge({b: 2}).merge({c: 3}))",
                "foo(**{a: 1}, b: 2, c: 3)",
            ),
        ] {
            assert_correction(&KeywordArgumentsMerging, before, after);
        }
    }
    #[test]
    fn unknown_effectful_and_overridden_merges_keep_offenses_without_edits() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "foo(**options.merge(y: 1))",
            "foo(**{}.merge(effect()))",
            "foo(**{a: 1}.merge(b: effect()))",
            "foo(**{key => 1}.merge(b: 2))",
            "foo(**{a: 1}.merge(**other))",
            "foo(**{}.merge(b: 1) { :value })",
            "def merge(*args); super; end; foo(**{}.merge(b: 1))",
            "class Hash; def merge(*args); {other: 1}; end; end; foo(**{}.merge(b: 1))",
            "p DATA.pos; foo(**{}.merge(b: 1))\n__END__\nraw",
            "p __FILE__; foo(**{}.merge(b: 1))",
        ] {
            let found = correction_offenses(&KeywordArgumentsMerging, source);
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
    #[test]
    fn string_visibility_change_retains_exact_offense_and_bytes() {
        let source = "class Hash\n  private \"merge\"\nend\ndef take(**kwargs);kwargs;end\nbegin\n  p take(**{}.merge(a: 1))\nrescue NoMethodError\n  p :missing\nend\n";
        let found = crate::cops::testing::correction_offenses(&KeywordArgumentsMerging, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (6, 12, 6));
        assert_eq!(
            found[0].message,
            "Provide additional arguments directly rather than using `merge`."
        );
        assert!(found[0].correction.is_none());
        crate::cops::testing::assert_correction(&KeywordArgumentsMerging, source, source);
    }
    #[test]
    fn no_offense_lookalikes() {
        for source in [
            "foo(x, **options)",
            "foo(x, **options.merge!(other_options))",
            "foo(**opts.merge(other, &my_proc))",
            "opts.merge(y: 1)",
            "foo(opts.merge(y: 1))",
            "{**opts.merge(y: 1)}",
            "super(**opts.merge(y: 1))",
            "foo(**opts.merge(y: 1) { :override })",
            "foo(**opts&.merge(y: 1))",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
