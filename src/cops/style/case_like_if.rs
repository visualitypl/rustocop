//! `Style/CaseLikeIf` — three or more branches comparing the same subject.
//! Detection only: changing comparison dispatch/evaluation count is unsafe.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};

#[cfg(test)]
thread_local! {
    static MATCH_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct CaseLikeIf;
impl Cop for CaseLikeIf {
    fn name(&self) -> &'static str {
        "Style/CaseLikeIf"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_if_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(first) = node.as_if_node() else {
            return;
        };
        if first.if_keyword_loc().is_none_or(|k| k.as_slice() != b"if")
            || first.end_keyword_loc().is_none()
        {
            return;
        }
        // Only actual elsif edges count, never an if in the else body. Each
        // chain is processed once: dispatch of its elsif nodes returns above.
        let mut predicates = vec![first.predicate()];
        let mut next = first.subsequent();
        while let Some(branch) = next.and_then(|n| n.as_if_node()) {
            if branch
                .if_keyword_loc()
                .is_none_or(|k| k.as_slice() != b"elsif")
            {
                break;
            }
            #[cfg(test)]
            MATCH_STEPS.with(|n| n.set(n.get() + 1));
            predicates.push(branch.predicate());
            next = branch.subsequent();
        }
        if predicates.len() < 3 {
            return;
        }
        let Some(target) = find_target(first.predicate()) else {
            return;
        };
        let target_key = ast::fingerprint_for_case_subject(&target, file);
        if predicates
            .into_iter()
            .any(|p| working_captures(&p) || !convertible(p, &target_key, file))
        {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Convert `if-elsif` to `case-when`.".into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}

// Prism has explicit parentheses and statement containers; parser's begin uses
// the first child for predicate descent and the last for binary operands.
fn grouped<'pr>(node: &Node<'pr>, last: bool) -> Option<Node<'pr>> {
    let body = node.as_parentheses_node()?.body()?;
    let statements = body.as_statements_node()?;
    if last {
        statements.body().iter().last()
    } else {
        statements.body().iter().next()
    }
}
fn deparenthesize(mut node: Node<'_>) -> Node<'_> {
    while let Some(inner) = grouped(&node, true) {
        node = inner;
    }
    node
}
fn send<'pr>(node: &Node<'pr>) -> Option<CallNode<'pr>> {
    let call = if let Some(m) = node.as_match_write_node() {
        m.call()
    } else {
        node.as_call_node()?
    };
    // Safe navigation is a csend; a literal block wraps the send in a block.
    (!call.is_safe_navigation() && !call.block().is_some_and(|b| b.as_block_node().is_some()))
        .then_some(call)
}
fn regexp(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::RegularExpressionNode { .. } | Node::InterpolatedRegularExpressionNode { .. }
    )
}
fn const_name<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    node.as_constant_read_node()
        .map(|n| n.name())
        .or_else(|| node.as_constant_path_node().and_then(|n| n.name()))
        .map(|n| n.as_slice())
}
fn constant_reference(node: &Node<'_>) -> bool {
    const_name(node).is_some_and(|name| {
        let name = String::from_utf8_lossy(name);
        name.chars().count() > 1 && name.to_uppercase() == name
    })
}
fn class_reference(node: &Node<'_>, file: &SourceFile) -> bool {
    const_name(node).is_some_and(|name| file.identifier_has_lowercase(name))
}
fn find_target(mut node: Node<'_>) -> Option<Node<'_>> {
    // Iterative left descent bounds stack usage for wide OR conditions.
    loop {
        if let Some(inner) = grouped(&node, false) {
            node = inner;
            continue;
        }
        if let Some(or) = node.as_or_node() {
            node = or.left();
            continue;
        }
        break;
    }
    let call = send(&node)?;
    let receiver = call.receiver()?;
    let args = ast::call_arguments(&call);
    let argument = args.into_iter().next();
    match call.name().as_slice() {
        b"is_a?" => Some(receiver),
        b"==" | b"eql?" | b"equal?" => {
            let argument = argument?;
            if ast::is_literal(&argument)
                || argument.as_source_encoding_node().is_some()
                || ast::negated_numeric_literal(&argument).is_some()
                || constant_reference(&argument)
            {
                Some(receiver)
            } else if ast::is_literal(&receiver)
                || receiver.as_source_encoding_node().is_some()
                || ast::negated_numeric_literal(&receiver).is_some()
                || constant_reference(&receiver)
            {
                Some(argument)
            } else {
                None
            }
        }
        b"===" => argument,
        b"include?" | b"cover?" => deparenthesize(receiver).as_range_node().and(argument),
        b"match" | b"match?" | b"=~" => {
            let argument = argument?;
            if regexp(&receiver) {
                Some(argument)
            } else if regexp(&argument) {
                Some(receiver)
            } else {
                None
            }
        }
        _ => None,
    }
}
fn convertible(node: Node<'_>, target: &ast::Fingerprint<'_>, file: &SourceFile) -> bool {
    let same = |n: &Node<'_>| ast::fingerprint_for_case_subject(n, file) == *target;
    let mut pending = vec![node];
    while let Some(node) = pending.pop() {
        #[cfg(test)]
        MATCH_STEPS.with(|n| n.set(n.get() + 1));
        if let Some(inner) = grouped(&node, false) {
            pending.push(inner);
            continue;
        }
        if let Some(or) = node.as_or_node() {
            pending.push(or.right());
            pending.push(or.left());
            continue;
        }
        let Some(call) = send(&node) else {
            return false;
        };
        let args = ast::call_arguments(&call);
        let Some(argument) = args.into_iter().next() else {
            return false;
        };
        let Some(receiver) = call.receiver() else {
            return false;
        };
        let condition = match call.name().as_slice() {
            b"is_a?" if same(&receiver) => Some(argument),
            b"===" if same(&argument) => Some(receiver),
            b"include?" | b"cover?" if same(&argument) => {
                let range = deparenthesize(receiver);
                range.as_range_node().map(|_| range)
            }
            name @ (b"==" | b"eql?" | b"equal?" | b"=~" | b"match" | b"match?") => {
                let lhs = deparenthesize(receiver);
                let rhs = deparenthesize(argument);
                let other = if same(&lhs) {
                    Some(rhs)
                } else if same(&rhs) {
                    Some(lhs)
                } else {
                    None
                };
                other.filter(|n| {
                    !matches!(name, b"==" | b"eql?" | b"equal?") || !class_reference(n, file)
                })
            }
            _ => None,
        };
        if condition.is_none() {
            return false;
        }
    }
    true
}
fn working_captures(node: &Node<'_>) -> bool {
    // Match the pinned guard at the branch root (not a recursive guard on OR
    // leaves). Regexp#match can expose captures from either operand; =~ only
    // binds locals with a literal regexp on the left.
    let Some(call) = send(node) else { return false };
    let has_named = |n: &Node<'_>| {
        if let Some(r) = n.as_regular_expression_node() {
            return crate::cops::lint::has_named_captures(
                r.content_loc().as_slice(),
                r.is_extended(),
            );
        }
        let Some(r) = n.as_interpolated_regular_expression_node() else {
            return false;
        };
        // Upstream blanks interpolations before scanning: a capture may span
        // static fragments, but Ruby code inside #{...} is never regexp text.
        let mut pattern = Vec::new();
        for part in r.parts().iter() {
            if part.as_string_node().is_some() {
                pattern.extend_from_slice(part.location().as_slice());
            } else {
                pattern.resize(pattern.len() + part.location().as_slice().len(), b' ');
            }
        }
        crate::cops::lint::has_named_captures(&pattern, r.is_extended())
    };
    let receiver = call.receiver();
    let args = ast::call_arguments(&call);
    match call.name().as_slice() {
        b"=~" => node.as_match_write_node().is_some() && receiver.as_ref().is_some_and(has_named),
        b"match" => receiver.as_ref().is_some_and(has_named) || args.first().is_some_and(has_named),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Ported/adapted from RuboCop style/case_like_if_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Two-branch positives are extended
    //! to three branches for our fixed MinBranchesCount=3.
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == CaseLikeIf.name())
            .collect()
    }
    fn chain(a: &str, b: &str, c: &str) -> String {
        format!("if {a}\n  :one\nelsif {b}\n  :two\nelsif {c}\n  :three\nelse\n  :other\nend\n")
    }
    fn assert_offense(s: &str, line: u32, col: u32) {
        let found = offenses(s);
        assert_eq!(found.len(), 1, "{s}");
        assert_eq!((found[0].line, found[0].col), (line, col), "{s}");
        assert_eq!(found[0].message, "Convert `if-elsif` to `case-when`.");
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn comparison_families_and_reversed_operands() {
        for (a, b, c) in [
            ("Integer === x", "/foo/ === x", "(1..10) === x"),
            ("x == 1", "'str' == x", "x == :sym"),
            ("x == CONSTANT1", "CONSTANT2 == x", "x == ::NS::CONSTANT3"),
            ("x.eql?(1)", "2.equal?(x)", "x == 3"),
            ("x.is_a?(Foo)", "x.is_a?(Bar)", "x.is_a?(Baz)"),
            ("/foo/.match?(x)", "x.match?(/bar/)", "x.match?(/baz/)"),
            ("/foo/ =~ x", "x =~ returns_regexp(arg)", "x.match(/baz/)"),
            ("/foo/.match?(x)", "returns_regexp(arg).match?(x)", "x == 1"),
            (
                "(1..10).include?(x)",
                "(11...100).cover?(x)",
                "(..0).include?(x)",
            ),
            (
                "Integer === x || x == 2",
                "1 == x || (1..10) === x || x.match?(/foo/)",
                "x == 3",
            ),
            (
                "Integer === x || ((x == 2) || (3 == x)) || x =~ /foo/",
                "x == 4",
                "x == 5",
            ),
            (
                "x.type == 1 || x.type == 2",
                "/foo/ === x.type",
                "x.type == 3",
            ),
        ] {
            assert_offense(&chain(a, b, c), 1, 1);
        }
        assert_offense(
            &format!(
                "def f(x)\n{}end\n",
                chain("x == 1", "x == 2", "x == 3")
                    .lines()
                    .map(|l| format!("  {l}\n"))
                    .collect::<String>()
            ),
            2,
            3,
        );
    }
    #[test]
    fn rejects_nonconvertible_branches_and_class_equality() {
        for (a, b, c) in [
            ("x == foo(1)", "bar(1) == x", "x == 3"),
            ("x == Foo", "Bar == x", "x == 3"),
            // POSIX lowercase need not change under Unicode upcase.
            ("ZZ == Aᵢ", "Aᵢ == 1", "Aᵢ == 2"),
            ("x == 1", "x == Foo", "x == 3"),
            ("x == F", "B == x", "x == 3"),
            ("equal?(Foo)", "Bar == x", "x == 3"),
            ("y.match?(x)", "x.match?('str')", "x == 3"),
            ("match?(/foo/)", "x.match?(/bar/)", "x == 3"),
            ("include?(Foo)", "include?(Bar)", "x == 3"),
            ("x == 1", "cover?(Bar)", "x == 3"),
            ("x =~ returns_regexp(arg)", "x =~ /foo/", "x == 3"),
            ("returns_regexp(arg).match?(x)", "x.match?(/bar/)", "x == 3"),
            ("Integer === x || (x == 2 && x == 3)", "x == 4", "x == 5"),
            ("x == 2", "3 == y", "x == 4"),
            ("x.equal?", "y", "x == 4"),
            ("x&.is_a?(Foo)", "x.is_a?(Bar)", "x.is_a?(Baz)"),
            ("x.is_a?(Foo) { effect }", "x.is_a?(Bar)", "x.is_a?(Baz)"),
        ] {
            let s = chain(a, b, c);
            assert!(offenses(&s).is_empty(), "{s}");
        }
    }
    #[test]
    fn only_real_elsif_branches_count() {
        for s in [
            "if x == 1; end",
            "if x == 1; else; end",
            "unless x == 1; else; end",
            "x == 1 ? y : z",
            "foo if x == 1",
            "if x == 1; elsif x == 2; else; end",
            "if x == 1; elsif x == 2; else; foo if x == 3; end",
            "if x == 1; elsif x == 2; else; if x == 3; end; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        let s = chain("x == 1", "x == 2", "x == 3");
        for body in [
            "if x == 4; :four; end",
            ":four if x == 4",
            "unless x == 4; :four; end",
            "x == 4 ? y : z",
        ] {
            assert_offense(&s.replace(":other", body), 1, 1);
        }
        assert_offense(&s.replace("else\n  :other\n", ""), 1, 1);
    }
    #[test]
    fn working_named_captures_are_exempt_but_predicate_matches_are_not() {
        for cond in [
            "/(?<name>.*)/ =~ x",
            "/(?<name>.*)/.match(x)",
            "x.match(/(?<name>.*)/)",
            "x.match(/(?'name'.*)/)",
        ] {
            let s = chain(cond, "x == 1", "x == 2");
            assert!(offenses(&s).is_empty(), "{s}");
            let s = chain("x == 0", cond, "x == 2");
            assert!(offenses(&s).is_empty(), "{s}");
        }
        for cond in [
            "x =~ /(?<name>.*)/",
            "/(?<name>.*)/.match?(x)",
            "x.match?(/(?<name>.*)/)",
            "x.match(/[(?<name>)]/)",
            "x.match(/\\(?<name>/)",
            "x.match(/# (?<name>.*)\nfoo/x)",
        ] {
            assert_offense(&chain(cond, "x == 1", "x == 2"), 1, 1);
        }
    }
    #[test]
    fn matching_work_is_bounded_for_siblings_long_chains_and_wide_disjunctions() {
        use crate::cops::testing::correction_offenses;
        let siblings = chain("x == 1", "x == 2", "x == 3").repeat(6000);
        // Deep syntax is intentionally rejected by the shared depth preflight.
        // Keep the long chain below that limit and balance the wide OR tree.
        let long = format!("if x == 0\n{}end\n", "elsif x == 1\n".repeat(48));
        fn balanced(count: usize) -> String {
            if count == 1 {
                return "x == 1".into();
            }
            format!(
                "({} || {})",
                balanced(count / 2),
                balanced(count - count / 2)
            )
        }
        let wide = chain(&balanced(1000), "x == 2", "x == 3");
        for (source, count, steps) in [(siblings, 6000, 30000), (long, 1, 97), (wide, 1, 3002)] {
            MATCH_STEPS.with(|n| n.set(0));
            let found = correction_offenses(&CaseLikeIf, &source);
            assert_eq!(found.len(), count, "source bytes: {}", source.len());
            assert!(found.iter().all(|o| o.correction.is_none()));
            MATCH_STEPS.with(|n| assert_eq!(n.get(), steps));
        }
    }

    #[test]
    fn encoding_literals_and_interpolated_capture_guards() {
        assert_offense(&chain("/(?<n>#{s})/ =~ x", "x == 1", "x == 2"), 1, 1);
        assert_offense(&chain("x == __ENCODING__", "x == 1", "x == 2"), 1, 1);
        for cond in [
            "x.match(/(?<n>#{s})/)",
            "/(?<n>#{s})/.match(x)",
            "x.match(/#{'(?<n>'}a/)",
        ] {
            let s = chain(cond, "x == 1", "x == 2");
            // Interpolation contents are Ruby code, not part of the pattern.
            if cond.contains("#{'") {
                assert_offense(&s, 1, 1);
            } else {
                assert!(offenses(&s).is_empty(), "{s}");
            }
        }
    }

    #[test]
    fn parser_equivalent_encoding_and_negation_subjects() {
        for (a, b, c) in [
            (
                "foo(Encoding::UTF_8) == 1",
                "foo(__ENCODING__) == 2",
                "foo(__ENCODING__) == 3",
            ),
            (
                "foo(__ENCODING__) == 1",
                "foo(Encoding::UTF_8) == 2",
                "foo(__ENCODING__) == 3",
            ),
            ("foo(!x) == 1", "foo(x.!) == 2", "foo(x.!()) == 3"),
            ("foo(x.!) == 1", "foo(not x) == 2", "foo(!x) == 3"),
        ] {
            assert_offense(&chain(a, b, c), 1, 1);
        }
        for (a, b, c) in [
            (
                "foo(Encoding::UTF_8) == 1",
                "foo(Encoding::US_ASCII) == 2",
                "foo(__ENCODING__) == 3",
            ),
            (
                "foo(Encoding::UTF_8) == 1",
                "foo(::Encoding::UTF_8) == 2",
                "foo(__ENCODING__) == 3",
            ),
            ("foo(!x) == 1", "foo(y.!) == 2", "foo(!x) == 3"),
            ("foo(!x) == 1", "foo(x&.!) == 2", "foo(!x) == 3"),
            ("foo(!x) == 1", "foo(x.!(1)) == 2", "foo(!x) == 3"),
            ("foo(!x) == 1", "foo(x.! { effect }) == 2", "foo(!x) == 3"),
        ] {
            let source = chain(a, b, c);
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn structural_subjects_and_local_shadowing() {
        assert_offense(
            &format!("x = 1\n{}", chain("x == 1", "x == 2", "x == 3")),
            2,
            1,
        );
        assert_offense(
            &chain("foo(0x10) == 1", "foo(16) == 2", "foo(0o20) == 3"),
            1,
            1,
        );
        assert_offense(
            &chain("foo(-0.0) == 1", "foo(0.0) == 2", "foo(0.0) == 3"),
            1,
            1,
        );
        let s = chain("foo(__LINE__) == 1", "foo(1) == 2", "foo(1) == 3");
        assert_offense(&s, 1, 1);
        for s in [
            chain("foo(1) == 1", "foo(2) == 2", "foo(1) == 3"),
            chain(
                "foo(__LINE__) == 1",
                "foo(__LINE__) == 2",
                "foo(__LINE__) == 3",
            ),
        ] {
            assert!(offenses(&s).is_empty(), "{s}");
        }
    }
}
