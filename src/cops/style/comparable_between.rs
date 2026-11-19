//! `Style/ComparableBetween` — inclusive logical comparisons; unsafe, detection only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ComparableBetween;
impl Cop for ComparableBetween {
    fn name(&self) -> &'static str {
        "Style/ComparableBetween"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_and_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(and) = node.as_and_node() else {
            return;
        };
        let (Some((greater1, less1)), Some((greater2, less2))) =
            (comparison(and.left()), comparison(and.right()))
        else {
            return;
        };
        let eq = |a: &Node<'_>, b: &Node<'_>| {
            ast::fingerprint_for_comparison(a, file) == ast::fingerprint_for_comparison(b, file)
        };
        // Prefer the min-first match, just as the pinned matcher does.
        let (value, min, max) = if eq(&greater1, &less2) {
            (&greater1, &less1, &greater2)
        } else if eq(&less1, &greater2) {
            (&greater2, &less2, &greater1)
        } else {
            return;
        };
        let loc = node.location();
        let text = |n: &Node<'_>| String::from_utf8_lossy(n.location().as_slice()).into_owned();
        let correction = cx
            .corrections_requested()
            .then(|| crate::offense::Correction {
                range: loc.start_offset()..loc.end_offset(),
                replacement: format!("{}.between?({}, {})", text(value), text(min), text(max)),
            });
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: format!(
                "Prefer `{}.between?({}, {})` over logical comparison.",
                single_line(value.location().as_slice()),
                single_line(min.location().as_slice()),
                single_line(max.location().as_slice())
            ),
            correction,
        });
    }
}
fn comparison(node: Node<'_>) -> Option<(Node<'_>, Node<'_>)> {
    let c = node.as_call_node()?;
    if c.is_safe_navigation() || c.block().is_some() {
        return None;
    }
    let lhs = c.receiver()?;
    let args = ast::call_arguments(&c);
    if args.len() != 1 {
        return None;
    }
    let rhs = args.into_iter().next()?;
    match c.name().as_slice() {
        b">=" => Some((lhs, rhs)),
        b"<=" => Some((rhs, lhs)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/comparable_between_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == ComparableBetween.name())
            .collect()
    }
    #[test]
    fn all_eight_orders_and_keyword_and() {
        for s in [
            "x >= min && x <= max",
            "x >= min && max >= x",
            "min <= x && x <= max",
            "min <= x && max >= x",
            "x <= max && x >= min",
            "x <= max && min <= x",
            "max >= x && x >= min",
            "max >= x && min <= x",
            "x >= min and x <= max",
        ] {
            let s = format!("\n  {s}");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (2, 3));
            assert_eq!(
                o[0].message,
                "Prefer `x.between?(min, max)` over logical comparison."
            );
            assert!(
                crate::cops::testing::correction_offenses(&ComparableBetween, &s)[0]
                    .correction
                    .is_some()
            );
        }
    }
    #[test]
    fn excludes_strict_different_subjects_grouped_comparisons_and_custom_block_calls() {
        for s in [
            "x > min && x <= max",
            "x >= min && x < max",
            "x > min && x < max",
            "x >= min && y <= max",
            "x >= min || x <= max",
            "(x >= min) && x <= max",
            "x >= min && (x <= max)",
            "x&.>=(min) && x <= max",
            "x.>=(min) { foo } && x <= max",
            "obj.x(1) >= min && obj.x(2) <= max",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn numeric_zero_subject_equality() {
        for (source, value) in [
            ("0 >= min && -0 <= max", "0"),
            ("0.0 >= min && -0.0 <= max", "0.0"),
            ("-0 >= min && 0 <= max", "-0"),
            ("-0.0 >= min && 0.0 <= max", "-0.0"),
            ("0.0 <= max && -0.0 >= min", "-0.0"),
            ("obj.f(0) >= min && obj.f(-0) <= max", "obj.f(0)"),
            ("obj.f(0.0) >= min && obj.f(-0.0) <= max", "obj.f(0.0)"),
        ] {
            let o = offenses(source);
            assert_eq!(o.len(), 1, "{source}");
            assert_eq!((o[0].line, o[0].col), (1, 1));
            assert_eq!(
                o[0].message,
                format!("Prefer `{value}.between?(min, max)` over logical comparison.")
            );
            assert!(o[0].correction.is_none());
        }
        for source in [
            "0 >= min && 0.0 <= max",
            "1 >= min && -1 <= max",
            "1.0 >= min && -1.0 <= max",
            "obj.f(0) >= min && obj.f(-0.0) <= max",
            "__LINE__ >= min &&\n __LINE__ <= max",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn structural_subjects_self_bounds_and_evaluated_line_values() {
        for (s, prefer) in [
            ("x >= x && x <= max", "x.between?(x, max)"),
            ("x >= x && x <= x", "x.between?(x, x)"),
            (
                "obj.value >= min && obj.value <= max",
                "obj.value.between?(min, max)",
            ),
            ("x=1; x >= min && x <= max", "x.between?(min, max)"),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                o[0].message,
                format!("Prefer `{prefer}` over logical comparison.")
            );
        }
        assert!(offenses("__LINE__ >= min &&\n __LINE__ <= max").is_empty());
    }
}
