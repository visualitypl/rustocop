//! `Style/IfInsideElse` — flatten an else-only nested if (modifiers included).

use crate::{
    cops::{
        Cop, NodeContext, variables,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{IfNode, Location, Node};
use std::ops::Range;

#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub struct IfInsideElse;

const REWRITE_BUDGET: usize = 32 * 1024;
impl Cop for IfInsideElse {
    fn name(&self) -> &'static str {
        "Style/IfInsideElse"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_if_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(outer) = node.as_if_node() else {
            return;
        };
        if outer.if_keyword_loc().is_none() {
            return;
        }
        let Some(other) = outer.subsequent().and_then(|n| n.as_else_node()) else {
            return;
        };
        let Some(body) = other.statements() else {
            return;
        };
        if body.body().len() != 1 {
            return;
        }
        let inner_node = body.body().iter().next().unwrap();
        let Some(inner) = inner_node.as_if_node() else {
            return;
        };
        let Some(keyword) = inner.if_keyword_loc() else {
            return;
        };
        if inner.end_keyword_loc().is_some() {
            // Upstream compares comment *starts* strictly after else's end;
            // an attached `else# comment` is not part of this exclusion.
            let comments = file.comments();
            let first =
                comments.partition_point(|c| c.start <= other.else_keyword_loc().end_offset());
            if comments
                .get(first)
                .is_some_and(|c| c.start < keyword.start_offset())
            {
                return;
            }
        }
        let (line, col) = file.line_col(keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Convert `if` nested inside `else` to `elsif`.".into(),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| correction(&inner, other.else_keyword_loc(), cx, file))
                .flatten(),
        });
    }
}

fn range(loc: Location<'_>) -> Range<usize> {
    loc.start_offset()..loc.end_offset()
}
fn edit(range: Range<usize>, replacement: impl Into<String>) -> Correction {
    Correction {
        range,
        replacement: replacement.into(),
    }
}
fn whole_lines(file: &SourceFile, r: Range<usize>) -> Range<usize> {
    file.line_start(file.line_of(r.start))
        ..file.line_start(file.line_of(r.end.saturating_sub(1).max(r.start)) + 1)
}
fn begins_line(file: &SourceFile, offset: usize) -> bool {
    file.bytes[file.line_start(file.line_of(offset))..offset]
        .iter()
        .all(|b| matches!(b, b' ' | b'\t'))
}
fn ends_line(file: &SourceFile, offset: usize) -> bool {
    file.bytes[offset..file.line_start(file.line_of(offset) + 1)]
        .iter()
        .all(u8::is_ascii_whitespace)
}

fn correction(
    inner: &IfNode<'_>,
    outer_else: Location<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    // Fixed local work budget keeps nested eligible conditionals bounded. No
    // correction-only subtree walks, source copies or proofs during plain lint.
    let loc = inner.location();
    let span = outer_else.start_offset()..loc.end_offset();
    if span.len() > REWRITE_BUDGET {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, &inner.as_node()) || file.heredoc_pending_at(root, span.start) {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + span.len()));
    let keyword = inner.if_keyword_loc()?;
    let predicate = inner.predicate();
    let text = std::str::from_utf8(predicate.location().as_slice()).ok()?;
    let mut edits = vec![edit(range(outer_else), format!("elsif {text}"))];
    if inner.end_keyword_loc().is_none() {
        if !begins_line(file, loc.start_offset()) {
            return None;
        }
        let body = inner.statements()?;
        // Reversing lexical order can change reads to calls (or calls to
        // reads), including captures/patterns and block-owned declarations.
        // Use shared variable classification; conservatively retain offenses
        // for any declaration in either half, even when previously bound.
        struct Declarations(bool);
        impl<'pr> NodeVisitor<'pr> for Declarations {
            fn enter(&mut self, n: Node<'pr>) {
                self.0 |= variables::is_local_assignment(&n) || n.as_match_write_node().is_some();
            }
        }
        let mut declarations = Declarations(false);
        walk(&mut declarations, &inner.as_node());
        if declarations.0 {
            return None;
        }
        edits.push(edit(
            body.location().end_offset()..predicate.location().end_offset(),
            "",
        ));
    } else if inner.then_keyword_loc().is_some() {
        // Match upstream's first explicit pass, not its implicit correction
        // loop. Expanding then syntax leaves the nesting for a later run.
        if super::correction::contains_comment(file, range(inner.location())) {
            return None;
        }
        let indent = &file.bytes
            [file.line_start(file.line_of(keyword.start_offset()))..keyword.start_offset()];
        if !indent.iter().all(|b| matches!(b, b' ' | b'\t')) {
            return None;
        }
        let indent = std::str::from_utf8(indent).ok()?;
        return Some(edit(range(loc), expand_then(inner, indent, file)?));
    } else {
        let header = keyword.start_offset()..predicate.location().end_offset();
        let end = range(inner.end_keyword_loc()?);
        if !begins_line(file, header.start)
            || !ends_line(file, header.end)
            || !begins_line(file, end.start)
            || !ends_line(file, end.end)
        {
            return None;
        }
        if let Some(body) = inner.statements() {
            let mut body_range = range(body.location());
            let comments = file.comments();
            let first = comments.partition_point(|c| c.end <= header.end);
            let comments_end = file.line_start(file.line_of(body_range.end) + 1);
            for comment in comments[first..]
                .iter()
                .take_while(|c| c.start < comments_end)
            {
                // Embedded-document delimiters must stay in column one;
                // moving one onto the indented header makes it executable.
                if file.bytes[comment.clone()].starts_with(b"=begin") {
                    return None;
                }
                body_range.start = body_range.start.min(comment.start);
                body_range.end = body_range.end.max(comment.end);
            }
            if !begins_line(file, body_range.start) || !ends_line(file, body_range.end) {
                return None;
            }
            edits.push(edit(
                header,
                std::str::from_utf8(&file.bytes[body_range.clone()]).ok()?,
            ));
            edits.push(edit(whole_lines(file, body_range), ""));
        } else {
            edits.push(edit(whole_lines(file, header), ""));
        }
        edits.push(edit(whole_lines(file, end), ""));
    }
    super::correction::combine(file, edits)
}
fn expand_then(inner: &IfNode<'_>, indent: &str, file: &SourceFile) -> Option<String> {
    let keyword = inner.if_keyword_loc()?;
    let line_end = file.line_start(file.line_of(keyword.start_offset()) + 1);
    let newline = if file.bytes.get(line_end.saturating_sub(2)..line_end) == Some(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    // Iterate the elsif spine instead of recursively rebuilding its suffix.
    let mut current = inner.as_node();
    let mut out = String::new();
    let mut elsif = false;
    loop {
        let branch = current.as_if_node()?;
        let predicate = branch.predicate();
        let predicate = std::str::from_utf8(predicate.location().as_slice()).ok()?;
        let body = branch.statements();
        let body = body
            .as_ref()
            .map(|b| std::str::from_utf8(b.location().as_slice()))
            .transpose()
            .ok()?
            .unwrap_or("nil");
        out.push_str(&format!(
            "{}{word} {predicate}{newline}{indent}{body}{newline}",
            if elsif { indent } else { "" },
            word = if elsif { "elsif" } else { "if" }
        ));
        if out.len() > REWRITE_BUDGET {
            return None;
        }
        let Some(other) = branch.subsequent() else {
            out.push_str("end");
            break;
        };
        if other.as_if_node().is_some() {
            current = other;
            elsif = true;
            continue;
        }
        let other = other.as_else_node()?;
        let body = other.statements();
        let body = body
            .as_ref()
            .map(|b| std::str::from_utf8(b.location().as_slice()))
            .transpose()
            .ok()?
            .unwrap_or("nil");
        out.push_str(&format!(
            "{indent}else{newline}{indent}{body}{newline}{indent}end"
        ));
        break;
    }
    (out.len() <= REWRITE_BUDGET).then_some(out)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `style/if_inside_else_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Case {
        spec_line: u32,
        source: String,
        positions: Vec<(u32, u32)>,
        messages: Vec<String>,
        correction: Option<String>,
    }
    #[test]
    fn all_pinned_fixed_policy_diagnostics() {
        let cases: Vec<Case> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/if_inside_else_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 29);
        for c in cases {
            let found: Vec<_> = crate::pipeline::lint_str(&c.source)
                .into_iter()
                .filter(|o| o.cop == "Style/IfInsideElse")
                .collect();
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                c.positions,
                "spec {}",
                c.spec_line
            );
            assert_eq!(
                found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
                c.messages,
                "spec {}",
                c.spec_line
            );
            assert!(
                found
                    .iter()
                    .all(|o| o.end_line == o.line && o.correction.is_none())
            );
        }
    }
    #[test]
    fn canonical_multiline_and_modifier_corrections() {
        // Pinned upstream tests comments strictly after the else token end.
        assert_correction(
            &IfInsideElse,
            "if a\nelse# attached\n  if b\n    work\n  end\nend\n",
            "if a\nelsif b# attached\n  work\nend\n",
        );
        assert_correction(
            &IfInsideElse,
            "if a\n  blah\nelse\n  if b\n    foo\n  end\nend\n",
            "if a\n  blah\nelsif b\n  foo\nend\n",
        );
        assert_correction(
            &IfInsideElse,
            "if a\n  blah\nelse\n  foo if b\nend\n",
            "if a\n  blah\nelsif b\n  foo\nend\n",
        );
    }
    #[test]
    fn withholds_uncertain_comments_heredocs_and_shared_lines() {
        for source in [
            "if a\nelse\n  if b # condition\n    work\n  end\nend\n",
            "if a\nelse\n  if b\n    work\n  end # closing\nend\n",
            "if a\nelse\n  if b\n    <<~TEXT\n      hello\n    TEXT\n  end\nend\n",
            "if a\nelse\n  if b then foo # important\n  end\nend\n",
            "if a\nelse\n  if b\n=begin\nimportant\n=end\n    work\n  end\nend\n",
        ] {
            let found = correction_offenses(&IfInsideElse, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn no_offense_lookalikes_preserve_else_association_and_begin_boundaries() {
        for source in [
            "unless a\nelse\n  if b\n    work\n  end\nend\n",
            "if a\nelse\n  begin\n    if b\n      work\n    end\n  end\nend\n",
            "if a\nelse\n  (if b\n    work\n  end)\nend\n",
            "if a\nelse # comment\n  if b\n    work\n  end\nend\n",
            "if a\nelse\n  work unless b\nend\n",
        ] {
            assert!(
                correction_offenses(&IfInsideElse, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn preserves_crlf_bom_encoding_and_unread_data_payload() {
        assert_correction(
            &IfInsideElse,
            "\u{feff}# encoding: UTF-8\r\nif a\r\nelse\r\n  if b\r\n    work\r\n  end\r\nend\r\n__END__\r\nraw if b\r\n",
            "\u{feff}# encoding: UTF-8\r\nif a\r\nelsif b\r\n  work\r\nend\r\n__END__\r\nraw if b\r\n",
        );
    }
    #[test]
    fn correction_work_is_lazy_and_bounded_over_wide_and_large_sources() {
        let source = "if a\nelse\n  if b\n    work\n  end\nend\n".repeat(2000);
        REWRITE_BYTES.with(|n| n.set(0));
        let found: Vec<_> = crate::pipeline::lint_str(&source)
            .into_iter()
            .filter(|o| o.cop == "Style/IfInsideElse")
            .collect();
        assert_eq!(found.len(), 2000);
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        let found = correction_offenses(&IfInsideElse, &source);
        assert!(found.iter().all(|o| o.correction.is_some()));
        REWRITE_BYTES.with(|n| assert!(n.get() <= source.len()));
        let source = format!(
            "if a\nelse\n  if b\n{}  end\nend\n",
            "    work\n".repeat(5000)
        );
        REWRITE_BYTES.with(|n| n.set(0));
        let found = correction_offenses(&IfInsideElse, &source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        // A short one-line elsif spine must not multiply a very long indent
        // into an arbitrarily large generated correction.
        let source = format!(
            "if a\nelse\n{}if b then 1 {} else 3 end\nend\n",
            " ".repeat(1024),
            "elsif c then 2 ".repeat(40)
        );
        let found = correction_offenses(&IfInsideElse, &source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn every_pinned_correction_and_explicit_withhold() {
        let cases: Vec<Case> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/if_inside_else_pinned.json"
        ))
        .unwrap();
        for c in cases {
            if let Some(mut after) = c.correction {
                // These upstream expectations include a second implicit pass.
                // Our CLI deliberately expands then syntax in one pass only.
                if c.spec_line == 186 {
                    after = "if x\n  'x'\nelse\n  if y\n  'y'\n  else\n  'z'\n  end\nend\n".into();
                }
                if c.spec_line == 230 {
                    after = "if x\n  'x'\nelse\n  if y\n  'y'\n  elsif z\n  'z'\n  else\n  'a'\n  end\nend\n".into();
                }
                assert_correction(&IfInsideElse, &c.source, &after);
            } else if !c.positions.is_empty() {
                assert!(
                    correction_offenses(&IfInsideElse, &c.source)
                        .iter()
                        .all(|o| o.correction.is_none()),
                    "spec {}",
                    c.spec_line
                );
            }
        }
    }
}
