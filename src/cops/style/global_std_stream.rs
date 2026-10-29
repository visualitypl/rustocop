//! `Style/GlobalStdStream` — prefer redirectable globals; unsafe replacements are never emitted.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct GlobalStdStream;
impl Cop for GlobalStdStream {
    fn name(&self) -> &'static str {
        "Style/GlobalStdStream"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(name) = ["STDIN", "STDOUT", "STDERR"]
            .into_iter()
            .find(|name| ast::is_global_const(node, &[*name]))
        else {
            return;
        };
        if cx.parent().is_some_and(|p| {
            p.location().start_offset() == node.location().start_offset()
                && matches!(
                    p,
                    Node::ConstantPathWriteNode { .. }
                        | Node::ConstantPathAndWriteNode { .. }
                        | Node::ConstantPathOrWriteNode { .. }
                        | Node::ConstantPathOperatorWriteNode { .. }
                )
        }) {
            return;
        }
        // Only direct, unqualified reads restoring the matching global are exempt.
        // Rooted constants, parens and compound assignments intentionally are not.
        if node.as_constant_read_node().is_some()
            && cx
                .parent()
                .and_then(|p| p.as_global_variable_write_node())
                .is_some_and(|w| {
                    w.name().as_slice() == format!("${}", name.to_lowercase()).as_bytes()
                })
        {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `${}` instead of `{name}`.", name.to_lowercase()),
            line,
            col,
            end_line: file.line_of(node.location().end_offset().saturating_sub(1)),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/global_std_stream_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    #[test]
    fn pinned_constants_in_calls_hashes_and_defaults() {
        for name in ["STDIN", "STDOUT", "STDERR"] {
            for prefix in ["", "::"] {
                let source = format!(
                    "{prefix}{name}.puts('hello')\nhash = {{ out: {prefix}{name}, key: value }}\ndef m(out = {prefix}{name})\nend\n"
                );
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == "Style/GlobalStdStream")
                    .collect();
                assert_eq!(found.len(), 3);
                for (o, pos) in found.iter().zip([(1, 1), (2, 15), (3, 13)]) {
                    assert_eq!((o.line, o.col), pos);
                    assert_eq!(
                        o.message,
                        format!("Use `${}` instead of `{name}`.", name.to_lowercase())
                    );
                }
                assert!(
                    correction_offenses(&GlobalStdStream, &source)
                        .iter()
                        .all(|o| o.correction.is_none())
                );
            }
        }
    }
    #[test]
    fn namespaced_constants_and_stream_restoration_are_exempt() {
        for source in [
            "SOME_CONST.puts('hello')",
            "$stdin = STDIN",
            "$stdout = STDOUT",
            "$stderr = STDERR",
            "$stdin = SOME_CONST",
            "Foo::STDOUT",
            "Foo::Bar::STDOUT",
            "::Foo::STDOUT",
            "::Foo::BaR::STDOUT",
            "foo::STDOUT",
            "STDOUT = custom",
            "::STDOUT = custom",
            "$stdout",
            "obj.STDERR",
            "'STDOUT'",
            "# STDERR\n",
        ] {
            assert!(
                correction_offenses(&GlobalStdStream, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn restoration_exemption_is_exact_not_an_alias_or_rooted_constant() {
        for source in [
            "$stdout = STDERR",
            "$stdout = ::STDOUT",
            "$stdout = (STDOUT)",
            "$stderr ||= STDERR",
            "STDOUT::Child",
            "module M; STDOUT = Custom; STDOUT; end",
        ] {
            let found = correction_offenses(&GlobalStdStream, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
    }
}
