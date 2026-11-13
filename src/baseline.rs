//! `--write-baseline`: a `rustocop.toml` that silences the offenses a
//! codebase has today, so a first run is quiet and new code is still checked.
//!
//! As RuboCop's `--auto-gen-config` does, a cop with offenses in few files
//! skips those files, and a cop with offenses in many files is switched off.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::Path,
};

use crate::pipeline::RunReport;

/// The most files a cop skips before it is switched off instead: the default
/// of RuboCop's `--exclude-limit`.
// A fixed limit; add a flag when someone asks for another one.
pub const FILE_LIMIT: usize = 15;

/// The offenses a baseline silences, gathered from one or more runs.
#[derive(Default)]
pub struct Baseline {
    cops: BTreeMap<&'static str, Silenced>,
}

/// What a baseline silences for one cop.
#[derive(Default)]
struct Silenced {
    offenses: usize,
    /// The files with offenses, as glob patterns relative to the config file.
    files: BTreeSet<String>,
    /// The files where the cop reported after it was switched off: a
    /// `rubocop:enable` comment switches it on again there.
    switched_on: BTreeSet<String>,
}

impl Silenced {
    fn switched_off(&self) -> bool {
        self.files.len() > FILE_LIMIT
    }
}

impl Baseline {
    /// Add the offenses of `report`, whose paths are written relative to
    /// `root`, the directory of the config file.
    pub fn add(&mut self, report: &RunReport, root: &Path) {
        let off =
            |(cop, silenced): (&&'static str, &Silenced)| silenced.switched_off().then_some(*cop);
        let switched_off: BTreeSet<_> = self.cops.iter().filter_map(off).collect();
        for result in &report.results {
            let path = result.path.strip_prefix(root).unwrap_or(&result.path);
            // A path that is not UTF-8 gives a pattern that matches nothing;
            // its offenses stay reported.
            let path = path.to_string_lossy();
            let path = path.trim_start_matches("./");
            // `escape` leaves a backslash, which escapes the next character
            // in a pattern everywhere but on Windows.
            let pattern = if cfg!(windows) {
                globset::escape(path)
            } else {
                globset::escape(&path.replace('\\', "\\\\"))
            };
            let located = result.offenses.iter().map(|offense| offense.cop);
            let global = result.global_offenses.iter().map(|offense| offense.cop);
            for cop in located.chain(global) {
                let silenced = self.cops.entry(cop).or_default();
                silenced.offenses += 1;
                silenced.files.insert(pattern.clone());
                if switched_off.contains(cop) {
                    silenced.switched_on.insert(pattern.clone());
                }
            }
        }
    }

    /// How many offenses the baseline silences.
    pub fn offenses(&self) -> usize {
        self.cops.values().map(|silenced| silenced.offenses).sum()
    }

    /// How many cops the baseline switches off.
    pub fn switched_off(&self) -> usize {
        let off = |silenced: &&Silenced| silenced.switched_off();
        self.cops.values().filter(off).count()
    }

    /// How many cops skip files.
    pub fn skipping(&self) -> usize {
        self.cops.len() - self.switched_off()
    }

    /// What the baseline covers, in one phrase.
    pub fn summary(&self) -> String {
        let (offenses, off, skipping) = (self.offenses(), self.switched_off(), self.skipping());
        format!(
            "{offenses} offense{} silenced, {off} cop{} switched off, {skipping} cop{} with skipped files",
            plural(offenses),
            plural(off),
            plural(skipping)
        )
    }

    /// The text of the `rustocop.toml`.
    pub fn toml(&self) -> String {
        let offenses = self.offenses();
        let mut toml = format!(
            "# Written by `rustocop --write-baseline`. It silences the {offenses} offense{} that\n\
             # the code had then: a cop with offenses in more than {FILE_LIMIT} files is switched\n\
             # off, and any other cop skips the files that had offenses. Delete an entry\n\
             # to see its offenses again.\n",
            plural(offenses)
        );
        for (cop, silenced) in &self.cops {
            let files = silenced.files.len();
            let _ = write!(
                toml,
                "\n# {} offense{} in {files} file{}\n[cops.\"{cop}\"]\n",
                silenced.offenses,
                plural(silenced.offenses),
                plural(files)
            );
            let skipped = if silenced.switched_off() {
                toml.push_str("enabled = false\n");
                if !silenced.switched_on.is_empty() {
                    toml.push_str(
                        "# A `rubocop:enable` comment switches the cop on in these files.\n",
                    );
                }
                &silenced.switched_on
            } else {
                &silenced.files
            };
            if !skipped.is_empty() {
                toml.push_str("exclude = [\n");
                for file in skipped {
                    let _ = writeln!(toml, "  {},", toml::Value::String(file.clone()));
                }
                toml.push_str("]\n");
            }
        }
        toml
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{RawConfig, ResolvedConfig},
        cops::Registry,
        offense::{GlobalOffense, Offense, Severity},
        pipeline::FileResult,
    };
    use std::path::PathBuf;

    fn result(path: &str, cops: &[&'static str]) -> FileResult {
        FileResult {
            path: PathBuf::from(path),
            offenses: cops
                .iter()
                .map(|cop| Offense {
                    correction: None,
                    cop,
                    severity: Severity::Convention,
                    message: String::new(),
                    line: 1,
                    end_line: 1,
                    col: 1,
                })
                .collect(),
            global_offenses: Vec::new(),
        }
    }

    #[test]
    fn a_cop_skips_few_files_and_is_switched_off_for_many() {
        let mut results: Vec<_> = (0..=FILE_LIMIT)
            .map(|n| result(&format!("/app/lib/f{n:02}.rb"), &["Style/StringLiterals"]))
            .collect();
        results[0]
            .offenses
            .push(result("", &["Lint/Debugger"]).offenses.remove(0));
        results[0]
            .offenses
            .push(result("", &["Lint/Debugger"]).offenses.remove(0));
        results[1].global_offenses.push(GlobalOffense {
            cop: "Naming/FileName",
            severity: Severity::Convention,
            message: String::new(),
        });
        let report = RunReport {
            results,
            ..RunReport::default()
        };
        let mut baseline = Baseline::default();
        baseline.add(&report, Path::new("/app"));
        assert_eq!(
            (
                baseline.offenses(),
                baseline.switched_off(),
                baseline.skipping()
            ),
            (FILE_LIMIT + 4, 1, 2)
        );
        let toml = baseline.toml();
        let entries = toml.split_once("\n\n").unwrap().1;
        assert_eq!(
            entries,
            "# 2 offenses in 1 file\n\
             [cops.\"Lint/Debugger\"]\n\
             exclude = [\n  \"lib/f00.rb\",\n]\n\
             \n\
             # 1 offense in 1 file\n\
             [cops.\"Naming/FileName\"]\n\
             exclude = [\n  \"lib/f01.rb\",\n]\n\
             \n\
             # 16 offenses in 16 files\n\
             [cops.\"Style/StringLiterals\"]\n\
             enabled = false\n"
        );
    }

    #[test]
    fn the_config_is_valid_and_its_patterns_match_only_the_named_files() {
        let names = ["lib/a[1].rb", "lib/{x,y}*?.rb", "lib/quo\"te\\.rb"];
        let report = RunReport {
            results: names
                .iter()
                .map(|name| result(name, &["Lint/Debugger"]))
                .collect(),
            ..RunReport::default()
        };
        let mut baseline = Baseline::default();
        baseline.add(&report, Path::new(""));
        let text = baseline.toml();
        let raw: RawConfig = toml::from_str(&text).unwrap();
        let config = ResolvedConfig::resolve(raw, None, &Registry::new()).unwrap();
        let excludes = &config.cop_settings["Lint/Debugger"].excludes;
        for name in names {
            assert!(
                crate::config::matches_any(excludes, Path::new(name)),
                "{name}"
            );
        }
        for other in ["lib/a1.rb", "lib/x.rb", "lib/xab.rb", "lib/quote.rb"] {
            assert!(
                !crate::config::matches_any(excludes, Path::new(other)),
                "{other}"
            );
        }
    }

    #[test]
    fn a_switched_off_cop_skips_the_files_where_it_still_reports() {
        let files = |range: std::ops::RangeInclusive<usize>| RunReport {
            results: range
                .map(|n| result(&format!("f{n:02}.rb"), &["Lint/Void"]))
                .collect(),
            ..RunReport::default()
        };
        let mut baseline = Baseline::default();
        baseline.add(&files(0..=FILE_LIMIT), Path::new(""));
        assert!(
            baseline
                .toml()
                .ends_with("[cops.\"Lint/Void\"]\nenabled = false\n")
        );
        baseline.add(&files(3..=3), Path::new(""));
        assert!(baseline.toml().ends_with(
            "# 17 offenses in 16 files\n\
             [cops.\"Lint/Void\"]\n\
             enabled = false\n\
             # A `rubocop:enable` comment switches the cop on in these files.\n\
             exclude = [\n  \"f03.rb\",\n]\n"
        ));
        assert_eq!(
            baseline.summary(),
            "17 offenses silenced, 1 cop switched off, 0 cops with skipped files"
        );
    }

    #[test]
    fn runs_add_up_and_no_offenses_give_no_entries() {
        let mut baseline = Baseline::default();
        baseline.add(&RunReport::default(), Path::new(""));
        assert_eq!((baseline.offenses(), baseline.skipping()), (0, 0));
        assert!(baseline.toml().lines().all(|line| line.starts_with('#')));
        for file in ["a.rb", "b.rb"] {
            let report = RunReport {
                results: vec![result(file, &["Lint/Debugger"])],
                ..RunReport::default()
            };
            baseline.add(&report, Path::new(""));
        }
        assert_eq!((baseline.offenses(), baseline.skipping()), (2, 1));
        assert!(baseline.toml().contains("  \"a.rb\",\n  \"b.rb\",\n"));
    }
}
