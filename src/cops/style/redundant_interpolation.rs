//! `Style/RedundantInterpolation` — detection only (upstream fixes are unsafe).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantInterpolation;
impl Cop for RedundantInterpolation {
    fn name(&self) -> &'static str {
        "Style/RedundantInterpolation"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(s) = node.as_interpolated_string_node() else {
            return;
        };
        if cx.parent().is_some_and(|p| {
            p.as_interpolated_string_node().is_some()
                || p.as_array_node().is_some_and(|a| {
                    a.opening_loc()
                        .is_some_and(|l| l.as_slice().starts_with(b"%"))
                })
        }) {
            return;
        }
        // Prism retains physical line continuations as empty StringNode parts;
        // the pinned parser normalizes them away. Keep real content and implicit
        // concatenations (nested strings) distinct from these empty fragments.
        let mut parts = s.parts().iter().filter(|part| {
            !part.as_string_node().is_some_and(|s| {
                let mut raw = s.content_loc().as_slice();
                if raw.is_empty() || !s.unescaped().is_empty() {
                    return false;
                }
                while !raw.is_empty() {
                    if let Some(rest) = raw
                        .strip_prefix(b"\\\r\n")
                        .or_else(|| raw.strip_prefix(b"\\\n"))
                    {
                        raw = rest;
                    } else {
                        return false;
                    }
                }
                true
            })
        });
        let Some(part) = parts.next() else {
            return;
        };
        if parts.next().is_some() {
            return;
        }
        if let Some(e) = part.as_embedded_statements_node() {
            if e.statements().is_some_and(|s| {
                s.body()
                    .iter()
                    .any(|n| n.as_match_required_node().is_some())
            }) {
                return;
            }
        } else if part.as_embedded_variable_node().is_none() {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Prefer `to_s` over string interpolation.".into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! RuboCop redundant_interpolation_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    #[test]
    fn pinned_offenses_are_detection_only() {
        for expr in [
            "1 + 1",
            "1 + 1; 2 + 2",
            "@var",
            "@@var",
            "$var",
            "$1",
            "$+",
            "number",
            "do_something(42)",
            "do_something 42",
            "foo.do_something 42",
            "42 in var",
        ] {
            for (open, close) in [("\"", "\""), ("%|", "|"), ("%Q(", ")")] {
                let src = format!("{open}#{{{expr}}}{close}");
                let o: Vec<_> = lint_str(&src)
                    .into_iter()
                    .filter(|o| o.cop == RedundantInterpolation.name())
                    .collect();
                assert_eq!(o.len(), 1, "{src}");
                assert_eq!((o[0].line, o[0].col), (1, 1));
                assert_eq!(o[0].message, "Prefer `to_s` over string interpolation.");
                assert!(
                    crate::cops::testing::correction_offenses(&RedundantInterpolation, &src)[0]
                        .correction
                        .is_none()
                );
            }
        }
        for src in [
            r##""#@var""##,
            r##""#@@var""##,
            r##""#$var""##,
            r##""#$1""##,
            r##""#$+""##,
            r##"var = 1; "#{var}""##,
            r##"["#{@var}", 'foo']"##,
        ] {
            assert_eq!(
                lint_str(src)
                    .iter()
                    .filter(|o| o.cop == RedundantInterpolation.name())
                    .count(),
                1,
                "{src}"
            );
        }
    }
    #[test]
    fn continuation_only_fragments_are_not_string_content() {
        for newline in ["\n", "\r\n"] {
            for (open, close) in [("\"", "\""), ("%Q(", ")")] {
                for (leading, trailing) in [(true, false), (false, true), (true, true)] {
                    let continuation = format!("\\{newline}");
                    let src = format!(
                        "  {open}{}#{{x}}{}{close}",
                        if leading { &continuation } else { "" },
                        if trailing { &continuation } else { "" },
                    );
                    let o: Vec<_> = lint_str(&src)
                        .into_iter()
                        .filter(|o| o.cop == RedundantInterpolation.name())
                        .collect();
                    assert_eq!(o.len(), 1, "{src:?}");
                    assert_eq!((o[0].line, o[0].col), (1, 3), "{src:?}");
                    assert_eq!(o[0].message, "Prefer `to_s` over string interpolation.");
                    assert!(o[0].correction.is_none());
                }
            }
        }
        for src in [
            "\"#{x}\\\n \"",
            "\" \\\n#{x}\"",
            "\"#{x}\\\n\" 'tail'",
            "'head' \"\\\n#{x}\"",
            "%W[#{x}\\\n]",
            "\"#{x}\\\n#{y}\"",
            "\"#{x}\n\"",
        ] {
            assert!(
                lint_str(src)
                    .iter()
                    .all(|o| o.cop != RedundantInterpolation.name()),
                "{src:?}"
            );
        }
    }
    #[test]
    fn pinned_and_semantic_lookalikes() {
        for src in [
            r##""#{42 => var}""##,
            r##""#{x; 42 => var}""##,
            r##""this is #{@sparta}""##,
            r##""#{@sparta} this is""##,
            r##""#{sparta}" ' this is'"##,
            r##"'this is ' "#{sparta}""##,
            "%W(#{@var} foo)",
            "%I(#{@var})",
            ":\"#{x}\"",
            "/#{x}/",
            "`#{x}`",
            "'#{x}'",
            "\"#{x}#{y}\"",
        ] {
            assert!(
                lint_str(src)
                    .iter()
                    .all(|o| o.cop != RedundantInterpolation.name()),
                "{src}"
            );
        }
    }
}
