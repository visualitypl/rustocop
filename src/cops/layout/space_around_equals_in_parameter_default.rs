//! `Layout/SpaceAroundEqualsInParameterDefault` — spaces around positional defaults.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceAroundEqualsInParameterDefault;

impl Cop for SpaceAroundEqualsInParameterDefault {
    fn name(&self) -> &'static str {
        "Layout/SpaceAroundEqualsInParameterDefault"
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
        let Some(parameter) = node.as_optional_parameter_node() else {
            return;
        };
        let start = parameter.name_loc().end_offset();
        let operator = parameter.operator_loc();
        let end = operator.end_offset();
        if file.bytes.get(start).is_some_and(u8::is_ascii_whitespace)
            && file.bytes.get(end).is_some_and(u8::is_ascii_whitespace)
        {
            return;
        }
        let (line, col) = file.line_col(start);
        let horizontal_before = file.bytes[start..operator.start_offset()]
            .iter()
            .all(|b| matches!(b, b' ' | b'\t' | 0x0b | 0x0c));
        let correction = (cx.corrections_requested() && horizontal_before).then(|| {
            let mut finish = end;
            while file
                .bytes
                .get(finish)
                .is_some_and(|b| matches!(b, b' ' | b'\t' | 0x0b | 0x0c))
            {
                finish += 1;
            }
            // Never collapse continuations, comments or line breaks.
            let suffix = if file
                .bytes
                .get(finish)
                .is_some_and(|b| matches!(b, b'\r' | b'\n'))
            {
                ""
            } else {
                " "
            };
            Correction {
                range: start..finish,
                replacement: format!(" ={suffix}"),
            }
        });
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: "Surrounding space missing in default value assignment.".into(),
            line,
            end_line: file.line_of(end - 1),
            col,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `space_around_equals_in_parameter_default_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == SpaceAroundEqualsInParameterDefault.name())
            .collect()
    }

    #[test]
    fn pinned_defaults_positions_messages_and_corrections() {
        for (before, after, columns) in [
            (
                "def f(x, y=0, z= 1)\nend",
                "def f(x, y = 0, z = 1)\nend",
                vec![11, 16],
            ),
            (
                "def f(x, y= 0, z=1)\nend",
                "def f(x, y = 0, z = 1)\nend",
                vec![11, 17],
            ),
            ("def f(x, y=\"\")\nend", "def f(x, y = \"\")\nend", vec![11]),
            ("def f(x, y=[])\nend", "def f(x, y = [])\nend", vec![11]),
            (
                "def f(x=-1, y= 0, z =+1)\nend",
                "def f(x = -1, y = 0, z = +1)\nend",
                vec![8, 14, 20],
            ),
        ] {
            let found = offenses(before);
            assert_eq!(
                found.iter().map(|o| o.col).collect::<Vec<_>>(),
                columns,
                "{before}"
            );
            for o in found {
                assert_eq!((o.line, o.end_line), (1, 1));
                assert_eq!(
                    o.message,
                    "Surrounding space missing in default value assignment."
                );
                assert_eq!(o.severity, crate::offense::Severity::Convention);
            }
            assert_correction(&SpaceAroundEqualsInParameterDefault, before, after);
            assert!(offenses(after).is_empty());
        }
    }

    #[test]
    fn block_lambda_singleton_and_unparenthesized_defaults() {
        for (before, after) in [
            ("def f x=1; end", "def f x = 1; end"),
            ("def self.f(x=1); end", "def self.f(x = 1); end"),
            ("f { |x=1; y| x }", "f { |x = 1; y| x }"),
            ("->(x=1) { x }", "->(x = 1) { x }"),
            ("lambda { |x=+1| x }", "lambda { |x = +1| x }"),
            (
                "def f(x=\n  __LINE__); x; end",
                "def f(x =\n  __LINE__); x; end",
            ),
            ("def f(x=# note\n  1); end", "def f(x = # note\n  1); end"),
            (
                "\u{feff}def f(é=1)\r\nend\r\n",
                "\u{feff}def f(é = 1)\r\nend\r\n",
            ),
        ] {
            assert!(!offenses(before).is_empty(), "{before}");
            assert_correction(&SpaceAroundEqualsInParameterDefault, before, after);
        }
    }

    #[test]
    fn reports_but_withholds_nonhorizontal_whitespace_before_equals() {
        for source in ["def f(x\\\n=1); end", "def f(x\\\n= 1); end"] {
            let found = crate::cops::testing::correction_offenses(
                &SpaceAroundEqualsInParameterDefault,
                source,
            );
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn accepts_spaces_keywords_literals_and_ordinary_assignments() {
        for source in [
            "",
            "def f(x, y = 0, z = {}); end",
            "def f(x, y = +1); end",
            "def f(x  =  1, y: 2, **kw); end",
            "f(x: 1)",
            "x=1",
            "'def f(x=1); end'",
            "# def f(x=1); end\n",
            "__END__\ndef f(x=1); end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
