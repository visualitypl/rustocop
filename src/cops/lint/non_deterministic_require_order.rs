//! `Lint/NonDeterministicRequireOrder` — real legacy Dir/require matcher.
//! Ruby >=3.0 sorts glob results by default; the mandatory <=2.7 selection
//! gate keeps this rule inactive at our fixed3.4 target, even when forced.

use ruby_prism::{BlockNode, CallNode, Node};

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

pub struct NonDeterministicRequireOrder;

impl Cop for NonDeterministicRequireOrder {
    fn name(&self) -> &'static str {
        "Lint/NonDeterministicRequireOrder"
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
        if method_require_pass(&call) && unsorted(&call, true) {
            report(self, &call, cx, file, out);
        }
        // The pin searches every descendant for exactly require(lvar name),
        // not binding identity or reaching definitions. Prism already tells
        // lvars from method calls. VariableForce identity/flow would change
        // upstream behavior for shadowed or reassigned names; do not infer it.
        if call.is_safe_navigation()
            || call.receiver().is_some()
            || !matches!(call.name().as_slice(), b"require" | b"require_relative")
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let [arg] = args.as_slice() else { return };
        let Some(local) = arg.as_local_variable_read_node() else {
            return;
        };
        let mut child = node;
        let mut ancestors = cx.ancestors().peekable();
        while let Some(ancestor) = ancestors.next() {
            if let Some(block) = ancestor.as_block_node()
                // Identify this direct body edge, not structural equality
                // (which would walk the whole subtree for each require).
                && block.body().is_some_and(|body| {
                    std::mem::discriminant(&body) == std::mem::discriminant(child)
                        && body.location().start_offset() == child.location().start_offset()
                        && body.location().end_offset() == child.location().end_offset()
                })
                && parameter_matches(&block, local.name().as_slice())
                && let Some(parent) = ancestors.peek().and_then(|p| p.as_call_node())
                && unsorted(&parent, false)
            {
                report(self, &parent, cx, file, out);
            }
            child = ancestor;
        }
    }
}

fn parameter_matches(block: &BlockNode<'_>, name: &[u8]) -> bool {
    let Some(parameters) = block.parameters() else {
        return false;
    };
    if let Some(numbered) = parameters.as_numbered_parameters_node() {
        // Descendants can cross definition boundaries into numbered blocks
        // with a different maximum. Match only this block's argument list.
        return name.len() == 2
            && name[0] == b'_'
            && (b'1'..=b'9').contains(&name[1])
            && name[1] - b'0' <= numbered.maximum();
    }
    let Some(block_parameters) = parameters.as_block_parameters_node() else {
        return false;
    };
    let Some(parameters) = block_parameters.parameters() else {
        return false;
    };
    if !block_parameters.locals().is_empty()
        || !parameters.optionals().is_empty()
        || !parameters.posts().is_empty()
        || !parameters.keywords().is_empty()
        || parameters
            .rest()
            .is_some_and(|rest| rest.as_implicit_rest_node().is_none())
        || parameters.keyword_rest().is_some()
        || parameters.block().is_some()
    {
        return false;
    }
    if parameters.requireds().len() != 1 {
        return false;
    }
    parameters
        .requireds()
        .iter()
        .next()
        .and_then(|p| p.as_required_parameter_node())
        .is_some_and(|p| p.name().as_slice() == name)
}

fn dir_call(call: &CallNode<'_>, glob_only: bool) -> bool {
    !call.is_safe_navigation()
        && (call.name().as_slice() == b"glob" || (!glob_only && call.name().as_slice() == b"[]"))
        && call
            .receiver()
            .is_some_and(|r| ast::is_global_const(&r, &["Dir"]))
}

fn unsorted(call: &CallNode<'_>, pass: bool) -> bool {
    if dir_call(call, true) {
        return true;
    }
    if call.is_safe_navigation() || call.name().as_slice() != b"each" || call.arguments().is_some()
    {
        return false;
    }
    if !pass
        && call
            .block()
            .is_some_and(|b| b.as_block_argument_node().is_some())
    {
        return false;
    }
    call.receiver()
        .and_then(|r| r.as_call_node())
        .is_some_and(|receiver| {
            !receiver
                .block()
                .is_some_and(|b| b.as_block_node().is_some())
                && dir_call(&receiver, false)
        })
}

fn method_require_pass(call: &CallNode<'_>) -> bool {
    let Some(pass) = call.block().and_then(|b| b.as_block_argument_node()) else {
        return false;
    };
    let Some(method) = pass.expression().and_then(|e| e.as_call_node()) else {
        return false;
    };
    if method.is_safe_navigation()
        || method.receiver().is_some()
        || method.block().is_some()
        || method.name().as_slice() != b"method"
    {
        return false;
    }
    let args = ast::call_arguments(&method);
    let [arg] = args.as_slice() else { return false };
    arg.as_symbol_node()
        .is_some_and(|s| matches!(s.unescaped(), b"require" | b"require_relative"))
}

fn report(
    cop: &NonDeterministicRequireOrder,
    call: &CallNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let start = call.location().start_offset();
    let end = ast::source_end(&call.as_node());
    // The inner glob's method pass and outer each's body can share a start
    // without sharing a send range. Deduplicate the complete span, not the
    // diagnostic position or Prism's location (which includes literal blocks).
    // The walker owns this memo: cops stay stateless across files and runs.
    if !cx.require_order_reports.borrow_mut().insert((start, end)) {
        return;
    }
    let (line, col) = file.line_col(start);
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        message: "Sort files before requiring them.".into(),
        line,
        col,
        end_line: file.line_of(end.saturating_sub(1)),
        correction: None, // Unsafe upstream, and version-inactive at fixed3.4.
    });
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's non_deterministic_require_order_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::run_cops, offense::Offense, source::SourceFile};

    // Internal direct matcher seam, not a public target/version bypass.
    fn legacy(source: &str) -> Vec<Offense> {
        let parsed = crate::parser::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0, "{source}");
        let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        let mut out = vec![];
        assert!(run_cops(&[&NonDeterministicRequireOrder], &parsed, &file, &mut out).is_empty());
        out
    }

    #[test]
    fn real_legacy_require_loops_have_exact_positions_messages_and_no_edits() {
        for dir in ["Dir", "::Dir"] {
            for glob in ["['*.rb']", ".glob('*.rb', File::FNM_DOTMATCH)"] {
                for require in ["require", "require_relative"] {
                    for body in [
                        format!("|file|\n    {require} file"),
                        format!("\n    {require} _1"),
                    ] {
                        let source = format!("# legacy\n  {dir}{glob}.each do {body}\n  end\n");
                        let out = legacy(&source);
                        assert_eq!(out.len(), 1, "{source}");
                        assert_eq!((out[0].line, out[0].col, out[0].end_line), (2, 3, 2));
                        assert_eq!(out[0].message, "Sort files before requiring them.");
                        assert!(out[0].correction.is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn direct_glob_and_method_block_passes() {
        for source in [
            "Dir.glob('*.rb') { |file| require file }",
            "Dir.glob('*.rb') { require_relative _1 }",
            "Dir['*.rb'].each(&method(:require))",
            "::Dir.glob('*.rb').each(&method(:require_relative))",
            "Dir.glob(\n '*.rb',\n File::FNM_DOTMATCH,\n &method(:require)\n)",
        ] {
            let out = legacy(source);
            assert_eq!(out.len(), 1, "{source}");
            assert_eq!((out[0].line, out[0].col), (1, 1));
            assert_eq!(out[0].message, "Sort files before requiring them.");
            assert!(out[0].correction.is_none());
        }
    }

    #[test]
    fn receiver_block_arguments_do_not_hide_legacy_offenses() {
        for source in [
            "Dir.glob('*.rb', &nil).each { |file| require file }",
            "Dir.glob('*.rb', &nil).each(&method(:require))",
        ] {
            let out = legacy(source);
            assert_eq!(out.len(), 1, "{source}");
            assert_eq!((out[0].line, out[0].col), (1, 1));
            assert_eq!(out[0].message, "Sort files before requiring them.");
            assert!(out[0].correction.is_none());
        }
        for source in [
            "Dir.glob('*.rb') { work }.each { |file| require file }",
            "Dir.glob('*.rb') { work }.each(&method(:require))",
        ] {
            assert!(legacy(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn descendant_numbered_locals_must_belong_to_the_outer_argument_list() {
        let source = "Dir['*.rb'].each { puts _1; def helper; inner { require _2 }; end }";
        assert!(legacy(source).is_empty(), "{source}");
        // Preserve the pin's syntactic descendant matching within the maximum,
        // even across a definition boundary and despite different bindings.
        for source in [
            "Dir['*.rb'].each { puts _1; def helper; inner { require _1 }; end }",
            "Dir['*.rb'].each { puts _2; def helper; inner { require _2 }; end }",
        ] {
            assert_eq!(legacy(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn same_start_distinct_send_spans_both_report_without_changing_diagnostics() {
        for require in ["require", "require_relative"] {
            for body in [
                format!("|f| {require} f"),
                format!("|f| {require} f; {require} f"),
                format!("{require} _1; {require} _2; {require} _1"),
            ] {
                for (prefix, line, col) in [("", 1, 1), ("# legacy\n  ", 2, 3)] {
                    let source = format!(
                        "{prefix}Dir.glob('*.rb', &method(:{require})).each {{ {body} }}\n"
                    );
                    let out = legacy(&source);
                    assert_eq!(out.len(), 2, "{source}");
                    for offense in out {
                        assert_eq!(
                            (offense.line, offense.col, offense.end_line),
                            (line, col, line)
                        );
                        assert_eq!(offense.cop, "Lint/NonDeterministicRequireOrder");
                        assert_eq!(offense.message, "Sort files before requiring them.");
                        assert!(offense.correction.is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn full_send_span_dedup_preserves_multiline_call_ends_and_single_loop_matches() {
        for require in ["require", "require_relative"] {
            let source = format!(
                "Dir.glob(\n '*.rb', &method(:{require})\n).each {{ |f|\n {require} f\n {require} f\n}}\n"
            );
            let out = legacy(&source);
            assert_eq!(out.len(), 2, "{source}");
            assert!(out.iter().all(|o| (o.line, o.col, o.end_line) == (1, 1, 3)));
            assert!(out.iter().all(
                |o| o.message == "Sort files before requiring them." && o.correction.is_none()
            ));
            for source in [
                format!("Dir.glob('*.rb').each {{ |f| {require} f; {require} f }}"),
                format!("Dir.glob('*.rb').each {{ {require} _1; {require} _2; {require} _1 }}"),
            ] {
                let out = legacy(&source);
                assert_eq!(out.len(), 1, "{source}");
                assert_eq!((out[0].line, out[0].col, out[0].end_line), (1, 1, 1));
                assert_eq!(out[0].message, "Sort files before requiring them.");
                assert!(out[0].correction.is_none());
            }
        }
    }

    #[test]
    fn memo_keys_are_complete_send_spans_not_positions_or_literal_block_spans() {
        for (require, inner_end) in [("require", 35), ("require_relative", 44)] {
            let source =
                format!("Dir.glob('*.rb', &method(:{require})).each {{ |f| {require} f }}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
            let call = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .iter()
                .next()
                .unwrap()
                .as_call_node()
                .unwrap();
            let receiver = call.receiver().unwrap().as_call_node().unwrap();
            let ranges = Default::default();
            let cx = NodeContext {
                ancestors: &[],
                corrections_requested: true,
                require_order_reports: &ranges,
            };
            let mut out = vec![];
            for candidate in [&receiver, &call, &receiver, &call] {
                report(
                    &NonDeterministicRequireOrder,
                    candidate,
                    &cx,
                    &file,
                    &mut out,
                );
            }
            assert_eq!(out.len(), 2);
            let mut actual = ranges.into_inner().into_iter().collect::<Vec<_>>();
            actual.sort_unstable();
            assert_eq!(actual, [(0, inner_end), (0, inner_end + 5)]);
        }
    }

    #[test]
    fn repeated_runs_and_files_do_not_share_reported_spans_or_inspect_old_offenses() {
        let source = "Dir.glob('*.rb', &method(:require)).each { |f| require f }\n";
        let parsed = crate::parser::parse(source.as_bytes());
        let mut out = Vec::new();
        for path in ["first.rb", "first.rb", "second.rb"] {
            let file = SourceFile::new(path.into(), source.as_bytes().to_vec());
            let before = out.len();
            assert!(
                run_cops(&[&NonDeterministicRequireOrder], &parsed, &file, &mut out).is_empty()
            );
            assert_eq!(out.len() - before, 2, "{path}");
        }
    }

    #[test]
    fn no_offense_structural_lookalikes() {
        for source in [
            "Dir['*.rb'].sort.each { |file| require file }",
            "Dir.glob('*.rb').sort.each(&method(:require))",
            "Dir['*.rb'].each { |file| puts file }",
            "Dir['*.rb'].each { |file| require 'file' }",
            "Dir['*.rb'].each { |file| require other }",
            "Dir['*.rb'].each { |file| Kernel.require file }",
            "Dir['*.rb'].each { |file| require file, other }",
            "Dir['*.rb'].each { |file| require(file, &block) }",
            "Dir['*.rb'].each { |file, other| require file }",
            "Dir['*.rb'].each { |*files| require files }",
            "Dir['*.rb'].each { |file = nil| require file }",
            "Dir['*.rb'].each { |(file)| require file }",
            "Dir['*.rb'].each { |file; other| require file }",
            "Dir['*.rb'].each { || require file }",
            "Dir['*.rb'].each { require it }",
            "Dir['*.rb'].each { |file| }",
            "Dir['*.rb'].map { |file| require file }",
            "Dir['*.rb'] { |file| require file }",
            "Other::Dir['*.rb'].each { |file| require file }",
            "dir.glob('*.rb').each { |file| require file }",
            "Dir['*.rb']&.each { |file| require file }",
            "Dir&.glob('*.rb') { |file| require file }",
            "Dir['*.rb'].each(1) { |file| require file }",
            "Dir['*.rb'].each(&Kernel.method(:require))",
            "Dir['*.rb'].each(&method('require'))",
            "Dir['*.rb'].each(&method(:load))",
            "Dir['*.rb'].each(&method(:require, :extra))",
        ] {
            assert!(legacy(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn complete_descendant_search_is_syntactic_not_speculative_binding_flow() {
        for source in [
            "Dir['*.rb'].each { |file| if file; require file; end }",
            "Dir['*.rb'].each { |file| begin; work; rescue; require file; end }",
            "Dir['*.rb'].each { |file| inner { |file| require file } }",
            "Dir['*.rb'].each { |file| def helper(file); require file; end }",
            "Dir['*.rb'].each { |file| file = other; require file; require file }",
            "Dir['*.rb'].each { consume(require _2); require _1 }",
        ] {
            assert_eq!(legacy(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn every_legacy_pinned_spec_source_and_control_matches_actual_reference() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/non_deterministic_require_order.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 126);
        for row in rows.as_array().unwrap() {
            if row["legacy_version"].as_f64().unwrap() > 2.7 {
                continue;
            }
            let out = legacy(row["source"].as_str().unwrap());
            assert_eq!(
                serde_json::json!(out.iter().map(|o| [o.line, o.col]).collect::<Vec<_>>()),
                row["legacy_positions"],
                "{row}"
            );
            assert_eq!(
                serde_json::json!(out.iter().map(|o| &o.message).collect::<Vec<_>>()),
                row["legacy_messages"],
                "{row}"
            );
            assert!(out.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn large_bodies_and_sibling_loops_do_not_rescan_the_root_or_duplicate_findings() {
        let source = format!(
            "Dir['*.rb'].each {{ |file|\n{}\n}}",
            "require file\n".repeat(6000)
        );
        assert_eq!(legacy(&source).len(), 1);
        let source = "Dir['*.rb'].each { |file| require file }\n".repeat(1000);
        assert_eq!(legacy(&source).len(), 1000);
    }

    #[test]
    fn fixed_ruby34_pipeline_never_dispatches_the_legacy_matcher() {
        for source in [
            "Dir['*.rb'].each { |file| require file }",
            "Dir.glob('*.rb', sort: false).each(&method(:require))",
            "Dir.glob('*.rb') { require _1 }",
            "Dir.glob('*.rb', &method(:require)).each { |f| require f }\n",
            "Dir.glob('*.rb', &method(:require_relative)).each { |f| require_relative f }\n",
        ] {
            assert!(
                crate::pipeline::lint_str(source)
                    .iter()
                    .all(|o| o.cop != "Lint/NonDeterministicRequireOrder")
            );
        }
    }
}
