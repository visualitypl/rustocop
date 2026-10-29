//! `Style/MultilineTernaryOperator` — fixed-default multiline conditional style.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Node, StatementsNode};
pub struct MultilineTernaryOperator;
const BUDGET: usize = 16 * 1024;
const MSG_IF: &str = "Avoid multi-line ternary operators, use `if` or `unless` instead.";
const MSG_SINGLE: &str = "Avoid multi-line ternary operators, use single-line instead.";
impl Cop for MultilineTernaryOperator {
    fn name(&self) -> &'static str {
        "Style/MultilineTernaryOperator"
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
        if !node
            .as_if_node()
            .is_some_and(|n| n.if_keyword_loc().is_none())
        {
            return;
        }
        let single = single_line_parent(cx);
        let Some(p) = parts(node, single, file) else {
            return;
        };
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: if single { MSG_SINGLE } else { MSG_IF }.into(),
            correction: cx
                .corrections_requested()
                .then(|| rewrite(node, &p, single, cx, file))
                .flatten(),
        });
    }
}
struct Parts<'pr> {
    condition: Node<'pr>,
    yes: Node<'pr>,
    no: Node<'pr>,
}
fn one<'pr>(body: StatementsNode<'pr>) -> Option<Node<'pr>> {
    (body.body().len() == 1)
        .then(|| body.body().iter().next())
        .flatten()
}
fn parts<'pr>(node: &Node<'pr>, single: bool, file: &SourceFile) -> Option<Parts<'pr>> {
    let n = node.as_if_node()?;
    if n.if_keyword_loc().is_some()
        || file.line_of(n.location().start_offset()) == file.last_line_of(n.location())
    {
        return None;
    }
    let p = Parts {
        condition: n.predicate(),
        yes: one(n.statements()?)?,
        no: one(n.subsequent()?.as_else_node()?.statements()?)?,
    };
    // Upstream exempts multiline *components* when joining only the operator
    // separators would be a no-op. Compare slices without allocating a source
    // replacement on the plain-lint path.
    if single {
        let c = p.condition.location();
        let y = p.yes.location();
        let n = p.no.location();
        if file.bytes.get(c.end_offset()..y.start_offset()) == Some(b" ? ")
            && file.bytes.get(y.end_offset()..n.start_offset()) == Some(b" : ")
        {
            return None;
        }
    }
    Some(p)
}
fn single_line_parent(cx: &NodeContext<'_, '_>) -> bool {
    let Some(parent) = cx
        .ancestors()
        .find(|n| n.as_statements_node().is_none() && n.as_arguments_node().is_none())
    else {
        return false;
    };
    if let Some(n) = parent.as_defined_node() {
        return n.lparen_loc().is_none();
    }
    if let Some(n) = parent.as_call_node() {
        return !n.is_attribute_write();
    }
    matches!(
        parent,
        Node::ReturnNode { .. }
            | Node::BreakNode { .. }
            | Node::NextNode { .. }
            | Node::YieldNode { .. }
            | Node::SuperNode { .. }
    )
}
fn text<'pr>(node: &Node<'pr>) -> Option<&'pr str> {
    std::str::from_utf8(node.location().as_slice()).ok()
}
fn render(node: &Node<'_>, single: bool, nl: &str, file: &SourceFile) -> Option<String> {
    let Some(p) = parts(node, single, file) else {
        return Some(text(node)?.to_owned());
    };
    // The `?` terminates an open range; a newline after `if` does not.
    // Check every recursively expanded branch, not just the outer ternary.
    if !single && super::multiline_if_then::ends_with_open_range(&p.condition) {
        return None;
    }
    let condition = text(&p.condition)?;
    // Recursing into branch ternaries makes one atomic edit: no partial nested
    // corrections or implicit multipass CLI. Parser depth and span budget bound it.
    let yes = render(&p.yes, false, nl, file)?;
    let no = render(&p.no, false, nl, file)?;
    Some(if single {
        format!("{condition} ? {yes} : {no}")
    } else {
        format!("if {condition}{nl}  {yes}{nl}else{nl}  {no}{nl}end")
    })
}
fn rewrite(
    node: &Node<'_>,
    p: &Parts<'_>,
    single: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    let mut range = loc.start_offset()..loc.end_offset();
    if range.len() > BUDGET || super::correction::touches_directive(file, range.clone()) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, node)
        || file.heredoc_pending_at(root, range.start)
        || file.class_method_bindings(root).observes_sources
        || file.bytes[range.clone()]
            .windows(8)
            .any(|s| s == b"__LINE__")
    {
        return None;
    }
    let end = file.line_start(file.line_of(range.start) + 1);
    let nl = if file.bytes.get(end.saturating_sub(2)..end) == Some(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + range.len()));
    let mut replacement = render(node, single, nl, file)?;
    let comments = file.comments();
    let first = comments.partition_point(|c| c.end <= range.start);
    let inside: Vec<_> = comments[first..]
        .iter()
        .take_while(|c| c.start < range.end)
        .collect();
    if !inside.is_empty() {
        // Only separator comments can be lifted; never duplicate comments
        // contained in a verbatim condition/branch or move a magic header.
        if [&p.condition, &p.yes, &p.no].iter().any(|n| {
            super::correction::contains_comment(
                file,
                n.location().start_offset()..n.location().end_offset(),
            )
        }) {
            return None;
        }
        let parent = cx
            .ancestors()
            .find(|n| n.as_statements_node().is_none() && n.as_arguments_node().is_none());
        let start = parent
            .filter(|n| n.as_local_variable_write_node().is_some())
            .map_or(range.start, |n| n.location().start_offset());
        let prefix = &file.bytes[file.line_start(file.line_of(start))..start];
        if !prefix.iter().all(|b| matches!(b, b' ' | b'\t')) || range.end - start > BUDGET {
            return None;
        }
        let indent = std::str::from_utf8(prefix).ok()?;
        let mut lifted = String::new();
        for c in inside {
            let bytes = &file.bytes[c.clone()];
            if bytes.starts_with(b"=begin") || !super::correction::comment_relocation_safe(bytes) {
                return None;
            }
            lifted.push_str(std::str::from_utf8(bytes).ok()?);
            lifted.push_str(nl);
            lifted.push_str(indent);
        }
        lifted.push_str(std::str::from_utf8(&file.bytes[start..range.start]).ok()?);
        lifted.push_str(&replacement);
        replacement = lifted;
        range.start = start;
    }
    let edit = Correction { range, replacement };
    (super::correction::preserves_line_values(file, &edit)
        && super::correction::preserves_data_positions(file, cx, &edit)
        && !super::correction::touches_directive(file, edit.range.clone()))
    .then_some(edit)
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's multiline_ternary_operator_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
    #[test]
    fn parent_contexts_and_noop_multiline_components() {
        use crate::cops::testing::correction_offenses;
        for (source, message) in [
            ("def f; return cond ?\n yes : no; end\n", MSG_SINGLE),
            ("loop { break cond ?\n yes : no }\n", MSG_SINGLE),
            ("loop { next cond ?\n yes : no }\n", MSG_SINGLE),
            ("consume(cond ?\n yes : no)\n", MSG_SINGLE),
            ("obj&.consume(cond ?\n yes : no)\n", MSG_SINGLE),
            ("defined? cond ?\n yes : no\n", MSG_SINGLE),
            ("defined?(cond ?\n yes : no)\n", MSG_IF),
            ("object.value = cond ?\n yes : no\n", MSG_IF),
            ("object[:key] = cond ?\n yes : no\n", MSG_IF),
            ("(cond ?\n yes : no).inspect\n", MSG_IF),
        ] {
            let found = correction_offenses(&MultilineTernaryOperator, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].message, message, "{source}");
            assert!(found[0].correction.is_some(), "{source}");
        }
        for source in [
            "a = cond ? yes : no\n",
            "consume(arg\n .ready? ? yes : no)\n",
            "consume(cond ? \"a\nb\" : no)\n",
        ] {
            assert!(
                correction_offenses(&MultilineTernaryOperator, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn open_range_predicates_withhold_atomic_expansion() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "x = true.. ?\n false : 2\n",
            "x = true... ?\n false : 2\n",
            "x = true ?\n true.. ?\n false : 2 : 3\n",
            "x = false ?\n 3 : true.. ?\n false : 2\n",
            "consume(true ?\n true.. ?\n false : 2 : 3)\n",
        ] {
            let found = correction_offenses(&MultilineTernaryOperator, source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
        // Joining a parent-required ternary keeps the `?` range delimiter.
        assert_correction(
            &MultilineTernaryOperator,
            "consume(true.. ?\n false : 2)\n",
            "consume(true.. ? false : 2)\n",
        );
    }
    #[test]
    fn sensitive_source_edits_remain_diagnosed() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "cond ?\n __LINE__ : no\n",
            "cond ?\n yes : no\np __LINE__\n",
            "cond ?\n DATA.pos : no\n__END__\npayload\n",
            "cond ?\n method(:f).source_location : no\n",
            "a = cond ? # coding: utf-8\n yes : no\n",
            "a = cond ? # frozen_string_literal: true\n yes : no\n",
            "a = cond ?\n # rubocop:disable Lint/Debugger\n yes : no\n",
            "a = cond ? (yes # internal\n) : no\n",
            "a = cond ? <<~TEXT :\n literal\nTEXT\n no\n",
            "consume(cond ? # do not lift past an argument evaluation\n yes : no)\n",
        ] {
            let found = correction_offenses(&MultilineTernaryOperator, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn literal_interiors_comments_and_crlf_are_verbatim() {
        assert_correction(
            &MultilineTernaryOperator,
            "\u{feff}# coding: utf-8\r\na = cond ?\r\n \"é\r\n  b\" : %q(c\r\n d) # tail\r\n__END__\r\npayload\x00\r\n",
            "\u{feff}# coding: utf-8\r\na = if cond\r\n  \"é\r\n  b\"\r\nelse\r\n  %q(c\r\n d)\r\nend # tail\r\n__END__\r\npayload\x00\r\n",
        );
        assert_correction(
            &MultilineTernaryOperator,
            "p __LINE__\ncond ?\n yes : no\n",
            "p __LINE__\nif cond\n  yes\nelse\n  no\nend\n",
        );
    }
    #[test]
    fn correction_work_is_lazy_and_bounded() {
        use crate::cops::testing::correction_offenses;
        let source = "a = cond ?\n yes : no\n".repeat(6000);
        REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&MultilineTernaryOperator], &source).len(),
            6000
        );
        assert_eq!(REWRITE_BYTES.with(|n| n.get()), 0);
        assert_eq!(
            correction_offenses(&MultilineTernaryOperator, &source)
                .iter()
                .filter(|o| o.correction.is_some())
                .count(),
            6000
        );
        assert!(REWRITE_BYTES.with(|n| n.get()) <= source.len());
        let huge = format!("a = cond ? [\n{}] : no\n", "  1,\n".repeat(4000));
        let found = correction_offenses(&MultilineTernaryOperator, &huge);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/multiline_ternary_operator_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 19);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/MultilineTernaryOperator")
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                case["positions"],
                "{case}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                case["messages"],
                "{case}"
            );
            if let Some(after) = case["correction"].as_str() {
                assert_correction(&MultilineTernaryOperator, source, after);
            }
        }
    }
}
