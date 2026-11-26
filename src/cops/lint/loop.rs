//! `Lint/Loop` — posttest begin/end loops, without unsafe rewrites.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct Loop;
impl Cop for Loop {
    fn name(&self) -> &'static str {
        "Lint/Loop"
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
        let keyword = if let Some(n) = node.as_while_node() {
            n.is_begin_modifier().then(|| n.keyword_loc())
        } else if let Some(n) = node.as_until_node() {
            n.is_begin_modifier().then(|| n.keyword_loc())
        } else {
            None
        };
        let Some(keyword) = keyword else { return };
        let correction = cx
            .corrections_requested()
            .then(|| correction(node, file))
            .flatten();
        let (line, col) = file.reference_line_col(keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `Kernel#loop` with `break` rather than `begin/end/until`(or `while`)."
                .into(),
            line,
            col,
            end_line: line,
            correction,
        });
    }
}

/// Upstream's corrector: `begin` becomes `loop do`, the condition goes, and
/// a `break` with the inverted keyword goes before `end`.
fn correction(node: &Node<'_>, file: &SourceFile) -> Option<crate::offense::Correction> {
    let (statements, predicate, until) = match node.as_while_node() {
        Some(n) => (n.statements()?, n.predicate(), false),
        None => {
            let n = node.as_until_node()?;
            (n.statements()?, n.predicate(), true)
        }
    };
    let body = statements.body().iter().next()?;
    let begin = body.as_begin_node()?;
    let (open, end) = (begin.begin_keyword_loc()?, begin.end_keyword_loc()?);
    let column = |offset: usize| file.line_col(offset).1 as usize - 1;
    let indent = " ".repeat(column(node.location().start_offset()));
    let single = file.line_of(body.location().start_offset()) == file.line_of(end.start_offset());
    let keyword = if until { "if" } else { "unless" };
    let mut text = format!(
        "break {keyword} {}\n{indent}",
        String::from_utf8_lossy(predicate.location().as_slice())
    );
    if single {
        text = format!("\n{indent}  {text}");
    }
    let mut edits = vec![
        (open.start_offset()..open.end_offset(), "loop do".to_owned()),
        (
            end.end_offset()..node.location().end_offset(),
            String::new(),
        ),
    ];
    if single {
        let mut from = end.start_offset();
        while from > 0 && matches!(file.bytes[from - 1], b' ' | b'\t') {
            from -= 1;
        }
        edits.push((from..end.start_offset(), text));
    } else {
        edits.push((end.start_offset()..end.start_offset(), text));
    }
    crate::offense::Correction::from_edits(&file.bytes, edits)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/loop_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/Loop")
            .collect()
    }
    #[test]
    fn pinned_posttest_keyword_positions_and_messages() {
        for keyword in ["while", "until"] {
            for (source, line, col) in [
                (format!("begin\n  something\nend {keyword} test\n"), 3, 5),
                (format!("begin something end {keyword} test\n"), 1, 21),
            ] {
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (line, col, line)
                );
                assert_eq!(
                    found[0].message,
                    "Use `Kernel#loop` with `break` rather than `begin/end/until`(or `while`)."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn ignores_pretest_loops_and_lookalikes() {
        for source in [
            "loop do; one; two; break unless test; end",
            "loop do; one; two; break if test; end",
            "while test; something; end",
            "until test; something; end",
            "something while test",
            "something until test",
            "(begin; something; end) while test",
            "while begin; test; end; something; end",
            "'begin something end while test' # begin end until test",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
