//! `Style/RandomWithOffset` — prefer explicit integer ranges.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct RandomWithOffset;
impl Cop for RandomWithOffset {
    fn name(&self) -> &'static str {
        "Style/RandomWithOffset"
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
        let Some(matched) = offensive(node) else {
            return;
        };
        let correction = cx
            .corrections_requested()
            .then(|| {
                let text = matched.translation()?;
                super::file_api::replacement(node, cx, file, text, "Random")
            })
            .flatten();
        let start = node.location().start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message:
                "Prefer ranges when generating random numbers instead of integers with offsets."
                    .into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1).max(start)),
            correction,
        });
    }
}

struct Match<'pr> {
    random: CallNode<'pr>,
    bound: Node<'pr>,
    offset: Option<Node<'pr>>,
    subtract: bool,
    reflection: bool,
}

fn offensive<'pr>(node: &Node<'pr>) -> Option<Match<'pr>> {
    let call = node.as_call_node()?;
    if call.is_safe_navigation() {
        return None;
    }
    let receiver = call.receiver()?;
    let args = ast::call_arguments(&call);
    let name = call.name().as_slice();
    let (random, offset, reflection) = match (name, args.as_slice()) {
        (b"+" | b"-", [offset]) if offset.as_integer_node().is_some() => {
            (receiver, Some(offset.as_integer_node()?.as_node()), false)
        }
        (b"+" | b"-", [random]) if receiver.as_integer_node().is_some() => (
            random.as_call_node()?.as_node(),
            Some(receiver),
            name == b"-",
        ),
        (b"succ" | b"pred" | b"next", []) => (receiver, None, false),
        _ => return None,
    };
    let random = super::file_api::send(&random)?;
    if random.name().as_slice() != b"rand"
        || random.receiver().is_some_and(|r| {
            !ast::is_global_const(&r, &["Kernel"]) && !ast::is_global_const(&r, &["Random"])
        })
    {
        return None;
    }
    let bound = super::file_api::argument(&random)?;
    if bound.as_integer_node().is_none()
        && !bound.as_range_node().is_some_and(|r| {
            r.left().is_some_and(|n| n.as_integer_node().is_some())
                && r.right().is_some_and(|n| n.as_integer_node().is_some())
        })
    {
        return None;
    }
    Some(Match {
        random,
        bound,
        offset,
        subtract: matches!(name, b"-" | b"pred"),
        reflection,
    })
}

// Correction arithmetic is deliberately bounded, not detection. Ruby's negative
// and zero rand bounds have different types/error behavior; huge integers can
// select different RNG algorithms. Restrict safe translations to Fixnum-sized
// positive widths and endpoints, retaining every other finding without a fix.
const FIXNUM_MAX: i128 = (1_i128 << 62) - 1;
fn is_fixnum(value: i128) -> bool {
    (-FIXNUM_MAX - 1..=FIXNUM_MAX).contains(&value)
}
fn integer(node: &Node<'_>) -> Option<i128> {
    let value = node.as_integer_node()?.value();
    let (negative, digits) = value.to_u32_digits();
    if digits.len() > 2 {
        return None;
    }
    let value = digits
        .iter()
        .rev()
        .fold(0_i128, |n, d| (n << 32) + i128::from(*d));
    Some(if negative { -value } else { value })
}
impl Match<'_> {
    fn translation(&self) -> Option<String> {
        // Reflecting the interval keeps its distribution, but reverses seeded
        // draws. Even upstream's canonical reflection examples are not safe.
        if self.reflection {
            return None;
        }
        let (left, right) = if let Some(range) = self.bound.as_range_node() {
            let left = integer(&range.left()?)?;
            let right = integer(&range.right()?)?;
            // Check the original exclusive endpoint too, before normalizing it.
            if !is_fixnum(left) || !is_fixnum(right) {
                return None;
            }
            (
                left,
                right - i128::from(range.operator_loc().as_slice() == b"..."),
            )
        } else {
            (0, integer(&self.bound)? - 1)
        };
        let width = right - left + 1;
        let offset = self.offset.as_ref().map_or(Some(1), integer)?;
        if !is_fixnum(offset) {
            return None;
        }
        let offset = if self.subtract { -offset } else { offset };
        let (new_left, new_right) = (left + offset, right + offset);
        if !(1..=FIXNUM_MAX).contains(&width)
            || [left, right, offset, new_left, new_right]
                .iter()
                .any(|n| !is_fixnum(*n))
        {
            return None;
        }
        let prefix = self
            .random
            .receiver()
            .map(|n| String::from_utf8(n.location().as_slice().to_vec()).ok())
            .unwrap_or(Some(String::new()))?;
        let dot = if prefix.is_empty() { "" } else { "." };
        Some(format!("{prefix}{dot}rand({new_left}..{new_right})"))
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/random_with_offset_spec.rb (MIT
    //! License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_offenses_and_translation_corrections() {
        for (before, after) in [
            ("rand(6) + 1", "rand(1..6)"),
            ("1 + rand(6)", "rand(1..6)"),
            ("rand(6).succ", "rand(1..6)"),
            ("rand(6) - 1", "rand(-1..4)"),
            ("rand(6).pred", "rand(-1..4)"),
            ("rand(6).next", "rand(1..6)"),
            ("rand(0..6) + 1", "rand(1..7)"),
            ("rand(0...6) + 1", "rand(1..6)"),
            ("rand(0..6) - 1", "rand(-1..5)"),
            ("rand(0...6) - 1", "rand(-1..4)"),
            ("rand(0..6).succ", "rand(1..7)"),
            ("rand(0...6).succ", "rand(1..6)"),
        ] {
            for prefix in ["", "Kernel.", "::Kernel.", "Random.", "::Random."] {
                let before = before.replace("rand", &format!("{prefix}rand"));
                let after = after.replace("rand", &format!("{prefix}rand"));
                let source = format!("# header\n  {before}\n");
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == RandomWithOffset.name())
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                assert_eq!(
                    found[0].message,
                    "Prefer ranges when generating random numbers instead of integers with offsets."
                );
                assert!(found[0].correction.is_none());
                assert_correction(
                    &RandomWithOffset,
                    &source,
                    &format!("# header\n  {after}\n"),
                );
            }
        }
    }
    #[test]
    fn reflections_are_reported_but_never_change_seeded_draws() {
        // Upstream corrects these to reflected ranges. The distribution agrees,
        // but every seeded draw is reversed: intentionally withhold those edits.
        for source in [
            "1 - rand(6)",
            "1 - ::Random.rand(6)",
            "1 - rand(0..6)",
            "1 - rand(0...6)",
        ] {
            let found = correction_offenses(&RandomWithOffset, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn pinned_and_extended_no_offense_lookalikes() {
        for source in [
            "rand(0..limit) + 1",
            "1 - rand(0...limit)",
            "rand(0..limit).succ",
            "rand(0...limit).pred",
            "rand(1..6)",
            "rand(1...6)",
            "rng.rand(6) + 1",
            "Random.new.rand(6) + 1",
            "Other::Random.rand(6) + 1",
            "Other::Kernel.rand(6).succ",
            "Random&.rand(6) + 1",
            "rand(6)&.succ",
            "rand(6).succ(1)",
            "rand(6, 7) + 1",
            "rand(6, &b) + 1",
            "rand(6) { 1 } + 1",
            "rand(6.0) + 1",
            "rand(6) + offset",
            "rand = 1; rand + 1",
            "rand(..6).succ",
            "(rand(6)) + 1",
            "rand((6)) + 1",
        ] {
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(
                correction_offenses(&RandomWithOffset, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn uncertain_bounds_blocks_comments_and_line_changes_are_withheld() {
        for source in [
            "rand(0) + 1",
            "rand(-6) + 1",
            "rand(2..1).succ",
            "rand(1...1).pred",
            "rand(999999999999999999999999999999).succ",
            "rand(6) + 4611686018427387903",
            "rand(1...4611686018427387904) - 1",
            "rand(0..5) - 4611686018427387904",
            "rand(6).succ { break :value }",
            "rand(\n # keep\n 6) + 1",
            "rand(\n6) + 1\np __LINE__",
        ] {
            let found = correction_offenses(&RandomWithOffset, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn literal_values_defaults_and_operator_binding() {
        for (before, after) in [
            (
                "def f(x = rand(0x6) + 0b1); x; end",
                "def f(x = rand(1..6)); x; end",
            ),
            ("2 * (rand(-2...4) - -1)", "2 * (rand(-1..4))"),
            ("rand(6).+(1)", "rand(1..6)"),
        ] {
            assert_correction(&RandomWithOffset, before, after);
        }
    }
    #[test]
    fn visible_rand_range_and_integer_rebindings_withhold_edits() {
        for binding in [
            "def rand(n); 99; end",
            "Random = Custom",
            "Kernel = Custom",
            "Random.define_singleton_method(:rand) { |n| 99 }",
            "Kernel.singleton_class.class_eval { alias rand object_id }",
            "Random.instance_eval('def rand(n); 99; end')",
            "class Integer; def succ; 99; end; end",
            "Integer.define_method(:+) { |n| 99 }",
            "module M; refine Integer do; def succ; 99; end; end; end",
            "Range = Custom",
            "alias rand other",
        ] {
            let source = format!("{binding}\nrand(6).succ\n");
            let found = correction_offenses(&RandomWithOffset, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
}
