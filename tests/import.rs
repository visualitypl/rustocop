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
             AllCops: NewCops\n"
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
         exclude = [\n  \"lib/old.rb\",\n]\n\
         \n\
         [cops.\"Style/StringLiterals\"]\n\
         enforced_style = \"single_quotes\"\n\
         \n\
         [cops.\"Style/StringLiteralsInInterpolation\"]\n\
         enforced_style = \"single_quotes\"\n"
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
        "inherit_gem:\n  standard: config/base.yml\n\
         Style/HashSyntax:\n  EnforcedShorthandSyntax: always\n",
    )]);
    assert_eq!(
        rustocop(dir.path(), &["--import-rubocop-config"]),
        (
            0,
            "No setting of .rubocop.yml has a rustocop counterpart; no rustocop.toml is written.\n\
             Not carried over; rustocop has no setting for these:\n  \
             inherit_gem\n  \
             Style/HashSyntax: EnforcedShorthandSyntax\n"
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

#[test]
fn a_reader_that_closes_the_pipe_ends_the_output_without_a_panic() {
    use std::process::{Command, Stdio};

    // Each output is more than a pipe holds, so the write waits for the
    // reader and then fails.
    let cops: String = (0..8000)
        .map(|n| format!("Nope/Cop{n}:\n  Enabled: false\n"))
        .collect();
    let dir = project(&[
        (".rubocop.yml", &cops),
        ("a.rb", &"binding.pry\n".repeat(2000)),
    ]);
    for (args, code) in [
        (&["--import-rubocop-config"][..], 0),
        (&["--only", "Lint/Debugger"], 1),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .args(args)
            .current_dir(dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            (
                output.status.code(),
                String::from_utf8(output.stderr).unwrap().as_str()
            ),
            (Some(code), ""),
            "{args:?}"
        );
    }
}

#[test]
fn an_imported_config_has_the_quotes_of_rubocop() {
    // A config that names no quote style means RuboCop's default, single
    // quotes, for strings and for the symbols that follow them.
    let source = "x = \"a\"\ny = 'b'\nz = :\"c d\"\n";
    let dir = project(&[
        (".rubocop.yml", "AllCops:\n  NewCops: enable\n"),
        ("a.rb", source),
    ]);
    let only = "Style/StringLiterals,Style/QuotedSymbols";
    let before = rustocop(dir.path(), &["--only", only]).1;
    assert!(
        before.starts_with("a.rb:2:5: C: Prefer double-quoted strings"),
        "{before}"
    );

    assert_eq!(rustocop(dir.path(), &["--import-rubocop-config"]).0, 0);
    assert_eq!(
        rustocop(dir.path(), &["--only", only]),
        (
            1,
            "a.rb:1:5: C: Prefer single-quoted strings when you don't need string interpolation \
             or special symbols. (Style/StringLiterals)\n\
             a.rb:3:5: C: Prefer single-quoted symbols when you don't need string interpolation \
             or special symbols. (Style/QuotedSymbols)\n\
             \n1 file inspected, 2 offenses detected\n"
                .into(),
            String::new()
        )
    );
    assert_eq!(rustocop(dir.path(), &["--fix", "--only", only]).0, 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        "x = 'a'\ny = 'b'\nz = :'c d'\n"
    );

    // A style that the cop does not have is an error before any linting.
    let config = "[cops.\"Style/StringLiterals\"]\nenforced_style = \"backticks\"\n";
    std::fs::write(dir.path().join("rustocop.toml"), config).unwrap();
    let (code, _, stderr) = rustocop(dir.path(), &[]);
    assert_eq!(code, 2);
    assert_eq!(
        stderr,
        "error: unknown `enforced_style = \"backticks\"` for `Style/StringLiterals`; use \
         \"double_quotes\" or \"single_quotes\"\n"
    );
    let config = "[cops.\"Lint/Debugger\"]\nenforced_style = \"single_quotes\"\n";
    std::fs::write(dir.path().join("rustocop.toml"), config).unwrap();
    let (code, _, stderr) = rustocop(dir.path(), &[]);
    assert_eq!(code, 2);
    assert_eq!(
        stderr,
        "error: `enforced_style` is not a valid setting for `Lint/Debugger`\n"
    );
}

#[test]
fn allowed_patterns_accept_the_long_lines_that_match_one() {
    let source = format!("# {}\nx = {}1\n", "c".repeat(130), "1 + ".repeat(40));
    let dir = project(&[("a.rb", &source)]);
    let long_lines = |config: &str| {
        std::fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let (code, stdout, stderr) = rustocop(dir.path(), &["--only", "Layout/LineLength"]);
        assert_eq!(stderr, "", "{config}");
        let found = stdout
            .lines()
            .filter(|line| line.contains("Layout/LineLength"));
        (code, found.map(str::to_owned).collect::<Vec<_>>())
    };
    let table = "[cops.\"Layout/LineLength\"]\n";
    assert_eq!(long_lines("").1.len(), 2);
    // The comment line matches the pattern; the other long line is reported.
    let (code, found) = long_lines(&format!("{table}allowed_patterns = ['^\\s*#']\n"));
    assert_eq!(code, 1);
    assert_eq!(
        found,
        ["a.rb:2:121: C: Line is too long. [165/120] (Layout/LineLength)"]
    );
    // The patterns and `max` hold together, in either order.
    for config in [
        format!("{table}max = 100\nallowed_patterns = ['^\\s*#']\n"),
        format!("{table}allowed_patterns = ['^\\s*#']\nmax = 100\n"),
    ] {
        let found = long_lines(&config).1;
        assert_eq!(
            found,
            ["a.rb:2:101: C: Line is too long. [165/100] (Layout/LineLength)"]
        );
    }
    assert_eq!(
        long_lines(&format!("{table}allowed_patterns = ['^#', '1 \\+ 1$']\n")),
        (0, vec![])
    );

    // A pattern that the regex syntax does not have, and a cop without the
    // setting, are errors before any linting.
    let config = format!("{table}allowed_patterns = ['(?<=a)b']\n");
    std::fs::write(dir.path().join("rustocop.toml"), config).unwrap();
    let (code, _, stderr) = rustocop(dir.path(), &[]);
    assert_eq!(code, 2);
    let expected = "error: invalid `allowed_patterns` entry `(?<=a)b` for `Layout/LineLength`";
    assert!(stderr.starts_with(expected), "{stderr}");
    let config = "[cops.\"Lint/Debugger\"]\nallowed_patterns = ['x']\n";
    std::fs::write(dir.path().join("rustocop.toml"), config).unwrap();
    assert_eq!(
        rustocop(dir.path(), &[]),
        (
            2,
            String::new(),
            "error: `allowed_patterns` is not a valid setting for `Lint/Debugger`\n".into()
        )
    );
}

#[test]
fn an_imported_omakase_config_lints_as_rubocop_does() {
    // The settings of rubocop-rails-omakase 1.1.0 that a rustocop.toml can say.
    let omakase = "Layout/CaseIndentation:\n  Enabled: true\n  EnforcedStyle: end\n\
                   Layout/EndAlignment:\n  Enabled: true\n  EnforcedStyleAlignWith: variable\n\
                   Layout/SpaceInsideArrayLiteralBrackets:\n  Enabled: true\n  \
                   EnforcedStyle: space\n  EnforcedStyleForEmptyBrackets: no_space\n\
                   Layout/SpaceInsideBlockBraces:\n  Enabled: true\n  \
                   EnforcedStyleForEmptyBraces: space\n\
                   Style/RedundantReturn:\n  Enabled: true\n  AllowMultipleReturnValues: true\n\
                   Style/Semicolon:\n  Enabled: true\n  AllowAsExpressionSeparator: true\n";
    let good = "result = if ready?\n  1\nelse\n  2\nend\n\n[ 1, 2 ].each { }\n\n\
                def pair\n  return 1, 2\nend\n\nx = 1; y = 2\n\n\
                output = case result\nwhen 1 then :one\nend\n";
    let dir = project(&[
        (".rubocop.yml", omakase),
        ("good.rb", good),
        ("bad.rb", "[1, 2]\n[ ]\neach {}\n"),
    ]);
    assert_eq!(rustocop(dir.path(), &["--import-rubocop-config"]).0, 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("rustocop.toml")).unwrap(),
        "# Written by `rustocop --import-rubocop-config` from .rubocop.yml.\n\
         \n[cops.\"Layout/CaseIndentation\"]\nenabled = true\nenforced_style = \"end\"\n\
         \n[cops.\"Layout/EndAlignment\"]\nenabled = true\nenforced_style_align_with = \"variable\"\n\
         \n[cops.\"Layout/SpaceInsideArrayLiteralBrackets\"]\nenabled = true\n\
         enforced_style = \"space\"\nenforced_style_for_empty_brackets = \"no_space\"\n\
         \n[cops.\"Layout/SpaceInsideBlockBraces\"]\nenabled = true\n\
         enforced_style_for_empty_braces = \"space\"\n\
         \n[cops.\"Style/RedundantReturn\"]\nallow_multiple_return_values = true\nenabled = true\n\
         \n[cops.\"Style/Semicolon\"]\nallow_as_expression_separator = true\nenabled = true\n\
         \n[cops.\"Style/StringLiterals\"]\nenforced_style = \"single_quotes\"\n\
         \n[cops.\"Style/StringLiteralsInInterpolation\"]\nenforced_style = \"single_quotes\"\n"
    );
    let only = "Layout/CaseIndentation,Layout/EndAlignment,Layout/ElseAlignment,\
                Layout/IndentationWidth,Layout/SpaceInsideArrayLiteralBrackets,\
                Layout/SpaceInsideBlockBraces,Style/RedundantReturn,Style/Semicolon";
    assert_eq!(
        rustocop(dir.path(), &["--only", only]),
        (
            1,
            "bad.rb:1:1: C: Use space inside array brackets. \
             (Layout/SpaceInsideArrayLiteralBrackets)\n\
             bad.rb:1:6: C: Use space inside array brackets. \
             (Layout/SpaceInsideArrayLiteralBrackets)\n\
             bad.rb:2:1: C: Do not use space inside empty array brackets. \
             (Layout/SpaceInsideArrayLiteralBrackets)\n\
             bad.rb:3:6: C: Space missing inside empty braces. (Layout/SpaceInsideBlockBraces)\n\
             \n2 files inspected, 4 offenses detected\n"
                .into(),
            String::new()
        )
    );
    assert_eq!(rustocop(dir.path(), &["-a", "--only", only]).0, 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("bad.rb")).unwrap(),
        "[ 1, 2 ]\n[]\neach { }\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("good.rb")).unwrap(),
        good
    );
}
