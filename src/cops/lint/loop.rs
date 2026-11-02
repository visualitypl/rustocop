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
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
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
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(keyword.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `Kernel#loop` with `break` rather than `begin/end/until`(or `while`)."
                .into(),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
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
