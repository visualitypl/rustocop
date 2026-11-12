//! `Migration/DepartmentName`: cop names in directive comments must carry
//! their department (`# rubocop:disable Style/Alias`, not `Alias`).
//!
//! A port of RuboCop 1.92.0's `on_new_investigation` (MIT, Copyright (c)
//! 2012-26 Bozhidar Batsov). Every Ruby comment is inspected, so it runs on
//! comment-only and empty programs too. The pinned `DISABLE_COMMENT_FORMAT`
//! is matched at the start of the comment for both accepted directive
//! spellings (`rubocop` and rustocop's own `rustocop`), with the original
//! grammar otherwise: spaces only, `disable`/`enable`/`todo` modes.
//!
//! The correction prefixes the department found in the actual registry
//! (after RuboCop's badge camel-casing), else the pinned legacy cop name.
//! rustocop's directives match names literally, so the edit is emitted only
//! when the qualified name selects exactly the registered cops the original
//! token selects, for suppression and for source opt-in. Ambiguous,
//! case-variant and unknown names keep their finding and write nothing.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    directives::comments::{hits, registered_names},
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Migration/DepartmentName` cop.
pub struct DepartmentName;

const MSG: &str = "Department name is missing.";

/// `RuboCop::ConfigObsoletion.legacy_cop_names` at the pinned revision: the
/// old names of the `renamed`, `removed`, `split` and `extracted` rules of
/// `config/obsoletion.yml`, in rule order (the first match wins).
const LEGACY_COP_NAMES: [&str; 68] = [
    "Layout/AlignArguments",
    "Layout/AlignArray",
    "Layout/AlignHash",
    "Layout/AlignParameters",
    "Layout/IndentArray",
    "Layout/IndentAssignment",
    "Layout/IndentFirstArgument",
    "Layout/IndentFirstArrayElement",
    "Layout/IndentFirstHashElement",
    "Layout/IndentFirstParameter",
    "Layout/IndentHash",
    "Layout/IndentHeredoc",
    "Layout/LeadingBlankLines",
    "Layout/Tab",
    "Layout/TrailingBlankLines",
    "Lint/BlockAlignment",
    "Lint/DefEndAlignment",
    "Lint/DuplicatedKey",
    "Lint/EndAlignment",
    "Lint/EndInMethod",
    "Lint/Eval",
    "Lint/HandleExceptions",
    "Lint/MultipleCompare",
    "Lint/StringConversionInInterpolation",
    "Lint/UnneededCopDisableDirective",
    "Lint/UnneededCopEnableDirective",
    "Lint/UnneededRequireStatement",
    "Lint/UnneededSplatExpansion",
    "Metrics/LineLength",
    "Naming/PredicateName",
    "Naming/UncommunicativeBlockParamName",
    "Naming/UncommunicativeMethodParamName",
    "Style/AccessorMethodName",
    "Style/AsciiIdentifiers",
    "Style/ClassAndModuleCamelCase",
    "Style/ConstantName",
    "Style/DeprecatedHashMethods",
    "Style/FileName",
    "Style/FlipFlop",
    "Style/MethodCallParentheses",
    "Style/MethodName",
    "Style/OpMethod",
    "Style/PredicateName",
    "Style/SingleSpaceBeforeFirstArg",
    "Style/UnneededCapitalW",
    "Style/UnneededCondition",
    "Style/UnneededInterpolation",
    "Style/UnneededPercentQ",
    "Style/UnneededSort",
    "Style/VariableName",
    "Style/VariableNumber",
    "Gemspec/DateAssignment",
    "Layout/SpaceAfterControlKeyword",
    "Layout/SpaceBeforeModifierKeyword",
    "Lint/InvalidCharacterLiteral",
    "Lint/RescueWithoutErrorClass",
    "Lint/SpaceBeforeFirstArg",
    "Lint/UselessComparison",
    "Style/BracesAroundHashParameters",
    "Style/DoubleCopDisableDirective",
    "Style/MethodMissingSuper",
    "Style/SpaceAfterControlKeyword",
    "Style/SpaceBeforeModifierKeyword",
    "Style/TrailingComma",
    "Style/TrailingCommaInLiteral",
    "Style/MethodMissing",
    "Performance/*",
    "Rails/*",
];

impl Cop for DepartmentName {
    fn name(&self) -> &'static str {
        "Migration/DepartmentName"
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
        // RuboCop's on_new_investigation: once per file, whatever the AST.
        if node.as_program_node().is_none() {
            return;
        }
        // The closed list of source observers holds every edit.
        let mut observed = None;
        let mut listed_observer = || {
            *observed.get_or_insert_with(|| {
                file.class_method_bindings(node)
                    .has_listed_source_observer(file.data_start().is_some())
            })
        };
        for range in file.comments() {
            let text = &file.bytes[range.clone()];
            let Some(header) = header_len(text) else {
                continue;
            };
            let names = &text[header..];
            let mut position = 0;
            // `scan(/[^,]+|\W+/)`: a run up to the next comma, or at a comma
            // a run of non-word bytes.
            while position < names.len() {
                let rest = &names[position..];
                let len = if rest[0] == b',' {
                    rest.iter().take_while(|&&b| !is_word(b)).count()
                } else {
                    rest.iter().take_while(|&&b| b != b',').count()
                };
                let piece = &rest[..len];
                let trimmed = ruby_strip(piece);
                if !valid_content_token(trimmed) {
                    // Upstream starts the range at the untrimmed piece and
                    // gives it the trimmed length.
                    let start = range.start + header + position;
                    let end = start + trimmed.len();
                    let (line, col) = file.line_col(start);
                    out.push(Offense {
                        cop: self.name(),
                        severity: self.severity(),
                        message: MSG.into(),
                        line,
                        end_line: line,
                        col,
                        correction: cx
                            .corrections_requested()
                            .then(|| qualify(&file.bytes[start..end]))
                            .flatten()
                            .filter(|_| !listed_observer())
                            .map(|replacement| Correction {
                                range: start..end,
                                replacement,
                            }),
                    });
                }
                if piece
                    .iter()
                    .any(|&b| !(b.is_ascii_alphabetic() || matches!(b, b'/' | b',' | b' ')))
                {
                    break;
                }
                position += len;
            }
        }
    }
}

/// The length of `# *(rubocop|rustocop) *: *((dis|en)able|todo) +` at the
/// start of `text` (`DISABLE_COMMENT_FORMAT`'s first group).
fn header_len(text: &[u8]) -> Option<usize> {
    let spaces = |text: &[u8]| text.iter().take_while(|&&b| b == b' ').count();
    let mut position = 1 + spaces(text.strip_prefix(b"#")?);
    let rest = &text[position..];
    position += ["rubocop", "rustocop"]
        .into_iter()
        .find(|prefix| rest.starts_with(prefix.as_bytes()))?
        .len();
    position += spaces(&text[position..]);
    if text.get(position) != Some(&b':') {
        return None;
    }
    position += 1;
    position += spaces(&text[position..]);
    let rest = &text[position..];
    position += ["disable", "enable", "todo"]
        .into_iter()
        .find(|mode| rest.starts_with(mode.as_bytes()))?
        .len();
    let blanks = spaces(&text[position..]);
    (blanks > 0).then_some(position + blanks)
}

/// Ruby's `\w`, which is ASCII-only.
fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Ruby's `String#strip`: NUL and ASCII whitespace on both ends.
fn ruby_strip(text: &[u8]) -> &[u8] {
    let blank = |b: &u8| matches!(b, b'\0' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' | b' ');
    let start = text.iter().position(|b| !blank(b)).unwrap_or(text.len());
    let end = text
        .iter()
        .rposition(|b| !blank(b))
        .map_or(start, |i| i + 1);
    &text[start..end]
}

/// `valid_content_token?`: a non-word character, the unanchored
/// `[A-Za-z]+/[A-Za-z]+|all` token pattern, or a department of the actual
/// registry.
fn valid_content_token(token: &[u8]) -> bool {
    token.iter().any(|&b| !is_word(b))
        || token.windows(3).any(|w| w == b"all")
        || registered_names()
            .iter()
            .any(|cop| cop.split('/').next().map(str::as_bytes) == Some(token))
}

/// The upstream replacement for the highlighted source, kept only when it
/// selects exactly what the original token selects.
fn qualify(source: &[u8]) -> Option<String> {
    let name = std::str::from_utf8(source).ok()?;
    let qualified = qualified_cop_name(name)?;
    selects_same_cops(name, &qualified).then_some(qualified)
}

/// `Registry#qualified_cop_name(name, nil, warn: false)` for a single,
/// department-less name, falling back to `qualified_legacy_cop_name`.
/// `None` where upstream has no department to add (its replacement is
/// `nil`, an empty edit) or raises `AmbiguousCopName`.
fn qualified_cop_name(name: &str) -> Option<String> {
    let badge = if name == "rspec" {
        "RSpec".to_owned()
    } else {
        crate::cops::lint::cop_directive_syntax::camel_case(name)
    };
    let mut candidates = registered_names()
        .iter()
        .filter(|cop| cop.rsplit('/').next() == Some(badge.as_str()));
    match (candidates.next(), candidates.next()) {
        (Some(cop), None) => Some((*cop).to_owned()),
        (Some(_), Some(_)) => None,
        (None, _) => LEGACY_COP_NAMES
            .iter()
            .find(|legacy| legacy.split('/').nth(1) == Some(name))
            .map(|legacy| (*legacy).to_owned()),
    }
}

/// Whether directive name `new` selects the same registered cops as `old`,
/// both for suppression ranges and for the source opt-in of `enable`.
fn selects_same_cops(old: &str, new: &str) -> bool {
    let opts_in = |name: &str, cop: &str| name == cop || Some(name) == cop.rsplit('/').next();
    registered_names()
        .iter()
        .all(|cop| hits(old, cop) == hits(new, cop) && opts_in(old, cop) == opts_in(new, cop))
}

#[cfg(test)]
mod tests;
