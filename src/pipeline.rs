//! The lint pipeline: discover files, parse, run cops, filter offenses.

use std::{
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use rayon::prelude::*;

use crate::{
    config::ResolvedConfig,
    cops::{Registry, run_cops_with_corrections, select_active},
    discovery::collect_files,
    isolation::{CaughtPanic, QuietPanics, catch},
    offense::Offense,
    source::SourceFile,
};

/// CLI-selected run options.
#[derive(Debug, Default)]
pub struct RunOptions {
    /// `--only`: cop or department names.
    pub only: Vec<String>,
    /// `--except`: cop or department names.
    pub except: Vec<String>,
    /// `--stdin PATH`: read source from stdin, display it as PATH.
    pub stdin: Option<PathBuf>,
    /// `--fix`: apply safe corrections to files in a single pass.
    pub fix: bool,
}

/// Exit code: clean run.
pub const EXIT_CLEAN: i32 = 0;
/// Exit code: offenses detected.
pub const EXIT_OFFENSES: i32 = 1;
/// Exit code: errors (bad config, unreadable or unparseable files, internal
/// errors).
pub const EXIT_ERRORS: i32 = 2;

/// The outcome of a run.
#[derive(Debug, Default)]
pub struct RunReport {
    /// How many files were inspected.
    pub files_inspected: usize,
    /// Whether this was a `--fix` run (also controls summary wording).
    pub fix: bool,
    /// Number of corrections successfully written to disk.
    pub corrections_applied: usize,
    /// Offense lists, one per inspected file (sorted by path).
    pub results: Vec<FileResult>,
    /// Errors encountered while linting (io, syntax), sorted by path.
    pub errors: Vec<LintError>,
    /// Panics caught while linting, sorted by path (in run order within a
    /// file). Reported on stderr.
    pub internal_errors: Vec<InternalError>,
}

/// Offenses for one file.
#[derive(Debug)]
pub struct FileResult {
    /// The file's display path.
    pub path: PathBuf,
    /// The offenses found in it, sorted by (line, column, cop name, message).
    pub offenses: Vec<Offense>,
}

/// An error for one file (read failure, syntax or excessive AST depth). The run continues.
#[derive(Debug)]
pub struct LintError {
    /// The file's display path.
    pub path: PathBuf,
    /// The 1-based (line, column) of a syntax/depth error; `None` for a read
    /// failure, which concerns the whole file.
    pub position: Option<(u32, u32)>,
    /// What went wrong.
    pub message: String,
}

/// A panic caught while linting one file: a bug in rustocop, usually in a
/// cop. The run continues; a panicking cop sits out the rest of the file and
/// the other cops' offenses are reported as usual.
#[derive(Debug)]
pub struct InternalError {
    /// The file's display path.
    pub path: PathBuf,
    /// The cop that panicked; `None` for a panic outside any cop.
    pub cop: Option<&'static str>,
    /// The 1-based (line, column) of the node the cop was checking, if any.
    pub position: Option<(u32, u32)>,
    /// The panic.
    pub panic: CaughtPanic,
}

/// `path:line:col: internal error: Cop/Name panicked: message (at src/...)`,
/// shaped like the other per-file errors. Excludes the backtrace.
impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path.display())?;
        if let Some((line, col)) = self.position {
            write!(f, ":{line}:{col}")?;
        }
        let culprit = self.cop.unwrap_or("rustocop");
        write!(
            f,
            ": internal error: {culprit} panicked: {}",
            self.panic.message
        )?;
        if let Some(location) = &self.panic.location {
            write!(f, " (at {location})")?;
        }
        Ok(())
    }
}

/// Everything linting one file produced.
#[derive(Debug)]
pub struct FileReport {
    /// Main-parse structural evidence retained for the merged alias batch.
    heredoc_boundaries: Option<std::sync::Arc<crate::source::heredoc_boundaries::Boundaries>>,
    /// Number of corrections successfully written to this file.
    pub corrections_applied: usize,
    /// The file's offenses.
    pub result: FileResult,
    /// Read, syntax/depth, correction-validation or write errors. Read and
    /// initial syntax/depth errors prevent cops from running.
    pub errors: Vec<LintError>,
    /// Panics caught while linting the file.
    pub internal_errors: Vec<InternalError>,
}

impl FileReport {
    /// A report with no offenses and no errors yet.
    fn empty(display_path: &Path) -> Self {
        FileReport {
            heredoc_boundaries: None,
            corrections_applied: 0,
            result: FileResult {
                path: display_path.to_path_buf(),
                offenses: Vec::new(),
            },
            errors: Vec::new(),
            internal_errors: Vec::new(),
        }
    }
}

impl RunReport {
    /// Total offense count.
    pub fn offense_count(&self) -> usize {
        self.results.iter().map(|r| r.offenses.len()).sum()
    }

    /// Whether the run hit any errors, internal ones included.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty() || !self.internal_errors.is_empty()
    }

    /// The process exit code: errors win over offenses.
    pub fn exit_code(&self) -> i32 {
        if self.has_errors() {
            EXIT_ERRORS
        } else if self.offense_count() > 0 {
            EXIT_OFFENSES
        } else {
            EXIT_CLEAN
        }
    }
}

/// Reject every selector that matches no registered cop, even if other
/// selectors match or configuration would disable the selected cops.
fn validate_selections(options: &RunOptions, registry: &Registry) -> Result<()> {
    for (flag, names) in [("--only", &options.only), ("--except", &options.except)] {
        for name in names {
            let matches = registry.cops().iter().any(|cop| {
                cop.name() == name || cop.name().split('/').next() == Some(name.as_str())
            });
            if !matches {
                let suggestion = registry
                    .suggest_name(name)
                    .map(|name| format!("; did you mean `{name}`?"))
                    .unwrap_or_default();
                bail!("unknown cop or department `{name}` in {flag}{suggestion}");
            }
        }
    }
    Ok(())
}

/// Run rustocop over the given paths. Invalid selectors fail before any
/// files or stdin are read.
pub fn run(
    paths: &[PathBuf],
    options: &RunOptions,
    config: &ResolvedConfig,
    registry: &Registry,
) -> Result<RunReport> {
    if options.fix && options.stdin.is_some() {
        bail!("--fix cannot be used with --stdin; pass file paths to rewrite files");
    }
    validate_selections(options, registry)?;
    let active = CopSelection::new(registry, config, &options.only, &options.except);

    let targets: Vec<(PathBuf, PathBuf, Option<Vec<u8>>)> = if let Some(stdin_path) = &options.stdin
    {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .context("could not read source from stdin")?;
        let display_path = crate::discovery::display_path(stdin_path, &config.root);
        vec![(stdin_path.clone(), display_path, Some(bytes))]
    } else {
        collect_files(paths, &config.root, &config.global_excludes)?
            .into_iter()
            .map(|read_path| {
                let display_path = crate::discovery::display_path(&read_path, &config.root);
                (read_path, display_path, None)
            })
            .collect()
    };

    // Preserve every display path's cop exclusions and reporting. In fix mode,
    // coordinate aliases in one worker so they share one correction pass/write.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut identities = std::collections::HashMap::new();
    for (index, (path, _, _)) in targets.iter().enumerate() {
        let group = if options.fix {
            let identity = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            *identities.entry(identity).or_insert(groups.len())
        } else {
            groups.len()
        };
        if group == groups.len() {
            groups.push(Vec::new());
        }
        groups[group].push(index);
    }
    let mut corrections_applied = 0;
    let mut results = Vec::with_capacity(targets.len());
    let mut errors = Vec::new();
    let mut internal_errors = Vec::new();
    groups
        .par_iter()
        .flat_map_iter(|group| {
            let (read_path, display_path, bytes) = &targets[group[0]];
            if group.len() == 1 {
                return vec![lint_one_with_fix(
                    read_path,
                    display_path,
                    bytes.as_deref(),
                    &active,
                    config,
                    options.fix,
                )];
            }
            // Lint all aliases against the same snapshot, retaining path-specific
            // eligibility. If reading fails, report each alias's own read error.
            let original = fs::read(read_path);
            // Every merged edit must be safe under every known loading alias,
            // even an alias excluded for this cop. Use an incompatible path as
            // the snapshot's provenance for all aliases when one is present.
            let load_path = group
                .iter()
                .map(|&index| &targets[index].0)
                .find(|path| SourceFile::path_is_noncanonical(path))
                .unwrap_or(read_path);
            let reports = group
                .iter()
                .map(|&index| {
                    let (path, display, _) = &targets[index];
                    lint_one_in_mode(
                        if original.is_ok() { load_path } else { path },
                        display,
                        original
                            .as_deref()
                            .ok()
                            .map_or(SourceInput::Disk, SourceInput::DiskSnapshot),
                        &active,
                        config,
                        CorrectionMode::Collect,
                    )
                })
                .collect();
            match original {
                Ok(bytes) => correct_files(read_path, &bytes, reports, &active, config),
                Err(_) => reports,
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .for_each(|mut file| {
            corrections_applied += file.corrections_applied;
            if file.result.offenses.is_empty()
                && file.errors.is_empty()
                && file.internal_errors.is_empty()
            {
                return; // keep quiet on clean files
            }
            errors.append(&mut file.errors);
            internal_errors.append(&mut file.internal_errors);
            results.push(file.result);
        });

    results.sort_by(|a, b| a.path.cmp(&b.path));
    errors.sort_by(|a, b| a.path.cmp(&b.path));
    internal_errors.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(RunReport {
        fix: options.fix,
        corrections_applied,
        files_inspected: targets.len(),
        results,
        errors,
        internal_errors,
    })
}

/// Lint one file. Public for cop test seams.
///
/// `read_path` is where the bytes come from; `display_path` is what output
/// shows. Offense filtering (inline directives, per-cop excludes) happens
/// here, so cops can stay simple and report everything they see.
///
/// Never panics: a panicking cop is reported as an internal error and sits
/// out the rest of the file (see [`crate::cops::run_cops`]); a panic anywhere else in
/// linting the file drops the file's offenses for one internal error.
pub fn lint_one(
    read_path: &Path,
    display_path: &Path,
    bytes: Option<&[u8]>,
    active: &[&dyn crate::cops::Cop],
    config: &ResolvedConfig,
) -> FileReport {
    lint_one_with_fix(
        read_path,
        display_path,
        bytes,
        &CopSelection::explicit(active, config),
        config,
        false,
    )
}

/// CLI selection is fixed for the run; disabled candidates are mobilized only
/// by source opt-ins, never across an --only/--except or applicability boundary.
struct CopSelection<'a> {
    enabled: Vec<&'a dyn crate::cops::Cop>,
    disabled: Vec<&'a dyn crate::cops::Cop>,
    // Full configured universe, independent of --only/--except mobilization.
    directive_disabled: Vec<String>,
}
impl<'a> CopSelection<'a> {
    fn explicit(active: &[&'a dyn crate::cops::Cop], config: &ResolvedConfig) -> Self {
        static REGISTRY: std::sync::LazyLock<Registry> = std::sync::LazyLock::new(Registry::new);
        Self {
            enabled: active.to_vec(),
            disabled: Vec::new(),
            directive_disabled: Self::configured_disabled(&REGISTRY, config),
        }
    }
    fn configured_disabled(registry: &Registry, config: &ResolvedConfig) -> Vec<String> {
        registry
            .cops()
            .iter()
            .filter(|c| !crate::cops::configured_enabled(c.as_ref(), config))
            .map(|c| c.name().to_owned())
            .collect()
    }
    fn new(
        registry: &'a Registry,
        config: &'a ResolvedConfig,
        only: &[String],
        except: &[String],
    ) -> Self {
        let enabled = select_active(registry, config, only, except);
        let disabled = if only.is_empty() {
            let all: Vec<_> = registry
                .cops()
                .iter()
                .map(|c| c.name().to_owned())
                .collect();
            select_active(registry, config, &all, except)
                .into_iter()
                .filter(|c| !enabled.iter().any(|e| e.name() == c.name()))
                .collect()
        } else {
            Vec::new()
        };
        let directive_disabled = Self::configured_disabled(registry, config);
        Self {
            enabled,
            disabled,
            directive_disabled,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CorrectionMode {
    None,
    /// Generate corrections without writing; used to merge alias eligibility.
    Collect,
    Apply,
}

/// Byte ownership and disk/load-path provenance are independent: alias groups
/// and post-fix validation lint an in-memory snapshot of a disk input.
#[derive(Clone, Copy)]
enum SourceInput<'a> {
    Disk,
    DiskSnapshot(&'a [u8]),
    Memory(&'a [u8]),
}
impl<'a> From<Option<&'a [u8]>> for SourceInput<'a> {
    fn from(bytes: Option<&'a [u8]>) -> Self {
        bytes.map_or(Self::Disk, Self::Memory)
    }
}

fn lint_one_with_fix(
    read_path: &Path,
    display_path: &Path,
    bytes: Option<&[u8]>,
    active: &CopSelection<'_>,
    config: &ResolvedConfig,
    fix: bool,
) -> FileReport {
    lint_one_in_mode(
        read_path,
        display_path,
        bytes.into(),
        active,
        config,
        if fix {
            CorrectionMode::Apply
        } else {
            CorrectionMode::None
        },
    )
}

fn lint_one_in_mode(
    read_path: &Path,
    display_path: &Path,
    input: SourceInput<'_>,
    active: &CopSelection<'_>,
    config: &ResolvedConfig,
    mode: CorrectionMode,
) -> FileReport {
    let _quiet = QuietPanics::enter();
    guard_file(display_path, || {
        lint_file(read_path, display_path, input, active, config, mode)
    })
}

/// Run `lint` for the file at `display_path`, turning a panic that escapes
/// it (one outside any cop: parsing, directives, filtering) into an internal
/// error for the file.
fn guard_file(display_path: &Path, lint: impl FnOnce() -> FileReport) -> FileReport {
    catch(lint).unwrap_or_else(|panic| {
        let mut report = FileReport::empty(display_path);
        report.internal_errors.push(InternalError {
            path: display_path.to_path_buf(),
            cop: None,
            position: None,
            panic,
        });
        report
    })
}

/// [`lint_one`]'s work, outside its panic guard.
fn lint_file(
    read_path: &Path,
    display_path: &Path,
    input: SourceInput<'_>,
    active: &CopSelection<'_>,
    config: &ResolvedConfig,
    mode: CorrectionMode,
) -> FileReport {
    let mut report = FileReport::empty(display_path);
    // The same config-relative path drives pinned policies, user exclusions,
    // explicit files and stdin. Never canonicalize symlinks for eligibility.
    let relative = display_path
        .strip_prefix(&config.root)
        .unwrap_or(display_path);
    if crate::config::matches_any(&config.global_excludes, relative) {
        return report;
    }
    let eligible = |cop: &&dyn crate::cops::Cop| {
        cop.path_policy()
            .is_none_or(|policy| policy.allows(relative))
            && !cop_excluded(config, relative, cop.name())
    };
    let from_disk = !matches!(input, SourceInput::Memory(_));
    let bytes = match input {
        SourceInput::Memory(bytes) | SourceInput::DiskSnapshot(bytes) => bytes.to_vec(),
        SourceInput::Disk => match fs::read(read_path) {
            Ok(bytes) => bytes,
            Err(err) => {
                report.errors.push(LintError {
                    path: display_path.to_path_buf(),
                    position: None,
                    message: format!("could not read file: {err}"),
                });
                return report;
            }
        },
    };
    let file = SourceFile::new(display_path.to_path_buf(), bytes).with_source_path(read_path);
    let file = if from_disk {
        file.with_read_path(read_path)
    } else {
        file
    };
    let result = crate::parser::parse(&file.bytes);
    // Diagnostics need the same cached encoding metadata as cop offenses;
    // record it even when this parse has errors, without a second parse.
    file.record_encoding(&result);

    let positions = file.line_cols(
        result
            .errors()
            .map(|diagnostic| diagnostic.location().start_offset()),
    );
    for (diagnostic, (line, col)) in result.errors().zip(positions) {
        report.errors.push(LintError {
            path: file.path.clone(),
            position: Some((line, col)),
            message: format!("syntax error on line {line}: {}", diagnostic.message()),
        });
    }
    if !report.errors.is_empty() {
        // Preserve syntax-error precedence. Parsed also bounds teardown of
        // recovered trees, including on early return or unwinding.
        return report;
    }

    if let Some(error) = result.depth_error() {
        report.errors.push(LintError {
            path: file.path.clone(),
            position: Some(file.line_col(error.offset)),
            message: error.to_string(),
        });
        return report;
    }

    file.record_comments(&result);
    file.record_continuations(&result.continuations);
    file.record_warnings(&result);
    file.record_data_start(&result);
    file.record_directive_disabled(&active.directive_disabled);
    let disabled: Vec<_> = active.disabled.iter().map(|c| c.name()).collect();
    let directives = file.directives_with_disabled(&result, &disabled);
    let running: Vec<_> = active
        .enabled
        .iter()
        .copied()
        .chain(
            active
                .disabled
                .iter()
                .copied()
                .filter(|c| directives.opts_in(c.name())),
        )
        .filter(eligible)
        .collect();

    let mut offenses = Vec::new();
    let panics = run_cops_with_corrections(
        &running,
        &result,
        &file,
        &mut offenses,
        mode != CorrectionMode::None,
    );
    report.internal_errors = panics
        .into_iter()
        .map(|panic| InternalError {
            path: file.path.clone(),
            cop: Some(panic.cop),
            position: panic.position,
            panic: panic.panic,
        })
        .collect();

    offenses.retain(|offense| {
        directives.is_enabled_for_lines(offense.cop, offense.line, offense.end_line)
    });
    offenses.sort_by(|a, b| {
        (a.line, a.col, a.cop, a.message.as_str()).cmp(&(b.line, b.col, b.cop, b.message.as_str()))
    });
    // A correction may reach beyond its offense highlight. Never edit disabled
    // lines in that larger span, even when the offense itself is enabled.
    for offense in &mut offenses {
        if let Some(correction) = &offense.correction {
            let range = &correction.range;
            if range.start <= range.end
                && range.end <= file.bytes.len()
                && !directives.is_enabled_for_lines(
                    offense.cop,
                    file.line_of(range.start),
                    file.line_of(range.end.saturating_sub(1).max(range.start)),
                )
            {
                offense.correction = None;
            }
        }
    }
    if mode != CorrectionMode::None && offenses.iter().any(|o| o.correction.is_some()) {
        report.heredoc_boundaries = file.cached_heredoc_boundaries().cloned();
    }
    report.result.offenses = offenses;
    if mode != CorrectionMode::Apply {
        return report;
    }
    correct_files(read_path, &file.bytes, vec![report], active, config)
        .pop()
        .unwrap()
}

/// Merge eligible corrections across aliases, write once, and retain a separate
/// post-fix report for each display path. Any failing alias prevents the write.
fn correct_files(
    read_path: &Path,
    original: &[u8],
    mut reports: Vec<FileReport>,
    active: &CopSelection<'_>,
    config: &ResolvedConfig,
) -> Vec<FileReport> {
    if reports
        .iter()
        .any(|report| !report.errors.is_empty() || !report.internal_errors.is_empty())
    {
        return reports;
    }
    let mut safe: Vec<_> = reports
        .iter()
        .flat_map(|report| &report.result.offenses)
        .filter(|offense| {
            active
                .enabled
                .iter()
                .chain(&active.disabled)
                .any(|cop| cop.name() == offense.cop && cop.safe_fixes())
        })
        .cloned()
        .collect();
    // Aliases can contribute disjoint candidates which were never considered
    // together by a cop run. Reuse their original main-parse evidence, not a
    // second parse, and prove the union before conflict selection.
    if let Some(boundaries) = reports.iter().find_map(|r| r.heredoc_boundaries.as_ref()) {
        boundaries.filter(original, &mut safe);
    }
    let display_path = reports[0].result.path.clone();
    let applied = match crate::corrections::apply(original, &safe) {
        Ok(applied) => applied,
        Err(error) => {
            reports[0].errors.push(LintError {
                path: display_path.clone(),
                position: None,
                message: format!("could not apply corrections: {error}"),
            });
            return reports;
        }
    };
    if applied.count == 0 {
        return reports;
    }
    // Re-lint each display path exactly once to validate the new Ruby and
    // report current positions (including unsafe, unfixable and deferred edits).
    let mut corrected: Vec<_> = reports
        .iter()
        .map(|report| {
            lint_one_in_mode(
                read_path,
                &report.result.path,
                SourceInput::DiskSnapshot(&applied.bytes),
                active,
                config,
                CorrectionMode::None,
            )
        })
        .collect();
    if corrected
        .iter()
        .any(|report| !report.errors.is_empty() || !report.internal_errors.is_empty())
    {
        for (report, corrected) in reports.iter_mut().zip(&mut corrected) {
            if !corrected.errors.is_empty() || !corrected.internal_errors.is_empty() {
                report.errors.push(LintError {
                    path: report.result.path.clone(),
                    position: None,
                    message: "corrections failed validation; file left unchanged".into(),
                });
                report
                    .internal_errors
                    .append(&mut corrected.internal_errors);
            }
        }
        return reports;
    }
    if let Err(error) = write_corrected(read_path, original, &applied.bytes) {
        reports[0].errors.push(LintError {
            path: display_path,
            position: None,
            message: format!("could not write corrections: {error:#}"),
        });
        return reports;
    }
    corrected[0].corrections_applied = applied.count;
    corrected
}

/// Write through a sibling temporary file, so a failed write cannot truncate
/// the original. Resolve symlinks first to preserve them, and retain permissions.
fn write_corrected(path: &Path, original: &[u8], corrected: &[u8]) -> Result<()> {
    use std::io::Write;

    let path = fs::canonicalize(path)?;
    let metadata = fs::metadata(&path)?;
    anyhow::ensure!(!metadata.permissions().readonly(), "file is read-only");
    anyhow::ensure!(
        fs::read(&path)? == original,
        "file changed while linting; not overwritten"
    );
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("file has no parent directory")?)?;
    temporary.write_all(corrected)?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())?;
    temporary.persist(&path)?;
    Ok(())
}

/// Whether the cop is excluded for this file by config.
fn cop_excluded(config: &ResolvedConfig, path: &Path, cop: &str) -> bool {
    config
        .cop_settings
        .get(cop)
        .is_some_and(|settings| crate::config::matches_any(&settings.excludes, path))
}

/// Lint an inline source with the default config — the cop test seam.
#[cfg(test)]
pub(crate) fn lint_str(source: &str) -> Vec<Offense> {
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    lint_str_selected(
        &CopSelection::new(&registry, &config, &[], &[]),
        source,
        CorrectionMode::None,
    )
}

/// Lint an inline source with these cops and the default config. Panics,
/// failing the test, if a cop panicked: the pipeline would only report it.
#[cfg(test)]
pub(crate) fn lint_str_with(active: &[&dyn crate::cops::Cop], source: &str) -> Vec<Offense> {
    lint_str_in_mode(active, source, CorrectionMode::None)
}

/// The correction test seam: collect eligible corrections, but never write.
#[cfg(test)]
pub(crate) fn lint_str_with_corrections(
    active: &[&dyn crate::cops::Cop],
    source: &str,
) -> Vec<Offense> {
    lint_str_in_mode(active, source, CorrectionMode::Collect)
}

#[cfg(test)]
fn lint_str_in_mode(
    active: &[&dyn crate::cops::Cop],
    source: &str,
    mode: CorrectionMode,
) -> Vec<Offense> {
    lint_str_selected(
        &CopSelection::explicit(active, &ResolvedConfig::default()),
        source,
        mode,
    )
}

#[cfg(test)]
fn lint_str_selected(
    active: &CopSelection<'_>,
    source: &str,
    mode: CorrectionMode,
) -> Vec<Offense> {
    let config = ResolvedConfig::default();
    let report = lint_one_in_mode(
        Path::new("test.rb"),
        Path::new("test.rb"),
        SourceInput::Memory(source.as_bytes()),
        active,
        &config,
        mode,
    );
    if !report.internal_errors.is_empty() {
        let mut details = String::new();
        for error in &report.internal_errors {
            details.push_str(&format!("\n{error}"));
            if let Some(backtrace) = &error.panic.backtrace {
                details.push_str(&format!("\n{backtrace}"));
            }
        }
        panic!("linting {source:?} panicked:{details}");
    }
    report.result.offenses
}

#[cfg(test)]
mod fix_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_preflight_runs_before_cops_on_default_test_stacks() {
        let mut source = String::new();
        for i in 0..100 {
            source.push_str(&format!("def foo{i}(*args)\nbar(*args)\n"));
        }
        source.push_str(&"end\n".repeat(100));
        let config = ResolvedConfig::default();
        let registry = Registry::new();
        let active = select_active(&registry, &config, &[], &[]);
        let report = lint_one(
            Path::new("deep.rb"),
            Path::new("deep.rb"),
            Some(source.as_bytes()),
            &active,
            &config,
        );
        assert!(report.result.offenses.is_empty());
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].message, "AST exceeds maximum depth of 64");
        assert_eq!(report.errors[0].position, Some((60, 5)));
    }

    #[test]
    fn rejected_receiver_chain_cleanup_fits_default_test_stack() {
        let source = format!("root{}\n", ".foo".repeat(5000));
        let report = lint_one(
            Path::new("chain.rb"),
            Path::new("chain.rb"),
            Some(source.as_bytes()),
            &[],
            &ResolvedConfig::default(),
        );
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].message, "AST exceeds maximum depth of 64");
        assert!(report.result.offenses.is_empty());
        assert!(report.internal_errors.is_empty());
    }

    #[test]
    fn supported_depth_boundary_preserves_real_cop_and_scope_diagnostics() {
        let mut defs = String::new();
        for i in 0..29 {
            defs.push_str(&format!("def foo{i}(*args)\nbar(*args)\n"));
        }
        defs.push_str(&"end\n".repeat(29));
        let sources = [
            defs,
            format!("{}1{}", "[".repeat(61), "]".repeat(61)),
            format!("root{}", ".foo".repeat(61)),
            format!("{}[1]\n{}", "begin\n".repeat(30), "end\n".repeat(30)),
            format!("{}[1]\n{}", "each do\n".repeat(20), "end\n".repeat(20)),
        ];
        let config = ResolvedConfig::default();
        let registry = Registry::new();
        let active = select_active(&registry, &config, &[], &[]);
        for (index, source) in sources.iter().enumerate() {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            assert_eq!(
                crate::cops::depth::check(&parsed.node().as_program_node().unwrap()),
                Ok(())
            );
            let report = lint_one(
                Path::new("boundary.rb"),
                Path::new("boundary.rb"),
                Some(source.as_bytes()),
                &active,
                &config,
            );
            assert!(report.errors.is_empty(), "{report:?}");
            assert!(report.internal_errors.is_empty(), "{report:?}");
            if index == 0 {
                let forwarding: Vec<_> = report
                    .result
                    .offenses
                    .iter()
                    .filter(|offense| offense.cop == "Style/ArgumentsForwarding")
                    .collect();
                assert_eq!(forwarding.len(), 58);
                assert_eq!((forwarding[0].line, forwarding[0].col), (1, 10));
            }
        }
    }

    #[test]
    fn syntax_errors_take_precedence_over_depth_rejection() {
        let source = format!("{}1{}", "[".repeat(100), "]".repeat(99));
        let report = lint_one(
            Path::new("bad.rb"),
            Path::new("bad.rb"),
            Some(source.as_bytes()),
            &[],
            &ResolvedConfig::default(),
        );
        assert!(!report.errors.is_empty());
        assert!(
            report
                .errors
                .iter()
                .all(|error| error.message.starts_with("syntax error"))
        );
        assert!(report.result.offenses.is_empty());
        assert!(report.internal_errors.is_empty());
    }

    #[test]
    fn highlights_use_expression_ranges_not_enclosing_constructs() {
        // RuboCop's highlights, including its send/block distinction and
        // keyword-only offenses. The last line is inclusive.
        for (cop, source, expected) in [
            (
                "Lint/Debugger",
                "binding.pry(\n  value\n) do\n  work\nend\n",
                (1, 3),
            ),
            (
                "Lint/DeprecatedClassMethods",
                "File.exists?(\n  path\n)\n",
                (1, 1),
            ),
            (
                "Lint/DeprecatedClassMethods",
                "attr :name,\n  true\n",
                (1, 2),
            ),
            ("Lint/RaiseException", "raise ::\n  Exception\n", (1, 2)),
            ("Lint/SelfAssignment", "values[0] =\n  values[0]\n", (1, 2)),
            (
                "Lint/BinaryOperatorWithIdenticalOperands",
                "value.==(\n  value\n) do\n  work\nend\n",
                (1, 3),
            ),
            (
                "Lint/InheritException",
                "class C < ::\n  Exception\nend\n",
                (1, 2),
            ),
            (
                "Lint/RescueException",
                "begin\n  work\nrescue Exception\n  recover(\n    error)\nrescue Other\n  other\nend\n",
                (3, 5),
            ),
            (
                "Lint/SuppressedException",
                "begin\n  work\nrescue Exception,\n  Other\nend\n",
                (3, 4),
            ),
            (
                "Lint/DuplicateMethods",
                "def foo; end\ndef foo(\n  parameter\n)\n  work\nend\n",
                (2, 2),
            ),
            (
                "Lint/NestedMethodDefinition",
                "def outer\n  def inner\n    work\n  end\nend\n",
                (2, 4),
            ),
            (
                "Lint/EnsureReturn",
                "begin\n  work\nensure\n  return [\n    value\n  ]\nend\n",
                (4, 6),
            ),
            (
                "Style/RedundantReturn",
                "def foo\n  return [\n    value\n  ]\nend\n",
                (2, 2),
            ),
            ("Style/HashSyntax", "{ :foo \\\n  => value }\n", (1, 2)),
            ("Style/RedundantParentheses", "x = (\n  value\n)\n", (1, 3)),
            (
                "Lint/DuplicateHashKey",
                "{ [1, 2] => a, [\n  1, 2\n] => b }\n",
                (1, 3),
            ),
            (
                "Lint/DuplicateSetElement",
                "Set[ [1, 2], [\n  1, 2\n] ]\n",
                (1, 3),
            ),
            (
                "Lint/DuplicateCaseCondition",
                "case x\nwhen [1, 2], [\n  1, 2\n]\n  work\nend\n",
                (2, 4),
            ),
            (
                "Lint/DuplicateElsifCondition",
                "if foo(1)\n  work\nelsif foo(\n  1\n)\n  work\nend\n",
                (3, 5),
            ),
            (
                "Lint/DuplicateRescueException",
                "begin\n  work\nrescue ::Exception\n  work\nrescue ::\n  Exception\n  work\nend\n",
                (5, 6),
            ),
            (
                "Lint/LiteralAsCondition",
                "if [\n  1, 2\n]\n  work\nend\n",
                (1, 3),
            ),
            (
                "Lint/LiteralInInterpolation",
                "\"#{[\n  1, 2\n]}\"\n",
                (1, 3),
            ),
            ("Lint/EmptyInterpolation", "\"#{\n}\"\n", (1, 2)),
            (
                "Naming/ClassAndModuleCamelCase",
                "class Bad_Name::\n  Inner\nend\n",
                (1, 2),
            ),
            ("Security/JSONLoad", "JSON.load(\n  value\n)\n", (1, 1)),
            (
                "Security/IoMethods",
                "IO.read(\n  path\n) do\n  work\nend\n",
                (1, 3),
            ),
            ("Security/CompoundHash", "[\n  value\n].hash\n", (1, 3)),
            (
                "Naming/MethodName",
                "define_method(\n  :badName\n) do\n  work\nend\n",
                (1, 3),
            ),
            (
                "Naming/MethodParameterName",
                "def foo(x = [\n  value\n])\nend\n",
                (1, 1),
            ),
            (
                "Lint/Void",
                "def foo\n  lambda do\n    work\n  end\n  work\nend\n",
                (2, 4),
            ),
            (
                "Lint/UnreachableCode",
                "def foo\n  return\n  work do\n    more\n  end\nend\n",
                (3, 5),
            ),
        ] {
            assert!(
                ruby_prism::parse(source.as_bytes())
                    .errors()
                    .next()
                    .is_none(),
                "fixture must parse: {source}"
            );
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == cop)
                .collect();
            assert_eq!(found.len(), 1, "{cop}: {source}");
            assert_eq!(
                (found[0].line, found[0].end_line),
                expected,
                "{cop}: {source}"
            );
            let mut lines: Vec<_> = source.lines().map(str::to_owned).collect();
            lines[expected.1 as usize - 1].push_str(&format!(" # rubocop:disable {cop}"));
            let suppressed = lines.join("\n");
            assert!(
                ruby_prism::parse(suppressed.as_bytes())
                    .errors()
                    .next()
                    .is_none(),
                "fixture must parse: {suppressed}"
            );
            assert!(
                lint_str(&suppressed).iter().all(|o| o.cop != cop),
                "last-line disable: {cop}: {suppressed}"
            );
        }
    }

    #[test]
    fn same_position_offenses_are_sorted_by_cop_in_either_registry_order() {
        use crate::cops::lint::{RescueException, SuppressedException};

        for cops in [
            vec![
                Box::new(RescueException) as Box<dyn crate::cops::Cop>,
                Box::new(SuppressedException),
            ],
            vec![
                Box::new(SuppressedException) as Box<dyn crate::cops::Cop>,
                Box::new(RescueException),
            ],
        ] {
            let report = run_files(
                &Registry::from_cops(cops),
                &[("x.rb", "begin\n  work\nrescue Exception\nend\n")],
            );
            let found: Vec<_> = report.results[0]
                .offenses
                .iter()
                .map(|o| (o.line, o.col, o.cop))
                .collect();
            assert_eq!(
                found,
                [
                    (3, 1, "Lint/RescueException"),
                    (3, 1, "Lint/SuppressedException"),
                ]
            );
        }
    }

    /// Emits messages in reverse alphabetical order at the same position.
    struct MessageCop(&'static str);

    impl crate::cops::Cop for MessageCop {
        fn name(&self) -> &'static str {
            self.0
        }

        fn check_source(&self, _file: &SourceFile, out: &mut Vec<Offense>) {
            for message in ["Zulu", "Alpha"] {
                out.push(Offense {
                    correction: None,
                    cop: self.name(),
                    severity: self.severity(),
                    message: message.to_string(),
                    line: 1,
                    end_line: 1,
                    col: 1,
                });
            }
        }
    }

    #[test]
    fn same_position_offenses_from_one_cop_are_sorted_by_message() {
        let found = lint_str_with(&[&MessageCop("Lint/Messages")], "work\n");
        let messages: Vec<_> = found.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(messages, ["Alpha", "Zulu"]);
    }

    #[test]
    fn every_output_format_preserves_cop_then_message_order() {
        let registry = Registry::from_cops(vec![
            Box::new(MessageCop("Lint/Beta")),
            Box::new(MessageCop("Lint/Alpha")),
        ]);
        let report = run_files(&registry, &[("x.rb", "work\n")]);
        assert_eq!(
            crate::output::render_text(&report),
            "x.rb:1:1: E: Alpha (Lint/Alpha)\n\
             x.rb:1:1: E: Zulu (Lint/Alpha)\n\
             x.rb:1:1: E: Alpha (Lint/Beta)\n\
             x.rb:1:1: E: Zulu (Lint/Beta)\n\n\
             1 file inspected, 4 offenses detected\n"
        );
        assert_eq!(
            crate::output::render_github(&report),
            "::error file=x.rb,line=1,col=1,title=Lint/Alpha::x.rb:1:1: Lint/Alpha: Alpha\n\
             ::error file=x.rb,line=1,col=1,title=Lint/Alpha::x.rb:1:1: Lint/Alpha: Zulu\n\
             ::error file=x.rb,line=1,col=1,title=Lint/Beta::x.rb:1:1: Lint/Beta: Alpha\n\
             ::error file=x.rb,line=1,col=1,title=Lint/Beta::x.rb:1:1: Lint/Beta: Zulu\n\
             1 file inspected, 4 offenses detected\n"
        );
    }

    #[test]
    fn reports_syntax_errors_and_continues() {
        let registry = Registry::new();
        let mut config = ResolvedConfig::default();
        let options = RunOptions::default();
        let tmp = tempfile::tempdir().unwrap();
        config.root = tmp.path().to_path_buf();
        let bad = tmp.path().join("broken.rb");
        std::fs::write(&bad, "def oops(\n").unwrap();
        let good = tmp.path().join("fine.rb");
        std::fs::write(&good, "1 + 1\n").unwrap();

        let report = run(&[tmp.path().to_path_buf()], &options, &config, &registry).unwrap();
        assert_eq!(report.files_inspected, 2);
        assert!(!report.errors.is_empty());
        assert!(report.errors[0].message.contains("syntax error"));
        assert_eq!(report.offense_count(), 0);
        assert!(report.has_errors());
    }

    #[test]
    fn syntax_errors_carry_their_position() {
        let report = run_files(&Registry::new(), &[("x.rb", "x = 1\nfoo(1, ]\n")]);
        // Prism reports several diagnostics, all at the stray `]`.
        assert!(!report.errors.is_empty());
        for error in &report.errors {
            assert_eq!(error.position, Some((2, 8)), "{error:?}");
            assert!(
                error.message.starts_with("syntax error on line 2: "),
                "{error:?}"
            );
        }
    }

    pub(super) fn encoded_position_prefixes() -> Vec<(&'static [u8], u32)> {
        vec![
            (b"# coding: ascii-8bit\n\"\xe3\x80\x80\"; ", 8),
            (b"# coding: ISO-8859-1\n\"\xe3\x80\x80\"; ", 8),
            (b"# coding: Windows-1252\n\"\xc3\xa9\"; ", 7),
            (b"# coding: Shift_JIS\n\"\x82\xa0\"; ", 6),
            (b"# coding: EUC-JP\n\"\xa4\xa2\"; ", 6),
            (b"# coding: UTF-8\n\"\xe3\x80\x80\"; ", 6),
            // Prism stops at the first legacy `coding` candidate even when
            // it lacks a separator; the later declaration is ignored.
            (
                b"# coding guidelines; coding: ascii-8bit\n\"\xe3\x80\x80\"; ",
                6,
            ),
            // A BOM overrides the pragma, including on subsequent lines.
            (b"\xef\xbb\xbf# coding: ascii-8bit\n\"\xe3\x80\x80\"; ", 6),
        ]
    }

    #[test]
    fn encoded_source_offenses_use_ruby_character_columns() {
        for (prefix, col) in encoded_position_prefixes() {
            let mut source = prefix.to_vec();
            source.extend_from_slice(b"binding.pry\n");
            let report = lint_one(
                Path::new("x.rb"),
                Path::new("x.rb"),
                Some(&source),
                &[&crate::cops::lint::Debugger],
                &ResolvedConfig::default(),
            );
            assert!(report.errors.is_empty(), "{report:?}");
            assert!(report.internal_errors.is_empty(), "{report:?}");
            let found = &report.result.offenses;
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col), (2, col), "{source:?}");
            assert_eq!(
                found[0].message,
                "Remove debugger entry point `binding.pry`."
            );
        }
    }

    #[test]
    fn encoded_source_syntax_errors_use_ruby_character_columns() {
        for (prefix, col) in encoded_position_prefixes() {
            let mut source = prefix.to_vec();
            source.extend_from_slice(b"foo(1, ]\n");
            let report = lint_one(
                Path::new("x.rb"),
                Path::new("x.rb"),
                Some(&source),
                &[],
                &ResolvedConfig::default(),
            );
            assert!(report.internal_errors.is_empty(), "{report:?}");
            assert!(!report.errors.is_empty(), "{source:?}");
            for error in &report.errors {
                assert_eq!(error.position, Some((2, col + 7)), "{source:?}: {error:?}");
                assert!(error.message.starts_with("syntax error on line 2: "));
            }
        }
    }

    #[test]
    fn utf8_bom_does_not_shift_offense_columns() {
        for (source, first_col) in [
            ("binding.pry\n  binding.pry\n", 1),
            ("é = 1; binding.pry\n  binding.pry\n", 8),
        ] {
            let plain = lint_str_with(&[&crate::cops::lint::Debugger], source);
            let bom = lint_str_with(
                &[&crate::cops::lint::Debugger],
                &format!("\u{feff}{source}"),
            );
            assert_eq!(plain.len(), 2);
            assert_eq!((plain[0].line, plain[0].col), (1, first_col));
            assert_eq!((plain[1].line, plain[1].col), (2, 3));
            assert_eq!(bom.len(), plain.len());
            for (actual, expected) in bom.iter().zip(&plain) {
                assert_eq!(
                    (actual.line, actual.end_line, actual.col, &actual.message),
                    (
                        expected.line,
                        expected.end_line,
                        expected.col,
                        &expected.message
                    ),
                    "{source:?}"
                );
            }
        }
    }

    #[test]
    fn utf8_bom_does_not_shift_syntax_error_columns() {
        for (source, col) in [("foo(1, ]\n", 8), ("é = 1; foo(1, ]\n", 15)] {
            let registry = Registry::new();
            let plain = run_files(&registry, &[("x.rb", source)]);
            let bom = run_files(&registry, &[("x.rb", &format!("\u{feff}{source}"))]);
            assert!(!plain.errors.is_empty());
            assert_eq!(bom.errors.len(), plain.errors.len());
            for (actual, expected) in bom.errors.iter().zip(&plain.errors) {
                assert_eq!(expected.position, Some((1, col)));
                assert_eq!(actual.position, expected.position, "{source:?}");
                assert_eq!(actual.message, expected.message);
            }
        }
    }

    #[test]
    fn read_errors_have_no_position() {
        let config = ResolvedConfig::default();
        let tmp = tempfile::tempdir().unwrap();
        let report = lint_one(
            &tmp.path().join("missing.rb"),
            Path::new("missing.rb"),
            None,
            &[],
            &config,
        );
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].position, None);
        assert!(
            report.errors[0]
                .message
                .starts_with("could not read file: ")
        );
    }

    #[test]
    fn stdin_lints_and_reports_offenses() {
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let options = RunOptions {
            stdin: Some(PathBuf::from("stdin.rb")),
            ..RunOptions::default()
        };
        // run() reads real stdin; exercise lint_one's stdin path directly.
        let report = lint_one(
            Path::new("stdin.rb"),
            Path::new("stdin.rb"),
            Some(b"binding.pry\n"),
            &select_active(&registry, &config, &[], &[]),
            &config,
        );
        assert!(report.errors.is_empty());
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.result.offenses.len(), 1);
        assert_eq!(report.result.offenses[0].cop, "Lint/Debugger");
        assert_eq!(options.stdin, Some(PathBuf::from("stdin.rb")));
    }

    #[test]
    fn empty_input_runs_source_lifecycle_without_requiring_statements() {
        let found = lint_str("");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].cop, "Lint/EmptyFile");
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert!(found[0].correction.is_none());
        assert!(lint_str("# just a comment\n").is_empty());
    }

    #[test]
    fn multiple_files_are_sorted_and_counted() {
        let registry = Registry::new();
        let mut config = ResolvedConfig::default();
        let options = RunOptions::default();
        let tmp = tempfile::tempdir().unwrap();
        config.root = tmp.path().to_path_buf();
        std::fs::write(tmp.path().join("z.rb"), "binding.pry\n").unwrap();
        std::fs::write(tmp.path().join("a.rb"), "debugger\n").unwrap();

        let report = run(&[tmp.path().to_path_buf()], &options, &config, &registry).unwrap();
        assert_eq!(report.files_inspected, 2);
        assert_eq!(report.results.len(), 2);
        assert_eq!(report.results[0].path, PathBuf::from("a.rb"));
        assert_eq!(report.offense_count(), 2);
    }

    /// A registry of the deliberately panicking test cops plus Lint/Debugger.
    fn panicking_registry() -> Registry {
        Registry::from_cops(vec![
            Box::new(crate::cops::testing::PanicOnCall),
            Box::new(crate::cops::testing::PanicOnSource),
            Box::new(crate::cops::lint::Debugger),
        ])
    }

    /// Lint a temporary directory holding `files` with `registry`.
    fn run_files(registry: &Registry, files: &[(&str, &str)]) -> RunReport {
        let mut config = ResolvedConfig::default();
        let tmp = tempfile::tempdir().unwrap();
        config.root = tmp.path().to_path_buf();
        for (name, source) in files {
            let path = tmp.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }
        let options = RunOptions::default();
        run(&[tmp.path().to_path_buf()], &options, &config, registry).unwrap()
    }

    fn offenses_of(report: &RunReport) -> Vec<(String, &'static str, u32)> {
        report
            .results
            .iter()
            .flat_map(|result| {
                result
                    .offenses
                    .iter()
                    .map(|o| (result.path.display().to_string(), o.cop, o.line))
            })
            .collect()
    }

    #[test]
    fn a_panicking_cop_on_one_file_does_not_stop_other_files_or_cops() {
        let report = run_files(
            &panicking_registry(),
            &[
                ("a.rb", "mark\n"),
                ("b.rb", "mark\nboom\nbinding.pry\nmark\n"),
                ("c.rb", "debugger\nmark\n"),
            ],
        );
        assert_eq!(report.files_inspected, 3);
        assert_eq!(
            offenses_of(&report),
            [
                ("a.rb".to_string(), "Lint/PanicOnCall", 1),
                ("a.rb".to_string(), "Lint/PanicOnSource", 1),
                // Each panicking cop sits out the rest of b.rb only.
                ("b.rb".to_string(), "Lint/PanicOnCall", 1),
                ("b.rb".to_string(), "Lint/Debugger", 3),
                ("c.rb".to_string(), "Lint/Debugger", 1),
                ("c.rb".to_string(), "Lint/PanicOnCall", 2),
                ("c.rb".to_string(), "Lint/PanicOnSource", 2),
            ]
        );
        // One report per cop and file, in run order: check_source first.
        let reported: Vec<_> = report
            .internal_errors
            .iter()
            .map(|e| (e.path.clone(), e.cop, e.position))
            .collect();
        assert_eq!(
            reported,
            [
                (PathBuf::from("b.rb"), Some("Lint/PanicOnSource"), None),
                (
                    PathBuf::from("b.rb"),
                    Some("Lint/PanicOnCall"),
                    Some((2, 1))
                ),
            ]
        );
        assert!(report.errors.is_empty());
        assert!(report.has_errors());
        assert_eq!(report.exit_code(), EXIT_ERRORS);
    }

    #[test]
    fn internal_errors_name_the_cop_the_file_and_the_panic() {
        let report = run_files(&panicking_registry(), &[("lib/x.rb", "boom\n")]);
        let rendered = report.internal_errors[1].to_string();
        assert!(
            rendered.starts_with(
                "lib/x.rb:1:1: internal error: Lint/PanicOnCall panicked: index out of bounds"
            ),
            "{rendered}"
        );
        assert!(rendered.contains("(at src/cops/testing.rs:"), "{rendered}");
        let rendered = report.internal_errors[0].to_string();
        assert!(
            rendered.starts_with("lib/x.rb: internal error: Lint/PanicOnSource panicked: PanicOnSource saw `boom` (at src/cops/testing.rs:"),
            "{rendered}"
        );
    }

    #[test]
    fn panics_are_not_reported_for_files_that_do_not_parse() {
        let report = run_files(&panicking_registry(), &[("x.rb", "boom(\n")]);
        assert!(report.internal_errors.is_empty());
        assert!(report.errors[0].message.contains("syntax error"));
    }

    #[test]
    fn directives_still_filter_the_offenses_of_a_cop_that_panicked() {
        let report = run_files(
            &panicking_registry(),
            &[(
                "x.rb",
                "mark # rubocop:disable Lint/PanicOnCall\nmark\nboom\n",
            )],
        );
        assert_eq!(
            offenses_of(&report),
            [("x.rb".to_string(), "Lint/PanicOnCall", 2)]
        );
        assert_eq!(report.internal_errors.len(), 2);
    }

    #[test]
    fn a_panic_outside_any_cop_is_an_internal_error_for_the_file() {
        let _quiet = crate::isolation::QuietPanics::enter();
        let report = guard_file(Path::new("lib/x.rb"), || panic!("directive parser bug"));
        assert!(report.result.offenses.is_empty());
        assert!(report.errors.is_empty());
        assert_eq!(report.internal_errors.len(), 1);
        let error = &report.internal_errors[0];
        assert_eq!(error.cop, None);
        assert!(
            error.to_string().starts_with(
                "lib/x.rb: internal error: rustocop panicked: directive parser bug (at src/pipeline.rs:"
            ),
            "{error}"
        );
    }

    #[test]
    #[should_panic(expected = "Lint/PanicOnCall panicked: index out of bounds")]
    fn cop_tests_fail_loudly_when_the_cop_panics() {
        lint_str_with(&[&crate::cops::testing::PanicOnCall], "boom\n");
    }

    #[test]
    fn exit_codes() {
        let mut report = RunReport::default();
        assert_eq!(report.exit_code(), EXIT_CLEAN);
        report.results.push(FileResult {
            path: PathBuf::from("a.rb"),
            offenses: lint_str("binding.pry\n"),
        });
        assert_eq!(report.exit_code(), EXIT_OFFENSES);
        report.internal_errors.push(InternalError {
            path: PathBuf::from("a.rb"),
            cop: Some("Lint/Debugger"),
            position: None,
            panic: crate::isolation::CaughtPanic {
                message: "oops".to_string(),
                location: None,
                backtrace: None,
            },
        });
        assert_eq!(report.exit_code(), EXIT_ERRORS, "errors win over offenses");
        report.internal_errors.clear();
        report.errors.push(LintError {
            path: PathBuf::from("b.rb"),
            position: Some((1, 1)),
            message: "syntax error".to_string(),
        });
        assert_eq!(report.exit_code(), EXIT_ERRORS);
    }
}
