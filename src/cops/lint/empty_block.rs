//! `Lint/EmptyBlock` — empty ordinary blocks, allowing comments and lambdas/procs.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

pub struct EmptyBlock;

impl Cop for EmptyBlock {
    fn name(&self) -> &'static str {
        "Lint/EmptyBlock"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(block) = node.as_block_node() else {
            return;
        };
        if block.body().is_some() {
            return;
        }
        // Prism nests the literal block under its call (including super).
        // RuboCop reports the whole block expression, starting at that call.
        let Some(parent) = cx.parent() else {
            return;
        };
        if parent.as_call_node().is_some_and(|call| {
            // RuboCop AST's BlockNode#lambda? matches the method name alone;
            // its inherited proc? matcher requires a zero-argument send.
            call.name().as_slice() == b"lambda"
                || (ast::call_arguments(&call).is_empty()
                    && !call.is_safe_navigation()
                    && match call.name().as_slice() {
                        b"proc" => call.receiver().is_none(),
                        b"new" => call
                            .receiver()
                            .is_some_and(|r| ast::is_global_const(&r, &["Proc"])),
                        _ => false,
                    })
        }) {
            return;
        }
        let (line, col) = file.line_col(parent.location().start_offset());
        let end_line = file.last_line_of(block.location());
        if file.has_comment_in_lines(line..end_line + 1) {
            // A disabling directive is not an explanatory comment. Leave its
            // suppression to the pipeline rather than hiding the raw finding.
            let records = file.directive_records();
            let first = records.partition_point(|record| record.line < line);
            let disables = records.get(first).is_some_and(|record| {
                record.line == line
                    && matches!(
                        record.mode,
                        "disable" | "todo" | "disable-next" | "todo-next"
                    )
                    && record
                        .names
                        .iter()
                        .any(|n| matches!(n.as_str(), "Lint/EmptyBlock" | "Lint" | "all"))
            });
            if !disables {
                return;
            }
        }
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line,
            message: "Empty block detected.".into(),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/empty_block_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EmptyBlock")
            .collect()
    }

    #[test]
    fn flags_empty_ordinary_blocks_at_call_start() {
        for (source, line, col, end) in [
            ("items.each { |item| }", 1, 1, 1),
            ("  Foo.proc {}", 1, 3, 1),
            ("# header\n  object&.each do\n  end\n", 2, 3, 3),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end),
                "{source}"
            );
            assert_eq!(found[0].message, "Empty block detected.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn allows_comments_and_nonempty_bodies() {
        for source in [
            "items.each do |item|\n  # TODO: implement later\nend",
            "items.each { |item| } # TODO: implement later",
            "items.each { |item| puts item }",
            "items.each { nil }",
            "items.each { false }",
            "items.each { () }",
            "items.each { begin; end }",
            "items.each { ; nil; }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn allows_lambda_named_calls_and_exact_proc_constructors() {
        for source in [
            "lambda do\nend",
            "-> {}",
            "proc do\nend",
            "Proc.new {}",
            "::Proc.new {}",
            "object.lambda {}",
            "Kernel.lambda {}",
            "lambda(1) {}",
            "obj&.lambda(1) {}",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "Kernel.proc {}",
            "self.proc {}",
            "Other::Proc.new {}",
            "proc(1) {}",
            "Proc.new(1) {}",
            "Proc&.new {}",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn comments_use_physical_lines_not_text_or_byte_containment() {
        for source in [
            "each do # header\nend",
            "each do\nend # tail",
            "each {}; other # same line",
            "\"#{each {}}\" # same line",
            "each do\n=begin\ncomment\n=end\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "# before\neach {}",
            "each {}\n# after",
            "each(\"# text\") {}",
            "each(<<~TEXT) {}\n# literal\nTEXT\n",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn disabling_comment_retains_raw_finding_before_pipeline_suppression() {
        for prefix in ["rubocop", "rustocop"] {
            for mode in ["disable", "todo", "disable-next", "todo-next"] {
                for target in ["Lint/EmptyBlock", "Lint", "all"] {
                    let source = format!("each do # {prefix}:{mode} {target}\nend\n");
                    let file = SourceFile::new("example.rb".into(), source.into_bytes());
                    let parsed = crate::parser::parse(&file.bytes);
                    file.record_comments(&parsed);
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
                    let block = call.block().unwrap();
                    let parents = [call.as_node()];
                    let reports = Default::default();
                    let cx = NodeContext {
                        ancestors: &parents,
                        corrections_requested: false,
                        require_order_reports: &reports,
                    };
                    let mut found = Vec::new();
                    EmptyBlock.check_node(&block, &cx, &file, &mut found);
                    assert_eq!(found.len(), 1, "{prefix}:{mode} {target}");
                }
            }
        }
    }

    #[test]
    fn inspects_nested_and_noncall_blocks_once() {
        for source in [
            "each { other {} }",
            "def f; super {}; end",
            "def f; super() do; end; end",
            "consume(each {})",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "BEGIN {}",
            "END {}",
            "each(&block)",
            "module M; end",
            "def f; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
