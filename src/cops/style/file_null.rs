//! `Style/FileNull` — portable null device literals (detection only).
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};
pub struct FileNull;
impl Cop for FileNull {
    fn name(&self) -> &'static str {
        "Style/FileNull"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((value, location)) = literal(node, |value, location| {
            (value.eq_ignore_ascii_case(b"/dev/null")
                || value.eq_ignore_ascii_case(b"NUL:")
                || value.eq_ignore_ascii_case(b"NUL"))
            .then(|| (String::from_utf8_lossy(value).into_owned(), location))
        })
        .flatten() else {
            return;
        };
        let device = !value.eq_ignore_ascii_case("NUL");
        if node.as_string_node().is_some()
            && cx.parent().is_some_and(|p| {
                matches!(
                    p,
                    Node::ArrayNode { .. }
                        | Node::AssocNode { .. }
                        | Node::InterpolatedStringNode { .. }
                )
            })
        {
            return;
        }
        if !device && !file.contains_dev_null(cx.ancestors().next_back().unwrap_or(node)) {
            return;
        }
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `File::NULL` instead of `{value}`."),
            line,
            col,
            end_line: file.line_of(location.end_offset().saturating_sub(1)),
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: location.start_offset()..location.end_offset(),
                    replacement: "File::NULL".into(),
                }),
        });
    }
}

// RuboCop represents static command/regexp content as a child str. Prism
// stores that same value directly on the token node; report its content range.
fn literal<'pr, T>(node: &Node<'pr>, f: impl FnOnce(&[u8], Location<'pr>) -> T) -> Option<T> {
    if let Some(string) = node.as_string_node() {
        Some(f(string.unescaped(), string.location()))
    } else if let Some(command) = node.as_x_string_node() {
        Some(f(command.unescaped(), command.content_loc()))
    } else if let Some(regex) = node.as_regular_expression_node() {
        Some(f(regex.unescaped(), regex.content_loc()))
    } else {
        node.as_match_last_line_node()
            .map(|regex| f(regex.unescaped(), regex.content_loc()))
    }
}

/// File-wide evidence includes strings in otherwise ignored containers and
/// static interpolation parts. Reuse the main AST and cache exactly one walk.
#[derive(Default)]
pub(crate) struct NullDevices {
    pub(crate) contains_dev_null: bool,
    #[cfg(test)]
    pub(crate) visited: usize,
}
impl NullDevices {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut index = Self::default();
        walk(&mut index, root);
        index
    }
}
impl<'pr> NodeVisitor<'pr> for NullDevices {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visited += 1;
        }
        self.contains_dev_null |=
            literal(&node, |value, _| value.eq_ignore_ascii_case(b"/dev/null")).unwrap_or(false);
    }
    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/file_null_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<crate::offense::Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/FileNull")
            .collect()
    }
    #[test]
    fn pinned_literals_positions_messages_and_unsafe_fixes() {
        for (value, prefix) in [
            ("/dev/null", ""),
            ("NUL:", ""),
            ("NUL", "CONST = '/dev/null'; "),
            ("nul", "CONST = '/DEV/NULL'; "),
        ] {
            let source = format!("# header\n{prefix}  path = '{value}'\n");
            let found = offenses(&source);
            let last = found.last().unwrap();
            assert_eq!((last.line, last.col), (2, prefix.len() as u32 + 10));
            assert_eq!(
                last.message,
                format!("Use `File::NULL` instead of `{value}`.")
            );
            assert!(
                correction_offenses(&FileNull, &source)
                    .iter()
                    .all(|o| o.correction.is_some())
            );
        }
        assert_eq!(offenses("path = cond ? '/dev/null' : 'NUL:'").len(), 2);
        for literal in ["\"/dev\\x2fnull\"", "%q(/dev/null)", "\"/DEV/NULL\""] {
            assert_eq!(offenses(literal).len(), 1, "{literal}");
        }
    }
    #[test]
    fn synthetic_string_children_of_commands_and_regexps_match_reference_positions() {
        for (source, col) in [
            ("`/dev/null`", 2),
            ("%x(/dev/null)", 4),
            (r"/\/dev\/null/", 2),
            ("[`/dev/null`]", 3),
            ("`/dev/null#{suffix}`", 2),
            (r"/\/dev\/null#{suffix}/", 2),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!(found[0].message, "Use `File::NULL` instead of `/dev/null`.");
            assert!(
                correction_offenses(&FileNull, source)[0]
                    .correction
                    .is_some()
            );
            let with_nul = format!("{source}; 'NUL'");
            assert_eq!(offenses(&with_nul).len(), 2, "{with_nul}");
        }
    }

    #[test]
    fn conditional_regexps_report_content_and_supply_file_wide_evidence() {
        for (source, col) in [
            (r"if /\/dev\/null/; end", 5),
            (r"unless %r{/DEV/NULL}; end", 11),
            (r"while /\/dev\/null/; end", 8),
            (r"work if /\/dev\/null/", 10),
        ] {
            let source = format!("{source}\n'NUL'");
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!((found[1].line, found[1].col), (2, 1));
            assert_eq!(
                found[0].message,
                format!(
                    "Use `File::NULL` instead of `{}`.",
                    if source.contains("/DEV/NULL") {
                        "/DEV/NULL"
                    } else {
                        "/dev/null"
                    }
                )
            );
            assert_eq!(found[1].message, "Use `File::NULL` instead of `NUL`.");
            assert!(
                correction_offenses(&FileNull, &source)
                    .iter()
                    .all(|o| o.correction.is_some())
            );
        }
        let found = offenses(r#"if /\/dev\/null/; end; "NUL""#);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 5), (1, 24)]
        );
        assert_eq!(offenses("'NUL'\nif %r{/dev/null}; end").len(), 2);
        for source in [
            "if %r{/dev/null/}; end; 'NUL'",
            "if %r{other}; end; 'NUL'",
            "if /NUL/; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn ignores_nonstandalone_literals_and_lookalikes() {
        for source in [
            "\"\"",
            "\"\\xa4\"",
            "path = 'NUL'",
            "'the null device /dev/null'",
            "\"/dev/null#{suffix}\"",
            "\"/dev/null\" \"more\"",
            "['/dev/null', 'NUL']",
            "%w[/dev/null NUL]",
            "{ '/dev/null' => true, 'nul' => false }",
            "{ unix: '/dev/null', windows: 'nul' }",
            "File::NULL",
            "'/dev/null/'",
            "'NUL:foo'",
            "# '/dev/null'\n'NUL'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // File-wide evidence includes ignored arrays and interpolation parts.
        assert_eq!(offenses("x = ['/dev/null']; path = 'NUL'").len(), 1);
        assert_eq!(
            offenses("x = \"/dev/null#{suffix}\"; path = 'NUL'").len(),
            1
        );
    }
}
