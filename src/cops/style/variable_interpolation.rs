//! `Style/VariableInterpolation` — brace direct interpolated variables.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct VariableInterpolation;
impl Cop for VariableInterpolation {
    fn name(&self) -> &'static str {
        "Style/VariableInterpolation"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_embedded_variable_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Prism identifies the direct #@/#$ syntax explicitly, so no source
        // search or enclosing-literal walk can confuse escaped hashes or #{x}.
        let Some(embedded) = node.as_embedded_variable_node() else {
            return;
        };
        let variable = embedded.variable();
        let loc = variable.location();
        let name = String::from_utf8_lossy(loc.as_slice());
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Replace interpolated variable `{name}` with expression `#{{{name}}}`."
            ),
            line,
            col,
            end_line: file.last_line_of(variable.location()),
            correction: cx
                .corrections_requested()
                .then(|| {
                    super::literal_spelling::safe_edit(
                        file,
                        cx,
                        Correction {
                            range: loc.start_offset()..loc.end_offset(),
                            replacement: format!(
                                "{{{}}}",
                                std::str::from_utf8(loc.as_slice()).ok()?
                            ),
                        },
                    )
                })
                .flatten(),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/variable_interpolation_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn all_eight_pinned_correction_pairs_and_messages() {
        for (before, after, var, col) in [
            (
                r##"puts "this is a #$test""##,
                r##"puts "this is a #{$test}""##,
                "$test",
                18,
            ),
            (
                r"puts /this is a #$test/",
                r"puts /this is a #{$test}/",
                "$test",
                18,
            ),
            (
                r"puts `this is a #$test`",
                r"puts `this is a #{$test}`",
                "$test",
                18,
            ),
            (
                r##"puts :"this is a #$test""##,
                r##"puts :"this is a #{$test}""##,
                "$test",
                19,
            ),
            (
                r##"puts "this is a #$1""##,
                r##"puts "this is a #{$1}""##,
                "$1",
                18,
            ),
            (
                r##"puts "this is a #$+""##,
                r##"puts "this is a #{$+}""##,
                "$+",
                18,
            ),
            (
                r##"puts "this is a #@test""##,
                r##"puts "this is a #{@test}""##,
                "@test",
                18,
            ),
            (
                r##"puts "this is a #@@t""##,
                r##"puts "this is a #{@@t}""##,
                "@@t",
                18,
            ),
        ] {
            let found: Vec<_> = lint_str(before)
                .into_iter()
                .filter(|o| o.cop == VariableInterpolation.name())
                .collect();
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1),
                "{before}"
            );
            assert_eq!(
                found[0].message,
                format!("Replace interpolated variable `{var}` with expression `#{{{var}}}`.")
            );
            assert_correction(&VariableInterpolation, before, after);
        }
    }
    #[test]
    fn braced_escaped_and_noninterpolating_lookalikes() {
        for source in [
            r##"puts "this is a #{@test} #{@@t} #{$t} #{$1} #{$+}""##,
            r##""\#@test \#$test""##,
            "'#@test #$test'",
            "%q[#@test]",
            "%w[#$test]",
            "%i[#@test]",
            "%s[#$test]",
            "@test",
            "p <<~'X'\n#@test\nX\n",
            "# #@test\n",
            r##""#{"#{@test}"}""##,
        ] {
            assert!(
                lint_str(source)
                    .iter()
                    .all(|o| o.cop != VariableInterpolation.name()),
                "{source}"
            );
        }
    }
    #[test]
    fn special_globals_and_references_use_exact_main_parse_token_ranges() {
        for var in [
            "$1",
            "$9",
            "$+",
            "$&",
            "$`",
            "$'",
            "$~",
            "$$",
            "$?",
            "$!",
            "$=",
            "$:",
            "$;",
            "$/",
            "$\\",
            "$>",
            "$<",
            "$.",
            "$_",
            "$-w",
            "$LOAD_PATH",
            "@café",
            "@@café",
        ] {
            // The source delimiter must not be a special global's name.
            let before = format!("%Q!#{var}!");
            if var == "$!" {
                continue;
            }
            let after = format!("%Q!#{{{var}}}!");
            assert_correction(&VariableInterpolation, &before, &after);
        }
        assert_correction(&VariableInterpolation, "%Q(#$!)", "%Q(#{$!})");
        assert_correction(&VariableInterpolation, "%Q(#$\")", "%Q(#{$\"})");
    }
    #[test]
    fn nested_word_symbol_regexp_match_and_heredoc_contexts() {
        for (before, after) in [
            ("%W[a#@x]", "%W[a#{@x}]"),
            ("%I[a#$x]", "%I[a#{$x}]"),
            ("p <<~X\n  #@test\nX\n", "p <<~X\n  #{@test}\nX\n"),
            ("if /#$x/o; end", "if /#{$x}/o; end"),
            (r##""#{"#@x"}#$y""##, r##""#{"#{@x}"}#{$y}""##),
            (r##""\\#@x""##, r##""\\#{@x}""##),
            ("%x[echo #@x]", "%x[echo #{@x}]"),
        ] {
            assert_correction(&VariableInterpolation, before, after);
        }
    }
}
