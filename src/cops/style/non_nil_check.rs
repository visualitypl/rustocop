//! `Style/NonNilCheck` — fixed non-semantic-change non-nil comparisons.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NonNilCheck;
impl Cop for NonNilCheck {
    fn name(&self) -> &'static str {
        "Style/NonNilCheck"
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
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"!="
            || !super::nil_comparison::compares_nil(&call)
            || predicate_tail(node, cx)
        {
            return;
        }
        let receiver = call.receiver().unwrap();
        let source = String::from_utf8_lossy(receiver.location().as_slice());
        let receiver = if super::nil_comparison::operator_expression(&receiver, true)
            || source.starts_with(['-', '+'])
        {
            format!("({source})")
        } else {
            source.into_owned()
        };
        let replacement = format!("!{receiver}.nil?");
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let current = String::from_utf8_lossy(&file.bytes[start..end]);
        let (line, col) = file.line_col(start);
        let correction = if cx.corrections_requested()
            && super::nil_comparison::trusted_call(&call)
            && super::nil_comparison::core_safe(node, cx, file)
        {
            let chained = cx
                .parent()
                .and_then(|n| n.as_call_node())
                .and_then(|c| c.receiver())
                .is_some_and(|n| {
                    n.location().start_offset() == start
                        && n.location().end_offset() == node.location().end_offset()
                });
            let replacement = if chained {
                format!("({replacement})")
            } else {
                replacement.clone()
            };
            super::literal_spelling::safe_edit(
                file,
                cx,
                Correction {
                    range: start..end,
                    replacement,
                },
            )
        } else {
            None
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end - 1),
            message: format!("Prefer `{replacement}` over `{current}`."),
            correction,
        });
    }
}
fn predicate_tail(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    if node
        .as_call_node()
        .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()))
    {
        return false;
    }
    let Some(def) = cx.ancestors().find_map(|n| n.as_def_node()) else {
        return false;
    };
    if !def.name().as_slice().ends_with(b"?") {
        return false;
    }
    let Some(body) = def.body() else { return false };
    // Prism always adds a StatementsNode, unlike RuboCop's single-expression
    // bodies. Remove only that scaffold, then take the last child of one
    // upstream `begin` (a statement sequence or explicit parentheses).
    let body = body
        .as_statements_node()
        .filter(|s| s.body().len() == 1)
        .and_then(|s| s.body().last())
        .unwrap_or(body);
    let body = if let Some(group) = body.as_parentheses_node() {
        let Some(body) = group.body() else {
            return false;
        };
        body
    } else {
        body
    };
    let tail = body
        .as_statements_node()
        .and_then(|s| s.body().last())
        .unwrap_or(body);
    tail.location().start_offset() == node.location().start_offset()
        && tail.location().end_offset() == node.location().end_offset()
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/non_nil_check_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/NonNilCheck")
            .collect()
    }
    #[test]
    fn flags_comparison_and_message_precedence() {
        for (s, replacement) in [
            ("x != nil", "!x.nil?"),
            ("a + b != nil", "!(a + b).nil?"),
            ("!a != nil", "!(!a).nil?"),
            ("-a != nil", "!(-a).nil?"),
            ("~a != nil", "!(~a).nil?"),
            ("(line =~ //) != nil", "!(line =~ //).nil?"),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 1));
            assert_eq!(o[0].message, format!("Prefer `{replacement}` over `{s}`."));
        }
    }
    #[test]
    fn predicate_tail_only_is_exempt() {
        for s in [
            "def signed_in?; current_user != nil; end",
            "def Test.signed_in?; something; current_user != nil; end",
            "def signed_in? = current_user != nil",
            "def signed_in?; (current_user != nil); end",
            "def Test.signed_in?; (current_user != nil); end",
            "def signed_in? = (current_user != nil)",
            "def signed_in?; (something; current_user != nil); end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            "def signed_in; current_user != nil; end",
            "def signed_in?; current_user != nil; something; end",
            "def signed_in?; return current_user != nil; end",
            "def signed_in?; ((current_user != nil)); end",
            "def signed_in?; something; (current_user != nil); end",
            "def signed_in?; if current_user != nil; end; end",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
    #[test]
    fn corrects_literals_without_collapsing_false_or_changing_chained_binding() {
        use crate::cops::testing::correction_offenses;
        for (s, expected) in [
            ("false != nil", "!false.nil?"),
            ("nil != nil", "!nil.nil?"),
            ("-1 != nil", "!(-1).nil?"),
            ("1.!=(nil).to_s", "(!1.nil?).to_s"),
        ] {
            let o = correction_offenses(&NonNilCheck, s);
            let c = o[0].correction.as_ref().expect(s);
            assert_eq!(
                format!(
                    "{}{}{}",
                    &s[..c.range.start],
                    c.replacement,
                    &s[c.range.end..]
                ),
                expected
            );
        }
        for s in [
            "x != nil",
            "a + b != nil",
            "!a != nil",
            "class Integer; def !=(x); false; end; end; 1 != nil",
        ] {
            assert!(
                correction_offenses(&NonNilCheck, s)[0].correction.is_none(),
                "{s}"
            );
        }
    }
    #[test]
    fn ignores_semantic_changes_and_lookalikes() {
        for s in [
            "x != 0",
            "!x.nil?",
            "not x.nil?",
            "!nil?",
            "puts x unless x.nil?",
            "x&.!=(nil)",
            "x.!=(nil, &block)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
