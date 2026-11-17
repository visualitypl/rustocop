//! `Style/PerlBackrefs` — prefer explicit regexp match access.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct PerlBackrefs;
impl Cop for PerlBackrefs {
    fn name(&self) -> &'static str {
        "Style/PerlBackrefs"
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
        let loc = node.location();
        let expression = if node.as_numbered_reference_read_node().is_some() {
            format!(
                "Regexp.last_match({})",
                String::from_utf8_lossy(&loc.as_slice()[1..])
            )
        } else if node.as_back_reference_read_node().is_some()
            || node.as_global_variable_read_node().is_some()
        {
            match loc.as_slice() {
                b"$&" | b"$MATCH" => "Regexp.last_match(0)".into(),
                b"$`" | b"$PREMATCH" => "Regexp.last_match.pre_match".into(),
                b"$'" | b"$POSTMATCH" => "Regexp.last_match.post_match".into(),
                _ => return,
            }
        } else {
            return;
        };
        let expression = if cx
            .ancestors()
            .any(|n| matches!(n, Node::ClassNode { .. } | Node::ModuleNode { .. }))
        {
            format!("::{expression}")
        } else {
            expression
        };
        // Pre/post match calls raise when no match exists. English aliases need
        // a dominating English load and can be independently rebound. Preserve
        // those reads; detection must not depend on correction eligibility.
        let correction = cx.corrections_requested().then(|| {
            let replacement = if cx
                .parent()
                .is_some_and(|n| n.as_embedded_variable_node().is_some())
            {
                format!("{{{expression}}}")
            } else {
                expression.clone()
            };
            Correction {
                range: loc.start_offset()..loc.end_offset(),
                replacement,
            }
        });
        let original = String::from_utf8_lossy(loc.as_slice());
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Prefer `{expression}` over `{original}`."),
            line,
            col,
            end_line: line,
            correction,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/perl_backrefs_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/PerlBackrefs")
            .collect()
    }
    #[test]
    fn braceless_and_explicit_interpolation_and_namespaces() {
        let found = offenses("module Foo\n  class Regexp\n  end\n\n  puts $1\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (5, 8));
        assert_eq!(
            found[0].message,
            "Prefer `::Regexp.last_match(1)` over `$1`."
        );
        for (before, after) in [
            ("\"#$1\"", "\"#{Regexp.last_match(1)}\""),
            ("`#$1`", "`#{Regexp.last_match(1)}`"),
            ("/#$1/", "/#{Regexp.last_match(1)}/"),
            ("\"#{$1}\"", "\"#{Regexp.last_match(1)}\""),
            ("%W[#$1]", "%W[#{Regexp.last_match(1)}]"),
            ("%I[#$1]", "%I[#{Regexp.last_match(1)}]"),
            (
                "module Foo\n  class Regexp\n  end\n  puts $1\nend\n",
                "module Foo\n  class Regexp\n  end\n  puts ::Regexp.last_match(1)\nend\n",
            ),
            (
                "class Foo; def f; $1; end; end",
                "class Foo; def f; ::Regexp.last_match(1); end; end",
            ),
        ] {
            assert_correction(&PerlBackrefs, before, after);
        }
    }
    #[test]
    fn no_equivalent_or_non_read_lookalikes() {
        let found = offenses("alias $x $MATCH");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 10));
        for source in [
            "$+",
            "$LAST_PAREN_MATCH",
            "$~",
            "$0",
            "$MATCH = 1",
            "$MATCH ||= 1",
            "'$1' # $&\n",
            "Regexp.last_match(1)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
