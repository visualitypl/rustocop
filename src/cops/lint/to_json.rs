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
            correction: cx
                .corrections_requested()
                .then(|| inactive_signature(node, cx, file))
                .flatten(),
        });
    }
}
// A live signature edit changes arity, binding, reflection, method hooks,
// dispatch, exceptions and compiler/code-object observations. Only an empty
// ASCII definition in the sole closed top-level `if false` body is proved:
// Ruby creates neither method nor argument binding there. No defaults/body,
// receiver evaluation, line movement or warning changes can be introduced.
// Shared main-parse source/encoding/position/code-object/heap guards still apply.
fn inactive_signature(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let def = node.as_def_node()?;
    let raw = node.location().as_slice();
    let normalized: Vec<_> = raw
        .iter()
        .enumerate()
        .filter_map(|(i, b)| (*b != b'\r' || raw.get(i + 1) != Some(&b'\n')).then_some(*b))
        .collect();
    let text = std::str::from_utf8(&normalized).ok()?;
    // Whitespace indentation is inert, but arbitrary bodies/comments/heredocs
    // are not accepted by this canonical source capability.
    let (header, tail) = text.split_once('\n')?;
    if !matches!(
        header,
        "def to_json" | "def to_json()" | "def self.to_json" | "def self.to_json()"
    ) || tail.trim_start_matches([' ', '\t']) != "end"
    {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let body = ancestors.next()?.as_statements_node()?;
    let conditional = ancestors.next()?.as_if_node()?;
    let top = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some()
        || body.body().len() != 1
        || top.body().len() != 1
        || conditional.predicate().as_false_node().is_none()
        || conditional.subsequent().is_some()
        || conditional.end_keyword_loc().is_none()
        || conditional.if_keyword_loc()?.as_slice() != b"if"
    {
        return None;
    }
    let (offset, replacement) = if let Some(open) = def.lparen_loc() {
        (open.end_offset(), "*_args")
    } else {
        (def.name_loc().end_offset(), "(*_args)")
    };
    Some(crate::offense::Correction {
        range: offset..offset,
        replacement: replacement.into(),
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop to_json_spec.rb (MIT, Copyright 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str_with_corrections(&[&ToJSON], s)
    }
    #[test]
    fn zero_formals_and_full_method_ranges() {
        for signature in ["to_json", "to_json()", "self.to_json", "object.to_json()"] {
            let s = format!("def {signature}\n  nil\nend\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 1, 3));
            assert_eq!(
                o[0].message,
                "`#to_json` requires an optional argument to be parsable via JSON.generate(obj)."
            );
            assert!(o[0].correction.is_none());
        }
        for p in ["*_args", "x", "x=nil", "x:", "**", "&b", "..."] {
            assert!(offenses(&format!("def to_json({p}); end")).is_empty());
        }
        assert!(offenses("def not_to_json; end").is_empty());
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
