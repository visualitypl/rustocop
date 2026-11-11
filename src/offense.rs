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

/// The severity attached to an offense.
///
/// Deliberately coarser than RuboCop's: `Lint`/`Security` offenses are errors,
/// everything else is a convention. rustocop's exit code treats all offenses
/// the same; severity is informational.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A style or convention offense.
    Convention,
    /// A warning.
    Warning,
    /// A likely bug or dangerous pattern.
    Error,
}

impl Severity {
    /// The default severity for a department.
    pub fn for_department(department: Department) -> Self {
        match department {
            Department::Lint | Department::Security => Severity::Error,
            Department::Style
            | Department::Naming
            | Department::Layout
            | Department::Metrics
            | Department::Bundler
            | Department::Gemspec => Severity::Convention,
        }
    }

    /// The severity name, as printed in output.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Convention => "convention",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }

    /// The single-letter severity marker, as printed in text output.
    pub fn letter(self) -> char {
        match self {
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
