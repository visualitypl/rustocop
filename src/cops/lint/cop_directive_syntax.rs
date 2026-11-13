//! `Lint/CopDirectiveSyntax`: validate source directive comments.
use crate::{
    cops::{Cop, NodeContext},
    directives::{
        comments::registered_names,
        syntax::{self, Match},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct CopDirectiveSyntax;
const COMMON: &str = "Malformed directive comment detected.";
const SEPARATORS: &str =
    "Cop names must be separated by commas. Comment in the directive must start with `--`.";
const SIGNED: &str = "`push` and `next` arguments must be `+`- or `-`-prefixed cop names, and `pop` takes no arguments.";
const MULTIPLE: &str = "Only the first directive on a line takes effect. List the cop names in a single directive instead.";

impl Cop for CopDirectiveSyntax {
    fn name(&self) -> &'static str {
        "Lint/CopDirectiveSyntax"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        for range in file.comments() {
            let text = &file.bytes[range.clone()];
            let Some(marker_end) = syntax::marker_end(text) else {
                if let Some(message) = near_miss(text) {
                    let (line, col) = file.line_col(range.start);
                    out.push(Offense {
                        cop: self.name(),
                        severity: self.severity(),
                        message,
                        line,
                        col,
                        end_line: file.line_of(range.end.saturating_sub(1).max(range.start)),
                        correction: None,
                    });
                }
                continue;
            };
            let matches = syntax::matches(text);
            let first = matches.first().filter(|m| m.start == 0);
            let reason = first.is_some_and(|m| {
                let tail = trim(&text[m.end..]);
                tail.starts_with(b"--") && !trim(&tail[2..]).is_empty()
            });
            let multiple = matches.len() > 1
                && (!reason
                    || matches
                        .iter()
                        .skip(1)
                        .any(|m| trim(&text[m.end..]).is_empty()));
            let malformed = if multiple {
                Some(MULTIPLE)
            } else if let Some(m) = first {
                let tail = trim(&text[m.end..]);
                if m.names.is_empty() && tail.is_empty() && !matches!(m.mode, "push" | "pop") {
                    Some("The cop name is missing.")
                } else if (m.mode == "pop" && !m.names.is_empty())
                    || (matches!(m.mode, "push" | "next") && !m.names.is_empty() && !m.signed)
                {
                    Some(SIGNED)
                } else if !tail.is_empty() && !tail.starts_with(b"--") {
                    Some(SEPARATORS)
                } else {
                    None
                }
            } else {
                let mode = trim(&text[marker_end..]);
                Some(if mode.is_empty() {
                    "The mode name is missing."
                } else {
                    syntax::MODES_MESSAGE
                })
            };
            let message = if let Some(message) = malformed {
                Some(format!("{COMMON} {message}"))
            } else if let Some(m) = first {
                if matches!(
                    m.mode,
                    "disable-next" | "todo-next" | "enable-next" | "next"
                ) && !crate::directives::starts_line(file, range.start)
                {
                    Some("A `-next` directive must be on its own line, above the statement it applies to.".into())
                } else {
                    unknown_name(m)
                }
            } else {
                None
            };
            if let Some(message) = message {
                let (line, col) = file.line_col(range.start);
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message,
                    line,
                    col,
                    end_line: file.line_of(range.end.saturating_sub(1).max(range.start)),
                    correction: (multiple && cx.corrections_requested())
                        .then(|| merge(file, range.clone(), &matches))
                        .flatten(),
                });
            }
        }
    }
}

/// Every later marker is inert today. Merging may only add selectors already
/// covered by the first marker, including unselected and configured-off cops.
/// This registry-bounded proof avoids candidate parses and source-wide rescans.
fn merge(
    file: &SourceFile,
    mut range: std::ops::Range<usize>,
    matches: &[Match<'_>],
) -> Option<Correction> {
    let text = &file.bytes[range.clone()];
    let first = matches.first()?;
    if first.start != 0
        || !matches!(
            first.mode,
            "disable" | "todo" | "enable" | "disable-next" | "todo-next" | "enable-next"
        )
    {
        return None;
    }
    if first.mode.ends_with("-next") && !crate::directives::starts_line(file, range.start) {
        return None;
    }
    let covered: Vec<_> = registered_names()
        .iter()
        .filter(|cop| {
            first
                .names
                .iter()
                .any(|name| crate::directives::comments::hits(name, cop))
        })
        .copied()
        .collect();
    let opted_in: Vec<_> = registered_names()
        .iter()
        .filter(|cop| {
            first
                .names
                .iter()
                .any(|name| *name == **cop || Some(*name) == cop.rsplit('/').next())
        })
        .copied()
        .collect();
    let mut names = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, m) in matches.iter().enumerate() {
        if m.mode != first.mode
            && !(matches!(m.mode, "disable" | "todo") && matches!(first.mode, "disable" | "todo"))
        {
            return None;
        }
        if m.signed || m.names.is_empty() || m.names.contains(&"all") {
            return None;
        }
        let to = matches.get(i + 1).map_or(text.len(), |next| next.start);
        if !trim(&text[m.end..to]).is_empty() {
            return None;
        }
        for name in &m.names {
            if i > 0
                && registered_names().iter().any(|cop| {
                    crate::directives::comments::hits(name, cop)
                        && (!covered.contains(cop)
                            || (first.mode == "enable"
                                && (*name == *cop || Some(*name) == cop.rsplit('/').next())
                                && !opted_in.contains(cop)))
                })
            {
                return None;
            }
            if seen.insert(*name) {
                names.push(*name);
            }
        }
    }
    // Adjacency above excludes prose/magic keys. Only ASCII header/list bytes
    // are rewritten, never physical newlines, legacy bytes, BOM or CRLF.
    if text.contains(&b'\n') {
        return None;
    }
    if text.last() == Some(&b'\r') && file.bytes.get(range.end) == Some(&b'\n') {
        range.end -= 1;
    }
    let prefix = if text[..first.end].windows(8).any(|w| w == b"rustocop") {
        "rustocop"
    } else {
        "rubocop"
    };
    Some(Correction {
        range,
        replacement: format!("# {prefix}:{} {}", first.mode, names.join(", ")),
    })
}
fn near_miss(text: &[u8]) -> Option<String> {
    static NEAR: std::sync::LazyLock<regex::bytes::Regex> = std::sync::LazyLock::new(|| {
        regex::bytes::Regex::new(r"\A\#+(?-u:\s*)([A-Za-z](?-u:[\w-]*))(?-u:\s*):(?-u:\s*)(?:disable-next|enable-next|todo-next|disable|enable|todo|next|push|pop)\b").unwrap()
    });
    let captures = NEAR.captures(text)?;
    let keyword = std::str::from_utf8(captures.get(1)?.as_bytes()).ok()?;
    if matches!(keyword, "rubocop" | "rustocop") {
        return None;
    }
    let lower: Vec<_> = keyword.to_lowercase().chars().collect();
    let (distance, expected) = ["rubocop", "rustocop"]
        .into_iter()
        .map(|expected| {
            (
                crate::cops::name_similarity::levenshtein(
                    &lower,
                    &expected.chars().collect::<Vec<_>>(),
                ),
                expected,
            )
        })
        .min()?;
    (distance <= 2)
        .then(|| format!("{COMMON} The directive keyword must be `{expected}`, not `{keyword}`."))
}
fn trim(text: &[u8]) -> &[u8] {
    let whitespace = |b: &u8| b.is_ascii_whitespace() || *b == b'\x0b';
    let start = text
        .iter()
        .position(|b| !whitespace(b))
        .unwrap_or(text.len());
    let end = text
        .iter()
        .rposition(|b| !whitespace(b))
        .map_or(start, |i| i + 1);
    &text[start..end]
}
fn unknown_name(m: &Match<'_>) -> Option<String> {
    if m.mode == "pop" || m.names == ["all"] {
        return None;
    }
    let signed = matches!(m.mode, "push" | "next");
    let name = m
        .names
        .iter()
        .map(|name| if signed { &name[1..] } else { *name })
        .find(|name| !known_name(name))?;
    let similar = crate::cops::name_similarity::find_similar_name(name, registered_names());
    Some(format!(
        "Unknown cop name `{name}`{}.",
        similar
            .map(|s| format!(" (did you mean `{s}`?)"))
            .unwrap_or_default()
    ))
}

/// Validate like the reference registry's Badge/qualified_cop_name, bounded
/// to landed cops. Qualification is diagnostic-only: suppression and merge
/// safety must continue to use the original, literal selectors.
fn known_name(name: &str) -> bool {
    let names = registered_names();
    if names
        .iter()
        .any(|cop| name != "all" && crate::directives::comments::hits(name, cop))
    {
        return true;
    }
    let badge = name
        .split('/')
        .map(camel_case)
        .collect::<Vec<_>>()
        .join("/");
    // The reference returns the original spelling if its parsed badge is
    // already registered; its subsequent membership check is literal.
    if names.contains(&badge.as_str()) {
        return false;
    }
    let short = badge.rsplit('/').next();
    let mut candidates = names.iter().filter(|cop| cop.rsplit('/').next() == short);
    candidates.next().is_some() && candidates.next().is_none()
}

/// RuboCop Badge.camel_case: uppercase an initial lowercase letter and replace
/// underscore + lowercase pairs (not arbitrary underscore or case folding).
pub(crate) fn camel_case(part: &str) -> String {
    let mut result = String::with_capacity(part.len());
    let mut bytes = part.bytes().enumerate().peekable();
    while let Some((i, byte)) = bytes.next() {
        if byte == b'_'
            && bytes
                .peek()
                .is_some_and(|(_, next)| next.is_ascii_lowercase())
        {
            result.push(bytes.next().unwrap().1.to_ascii_uppercase() as char);
        } else if i == 0 {
            result.push(byte.to_ascii_uppercase() as char);
        } else {
            result.push(byte as char);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/cop_directive_syntax_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == CopDirectiveSyntax.name())
            .collect()
    }
    #[test]
    fn malformed_comments_have_exact_positions_and_messages() {
        for prefix in ["rubocop", "rustocop"] {
            for (text, message) in [
                ("", "The mode name is missing."),
                (
                    "disabled Layout/LineLength",
                    "The mode name must be one of `enable`, `disable`, `disable-next`, `enable-next`, `todo`, `todo-next`, `next`, `push`, or `pop`.",
                ),
                ("disable", "The cop name is missing."),
                ("next", "The cop name is missing."),
                ("enable-next", "The cop name is missing."),
                (
                    "disable Layout/LineLength Style/Encoding",
                    "Cop names must be separated by commas. Comment in the directive must start with `--`.",
                ),
                (
                    "disable Layout/LineLength == bad comment",
                    "Cop names must be separated by commas. Comment in the directive must start with `--`.",
                ),
                (
                    "next Layout/LineLength",
                    "`push` and `next` arguments must be `+`- or `-`-prefixed cop names, and `pop` takes no arguments.",
                ),
                (
                    "push Layout/LineLength",
                    "`push` and `next` arguments must be `+`- or `-`-prefixed cop names, and `pop` takes no arguments.",
                ),
                (
                    "pop +Layout/LineLength",
                    "`push` and `next` arguments must be `+`- or `-`-prefixed cop names, and `pop` takes no arguments.",
                ),
            ] {
                let s = format!("foo\n  # {prefix}:{text}\n");
                let found = offenses(&s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col), (2, 3), "{s}");
                assert_eq!(
                    found[0].message,
                    format!("Malformed directive comment detected. {message}"),
                    "{s}"
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn corrections_merge_only_suppression_preserving_adjacent_lists() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        for prefix in ["rubocop", "rustocop"] {
            for mode in [
                "disable",
                "todo",
                "enable",
                "disable-next",
                "todo-next",
                "enable-next",
            ] {
                let before =
                    format!("# {prefix}:{mode} Style/For # {prefix}:{mode} Style/For\nfoo\n");
                let after = format!("# {prefix}:{mode} Style/For\nfoo\n");
                assert_correction(&CopDirectiveSyntax, &before, &after);
            }
            assert_correction(
                &CopDirectiveSyntax,
                &format!(
                    "x = 1 # {prefix}:todo Style/For # rubocop:disable Migration/Nonexistent\n"
                ),
                &format!("x = 1 # {prefix}:todo Style/For, Migration/Nonexistent\n"),
            );
            assert_correction(
                &CopDirectiveSyntax,
                &format!(
                    "# {prefix}:disable Style, Style/For # rubocop:disable Style/Encoding\nfoo\n"
                ),
                &format!("# {prefix}:disable Style, Style/For, Style/Encoding\nfoo\n"),
            );
        }
        for source in [
            // The upstream canonical edit adds a new active suppression.
            "# rubocop:disable Layout/LineLength # rubocop:disable Style/Encoding\n",
            "x = 1 # rustocop:todo Style/For # rubocop:disable Metrics/AbcSize\n",
            // Migration/DepartmentName is a registered cop.
            "x = 1 # rustocop:todo Style/For # rubocop:disable Migration/DepartmentName\n",
            "x = 1 # rubocop:todo Style/For # rubocop:disable Migration/DepartmentName\n",
            "# rubocop:disable Style/For # rubocop:enable Style/For\n",
            "# rubocop:disable Style/For -- keep it # rubocop:disable Style/For\n",
            "# rubocop:disable Style/For # rubocop:disable all\n",
            "# rubocop:enable Style # rubocop:enable Style/For\n",
            "# rubocop:push -Style/For # rubocop:push -Style/For\n",
            "# rubocop:next -Style/For # rubocop:next -Style/For\nfoo\n",
            "foo # rubocop:disable-next Style/For # rubocop:disable-next Style/For\n",
        ] {
            let found = correction_offenses(&CopDirectiveSyntax, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn keyword_typos_are_not_confused_with_project_extensions() {
        for (keyword, expected) in [
            ("rucocop", "rubocop"),
            ("RuboCop", "rubocop"),
            ("robocop", "rubocop"),
            ("Rustocop", "rustocop"),
        ] {
            let s = format!("a = 1 # {keyword}:disable Layout/LineLength\n");
            let found = offenses(&s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 7));
            assert_eq!(
                found[0].message,
                format!("{COMMON} The directive keyword must be `{expected}`, not `{keyword}`.")
            );
        }
    }
    #[test]
    fn names_placement_and_stacked_markers() {
        for mode in [
            "disable",
            "enable",
            "disable-next",
            "enable-next",
            "next -",
            "push -",
        ] {
            let s = format!(
                "# rubocop:{mode}{}Layout/LineLenght\nfoo\n",
                if mode.ends_with('-') { "" } else { " " }
            );
            let found = offenses(&s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!(
                found[0].message,
                "Unknown cop name `Layout/LineLenght` (did you mean `Layout/LineLength`?)."
            );
        }
        assert_eq!(
            offenses("# rubocop:disable Layout/LineLength, Style/Encodingg\n")[0].message,
            "Unknown cop name `Style/Encodingg` (did you mean `Style/Encoding`?)."
        );
        assert!(offenses("# rubocop:disable Style/FrozenStringLiteralComment\n").is_empty());
        // Names are the actual landed registry, not a future-cop whitelist.
        assert!(
            offenses("# rubocop:disable Migration/Nonexistent\n")[0]
                .message
                .starts_with("Unknown cop name")
        );
        assert!(offenses("# rubocop:disable Metrics/AbcSize\n").is_empty());
        assert!(offenses("# rubocop:disable Migration/DepartmentName\n").is_empty());
        assert!(offenses("# rubocop:disable Migration\n").is_empty());
        for mode in ["disable-next", "todo-next", "enable-next", "next -"] {
            let s = format!(
                "foo # rustocop:{mode}{}Layout/LineLength\nbar\n",
                if mode.ends_with('-') { "" } else { " " }
            );
            let found = offenses(&s);
            assert_eq!((found[0].line, found[0].col), (1, 5));
            assert_eq!(
                found[0].message,
                "A `-next` directive must be on its own line, above the statement it applies to."
            );
        }
        for s in [
            "# rubocop:disable Layout/LineLength # rubocop:disable Style/Encoding\n",
            "x = 1 # rustocop:todo Style/For # rubocop:disable Style/Encoding\n",
            "# rubocop:disable Style/For -- keep it # rubocop:disable Style/Encoding\n",
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!(found[0].message, format!("{COMMON} {MULTIPLE}"));
        }
    }
    #[test]
    fn name_validation_qualifies_landed_cops_without_changing_suppression() {
        for prefix in ["rubocop", "rustocop"] {
            for name in [
                "Style/LineLength",
                "line_length",
                "Wrong/line_length",
                "Wrong/frozen_string_literal_comment",
                "Wrong/AbcSize",
                "abc_size",
                "Wrong/DepartmentName",
                "department_name",
            ] {
                for mode in [
                    "disable",
                    "enable",
                    "todo",
                    "disable-next",
                    "next -",
                    "push +",
                ] {
                    let separator = if mode.ends_with(['-', '+']) { "" } else { " " };
                    let source = format!("# {prefix}:{mode}{separator}{name}\nfoo\n");
                    assert!(offenses(&source).is_empty(), "{source}");
                }
                assert!(!crate::directives::comments::hits(
                    name,
                    "Layout/LineLength"
                ));
            }
        }
        for name in [
            "Wrong/Nonexistent",
            "nonexistent",
            "LINE_LENGTH",
            "layout/line_length",
        ] {
            let source = format!("# rubocop:disable {name}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(
                found[0]
                    .message
                    .starts_with(&format!("Unknown cop name `{name}`"))
            );
        }
    }
    #[test]
    fn whitespace_unicode_boundaries_and_empty_files() {
        for s in [
            "",
            "# ordinary\n",
            "#\u{a0}rucocop:disable Style/For\n",
            "# rucocop\u{a0}:disable Style/For\n",
            "# rucocop:\u{a0}disable Style/For\n",
            "#\u{b}rustocop\u{b}:\u{b}disable\u{b}Style/For\u{b}\n",
            "a = 1 # rubocop:disable Style/For -- reason\n",
            "# rubocop:disable Layout/LineLength -- see # rubocop:disable Style/Encoding in old branch\n",
        ] {
            assert!(offenses(s).is_empty(), "{s:?}");
        }
        for suffix in ["é", "中", "\u{301}", "_", "2"] {
            let s = format!("# rustocop:disable{suffix} Style/For\n");
            assert_eq!(
                offenses(&s)[0].message,
                format!("{COMMON} {}", syntax::MODES_MESSAGE)
            );
        }
    }
    #[test]
    fn valid_comments_and_non_directive_lookalikes() {
        for prefix in ["rubocop", "rustocop"] {
            for text in [
                "disable Layout/LineLength",
                "disable Layout",
                "disable Layout/LineLength, Style/Encoding",
                "disable all",
                "enable LineLength",
                "todo Layout/LineLength",
                "push",
                "pop",
                "next +Layout/LineLength -Style/For -- tradeoff",
                "push -Lint/Debugger +Style/For",
                "disable-next Layout/LineLength",
                "todo-next Layout/LineLength -- reason",
                "enable-next Layout/LineLength",
                "disable Layout/LineLength -- This is a good comment.",
                "disable Layout/LineLength -- see # rubocop:disable Style/Encoding in old branch",
            ] {
                let s = format!("# {prefix}:{text}\nfoo\n");
                assert!(offenses(&s).is_empty(), "{s}");
            }
        }
        for s in [
            "# # rubocop:disable Layout/LineLength\n",
            "# \"rubocop:disable Layout/LineLength\"\n",
            "# pylint: disable=foo\n",
            "# TODO: disable eventually\n",
            "x = '# rubocop:disabled'\n",
            "x = <<~H\n# rubocop:disabled\nH\n",
            "__END__\n# rubocop:disabled\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
