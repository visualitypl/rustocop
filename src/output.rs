//! Output rendering: human text, GitHub Actions annotations, a per-cop
//! offense count and RuboCop's JSON.

use std::{
    collections::{BTreeMap, HashSet},
    fmt::Write as _,
    path::Path,
};

use serde::Serialize;

use crate::{cops::Registry, pipeline::RunReport};

/// The supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// Human-readable, RuboCop-inspired text.
    Text,
    /// GitHub Actions workflow commands (`::error ...`).
    Github,
    /// The number of offenses per cop, most first.
    Offenses,
    /// JSON in the schema of RuboCop's `--format json`.
    Json,
}

/// Render the report in RuboCop-inspired text format:
///
/// ```text
/// path/to/file.rb:8:5: E: Remove debugger entry point `binding.pry`. (Lint/Debugger)
///
/// 2 files inspected, 1 offense detected
/// ```
pub fn render_text(report: &RunReport) -> String {
    let mut out = String::new();
    for result in &report.results {
        for offense in &result.global_offenses {
            let _ = writeln!(
                out,
                "{}: {}: {} ({})",
                result.path.display(),
                offense.severity.letter(),
                offense.message,
                offense.cop
            );
        }
        for offense in &result.offenses {
            let _ = writeln!(
                out,
                "{}:{}:{}: {}: {} ({})",
                result.path.display(),
                offense.line,
                offense.col,
                offense.severity.letter(),
                offense.message,
                offense.cop
            );
        }
    }
    push_errors_and_summary(&mut out, report);
    out
}

/// Render the number of offenses per cop, most first and by name within one
/// count, then the file errors and the summary line of [`render_text`]:
///
/// ```text
/// 53363  Style/StringLiterals
///   933  Style/FormatStringToken
///
/// 1793 files inspected, 54296 offenses detected
/// ```
pub fn render_offenses(report: &RunReport) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for result in &report.results {
        let located = result.offenses.iter().map(|offense| offense.cop);
        let global = result.global_offenses.iter().map(|offense| offense.cop);
        for cop in located.chain(global) {
            *counts.entry(cop).or_default() += 1;
        }
    }
    // The map is in name order and the sort is stable.
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    let width = counts
        .first()
        .map_or(0, |(_, count)| count.to_string().len());
    let mut out = String::new();
    for (cop, count) in counts {
        let _ = writeln!(out, "{count:>width$}  {cop}");
    }
    push_errors_and_summary(&mut out, report);
    out
}

/// Render the report as one line of JSON in the schema of RuboCop's
/// `--format json`, for the editor plugins and tools that read that:
///
/// ```json
/// {"files":[{"offenses":[{"cop_name":"Lint/Debugger","correctable":false,
///   "corrected":false,"location":{"column":5,"last_column":5,"last_line":8,
///   "length":1,"line":8,"start_column":5,"start_line":8},
///   "message":"Remove …","severity":"error"}],"path":"app/a.rb"}],
///  "metadata":{"rustocop_version":"1.1.0"},
///  "summary":{"inspected_file_count":1,"offense_count":1,"target_file_count":1}}
/// ```
///
/// Where the schema asks for more than a report holds:
///
/// - An offense records where it starts and not where it ends, so each
///   location is the one character at the start.
/// - A whole-file finding is at line 1, column 1, where RuboCop reports the
///   same cops.
/// - `correctable` says that the cop has corrections, safe or unsafe, not
///   that this offense has one.
/// - A syntax error is an offense of `Lint/Syntax` with the severity
///   `fatal`, as in RuboCop. Any other file error, and each internal error,
///   is a string in the `errors` of its file and counts in `error_count` of
///   the summary. Both keys are absent when there are no such errors.
pub fn render_json(report: &RunReport, registry: &Registry) -> String {
    // Typed and borrowed, not a `serde_json::Value` tree: a tree of a million
    // offenses takes gigabytes. Fields are in alphabetical order, the order
    // of RuboCop's keys in rustocop's output since 1.0.
    #[derive(Serialize)]
    struct Output<'a> {
        files: Vec<File<'a>>,
        metadata: Metadata,
        summary: Summary,
    }
    #[derive(Serialize)]
    struct Metadata {
        rustocop_version: &'static str,
    }
    #[derive(Serialize)]
    struct Summary {
        #[serde(skip_serializing_if = "Option::is_none")]
        error_count: Option<usize>,
        inspected_file_count: usize,
        offense_count: usize,
        target_file_count: usize,
    }
    #[derive(Serialize)]
    struct File<'a> {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        errors: Vec<String>,
        offenses: Vec<JsonOffense<'a>>,
        path: String,
    }
    #[derive(Serialize)]
    struct JsonOffense<'a> {
        cop_name: &'a str,
        correctable: bool,
        corrected: bool,
        location: Location,
        message: &'a str,
        severity: &'a str,
    }
    #[derive(Serialize)]
    struct Location {
        column: u32,
        last_column: u32,
        last_line: u32,
        length: u32,
        line: u32,
        start_column: u32,
        start_line: u32,
    }

    let cops = registry.cops().iter();
    let correctable: HashSet<&str> = cops
        .filter(|cop| cop.safe_fixes() || cop.unsafe_fixes())
        .map(|cop| cop.name())
        .collect();
    let offense = |severity, message, cop, (line, column): (u32, u32)| JsonOffense {
        cop_name: cop,
        correctable: correctable.contains(cop),
        corrected: false,
        location: Location {
            column,
            last_column: column,
            last_line: line,
            length: 1,
            line,
            start_column: column,
            start_line: line,
        },
        message,
        severity,
    };

    // The offenses and the errors of each file, in path order.
    let mut files: BTreeMap<&Path, (Vec<JsonOffense<'_>>, Vec<String>)> = BTreeMap::new();
    for result in &report.results {
        let (offenses, _) = files.entry(&result.path).or_default();
        for found in &result.global_offenses {
            let severity = found.severity.as_str();
            offenses.push(offense(severity, &found.message, found.cop, (1, 1)));
        }
        for found in &result.offenses {
            let (severity, start) = (found.severity.as_str(), (found.line, found.col));
            offenses.push(offense(severity, &found.message, found.cop, start));
        }
    }
    for error in &report.errors {
        let (offenses, errors) = files.entry(&error.path).or_default();
        match error.position {
            Some(start) => {
                // The text formats name the cop in the message.
                let message = error.message.strip_suffix(" (Lint/Syntax)");
                let message = message.unwrap_or(&error.message);
                offenses.push(offense("fatal", message, "Lint/Syntax", start));
            }
            None => errors.push(error.message.clone()),
        }
    }
    for error in &report.internal_errors {
        let (_, errors) = files.entry(&error.path).or_default();
        errors.push(error.to_string());
    }

    let (mut offense_count, mut error_count) = (0, 0);
    let files = files
        .into_iter()
        .map(|(path, (offenses, errors))| {
            offense_count += offenses.len();
            error_count += errors.len();
            File {
                errors,
                offenses,
                path: path.display().to_string(),
            }
        })
        .collect();
    let output = Output {
        files,
        metadata: Metadata {
            rustocop_version: env!("CARGO_PKG_VERSION"),
        },
        summary: Summary {
            error_count: (error_count > 0).then_some(error_count),
            inspected_file_count: report.files_inspected,
            offense_count,
            target_file_count: report.files_inspected,
        },
    };
    let mut out = serde_json::to_string(&output).expect("JSON of strings and numbers");
    out.push('\n');
    out
}

/// The end of the text formats: one line per file error, a blank line and
/// the summary line.
fn push_errors_and_summary(out: &mut String, report: &RunReport) {
    for error in &report.errors {
        let _ = writeln!(out, "{}: error: {}", error.path.display(), error.message);
    }
    out.push('\n');
    out.push_str(&summary_line(report));
    out.push('\n');
}

/// Render the report as GitHub Actions error annotations, plus a summary line.
///
/// Offenses come first, then one annotation per file error (read failure or
/// syntax error), titled `rustocop`, at the error's line and column when it
/// has one and on the whole file otherwise:
///
/// ```text
/// ::error file=app/a.rb,line=8,col=5,title=Lint/Debugger::app/a.rb:8:5: Lint/Debugger: Remove …
/// ::error file=lib/b.rb,line=3,col=7,title=rustocop::lib/b.rb: error: syntax error on line 3: …
/// ::error file=lib/c.rb,title=rustocop::lib/c.rb: error: could not read file: …
/// 3 files inspected, 1 offense detected, 2 errors
/// ```
///
/// Internal errors are not annotated: they are rustocop bugs, not problems in
/// the linted code, and go to stderr in every format.
///
/// Property values (`file`, `title`) and the message data are escaped as the
/// workflow-command syntax requires, so a newline in a message or a comma in a
/// path cannot end the annotation or split a property early.
pub fn render_github(report: &RunReport) -> String {
    let mut out = String::new();
    for result in &report.results {
        let path = result.path.display().to_string();
        let file = escape_property(&path);
        for offense in &result.global_offenses {
            let title = escape_property(offense.cop);
            let data = escape_data(&format!("{path}: {}: {}", offense.cop, offense.message));
            let _ = writeln!(out, "::error file={file},title={title}::{data}");
        }
        for offense in &result.offenses {
            let title = escape_property(offense.cop);
            let data = escape_data(&format!(
                "{path}:{}:{}: {}: {}",
                offense.line, offense.col, offense.cop, offense.message
            ));
            let _ = writeln!(
                out,
                "::error file={file},line={},col={},title={title}::{data}",
                offense.line, offense.col,
            );
        }
    }
    for error in &report.errors {
        let path = error.path.display().to_string();
        let file = escape_property(&path);
        let position = error
            .position
            .map(|(line, col)| format!(",line={line},col={col}"))
            .unwrap_or_default();
        let data = escape_data(&format!("{path}: error: {}", error.message));
        let _ = writeln!(out, "::error file={file}{position},title=rustocop::{data}");
    }
    out.push_str(&summary_line(report));
    out.push('\n');
    out
}

/// Render the run's internal errors (caught panics) for stderr: one line per
/// error naming the file, the cop and the panic, each followed by its
/// backtrace when one was captured, then a bug-report hint. Empty when there
/// are none.
///
/// ```text
/// app/a.rb:3:7: internal error: Lint/Void panicked: index out of bounds: … (at src/cops/lint/void.rs:12:5)
/// note: internal errors are bugs in rustocop 0.1.0; please report them with the code that triggers them
/// note: run with RUST_BACKTRACE=1 to include a backtrace
/// ```
pub fn render_internal_errors(report: &RunReport) -> String {
    let mut out = String::new();
    if report.internal_errors.is_empty() {
        return out;
    }
    let mut any_backtrace = false;
    for error in &report.internal_errors {
        let _ = writeln!(out, "{error}");
        if let Some(backtrace) = &error.panic.backtrace {
            any_backtrace = true;
            out.push_str("stack backtrace:\n");
            out.push_str(backtrace);
            if !backtrace.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    let _ = writeln!(
        out,
        "note: internal errors are bugs in rustocop {}; please report them with the code that triggers them",
        env!("CARGO_PKG_VERSION")
    );
    if !any_backtrace {
        out.push_str("note: run with RUST_BACKTRACE=1 to include a backtrace\n");
    }
    out
}

/// Escape workflow-command message data, as `@actions/core`'s `escapeData`
/// does: `%`, CR and LF are percent-encoded (`%` first, so the encoding is
/// unambiguous).
fn escape_data(data: &str) -> String {
    escape_workflow(data, false)
}

/// Escape a workflow-command property value, as `@actions/core`'s
/// `escapeProperty` does: like [`escape_data`], plus `:` and `,`, which
/// delimit properties.
fn escape_property(value: &str) -> String {
    escape_workflow(value, true)
}

fn escape_workflow(text: &str, property: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\r' => out.push_str("%0D"),
            '\n' => out.push_str("%0A"),
            ':' if property => out.push_str("%3A"),
            ',' if property => out.push_str("%2C"),
            _ => out.push(ch),
        }
    }
    out
}

fn summary_line(report: &RunReport) -> String {
    let offenses = report.offense_count();
    let offense_text = if report.fix {
        format!(
            "{} correction{} applied, {offenses} offense{} remaining",
            report.corrections_applied,
            plural(report.corrections_applied),
            plural(offenses),
        )
    } else if offenses == 0 {
        "no offenses detected".to_string()
    } else {
        format!("{offenses} offense{} detected", plural(offenses))
    };
    let mut summary = format!(
        "{} file{} inspected, {offense_text}",
        report.files_inspected,
        plural(report.files_inspected)
    );
    let errors = report.errors.len() + report.internal_errors.len();
    if errors > 0 {
        let _ = write!(&mut summary, ", {errors} error{}", plural(errors));
    }
    summary
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolation::CaughtPanic;
    use crate::offense::{GlobalOffense, Offense, Severity};
    use crate::pipeline::{FileResult, InternalError, LintError};
    use serde_json::{Value, json};
    use std::path::PathBuf;

    fn sample_report() -> RunReport {
        RunReport {
            files_inspected: 2,
            results: vec![FileResult {
                global_offenses: Vec::new(),
                path: PathBuf::from("app/models/user.rb"),
                offenses: vec![Offense {
                    correction: None,
                    cop: "Lint/Debugger",
                    severity: Severity::Error,
                    message: "Remove debugger entry point `binding.pry`.".to_string(),
                    line: 8,
                    end_line: 8,
                    col: 5,
                }],
            }],
            errors: vec![LintError {
                path: PathBuf::from("lib/broken.rb"),
                position: Some((3, 7)),
                message: "syntax error on line 3: expected ')'".to_string(),
            }],
            internal_errors: Vec::new(),
            ..RunReport::default()
        }
    }

    fn internal_error(
        path: &str,
        cop: Option<&'static str>,
        position: Option<(u32, u32)>,
        backtrace: Option<&str>,
    ) -> InternalError {
        InternalError {
            path: PathBuf::from(path),
            cop,
            position,
            panic: CaughtPanic {
                message: "index out of bounds: the len is 0 but the index is 0".to_string(),
                location: Some("src/cops/lint/void.rs:12:5".to_string()),
                backtrace: backtrace.map(str::to_string),
            },
        }
    }

    #[test]
    fn text_format_lists_offenses_and_summary() {
        let text = render_text(&sample_report());
        assert!(text.contains(
            "app/models/user.rb:8:5: E: Remove debugger entry point `binding.pry`. (Lint/Debugger)"
        ));
        assert!(text.contains("lib/broken.rb: error: syntax error on line 3: expected ')'"));
        assert!(text.contains("2 files inspected, 1 offense detected, 1 error"));
    }

    #[test]
    fn offenses_format_counts_per_cop_most_first_then_by_name() {
        let mut report = sample_report();
        let debugger = report.results[0].offenses[0].clone();
        let offense = |cop| Offense {
            cop,
            ..debugger.clone()
        };
        report.results[0].offenses = [
            "Style/Not",
            "Lint/Void",
            "Lint/Debugger",
            "Lint/Void",
            "Lint/Debugger",
        ]
        .map(offense)
        .to_vec();
        report.results[0]
            .offenses
            .extend(vec![offense("Style/Semicolon"); 10]);
        report.results[0].global_offenses.push(GlobalOffense {
            cop: "Naming/FileName",
            severity: Severity::Convention,
            message: String::new(),
        });
        assert_eq!(
            render_offenses(&report),
            "10  Style/Semicolon\n \
              2  Lint/Debugger\n \
              2  Lint/Void\n \
              1  Naming/FileName\n \
              1  Style/Not\n\
             lib/broken.rb: error: syntax error on line 3: expected ')'\n\
             \n\
             2 files inspected, 16 offenses detected, 1 error\n"
        );
    }

    #[test]
    fn json_format_has_the_schema_of_rubocop() {
        let mut report = sample_report();
        report.results[0].global_offenses.push(GlobalOffense {
            cop: "Naming/FileName",
            severity: Severity::Convention,
            message: "Bad \"name\".".to_string(),
        });
        report.results[0].offenses[0].end_line = 9;
        let second = Offense {
            cop: "Style/StringLiterals",
            severity: Severity::Convention,
            message: "Prefer double quotes.".to_string(),
            col: 1,
            ..report.results[0].offenses[0].clone()
        };
        report.results[0].offenses.push(second);
        report.errors.push(LintError {
            path: PathBuf::from("lib/gone.rb"),
            position: None,
            message: "could not read file".to_string(),
        });
        report
            .internal_errors
            .push(internal_error("lib/gone.rb", Some("Lint/Void"), None, None));

        let location = |line: u32, column: u32| {
            json!({
                "start_line": line, "start_column": column, "last_line": line,
                "last_column": column, "length": 1, "line": line, "column": column,
            })
        };
        let offense = |severity: &str, message: &str, cop: &str, correctable, location| {
            json!({
                "severity": severity, "message": message, "cop_name": cop,
                "corrected": false, "correctable": correctable, "location": location,
            })
        };
        let rendered = render_json(&report, &Registry::new());
        assert!(rendered.ends_with("}\n") && rendered.lines().count() == 1);
        let internal = report.internal_errors[0].to_string();
        assert_eq!(
            serde_json::from_str::<Value>(&rendered).unwrap(),
            json!({
                "metadata": { "rustocop_version": env!("CARGO_PKG_VERSION") },
                "files": [
                    {
                        "path": "app/models/user.rb",
                        "offenses": [
                            offense("convention", "Bad \"name\".", "Naming/FileName", false, location(1, 1)),
                            offense(
                                "error",
                                "Remove debugger entry point `binding.pry`.",
                                "Lint/Debugger",
                                false,
                                location(8, 5),
                            ),
                            offense(
                                "convention",
                                "Prefer double quotes.",
                                "Style/StringLiterals",
                                true,
                                location(8, 1),
                            ),
                        ],
                    },
                    {
                        "path": "lib/broken.rb",
                        "offenses": [offense(
                            "fatal",
                            "syntax error on line 3: expected ')'",
                            "Lint/Syntax",
                            false,
                            location(3, 7),
                        )],
                    },
                    {
                        "path": "lib/gone.rb",
                        "offenses": [],
                        "errors": ["could not read file", internal],
                    },
                ],
                "summary": {
                    "offense_count": 4,
                    "target_file_count": 2,
                    "inspected_file_count": 2,
                    "error_count": 2,
                },
            })
        );

        let clean = render_json(&RunReport::default(), &Registry::new());
        assert_eq!(
            serde_json::from_str::<Value>(&clean).unwrap(),
            json!({
                "metadata": { "rustocop_version": env!("CARGO_PKG_VERSION") },
                "files": [],
                "summary": { "offense_count": 0, "target_file_count": 0, "inspected_file_count": 0 },
            })
        );
    }

    #[test]
    fn github_format_emits_workflow_commands() {
        let text = render_github(&sample_report());
        assert!(text.contains("::error file=app/models/user.rb,line=8,col=5,title=Lint/Debugger::app/models/user.rb:8:5: Lint/Debugger: Remove debugger entry point `binding.pry`."));
    }

    fn github_report(path: &str, message: &str) -> RunReport {
        RunReport {
            files_inspected: 1,
            results: vec![FileResult {
                global_offenses: Vec::new(),
                path: PathBuf::from(path),
                offenses: vec![Offense {
                    correction: None,
                    cop: "Lint/Void",
                    severity: Severity::Error,
                    message: message.to_string(),
                    line: 3,
                    end_line: 3,
                    col: 1,
                }],
            }],
            ..RunReport::default()
        }
    }

    #[test]
    fn escape_data_encodes_percent_cr_and_lf_only() {
        assert_eq!(escape_data("plain: text, here"), "plain: text, here");
        assert_eq!(escape_data("100%"), "100%25");
        assert_eq!(escape_data("a\r\nb\nc\rd"), "a%0D%0Ab%0Ac%0Dd");
        // `%` is encoded first so existing escapes are not double-decoded.
        assert_eq!(escape_data("%0A"), "%250A");
    }

    #[test]
    fn escape_property_also_encodes_colon_and_comma() {
        assert_eq!(escape_property("app/models/user.rb"), "app/models/user.rb");
        assert_eq!(escape_property("a,b:c%d\re\nf"), "a%2Cb%3Ac%25d%0De%0Af");
    }

    #[test]
    fn github_format_escapes_a_multi_line_message() {
        let report = github_report("app/a.rb", "Void value `foo(\n  1%\r\n)` detected.");
        let text = render_github(&report);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "::error file=app/a.rb,line=3,col=1,title=Lint/Void::app/a.rb:3:1: Lint/Void: \
                 Void value `foo(%0A  1%25%0D%0A)` detected.",
                "1 file inspected, 1 offense detected",
            ]
        );
    }

    #[test]
    fn github_format_escapes_commas_and_colons_in_the_file_property() {
        let report = github_report("app/a,b:c.rb", "Useless `x`.");
        let text = render_github(&report);
        let first = text.lines().next().unwrap();
        // The property is fully escaped; the message data keeps `,` and `:`
        // (only `%`, CR and LF are special there).
        assert_eq!(
            first,
            "::error file=app/a%2Cb%3Ac.rb,line=3,col=1,title=Lint/Void::app/a,b:c.rb:3:1: \
             Lint/Void: Useless `x`."
        );
    }

    #[test]
    fn github_format_escapes_percent_and_newlines_in_the_path() {
        let report = github_report("app/100%\nx.rb", "Useless `x`.");
        let text = render_github(&report);
        let first = text.lines().next().unwrap();
        assert_eq!(
            first,
            "::error file=app/100%25%0Ax.rb,line=3,col=1,title=Lint/Void::app/100%25%0Ax.rb:3:1: \
             Lint/Void: Useless `x`."
        );
    }

    #[test]
    fn github_format_annotates_a_syntax_error_at_its_line() {
        let text = render_github(&sample_report());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[1],
            "::error file=lib/broken.rb,line=3,col=7,title=rustocop::lib/broken.rb: error: \
             syntax error on line 3: expected ')'"
        );
        assert_eq!(lines[2], "2 files inspected, 1 offense detected, 1 error");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn github_format_annotates_every_error_in_order() {
        let report = RunReport {
            files_inspected: 2,
            errors: vec![
                LintError {
                    path: PathBuf::from("a.rb"),
                    position: Some((1, 10)),
                    message: "syntax error on line 1: first".to_string(),
                },
                LintError {
                    path: PathBuf::from("a.rb"),
                    position: Some((2, 1)),
                    message: "syntax error on line 2: second".to_string(),
                },
                LintError {
                    path: PathBuf::from("b.rb"),
                    position: None,
                    message: "could not read file: Permission denied (os error 13)".to_string(),
                },
            ],
            ..RunReport::default()
        };
        let text = render_github(&report);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "::error file=a.rb,line=1,col=10,title=rustocop::a.rb: error: syntax error on line 1: first",
                "::error file=a.rb,line=2,col=1,title=rustocop::a.rb: error: syntax error on line 2: second",
                // A read error has no line: a file-level annotation.
                "::error file=b.rb,title=rustocop::b.rb: error: could not read file: \
                 Permission denied (os error 13)",
                "2 files inspected, no offenses detected, 3 errors",
            ]
        );
    }

    #[test]
    fn github_format_escapes_error_paths_and_messages() {
        let report = RunReport {
            files_inspected: 1,
            errors: vec![LintError {
                path: PathBuf::from("lib/a,b:100%.rb"),
                position: Some((4, 2)),
                message: "syntax error on line 4: unexpected `%`\nhere".to_string(),
            }],
            ..RunReport::default()
        };
        let text = render_github(&report);
        let first = text.lines().next().unwrap();
        assert_eq!(
            first,
            "::error file=lib/a%2Cb%3A100%25.rb,line=4,col=2,title=rustocop::lib/a,b:100%25.rb: \
             error: syntax error on line 4: unexpected `%25`%0Ahere"
        );
    }

    #[test]
    fn global_findings_have_exact_location_free_output_and_escaping() {
        let report = RunReport {
            files_inspected: 1,
            fix: true,
            results: vec![FileResult {
                path: PathBuf::from("a,b:100%\n/gems.locked"),
                offenses: Vec::new(),
                global_offenses: vec![crate::offense::GlobalOffense {
                    cop: "Bundler/GemFilename",
                    severity: Severity::Convention,
                    message: "Use Gemfile.lock: 100%\r\nnot a source repair".into(),
                }],
            }],
            ..RunReport::default()
        };
        assert_eq!(
            render_text(&report),
            "a,b:100%\n/gems.locked: C: Use Gemfile.lock: 100%\r\nnot a source repair (Bundler/GemFilename)\n\n1 file inspected, 0 corrections applied, 1 offense remaining\n"
        );
        assert_eq!(
            render_github(&report),
            "::error file=a%2Cb%3A100%25%0A/gems.locked,title=Bundler/GemFilename::a,b:100%25%0A/gems.locked: Bundler/GemFilename: Use Gemfile.lock: 100%25%0D%0Anot a source repair\n1 file inspected, 0 corrections applied, 1 offense remaining\n"
        );
        assert_eq!(report.offense_count(), 1);
        assert_eq!(report.exit_code(), crate::pipeline::EXIT_OFFENSES);
        assert!(!report.has_errors());
    }

    #[test]
    fn summary_counts_zero_offenses_rubocop_style() {
        let report = RunReport {
            files_inspected: 3,
            ..RunReport::default()
        };
        let text = render_text(&report);
        assert!(text.contains("3 files inspected, no offenses detected"));
    }

    #[test]
    fn no_internal_errors_render_nothing() {
        assert_eq!(render_internal_errors(&sample_report()), "");
    }

    #[test]
    fn internal_errors_render_cop_file_panic_and_a_bug_report_hint() {
        let mut report = sample_report();
        report.internal_errors = vec![
            internal_error("app/a.rb", Some("Lint/Void"), Some((3, 7)), None),
            internal_error("app/b.rb", None, None, None),
        ];
        let rendered = render_internal_errors(&report);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(
            lines[0],
            "app/a.rb:3:7: internal error: Lint/Void panicked: index out of bounds: \
             the len is 0 but the index is 0 (at src/cops/lint/void.rs:12:5)"
        );
        assert!(lines[1].starts_with("app/b.rb: internal error: rustocop panicked: "));
        assert!(
            lines[2].contains(&format!("bugs in rustocop {}", env!("CARGO_PKG_VERSION"))),
            "{rendered}"
        );
        assert!(lines[2].contains("please report"), "{rendered}");
        assert!(lines[3].contains("RUST_BACKTRACE=1"), "{rendered}");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn captured_backtraces_follow_their_error() {
        let mut report = sample_report();
        report.internal_errors = vec![internal_error(
            "app/a.rb",
            Some("Lint/Void"),
            None,
            Some("   0: rustocop::cops::lint::void::check\n"),
        )];
        let rendered = render_internal_errors(&report);
        let lines: Vec<&str> = rendered.lines().collect();
        assert!(lines[0].starts_with("app/a.rb: internal error: Lint/Void panicked"));
        assert_eq!(lines[1], "stack backtrace:");
        assert_eq!(lines[2], "   0: rustocop::cops::lint::void::check");
        assert!(lines[3].contains("please report"), "{rendered}");
        assert!(!rendered.contains("RUST_BACKTRACE"), "{rendered}");
    }

    #[test]
    fn internal_errors_count_as_errors_in_the_summary_but_stay_off_stdout() {
        let mut report = sample_report();
        report.internal_errors = vec![internal_error("app/a.rb", Some("Lint/Void"), None, None)];
        let text = render_text(&report);
        assert!(
            text.contains("2 files inspected, 1 offense detected, 2 errors"),
            "{text}"
        );
        assert!(!text.contains("internal error"), "{text}");
        let github = render_github(&report);
        assert!(github.contains("1 offense detected, 2 errors"), "{github}");
        assert!(!github.contains("internal error"), "{github}");
    }
}
