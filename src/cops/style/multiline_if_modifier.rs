//! `Style/MultilineIfModifier` — expand modifiers with multiline bodies.
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MultilineIfModifier;
impl Cop for MultilineIfModifier {
    fn name(&self) -> &'static str {
        "Style/MultilineIfModifier"
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
        let Some(parts) = candidate(node, file) else {
            return;
        };
        // Upstream ignores the whole subtree after reporting its outer modifier.
        if cx.ancestors().any(|n| candidate(n, file).is_some()) {
            return;
        }
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: format!(
                "Favor a normal {}-statement over a modifier clause in a multiline statement.",
                parts.keyword
            ),
            correction: cx
                .corrections_requested()
                .then(|| rewrite(node, cx, file))
                .flatten(),
        });
    }
}
struct Parts<'pr> {
    body: Node<'pr>,
    predicate: Node<'pr>,
    keyword: &'static str,
}
fn candidate<'pr>(node: &Node<'pr>, file: &SourceFile) -> Option<Parts<'pr>> {
    let (body, predicate, keyword) = if let Some(n) = node.as_if_node() {
        if n.end_keyword_loc().is_some() || n.if_keyword_loc().is_none() {
            return None;
        }
        (n.statements()?, n.predicate(), "if")
    } else {
        let n = node.as_unless_node()?;
        if n.end_keyword_loc().is_some() {
            return None;
        }
        (n.statements()?, n.predicate(), "unless")
    };
    let body = body.body().iter().next()?;
    (file.line_of(body.location().start_offset()) != file.last_line_of(body.location())).then_some(
        Parts {
            body,
            predicate,
            keyword,
        },
    )
}
const REWRITE_BUDGET: usize = 16 * 1024;
fn rewrite(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> Option<Correction> {
    let loc = node.location();
    let range = loc.start_offset()..loc.end_offset();
    if range.len() > REWRITE_BUDGET || super::correction::touches_directive(file, range.clone()) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    // Deferred heredoc payloads need a different physical ownership proof. Do
    // not move them, or add lines while an external payload is pending.
    if file.contains_heredoc(root, node) || file.heredoc_pending_at(root, range.start) {
        return None;
    }
    let prefix = &file.bytes[file.line_start(file.line_of(range.start))..range.start];
    let prefix = if file.line_of(range.start) == 1 {
        prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix)
    } else {
        prefix
    };
    if !prefix.iter().all(|b| matches!(b, b' ' | b'\t')) || prefix.len() > REWRITE_BUDGET {
        return None;
    }
    let suffix = &file.bytes[range.end..file.line_start(file.line_of(range.end) + 1)];
    if !suffix.trim_ascii_start().is_empty() && !suffix.trim_ascii_start().starts_with(b"#") {
        return None;
    }
    let proof = file.class_method_bindings(root);
    if proof.observes_sources {
        return None;
    }
    let comments = file.comments();
    let first = comments.partition_point(|c| c.end <= range.start);
    if comments[first..]
        .iter()
        .take_while(|c| c.start < range.end)
        .any(|c| {
            let bytes = &file.bytes[c.clone()];
            bytes.starts_with(b"=begin") || !super::correction::comment_relocation_safe(bytes)
        })
    {
        return None;
    }
    let indent = std::str::from_utf8(prefix).ok()?;
    let nl = if file.bytes.get(
        file.line_start(file.line_of(range.start) + 1)
            .saturating_sub(2),
    ) == Some(&b'\r')
    {
        "\r\n"
    } else {
        "\n"
    };
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + range.len()));
    let replacement = render(node, indent, indent, nl, file)?;
    let edit = Correction { range, replacement };
    (super::correction::preserves_line_values(file, &edit)
        && super::correction::preserves_data_positions(file, cx, &edit))
    .then_some(edit)
}
fn render(
    node: &Node<'_>,
    original_indent: &str,
    indent: &str,
    nl: &str,
    file: &SourceFile,
) -> Option<String> {
    let parts = candidate(node, file)?;
    if !super::if_unless_modifier::expansion_binding_safe(&parts.body, &parts.predicate)
        || super::correction::contains_comment(
            file,
            // Reconstruction retains body/predicate text only. Comments in the
            // intervening modifier header cannot be silently discarded, even
            // when this modifier is recursively expanded inside another one.
            parts.body.location().end_offset()..parts.predicate.location().end_offset(),
        )
    {
        return None;
    }
    let predicate = std::str::from_utf8(parts.predicate.location().as_slice()).ok()?;
    let inner = format!("{indent}  ");
    let body = if candidate(&parts.body, file).is_some() {
        format!(
            "{inner}{}",
            render(&parts.body, original_indent, &inner, nl, file)?
        )
    } else {
        let mut verbatim = Verbatim {
            file,
            lines: std::collections::HashSet::new(),
        };
        walk(&mut verbatim, &parts.body);
        let text = std::str::from_utf8(parts.body.location().as_slice()).ok()?;
        let start_line = file.line_of(parts.body.location().start_offset());
        text.split_inclusive('\n')
            .enumerate()
            .map(|(index, text)| {
                let line = start_line + index as u32;
                if index != 0 && (verbatim.lines.contains(&line) || text.trim().is_empty()) {
                    return text.to_owned();
                }
                if index == 0 {
                    return format!("{inner}{text}");
                }
                if text
                    .as_bytes()
                    .get(..original_indent.len())
                    .is_some_and(|p| p.iter().all(|b| matches!(b, b' ' | b'\t')))
                {
                    format!("{inner}{}", &text[original_indent.len()..])
                } else {
                    text.to_owned()
                }
            })
            .collect::<String>()
    };
    Some(format!(
        "{} {predicate}{nl}{body}{nl}{indent}end",
        parts.keyword
    ))
}
struct Verbatim<'a> {
    file: &'a SourceFile,
    lines: std::collections::HashSet<u32>,
}
impl<'pr> NodeVisitor<'pr> for Verbatim<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        // Lines starting inside literals are value bytes, not indentation.
        if matches!(
            node,
            Node::StringNode { .. }
                | Node::InterpolatedStringNode { .. }
                | Node::SymbolNode { .. }
                | Node::InterpolatedSymbolNode { .. }
                | Node::RegularExpressionNode { .. }
                | Node::InterpolatedRegularExpressionNode { .. }
                | Node::MatchLastLineNode { .. }
                | Node::InterpolatedMatchLastLineNode { .. }
                | Node::XStringNode { .. }
                | Node::InterpolatedXStringNode { .. }
        ) {
            let loc = node.location();
            let start = self.file.line_of(loc.start_offset());
            let end = self.file.last_line_of(loc);
            self.lines.extend(start + 1..=end);
        }
    }
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! RuboCop multiline_if_modifier_spec.rb examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn detects_body_not_condition_lines() {
        for keyword in ["if", "unless"] {
            let source = format!("  {{\n    result: run\n  }} {keyword} cond\n");
            let found = correction_offenses(&MultilineIfModifier, &source);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, 3));
            assert_eq!(
                found[0].message,
                format!(
                    "Favor a normal {keyword}-statement over a modifier clause in a multiline statement."
                )
            );
            assert!(
                correction_offenses(
                    &MultilineIfModifier,
                    &format!("run {keyword} cond &&\n cond2\n")
                )
                .is_empty()
            );
            assert_correction(
                &MultilineIfModifier,
                &source,
                &format!("  {keyword} cond\n    {{\n      result: run\n    }}\n  end\n"),
            );
        }
    }
    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/multiline_if_modifier_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 17);
        let mut corrections = 0;
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/MultilineIfModifier")
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
                corrections += 1;
                // Deferred payload movement / heredoc indentation ownership is
                // deliberately not guessed; every one remains diagnosed.
                if [70, 89, 108, 137, 158, 179].contains(&case["spec_line"].as_u64().unwrap()) {
                    assert!(
                        correction_offenses(&MultilineIfModifier, source)
                            .iter()
                            .all(|o| o.correction.is_none())
                    );
                } else {
                    assert_correction(&MultilineIfModifier, source, after);
                }
            }
        }
        assert_eq!(corrections, 13);
    }
    #[test]
    fn ambiguous_binding_and_control_flow_remain_diagnosed_without_edits() {
        for source in [
            "[\n x\n] if (x = 1)\n",
            "[\n x = 1\n] if defined?(x)\n",
            "[\n x = 1\n] if x\n",
            "[\n /(?<x>a)/ =~ 'a'\n] if x\n",
            "begin\n work\nrescue\n recover\nensure\n cleanup\nend if ready\n",
            "[\n (work rescue recover)\n] if ready\n",
            "while ready\n work\nend if ready\n",
            "def f\n work\nend if ready\n",
            "[\n] if (ready # header\n)\n",
            "consume(([\n] if ready))\n",
        ] {
            let found = correction_offenses(&MultilineIfModifier, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn multiline_literal_bytes_and_ordinary_comments_survive() {
        assert_correction(
            &MultilineIfModifier,
            "[\n  \"a\n  b\", /c\n  d/x, %s(e\n  f) # note\n] unless ready # trailing\n",
            "unless ready\n  [\n    \"a\n  b\", /c\n  d/x, %s(e\n  f) # note\n  ]\nend # trailing\n",
        );
    }
    #[test]
    fn implicit_match_regexp_interiors_remain_verbatim() {
        for regexp in ["/a\n  b/", "/#{'a'}\n  b/"] {
            assert_correction(
                &MultilineIfModifier,
                &format!("p [\n  (:matched if {regexp})\n] if true\n"),
                &format!("if true\n  p [\n    (:matched if {regexp})\n  ]\nend\n"),
            );
        }
    }
    #[test]
    fn comments_between_modifier_keyword_and_predicate_withhold_edits() {
        for keyword in ["if", "unless"] {
            for source in [
                format!("[\n  work\n] {keyword} # important explanation\n ready\n"),
                format!("[\n  work\n] {keyword} # inner explanation\n ready {keyword} outer\n"),
            ] {
                let found = correction_offenses(&MultilineIfModifier, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 1));
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn line_values_observers_and_directives_withhold_individual_changes() {
        for source in [
            "[\n __LINE__\n] if ready\n",
            "[\n] if ready\np __LINE__\n",
            "[\n DATA.pos\n] if ready\n__END__\npayload\n",
            "[\n method(:f).source_location\n] if ready\n",
            "[\n # coding: utf-8\n] if ready\n",
            "[\n # rubocop:disable Lint/Debugger\n] if ready\n",
            "[\n=begin\ncomment\n=end\n] if ready\n",
        ] {
            let found = correction_offenses(&MultilineIfModifier, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn ordinary_newlines_preserve_bom_crlf_data_and_following_safe_siblings() {
        assert_correction(
            &MultilineIfModifier,
            "\u{feff}# coding: utf-8\r\n[\r\n  \"é\"\r\n] unless ready\r\n__END__\r\npayload\x00\r\n",
            "\u{feff}# coding: utf-8\r\nunless ready\r\n  [\r\n    \"é\"\r\n  ]\r\nend\r\n__END__\r\npayload\x00\r\n",
        );
        assert_correction(
            &MultilineIfModifier,
            "p __LINE__\n[\n] if ready\n",
            "p __LINE__\nif ready\n  [\n  ]\nend\n",
        );
    }
    #[test]
    fn correction_work_is_lazy_bounded_and_linear_across_siblings() {
        let source = "[\n] if ready\n".repeat(6000);
        REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&MultilineIfModifier], &source).len(),
            6000
        );
        assert_eq!(REWRITE_BYTES.with(|n| n.get()), 0);
        let found = correction_offenses(&MultilineIfModifier, &source);
        assert_eq!(
            found.iter().filter(|o| o.correction.is_some()).count(),
            6000
        );
        assert!(REWRITE_BYTES.with(|n| n.get()) <= source.len());
        let huge = format!("[\n{}] if ready\n", "  1,\n".repeat(4000));
        let found = correction_offenses(&MultilineIfModifier, &huge);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn expands_nested_modifiers_atomically() {
        for keyword in ["if", "unless"] {
            assert_correction(
                &MultilineIfModifier,
                &format!("[\n] {keyword} inner {keyword} outer\n"),
                &format!("{keyword} outer\n  {keyword} inner\n    [\n    ]\n  end\nend\n"),
            );
        }
    }
}
