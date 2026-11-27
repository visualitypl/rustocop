//! `Layout/EndAlignment` — align closing `end` with its opening keyword, or
//! with what `enforced_style_align_with` (RuboCop's `EnforcedStyleAlignWith`)
//! names: `keyword` (the default), `variable` (the start of an assignment,
//! or of a call, that the construct is the value of) or `start_of_line`.

use ruby_prism::{Location, Node};

use super::alignment::{EndAlignWith, assignment_owner, start_line_range};
use crate::{
    cops::{
        Cop, NodeContext, ast,
        settings::{Kind, Setting, Values},
    },
    offense::Offense,
    source::SourceFile,
};

#[derive(Default)]
pub struct EndAlignment {
    style: EndAlignWith,
}

impl Cop for EndAlignment {
    fn name(&self) -> &'static str {
        "Layout/EndAlignment"
    }

    fn settings(&self) -> &'static [Setting] {
        const ENFORCED_STYLE_ALIGN_WITH: Setting = Setting {
            name: "enforced_style_align_with",
            kind: Kind::Choice(&["keyword", "variable", "start_of_line"]),
        };
        &[ENFORCED_STYLE_ALIGN_WITH]
    }

    fn configure(&self, values: &Values) -> Option<Box<dyn Cop>> {
        let style = values
            .choice("enforced_style_align_with")
            .map_or(self.style, EndAlignWith::parse);
        Some(Box::new(EndAlignment { style }))
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
        let Some((keyword, end)) = keywords(node) else {
            return;
        };
        let keyword = keyword.start_offset()..keyword.end_offset();
        let anchor = match self.style {
            EndAlignWith::Keyword => keyword,
            EndAlignWith::StartOfLine => start_line_range(file, node.location().start_offset()),
            // `check_asgn_alignment` when the construct is the value of an
            // assignment or call, `check_other_alignment` otherwise.
            EndAlignWith::Variable => match outer(node, cx) {
                Some(outer)
                    if file.line_of(node.location().start_offset())
                        <= file.line_of(outer.location().start_offset()) =>
                {
                    outer.location().start_offset()..keyword.end
                }
                _ => keyword,
            },
        };
        let (align_line, align_col) = file.line_col(anchor.start);
        let (line, col) = file.line_col(end.start_offset());
        if line == align_line || col == align_col {
            return;
        }
        let correction = cx.corrections_requested().then(|| {
            let column = file.line_col(self.alignment(node, cx, file)).1;
            super::alignment::align_end(
                file,
                end.start_offset(),
                &" ".repeat((column - 1) as usize),
            )
        });
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "`end` at {line}, {} is not aligned with `{}` at {align_line}, {}.",
                col - 1,
                String::from_utf8_lossy(&file.bytes[anchor]),
                align_col - 1,
            ),
            line,
            end_line: line,
            col,
        });
    }
}

impl EndAlignment {
    /// The offset whose column the corrector moves `end` to
    /// (`alignment_node`).
    fn alignment<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
    ) -> usize {
        let start = node.location().start_offset();
        match self.style {
            EndAlignWith::Keyword => start,
            EndAlignWith::StartOfLine => start_line_range(file, start).start,
            // `alignment_node_for_variable_style`.
            EndAlignWith::Variable => {
                if let Some(call) = case_argument_of(node, cx)
                    && file.line_of(call.location().start_offset()) == file.line_of(start)
                {
                    return call.location().start_offset();
                }
                match assignment_or_operator_method(node, cx) {
                    Some(owner)
                        if file.line_of(start) <= file.line_of(owner.location().start_offset()) =>
                    {
                        owner.location().start_offset()
                    }
                    _ => start,
                }
            }
        }
    }
}

/// The node that RuboCop's `check_asgn_alignment` aligns `node` with: the
/// assignment or call it is the value of (`CheckAssignment`, for a
/// conditional), the call that a `case` is an argument of, the assignment
/// that a `class << self` is the value of. `None` for
/// `check_other_alignment`.
fn outer<'a, 'pr>(node: &Node<'pr>, cx: &NodeContext<'a, 'pr>) -> Option<&'a Node<'pr>> {
    match node {
        Node::SingletonClassNode { .. } => cx.parent().filter(|parent| {
            ast::is_assignment(parent)
                && super::multiline_expression::assignment_value(parent).is_some_and(|value| {
                    value.location().start_offset() == node.location().start_offset()
                })
        }),
        Node::ClassNode { .. } | Node::ModuleNode { .. } => None,
        _ => assignment_owner(node, cx.ancestors(), true, false)
            .or_else(|| case_argument_of(node, cx)),
    }
}

/// The call (not `&.`) that `node`, a `case`, is an argument of
/// (`argument?`).
fn case_argument_of<'a, 'pr>(node: &Node<'pr>, cx: &NodeContext<'a, 'pr>) -> Option<&'a Node<'pr>> {
    if !matches!(node, Node::CaseNode { .. } | Node::CaseMatchNode { .. }) {
        return None;
    }
    let mut ancestors = cx.ancestors();
    ancestors.next()?.as_arguments_node()?;
    ancestors.next().filter(|call| {
        call.as_call_node()
            .is_some_and(|call| !call.is_safe_navigation())
    })
}

/// RuboCop's `assignment_or_operator_method`: the nearest ancestor that is an
/// assignment (or `<<` call) whose value leads to `node`, an operator call
/// whose receiver chain does not, or another call whose last argument
/// does (`leading_node`). A call is not an ancestor of its block's body in
/// the parser gem, as it is in Prism.
fn assignment_or_operator_method<'a, 'pr>(
    node: &Node<'pr>,
    cx: &NodeContext<'a, 'pr>,
) -> Option<&'a Node<'pr>> {
    let span = |n: &Node<'_>| (n.location().start_offset(), n.location().end_offset());
    let is_node = |candidate: Option<Node<'pr>>| candidate.is_some_and(|c| span(&c) == span(node));
    let mut child = span(node);
    for ancestor in cx.ancestors() {
        let previous = std::mem::replace(&mut child, span(ancestor));
        let found = if ast::is_assignment(ancestor) {
            is_node(leading_node(super::multiline_expression::assignment_value(
                ancestor,
            )))
        } else if let Some(call) = ancestor.as_call_node()
            && !call.is_safe_navigation()
            && call.block().is_none_or(|block| span(&block) != previous)
        {
            if call.name().as_slice() == b"<<" {
                is_node(leading_node(last_argument(&call)))
            } else if ast::is_operator_method(call.name().as_slice()) {
                !is_node(leading_node(call.receiver()))
            } else {
                is_node(leading_node(last_argument(&call)))
            }
        } else {
            false
        };
        if found {
            return Some(ancestor);
        }
    }
    None
}

/// The parser gem's `last_argument`, where a block pass is an argument.
fn last_argument<'pr>(call: &ruby_prism::CallNode<'pr>) -> Option<Node<'pr>> {
    match call.block() {
        Some(block) if block.as_block_argument_node().is_some() => Some(block),
        _ => call.arguments()?.arguments().iter().last(),
    }
}

/// `EndAlignment#leading_node`: down the receivers of a call chain
/// (`first_part_of_call_chain`), then through `(…)`, `||` and `&&`.
fn leading_node(node: Option<Node<'_>>) -> Option<Node<'_>> {
    let mut node = node?;
    while let Some(call) = node.as_call_node() {
        node = call.receiver()?;
    }
    loop {
        node = if let Some(parentheses) = node.as_parentheses_node() {
            let body = parentheses.body()?;
            match body.as_statements_node() {
                Some(statements) => statements.body().iter().next()?,
                None => body,
            }
        } else if let Some(or) = node.as_or_node() {
            or.left()
        } else if let Some(and) = node.as_and_node() {
            and.left()
        } else {
            return Some(node);
        };
    }
}

/// Only constructs owned by EndAlignment. Prism shares an if chain's closing
/// location with its elsif nodes; the opening `if` alone owns that end.
fn keywords<'pr>(node: &Node<'pr>) -> Option<(Location<'pr>, Location<'pr>)> {
    Some(match node {
        Node::ClassNode { .. } => {
            let n = node.as_class_node().unwrap();
            (n.class_keyword_loc(), n.end_keyword_loc())
        }
        Node::SingletonClassNode { .. } => {
            let n = node.as_singleton_class_node().unwrap();
            (n.class_keyword_loc(), n.end_keyword_loc())
        }
        Node::ModuleNode { .. } => {
            let n = node.as_module_node().unwrap();
            (n.module_keyword_loc(), n.end_keyword_loc())
        }
        Node::IfNode { .. } => {
            let n = node.as_if_node().unwrap();
            let keyword = n.if_keyword_loc()?;
            if keyword.as_slice() != b"if" {
                return None;
            }
            (keyword, n.end_keyword_loc()?)
        }
        Node::UnlessNode { .. } => {
            let n = node.as_unless_node().unwrap();
            (n.keyword_loc(), n.end_keyword_loc()?)
        }
        Node::WhileNode { .. } => {
            let n = node.as_while_node().unwrap();
            (n.keyword_loc(), n.closing_loc()?)
        }
        Node::UntilNode { .. } => {
            let n = node.as_until_node().unwrap();
            (n.keyword_loc(), n.closing_loc()?)
        }
        Node::CaseNode { .. } => {
            let n = node.as_case_node().unwrap();
            (n.case_keyword_loc(), n.end_keyword_loc())
        }
        Node::CaseMatchNode { .. } => {
            let n = node.as_case_match_node().unwrap();
            (n.case_keyword_loc(), n.end_keyword_loc())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/end_alignment_spec.rb`
    //! and `spec/support/alignment_examples.rb` at 97f30c4
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        offense::{Offense, Severity},
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Layout/EndAlignment")
            .collect()
    }

    #[test]
    fn pinned_assignments_arguments_and_chained_calls_align_with_keyword() {
        for (prefix, opening) in [
            ("var = ", "if test"),
            ("@var = ", "unless test"),
            ("@@var = ", "while test"),
            ("$var = ", "until test"),
            ("CNST = ", "if test"),
            ("a, b = ", "if test"),
            ("var ||= ", "if test"),
            ("var &&= ", "unless test"),
            ("var += ", "if test"),
            ("h[k] = ", "case a when b"),
            ("h.k = ", "case a; in b"),
            ("var << ", "until test"),
            ("test ", "case a when b"),
            ("test ", "case a; in b"),
            ("var = ", "class << self"),
        ] {
            for suffix in [
                "",
                ".ab.join(\"\")",
                "&.ab",
                ".tap { _1 }",
                " || x",
                " && x",
            ] {
                let before = format!("{prefix}{opening}\nend{suffix}\n");
                let after = format!(
                    "{prefix}{opening}\n{}end{suffix}\n",
                    " ".repeat(prefix.len())
                );
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!((found[0].line, found[0].col), (2, 1));
                assert_correction(&EndAlignment::default(), &before, &after);
                assert!(offenses(&after).is_empty(), "{after}");
            }
        }
        for (before, after) in [
            ("var =\n  if test\nend\n", "var =\n  if test\n  end\n"),
            ("foo((if test\nend).ab)\n", "foo((if test\n     end).ab)\n"),
            (
                "x = y || case a\nwhen b\n  end\n",
                "x = y || case a\nwhen b\n         end\n",
            ),
        ] {
            assert_correction(&EndAlignment::default(), before, after);
            assert!(offenses(after).is_empty());
        }
    }

    #[test]
    fn elsif_chains_have_one_closing_end_owned_by_initial_if() {
        let before =
            "if a\n  if b\n    x\n  elsif c\n    y\n    end\nelsif d\n  z\nelse\n  q\n  end\n";
        let after = "if a\n  if b\n    x\n  elsif c\n    y\n  end\nelsif d\n  z\nelse\n  q\nend\n";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(6, 5), (11, 3)]
        );
        assert_eq!(
            found[0].message,
            "`end` at 6, 4 is not aligned with `if` at 2, 2."
        );
        assert_eq!(
            found[1].message,
            "`end` at 11, 2 is not aligned with `if` at 1, 0."
        );
        assert_correction(&EndAlignment::default(), before, after);
        assert!(offenses(after).is_empty());
    }

    #[test]
    fn ignores_one_line_postfix_ternary_and_ends_owned_by_other_cops() {
        for source in [
            "",
            "\n",
            "# end\n",
            "__END__\n  if a\nend\n",
            "class A; end\nmodule B; end\nclass << self; end\n",
            "if a; b; elsif c; d; else; e; end\n",
            "unless a; end\nwhile a; end\nuntil a; end\n",
            "case a; when b; end\ncase a; in b; end\n",
            "a = cond ? x : y",
            "a = x if cond",
            "a unless cond",
            "a while cond",
            "a until cond",
            "begin\n x\n    end while cond\n",
            "def x\n  end\ndef self.x\n    end\n",
            "begin\nrescue\nensure\n  end\n",
            "foo do\n  end\n-> do\n  end\nfoo { }\n",
            "for a in b\n  end\n",
            "x = \"if a\\n  end\"\ny = /end/\nz = %w(if end)\n",
            "x = <<~H\nif a\n  end\nH\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn utf8_bom_crlf_tabs_and_missing_final_newline_preserve_every_other_byte() {
        for (before, after, position, message) in [
            (
                "\u{feff}class Test\r\n  end",
                "\u{feff}class Test\r\nend",
                (2, 3),
                "`end` at 2, 2 is not aligned with `class` at 1, 0.",
            ),
            (
                "é = if true\r\nend # 日本語\r\n",
                "é = if true\r\n    end # 日本語\r\n",
                (2, 1),
                "`end` at 2, 0 is not aligned with `if` at 1, 4.",
            ),
            (
                "  if a\n\tend\n",
                "  if a\n  end\n",
                (2, 2),
                "`end` at 2, 1 is not aligned with `if` at 1, 2.",
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), position);
            assert_eq!(found[0].message, message);
            assert_correction(&EndAlignment::default(), before, after);
        }
        let before = "class A\n  end\n__END__\r\n  end\n\traw";
        assert_correction(
            &EndAlignment::default(),
            before,
            &before.replacen("  end", "end", 1),
        );
    }

    #[test]
    fn both_directive_spellings_protect_end_indentation() {
        for prefix in ["rustocop", "rubocop"] {
            for target in ["Layout/EndAlignment", "EndAlignment", "Layout", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\nif a\n  end\n"),
                    format!("if a\n  end # {prefix}:disable {target}\n"),
                    format!("# {prefix}:disable-next {target}\nif a\n  end\n"),
                ] {
                    assert!(offenses(&source).is_empty(), "{source}");
                    assert!(correction_offenses(&EndAlignment::default(), &source).is_empty());
                }
                let before = format!(
                    "# {prefix}:disable {target}\nif a\n  end\n# {prefix}:enable {target}\nif b\n  end\n"
                );
                let after = before.strip_suffix("  end\n").unwrap().to_owned() + "end\n";
                assert_eq!(offenses(&before).len(), 1);
                assert_correction(&EndAlignment::default(), &before, &after);
            }
        }
    }

    #[test]
    fn pinned_constructs_report_exact_end_and_correct_only_its_indentation() {
        for (opening, keyword) in [
            ("class Test", "class"),
            ("class << self", "class"),
            ("module Test", "module"),
            ("if test", "if"),
            ("unless test", "unless"),
            ("while test", "while"),
            ("until test", "until"),
            ("case a when b", "case"),
            ("case a; in b", "case"),
        ] {
            for prefix in ["", "puts 1; "] {
                let before = format!("{prefix}{opening}\n  end # keep\n");
                let after = format!(
                    "{prefix}{opening}\n{}end # keep\n",
                    " ".repeat(prefix.len())
                );
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                let o = &found[0];
                assert_eq!((o.line, o.end_line, o.col), (2, 2, 3));
                assert_eq!(o.severity, Severity::Warning);
                assert_eq!(
                    o.message,
                    format!(
                        "`end` at 2, 2 is not aligned with `{keyword}` at 1, {}.",
                        prefix.len()
                    )
                );
                assert!(o.correction.is_none());
                assert_correction(&EndAlignment::default(), &before, &after);
                assert!(offenses(&after).is_empty());
            }
        }
    }
}
