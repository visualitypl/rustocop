//! `Style/StringConcatenation` — aggressive, detection-only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::collections::BTreeMap;

pub struct StringConcatenation;
impl Cop for StringConcatenation {
    fn name(&self) -> &'static str {
        "Style/StringConcatenation"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = plus(node) else { return };
        // Process connected plus nodes once. ArgumentsNode is transparent;
        // parentheses and block bodies are boundaries. A call's own literal
        // block introduces a new report root, but does not require a rescan.
        if cx
            .ancestors()
            .find(|p| p.as_arguments_node().is_none())
            .is_some_and(|p| plus(p).is_some())
        {
            return;
        }
        let mut pending = vec![(call, node.location().start_offset(), ast::source_end(node))];
        let mut reports = BTreeMap::<usize, usize>::new();
        while let Some((part, start, end)) = pending.pop() {
            let arguments = ast::call_arguments(&part);
            if let (Some(receiver), [argument]) = (part.receiver(), arguments.as_slice())
                && (static_string(&receiver) || static_string(argument))
                && !line_end(&part, &receiver, argument)
            {
                // RuboCop's CLI deduplicates same-position reports (e.g. a
                // block-bearing inner plus and its outer concatenation).
                reports
                    .entry(start)
                    .and_modify(|old| *old = (*old).max(end))
                    .or_insert(end);
            }
            for child in part.receiver().iter().chain(arguments.iter()) {
                if let Some(call) = plus(child) {
                    let (start, end) = if literal_block(&call) {
                        (child.location().start_offset(), ast::source_end(child))
                    } else {
                        (start, end)
                    };
                    pending.push((call, start, end));
                }
            }
        }
        for (start, end) in reports {
            let (line, col) = file.line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Prefer string interpolation to string concatenation.".into(),
                line,
                col,
                end_line: file.line_of(end.saturating_sub(1).max(start)),
                correction: None,
            });
        }
    }
}
fn plus<'pr>(node: &Node<'pr>) -> Option<CallNode<'pr>> {
    node.as_call_node()
        .filter(|c| c.name().as_slice() == b"+" && !c.is_safe_navigation())
}
fn literal_block(call: &CallNode<'_>) -> bool {
    call.block().is_some_and(|b| b.as_block_node().is_some())
}
// RuboCop's matcher uses str_type?, NOT {str dstr}. Implicit literal
// concatenation is dstr as well. __FILE__ is a synthetic str upstream.
fn static_string(node: &Node<'_>) -> bool {
    matches!(node, Node::StringNode { .. } | Node::SourceFileNode { .. })
}
fn line_end(call: &CallNode<'_>, receiver: &Node<'_>, argument: &Node<'_>) -> bool {
    if !static_string(receiver) || !static_string(argument) {
        return false;
    }
    let bytes = call.location().as_slice();
    bytes.iter().enumerate().any(|(i, b)| {
        *b == b'+'
            && bytes[i + 1..]
                .iter()
                .take_while(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0b' | b'\x0c'))
                .any(|b| *b == b'\n')
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `style/string_concatenation_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/StringConcatenation")
            .collect()
    }
    fn assert_at(s: &str, positions: &[(u32, u32)]) {
        let found = offenses(s);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            positions,
            "{s}"
        );
        for o in found {
            assert_eq!(
                o.message,
                "Prefer string interpolation to string concatenation."
            );
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn pinned_chains_and_expression_boundaries() {
        for (s, col) in [
            ("email_with_name = user.name + ' <' + user.email + '>'", 19),
            ("users = (user.name + ' ' + user.email) * 5", 10),
            (r#""\x0a" + "test" + "A" + "\xff""#, 1),
            (r##""foo" + "\"#{bar}\"" + 'baz'"##, 1),
            ("'(' + values.map { |v| v.titleize }.join(', ') + ')'", 1),
            ("'(' + values.map { _1.titleize }.join(', ') + ')'", 1),
            ("__FILE__ + '/c'", 1),
            (r#""__FILE__#{c}" + 'd'"#, 1),
            ("Pathname.new('/') + 'test'", 1),
            ("%(He said ) + %(\"Arrest that man!\".)", 1),
        ] {
            assert_at(s, &[(1, col)]);
        }
        assert_at(
            "(user.vip? ? greeting + ', ' : '') + user.name + ' <' + user.email + '>'",
            &[(1, 1), (1, 14)],
        );
    }
    #[test]
    fn implicit_and_interpolated_literals() {
        for s in [
            "\"a\" + \"b\" \"c\"",
            "\"a\" \"b\" + \"c\"",
            "\"a\" + \"b\" \"c\" + \"d\"",
            r#""string #{interpolation}" 'foo' + 'bar'"#,
            r#""foo" + "bar: #{baz}""#,
            r#"'"' + "foo" + '"'"#,
            r#""foo" + '#{bar}'"#,
            r#""foo" + '#@bar'"#,
            r#""foo" + '#@@bar'"#,
            r#""foo" + '#$bar'"#,
        ] {
            assert_at(s, &[(1, 1)]);
        }
        // Upstream's matcher requires at least one str, not merely dstr.
        for s in [
            r##""#{x}" + y"##,
            r#""a" "b" + y"#,
            r##""#{x}" + "#{y}""##,
            "x + y",
            "x - 's'",
            "`echo hi` + x",
            "x + %x(echo hi)",
        ] {
            assert_at(s, &[]);
        }
    }
    #[test]
    fn multiline_and_heredoc_detection_without_rewrites() {
        assert_at(
            "\"this is a long string \" +\n  \"this is a continuation\"",
            &[],
        );
        assert_at("user.name +\n  ' ' +\n  user.email +\n  '\\n'", &[(1, 1)]);
        assert_at(
            "\"result:\" + if condition\n  'true'\nelse\n  'false'\nend",
            &[(1, 1)],
        );
        assert_at("\"foo\" + <<~STR\n  text\nSTR\n", &[(1, 1)]);
        assert_at(
            "'(' + values.map do |v|\n  v.titleize\nend.join(', ') + ')'",
            &[(1, 1)],
        );
        assert_at("'a' +\n 'b' + x", &[]);
        assert_at("'a' +\n 'b' + 'c'", &[(1, 1)]);
        assert_at("('a' + 'b') + x", &[(1, 2)]);
    }
    #[test]
    fn block_boundaries_with_identical_start_positions() {
        assert_at("'a'.+(x) { y } + 'x'", &[(1, 1)]);
        assert_at("'a'.+(x) { y } + z", &[(1, 1)]);
        assert_at("'a'.+(x) { 'b' + z } + 'x'", &[(1, 1), (1, 12)]);
    }
    #[test]
    fn many_disjoint_chains_and_continuation_precedence() {
        let source = "a + 'b' + c + 'd'\n".repeat(1000);
        assert_eq!(offenses(&source).len(), 1000);
        assert_at("'a' +\\\n 'b'", &[(1, 1)]);
        assert_at("'a' + # continued\n 'b'", &[(1, 1)]);
        assert_at("'a' +\r\n 'b'", &[]);
        assert_at("'a' +\x0b\n 'b'", &[]);
        assert_at("'a' + \"#{x}\" +\n 'b'", &[(1, 1)]);
    }
    #[test]
    fn call_shapes_and_no_duplicate_chain_reports() {
        assert_at("x.+('a') + y + 'b'", &[(1, 1)]);
        assert_at("x&.+('a')", &[]);
        assert_at("x.+('a', 'b')", &[]);
        assert_at("x.+('a', &block)", &[]);
        assert_at("'a'.+(x) { y }", &[(1, 1)]);
        assert_at("'a' + ('b' + x)", &[(1, 1), (1, 8)]);
        assert_at("x + 'a' - y + z", &[(1, 1)]);
    }
}
