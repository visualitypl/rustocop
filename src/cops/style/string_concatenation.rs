//! `Style/StringConcatenation` — aggressive mode; unsafe correction.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::collections::BTreeMap;

pub struct StringConcatenation;
impl Cop for StringConcatenation {
    fn name(&self) -> &'static str {
        "Style/StringConcatenation"
    }
    fn unsafe_fixes(&self) -> bool {
        true
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
        // Report roots: the topmost plus, and each plus with a literal block.
        let mut roots = vec![(
            plus(node).unwrap(),
            node.location().start_offset(),
            ast::source_end(node),
        )];
        let mut pending = vec![(call, 0)];
        let mut reports = BTreeMap::<usize, (usize, usize)>::new();
        while let Some((part, root)) = pending.pop() {
            let (start, end) = (roots[root].1, roots[root].2);
            let arguments = ast::call_arguments(&part);
            if let (Some(receiver), [argument]) = (part.receiver(), arguments.as_slice())
                && (static_string(&receiver) || static_string(argument))
                && !line_end(&part, &receiver, argument)
            {
                // RuboCop's CLI deduplicates same-position reports (e.g. a
                // block-bearing inner plus and its outer concatenation).
                reports
                    .entry(start)
                    .and_modify(|old| {
                        if end > old.0 {
                            *old = (end, root);
                        }
                    })
                    .or_insert((end, root));
            }
            for child in part.receiver().iter().chain(arguments.iter()) {
                if let Some(call) = plus(child) {
                    let root = if literal_block(&call) {
                        roots.push((
                            plus(child).unwrap(),
                            child.location().start_offset(),
                            ast::source_end(child),
                        ));
                        roots.len() - 1
                    } else {
                        root
                    };
                    pending.push((call, root));
                }
            }
        }
        for (start, (end, root)) in reports {
            let correction = cx
                .corrections_requested()
                .then(|| correction(&roots[root].0, file))
                .flatten();
            let (line, col) = file.line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Prefer string interpolation to string concatenation.".into(),
                line,
                col,
                end_line: file.line_of(end.saturating_sub(1).max(start)),
                correction,
            });
        }
    }
}
/// Upstream's `replacement(parts)` for the topmost plus, unless a part is
/// multiline, a heredoc or contains a block.
fn correction(root: &CallNode<'_>, file: &SourceFile) -> Option<Correction> {
    let mut parts = Vec::new();
    collect_parts(root.as_node(), &mut parts);
    if parts.iter().any(|part| uncorrectable(part, file)) {
        return None;
    }
    let mut text = String::from("\"");
    for part in &parts {
        let adjusted = adjust(part, file);
        text.push_str(if adjusted == "\"" { "\\\"" } else { &adjusted });
    }
    text.push('"');
    let loc = root.location();
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: text,
    })
}

fn collect_parts<'pr>(node: Node<'pr>, parts: &mut Vec<Node<'pr>>) {
    match plus(&node).filter(|call| !literal_block(call)) {
        Some(call) => {
            if let Some(receiver) = call.receiver() {
                collect_parts(receiver, parts);
            }
            if let Some(argument) = ast::call_arguments(&call).into_iter().next() {
                collect_parts(argument, parts);
            }
        }
        None => parts.push(node),
    }
}

/// `part.multiline? || heredoc?(part) || part.each_descendant(:any_block)`;
/// a part's own literal block is the part itself upstream.
fn uncorrectable(part: &Node<'_>, file: &SourceFile) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Blocks<'a> {
        own: Option<usize>,
        found: &'a mut bool,
    }
    impl<'pr> NodeVisitor<'pr> for Blocks<'_> {
        fn enter(&mut self, node: Node<'pr>) {
            if matches!(node, Node::BlockNode { .. } | Node::LambdaNode { .. })
                && Some(node.location().start_offset()) != self.own
            {
                *self.found = true;
            }
        }
    }
    let loc = part.location();
    if file.line_of(loc.start_offset())
        != file.line_of(loc.end_offset().saturating_sub(1).max(loc.start_offset()))
    {
        return true;
    }
    let heredoc = match part {
        Node::StringNode { .. } => part.as_string_node().unwrap().opening_loc(),
        Node::InterpolatedStringNode { .. } => {
            part.as_interpolated_string_node().unwrap().opening_loc()
        }
        _ => None,
    };
    if heredoc.is_some_and(|o| o.as_slice().starts_with(b"<<")) {
        return true;
    }
    let own = match part {
        Node::LambdaNode { .. } => Some(loc.start_offset()),
        _ => part
            .as_call_node()
            .and_then(|c| c.block())
            .and_then(|b| b.as_block_node())
            .map(|b| b.location().start_offset()),
    };
    let mut found = false;
    walk(
        &mut Blocks {
            own,
            found: &mut found,
        },
        part,
    );
    found
}

/// Upstream's `adjust_str`.
fn adjust(part: &Node<'_>, file: &SourceFile) -> String {
    let source = || String::from_utf8_lossy(part.location().as_slice()).into_owned();
    let children = |nodes: &mut dyn Iterator<Item = Node<'_>>| {
        nodes.map(|child| adjust(&child, file)).collect::<String>()
    };
    match part {
        Node::StringNode { .. } => {
            let string = part.as_string_node().unwrap();
            let value = string.unescaped();
            match string.opening_loc().map(|o| o.as_slice()[0]) {
                Some(b'\'') => {
                    let mut out = Vec::new();
                    for (i, &b) in value.iter().enumerate() {
                        if matches!(b, b'\\' | b'"')
                            || (b == b'#' && matches!(value.get(i + 1), Some(b'{' | b'@' | b'$')))
                        {
                            out.push(b'\\');
                        }
                        out.push(b);
                    }
                    String::from_utf8_lossy(&out).into_owned()
                }
                Some(b'"') => {
                    let raw = part.location().as_slice();
                    String::from_utf8_lossy(&raw[1..raw.len() - 1]).into_owned()
                }
                _ => {
                    let mut out = Vec::new();
                    crate::cops::lint::duplicate_match_pattern::inspect_source(
                        value, file, &mut out,
                    );
                    String::from_utf8_lossy(&out[1..out.len() - 1]).into_owned()
                }
            }
        }
        Node::SourceFileNode { .. } => format!("#{{{}}}", source()),
        Node::InterpolatedStringNode { .. } => {
            children(&mut part.as_interpolated_string_node().unwrap().parts().iter())
        }
        Node::EmbeddedStatementsNode { .. } => {
            match part.as_embedded_statements_node().unwrap().statements() {
                Some(statements) => children(&mut statements.body().iter()),
                None => String::new(),
            }
        }
        Node::ParenthesesNode { .. } => match part.as_parentheses_node().unwrap().body() {
            Some(body) => match body.as_statements_node() {
                Some(statements) => children(&mut statements.body().iter()),
                None => adjust(&body, file),
            },
            None => String::new(),
        },
        Node::EmbeddedVariableNode { .. } => format!(
            "#{{{}}}",
            String::from_utf8_lossy(
                part.as_embedded_variable_node()
                    .unwrap()
                    .variable()
                    .location()
                    .as_slice()
            )
        ),
        _ => format!("#{{{}}}", source()),
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
