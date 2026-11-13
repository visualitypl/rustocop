//! `Style/BeginBlock` — discourage Perl-style startup hooks (detection only).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct BeginBlock;
impl Cop for BeginBlock {
    fn name(&self) -> &'static str {
        "Style/BeginBlock"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_pre_execution_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(hook) = node.as_pre_execution_node() else {
            return;
        };
        let (line, col) = file.line_col(hook.keyword_loc().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Avoid the use of `BEGIN` blocks.".into(),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/begin_block_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;

    #[test]
    fn pinned_keyword_position_message_and_no_correction() {
        let found = correction_offenses(&BeginBlock, "BEGIN { test }\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
        assert_eq!(found[0].message, "Avoid the use of `BEGIN` blocks.");
        assert!(found[0].correction.is_none());
        assert!(!BeginBlock.safe_fixes());
    }

    #[test]
    fn multiple_multiline_startup_hooks_report_only_the_keywords() {
        let source = "# startup\n  BEGIN {\n    work\n  }\nBEGIN {}\n";
        let found: Vec<_> = crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == BeginBlock.name())
            .collect();
        assert_eq!(
            found
                .iter()
                .map(|o| (o.line, o.col, o.end_line))
                .collect::<Vec<_>>(),
            [(2, 3, 2), (5, 1, 5)]
        );
        assert!(
            found
                .iter()
                .all(|o| o.message == "Avoid the use of `BEGIN` blocks.")
        );
    }

    #[test]
    fn ordinary_begins_strings_comments_and_data_are_not_hooks() {
        for source in [
            "begin; test; rescue; fallback; ensure; cleanup; end",
            "begin_block { test }",
            "'BEGIN { test }'\n# BEGIN {}\n__END__\nBEGIN {}\n",
            "END { test }",
        ] {
            assert!(
                correction_offenses(&BeginBlock, source).is_empty(),
                "{source}"
            );
        }
    }
}
