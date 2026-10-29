//! `Style/BarePercentLiterals` — fixed `bare_percent` spelling.

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct BarePercentLiterals;

impl Cop for BarePercentLiterals {
    fn name(&self) -> &'static str {
        "Style/BarePercentLiterals"
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
        let opening = if let Some(n) = node.as_string_node() {
            n.opening_loc()
        } else if let Some(n) = node.as_interpolated_string_node() {
            n.opening_loc()
        } else {
            None
        };
        let Some(opening) = opening.filter(|loc| loc.as_slice().starts_with(b"%Q")) else {
            return;
        };
        let start = opening.start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `%` instead of `%Q`.".into(),
            line,
            col,
            end_line: file.line_of(opening.end_offset().saturating_sub(1)),
            correction: cx
                .corrections_requested()
                .then(|| {
                    // `%=` is assignment; `% hi ` in a command argument becomes
                    // modulo instead of a string. Withhold ambiguous delimiters
                    // even in expression-begin contexts that could accept them.
                    // Only remove ASCII Q, never transcode the literal payload.
                    if opening
                        .as_slice()
                        .get(2)
                        .is_some_and(|&b| b == b'=' || super::is_ruby_whitespace(b))
                    {
                        return None;
                    }
                    let root = cx.ancestors().next_back()?;
                    let proof = file.class_method_bindings(root);
                    if proof.observes_sources || proof.observes_data {
                        return None;
                    }
                    Some(Correction {
                        range: start + 1..start + 2,
                        replacement: String::new(),
                    })
                })
                .flatten(),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/bare_percent_literals_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};

    #[test]
    fn ambiguous_assignment_and_whitespace_delimiters_are_reported_without_edits() {
        for source in [
            "foo %Q=hi=",
            "value = %Q=hi=",
            "%Q=hi=",
            "foo %Q hi ",
            "foo %Q\thi\t",
            "foo %Q\nhi\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&BarePercentLiterals, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for (before, after) in [
            ("foo %Q_hi_", "foo %_hi_"),
            ("foo %Q!hi!", "foo %!hi!"),
            (r"%Q\hi\", r"%\hi\"),
        ] {
            assert_correction(&BarePercentLiterals, before, after);
        }
        // Every other ASCII punctuation delimiter retains command-argument
        // binding, not merely parser success (whitespace/= are excluded above).
        for open in (b'!'..=b'~').filter(|b| b.is_ascii_punctuation() && *b != b'=') {
            let close = match open {
                b'(' => b')',
                b'[' => b']',
                b'{' => b'}',
                b'<' => b'>',
                _ => open,
            };
            let (open, close) = (char::from(open), char::from(close));
            let before = format!("foo %Q{open}hi{close}");
            let after = format!("foo %{open}hi{close}");
            assert_correction(&BarePercentLiterals, &before, &after);
            let parsed = ruby_prism::parse(after.as_bytes());
            let program = parsed.node();
            let call = program
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .iter()
                .next()
                .unwrap();
            let call = call.as_call_node().unwrap();
            assert_eq!(call.name().as_slice(), b"foo", "{after}");
            let arg = call.arguments().unwrap().arguments().iter().next().unwrap();
            assert_eq!(arg.as_string_node().unwrap().unescaped(), b"hi", "{after}");
        }
    }

    #[test]
    fn pinned_static_and_dynamic_diagnostics_and_corrections() {
        for (before, after) in [("%Q(hi)", "%(hi)"), ("%Q(#{x})", "%(#{x})")] {
            let source = format!("# heading\n  {before}\n");
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == "Style/BarePercentLiterals")
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(found[0].message, "Use `%` instead of `%Q`.");
            assert_correction(
                &BarePercentLiterals,
                &format!("{before}\n"),
                &format!("{after}\n"),
            );
        }
    }

    #[test]
    fn pinned_lookalikes_and_literal_children_are_not_reported() {
        for source in [
            "__FILE__",
            "/%Q?/",
            "\"\"",
            "\"#{file}hi\"",
            "'hi'",
            "%q(hi)",
            "%(hi)",
            "%(#{x})",
            "func <<HEREDOC\nhi\nHEREDOC\n",
            "%w[%Q(hi)]",
            "%s[%Q(hi)]",
            "%x[command]",
            "%Q(a); %Q(b)",
        ] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/BarePercentLiterals")
                .collect();
            let expected = if source == "%Q(a); %Q(b)" { 2 } else { 0 };
            assert_eq!(found.len(), expected, "{source}");
        }
    }
}
