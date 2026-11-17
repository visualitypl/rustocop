//! `Style/RandomWithOffset` — prefer explicit integer ranges.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
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
                Some(Correction {
                    range: node.location().start_offset()..ast::source_end(node),
                    replacement: matched.translation()?,
                })
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

fn integer(node: &Node<'_>) -> Option<i128> {
    let value = node.as_integer_node()?.value();
    let (negative, digits) = value.to_u32_digits();
    if digits.len() > 3 {
        return None;
    }
    let value = digits
        .iter()
        .rev()
        .fold(0_i128, |n, d| (n << 32) + i128::from(*d));
    Some(if negative { -value } else { value })
}
impl Match<'_> {
    /// RuboCop's `corrected_*`.
    fn translation(&self) -> Option<String> {
        let (left, right) = if let Some(range) = self.bound.as_range_node() {
            let left = integer(&range.left()?)?;
            let right = integer(&range.right()?)?;
            (
                left,
                right - i128::from(range.operator_loc().as_slice() == b"..."),
            )
        } else {
            (0, integer(&self.bound)? - 1)
        };
        let offset = self.offset.as_ref().map_or(Some(1), integer)?;
        let (new_left, new_right) = match (self.reflection, self.subtract) {
            (true, _) => (offset - right, offset - left),
            (false, true) => (left - offset, right - offset),
            (false, false) => (left + offset, right + offset),
        };
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
}
