//! `Style/GlobalVars` — do not introduce custom global state.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct GlobalVars;
impl Cop for GlobalVars {
    fn name(&self) -> &'static str {
        "Style/GlobalVars"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(loc) = global_name_location(node) else {
            return;
        };
        if BUILT_INS.contains(&loc.as_slice()) {
            return;
        }
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not introduce global variables.".into(),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}

/// Global reads and assignment names, excluding backreference reads.
fn global_name_location<'pr>(node: &Node<'pr>) -> Option<ruby_prism::Location<'pr>> {
    if node.as_global_variable_read_node().is_some() {
        return Some(node.location());
    }
    macro_rules! writes {
        ($($access:ident),*) => { $(if let Some(n) = node.$access() { return Some(n.name_loc()); })* };
    }
    writes!(
        as_global_variable_write_node,
        as_global_variable_or_write_node,
        as_global_variable_and_write_node,
        as_global_variable_operator_write_node
    );
    node.as_global_variable_target_node().map(|n| n.location())
}

const BUILT_INS: &[&[u8]] = &[
    b"$:",
    b"$LOAD_PATH",
    b"$\"",
    b"$LOADED_FEATURES",
    b"$0",
    b"$PROGRAM_NAME",
    b"$!",
    b"$ERROR_INFO",
    b"$@",
    b"$ERROR_POSITION",
    b"$;",
    b"$FS",
    b"$FIELD_SEPARATOR",
    b"$,",
    b"$OFS",
    b"$OUTPUT_FIELD_SEPARATOR",
    b"$/",
    b"$RS",
    b"$INPUT_RECORD_SEPARATOR",
    b"$\\",
    b"$ORS",
    b"$OUTPUT_RECORD_SEPARATOR",
    b"$.",
    b"$NR",
    b"$INPUT_LINE_NUMBER",
    b"$_",
    b"$LAST_READ_LINE",
    b"$>",
    b"$DEFAULT_OUTPUT",
    b"$<",
    b"$DEFAULT_INPUT",
    b"$$",
    b"$PID",
    b"$PROCESS_ID",
    b"$?",
    b"$CHILD_STATUS",
    b"$~",
    b"$LAST_MATCH_INFO",
    b"$=",
    b"$IGNORECASE",
    b"$*",
    b"$ARGV",
    b"$&",
    b"$MATCH",
    b"$`",
    b"$PREMATCH",
    b"$'",
    b"$POSTMATCH",
    b"$+",
    b"$LAST_PAREN_MATCH",
    b"$stdin",
    b"$stdout",
    b"$stderr",
    b"$DEBUG",
    b"$FILENAME",
    b"$VERBOSE",
    b"$SAFE",
    b"$-0",
    b"$-a",
    b"$-d",
    b"$-F",
    b"$-i",
    b"$-I",
    b"$-l",
    b"$-p",
    b"$-v",
    b"$-w",
    b"$CLASSPATH",
    b"$JRUBY_VERSION",
    b"$JRUBY_REVISION",
    b"$ENV_JAVA",
];

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/global_vars_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov). AllowedVariables is fixed empty.
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/GlobalVars")
            .collect()
    }
    #[test]
    fn custom_reads_and_all_assignment_shapes_report_only_the_name() {
        for code in [
            "$custom",
            "$custom = 1",
            "$custom ||= 1",
            "$custom &&= 1",
            "$custom += 1",
            "$custom, x = 1, 2",
            "for $custom in []; end",
        ] {
            let source = format!("# header\n  {code}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, if code.starts_with("for ") { 7 } else { 3 }, 2)
            );
            assert_eq!(found[0].message, "Do not introduce global variables.");
            assert!(
                correction_offenses(&GlobalVars, &source)[0]
                    .correction
                    .is_none()
            );
        }
        assert_eq!(offenses("puts $allowed").len(), 1);
    }
    #[test]
    fn every_pinned_builtin_and_numbered_backreference_is_allowed() {
        for name in [
            "$:",
            "$LOAD_PATH",
            "$\"",
            "$LOADED_FEATURES",
            "$0",
            "$PROGRAM_NAME",
            "$!",
            "$ERROR_INFO",
            "$@",
            "$ERROR_POSITION",
            "$;",
            "$FS",
            "$FIELD_SEPARATOR",
            "$,",
            "$OFS",
            "$OUTPUT_FIELD_SEPARATOR",
            "$/",
            "$RS",
            "$INPUT_RECORD_SEPARATOR",
            "$\\",
            "$ORS",
            "$OUTPUT_RECORD_SEPARATOR",
            "$.",
            "$NR",
            "$INPUT_LINE_NUMBER",
            "$_",
            "$LAST_READ_LINE",
            "$>",
            "$DEFAULT_OUTPUT",
            "$<",
            "$DEFAULT_INPUT",
            "$$",
            "$PID",
            "$PROCESS_ID",
            "$?",
            "$CHILD_STATUS",
            "$~",
            "$LAST_MATCH_INFO",
            "$=",
            "$IGNORECASE",
            "$*",
            "$ARGV",
            "$&",
            "$MATCH",
            "$`",
            "$PREMATCH",
            "$'",
            "$POSTMATCH",
            "$+",
            "$LAST_PAREN_MATCH",
            "$stdin",
            "$stdout",
            "$stderr",
            "$DEBUG",
            "$FILENAME",
            "$VERBOSE",
            "$SAFE",
            "$-0",
            "$-a",
            "$-d",
            "$-F",
            "$-i",
            "$-I",
            "$-l",
            "$-p",
            "$-v",
            "$-w",
            "$CLASSPATH",
            "$JRUBY_VERSION",
            "$JRUBY_REVISION",
            "$ENV_JAVA",
            "$1",
            "$123",
        ] {
            let source = format!("puts {}", name);
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn interpolation_defined_rescue_and_aliases_are_real_reads_or_writes() {
        assert_eq!(
            offenses("\"#$custom #{$other}\"; defined?($custom); begin; rescue => $error; end")
                .len(),
            4
        );
        let found = offenses("alias $other $custom");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 7), (1, 14)]
        );
        assert!(offenses("# $custom\n'$custom'\n@custom\ncustom = 1\n").is_empty());
    }
}
