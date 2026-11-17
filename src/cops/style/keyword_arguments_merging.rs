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
                .then(|| rewrite(&splat, &merge))
                .flatten(),
        });
    }
}
/// RuboCop's corrector: `**receiver, ` and the merged hashes spread.
fn rewrite(
    splat: &ruby_prism::AssocSplatNode<'_>,
    merge: &ruby_prism::CallNode<'_>,
) -> Option<Correction> {
    fn text<'a>(n: &'a Node<'_>) -> Option<&'a str> {
        std::str::from_utf8(n.location().as_slice()).ok()
    }
    let others = crate::cops::ast::call_arguments(merge)
        .iter()
        .map(|n| {
            let source = text(n)?;
            Some(if n.as_hash_node().is_some() {
                source[1..source.len() - 1].to_owned()
            } else if n.as_keyword_hash_node().is_some() {
                source.to_owned()
            } else {
                format!("**{source}")
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let loc = splat.location();
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: format!("**{}, {}", text(&merge.receiver()?)?, others.join(", ")),
    })
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
