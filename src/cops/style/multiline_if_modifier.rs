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
                .then(|| rewrite(node, file))
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
/// RuboCop's `autocorrect`.
fn rewrite(node: &Node<'_>, file: &SourceFile) -> Option<Correction> {
    let parts = candidate(node, file)?;
    let loc = node.location();
    let last_line = file.last_line_of(node.location());
    let mut edits = Vec::new();
    let mut moved = |part: &Node<'_>| -> Option<Option<String>> {
        let ranges: Vec<_> = heredocs(part, file)
            .into_iter()
            .filter(|h| file.line_of(h.body) > last_line)
            .collect();
        let start = ranges.iter().map(|h| h.body).min()?;
        let end = ranges.iter().map(|h| h.end).max()?;
        let lines = super::correction::whole_lines(file, start..end);
        let text = std::str::from_utf8(&file.bytes[lines.clone()]).ok()?;
        let text = text.strip_suffix('\n').unwrap_or(text).to_owned();
        edits.push(Correction {
            range: lines,
            replacement: String::new(),
        });
        Some(Some(text))
    };
    let condition_heredoc = moved(&parts.predicate).flatten();
    let body_heredoc = moved(&parts.body).flatten();
    let line_start = file.line_start(file.line_of(loc.start_offset()));
    let offset = " ".repeat(
        std::str::from_utf8(&file.bytes[line_start..loc.start_offset()])
            .ok()?
            .chars()
            .count(),
    );
    let indentation = format!("{offset}  ");
    let verbatim = verbatim_lines(&parts.body, file);
    let body_source = format!(
        "{offset}{}",
        std::str::from_utf8(parts.body.location().as_slice()).ok()?
    );
    let first_line = file.line_of(parts.body.location().start_offset());
    let body: String = body_source
        .split_inclusive('\n')
        .enumerate()
        .map(|(i, line)| {
            if line == "\n" || verbatim.contains(&(first_line + i as u32)) {
                return line.to_owned();
            }
            let leading: usize = line
                .chars()
                .take(offset.len())
                .take_while(|c| c.is_whitespace())
                .map(char::len_utf8)
                .sum();
            if line[..leading].chars().count() == offset.len() {
                format!("{indentation}{}", &line[leading..])
            } else {
                line.to_owned()
            }
        })
        .collect();
    let lines: Vec<String> = [
        Some(format!(
            "{} {}",
            parts.keyword,
            std::str::from_utf8(parts.predicate.location().as_slice()).ok()?
        )),
        condition_heredoc,
        Some(body),
        body_heredoc,
        Some(format!("{offset}end")),
    ]
    .into_iter()
    .flatten()
    .collect();
    edits.push(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: lines.join("\n"),
    });
    super::correction::combine(file, edits)
}

struct Heredoc {
    body: usize,
    end: usize,
}

/// Heredocs in a node: their body start and terminator end (no newline).
fn heredocs(node: &Node<'_>, file: &SourceFile) -> Vec<Heredoc> {
    struct Found<'a> {
        file: &'a SourceFile,
        found: Vec<Heredoc>,
    }
    impl<'pr> NodeVisitor<'pr> for Found<'_> {
        fn enter(&mut self, n: Node<'pr>) {
            let first = |parts: ruby_prism::NodeList<'pr>| {
                parts.iter().next().map(|p| p.location().start_offset())
            };
            let (opening, body, closing) = match &n {
                Node::StringNode { .. } => {
                    let s = n.as_string_node().unwrap();
                    (
                        s.opening_loc(),
                        Some(s.content_loc().start_offset()),
                        s.closing_loc(),
                    )
                }
                Node::InterpolatedStringNode { .. } => {
                    let s = n.as_interpolated_string_node().unwrap();
                    (s.opening_loc(), first(s.parts()), s.closing_loc())
                }
                Node::XStringNode { .. } => {
                    let s = n.as_x_string_node().unwrap();
                    (
                        Some(s.opening_loc()),
                        Some(s.content_loc().start_offset()),
                        Some(s.closing_loc()),
                    )
                }
                Node::InterpolatedXStringNode { .. } => {
                    let s = n.as_interpolated_x_string_node().unwrap();
                    (
                        Some(s.opening_loc()),
                        first(s.parts()),
                        Some(s.closing_loc()),
                    )
                }
                _ => return,
            };
            let (Some(opening), Some(closing)) = (opening, closing) else {
                return;
            };
            if !opening.as_slice().starts_with(b"<<") {
                return;
            }
            let mut end = closing.end_offset();
            while end > closing.start_offset() && matches!(self.file.bytes[end - 1], b'\n' | b'\r')
            {
                end -= 1;
            }
            self.found.push(Heredoc {
                body: body
                    .unwrap_or(closing.start_offset())
                    .min(closing.start_offset()),
                end,
            });
        }
    }
    let mut found = Found {
        file,
        found: Vec::new(),
    };
    walk(&mut found, node);
    found.found
}

/// RuboCop's `verbatim_lines_in`: lines starting inside a string.
fn verbatim_lines(body: &Node<'_>, file: &SourceFile) -> std::collections::HashSet<u32> {
    struct Lines<'a> {
        file: &'a SourceFile,
        lines: std::collections::HashSet<u32>,
    }
    impl<'pr> NodeVisitor<'pr> for Lines<'_> {
        fn enter(&mut self, n: Node<'pr>) {
            let (opening, closing) = match &n {
                Node::StringNode { .. } => {
                    let s = n.as_string_node().unwrap();
                    (s.opening_loc(), s.closing_loc())
                }
                Node::InterpolatedStringNode { .. } => {
                    let s = n.as_interpolated_string_node().unwrap();
                    (s.opening_loc(), s.closing_loc())
                }
                Node::XStringNode { .. } => {
                    let s = n.as_x_string_node().unwrap();
                    (Some(s.opening_loc()), Some(s.closing_loc()))
                }
                Node::InterpolatedXStringNode { .. } => {
                    let s = n.as_interpolated_x_string_node().unwrap();
                    (Some(s.opening_loc()), Some(s.closing_loc()))
                }
                _ => return,
            };
            let (Some(opening), Some(closing)) = (opening, closing) else {
                return;
            };
            if opening.as_slice().starts_with(b"<<") {
                if !opening.as_slice().starts_with(b"<<~") {
                    let first = self.file.line_of(opening.start_offset()) + 1;
                    let last = self.file.line_of(closing.start_offset());
                    self.lines.extend(first..=last);
                }
            } else {
                let loc = n.location();
                self.lines.extend(
                    self.file.line_of(loc.start_offset()) + 1..=self.file.last_line_of(loc),
                );
            }
        }
    }
    let mut lines = Lines {
        file,
        lines: Default::default(),
    };
    walk(&mut lines, body);
    lines.lines
}
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
}
