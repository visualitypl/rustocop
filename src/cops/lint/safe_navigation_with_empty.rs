//! `Lint/SafeNavigationWithEmpty` — safe `empty?` at the root of an if predicate.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct SafeNavigationWithEmpty;
impl Cop for SafeNavigationWithEmpty {
    fn name(&self) -> &'static str {
        "Lint/SafeNavigationWithEmpty"
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
        let predicate = if let Some(n) = node.as_if_node() {
            n.predicate()
        } else if let Some(n) = node.as_unless_node() {
            n.predicate()
        } else {
            return;
        };
        // Prism represents pattern guards with If/Unless; parser's if_guard /
        // unless_guard are NOT on_if callbacks.
        if cx.parent().is_some_and(|n| n.as_in_node().is_some()) {
            return;
        }
        let Some(call) = predicate.as_call_node() else {
            return;
        };
        if !call.is_safe_navigation()
            || call.name().as_slice() != b"empty?"
            || !ast::call_arguments(&call).is_empty()
            || call.block().is_some()
        {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        if receiver.as_call_node().is_some_and(|c| {
            c.is_safe_navigation() && !c.block().is_some_and(|b| b.as_block_node().is_some())
        }) {
            return;
        }
        let start = predicate.location().start_offset();
        let end = ast::source_end(&predicate);
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: "Avoid calling `empty?` with the safe navigation operator in conditionals."
                .into(),
            correction: cx
                .corrections_requested()
                .then(|| inactive_condition(node, &predicate, &receiver, cx, file))
                .flatten(),
        });
    }
}
// The ordinary canonical rewrite duplicates receiver evaluation and changes
// false/nil values, dispatch and exceptions. Only an empty conditional in the
// sole closed top-level `if false` is proved. Bare exact ASCII sends introduce
// no bindings, literals, allocations or warnings; neither condition executes.
// Main-parse source/code-object/heap/position/encoding/heredoc guards still apply.
fn inactive_condition(
    node: &Node<'_>,
    predicate: &Node<'_>,
    receiver: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let bare = receiver.as_call_node()?;
    let name = bare.name().as_slice();
    if bare.receiver().is_some()
        || bare.arguments().is_some()
        || bare.block().is_some()
        || name.is_empty()
        || !name.iter().all(|b| b.is_ascii_alphabetic() || *b == b'_')
        || receiver.location().as_slice() != name
    {
        return None;
    }
    let name = std::str::from_utf8(name).ok()?;
    if predicate.location().as_slice() != format!("{name}&.empty?").as_bytes() {
        return None;
    }
    let empty = if let Some(n) = node.as_if_node() {
        n.statements().is_none()
            && n.subsequent().is_none()
            && n.end_keyword_loc().is_some()
            && n.if_keyword_loc()?.as_slice() == b"if"
    } else {
        let n = node.as_unless_node()?;
        n.statements().is_none() && n.else_clause().is_none() && n.end_keyword_loc().is_some()
    };
    if !empty {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let body = ancestors.next()?.as_statements_node()?;
    let outer = ancestors.next()?.as_if_node()?;
    let top = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some()
        || body.body().len() != 1
        || top.body().len() != 1
        || outer.predicate().as_false_node().is_none()
        || outer.subsequent().is_some()
        || outer.end_keyword_loc().is_none()
        || outer.if_keyword_loc()?.as_slice() != b"if"
    {
        return None;
    }
    Some(Correction {
        range: predicate.location().start_offset()..predicate.location().end_offset(),
        replacement: format!("{name} && {name}.empty?"),
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/safe_navigation_with_empty_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/SafeNavigationWithEmpty")
            .collect()
    }
    #[test]
    fn fixed_root_matcher_and_no_general_condition_search() {
        for s in [
            "foo = build_collection\nreturn if foo&.empty?",
            "return if @foo&.empty?",
            "return if FOO&.empty?",
            "if foo&.empty?; end",
            "foo&.empty? ? a : b",
            "if a; elsif foo&.empty?; end",
            "if (foo&.bar)&.empty?; end",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "if foo&.empty?(1); end",
            "if foo&.empty?(&b); end",
            "if foo&.empty? { it }; end",
            "if (foo&.empty?); end",
            "if !foo&.empty?; end",
            "if foo&.empty? && bar; end",
            "while foo&.empty?; end",
            "case v; in x if foo&.empty?; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn canonical_correction_is_guarded_and_not_a_noop() {
        for keyword in ["if", "unless"] {
            let before = format!("if false\n  {keyword} probe&.empty?\n  end\nend\n");
            let found =
                crate::cops::testing::correction_offenses(&SafeNavigationWithEmpty, &before);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_some(), "{found:?}");
            let after = crate::corrections::apply(before.as_bytes(), &found).unwrap();
            assert_eq!(after.count, 1);
            assert_eq!(
                after.bytes,
                before
                    .replace("probe&.empty?", "probe && probe.empty?")
                    .as_bytes()
            );
            assert!(offenses(std::str::from_utf8(&after.bytes).unwrap()).is_empty());
        }
    }
    #[test]
    fn uncertain_individual_edits_retain_findings() {
        for s in [
            "return unless foo&.empty?",
            "foo = nil; return if foo&.empty?",
            "if false; if @foo&.empty?; end; end",
            "if false; if foo&.empty?(); end; end",
            "if false; if foo&.empty?; puts :x; end; end",
            "if false; if foo&.empty?; end; end\np __LINE__",
            "if false; if foo&.empty?; end; end\n__END__\ndata",
        ] {
            let found = crate::cops::testing::correction_offenses(&SafeNavigationWithEmpty, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
}
