//! Offense model: departments, severities, and the offenses cops report.

use std::fmt;

/// The cop departments rustocop ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Department {
    /// Safety checks that flag likely bugs.
    Lint,
    /// Idiomatic Ruby style, with opinionated defaults.
    Style,
    /// Naming conventions.
    Naming,
    /// Checks for potentially dangerous API usage.
    Security,
    /// Formatting concerns (only line length in v0.1).
    Layout,
    /// Method and block complexity metrics.
    Metrics,
    /// Bundler manifest and lockfile conventions.
    Bundler,
    /// Ruby gem specification conventions.
    Gemspec,
    /// Migration aids for directive comments.
    Migration,
}

impl Department {
    /// All departments.
    pub const ALL: &'static [Department] = &[
        Department::Lint,
        Department::Style,
        Department::Naming,
        Department::Security,
        Department::Layout,
        Department::Metrics,
        Department::Bundler,
        Department::Gemspec,
        Department::Migration,
    ];

    /// The department's name, as used in cop names and config keys.
    pub fn as_str(self) -> &'static str {
        match self {
            Department::Lint => "Lint",
            Department::Style => "Style",
            Department::Naming => "Naming",
            Department::Security => "Security",
            Department::Layout => "Layout",
            Department::Metrics => "Metrics",
            Department::Bundler => "Bundler",
            Department::Gemspec => "Gemspec",
            Department::Migration => "Migration",
        }
    }

    /// Parse a department name (case-sensitive).
    pub fn parse(s: &str) -> Option<Self> {
        Department::ALL.iter().copied().find(|d| d.as_str() == s)
    }
}

impl fmt::Display for Department {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The severity attached to an offense, as RuboCop names them. rustocop's
/// exit code treats all offenses the same; severity is informational.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Code that works but could be structured better (`Metrics`).
    Refactor,
    /// A style or convention offense.
    Convention,
    /// A warning.
    Warning,
    /// An error.
    Error,
}

impl Severity {
    /// RuboCop's default severity for a cop: `warning` for `Lint` and
    /// `Security` and the cops its `default.yml` marks so, `refactor` for
    /// `Metrics`, `convention` otherwise.
    pub fn for_cop(cop_name: &str) -> Self {
        const WARNINGS: &[&str] = &[
            "Bundler/DuplicatedGem",
            "Bundler/DuplicatedGroup",
            "Bundler/InsecureProtocolSource",
            "Gemspec/DeprecatedAttributeAssignment",
            "Gemspec/DuplicatedAssignment",
            "Gemspec/RequireMFA",
            "Gemspec/RequiredRubyVersion",
            "Gemspec/RubyVersionGlobalsUsage",
            "Layout/BeginEndAlignment",
            "Layout/DefEndAlignment",
            "Layout/EndAlignment",
        ];
        match department_of(cop_name) {
            Department::Lint | Department::Security => Severity::Warning,
            Department::Metrics => Severity::Refactor,
            _ if WARNINGS.contains(&cop_name) => Severity::Warning,
            _ => Severity::Convention,
        }
    }

    /// The severity name, as printed in output.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Refactor => "refactor",
            Severity::Convention => "convention",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }

    /// The single-letter severity marker, as printed in text output.
    pub fn letter(self) -> char {
        match self {
            Severity::Refactor => 'R',
            Severity::Convention => 'C',
            Severity::Warning => 'W',
            Severity::Error => 'E',
        }
    }
}

/// A replacement of a half-open byte range in the original source snapshot.
/// Empty ranges insert text; an empty replacement deletes the range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    /// Byte offsets into the source, start inclusive and end exclusive.
    pub range: std::ops::Range<usize>,
    /// Replacement source text.
    pub replacement: String,
}

/// A single reported offense.
#[derive(Debug, Clone)]
pub struct Offense {
    /// Optional correction, applied only when its cop declares safe fixes.
    pub correction: Option<Correction>,
    /// The full cop name, e.g. `Lint/Debugger`.
    pub cop: &'static str,
    /// The severity of the offense.
    pub severity: Severity,
    /// Human-readable description of the offense.
    pub message: String,
    /// 1-based first line of the offense highlight.
    pub line: u32,
    /// 1-based inclusive last line of the highlight (not the enclosing node).
    /// Equal to `line` for a single-line offense.
    pub end_line: u32,
    /// 1-based (character) column of the offense.
    pub col: u32,
}

/// A nonfatal finding about the whole supplied file, not a source range.
///
/// Deliberately has neither positions nor a correction: filename conventions
/// and missing gemspec declarations cannot claim a source highlight or repair.
#[derive(Debug, Clone)]
pub struct GlobalOffense {
    /// The full cop name.
    pub cop: &'static str,
    /// The severity of the finding.
    pub severity: Severity,
    /// Human-readable description.
    pub message: String,
}

impl Correction {
    /// One correction from several edits of the original source, as
    /// RuboCop's corrector combines them: an empty range is an insertion,
    /// placed before a replacement that starts at the same offset, and
    /// insertions at one offset keep the given order. `None` when edits
    /// overlap (upstream's clobbering error) or the result is not UTF-8.
    pub fn from_edits(
        source: &[u8],
        mut edits: Vec<(std::ops::Range<usize>, String)>,
    ) -> Option<Correction> {
        edits.retain(|(range, text)| !(range.is_empty() && text.is_empty()));
        edits.sort_by_key(|(range, _)| (range.start, !range.is_empty()));
        let start = edits.first()?.0.start;
        let mut replacement = Vec::new();
        let mut cursor = start;
        for (range, text) in &edits {
            if range.start < cursor {
                return None;
            }
            replacement.extend_from_slice(&source[cursor..range.start]);
            replacement.extend_from_slice(text.as_bytes());
            cursor = range.end;
        }
        Some(Correction {
            range: start..cursor,
            replacement: String::from_utf8(replacement).ok()?,
        })
    }
}

/// RuboCop's `range_with_surrounding_space(side: :right)` end: spaces and
/// tabs, then line breaks.
pub fn space_right(source: &[u8], mut pos: usize) -> usize {
    while matches!(source.get(pos), Some(b' ' | b'\t')) {
        pos += 1;
    }
    while source.get(pos) == Some(&b'\n') {
        pos += 1;
    }
    pos
}

/// RuboCop's `range_with_surrounding_space(side: :right, newlines: false)`
/// end: spaces and tabs only.
pub fn space_right_blank(source: &[u8], mut pos: usize) -> usize {
    while matches!(source.get(pos), Some(b' ' | b'\t')) {
        pos += 1;
    }
    pos
}

/// RuboCop's `range_with_surrounding_space(side: :left)` start: spaces and
/// tabs, then line breaks, before `pos`.
pub fn space_left(source: &[u8], mut pos: usize) -> usize {
    while pos > 0 && matches!(source[pos - 1], b' ' | b'\t') {
        pos -= 1;
    }
    while pos > 0 && source[pos - 1] == b'\n' {
        pos -= 1;
    }
    pos
}

/// Source text quoted in a message, on one line: line breaks become
/// spaces, as RuboCop's formatters print them.
pub fn single_line(source: &[u8]) -> String {
    String::from_utf8_lossy(source)
        .replace("\r\n", " ")
        .replace('\n', " ")
}

/// The department part of a full cop name (`Lint/Debugger` -> `Lint`).
///
/// Unknown departments resolve to `Lint`; this is only a fallback for offense
/// rendering — config validation never uses it.
pub fn department_of(cop_name: &str) -> Department {
    cop_name
        .split('/')
        .next()
        .and_then(Department::parse)
        .unwrap_or(Department::Lint)
}

#[cfg(test)]
mod tests {
    use super::Severity;

    #[test]
    fn severities_follow_rubocops_defaults() {
        for (cop, severity) in [
            ("Lint/Debugger", Severity::Warning),
            ("Security/Eval", Severity::Warning),
            ("Metrics/AbcSize", Severity::Refactor),
            ("Style/StringLiterals", Severity::Convention),
            ("Layout/EndAlignment", Severity::Warning),
            ("Layout/IndentationWidth", Severity::Convention),
            ("Gemspec/RequireMFA", Severity::Warning),
            ("Bundler/OrderedGems", Severity::Convention),
        ] {
            assert_eq!(Severity::for_cop(cop), severity, "{cop}");
        }
        assert_eq!(Severity::Refactor.letter(), 'R');
        assert_eq!(Severity::Warning.as_str(), "warning");
    }
}
