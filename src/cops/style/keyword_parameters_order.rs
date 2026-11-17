//! Required keywords precede optional keywords; uncertain edits stay reported.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct KeywordParametersOrder;
impl Cop for KeywordParametersOrder {
    fn name(&self) -> &'static str {
        "Style/KeywordParametersOrder"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_parameters_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(params) = node.as_parameters_node() else {
            return;
        };
        let keywords: Vec<_> = params.keywords().iter().collect();
        let Some(last_required) = keywords
            .iter()
            .rposition(|k| k.as_required_keyword_parameter_node().is_some())
        else {
            return;
        };
        let bad: Vec<_> = keywords[..last_required]
            .iter()
            .filter(|k| k.as_optional_keyword_parameter_node().is_some())
            .collect();
        if bad.is_empty() {
            return;
        }
        let fix = cx
            .corrections_requested()
            .then(|| correction(node, cx, file, &keywords, bad[0]))
            .flatten();
        for (index, keyword) in bad.into_iter().enumerate() {
            let start = keyword.location().start_offset();
            let (line, col) = file.line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file
                    .line_of(keyword.location().end_offset().saturating_sub(1).max(start)),
                message: "Place optional keyword parameters at the end of the parameters list."
                    .into(),
                correction: if index == 0 { fix.clone() } else { None },
            });
        }
    }
}
fn correction(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    keywords: &[Node<'_>],
    first: &Node<'_>,
) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    let facts = file.super_facts(root);
    let owner = cx.ancestors().find(|a| {
        matches!(
            a,
            Node::DefNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
        )
    })?;
    if facts.observes_parameters
        || facts
            .implicit_super_scopes
            .contains(&owner.location().start_offset())
    {
        return None;
    }
    let loc = node.location();
    if super::correction::contains_comment(file, loc.start_offset()..loc.end_offset()) {
        return None;
    }
    // A moved required keyword changes the lexical binding of earlier defaults.
    // Only inert scalar defaults are proven here; arbitrary calls, locals,
    // interpolation, heredocs and control-flow defaults remain unmodified.
    if keywords
        .iter()
        .filter_map(|k| k.as_optional_keyword_parameter_node())
        .any(|p| !inert_default(&p.value()))
    {
        return None;
    }
    let start = first.location().start_offset();
    let moved: Vec<_> = keywords
        .iter()
        .filter(|k| {
            k.location().start_offset() > start && k.as_required_keyword_parameter_node().is_some()
        })
        .collect();
    let mut edits = vec![Correction {
        range: start..start,
        replacement: format!(
            "{}, ",
            moved
                .iter()
                .map(|k| std::str::from_utf8(k.location().as_slice()))
                .collect::<Result<Vec<_>, _>>()
                .ok()?
                .join(", ")
        ),
    }];
    for keyword in moved {
        let mut begin = keyword.location().start_offset();
        while begin > start && file.bytes[begin - 1].is_ascii_whitespace() {
            begin -= 1;
        }
        if file.bytes.get(begin.checked_sub(1)?) != Some(&b',') {
            return None;
        }
        begin -= 1;
        let mut end = keyword.location().end_offset();
        while matches!(file.bytes.get(end), Some(b' ' | b'\t')) {
            end += 1;
        }
        edits.push(Correction {
            range: begin..end,
            replacement: String::new(),
        });
    }
    let edit = super::correction::combine(file, edits)?;
    let changes_lines = file.bytes[edit.range.clone()]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        != edit.replacement.bytes().filter(|b| *b == b'\n').count();
    if changes_lines
        && (edit.range.start < file.line_value_boundary()
            || super::correction::touches_directive(file, edit.range.clone())
            || file.contains_heredoc(root, node)
            || file.heredoc_pending_at(root, edit.range.start))
    {
        return None;
    }
    Some(edit)
}
fn inert_default(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
    ) || node.as_symbol_node().is_some()
        || node.as_string_node().is_some_and(|s| {
            !s.opening_loc()
                .is_some_and(|l| l.as_slice().starts_with(b"<<"))
        })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop keyword_parameters_order_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn pinned_comments_and_ordered_lookalikes() {
        let source = "def foo(optional: 123,\n    # Some explanation\n    required:)\nend\n";
        let found = correction_offenses(&KeywordParametersOrder, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 9));
        assert!(found[0].correction.is_none());
        for source in [
            "def m(arg, required:, optional: 1); end",
            "m(arg) do |block_arg, required:, optional: 1|; end",
            "def m(a = 1, b); end",
            "->(required:, optional: 1) {}",
            "def m(a: 1, b: 2); end",
            "def m(a:, b:); end",
        ] {
            assert!(
                correction_offenses(&KeywordParametersOrder, source).is_empty(),
                "{source}"
            );
        }
        assert_correction(
            &KeywordParametersOrder,
            "->(a: 1, b:) { [a, b] }",
            "->(b:, a: 1) { [a, b] }",
        );
        assert_correction(
            &KeywordParametersOrder,
            "m { |(x,y), a: 1, b:; z| [x,y,a,b,z] }",
            "m { |(x,y), b:, a: 1; z| [x,y,a,b,z] }",
        );
    }
    #[test]
    fn binding_evaluation_reflection_and_implicit_keyword_order_are_not_rewritten() {
        for source in [
            "def f(a: b, b:); [a,b]; end", // b was a method, not the later local
            "def f(a: (log << :a), b:); [a,b]; end",
            "def f(a: raise('oops'), b:); end",
            "def f(a: 1, b:); super; end",
            "def f(a: 1, b:); tap { super }; end",
            "def f(a: 1, b:); local_variables; end",
            "def f(a: 1, b:); binding; end",
            "def f(a: 1, b:); end; p instance_method(:f).parameters",
            "def f(a: 1, b:); end; p methods.map(&:parameters)",
            "def self.method_added(name); p instance_method(name).parameters; end; def f(a: 1, b:); end",
            "def f(a: 1, b:); end; p observer.call(self)",
            "def f(a: 1, b:); end; object.parameters ||= value",
            "def f(a: 1, b:); end; ::TracePoint.new(:call) {}",
            "def f(a: __LINE__, b:); end",
        ] {
            let found = correction_offenses(&KeywordParametersOrder, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn pinned_canonical_corrections() {
        for (before, after) in [
            (
                "def m(arg, optional: 1, required:)\nend\n",
                "def m(arg, required:, optional: 1)\nend\n",
            ),
            (
                "def m arg, optional: 1, required:\n  do_something\nend\n",
                "def m arg, required:, optional: 1\n  do_something\nend\n",
            ),
            (
                "def m arg, optional1: 1, optional2: 2, required:\n  do_something\nend\n",
                "def m arg, required:, optional1: 1, optional2: 2\n  do_something\nend\n",
            ),
            (
                "def m optional: 1, required:, other_optional: 2\n  do_something\nend\n",
                "def m required:, optional: 1, other_optional: 2\n  do_something\nend\n",
            ),
            (
                "def m(optional: 1, \n      required:, \n      other: 2)\nend\n",
                "def m(required:, optional: 1, \n      other: 2)\nend\n",
            ),
            (
                "def m(arg, optional1: 1, required1:, optional2: 2, required2:, **rest, &block)\nend\n",
                "def m(arg, required1:, required2:, optional1: 1, optional2: 2, **rest, &block)\nend\n",
            ),
            (
                "def m arg, optional1: 1, required1:, optional2: 2, required2:, **rest\n  do_something\nend\n",
                "def m arg, required1:, required2:, optional1: 1, optional2: 2, **rest\n  do_something\nend\n",
            ),
            (
                "def m arg, optional1: 1, required1:, optional2: 2, required2:, **rest, &block\n  do_something\nend\n",
                "def m arg, required1:, required2:, optional1: 1, optional2: 2, **rest, &block\n  do_something\nend\n",
            ),
            (
                "m(arg) do |block_arg, optional: 1, required:|\nend\n",
                "m(arg) do |block_arg, required:, optional: 1|\nend\n",
            ),
        ] {
            assert_correction(&KeywordParametersOrder, before, after);
        }
        let found = correction_offenses(
            &KeywordParametersOrder,
            "def m(arg, optional: 1, required:)\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 12));
        assert_eq!(
            found[0].message,
            "Place optional keyword parameters at the end of the parameters list."
        );
    }
}
