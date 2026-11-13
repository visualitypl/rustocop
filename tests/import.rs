//! End-to-end test of `--import-rubocop-config`.

use super::baseline::{project, rustocop};

#[test]
fn import_writes_the_config_of_the_next_run_and_lists_what_it_drops() {
    let dir = project(&[
        (
            ".rubocop.yml",
            "inherit_from: .rubocop_todo.yml\n\
             AllCops:\n  NewCops: enable\n  Exclude:\n    - 'gen/**/*'\n\
             Style/StringLiterals:\n  EnforcedStyle: single_quotes\n",
        ),
        (
            ".rubocop_todo.yml",
            "Lint/Debugger:\n  Exclude:\n    - 'lib/old.rb'\n",
        ),
        ("gen/a.rb", "binding.pry\n"),
        ("lib/old.rb", "binding.pry\n"),
        ("lib/new.rb", "binding.pry\n"),
    ]);
    let import = ["--import-rubocop-config"];
    assert_eq!(
        rustocop(dir.path(), &import),
        (
            0,
            "Wrote rustocop.toml from .rubocop_todo.yml and .rubocop.yml.\n\
             Not carried over; rustocop has no setting for these:\n  \
             AllCops: NewCops\n  \
             Style/StringLiterals: EnforcedStyle\n"
                .into(),
            String::new()
        )
    );
    let config = dir.path().join("rustocop.toml");
    let written = std::fs::read_to_string(&config).unwrap();
    assert_eq!(
        written,
        "# Written by `rustocop --import-rubocop-config` from .rubocop_todo.yml and .rubocop.yml.\n\
         exclude = [\n  \"gen/**/*\",\n]\n\
         \n\
         [cops.\"Lint/Debugger\"]\n\
         exclude = [\n  \"lib/old.rb\",\n]\n"
    );

    // The next run skips what RuboCop skipped.
    let (code, stdout, stderr) = rustocop(dir.path(), &["--only", "Lint/Debugger"]);
    assert_eq!((code, stderr.as_str()), (1, ""));
    assert!(stdout.contains("lib/new.rb:1:1"), "{stdout}");
    assert!(
        stdout.ends_with("2 files inspected, 1 offense detected\n"),
        "{stdout}"
    );

    // A config is never changed.
    assert_eq!(
        rustocop(dir.path(), &import),
        (
            2,
            String::new(),
            "error: rustocop.toml already exists; --import-rubocop-config does not change an existing config\n".into()
        )
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), written);

    // A baseline silences what the imported config still reports.
    let (code, stdout, stderr) = rustocop(dir.path(), &["--write-baseline"]);
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    assert_eq!(
        stdout,
        format!(
            "Updated {}: 1 offense silenced, 0 cops switched off, 1 cop with skipped files.\n",
            config.display()
        )
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        written.replace(
            "  \"lib/old.rb\",\n",
            "  \"lib/old.rb\",\n  \"lib/new.rb\",\n"
        )
    );
    assert_eq!(rustocop(dir.path(), &[]).0, 0);
}

#[test]
fn import_writes_nothing_without_settings_to_carry_over_and_needs_a_rubocop_config() {
    let dir = project(&[(
        ".rubocop.yml",
        "Style/StringLiterals:\n  EnforcedStyle: single_quotes\n",
    )]);
    assert_eq!(
        rustocop(dir.path(), &["--import-rubocop-config"]),
        (
            0,
            "No setting of .rubocop.yml has a rustocop counterpart; no rustocop.toml is written.\n\
             Not carried over; rustocop has no setting for these:\n  \
             Style/StringLiterals: EnforcedStyle\n"
                .into(),
            String::new()
        )
    );
    assert!(!dir.path().join("rustocop.toml").exists());

    let empty = project(&[("a.rb", "puts 1\n")]);
    let (code, stdout, stderr) = rustocop(empty.path(), &["--import-rubocop-config"]);
    assert_eq!((code, stdout.as_str()), (2, ""));
    assert!(
        stderr.starts_with("error: could not read .rubocop.yml: "),
        "{stderr}"
    );
    let (code, _, stderr) = rustocop(empty.path(), &["--import-rubocop-config", "--fix"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("cannot be used with"), "{stderr}");
}
