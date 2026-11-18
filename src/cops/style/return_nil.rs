//! `Style/ReturnNil` — opt-in preference for bare `return`.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ReturnNil;
impl Cop for ReturnNil {
    fn name(&self) -> &'static str {
        "Style/ReturnNil"
    }
    /// Upstream's Style/AndOr lists this cop as autocorrect-incompatible
    /// and runs first.
    fn autocorrect_deferred_to(&self) -> &'static [&'static str] {
        &["Style/AndOr"]
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_return_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(ret) = node.as_return_node() else {
            return;
        };
        if !nil_argument(ret.arguments()) || iterator_exit(cx) {
            return;
        }
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: "Use `return` instead of `return nil`.".into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, cx, file))
                .flatten(),
        });
    }
}

pub(super) fn nil_argument(arguments: Option<ruby_prism::ArgumentsNode<'_>>) -> bool {
    arguments.is_some_and(|args| {
        args.arguments().len() == 1
            && args
                .arguments()
                .first()
                .is_some_and(|n| n.as_nil_node().is_some())
    })
}

#[cfg(test)]
thread_local! { static STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Defer the same potentially nonlocal iterator exits as upstream, regardless
/// of whether Lint/NonLocalExitFromIterator was selected. The main context walk
/// bounds work by AST depth, with no subtree rescans or reparsing.
fn iterator_exit(cx: &NodeContext<'_, '_>) -> bool {
    for ancestor in cx.ancestors() {
        #[cfg(test)]
        STEPS.with(|steps| steps.set(steps.get() + 1));
        if ancestor.as_def_node().is_some() {
            break;
        }
        if let Some(lambda) = ancestor.as_lambda_node() {
            if lambda.parameters().is_some_and(|p| {
                matches!(
                    p,
                    Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. }
                )
            }) {
                continue;
            }
            break;
        }
        let Some(call) = ancestor.as_call_node() else {
            continue;
        };
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            continue;
        };
        // Upstream searches :block, not :numblock/:itblock ancestors.
        if block
            .parameters()
            .is_some_and(|p| p.as_block_parameters_node().is_none())
        {
            continue;
        }
        if call.name().as_slice() == b"lambda"
            || (!call.is_safe_navigation()
                && matches!(
                    call.name().as_slice(),
                    b"define_method" | b"define_singleton_method"
                )
                && call.arguments().is_some_and(|a| a.arguments().len() == 1))
        {
            break;
        }
        let parameters = block.parameters().is_some_and(|p| {
            p.as_block_parameters_node()
                .is_none_or(|p| p.parameters().is_some() || !p.locals().is_empty())
        });
        if !call.is_safe_navigation() && call.receiver().is_some() && parameters {
            return true;
        }
    }
    false
}

fn correction(node: &Node<'_>, _cx: &NodeContext<'_, '_>, file: &SourceFile) -> Option<Correction> {
    let loc = node.location();
    let range = loc.start_offset()..loc.end_offset();
    // Removing embedded comments or physical lines changes directives, rescue
    // backtraces and __LINE__. Keep the finding but withhold those individual edits.
    if super::correction::contains_comment(file, range.clone())
        || loc.as_slice().contains(&b'\n')
        || file
            .bytes
            .get(range.end)
            .is_some_and(|b| !b.is_ascii_whitespace() && !b";,)]}:#".contains(b))
    {
        return None;
    }
    let correction = Correction {
        range,
        replacement: "return".into(),
    };
    Some(correction)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/return_nil_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str_with,
    };

    #[test]
    fn pinned_examples_and_canonical_correction() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/return_nil_pinned.json"
        ))
        .unwrap();
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found = lint_str_with(&[&ReturnNil], source);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(serde_json::json!(positions), case["positions"], "{source}");
            for offense in &found {
                assert_eq!(offense.message, "Use `return` instead of `return nil`.");
            }
            if let Some(after) = case["correction"].as_str() {
                assert_correction(&ReturnNil, source, after);
            }
        }
    }

    #[test]
    fn does_not_treat_values_or_spelling_as_nil() {
        for source in [
            "return",
            "return false",
            "return value",
            "return nil, nil",
            "return *[nil]",
            "return (side_effect; nil)",
            "def nil_value; end",
            "'return nil' # return nil",
        ] {
            assert!(lint_str_with(&[&ReturnNil], source).is_empty(), "{source}");
        }
    }

    #[test]
    fn prism_grouping_and_implicit_block_parameters_match_pinned_ast() {
        for source in ["return(nil)", "return (nil)", "return ((nil))"] {
            assert!(lint_str_with(&[&ReturnNil], source).is_empty(), "{source}");
        }
        for body in [
            "obj.each { _1; return nil }",
            "obj.each { it; return nil }",
            "obj&.each { |x| return nil }",
        ] {
            let source = format!("def f\n{body}\nend\n");
            assert_eq!(lint_str_with(&[&ReturnNil], &source).len(), 1, "{source}");
        }
        assert_correction(&ReturnNil, "(return nil)", "(return)");
    }

    #[test]
    fn iterator_def_lambda_and_dynamic_definition_boundaries() {
        for body in [
            "items.each { |x| return nil }",
            "items.each { |; local| return nil }",
            "items.each { |x| proc { return nil } }",
            "items.each { |x| lambda { _1; return nil } }",
            "items.each { |x| -> { _1; return nil } }",
            "items.each { |x| -> { it; return nil } }",
        ] {
            assert!(
                lint_str_with(&[&ReturnNil], &format!("def f\n{body}\nend\n")).is_empty(),
                "{body}"
            );
        }
        for body in [
            "items.each { return nil }",
            "each { |x| return nil }",
            "proc { return nil }",
            "-> { return nil }",
            "lambda { return nil }",
            "object.lambda { |x| return nil }",
            "items.each { |x| object&.lambda { |x| return nil } }",
            "define_method(:f) { |x| return nil }",
            "object.define_singleton_method(:f) { |x| return nil }",
            "items.each { |x| -> { return nil } }",
            "items.each { |x| lambda { return nil } }",
            "items.each { |x| def inner; return nil; end }",
        ] {
            let source = format!("def f\n{body}\nend\n");
            assert_eq!(lint_str_with(&[&ReturnNil], &source).len(), 1, "{source}");
            assert_correction(&ReturnNil, &source, &source.replace("return nil", "return"));
        }
    }

    #[test]
    fn local_corrections_preserve_control_expression_binding() {
        for body in [
            "x || (return nil)",
            "x or return nil",
            "x ? (return nil) : value",
            "return nil unless x",
            "begin; work; rescue; return nil; ensure; cleanup; end",
        ] {
            let source = format!("def f\n{body}\nend\n");
            assert_correction(&ReturnNil, &source, &source.replace("return nil", "return"));
        }
    }

    #[test]
    fn multiline_returns_stay_reported_but_unedited() {
        for source in ["return \\\n nil", "return \\\r\n nil"] {
            let found = correction_offenses(&ReturnNil, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn wide_sources_use_only_bounded_ancestor_queries() {
        let source = format!("def f\n{}end\n", "return nil if x\n".repeat(4000));
        STEPS.with(|s| s.set(0));
        assert_eq!(lint_str_with(&[&ReturnNil], &source).len(), 4000);
        STEPS.with(|s| assert!(s.get() <= 4000 * 5, "{}", s.get()));
    }

    #[test]
    fn detects_exact_range_and_preserves_trailing_comment() {
        let source = "def f\n  return nil if x # reason\nend\n";
        let found = correction_offenses(&ReturnNil, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(found[0].message, "Use `return` instead of `return nil`.");
        assert_correction(&ReturnNil, source, "def f\n  return if x # reason\nend\n");
    }
}
