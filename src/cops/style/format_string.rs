//! `Style/FormatString` — fixed `format` style.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct FormatString;
impl Cop for FormatString {
    fn name(&self) -> &'static str {
        "Style/FormatString"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if !formatter(&call) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let current = if call.name().as_slice() == b"%" {
            "String#%"
        } else {
            "sprintf"
        };
        let correction = cx
            .corrections_requested()
            .then(|| correction(&call))
            .flatten();
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Favor `format` over `{current}`."),
            line,
            col,
            end_line: file.line_of(selector.end_offset().saturating_sub(1)),
            correction,
        });
    }
}
fn string(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::StringNode { .. } | Node::InterpolatedStringNode { .. }
    )
}
fn formatter(call: &CallNode<'_>) -> bool {
    if call.is_safe_navigation() {
        return false;
    }
    let args = ast::call_arguments(call);
    match (call.name().as_slice(), call.receiver()) {
        (b"sprintf", None) => args.len() >= 2,
        (b"%", Some(receiver)) => {
            string(&receiver)
                || matches!(args.as_slice(), [n] if n.as_array_node().is_some() || n.as_hash_node().is_some() || n.as_keyword_hash_node().is_some())
        }
        _ => false,
    }
}
/// RuboCop's corrector for the `format` style.
fn correction(call: &CallNode<'_>) -> Option<Correction> {
    let text = |n: &Node<'_>| {
        std::str::from_utf8(n.location().as_slice())
            .ok()
            .map(str::to_owned)
    };
    if call.name().as_slice() != b"%" {
        let selector = call.message_loc()?;
        return Some(Correction {
            range: selector.start_offset()..selector.end_offset(),
            replacement: "format".into(),
        });
    }
    let receiver = call.receiver()?;
    let args = ast::call_arguments(call);
    // `variable_argument?`: `String#%` spreads an array argument.
    if string(&receiver) && args.len() == 1 && !autocorrectable(&args[0]) {
        return None;
    }
    let rhs = args.first()?;
    let children = if let Some(array) = rhs.as_array_node() {
        Some(array.elements().iter().collect::<Vec<_>>())
    } else {
        rhs.as_hash_node()
            .map(|hash| hash.elements().iter().collect())
    };
    let rhs = match children {
        Some(children) => children
            .iter()
            .map(text)
            .collect::<Option<Vec<_>>>()?
            .join(", "),
        None => text(rhs)?,
    };
    let loc = call.location();
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: format!("format({}, {rhs})", text(&receiver)?),
    })
}

/// RuboCop's `autocorrectable?`: a literal, or a conversion call.
fn autocorrectable(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::StringNode { .. }
            | Node::InterpolatedStringNode { .. }
            | Node::XStringNode { .. }
            | Node::InterpolatedXStringNode { .. }
            | Node::SourceFileNode { .. }
            | Node::SourceLineNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::SymbolNode { .. }
            | Node::InterpolatedSymbolNode { .. }
            | Node::ArrayNode { .. }
            | Node::HashNode { .. }
            | Node::RegularExpressionNode { .. }
            | Node::InterpolatedRegularExpressionNode { .. }
            | Node::RangeNode { .. }
            | Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
    ) || n.as_call_node().is_some_and(|c| {
        !c.block().is_some_and(|b| b.as_block_node().is_some())
            && matches!(
                c.name().as_slice(),
                b"to_d" | b"to_f" | b"to_h" | b"to_i" | b"to_r" | b"to_s" | b"to_sym"
            )
    })
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop format_string_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed format style only.
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == FormatString.name())
            .collect()
    }
    #[test]
    fn pinned_selector_positions_and_messages() {
        for (s, col, current) in [
            ("puts \"%d\" % 10", 11, "String#%"),
            ("puts x % [10, 11]", 8, "String#%"),
            ("puts x % { a: 10, b: 11 }", 8, "String#%"),
            ("puts \"#{x * 5} %d #{@test}\" % 10", 29, "String#%"),
            ("sprintf(something, a, b)", 1, "sprintf"),
            ("sprintf('%020d', 123)", 1, "sprintf"),
        ] {
            let o = offenses(&format!("# header\n  {s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col + 2, 2), "{s}");
            assert_eq!(o[0].message, format!("Favor `format` over `{current}`."));
        }
    }
    #[test]
    fn pinned_unknown_values_and_conversions_still_report() {
        for rhs in [
            "a",
            "@pair",
            "@@pair",
            "$pair",
            "PAIR",
            "Foo::PAIR",
            "(pair)",
            "pair&.foo",
            "a.to_s",
            "a.to_h",
            "a.to_i",
            "a.to_f",
            "a.to_r",
            "a.to_d",
            "a.to_sym",
            "a&.to_s",
        ] {
            let s = format!("puts \"%s\" % {rhs}");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 11));
        }
        assert_eq!(offenses("a = something()\nputs \"%d\" % a")[0].line, 2);
    }
    #[test]
    fn pinned_and_structural_no_offense_lookalikes() {
        for s in [
            "puts 10 % 4",
            "puts x % Y",
            "format(something, a, b)",
            "sprintf :xml",
            "sprintf",
            "object.sprintf('%s', 1)",
            "Kernel.sprintf('%s', 1)",
            "::Kernel.sprintf('%s', 1)",
            "object&.sprintf('%s', 1)",
            "x % value",
            "x&.%(value)",
            "'%s'&.%(1)",
            "`no` % 1",
            "sprintf = 1; sprintf",
            "def sprintf(a, b); end",
            "# sprintf('%s', 1)\n'sprintf'",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn percent_argument_and_receiver_shapes() {
        for s in [
            "\"%s\".%()",
            "\"%s\".%(1, 2)",
            "\"%s\".%(1, &block)",
            "sprintf('%s', &block)",
            "x.%([1])",
            "x.%(a: 1)",
            "\"%s\" % %w[a b]",
            "\"%s\" % %i[a b]",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "x.%([1], 2)",
            "x.%([1], &block)",
            "x&.%([1])",
            "sprintf('%s') { 1 }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
