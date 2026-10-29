//! `Style/EmptyStringInsideInterpolation` — fixed trailing-conditional policy.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EmptyStringInsideInterpolation;
impl Cop for EmptyStringInsideInterpolation {
    fn name(&self) -> &'static str {
        "Style/EmptyStringInsideInterpolation"
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
        // Only immediate interpolation expressions, not arbitrary nested ifs.
        let mut parents = cx.ancestors();
        if !parents
            .next()
            .is_some_and(|p| p.as_statements_node().is_some())
            || !parents
                .next()
                .is_some_and(|p| p.as_embedded_statements_node().is_some())
        {
            return;
        }
        let (condition, yes, other, inverted) = if let Some(n) = node.as_if_node() {
            if n.end_keyword_loc().is_none() && n.then_keyword_loc().is_none() {
                return;
            }
            (n.predicate(), n.statements(), n.subsequent(), false)
        } else if let Some(n) = node.as_unless_node() {
            if n.end_keyword_loc().is_none() {
                return;
            }
            (
                n.predicate(),
                n.statements(),
                n.else_clause().map(|e| e.as_node()),
                true,
            )
        } else {
            return;
        };
        let yes = yes.map(|s| branch(&s));
        let no = other.and_then(|n| {
            if let Some(e) = n.as_else_node() {
                e.statements().map(|s| branch(&s))
            } else {
                Some(n)
            }
        });
        let (Some(yes), Some(no)) = (yes, no) else {
            return;
        };
        for (empty, outcome, keyword) in [
            (&yes, &no, if inverted { "if" } else { "unless" }),
            (&no, &yes, if inverted { "unless" } else { "if" }),
        ] {
            if !is_empty(empty) {
                continue;
            }
            let (line, col) = file.line_col(node.location().start_offset());
            let correction = cx
                .corrections_requested()
                .then(|| {
                    let root = cx.ancestors().next_back()?;
                    let facts = file.interpolation_facts(root);
                    // Moving source across writes can change Ruby's lexical local
                    // classification even though modifier evaluation is predicate-first.
                    if !facts.shape(node).movable
                        || !simple_condition(&condition)
                        || !simple_outcome(outcome)
                        || facts.shape(outcome).full_conditional
                        || facts.shape(&condition).full_conditional
                    {
                        return None;
                    }
                    // Empty String -> nil can change encoding as well as frozen
                    // state, even with mutable literal flags and primitive
                    // siblings. Without a whole-string encoding/folding proof,
                    // retain those offenses. An existing nil branch is unchanged.
                    empty.as_nil_node()?;
                    if file.contains_heredoc(root, node) {
                        return None;
                    }
                    let range = node.location().start_offset()..node.location().end_offset();
                    if super::correction::contains_comment(file, range.clone())
                        || file.bytes[range.clone()].contains(&b'\n')
                    {
                        return None;
                    }
                    let body = std::str::from_utf8(outcome.location().as_slice()).ok()?;
                    let predicate = std::str::from_utf8(condition.location().as_slice()).ok()?;
                    super::literal_spelling::safe_edit(
                        file,
                        cx,
                        Correction {
                            range,
                            replacement: format!("{body} {keyword} {predicate}"),
                        },
                    )
                })
                .flatten();
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Do not return empty strings in string interpolation.".into(),
                line,
                col,
                end_line: file.last_line_of(node.location()),
                correction,
            });
            // RuboCop deduplicates offenses at the same conditional when both
            // branches are empty; retain its first (empty-if) orientation.
            break;
        }
    }
}
fn branch<'pr>(s: &ruby_prism::StatementsNode<'pr>) -> Node<'pr> {
    if s.body().len() == 1 {
        s.body().iter().next().unwrap()
    } else {
        s.as_node()
    }
}
fn is_empty(n: &Node<'_>) -> bool {
    n.as_nil_node().is_some() || n.as_string_node().is_some_and(|s| s.unescaped().is_empty())
}
fn simple_outcome(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::StringNode { .. }
            | Node::InterpolatedStringNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::LocalVariableReadNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::GlobalVariableReadNode { .. }
            | Node::ClassVariableReadNode { .. }
            | Node::ConstantReadNode { .. }
    )
}
fn simple_condition(n: &Node<'_>) -> bool {
    simple_outcome(n)
        || n.as_call_node().is_some_and(|c| {
            c.receiver().is_none() && c.arguments().is_none() && c.block().is_none()
        })
}
#[cfg(test)]
mod tests {
    //! RuboCop empty_string_inside_interpolation_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == EmptyStringInsideInterpolation.name())
            .collect()
    }
    #[test]
    fn pinned_conditional_branches_positions_and_message() {
        for empty in ["''", "\"\"", "nil"] {
            for expr in [
                format!("condition ? 'foo' : {empty}"),
                format!("condition ? 42 : {empty}"),
                format!("if condition; 'foo' else {empty} end"),
                format!("condition ? {empty} : 'foo'"),
                format!("if condition; {empty} else 'foo' end"),
            ] {
                let src = format!("\"#{{{expr}}}\"");
                let o = offenses(&src);
                assert_eq!(o.len(), 1, "{src}");
                assert_eq!((o[0].line, o[0].col), (1, 4));
                assert_eq!(
                    o[0].message,
                    "Do not return empty strings in string interpolation."
                );
            }
        }
    }
    #[test]
    fn pinned_lookalikes() {
        for s in [
            "condition ? send_node : 'foo'",
            "condition ? 'foo' : send_node",
            "condition ? send_node : another_send_node",
            "'foo' if condition",
            "'' if condition",
            "'' unless condition",
        ] {
            assert!(offenses(&format!("\"#{{{s}}}\"")).is_empty());
        }
        assert!(offenses("condition ? 'foo' : ''").is_empty());
    }
    #[test]
    fn nil_branch_canonical_pinned_corrections() {
        for (expr, after) in [
            ("condition ? 'foo' : nil", "'foo' if condition"),
            ("condition ? 42 : nil", "42 if condition"),
            ("if condition; 'foo' else nil end", "'foo' if condition"),
            ("condition ? nil : 'foo'", "'foo' unless condition"),
            ("if condition; nil else 'foo' end", "'foo' unless condition"),
        ] {
            assert_correction(
                &EmptyStringInsideInterpolation,
                &format!("\"#{{{expr}}}\""),
                &format!("\"#{{{after}}}\""),
            );
        }
    }
    #[test]
    fn unknown_dispatch_empty_to_nil_is_reported_but_withheld() {
        for empty in ["''", "\"\""] {
            for expr in [
                format!("condition ? 'foo' : {empty}"),
                format!("condition ? 42 : {empty}"),
                format!("if condition; 'foo' else {empty} end"),
                format!("condition ? {empty} : 'foo'"),
                format!("if condition; {empty} else 'foo' end"),
            ] {
                let o = correction_offenses(
                    &EmptyStringInsideInterpolation,
                    &format!("\"#{{{expr}}}\""),
                );
                assert_eq!(o.len(), 1);
                assert!(o[0].correction.is_none());
            }
        }
    }
    #[test]
    fn all_four_nested_pinned_correction_examples_retain_diagnostics() {
        // All four upstream pairs are withheld: their bare predicates can
        // mutate NilClass/String#to_s, and the two .empty? predicates move syntax.
        for (before, upstream_after, count) in [
            (
                r##""#{outer ? "foo#{inner ? 'bar' : ''}" : ''}""##,
                r##""#{"foo#{'bar' if inner}" if outer}""##,
                2,
            ),
            (
                r##""#{outer ? '' : "foo#{inner ? '' : 'bar'}"}""##,
                r##""#{"foo#{'bar' unless inner}" unless outer}""##,
                2,
            ),
            (
                r##""#{"#{inner ? 'bar' : ''}".empty? ? 'foo' : ''}""##,
                r##""#{'foo' if "#{'bar' if inner}".empty?}""##,
                2,
            ),
            (
                r##""#{"#{inner ? 'bar' : 'baz'}".empty? ? 'foo' : ''}""##,
                r##""#{'foo' if "#{inner ? 'bar' : 'baz'}".empty?}""##,
                1,
            ),
        ] {
            assert_eq!(
                ruby_prism::parse(upstream_after.as_bytes())
                    .errors()
                    .count(),
                0
            );
            let o = correction_offenses(&EmptyStringInsideInterpolation, before);
            assert_eq!(o.len(), count, "{before}");
            assert!(o.iter().all(|o| o.correction.is_none()));
        }
    }
    #[test]
    fn source_binding_and_dispatch_hazards_keep_offenses() {
        for source in [
            r##""#{(x = true) ? x : nil}""##,
            r##""#{x ? (x = 1) : nil}""##,
            r##""#{true ? foo(1) : nil}""##,
            "\"#{if true\n 'x' # comment\nelse\n nil\nend}\"",
            "\"#{true ? <<~X : nil}\"\nx\nX\n",
            "class NilClass; def to_s; 'changed'; end; end\n\"#{false ? 'x' : ''}\"",
            "effect\n\"#{false ? 'x' : ''}\"",
            "\"#{object}\"\n\"#{false ? 'x' : ''}\"",
        ] {
            let o = correction_offenses(&EmptyStringInsideInterpolation, source);
            assert!(!o.is_empty(), "{source}");
            assert!(o.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
    #[test]
    fn interpolation_kinds_siblings_and_unless_keep_condition_semantics() {
        for (open, close) in [
            ("\"", "\""),
            (":\"", "\""),
            ("/", "/o"),
            ("`", "`"),
            ("%W[", "]"),
            ("%I[", "]"),
        ] {
            assert_correction(
                &EmptyStringInsideInterpolation,
                &format!("{open}#{{true ? 42 : nil}}{close}"),
                &format!("{open}#{{42 if true}}{close}"),
            );
        }
        assert_correction(
            &EmptyStringInsideInterpolation,
            r##""#{unless condition; nil else 'x' end}""##,
            r##""#{'x' if condition}""##,
        );
        assert_correction(
            &EmptyStringInsideInterpolation,
            r##""#{unless condition; 'x' else nil end}""##,
            r##""#{'x' unless condition}""##,
        );
        assert_eq!(
            offenses(r##""#{true ? '' : nil; false ? nil : ''}""##).len(),
            2
        );
        assert!(offenses(r##""#{consume(true ? '' : 'x')}""##).is_empty());
    }
    #[test]
    fn empty_if_branch_before_elsif_is_reported_without_an_unsafe_edit() {
        for source in [
            r##""#{if c; '' elsif d; 'x' else 'y' end}""##,
            r##""#{if c; '' else a; b end}""##,
            r##""#{if c; a; b else nil end}""##,
        ] {
            let o = correction_offenses(&EmptyStringInsideInterpolation, source);
            assert_eq!(o.len(), 1);
            assert_eq!((o[0].line, o[0].col), (1, 4));
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn nested_nil_conditionals_defer_outer_envelopes_to_keep_source_work_bounded() {
        let src = r##""#{c ? "#{d ? 'x' : nil}" : nil}""##;
        let o = correction_offenses(&EmptyStringInsideInterpolation, src);
        assert_eq!(o.len(), 2);
        assert!(o[0].correction.is_none());
        assert!(o[1].correction.is_some());
        assert_correction(
            &EmptyStringInsideInterpolation,
            src,
            r##""#{c ? "#{'x' if d}" : nil}""##,
        );
        for depth in [4, 8, 10] {
            let mut source = "'x'".to_owned();
            for _ in 0..depth {
                source = format!("\"#{{c ? {source} : nil}}\"");
            }
            let o = correction_offenses(&EmptyStringInsideInterpolation, &source);
            assert_eq!(o.len(), depth);
            let edits: Vec<_> = o.iter().filter_map(|o| o.correction.as_ref()).collect();
            assert_eq!(edits.len(), 1);
            assert!(
                edits
                    .iter()
                    .map(|e| e.range.len() + e.replacement.len())
                    .sum::<usize>()
                    <= source.len()
            );
        }
    }
    #[test]
    fn literal_flags_alone_do_not_prove_nil_equivalent_interpolation() {
        for header in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            for predicate in ["true", "false", "nil", "0"] {
                for empty in ["''", "\"\"", "%q()"] {
                    for expr in [
                        format!("{predicate} ? {empty} : 'yes'"),
                        format!("{predicate} ? 'yes' : {empty}"),
                        format!("if {predicate}; {empty} else 'yes' end"),
                        format!("unless {predicate}; {empty} else 42 end"),
                    ] {
                        for body in [
                            format!("\"#{{{expr}}}\""),
                            format!("\"prefix#{{{expr}}}suffix\""),
                            format!("\"#{{\"#{{{expr}}}\"}}\""),
                        ] {
                            let source = format!("{header}{body}");
                            let o = correction_offenses(&EmptyStringInsideInterpolation, &source);
                            assert_eq!(o.len(), 1, "{source}");
                            let offset = source.find(&expr).unwrap();
                            let file = SourceFile::new("a.rb".into(), source.as_bytes().to_vec());
                            assert_eq!((o[0].line, o[0].col), file.line_col(offset));
                            assert!(o[0].correction.is_none(), "{source}");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn primitive_siblings_do_not_prove_empty_to_nil_encoding_equivalence() {
        for header in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            for expr in ["true ? '' : 'yes'", "false ? 'yes' : ''"] {
                for body in [
                    format!("\"#{{1}}#{{{expr}}}#{{''}}\""),
                    format!("\"#{{''}}#{{{expr}}}#{{1}}\""),
                ] {
                    let source = format!("{header}{body}");
                    let o = correction_offenses(&EmptyStringInsideInterpolation, &source);
                    assert_eq!(o.len(), 1, "{source}");
                    assert!(o[0].correction.is_none(), "{source}");
                }
            }
        }
    }
    #[test]
    fn closed_literal_empty_branches_retain_offenses_without_encoding_proof() {
        for src in [r##""#{true ? 'foo' : ''}""##, r##""#{false ? '' : 42}""##] {
            let source = format!("# frozen_string_literal: false\n\"#{{1}}{}", &src[1..]);
            let o = correction_offenses(&EmptyStringInsideInterpolation, &source);
            assert_eq!(o.len(), 1);
            assert!(o[0].correction.is_none(), "{source}");
        }
    }
}
