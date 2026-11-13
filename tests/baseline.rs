//! End-to-end tests of `--format offenses` and `--write-baseline`.

use std::{
    path::Path,
    process::{Command, Output},
};

/// A scratch project holding `files`, with a `.git` entry so that config
/// discovery stops there.
pub(super) fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".git")).unwrap();
    for (name, source) in files {
        let path = tmp.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    tmp
}

pub(super) fn rustocop(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    let text = |bytes| String::from_utf8(bytes).unwrap();
    (status.code().unwrap(), text(stdout), text(stderr))
}

const DEBUGGER: &str = "binding.pry\n";

#[test]
fn offenses_format_counts_offenses_per_cop_most_first() {
    let dir = project(&[
        ("a.rb", "puts 'a'\nputs 'b'\nbinding.pry\n"),
        ("b.rb", "puts 'c'\n"),
        ("clean.rb", "puts 1\n"),
    ]);
    let only = "Lint/Debugger,Style/StringLiterals";
    assert_eq!(
        rustocop(dir.path(), &["--format", "offenses", "--only", only]),
        (
            1,
            "3  Style/StringLiterals\n1  Lint/Debugger\n\n3 files inspected, 4 offenses detected\n"
                .into(),
            String::new()
        )
    );
    assert_eq!(
        rustocop(dir.path(), &["--format", "offenses", "clean.rb"]),
        (
            0,
            "\n1 file inspected, no offenses detected\n".into(),
            String::new()
        )
    );
}

#[test]
fn a_baseline_silences_the_offenses_of_today_and_new_code_is_still_checked() {
    // StringLiterals reports in 16 files, more than the limit; Debugger in
    // two, and FileName in the one whose name is not snake case.
    let quoted: Vec<_> = (0..16).map(|n| format!("lib/q{n:02}.rb")).collect();
    let mut files: Vec<_> = quoted
        .iter()
        .map(|name| (name.as_str(), "puts 'a'\n"))
        .collect();
    files.extend([("lib/debug.rb", DEBUGGER), ("lib/we[i]rd *.rb", DEBUGGER)]);
    let dir = project(&files);
    let config = dir.path().join("rustocop.toml");

    let (code, stdout, stderr) = rustocop(dir.path(), &["--write-baseline"]);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    assert_eq!(
        stdout,
        format!(
            "Wrote {}: 19 offenses silenced, 1 cop switched off, 2 cops with skipped files.\n",
            config.display()
        )
    );
    let written = std::fs::read_to_string(&config).unwrap();
    let entries = written.split_once("\n\n").unwrap().1;
    assert_eq!(
        entries,
        "# 2 offenses in 2 files\n\
         [cops.\"Lint/Debugger\"]\n\
         exclude = [\n  \"lib/debug.rb\",\n  \"lib/we[[]i[]]rd [*].rb\",\n]\n\
         \n\
         # 1 offense in 1 file\n\
         [cops.\"Naming/FileName\"]\n\
         exclude = [\n  \"lib/we[[]i[]]rd [*].rb\",\n]\n\
         \n\
         # 16 offenses in 16 files\n\
         [cops.\"Style/StringLiterals\"]\n\
         enabled = false\n"
    );

    assert_eq!(
        rustocop(dir.path(), &[]),
        (
            0,
            "\n18 files inspected, no offenses detected\n".into(),
            String::new()
        )
    );
    // The skipped files stay skipped; any other file is checked.
    std::fs::write(dir.path().join("lib/weird.rb"), DEBUGGER).unwrap();
    let (code, stdout, _) = rustocop(dir.path(), &[]);
    assert_eq!(code, 1);
    assert_eq!(
        stdout,
        "lib/weird.rb:1:1: E: Remove debugger entry point `binding.pry`. (Lint/Debugger)\n\
         \n19 files inspected, 1 offense detected\n"
    );

    // A baseline is added to the config that is there.
    let (code, stdout, stderr) = rustocop(dir.path(), &["--write-baseline"]);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    assert_eq!(
        stdout,
        format!(
            "Updated {}: 1 offense silenced, 0 cops switched off, 1 cop with skipped files.\n",
            config.display()
        )
    );
    let debugger = "exclude = [\n  \"lib/debug.rb\",\n  \"lib/we[[]i[]]rd [*].rb\",\n]\n";
    let updated = debugger.replace("]\n", "  \"lib/weird.rb\",\n]\n");
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        written.replacen(debugger, &updated, 1)
    );
    assert_eq!(rustocop(dir.path(), &[]).0, 0);
}

#[test]
fn a_baseline_also_silences_a_cop_that_an_enable_comment_switches_on() {
    // Debugger reports in 16 files and is switched off. In one of them, the
    // comment switches it on again for the last line.
    let names: Vec<_> = (0..16).map(|n| format!("d{n:02}.rb")).collect();
    let mut files: Vec<_> = names.iter().map(|name| (name.as_str(), DEBUGGER)).collect();
    files[3].1 = "binding.pry\n# rubocop:disable Lint/Debugger\nbinding.irb\n# rubocop:enable Lint/Debugger\nbinding.pry\n";
    let dir = project(&files);

    let (code, stdout, _) = rustocop(dir.path(), &["--write-baseline"]);
    assert_eq!(code, 0, "{stdout}");
    let written = std::fs::read_to_string(dir.path().join("rustocop.toml")).unwrap();
    assert!(
        written.contains(
            "[cops.\"Lint/Debugger\"]\n\
             enabled = false\n\
             # A `rubocop:enable` comment switches the cop on in these files.\n\
             exclude = [\n  \"d03.rb\",\n]\n"
        ),
        "{written}"
    );
    assert_eq!(
        rustocop(dir.path(), &[]),
        (
            0,
            "\n16 files inspected, no offenses detected\n".into(),
            String::new()
        )
    );
}

#[test]
fn no_baseline_is_written_for_clean_code_and_file_errors_stay_errors() {
    let dir = project(&[("clean.rb", "puts 1\n")]);
    assert_eq!(
        rustocop(dir.path(), &["--write-baseline"]),
        (
            0,
            "No offenses; no baseline is needed.\n".into(),
            String::new()
        )
    );
    assert!(!dir.path().join("rustocop.toml").exists());

    let dir = project(&[("debug.rb", DEBUGGER), ("broken.rb", "def (\n")]);
    let (code, stdout, stderr) = rustocop(dir.path(), &["--write-baseline"]);
    assert_eq!((code, stderr.as_str()), (2, ""), "{stdout}");
    assert!(
        stdout.starts_with("broken.rb: error: syntax error on line 1: "),
        "{stdout}"
    );
    assert!(
        stdout.ends_with(
            "rustocop.toml: 1 offense silenced, 0 cops switched off, 1 cop with skipped files.\n"
        ),
        "{stdout}"
    );
    let (code, stdout, _) = rustocop(dir.path(), &[]);
    assert_eq!(code, 2);
    assert!(
        stdout.contains("\n2 files inspected, no offenses detected, "),
        "{stdout}"
    );
}

#[test]
fn a_baseline_covers_the_given_paths_and_rejects_selecting_options() {
    let dir = project(&[("app/a.rb", DEBUGGER), ("lib/b.rb", DEBUGGER)]);
    for option in [
        &["--fix"][..],
        &["--only", "Lint"],
        &["--except", "Lint"],
        &["--stdin", "a.rb"],
    ] {
        let (code, stdout, stderr) =
            rustocop(dir.path(), &[&["--write-baseline"], option].concat());
        assert_eq!((code, stdout.as_str()), (2, ""), "{option:?}");
        assert!(stderr.contains("cannot be used with"), "{stderr}");
    }
    assert!(!dir.path().join("rustocop.toml").exists());

    let (code, _, _) = rustocop(dir.path(), &["--write-baseline", "app"]);
    assert_eq!(code, 0);
    let written = std::fs::read_to_string(dir.path().join("rustocop.toml")).unwrap();
    assert!(
        written.contains("\"app/a.rb\"") && !written.contains("lib/b.rb"),
        "{written}"
    );
}
