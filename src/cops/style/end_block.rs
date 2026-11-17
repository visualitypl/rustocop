//! `Style/EndBlock` — prefer at_exit to Perl-style shutdown hooks.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct EndBlock;
impl Cop for EndBlock {
    fn name(&self) -> &'static str {
        "Style/EndBlock"
    }
    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_post_execution_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(hook) = node.as_post_execution_node() else {
            return;
        };
        let keyword = hook.keyword_loc();
        let (line, col) = file.line_col(keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Avoid the use of `END` blocks. Use `Kernel#at_exit` instead.".into(),
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: keyword.start_offset()..keyword.end_offset(),
                    replacement: "at_exit".into(),
                }),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/end_block_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};

    #[test]
    fn pinned_other_block_and_keyword_lookalikes_are_clean() {
        for source in [
            "end_block { test }\n",
            "at_exit { test }",
            "Kernel.at_exit { test }",
            "begin; test; ensure; cleanup; end",
            "BEGIN { test }",
            "'END {}'\n# END {}\n__END__\nEND {}\n",
        ] {
            assert!(
                correction_offenses(&EndBlock, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn simple_prefix_hooks_and_source_envelopes_have_token_only_corrections() {
        for before in [
            "END{}",
            "  END {\n    :work\n  } # kept\n",
            "END { :first }\nEND { :second }\n",
            "x = 1\nEND { [x,__LINE__,__ENCODING__] }\n",
            "\u{feff}# encoding: UTF-8\r\nEND { 'é' }\r\np __LINE__\r\n__END__\r\nEND { raw }\r\n",
            "END { <<~TEXT }\n  END {}\nTEXT\n",
            "x = true\nEND { begin; if x; :yes; else; :no; end; ensure; nil; end }\n",
        ] {
            let after = if let Some((code, data)) = before.split_once("__END__") {
                format!("{}__END__{data}", code.replace("END", "at_exit"))
            } else if before.contains("<<~TEXT") {
                before.replacen("END", "at_exit", 1)
            } else {
                before.replace("END", "at_exit")
            };
            assert_correction(&EndBlock, before, &after);
        }
    }

    #[test]
    fn detection_covers_methods_procs_lambdas_loops_and_nested_hooks() {
        for source in [
            "def f\n  END { work }\nend",
            "proc {\n  END { work }\n}",
            "-> {\n  END { work }\n}",
            "2.times {\n  END { work }\n}",
            "END {\n  END { work }\n}",
        ] {
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == EndBlock.name())
                .collect();
            let offense = found
                .iter()
                .find(|o| o.line == 2)
                .unwrap_or_else(|| panic!("{source}: {found:?}"));
            assert_eq!((offense.col, offense.end_line), (3, 2));
            assert_eq!(
                offense.message,
                "Avoid the use of `END` blocks. Use `Kernel#at_exit` instead."
            );
        }
    }
}
