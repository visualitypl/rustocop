//! `Style/CharacterLiteral` — prefer strings to short character literals.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct CharacterLiteral;
impl Cop for CharacterLiteral {
    fn name(&self) -> &'static str {
        "Style/CharacterLiteral"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_string_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(string) = node.as_string_node() else {
            return;
        };
        if !string.opening_loc().is_some_and(|l| l.as_slice() == b"?") {
            return;
        }
        let loc = string.location();
        let size = file
            .character_offsets(loc.start_offset()..loc.end_offset())
            .take(4)
            .count();
        if !(2..=3).contains(&size) {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| {
                let body = std::str::from_utf8(&loc.as_slice()[1..]).ok()?;
                // Physical line escapes have different character/string grammars.
                if body.contains(['\r', '\n']) {
                    return None;
                }
                let quote = if body.chars().count() == 2 || body == "'" {
                    '"'
                } else {
                    '\''
                };
                let correction = Correction {
                    range: loc.start_offset()..loc.end_offset(),
                    replacement: format!("{quote}{body}{quote}"),
                };
                super::literal_spelling::safe_edit(file, cx, correction)
            })
            .flatten();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not use the character literal - use string literal instead.".into(),
            line,
            col,
            end_line: file.last_line_of(loc),
            correction,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/character_literal_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    #[test]
    fn pinned_short_character_diagnostic() {
        for source in ["x = ?x", r"x = ?\n", "x = ?'"] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == CharacterLiteral.name())
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 5, 1));
            assert_eq!(
                found[0].message,
                "Do not use the character literal - use string literal instead."
            );
        }
    }

    #[test]
    fn pinned_corrections() {
        for (before, after) in [
            ("x = ?x", "x = 'x'"),
            (r"x = ?\n", r#"x = "\n""#),
            ("x = ?'", "x = \"'\""),
        ] {
            assert_correction(&CharacterLiteral, before, after);
        }
    }

    #[test]
    fn long_escapes_and_literal_lookalikes_are_accepted() {
        for source in [
            r"x = ?\C-\M-d",
            "%w{? A}",
            r"?\u0041",
            r"?\u{1f600}",
            r"?\123",
            r"?\M-a",
            "x ? y : z",
            "\"?x\"",
            ":'?x'",
            "`?x`",
        ] {
            assert!(
                correction_offenses(&CharacterLiteral, source).is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn unicode_quotes_backslashes_and_simple_escapes() {
        for (before, after) in [
            ("?é", "'é'"),
            ("?😀", "'😀'"),
            ("?\"", "'\"'"),
            (r"?\\", r#""\\""#),
            (r"?\t", r#""\t""#),
            (r"?\s", r#""\s""#),
            (r"?\'", r#""\'""#),
            (r"?\#", r##""\#""##),
        ] {
            assert_correction(&CharacterLiteral, before, after);
        }
    }
}
