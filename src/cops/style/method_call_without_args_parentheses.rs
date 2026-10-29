//! Empty invocation parentheses, retaining syntactically significant delimiters.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MethodCallWithoutArgsParentheses;
impl Cop for MethodCallWithoutArgsParentheses {
    fn name(&self) -> &'static str {
        "Style/MethodCallWithoutArgsParentheses"
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
        let (Some(open), Some(close), Some(message)) =
            (call.opening_loc(), call.closing_loc(), call.message_loc())
        else {
            return;
        };
        let name = call.name().as_slice();
        if open.as_slice() != b"("
            || !ast::call_arguments(&call).is_empty()
            || name.first().is_some_and(u8::is_ascii_uppercase)
            || message.as_slice() == b"not"
            || cx
                .parent()
                .is_some_and(|n| n.as_optional_parameter_node().is_some())
        {
            return;
        }
        // Without a separator, a following keyword (including do/end) would
        // become part of the selector: `foo()and true` -> `fooand true`.
        if file
            .bytes
            .get(close.end_offset())
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_' || !b.is_ascii())
        {
            return;
        }
        let range = open.start_offset()..close.end_offset();
        let comments = file.comments();
        let first = comments.partition_point(|c| c.end <= range.start);
        if comments.get(first).is_some_and(|c| c.start < range.end) {
            return;
        }
        let literal_block = call.block().is_some_and(|n| n.as_block_node().is_some());
        if call.receiver().is_none() {
            if name == b"it"
                && !literal_block
                && cx
                    .ancestors()
                    .find(|n| n.as_block_node().is_some() || n.as_lambda_node().is_some())
                    .is_some_and(|n| {
                        let params = n
                            .as_block_node()
                            .map(|b| b.parameters())
                            .unwrap_or_else(|| n.as_lambda_node().unwrap().parameters());
                        params.is_none_or(|p| p.as_block_parameters_node().is_none())
                    })
            {
                return;
            }
            let root = cx.ancestors().last().unwrap_or(node);
            let locals = file.visible_locals(root);
            if locals.same_name_assignment(&call, cx)
                || (!literal_block && locals.shadows(&call, cx))
            {
                return;
            }
        }
        {
            // Synthetic index assignment/target nodes are calls in RuboCop.
            macro_rules! index {
                ($access:ident) => {
                    if let Some(parent) = cx.parent().and_then(|n| n.$access()) {
                        let receiver: Option<Node<'_>> = parent.receiver().into();
                        if receiver.is_some_and(|n| {
                            n.location().start_offset() == call.location().start_offset()
                                && n.location().end_offset() == call.location().end_offset()
                        }) && parent.opening_loc().start_offset() > close.end_offset()
                        {
                            return;
                        }
                    }
                };
            }
            index!(as_index_target_node);
            index!(as_index_and_write_node);
            index!(as_index_or_write_node);
            index!(as_index_operator_write_node);
            // A following tight unary/operator argument or spaced index would
            // turn a no-argument call into a command. Use the existing AST edge
            // and local token gap, never reparse each candidate.
            if let Some(parent) = cx.parent().and_then(|n| n.as_call_node())
                && parent.receiver().is_some_and(|n| {
                    n.location().start_offset() == call.location().start_offset()
                        && n.location().end_offset() == call.location().end_offset()
                })
                && parent.call_operator_loc().is_none()
                && let Some(op) = parent.message_loc().or(parent.opening_loc())
            {
                let spaced = close.end_offset() < op.start_offset();
                let tight_rhs = ast::call_arguments(&parent)
                    .first()
                    .is_some_and(|arg| arg.location().start_offset() == op.end_offset());
                if spaced
                    && (matches!(parent.name().as_slice(), b"[]" | b"[]=")
                        || (tight_rhs
                            && matches!(
                                parent.name().as_slice(),
                                b"+" | b"-" | b"*" | b"**" | b"&" | b"/" | b"%"
                            )))
                {
                    return;
                }
            }
        }
        if cx
            .parent()
            .and_then(|n| n.as_if_node())
            .and_then(|n| n.then_keyword_loc())
            .is_some_and(|loc| loc.as_slice() == b"?" && loc.start_offset() == close.end_offset())
        {
            return;
        }
        let (line, col) = file.line_col(range.start);
        let multiline = file.bytes[range.clone()].contains(&b'\n');
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not use parentheses for method calls with no arguments.".into(),
            line,
            col,
            end_line: file.line_of(range.end - 1),
            // Removing physical lines can change __LINE__, heredoc content and
            // block/chaining continuation. Keep the finding; withhold that edit.
            correction: (cx.corrections_requested()
                && !multiline
                // Ruby's uppercase classification depends on source encoding.
                // A bare non-ASCII-leading selector could become a constant
                // read. Retain the offense, but do not guess its classification.
                && (call.receiver().is_some() || name.first().is_some_and(u8::is_ascii))
                && !(call.receiver().is_none()
                    && name == b"it"
                    && !literal_block
                    && cx
                        .ancestors()
                        .any(|n| n.as_block_node().is_some() || n.as_lambda_node().is_some())))
            .then_some(Correction {
                range,
                replacement: String::new(),
            })
            .filter(|edit| super::correction::preserves_data_positions(file, cx, edit)),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's corresponding Style specs (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/MethodCallWithoutArgsParentheses")
            .collect()
    }
    #[test]
    fn data_byte_guard_keeps_empty_call_findings() {
        use crate::cops::testing::correction_offenses;
        for call in ["foo()", "foo(  )", "foo() {}", "foo() do; end"] {
            let source = format!("obj.{call}\np Object.const_get(:DATA).tell\n__END__\nraw\n");
            let found = correction_offenses(&MethodCallWithoutArgsParentheses, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 8, 1));
            assert_eq!(
                found[0].message,
                "Do not use parentheses for method calls with no arguments."
            );
            assert!(found[0].correction.is_none(), "{source}");
        }
        // A binding-sensitive lookalike stays exempt, not a new finding.
        assert!(offenses("foo = 1; foo()\np DATA.pos\n__END__\nraw\n").is_empty());
    }

    #[test]
    fn pinned_offense_and_canonical_correction() {
        let found = offenses("top.test()\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 9, 1));
        assert_eq!(
            found[0].message,
            "Do not use parentheses for method calls with no arguments."
        );
        assert_correction(
            &MethodCallWithoutArgsParentheses,
            "top.test()\n",
            "top.test\n",
        );
    }
    #[test]
    fn receiver_and_navigation_delimiters_preserve_operator_binding() {
        for source in [
            "obj.foo() -1",
            "obj&.foo() [1]",
            "obj.foo() /bar",
            "obj.foo() **kw",
            "obj.foo() &block",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn tight_ternary_selector_must_not_merge_with_method_name() {
        for source in ["foo()? 1 : 2", "obj.foo()? 1 : 2", "foo()?(1):(2)"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn adjacent_keywords_must_not_merge_with_the_selector() {
        for source in [
            "foo()and true",
            "foo()or true",
            "foo()if true",
            "foo()unless true",
            "foo()while true",
            "foo()until true",
            "foo()rescue true",
            "foo()do; end",
            "begin; foo()end",
            "if foo()then; end",
            "obj.foo()and true",
            "obj&.foo()do; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_correction(
            &MethodCallWithoutArgsParentheses,
            "foo() and true",
            "foo and true",
        );
    }

    #[test]
    fn uncertain_non_ascii_bare_selectors_keep_method_binding() {
        use crate::cops::testing::correction_offenses;
        for source in ["É()", "Ж()", "é()"] {
            let found = correction_offenses(&MethodCallWithoutArgsParentheses, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(&MethodCallWithoutArgsParentheses, "obj.É()", "obj.É");
        assert_correction(&MethodCallWithoutArgsParentheses, "fÉ()", "fÉ");
    }

    #[test]
    fn pinned_assignments_chains_and_constructors_correct() {
        for (before, after, line, col) in [
            ("test = x.test()", "test = x.test", 1, 14),
            ("test = x&.test()", "test = x&.test", 1, 15),
            ("obj.method ||= func()", "obj.method ||= func", 1, 20),
            ("obj.method &&= func()", "obj.method &&= func", 1, 20),
            ("obj.method += func()", "obj.method += func", 1, 19),
            ("p test()\ntest = 1", "p test\ntest = 1", 1, 7),
            ("c[2], x = c()", "c[2], x = c", 1, 12),
            ("h[:a], h[:b] = c()", "h[:a], h[:b] = c", 1, 17),
            ("_a, _b = d&.e()", "_a, _b = d&.e", 1, 14),
            ("Hash.new()", "Hash.new", 1, 9),
            ("def f; it(); end", "def f; it; end", 1, 10),
            ("0.times { obj&.it() }", "0.times { obj&.it }", 1, 18),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{before}");
            assert_correction(&MethodCallWithoutArgsParentheses, before, after);
        }
    }

    #[test]
    fn scope_boundaries_patterns_parameters_and_blocks_keep_call_binding() {
        for source in [
            "test ||= test()",
            "one, test = 1, test()",
            "if cond then test = 2 end; p test()",
            "test=1; items.each { p test() }",
            "test = begin; case a; when b; c = test() if d; end; end",
            "items.each { |test| test() }",
            "items.each { |;test| test() }",
            "/(?<test>x)/ =~ s; test()",
            "[1] => [test]; test()",
            "0.times { _1 + it() }",
            "0.times { it(); it=1; it }",
            "test = test() {}",
            "test=1; x, test = 1, test() {}",
            "obj.test() [1] = 2",
            "obj.test() [1] ||= 2",
            "obj.test() [1] += 2",
            "obj.test() [1], x = values",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for (before, after) in [
            (
                "def a; test=1; end; def b; test(); end",
                "def a; test=1; end; def b; test; end",
            ),
            ("test=1; class C; test(); end", "test=1; class C; test; end"),
            (
                "items.each { test() }; test=1",
                "items.each { test }; test=1",
            ),
            ("test=1; test() {}", "test=1; test {}"),
        ] {
            assert_correction(&MethodCallWithoutArgsParentheses, before, after);
        }
    }

    #[test]
    fn heredoc_scope_bodies_preserve_local_versus_method_binding() {
        for (open, close) in [
            ("class C", "end"),
            ("module M", "end"),
            ("class << self", "end"),
            ("module M; class C", "end; end"),
            ("module M; end; class M::C", "end"),
            ("class C; test=0; end; class C", "end"),
        ] {
            let source = format!("{open}; test=1; X=<<~TXT\n#{{test()}}\nTXT\n{close}\n");
            assert!(offenses(&source).is_empty(), "{source}");
            assert!(
                crate::cops::testing::correction_offenses(
                    &MethodCallWithoutArgsParentheses,
                    &source
                )
                .is_empty(),
                "{source}"
            );

            let source = format!("test=1; {open}; X=<<~TXT\n#{{test()}}\nTXT\n{close}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 7, 2));
            assert_eq!(
                found[0].message,
                "Do not use parentheses for method calls with no arguments."
            );
            assert_correction(
                &MethodCallWithoutArgsParentheses,
                &source,
                &source.replace("test()", "test"),
            );
        }
    }

    #[test]
    fn heredoc_header_calls_use_outer_not_body_or_parameter_locals() {
        for header in [
            "class C < choose(<<~TXT)",
            "class choose(<<~TXT)::C",
            "module choose(<<~TXT)::M",
            "class << choose(<<~TXT)",
            "def (choose(<<~TXT)).method",
            "def (choose(<<~TXT)).method(test)",
        ] {
            let source = format!("test=1; {header}; end\n#{{test()}}\nTXT\n");
            assert!(offenses(&source).is_empty(), "{source}");
            let source = format!("{header}; test=1; end\n#{{test()}}\nTXT\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 7, 2));
            assert_correction(
                &MethodCallWithoutArgsParentheses,
                &source,
                &source.replace("test()", "test"),
            );
        }
    }

    #[test]
    fn heredoc_bodies_beyond_inline_ends_and_nested_block_captures_are_not_headers() {
        for source in [
            "class C; test=1; X=<<~TXT; end\n#{test()}\nTXT\n",
            "class C; test=1; X=<<~TXT; rescue; end\n#{test()}\nTXT\n",
            "class C; test=1; X=<<~TXT\n#{[1].map { test() }}\nTXT\nend",
            "class C; X=<<~TXT\n#{[1].map { |test| test() }}\nTXT\nend",
            "class C; test=1; X=<<~TXT\n#{-> { test() }}\nTXT\nend",
            "def f(test) = <<~TXT\n#{test()}\nTXT\n",
            "class C; test=1; X=<<~OUTER\n#{<<~INNER}\n#{test()}\nINNER\nOUTER\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn explicit_block_parameters_still_cannot_safely_remove_bare_it_delimiters() {
        use crate::cops::testing::correction_offenses;
        for source in ["0.times { || it() }", "0.times { |_n| it() }"] {
            let found = correction_offenses(&MethodCallWithoutArgsParentheses, source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn multiline_pinned_findings_are_retained_without_line_removal() {
        use crate::cops::testing::correction_offenses;
        for before in ["obj.do_something(\n)\n", "obj.do_something(\n) do\nend\n"] {
            let found = correction_offenses(&MethodCallWithoutArgsParentheses, before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 17, 2));
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "Test()",
            "thing.()",
            "not(something)",
            "top.test(a)",
            "test = test()",
            "test = 1; test()",
            "def f(test); test(); end",
            "p test() -1",
            "p test() [1]",
            "0.times { it() }",
            "def f(x = foo()); end",
            "obj.foo(\n # comment\n)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
