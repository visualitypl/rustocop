//! `Style/StderrPuts` — prefer suppressible warning output.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct StderrPuts;
impl Cop for StderrPuts {
    fn name(&self) -> &'static str {
        "Style/StderrPuts"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation()
            || call.name().as_slice() != b"puts"
            || ast::call_arguments(&call).is_empty()
        {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        if !ast::is_global_const(&receiver, &["STDERR"])
            && !receiver
                .as_global_variable_read_node()
                .is_some_and(|g| g.name().as_slice() == b"$stderr")
        {
            return;
        }
        let start = call.location().start_offset();
        let end = call.message_loc().unwrap().end_offset();
        let (line, col) = file.line_col(start);
        let bad = format!(
            "{}.puts",
            String::from_utf8_lossy(receiver.location().as_slice())
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `warn` instead of `{bad}` to allow such output to be disabled."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: cx.corrections_requested().then(|| Correction {
                range: start..end,
                replacement: "warn".into(),
            }),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/stderr_puts_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn byte_local_replacements_preserve_bom_crlf_encoding_data_and_line_values() {
        let before = "\u{feff}# encoding: UTF-8\r\n$VERBOSE = false\r\n$stderr.puts(\"café\\n\") # retained\r\np __LINE__\r\n__END__\r\n$stderr.puts('raw')\r\n";
        let after = before.replacen("$stderr.puts", "warn", 1);
        assert_correction(&StderrPuts, before, &after);
        assert_correction(
            &StderrPuts,
            "$VERBOSE = false; STDERR::puts('x')",
            "$VERBOSE = false; warn('x')",
        );
    }
    #[test]
    fn pinned_no_argument_calls_and_receiver_lookalikes() {
        for source in [
            "$stderr.puts",
            "STDERR.puts",
            "::STDERR.puts()",
            "$stdout.puts('x')",
            "$stdin.puts('x')",
            "Other::STDERR.puts('x')",
            "stderr.puts('x')",
            "$stderr&.puts('x')",
            "$stderr.print('x')",
            "puts('x')",
        ] {
            assert!(
                correction_offenses(&StderrPuts, source).is_empty(),
                "{source}"
            );
        }
    }
}
