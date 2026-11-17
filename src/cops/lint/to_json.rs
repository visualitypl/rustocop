//! `Lint/ToJSON` — zero-formal JSON method definitions.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ToJSON;
impl Cop for ToJSON {
    fn name(&self) -> &'static str {
        "Lint/ToJSON"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        if def.name().as_slice() != b"to_json" || def.parameters().is_some() {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(node.location().end_offset().saturating_sub(1)),
            message:
                "`#to_json` requires an optional argument to be parsable via JSON.generate(obj)."
                    .into(),
            correction: cx.corrections_requested().then(|| match def.lparen_loc() {
                Some(open) => crate::offense::Correction {
                    range: open.end_offset()..open.end_offset(),
                    replacement: "*_args".into(),
                },
                None => {
                    let at = def.name_loc().end_offset();
                    crate::offense::Correction {
                        range: at..at,
                        replacement: "(*_args)".into(),
                    }
                }
            }),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop to_json_spec.rb (MIT, Copyright 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str_with_corrections(&[&ToJSON], s)
    }
    #[test]
    fn canonical_insertion_only_in_closed_inactive_envelope() {
        for signature in ["to_json", "to_json()", "self.to_json", "self.to_json()"] {
            let s = format!("if false\n  def {signature}\n  end\nend\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1);
            let fixed = crate::corrections::apply(s.as_bytes(), &o).unwrap();
            let name = signature.trim_end_matches("()");
            assert_eq!(
                fixed.bytes,
                format!("if false\n  def {name}(*_args)\n  end\nend\n").as_bytes()
            );
        }
    }
}
