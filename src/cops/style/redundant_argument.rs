//! `Style/RedundantArgument`: fixed default values; detection only (unsafe upstream).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantArgument;
impl Cop for RedundantArgument {
    fn name(&self) -> &'static str {
        "Style/RedundantArgument"
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
        let name = call.name().as_slice();
        if call.receiver().is_none() && !matches!(name, b"exit" | b"exit!") {
            return;
        }
        if call
            .block()
            .is_some_and(|b| b.as_block_argument_node().is_some())
        {
            return;
        }
        let Some(args) = call.arguments() else { return };
        if args.arguments().len() != 1 {
            return;
        }
        let arg = args.arguments().first().unwrap();
        let matched = match name {
            b"join" => string_value(&arg, b""),
            b"split" => string_value(&arg, b" "),
            b"chomp" | b"chomp!" => string_value(&arg, b"\n"),
            b"sum" => integer_value(&arg, 0),
            b"to_i" => integer_value(&arg, 10),
            b"exit" => arg.as_true_node().is_some(),
            b"exit!" => arg.as_false_node().is_some(),
            _ => false,
        };
        if !matched {
            return;
        }
        let start = call
            .opening_loc()
            .map(|l| l.start_offset())
            .unwrap_or_else(|| {
                let mut start = arg.location().start_offset();
                while start > 0 && matches!(file.bytes[start - 1], b' ' | b'\t' | b'\r') {
                    start -= 1;
                }
                start
            });
        let end = call
            .closing_loc()
            .map(|l| l.end_offset())
            .unwrap_or(arg.location().end_offset());
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!(
                "Argument {} is redundant because it is implied by default.",
                String::from_utf8_lossy(arg.location().as_slice())
            ),
            correction: None,
        });
    }
}
fn integer_value(node: &Node<'_>, expected: u32) -> bool {
    node.as_integer_node().is_some_and(|i| {
        let value = i.value();
        let (negative, digits) = value.to_u32_digits();
        if expected == 0 {
            digits.iter().all(|d| *d == 0)
        } else {
            !negative && digits == [expected]
        }
    })
}
fn string_value(node: &Node<'_>, expected: &[u8]) -> bool {
    node.as_string_node()
        .is_some_and(|s| s.unescaped() == expected)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/redundant_argument_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantArgument")
            .collect()
    }
    #[test]
    fn every_matching_pinned_unsafe_correction_pair_is_withheld() {
        let pairs: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/argument_wrappers_corrections.json"
        ))
        .unwrap();
        let pairs: Vec<_> = pairs
            .iter()
            .filter(|p| p["cop"] == "Style/RedundantArgument")
            .collect();
        assert_eq!(pairs.len(), 6);
        for p in pairs {
            let before = p["before"].as_str().unwrap();
            assert!(!offenses(before).is_empty());
            assert!(
                crate::cops::testing::correction_offenses(&RedundantArgument, before)
                    .iter()
                    .all(|o| o.correction.is_none())
            );
            assert!(offenses(p["after"].as_str().unwrap()).is_empty());
        }
    }
    #[test]
    fn default_values_positions_messages_and_no_fixes() {
        for (method, arg) in [
            ("join", "''"),
            ("sum", "0"),
            ("exit", "true"),
            ("exit!", "false"),
            ("to_i", "10"),
            ("split", "' '"),
            ("chomp", r#""\n""#),
            ("chomp!", r#""\n""#),
        ] {
            for receiver in ["foo.", "foo&."] {
                for parens in [true, false] {
                    let call = if parens {
                        format!("{receiver}{method}({arg})")
                    } else {
                        format!("{receiver}{method} {arg}")
                    };
                    let source = format!("# test\n  {call}\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col),
                        (2, (3 + receiver.len() + method.len()) as u32)
                    );
                    assert_eq!(
                        found[0].message,
                        format!("Argument {arg} is redundant because it is implied by default.")
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
        for source in [
            "exit(true)",
            "exit!(false)",
            "[1,2,3].join('')",
            r#""first second".split(" ")"#,
            r#"foo.join("\x00"[0,0])"#,
        ] {
            if source.contains("[0,0]") {
                assert!(offenses(source).is_empty());
            } else {
                assert_eq!(offenses(source).len(), 1);
            }
        }
    }
    #[test]
    fn negative_zero_is_the_default_sum_argument() {
        for arg in ["-0", "-0x0", "-0b0", "-0o0", "-0d0", "-0_0"] {
            let source = format!("foo.sum({arg})");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 8));
            assert_eq!(found[0].end_line, 1);
            assert_eq!(
                found[0].message,
                format!("Argument {arg} is redundant because it is implied by default.")
            );
            assert!(found[0].correction.is_none());
        }
        for source in ["foo.sum(-1)", "foo.sum(-0.0)", "foo.to_i(-10)"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn decoded_literals_and_negative_lookalikes() {
        for source in [
            r#"foo.join("\x00")"#,
            r#"foo.chomp('\n')"#,
            r#"foo.chomp("\x82")"#,
            "foo.join",
            "foo.join('', 2)",
            "foo.join(',', &b)",
            "foo.sum(42)",
            "foo.split(',')",
            "join('')",
            "split(' ')",
            "foo.join(separator)",
            "foo.sum(0.0)",
            "foo.to_i(*[10])",
            "foo.join(\"#{value}\")",
            "super(0)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "foo.join(%q())",
            "foo.sum(0x0)",
            "foo.to_i(0xa)",
            r#"foo.split("\x20")"#,
            r#"foo.chomp("\x0a")"#,
            "foo.sum(0) { |x| x }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        assert!(offenses("foo.sum(0, &b)").is_empty());
    }
}
