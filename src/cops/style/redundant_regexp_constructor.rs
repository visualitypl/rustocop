//! `Style/RedundantRegexpConstructor` — regexp literals wrapped in constructors.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantRegexpConstructor;
impl Cop for RedundantRegexpConstructor {
    fn name(&self) -> &'static str {
        "Style/RedundantRegexpConstructor"
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
        let Some(regexp) = candidate(node) else {
            return;
        };
        let call = node.as_call_node().unwrap();
        let loc = node.location();
        let range = loc.start_offset()..ast::source_end(node);
        let (line, col) = file.line_col(loc.start_offset());
        let correction = (cx.corrections_requested() && !ambiguous_slash_regexp(node, &regexp, cx))
            .then(|| {
                Some(Correction {
                    range: range.clone(),
                    replacement: std::str::from_utf8(regexp.location().as_slice())
                        .ok()?
                        .into(),
                })
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Remove the redundant `Regexp.{}`.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
            line,
            col,
            end_line: file.line_of(range.end.saturating_sub(1)),
            correction,
        });
    }
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let call = node.as_call_node()?;
    if call.is_safe_navigation()
        || !matches!(call.name().as_slice(), b"new" | b"compile")
        || !ast::is_global_const(&call.receiver()?, &["Regexp"])
    {
        return None;
    }
    let args = call.arguments()?;
    if args.arguments().len() != 1 {
        return None;
    }
    let first = args.arguments().iter().next()?;
    matches!(
        first,
        Node::RegularExpressionNode { .. } | Node::InterpolatedRegularExpressionNode { .. }
    )
    .then_some(first)
}
/// RuboCop's `ambiguous_slash_regexp?`: `/ x/` or `/=x/` as a command's
/// first argument would read as division.
fn ambiguous_slash_regexp(node: &Node<'_>, regexp: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let (opening, content) = if let Some(r) = regexp.as_regular_expression_node() {
        (r.opening_loc(), r.content_loc().as_slice().to_vec())
    } else if let Some(r) = regexp.as_interpolated_regular_expression_node() {
        let start = r.opening_loc().end_offset();
        let end = r.closing_loc().start_offset();
        let base = regexp.location().start_offset();
        (
            r.opening_loc(),
            regexp.location().as_slice()[start - base..end - base].to_vec(),
        )
    } else {
        return false;
    };
    if opening.as_slice() != b"/"
        || !content
            .first()
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'=')
    {
        return false;
    }
    let start = node.location().start_offset();
    cx.ancestors().filter_map(|a| a.as_call_node()).any(|call| {
        let name = call.name().as_slice();
        let arguments = ast::call_arguments(&call);
        !arguments.is_empty()
            && !call.opening_loc().is_some_and(|o| o.as_slice() == b"(")
            && !ast::is_operator_method(name)
            && !(name.ends_with(b"=") && !matches!(name, b"==" | b"!=" | b"===" | b"<=" | b">="))
            && arguments[0].location().start_offset() == start
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop redundant_regexp_constructor_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantRegexpConstructor")
            .collect()
    }
    #[test]
    fn pinned_diagnostics_delimiters_and_options() {
        for prefix in ["Regexp", "::Regexp"] {
            for method in ["new", "compile"] {
                for pattern in ["/regexp/", "/regexp/i", r"/\A#{regexp}\z/io", "%r{foo/bar}"] {
                    let s = format!("\n  {prefix}.{method}({pattern})");
                    let found = offenses(&s);
                    assert_eq!(found.len(), 1, "{s}");
                    assert_eq!((found[0].line, found[0].col), (2, 3));
                    assert_eq!(
                        found[0].message,
                        format!("Remove the redundant `Regexp.{method}`.")
                    );
                }
            }
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for s in [
            "Regexp.new('regexp')",
            "Regexp.compile('regexp')",
            "Regexp.new('regexp', Regexp::IGNORECASE)",
            "Regexp.new(/x/, Regexp::IGNORECASE)",
            "Other::Regexp.new(/x/)",
            "object.new(/x/)",
            "Regexp&.new(/x/)",
            "/x/",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn canonical_discarded_constructor_corrections() {
        for pattern in ["/regexp/", "/regexp/i", "%r{foo/bar}", r"/\//n"] {
            for prefix in ["Regexp.new", "::Regexp.compile"] {
                assert_correction(
                    &RedundantRegexpConstructor,
                    &format!("{prefix}({pattern})\n"),
                    &format!("{pattern}\n"),
                );
            }
        }
    }
}
