//! `Style/CombinableLoops` — consecutive nonempty loops over the same data.
//!
//! Unsafe correction: fusion can change evaluation order, control flow and values.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct CombinableLoops;

impl Cop for CombinableLoops {
    fn name(&self) -> &'static str {
        "Style/CombinableLoops"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_statements_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(statements) = node.as_statements_node() else {
            return;
        };
        // A plain explicit begin/end (also in post-test loops) is kwbegin,
        // not RuboCop's implicit begin sequence. Rescue/ensure wrap a sequence.
        if cx
            .parent()
            .and_then(|n| n.as_begin_node())
            .is_some_and(|b| {
                b.begin_keyword_loc().is_some()
                    && b.rescue_clause().is_none()
                    && b.ensure_clause().is_none()
            })
        {
            return;
        }
        // Scan each statement list once, not the siblings anew for every loop.
        // Other multi-statement Prism lists correspond to a begin parent.
        let body: Vec<_> = statements.body().iter().collect();
        for (index, current) in body.iter().enumerate() {
            if index > 0 && same_loops(&body[index - 1], current, file) {
                let correction = cx
                    .corrections_requested()
                    .then(|| correction(&body, index, file))
                    .flatten();
                let loc = current.location();
                let (line, col) = file.line_col(loc.start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Combine this loop with the previous loop.".into(),
                    line,
                    col,
                    end_line: file.line_of(loc.end_offset().saturating_sub(1)),
                    correction,
                });
            }
        }
    }
}

/// Upstream's `combine_with_left_sibling`, when `combinable_blocks?` or
/// `combinable_fors?` holds for the loop at `index` and its left sibling.
fn correction(body: &[Node<'_>], index: usize, file: &SourceFile) -> Option<Correction> {
    let (node, left) = (&body[index], &body[index - 1]);
    if !combinable(node, left, file) {
        return None;
    }
    let bytes = &file.bytes;
    let span = |l: &ruby_prism::Location<'_>| l.start_offset()..l.end_offset();
    let start = node.location().start_offset();
    // closing_with_preceding_space: the line break is kept when the next
    // loop starts on the closing delimiter's line.
    let left_end = closing(left)?;
    let separate = file.line_of(left_end.start_offset()) != file.line_of(start);
    let mut from = left_end.start_offset();
    while from > 0 && matches!(bytes[from - 1], b' ' | b'\t') {
        from -= 1;
    }
    if separate {
        while from > 0 && bytes[from - 1] == b'\n' {
            from -= 1;
        }
        while from > 0 && is_space(bytes[from - 1]) {
            from -= 1;
        }
    }
    let mut edits = vec![(from..left_end.end_offset(), String::new())];
    // opening_with_following_space.
    let mut to = opening(node)?.end_offset();
    while matches!(bytes.get(to), Some(b' ' | b'\t')) {
        to += 1;
    }
    while bytes.get(to) == Some(&b'\n') {
        to += 1;
    }
    while bytes.get(to).is_some_and(|&b| is_space(b)) {
        to += 1;
    }
    edits.push((start..to, String::new()));
    // correct_end_of_block.
    if block(left).is_some() && !combined_with_right_sibling(body, index, file) {
        let mut first = index - 1;
        while first > 0
            && enabled(&body[first], file)
            && same_loops(&body[first - 1], &body[first], file)
            && combinable(&body[first], &body[first - 1], file)
        {
            first -= 1;
        }
        let braces = block(&body[first])?.opening_loc().as_slice() == b"{";
        let end = closing(node)?;
        edits.push((span(&end), String::new()));
        let at = node.location().end_offset();
        edits.push((at..at, if braces { "}" } else { " end" }.into()));
    }
    Correction::from_edits(bytes, edits)
}

/// Ruby's `\s`.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'\x0b')
}

fn block<'pr>(node: &Node<'pr>) -> Option<ruby_prism::BlockNode<'pr>> {
    node.as_call_node()?.block()?.as_block_node()
}

fn closing<'pr>(node: &Node<'pr>) -> Option<ruby_prism::Location<'pr>> {
    match node.as_for_node() {
        Some(f) => Some(f.end_keyword_loc()),
        None => Some(block(node)?.closing_loc()),
    }
}

/// `loop_opening`: a block's arguments or `{`/`do`, a `for`'s `do` or
/// collection.
fn opening<'pr>(node: &Node<'pr>) -> Option<ruby_prism::Location<'pr>> {
    if let Some(f) = node.as_for_node() {
        return Some(
            f.do_keyword_loc()
                .unwrap_or_else(|| f.collection().location()),
        );
    }
    let block = block(node)?;
    Some(match block.parameters() {
        Some(p) if p.as_block_parameters_node().is_some() => p.location(),
        _ => block.opening_loc(),
    })
}

fn loop_body<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    match node.as_for_node() {
        Some(f) => f.statements().map(|s| s.as_node()),
        None => block(node)?.body(),
    }
}

/// `combinable_blocks?` / `combinable_fors?`.
fn combinable(node: &Node<'_>, sibling: &Node<'_>, file: &SourceFile) -> bool {
    if directive_after_opening(node, file) {
        return false;
    }
    if let (Some(a), Some(b)) = (node.as_for_node(), sibling.as_for_node()) {
        return same_node(&a.index(), &b.index(), file);
    }
    let (Some(a), Some(b)) = (block(node), block(sibling)) else {
        return false;
    };
    let rescued = |n: &Node<'_>| {
        n.as_begin_node()
            .is_some_and(|b| b.rescue_clause().is_some() || b.ensure_clause().is_some())
    };
    same_parameters(a.parameters(), b.parameters(), file)
        && !a.body().is_some_and(|b| rescued(&b))
        && !b.body().is_some_and(|b| rescued(&b))
}

/// `arguments ==` and `argument_list ==`: numbered and `it` parameters have
/// empty arguments, unlike a plain block's (possibly empty) `args`.
fn same_parameters(a: Option<Node<'_>>, b: Option<Node<'_>>, file: &SourceFile) -> bool {
    enum Kind<'a, 'pr> {
        Plain(Option<&'a Node<'pr>>),
        Numbered(u8),
        It,
    }
    fn kind<'a, 'pr>(p: &'a Option<Node<'pr>>) -> Kind<'a, 'pr> {
        match p {
            None => Kind::Plain(None),
            Some(Node::NumberedParametersNode { .. }) => Kind::Numbered(
                p.as_ref()
                    .unwrap()
                    .as_numbered_parameters_node()
                    .unwrap()
                    .maximum(),
            ),
            Some(Node::ItParametersNode { .. }) => Kind::It,
            Some(n) => {
                let params = n.as_block_parameters_node();
                let empty = params
                    .as_ref()
                    .is_some_and(|b| b.parameters().is_none() && b.locals().is_empty());
                Kind::Plain((!empty).then_some(n))
            }
        }
    }
    match (kind(&a), kind(&b)) {
        (Kind::Plain(None), Kind::Plain(None)) | (Kind::It, Kind::It) => true,
        (Kind::Plain(Some(x)), Kind::Plain(Some(y))) => same_node(x, y, file),
        (Kind::Numbered(x), Kind::Numbered(y)) => x == y,
        _ => false,
    }
}

/// The opening is removed, so a directive comment on its line (other than
/// the body's first line) would cover the rest of the file.
fn directive_after_opening(node: &Node<'_>, file: &SourceFile) -> bool {
    let (Some(opening), Some(body)) = (opening(node), loop_body(node)) else {
        return false;
    };
    let line = file.line_of(opening.start_offset());
    if line == file.line_of(body.location().start_offset()) {
        return false;
    }
    let comments = file.comments();
    comments
        .iter()
        .find(|c| file.line_of(c.start) == line)
        .is_some_and(|c| {
            let text = &file.bytes[c.start + 1..c.end];
            let text = text.trim_ascii_start();
            text.strip_prefix(b"rubocop")
                .is_some_and(|rest| rest.trim_ascii_start().starts_with(b":"))
        })
}

fn enabled(node: &Node<'_>, file: &SourceFile) -> bool {
    let loc = node.location();
    file.cop_enabled_for_lines(
        "Style/CombinableLoops",
        file.line_of(loc.start_offset()),
        file.line_of(loc.end_offset().saturating_sub(1)),
    )
}

/// `combined_with_right_sibling?`: the next loop merges into this one.
fn combined_with_right_sibling(body: &[Node<'_>], index: usize, file: &SourceFile) -> bool {
    let Some(sibling) = body.get(index + 1) else {
        return false;
    };
    block(sibling).is_some()
        && enabled(sibling, file)
        && same_loops(&body[index], sibling, file)
        && combinable(sibling, &body[index], file)
}

#[cfg(test)]
thread_local! {
    static PAIR_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn same_loops(left: &Node<'_>, right: &Node<'_>, file: &SourceFile) -> bool {
    #[cfg(test)]
    PAIR_CHECKS.with(|count| count.set(count.get() + 1));
    if let (Some(left), Some(right)) = (left.as_for_node(), right.as_for_node()) {
        return left.statements().is_some()
            && right.statements().is_some()
            && same_node(&left.collection(), &right.collection(), file);
    }
    let (Some(left), Some(right)) = (left.as_call_node(), right.as_call_node()) else {
        return false;
    };
    let name = right.name().as_slice();
    (name.starts_with(b"each") || name.ends_with(b"_each"))
        && name == left.name().as_slice()
        && left
            .block()
            .and_then(|b| b.as_block_node())
            .is_some_and(|b| b.body().is_some())
        && right
            .block()
            .and_then(|b| b.as_block_node())
            .is_some_and(|b| b.body().is_some())
        && same_optional(left.receiver(), right.receiver(), file)
        && same_optional(
            left.arguments().map(|n| n.as_node()),
            right.arguments().map(|n| n.as_node()),
            file,
        )
}

fn same_optional(left: Option<Node<'_>>, right: Option<Node<'_>>, file: &SourceFile) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => same_node(&left, &right, file),
        _ => false,
    }
}

fn same_node(left: &Node<'_>, right: &Node<'_>, file: &SourceFile) -> bool {
    // Detection-only parser equality, including selector spellings, regexp
    // option sets, numeric values and cached __LINE__ values.
    ast::fingerprint_for_detection(left, file) == ast::fingerprint_for_detection(right, file)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/combinable_loops_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/CombinableLoops")
            .collect()
    }

    #[test]
    fn recognizes_method_prefix_suffix_and_structural_arguments() {
        for method in [
            "each",
            "each_with_index",
            "each_slice",
            "each_custom",
            "reverse_each",
            "custom_each",
        ] {
            let source =
                format!("items.{method}(0x2) {{ foo(_1) }}\nitems.{method}(2) do bar(it) end\n");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
        for source in [
            "each_slice(2) { foo }; each_slice 2 do bar end",
            "items&.each { foo }; items.each { bar }",
            "[1, 'a'].each { foo }; [0x1, \"a\"].each { bar }",
            "items.each { foo }; # comment\nitems.each { bar }; items.each { baz }",
        ] {
            let expected = if source.contains("baz") { 2 } else { 1 };
            assert_eq!(offenses(source).len(), expected, "{source}");
        }
    }

    #[test]
    fn equivalent_selector_spellings_and_regexp_option_sets() {
        for source in [
            "items[0].each { foo }\nitems.[](0).each { bar }",
            "items.each_slice(/a/im) { foo }\nitems.each_slice(/a/mi) { bar }",
            "items.each_slice(/#{pattern}/im) { foo }\nitems.each_slice(/#{pattern}/mi) { bar }",
            "for x in items[0]; foo; end\nfor y in items.[](0); bar; end",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 2));
            assert_eq!(
                found[0].message,
                "Combine this loop with the previous loop."
            );
            assert!(found[0].correction.is_none());
        }
        for source in [
            "items[0].each { foo }\nothers.[](0).each { bar }",
            "items[0].each { foo }\nitems.[](1).each { bar }",
            "items[0].each { foo }\nitems&.[](0).each { bar }",
            "items.each_slice(/a/im) { foo }\nitems.each_slice(/a/i) { bar }",
            "items.each_slice(/a/im) { foo }\nitems.each_slice(/b/mi) { bar }",
            "items.each_slice(/#{pattern}/im) { foo }\nitems.each_slice(/#{pattern}/i) { bar }",
            "items.each_slice(/a/n) { foo }\nitems.each_slice(/a/u) { bar }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn excludes_nonadjacent_empty_mixed_and_value_context_lookalikes() {
        for source in [
            "items.each { foo }; other; items.each { bar }",
            "items.each { foo }; others.each { bar }",
            "items.each(1) { foo }; items.each(2) { bar }",
            "items.each { foo }; items.reverse_each { bar }",
            "items.map { foo }; items.map { bar }",
            "items.teach { foo }; items.teach { bar }",
            "items.each {}; items.each { foo }",
            "items.each { foo }; items.each { # comment\n}",
            "for x in items; end; for y in items; foo; end",
            "for x in items; foo; end; for y in items; end",
            "for x in items; foo; end; items.each { bar }",
            "items.each { foo }; items.each(&proc {})",
            "x = items.each { foo }; y = items.each { bar }",
            "consume(items.each { foo }, items.each { bar })",
            "if ready; items.each { foo }; else; items.each { bar }; end",
            "items = []; items.each { foo }; self.items.each { bar }",
            "items.each { foo }; items = []; items.each { bar }",
            "while cond; foo; end; while cond; bar; end",
            "begin; foo; end while cond; begin; bar; end while cond",
            "until cond; foo; end; until cond; bar; end",
            "# items.each { foo }; items.each { bar }\n'items.each { foo }; items.each { bar }'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn nested_statement_lists_and_exception_bodies_are_checked() {
        for wrapper in [
            ("def f\n", "\nend"),
            ("if ready\n", "\nend"),
            ("begin\n", "\nrescue\nrecover\nend"),
            ("outer {\n", "\n}"),
            ("while ready\n", "\nend"),
            ("begin\n", "\nensure\ncleanup\nend"),
        ] {
            let source = format!(
                "{}items.each {{ foo }}\nitems.each {{ bar }}{}",
                wrapper.0, wrapper.1
            );
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 1));
        }
        for source in [
            "items.each do; foo; rescue; recover; end\nitems.each do; bar; ensure; cleanup; end",
            "items.each { break 1 }; items.each { next 2 }",
            "items.each { redo if ready }; items.each { return 1 }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn explicit_begin_sequences_are_kwbegin_not_implicit_begin_parents() {
        for suffix in ["end", "end while ready", "end until ready"] {
            for loops in [
                "items.each { foo }; items.each { bar }",
                "for x in items; foo; end; for y in items; bar; end",
            ] {
                let source = format!("begin\n{loops}\n{suffix}");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }

    #[test]
    fn sibling_comparisons_are_linear_and_do_not_search_past_barriers() {
        use super::{CombinableLoops, PAIR_CHECKS};
        use crate::cops::testing::correction_offenses;
        for n in [128, 1024] {
            let source = "items.each { foo }; barrier; items.each { bar }\n".repeat(n);
            PAIR_CHECKS.with(|count| count.set(0));
            let found = correction_offenses(&CombinableLoops, &source);
            assert_eq!(found.len(), n - 1);
            // Three top-level statements per repetition; all block bodies have
            // one statement, so they require no pair comparison. Each
            // correction compares one more left pair (first_combined_block).
            assert_eq!(PAIR_CHECKS.with(|count| count.get()), 3 * n - 1 + (n - 1));
            assert!(found.iter().all(|o| o.correction.is_some()));
        }
    }

    #[derive(serde::Deserialize)]
    struct Case {
        spec_line: u32,
        source: String,
        positions: Vec<(u32, u32)>,
        messages: Vec<String>,
    }

    #[test]
    fn every_pinned_example_has_exact_diagnostics_and_no_edits() {
        let cases: Vec<Case> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/combinable_loops_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 46);
        for c in cases {
            let found = offenses(&c.source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                c.positions,
                "spec line {}: {}",
                c.spec_line,
                c.source
            );
            assert_eq!(
                found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
                c.messages,
                "spec line {}",
                c.spec_line
            );
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn source_line_arguments_compare_by_evaluated_integer_value() {
        for source in [
            "each_slice(__LINE__) { foo }; each_slice(__LINE__) { bar }",
            "each_slice(__LINE__) { foo }\neach_slice(1) { bar }",
            "for a in [__LINE__]; foo; end; for b in [__LINE__]; bar; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        assert!(offenses("each_slice(__LINE__) { foo }\neach_slice(__LINE__) { bar }").is_empty());
    }

    #[test]
    fn consecutive_for_loops_ignore_iteration_variable_names() {
        let found =
            offenses("for item in items do foo(item) end\nfor other in items do bar(other) end\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 1));
        assert_eq!(
            found[0].message,
            "Combine this loop with the previous loop."
        );
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn consecutive_each_blocks_report_the_second_loop() {
        let found = offenses(
            "def f\n  items.each { |item| foo(item) }\n  items.each { |x| bar(x) }\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 3, 3));
        assert_eq!(
            found[0].message,
            "Combine this loop with the previous loop."
        );
        assert!(found[0].correction.is_none());
    }
}
