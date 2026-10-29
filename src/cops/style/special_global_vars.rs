//! `Style/SpecialGlobalVars` — fixed English-name policy, detection only.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SpecialGlobalVars;
impl Cop for SpecialGlobalVars {
    fn name(&self) -> &'static str {
        "Style/SpecialGlobalVars"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(read) = node.as_global_variable_read_node() else {
            return;
        };
        let name = read.name().as_slice();
        let (preferred, suffix) = match name {
            b"$:" => ("$LOAD_PATH", ""),
            b"$\"" => ("$LOADED_FEATURES", ""),
            b"$0" => ("$PROGRAM_NAME", ""),
            b"$*" => (
                "$ARGV",
                " from the stdlib 'English' module (don't forget to require it) or `ARGV`",
            ),
            _ => {
                let preferred = match name {
                    b"$!" => "$ERROR_INFO",
                    b"$@" => "$ERROR_POSITION",
                    b"$;" => "$FIELD_SEPARATOR` or `$FS",
                    b"$," => "$OUTPUT_FIELD_SEPARATOR` or `$OFS",
                    b"$/" => "$INPUT_RECORD_SEPARATOR` or `$RS",
                    b"$\\" => "$OUTPUT_RECORD_SEPARATOR` or `$ORS",
                    b"$." => "$INPUT_LINE_NUMBER` or `$NR",
                    b"$_" => "$LAST_READ_LINE",
                    b"$>" => "$DEFAULT_OUTPUT",
                    b"$<" => "$DEFAULT_INPUT",
                    b"$$" => "$PROCESS_ID` or `$PID",
                    b"$?" => "$CHILD_STATUS",
                    b"$~" => "$LAST_MATCH_INFO",
                    b"$=" => "$IGNORECASE",
                    _ => return,
                };
                (
                    preferred,
                    " from the stdlib 'English' module (don't forget to require it)",
                )
            }
        };
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Prefer `{preferred}`{suffix} over `{}`.",
                String::from_utf8_lossy(name)
            ),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/special_global_vars_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Unsafe corrections are omitted.
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SpecialGlobalVars")
            .collect()
    }
    #[test]
    fn pinned_and_remaining_english_mappings_report_exact_messages_without_edits() {
        for (name, preferred, english) in [
            ("$:", "$LOAD_PATH", false),
            ("$\"", "$LOADED_FEATURES", false),
            ("$0", "$PROGRAM_NAME", false),
            ("$!", "$ERROR_INFO", true),
            ("$@", "$ERROR_POSITION", true),
            ("$;", "$FIELD_SEPARATOR` or `$FS", true),
            ("$,", "$OUTPUT_FIELD_SEPARATOR` or `$OFS", true),
            ("$/", "$INPUT_RECORD_SEPARATOR` or `$RS", true),
            ("$\\", "$OUTPUT_RECORD_SEPARATOR` or `$ORS", true),
            ("$.", "$INPUT_LINE_NUMBER` or `$NR", true),
            ("$_", "$LAST_READ_LINE", true),
            ("$>", "$DEFAULT_OUTPUT", true),
            ("$<", "$DEFAULT_INPUT", true),
            ("$$", "$PROCESS_ID` or `$PID", true),
            ("$?", "$CHILD_STATUS", true),
            ("$~", "$LAST_MATCH_INFO", true),
            ("$=", "$IGNORECASE", true),
        ] {
            let source = format!("# header\nputs {name}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 6, 2));
            let suffix = if english {
                " from the stdlib 'English' module (don't forget to require it)"
            } else {
                ""
            };
            assert_eq!(
                found[0].message,
                format!("Prefer `{preferred}`{suffix} over `{name}`.")
            );
            assert!(
                correction_offenses(&SpecialGlobalVars, &source)[0]
                    .correction
                    .is_none()
            );
        }
        assert_eq!(
            offenses("puts $*")[0].message,
            "Prefer `$ARGV` from the stdlib 'English' module (don't forget to require it) or `ARGV` over `$*`."
        );
    }
    #[test]
    fn interpolation_locations_and_english_require_do_not_change_detection() {
        for (source, col) in [("\"#$:\"", 3), ("\"#{$!}\"", 4), ("`#$0`", 3), ("/#$$/", 3)] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
        }
        assert_eq!(offenses("require 'English'\nputs $$\n").len(), 1);
        assert_eq!(offenses("puts $$\nrequire 'English'\n").len(), 1);
        assert_eq!(offenses("puts $*[0]\nputs $*[1]\n").len(), 2);
    }
    #[test]
    fn writes_backrefs_and_already_english_names_are_not_reads_but_aliases_are() {
        for source in [
            "$/ = 'x'",
            "$/ ||= 'x'",
            "$_ &&= 'x'",
            "$0 += 'x'",
            "$0, x = 'x', 1",
            "puts $1; puts $+; puts $&; puts $MATCH; puts $LOAD_PATH; puts $FS; puts $PID; puts $ARGV; puts ARGV",
            "'$/' # $/\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses("$/ = $/").len(), 1);
        let found = offenses("alias $custom $/");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 15));
    }
}
