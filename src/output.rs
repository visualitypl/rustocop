//! Output rendering: human text, GitHub Actions annotations and a per-cop
//! offense count.

use std::{collections::BTreeMap, fmt::Write as _};

use crate::pipeline::RunReport;

/// The supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// Human-readable, RuboCop-inspired text.
    Text,
    /// GitHub Actions workflow commands (`::error ...`).
    Github,
    /// The number of offenses per cop, most first.
    Offenses,
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
