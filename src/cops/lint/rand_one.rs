//! `Lint/RandOne` — literal unit limits on bare or Kernel.rand calls.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct RandOne;
impl Cop for RandOne {
    fn name(&self) -> &'static str {
        "Lint/RandOne"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"rand"
            || call.is_safe_navigation()
            || call
                .receiver()
                .is_some_and(|r| !ast::is_global_const(&r, &["Kernel"]))
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let [arg] = args.as_slice() else { return };
        let number = ast::negated_numeric_literal(arg);
        let number = number.as_ref().unwrap_or(arg);
        let unit = if let Some(int) = number.as_integer_node() {
            int.value().to_u32_digits().1 == [1]
        } else {
            number
                .as_float_node()
                .is_some_and(|n| n.value().abs() == 1.0)
        };
        if !unit {
            return;
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        // Project the reference buffer, not the cooked literal or lossy original
        // bytes: declared legacy encodings, UTF8-MAC composition and CRLF matter.
        let projected = file.match_pattern_source(&file.bytes[start..end]);
        let method = match std::str::from_utf8(&projected) {
            Ok(text) => text.to_owned(),
            // Binary source can contain non-UTF8 bytes in comments. Preserve
            // them explicitly in the UTF8 diagnostic rather than replacing data.
            Err(_) => projected
                .utf8_chunks()
                .map(|chunk| {
                    let mut text = chunk.valid().to_owned();
                    for b in chunk.invalid() {
                        text.push_str(&format!("\\x{b:02X}"));
                    }
                    text
                })
                .collect::<String>(),
        }
        .replace("\r\n", "\n");
        let (line, col) = file.reference_line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "`{method}` always returns `0`. Perhaps you meant `rand(2)` or `rand`?"
            ),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/rand_one_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RandOne")
            .collect()
    }
    #[test]
    fn pinned_numeric_and_kernel_calls() {
        for call in [
            "rand 1",
            "rand(-1)",
            "rand(1.0)",
            "rand(-1.0)",
            "Kernel.rand(1)",
            "Kernel.rand(-1)",
            "Kernel.rand 1.0",
            "Kernel.rand(-1.0)",
            "::Kernel.rand(1)",
        ] {
            let source = format!("# random\n  {call}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            let o = &found[0];
            assert_eq!((o.line, o.col, o.end_line), (2, 3, 2));
            assert_eq!(
                o.message,
                format!("`{call}` always returns `0`. Perhaps you meant `rand(2)` or `rand`?")
            );
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn numeric_values_are_literal_not_constant_folded_or_type_inferred() {
        for literal in [
            "0x1",
            "0b1",
            "0o1",
            "01",
            "0d1",
            "+1",
            "-0x1",
            "1e0",
            "0.1e1",
            "1.00000000000000001",
            "--1",
            "-+1",
        ] {
            assert_eq!(offenses(&format!("rand({literal})")).len(), 1, "{literal}");
        }
        for arg in [
            "(1)", "---1", "-(1)", "1.-@", "1r", "1i", "1 + 0", "*[1]", "1, &b",
        ] {
            assert!(offenses(&format!("rand({arg})")).is_empty(), "{arg}");
        }
        for source in [
            "self.rand(1)",
            "(Kernel).rand(1)",
            "Other::Kernel.rand(1)",
            "Kernel&.rand(1)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn messages_project_source_and_literal_blocks_are_not_part_of_send() {
        let source = "# coding: UTF8-MAC\r\n'e\u{301}'; rand(\r\n # e\u{301}\r\n 1\r\n) do\r\n body\r\nend\r\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 6, 5));
        assert_eq!(
            found[0].message,
            "`rand(\n # é\n 1\n)` always returns `0`. Perhaps you meant `rand(2)` or `rand`?"
        );
    }
    #[test]
    fn pinned_no_offense_examples() {
        for source in [
            "rand",
            "rand(2)",
            "rand(-1..1)",
            "Kernel.rand",
            "Kernel.rand 2",
            "Kernel.rand(-1..1)",
            "::Kernel.rand",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
