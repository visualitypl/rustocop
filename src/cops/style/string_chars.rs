//! `Style/StringChars` — exact empty-argument spellings, detection only.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct StringChars;
impl Cop for StringChars {
    fn name(&self) -> &'static str {
        "Style/StringChars"
    }
    fn unsafe_fixes(&self) -> bool {
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
        if call.name().as_slice() != b"split" {
            return;
        }
        let args = crate::cops::ast::call_arguments(&call);
        let [arg] = args.as_slice() else { return };
        if !matches!(arg.location().as_slice(), b"//" | b"''" | b"\"\"") {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let start = selector.start_offset();
        let end = crate::cops::ast::source_end(node);
        let current = crate::offense::single_line(&file.bytes[start..end]);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `chars` instead of `{current}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: start..crate::cops::bundler::ordered_gem_node::send_end(&call),
                    replacement: "chars".into(),
                }),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop string_chars_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Unsafe corrections are not ported.
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == StringChars.name())
            .collect()
    }
    #[test]
    fn pinned_positions_and_messages() {
        for (s, col, current) in [
            ("string.split(//)", 8, "split(//)"),
            ("string.split('')", 8, "split('')"),
            ("string.split(\"\")", 8, "split(\"\")"),
            ("do_something { |foo| foo.split '' }", 26, "split ''"),
            ("string&.split(//)", 9, "split(//)"),
        ] {
            let o = offenses(&format!("# header\n{s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col, 2), "{s}");
            assert_eq!(o[0].message, format!("Use `chars` instead of `{current}`."));
            assert!(correction_offenses(&StringChars, s)[0].correction.is_some());
        }
        assert!(!StringChars.safe_fixes());
    }
    #[test]
    fn exact_source_not_decoded_emptiness_arity_and_lookalikes() {
        for s in [
            "string.chars",
            "string.split(/ /)",
            "string.split",
            "string.split(//i)",
            "string.split(%r{})",
            "string.split(%q{})",
            "string.split(%Q{})",
            "string.split(\"#{''}\")",
            "string.split('', 0)",
            "string.split('', &b)",
            "string.split(*[''])",
            "string.split((''))",
            "string.split(/(?:)/)",
            "def split(x); end",
            "# string.split('')\n\"string.split('')\"",
            "string.split(<<~S)\nS\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn bare_calls_blocks_and_multiline_send_ranges() {
        for (s, line, col, end) in [
            ("split ''", 1, 1, 1),
            ("s.split('') { ignored }", 1, 3, 1),
            ("s.split(\n  ''\n)", 1, 3, 3),
            ("`never execute`.split('')", 1, 17, 1),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                (o[0].line, o[0].col, o[0].end_line),
                (line, col, end),
                "{s}"
            );
        }
    }
}
