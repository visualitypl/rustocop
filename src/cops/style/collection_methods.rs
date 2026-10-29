//! `Style/CollectionMethods` — opt-in fixed Enumerable alias preferences.
//! Only calls with a literal/forwarded block (or inject's final Symbol) qualify.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct CollectionMethods;
impl Cop for CollectionMethods {
    fn name(&self) -> &'static str {
        "Style/CollectionMethods"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let prefer = match call.name().as_slice() {
            b"collect" => "map",
            b"collect!" => "map!",
            b"collect_concat" => "flat_map",
            b"inject" => "reduce",
            b"detect" => "find",
            b"find_all" => "select",
            b"member?" => "include?",
            _ => return,
        };
        if call.block().is_none()
            && !(call.name().as_slice() == b"inject"
                && call
                    .arguments()
                    .and_then(|a| a.arguments().iter().last())
                    .is_some_and(|n| n.as_symbol_node().is_some()))
        {
            return;
        }
        let Some(loc) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Prefer `{prefer}` over `{}`.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1)),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! RuboCop collection_methods_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    fn offenses(s: &str) -> Vec<Offense> {
        correction_offenses(&CollectionMethods, s)
    }
    #[test]
    fn all_seven_fixed_aliases_and_block_forms() {
        for (method, prefer) in [
            ("collect", "map"),
            ("collect!", "map!"),
            ("collect_concat", "flat_map"),
            ("inject", "reduce"),
            ("detect", "find"),
            ("find_all", "select"),
            ("member?", "include?"),
        ] {
            for recv in ["", "self.", "items.", "items&.", "factory()."] {
                for suffix in [
                    " { |e| e + 1 }",
                    " { _1 + 1 }",
                    " { it + 1 }",
                    "(&:test)",
                    "(0, &:test)",
                    "(&block)",
                    "(&nil)",
                    " { break :done }",
                    " { |a,b=1| a }",
                    " do |e| next e end",
                ] {
                    let s = format!("# header\n  {recv}{method}{suffix}");
                    let o = offenses(&s);
                    assert_eq!(o.len(), 1, "{s}");
                    assert_eq!(
                        (o[0].line, o[0].col, o[0].end_line),
                        (2, (3 + recv.len()) as u32, 2)
                    );
                    assert_eq!(o[0].message, format!("Prefer `{prefer}` over `{method}`."));
                    assert!(o[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn bare_symbols_only_for_inject_and_real_last_argument() {
        for s in [
            "x.inject(:+)",
            "x&.inject(0, :+)",
            "inject(:+)",
            "x.inject(:\"+\")",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "x.collect",
            "x.collect!",
            "x.collect_concat",
            "x.inject",
            "x.detect",
            "x.find_all",
            "x.member?(:a)",
            "x.collect(:+)",
            "x.collect(0, :+)",
            "x.inject(:+, 0)",
            "x.inject(op: :+)",
            "x.inject(*[:+])",
            "x.inject(\"+\")",
            "x.reduce(:+)",
            "x.map(&:foo)",
            "collect = 1; collect",
            "def collect; end",
            "def f(...); collect(...); end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn block_argument_forwarding_anonymous_and_literal_wrappers() {
        for s in [
            "def f(&); collect(&); end",
            "x.collect(&factory())",
            "x.inject(0) {}",
            "x.member?(item) { effect }",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
    #[test]
    fn bounded_many_and_wide_calls() {
        assert_eq!(offenses(&"x.collect { it }\n".repeat(6000)).len(), 6000);
        let args = "0,".repeat(6000);
        assert_eq!(offenses(&format!("x.inject({args}:+)")).len(), 1);
    }
}
