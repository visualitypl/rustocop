//! `Style/BlockDelimiters`: fixed `line_count_based`, with atomic token fixes.
//! Uncertain binding/loop/rescue edits remain reported without corrections.

use ruby_prism::{BlockNode, Node};

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct BlockDelimiters;

#[cfg(test)]
thread_local! {
    static COPIED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Cop for BlockDelimiters {
    fn name(&self) -> &'static str {
        "Style/BlockDelimiters"
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
        let Some(block) = offending_block(node, cx.parent(), file) else {
            return;
        };
        if ignored_by_ancestors(node, cx.ancestors(), file) {
            return;
        }
        let multiline = block.opening_loc().as_slice() == b"{";
        let (line, col) = file.line_col(block.opening_loc().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: if multiline {
                "Avoid using `{...}` for multi-line blocks."
            } else {
                "Prefer `{...}` over `do...end` for single-line blocks."
            }
            .into(),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| correction(node, &block, cx, file))
                .flatten()
                .filter(|edit| super::correction::preserves_data_positions(file, cx, edit)),
        });
    }
}

/// Replace both delimiters as one edit: a peer or disabled line must never
/// select only half a block. No reparse, comment relocation or newline changes.
/// Candidate spans cannot nest (detection suppresses descendants of offenders),
/// so the copied source work is bounded by the input size.
fn correction(
    node: &Node<'_>,
    block: &BlockNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let opening = block.opening_loc();
    let closing = block.closing_loc();
    let braces = opening.as_slice() == b"{";
    if braces {
        // `do` in a loop predicate can become the loop-body delimiter, even
        // inside parentheses. A nested block/lambda body resets that context.
        let mut child = node;
        let mut reachable_block = true;
        for parent in cx.ancestors() {
            if matches!(parent, Node::BlockNode { .. } | Node::LambdaNode { .. }) {
                break;
            }
            // Detection deliberately follows upstream's narrower traversal,
            // which stops at splats. Corrections must also account for * / **:
            // `outer *inner { ... }` becomes `outer *inner do ... end`,
            // transferring the block to outer (likewise for command super).
            if reachable_block && child.as_arguments_node().is_some() && binding_sensitive(parent) {
                return None;
            }
            // RuboCop's operator exemption includes explicit sends such as
            // `a.+ b { ... }`. Unlike infix `a + b`, changing that block to
            // do/end binds it to `a.+`. Keep this individual offense unedited.
            if child.as_arguments_node().is_some()
                && parent.as_call_node().is_some_and(|call| {
                    ast::is_operator_method(call.name().as_slice())
                        && call.call_operator_loc().is_some()
                        && call.opening_loc().is_none()
                })
            {
                return None;
            }
            let predicate = if let Some(loop_node) = parent.as_while_node() {
                Some(loop_node.predicate())
            } else {
                parent
                    .as_until_node()
                    .map(|loop_node| loop_node.predicate())
            };
            if predicate.is_some_and(|predicate| {
                predicate.location().start_offset() == child.location().start_offset()
                    && predicate.location().end_offset() == child.location().end_offset()
            }) {
                return None;
            }
            reachable_block = match parent {
                Node::CallNode { .. } => literal_block(parent).is_some() || reachable_block,
                Node::SuperNode { .. } | Node::ForwardingSuperNode { .. } => {
                    literal_block(parent).is_some()
                }
                Node::ArgumentsNode { .. }
                | Node::KeywordHashNode { .. }
                | Node::AssocNode { .. }
                | Node::SplatNode { .. }
                | Node::AssocSplatNode { .. } => reachable_block,
                _ => false,
            };
            child = parent;
        }
    } else {
        // With command arguments, braces bind to the last argument rather
        // than the original owner (`f g do ... end`). Match upstream's
        // conservative withholding, including super's command arguments.
        if node
            .as_call_node()
            .is_some_and(|call| call.arguments().is_some() && call.opening_loc().is_none())
            || node
                .as_super_node()
                .is_some_and(|call| call.arguments().is_some() && call.lparen_loc().is_none())
        {
            return None;
        }
        // The detection heuristic also reports `expr; rescue; fallback`.
        // Unlike a modifier rescue it requires begin/end inside braces;
        // withhold rather than change lines or rescue scope.
        if block
            .body()
            .and_then(|body| body.as_begin_node())
            .is_some_and(|body| body.begin_keyword_loc().is_none())
        {
            return None;
        }
    }
    let start = opening.start_offset();
    let end = closing.end_offset();
    let mut open = if braces { "do" } else { "{" }.to_owned();
    let mut close = if braces { "end" } else { "}" }.to_owned();
    if braces && start > 0 && !file.bytes[start - 1].is_ascii_whitespace() {
        open.insert(0, ' ');
    }
    if !file.bytes[opening.end_offset()].is_ascii_whitespace() {
        open.push(' ');
    }
    if braces && !file.bytes[closing.start_offset() - 1].is_ascii_whitespace() {
        close.insert(0, ' ');
    }
    // A brace itself separates tokens; `end` does not (`}then`, `}end`,
    // `}?`, `}!=`). Question/exclamation marks can suffix Ruby identifiers.
    if braces
        && file.bytes.get(end).is_some_and(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'?' | b'!') || *byte >= 0x80
        })
    {
        close.push(' ');
    }
    #[cfg(test)]
    COPIED_BYTES.with(|bytes| bytes.set(bytes.get() + end - start));
    super::correction::combine(
        file,
        vec![
            Correction {
                range: start..opening.end_offset(),
                replacement: open,
            },
            Correction {
                range: closing.start_offset()..end,
                replacement: close,
            },
        ],
    )
}

/// Follow the inverse of RuboCop's get_blocks traversal: only sends,
/// unbraced hashes and pairs are transparent. Encountering any enclosing
/// block starts a new candidate, since ignoring it ignores its whole subtree.
/// This is one ancestor pass, bounded by the pipeline's AST-depth budget;
/// there are no descendant walks or source scans per block.
fn ignored_by_ancestors<'a, 'pr: 'a>(
    node: &Node<'pr>,
    ancestors: impl Iterator<Item = &'a Node<'pr>>,
    file: &SourceFile,
) -> bool {
    let mut reachable_block = true;
    let mut child = node;
    let mut ancestors = ancestors.peekable();
    while let Some(parent) = ancestors.next() {
        if offending_block(parent, ancestors.peek().copied(), file).is_some() {
            return true;
        }
        if reachable_block && child.as_arguments_node().is_some() && binding_sensitive(parent) {
            return true;
        }
        reachable_block = match parent {
            Node::CallNode { .. } => literal_block(parent).is_some() || reachable_block,
            Node::SuperNode { .. } | Node::ForwardingSuperNode { .. } => {
                literal_block(parent).is_some()
            }
            Node::BlockNode { .. } | Node::LambdaNode { .. } => true,
            Node::ArgumentsNode { .. } | Node::KeywordHashNode { .. } | Node::AssocNode { .. } => {
                reachable_block
            }
            _ => false,
        };
        child = parent;
    }
    false
}

fn binding_sensitive(node: &Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        let name = call.name().as_slice();
        if call.closing_loc().is_some_and(|loc| loc.as_slice() == b")")
            || call.equal_loc().is_some()
            || (name.ends_with(b"=") && (!ast::is_operator_method(name) || name == b"[]="))
        {
            return false;
        }
        if ast::is_operator_method(name)
            && !call
                .block()
                .is_some_and(|block| block.as_block_argument_node().is_some())
            && let Some(args) = call.arguments()
            && args.arguments().len() == 1
            && let Some(arg) = args.arguments().first()
            && (literal_block(&arg).is_some_and(|block| {
                block.parameters().is_none_or(|params| {
                    params.as_numbered_parameters_node().is_none()
                        && params.as_it_parameters_node().is_none()
                })
            }) || arg.as_lambda_node().is_some())
        {
            return false;
        }
        true
    } else if let Some(call) = node.as_super_node() {
        call.lparen_loc().is_none()
    } else if let Some(call) = node.as_yield_node() {
        call.lparen_loc().is_none()
    } else {
        false
    }
}

/// RuboCop counts from the opening delimiter, excluding a multiline receiver.
fn offending_block<'pr>(
    node: &Node<'pr>,
    parent: Option<&Node<'pr>>,
    file: &SourceFile,
) -> Option<BlockNode<'pr>> {
    let block = literal_block(node)?;
    if node
        .as_call_node()
        .is_some_and(|call| matches!(call.name().as_slice(), b"lambda" | b"proc" | b"it"))
    {
        return None;
    }
    if parent.and_then(Node::as_for_node).is_some_and(|for_node| {
        for_node.collection().location().start_offset() == node.location().start_offset()
    }) {
        return None;
    }
    let multiline = file.line_of(block.opening_loc().start_offset())
        != file.line_of(block.closing_loc().end_offset() - 1);
    let braces = block.opening_loc().as_slice() == b"{";
    if multiline != braces {
        return None;
    }
    if !braces && requires_do_end(&block) {
        return None;
    }
    Some(block)
}

/// Match upstream's modifier-rescue shape heuristic. Prism has a distinct
/// RescueModifierNode, but also represents `expr; rescue; fallback` as an
/// implicit Begin with the same RuboCop rescue shape. Keep its detection
/// (without attempting an unsafe delimiter-only correction).
fn requires_do_end(block: &BlockNode<'_>) -> bool {
    let Some(body) = block.body().and_then(|body| body.as_begin_node()) else {
        return false;
    };
    if body.begin_keyword_loc().is_some() {
        return false;
    }
    if body.ensure_clause().is_some() {
        return true;
    }
    body.rescue_clause().is_some_and(|rescue| {
        body.statements().is_none()
            || body.else_clause().is_some()
            || rescue.subsequent().is_some()
            || !rescue.exceptions().is_empty()
            || rescue.reference().is_some()
    })
}

fn literal_block<'pr>(node: &Node<'pr>) -> Option<BlockNode<'pr>> {
    if let Some(call) = node.as_forwarding_super_node() {
        return call.block();
    }
    let block = if let Some(call) = node.as_call_node() {
        call.block()
    } else if let Some(call) = node.as_super_node() {
        call.block()
    } else {
        None
    }?;
    block.as_block_node()
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/block_delimiters_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "invalid fixture: {source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/BlockDelimiters")
            .collect()
    }

    fn assert_offense(source: &str, line: u32, col: u32, multiline: bool) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert_eq!(
            (found[0].line, found[0].col, found[0].end_line),
            (line, col, line),
            "{source}"
        );
        assert_eq!(
            found[0].message,
            if multiline {
                "Avoid using `{...}` for multi-line blocks."
            } else {
                "Prefer `{...}` over `do...end` for single-line blocks."
            }
        );
        assert!(found[0].correction.is_none());
    }

    fn corrected(source: &str) -> String {
        let found = crate::pipeline::lint_str_with_corrections(&[&super::BlockDelimiters], source);
        assert!(!found.is_empty(), "expected offense: {source}");
        String::from_utf8(
            crate::corrections::apply(source.as_bytes(), &found)
                .unwrap()
                .bytes,
        )
        .unwrap()
    }

    fn assert_same_ast(before: &str, after: &str) {
        let a = ruby_prism::parse(before.as_bytes());
        let b = ruby_prism::parse(after.as_bytes());
        assert!(a.errors().next().is_none(), "{before}");
        assert!(b.errors().next().is_none(), "{after}");
        assert_eq!(
            crate::cops::ast::fingerprint_ignoring_grouping(&a.node(), before.as_bytes()),
            crate::cops::ast::fingerprint_ignoring_grouping(&b.node(), after.as_bytes()),
            "{before} => {after}"
        );
    }

    #[test]
    fn pinned_correction_pairs_keep_comments_and_lines_in_place() {
        let pairs: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/block_delimiters_corrections.json"
        ))
        .unwrap();
        let pairs = pairs.as_array().unwrap();
        assert_eq!(pairs.len(), 21);
        for pair in pairs {
            let before = pair["before"].as_str().unwrap();
            let after = pair["after"].as_str().unwrap();
            assert_eq!(corrected(before), after, "pinned line {}", pair["line"]);
            assert_same_ast(before, after);
        }
    }

    #[test]
    fn operator_arguments_retain_block_ownership() {
        for (before, after) in [
            (
                "a + items.map { |x|\n x\n}",
                "a + items.map do |x|\n x\nend",
            ),
            ("a + items.map {\n 1\n}", "a + items.map do\n 1\nend"),
        ] {
            assert_eq!(corrected(before), after);
            assert_same_ast(before, after);
        }
    }

    #[test]
    fn uncertain_commands_loop_conditions_and_implicit_rescue_keep_offenses() {
        for source in [
            "f g do x end",
            "f 1 do x end",
            "f&.g 1 do x end",
            "s.subspec 'Subspec' do |sp| end",
            "a.+ items.map { |x|\n x\n}",
            "a&.+ items.map { |x|\n x\n}",
            "def f; super 1 do x end; end",
            "foo do bar; rescue; baz end",
            "while foo { |x|\n x\n}\n bar\nend",
            "until (foo && baz.any? { |x|\n x\n})\n bar\nend",
            "map do x end\np DATA.pos\n__END__\nraw\n",
        ] {
            let found =
                crate::pipeline::lint_str_with_corrections(&[&super::BlockDelimiters], source);
            assert_eq!(found.len(), 1, "{source}: {found:?}");
            assert!(found[0].correction.is_none(), "{source}: {found:?}");
            assert_eq!(corrected(source), source);
        }
    }

    #[test]
    fn splatted_command_arguments_keep_offenses_without_rebinding_blocks() {
        for splat in ["*", "**"] {
            for template in [
                "outer %",
                "obj.outer %",
                "obj&.outer %",
                "def f; super %; end",
                "outer first, %",
            ] {
                let block = "inner {\n value\n}";
                let source = template.replace('%', &format!("{splat}{block}"));
                let found =
                    crate::pipeline::lint_str_with_corrections(&[&super::BlockDelimiters], &source);
                assert_eq!(found.len(), 1, "{source}: {found:?}");
                assert!(found[0].correction.is_none(), "{source}: {found:?}");
                assert_eq!(corrected(&source), source);
                assert_same_ast(&source, &corrected(&source));

                // Both versions parse, but do/end transfers the block from
                // inner to the enclosing command (including super).
                let naive = source.replace(block, "inner do\n value\nend");
                let before = ruby_prism::parse(source.as_bytes());
                let after = ruby_prism::parse(naive.as_bytes());
                assert!(before.errors().next().is_none(), "{source}");
                assert!(after.errors().next().is_none(), "{naive}");
                assert_ne!(
                    crate::cops::ast::fingerprint_ignoring_grouping(
                        &before.node(),
                        source.as_bytes()
                    ),
                    crate::cops::ast::fingerprint_ignoring_grouping(
                        &after.node(),
                        naive.as_bytes()
                    ),
                    "{source} => {naive}"
                );
            }
            // Explicit argument parentheses and literal containers prevent
            // rebinding: splats alone must not disable every correction.
            let container = if splat == "*" {
                "array = [%]"
            } else {
                "hash = {%}"
            };
            for template in ["outer(%)", "def f; super(%); end", container] {
                let source = template.replace('%', &format!("{splat}inner {{\n value\n}}"));
                let after = corrected(&source);
                assert_eq!(after, source.replace("{\n value\n}", "do\n value\nend"));
                assert_same_ast(&source, &after);
            }
        }
    }

    #[test]
    fn generated_contexts_preserve_main_ast_binding_and_line_values() {
        let mut evaluated = 0;
        let mut edited = 0;
        for block in [
            "each { |x|\n x\n}",
            "each {\n _1\n}",
            "each {\n it\n}",
            "each do |x| x end",
            "each do _1 end",
            "each do it end",
            "each(1) { |x|\n x\n}",
            "each(1) do |x| x end",
            "obj&.each { |x|\n x\n}",
            "obj&.each do |x| x end",
            "obj[0] { |x|\n x\n}",
            "obj[0] do |x| x end",
            "obj::each { |x|\n x\n}",
            "obj::each do |x| x end",
            "each (arg) { |x|\n x\n}",
            "each (arg) do |x| x end",
            "each(<<~ARG) {\n literal\nARG\n 1\n}",
            "each(<<~ARG) do 1 end\n literal\nARG\n",
        ] {
            for template in [
                "%",
                "(%)",
                "sink(%)",
                "sink %",
                "sink *%",
                "sink **%",
                "sink(*%)",
                "sink(**%)",
                "[%]",
                "[*%]",
                "{**%}",
                "{k: %}",
                "sink k: %",
                "x = %",
                "obj.x = %",
                "obj[0] = %",
                "obj.x=(%)",
                "obj[0]=%",
                "a + %",
                "a == %",
                "a % %",
                "a.+ %",
                "a.== %",
                "a&.+ %",
                "- %",
                "+ %",
                "~ %",
                "a[0] += %",
                "a[0] ||= %",
                "a.x += %",
                "% + a",
                "a && %",
                "a and %",
                "a || %",
                "a or %",
                "not %",
                "!%",
                "% ? a : b",
                "%? a : b",
                "%!= a",
                "%!~ /x/",
                "a ? % : b",
                "% rescue a",
                "for x in % do\n x\nend",
                "for x in [%] do\n x\nend",
                "% if condition",
                "if % then x end",
                "unless % then x end",
                "while %; break; end",
                "until (%); break; end",
                "% .to_s",
                "%&.to_s",
                "%[0]",
                "%::foo",
                "% + sibling { |x|\n x\n}",
                "sibling { |x|\n x\n} + %",
                "sink % + other, 1",
                "sink (%.to_s)",
                "[%, other]",
                "def f; super %; end",
                "def f; super *%; end",
                "def f; super **%; end",
                "def f; super(*%); end",
                "def f; super(**%); end",
                "def f; yield %; end",
                "object.x ||= %",
                "object.x &&= %",
                "a, b = %, 1",
                "x in [%, *rest]",
                "case %; when 1; x; end",
                "begin; %; rescue; fallback; end",
                "outer do\n %\nend",
                "outer { % }",
                "-> { % }",
                "def f; return %; end",
                "def f; yield(%); end",
                "def f; super(%); end",
            ] {
                // Only replace the final placeholder: the infix percent is an operator.
                let at = template.rfind('%').unwrap();
                let source = format!(
                    "{}{}{}\np __LINE__\n",
                    &template[..at],
                    block,
                    &template[at + 1..]
                );
                let parsed = ruby_prism::parse(source.as_bytes());
                if parsed.errors().next().is_some() {
                    continue;
                }
                let found =
                    crate::pipeline::lint_str_with_corrections(&[&super::BlockDelimiters], &source);
                let after = String::from_utf8(
                    crate::corrections::apply(source.as_bytes(), &found)
                        .unwrap()
                        .bytes,
                )
                .unwrap();
                evaluated += 1;
                edited += usize::from(source != after);
                assert_same_ast(&source, &after);
            }
        }
        assert!(evaluated >= 400, "evaluated {evaluated}");
        assert!(edited >= 100, "edited {edited}");
    }

    #[test]
    fn exact_token_bytes_comments_heredocs_and_line_values_survive() {
        for (before, after) in [
            (
                "if each{\n 1\n}then x end",
                "if each do\n 1\nend then x end",
            ),
            (
                "outer do inner{\n x\n}end",
                "outer do inner do\n x\nend end",
            ),
            ("each{||\n 1\n}", "each do ||\n 1\nend"),
            ("each do|| 1 end", "each { || 1 }"),
            ("each{\n 1\n}? x : y", "each do\n 1\nend ? x : y"),
            ("each{\n 1\n}!= x", "each do\n 1\nend != x"),
            (
                "map do puts <<~MSG end\n } do end\nMSG\n",
                "map { puts <<~MSG }\n } do end\nMSG\n",
            ),
            (
                "map {\n puts <<~MSG\n } do end\nMSG\n}\n",
                "map do\n puts <<~MSG\n } do end\nMSG\nend\n",
            ),
            (
                "sink(<<~TEXT, map do 1 end)\n sibling } end\nTEXT\n",
                "sink(<<~TEXT, map { 1 })\n sibling } end\nTEXT\n",
            ),
            (
                "p __LINE__\nmap { |x| # { do\n p [__LINE__, x, '}end']\n} # encoding: UTF-8\np __LINE__\n__END__\nraw\n",
                "p __LINE__\nmap do |x| # { do\n p [__LINE__, x, '}end']\nend # encoding: UTF-8\np __LINE__\n__END__\nraw\n",
            ),
            (
                "\u{feff}# encoding: UTF-8\r\nmap { |é| # end\r\n é\r\n} # comment\r\n",
                "\u{feff}# encoding: UTF-8\r\nmap do |é| # end\r\n é\r\nend # comment\r\n",
            ),
            ("def f; super do x end; end", "def f; super { x }; end"),
            (
                "def f; super(1) {\n x\n}; end",
                "def f; super(1) do\n x\nend; end",
            ),
            (
                "foo do begin; bar; rescue; baz end end",
                "foo { begin; bar; rescue; baz end }",
            ),
        ] {
            assert_eq!(corrected(before), after);
            assert_same_ast(before, after);
        }
    }

    #[test]
    fn parameter_arity_control_flow_and_nested_captures_preserve_main_ast() {
        for body in ["return x", "break x", "next x", "[x, local]", "inner { x }"] {
            let before = format!("def f; each do |x; local| {body} end; end");
            let after = format!("def f; each {{ |x; local| {body} }}; end");
            assert_eq!(corrected(&before), after);
            assert_same_ast(&before, &after);
        }
        for (params, body) in [("", "_1"), ("", "it"), ("|it|", "it"), ("|x,y|", "[x,y]")] {
            let before =
                format!("x = 1\nouter do |y|\n inner {{ {params}\n [{body}, x, y]\n }}\nend");
            let after =
                format!("x = 1\nouter do |y|\n inner do {params}\n [{body}, x, y]\n end\nend");
            assert_eq!(corrected(&before), after);
            assert_same_ast(&before, &after);
        }
        for before in [
            "Proc.new { |x,y|\n [x,y]\n}",
            "proc {\n each do 1 end\n}",
            "->(x) { each do x end }",
        ] {
            let after = corrected(before);
            assert_ne!(before, after);
            assert_same_ast(before, &after);
        }
    }

    #[test]
    fn correction_work_is_lazy_and_bounded_by_disjoint_source_spans() {
        for count in [100, 1000] {
            let source = "outer do\n map { |x|\n  x\n }\nend\n".repeat(count);
            super::COPIED_BYTES.with(|bytes| bytes.set(0));
            assert_eq!(offenses(&source).len(), count);
            super::COPIED_BYTES.with(|bytes| assert_eq!(bytes.get(), 0));
            let found =
                crate::pipeline::lint_str_with_corrections(&[&super::BlockDelimiters], &source);
            assert_eq!(found.len(), count);
            assert!(found.iter().all(|offense| offense.correction.is_some()));
            super::COPIED_BYTES
                .with(|bytes| assert_eq!(bytes.get(), count * "{ |x|\n  x\n }".len()));
        }
    }

    #[test]
    fn allows_fixed_method_names_lambdas_and_for_collections() {
        for name in ["lambda", "proc", "it"] {
            for receiver in ["", "Kernel.", "Other::Factory.", "obj&."] {
                for suffix in [" do |x| x end", " { |x|\n x\n}"] {
                    let source = format!("{receiver}{name}{suffix}");
                    assert!(offenses(&source).is_empty(), "{source}");
                }
            }
        }
        for source in [
            "->(x) do x end",
            "-> {\n _1\n}",
            "-> {\n it\n}",
            "for d in aa { |c|\n c\n} do\n f\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_offense("for d in x do\n  each { |y|\n    y\n  }\nend", 2, 8, true);
        assert_offense("Proc.new { |x|\n x\n}", 1, 10, true);
        assert_offense("test_method {\n x\n}", 1, 13, true);
    }

    #[test]
    fn distinguishes_block_level_rescue_ensure_and_modifier_rescue() {
        for source in [
            "foo do rescue => e; bar end",
            "foo do rescue; bar end",
            "foo do bar ensure baz end",
            "foo do bar; rescue StandardError; baz end",
            "foo do bar; rescue => e; baz end",
            "foo do bar; rescue; baz; rescue; qux end",
            "foo do next unless bar; rescue StandardError; end",
            "foo do bar; rescue; baz; else; qux end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "foo do bar rescue baz end",
            "foo do next unless bar rescue StandardError; end",
            "foo do bar; rescue; baz end",
            "foo do begin; bar; rescue; baz end end",
        ] {
            assert_offense(source, 1, 5, false);
        }
    }

    #[test]
    fn exempts_binding_sensitive_command_arguments_through_sends_and_keywords() {
        for body in ["|n|\n n", "\n _1", "\n it"] {
            let block = format!("items.map {{ {body}\n}}");
            for source in [
                format!("puts {block}"),
                format!("puts {block}, 1"),
                format!("puts [0] + {block}, 1"),
                format!("puts {block} + [0], 1"),
                format!("foo&.bar {block}"),
                format!("foo.bar {block}&.quux"),
                format!("foo bar + {block}.qux.quux"),
                format!("foo :bar, baz: '%s' % {block}.join(',')"),
                format!("def f; yield {block}; end"),
                format!("def f; super {block}; end"),
                format!("puts outer {{ {block}\n}}"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        assert!(offenses("cr.stubs client: mock {\n expects(:save)\n}").is_empty());
        assert!(offenses("a % items.map { |x|\n x\n}.join(',')").is_empty());
    }

    #[test]
    fn checks_parenthesized_arguments_explicit_containers_assignments_and_operators() {
        for (source, line, col) in [
            ("puts(items.map { |x|\n x\n})", 1, 16),
            ("puts (items.map { |x|\n x\n})", 1, 17),
            ("puts [items.map { |x|\n x\n}]", 1, 17),
            ("puts({ k: items.map { |x|\n x\n} })", 1, 21),
            ("puts k: [items.map { |x|\n x\n}]", 1, 20),
            ("x.y = items.map { |x|\n x\n}", 1, 17),
            ("x.y=(items.map { |x|\n x\n})", 1, 16),
            ("x[0] = items.map { |x|\n x\n}", 1, 18),
            ("def f; yield(items.map { |x|\n x\n}); end", 1, 24),
            ("def f; super(items.map { |x|\n x\n}); end", 1, 24),
            ("a + items.map { |x|\n x\n}", 1, 15),
            ("a + items.map {\n x\n}", 1, 15),
        ] {
            assert_offense(source, line, col, true);
        }
        // Upstream's operator exemption specifically tests block_type?, not
        // numblock/itblock; preserve that distinction at the fixed Ruby 3.4.
        assert!(offenses("a + items.map {\n _1\n}").is_empty());
        assert!(offenses("a + items.map {\n it\n}").is_empty());
        assert_offense("s.subspec 'Subspec' do |sp| end", 1, 21, false);
    }

    #[test]
    fn suppresses_nested_offenses_only_inside_ignored_or_offending_blocks() {
        assert_offense("block { foo {\n x\n} }.bar # comment", 1, 7, true);
        assert_offense("(0..3).each { |a| a.times {\n puts a\n}}", 1, 13, true);
        assert_offense("outer do\n inner {\n x\n }\nend", 2, 8, true);
        assert_offense("proc {\n inner {\n x\n }\n}", 2, 8, true);
        assert_offense("-> {\n inner {\n x\n }\n}", 2, 8, true);
        // A synthetic outer block also encloses its send receiver/arguments.
        assert_offense("inner {\n x\n}.outer {\n y\n}", 3, 9, true);
        assert!(offenses("puts outer {\n inner {\n x\n }\n}").is_empty());
    }

    #[test]
    fn control_flow_and_explicit_numbered_it_parameters_keep_the_same_policy() {
        for (body, params) in [
            ("return x", "|x|"),
            ("break x", "|x|"),
            ("next x", "|x|"),
            ("_1", ""),
            ("it", ""),
            ("it", "|it|"),
            ("x", "|x; local|"),
            ("", ""),
        ] {
            let single = format!("def f; each do {params} {body} end; end");
            assert_offense(&single, 1, 13, false);
            let multi = format!("each {{ {params}\n {body}\n}}");
            assert_offense(&multi, 1, 6, true);
        }
        assert_offense(
            "x = 1\nouter do |y|\n inner { |z|\n  x + y + z\n }\nend",
            3,
            8,
            true,
        );
        assert_offense("it = 1\na + items.map {\n it\n}", 2, 15, true);
        assert_offense("def f; super do x end; end", 1, 14, false);
        assert_offense("def f; super(1) {\n x\n}; end", 1, 17, true);
        for source in [
            "lambda do |x, y| x end",
            "proc do |x, y| x end",
            "->(x, y) do x end",
            "def f; puts super {\n inner {\n  x\n }\n}; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn comments_heredocs_literals_crlf_and_encoding_do_not_confuse_delimiters() {
        assert_offense(
            "# encoding: UTF-8\r\nitems.map { |x| # } do\r\n x\r\n} # comment\r\n",
            2,
            11,
            true,
        );
        assert_offense("map {\n puts <<~MSG\n } do end\nMSG\n}\n", 1, 5, true);
        assert_offense("map do puts <<~MSG end\n  data\nMSG\n", 1, 5, false);
        assert_offense("while foo { |x|\n x\n}\n bar\nend", 1, 11, true);
        assert_offense(
            "until (foo && baz.any? { |x|\n x\n})\n bar\nend",
            1,
            24,
            true,
        );
        assert_offense(
            "my_method { |x|\n x.foo } unless bar # comment",
            1,
            11,
            true,
        );
        for source in [
            "# each do x end\n'foo { bar }'",
            "puts <<~MSG\neach do x end\nMSG\n",
            "each do\n puts <<~MSG\n { }\nMSG\nend\n",
            "p 1\n__END__\neach do x end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn ancestor_work_is_linear_in_sibling_count_and_bounded_by_depth() {
        use crate::{
            cops::{Cop, NodeContext},
            source::SourceFile,
        };
        use ruby_prism::Node;
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Probe(AtomicUsize);
        impl Cop for Probe {
            fn name(&self) -> &'static str {
                "Style/BlockDelimiters"
            }
            fn check_node<'pr>(
                &self,
                node: &Node<'pr>,
                cx: &NodeContext<'_, 'pr>,
                file: &SourceFile,
                _out: &mut Vec<Offense>,
            ) {
                if super::offending_block(node, cx.parent(), file).is_some() {
                    super::ignored_by_ancestors(
                        node,
                        cx.ancestors().inspect(|_| {
                            self.0.fetch_add(1, Ordering::Relaxed);
                        }),
                        file,
                    );
                }
            }
        }
        let mut counts = Vec::new();
        for count in [100, 1000] {
            let source = "outer do\n inner {\n  x\n }\nend\n".repeat(count);
            let probe = Probe(AtomicUsize::new(0));
            assert!(crate::pipeline::lint_str_with(&[&probe], &source).is_empty());
            let steps = probe.0.load(Ordering::Relaxed);
            assert!(steps <= 64 * count);
            counts.push(steps);
            assert_eq!(offenses(&source).len(), count);
        }
        assert!(counts[0] > 0);
        assert_eq!(counts[1], 10 * counts[0]);
    }

    #[test]
    fn detects_line_count_style_at_opening_delimiter() {
        assert_offense("each do |x| end", 1, 6, false);
        assert_offense("  each { |x|\n  }", 1, 8, true);
        assert_offense("foo\n  .each do x end", 2, 9, false);
        assert_offense(
            "baz.map { |x|\nfoo(x) }.map { |x| x.quux } # comment",
            1,
            9,
            true,
        );
        for source in [
            "each { |x| }",
            "each do |x|\nend",
            "each {}",
            "foo do\nend.map(&:to_s)",
            "foo\n  .each { x }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
