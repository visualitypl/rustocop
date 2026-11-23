//! End-to-end tests of the `rustocop` binary.

use std::{
    io::Write as _,
    path::Path,
    process::{Command, Output, Stdio},
};

/// A scratch project: a temporary directory holding `files`, with a `.git`
/// entry so config discovery stops there instead of finding a stray
/// `rustocop.toml` further up.
fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".git")).unwrap();
    for (name, source) in files {
        let path = tmp.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    tmp
}

/// Run the binary in `dir` with `args`, feeding it `stdin`.
fn rustocop(dir: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Err(error) = child.stdin.take().unwrap().write_all(stdin.as_bytes()) {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
    }
    child.wait_with_output().unwrap()
}

fn stdout_lines(output: &Output) -> Vec<String> {
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn local_v1_version_exits_without_linting_or_loading_configuration() {
    let dir = project(&[
        ("rustocop.toml", "unknown = true\n"),
        ("broken.rb", "def (\n"),
    ]);
    for flag in ["--version", "-V"] {
        let output = rustocop(dir.path(), &[flag], "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(output.stdout, b"rustocop 1.5.0\n");
        assert_eq!(
            std::fs::read(dir.path().join("broken.rb")).unwrap(),
            b"def (\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("rustocop.toml")).unwrap(),
            b"unknown = true\n"
        );
    }
}

#[test]
fn ruby_version_in_the_config_selects_the_grammar_and_the_cops() {
    let source = "require 'pathname'\nlist.reverse.find { |x| x }\nfoo\n  && bar\n";
    let cops = "Lint/RedundantRequireStatement,Style/ReverseFind";
    let lint = |config: &str| {
        let dir = project(&[("a.rb", source), ("rustocop.toml", config)]);
        let output = rustocop(dir.path(), &["--only", cops, "a.rb"], "");
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };

    for config in ["", "ruby_version = \"3.4\"\n"] {
        let (code, stdout, stderr) = lint(config);
        assert_eq!(code, Some(2), "{stdout}{stderr}");
        assert!(
            stdout.contains("a.rb: error: syntax error on line 4:"),
            "{stdout}"
        );
        assert!(
            !stdout.contains("Lint/RedundantRequireStatement"),
            "{stdout}"
        );
        assert!(!stdout.contains("Style/ReverseFind"), "{stdout}");
    }

    let (code, stdout, stderr) = lint("ruby_version = \"4.0\"\n");
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(!stdout.contains("syntax error"), "{stdout}");
    assert!(
        stdout.contains(
            "a.rb:1:1: E: Remove unnecessary `require` statement. (Lint/RedundantRequireStatement)"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("a.rb:2:6: C: Use `rfind` instead. (Style/ReverseFind)"),
        "{stdout}"
    );

    for unsupported in ["3.3", "4.1", "latest"] {
        let (code, stdout, stderr) = lint(&format!("ruby_version = \"{unsupported}\"\n"));
        assert_eq!(code, Some(2), "{stdout}{stderr}");
        assert!(stdout.is_empty(), "{stdout}");
        assert!(
            stderr.contains(&format!("unsupported `ruby_version = \"{unsupported}\"`")),
            "{stderr}"
        );
        assert!(stderr.contains("use \"3.4\" or \"4.0\""), "{stderr}");
    }
}

#[test]
fn the_ruby_version_comes_from_project_files_unless_the_config_sets_it() {
    let lock_4_0 = "GEM\n  specs:\n\nRUBY VERSION\n   ruby 4.0.9p0\n";
    let accepts_ruby_4 = |files: &[(&str, &str)], cwd: &str| {
        let mut all = vec![("app/a.rb", "foo\n  && bar\n")];
        all.extend_from_slice(files);
        let dir = project(&all);
        let output = rustocop(&dir.path().join(cwd), &["--only", "Lint/Debugger"], "");
        let stdout = String::from_utf8(output.stdout).unwrap();
        match output.status.code() {
            Some(0) => {
                assert!(!stdout.contains("syntax error"), "{stdout}");
                true
            }
            code => {
                assert_eq!(code, Some(2), "{stdout}");
                assert!(stdout.contains("syntax error on line 2:"), "{stdout}");
                false
            }
        }
    };

    assert!(!accepts_ruby_4(&[], ""));
    assert!(accepts_ruby_4(&[(".ruby-version", "4.0.9\n")], ""));
    assert!(accepts_ruby_4(&[(".ruby-version", "4.0.9\n")], "app"));
    assert!(accepts_ruby_4(&[(".tool-versions", "ruby 4.0.9\n")], ""));
    assert!(accepts_ruby_4(
        &[("mise.toml", "[tools]\nruby = \"4.0\"\n")],
        ""
    ));
    assert!(accepts_ruby_4(&[("Gemfile.lock", lock_4_0)], ""));
    assert!(accepts_ruby_4(
        &[
            (".ruby-version", "4.0.9\n"),
            ("rustocop.toml", "exclude = []\n")
        ],
        ""
    ));

    assert!(!accepts_ruby_4(&[(".ruby-version", "3.4.1\n")], ""));
    assert!(!accepts_ruby_4(&[(".ruby-version", "3.2.2\n")], ""));
    assert!(!accepts_ruby_4(&[(".ruby-version", "jruby-9.4.8.0\n")], ""));
    assert!(!accepts_ruby_4(
        &[(".ruby-version", "3.4.1\n"), ("Gemfile.lock", lock_4_0)],
        ""
    ));
    assert!(!accepts_ruby_4(
        &[
            (".ruby-version", "4.0.9\n"),
            ("rustocop.toml", "ruby_version = \"3.4\"\n")
        ],
        ""
    ));
    assert!(accepts_ruby_4(
        &[
            (".ruby-version", "3.4.1\n"),
            ("rustocop.toml", "ruby_version = \"4.0\"\n")
        ],
        ""
    ));
}

#[test]
fn show_ruby_version_prints_the_version_and_its_source_without_linting() {
    let show = |files: &[(&str, &str)], cwd: &str| {
        let mut all = vec![("app/a.rb", "def (\n")];
        all.extend_from_slice(files);
        let dir = project(&all);
        let root = dir.path().canonicalize().unwrap();
        let output = rustocop(&root.join(cwd), &["--show-ruby-version"], "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        String::from_utf8(output.stdout)
            .unwrap()
            .replace(root.to_str().unwrap(), "ROOT")
    };

    assert_eq!(
        show(&[], ""),
        "Ruby 3.4 (default; no file declares a version)\n"
    );
    assert_eq!(
        show(&[(".ruby-version", "4.0.9\n")], "app"),
        "Ruby 4.0 (ROOT/.ruby-version declares 4.0.9)\n"
    );
    assert_eq!(
        show(&[(".tool-versions", "ruby 3.2.2\n")], ""),
        "Ruby 3.4 (ROOT/.tool-versions declares 3.2.2)\n"
    );
    assert_eq!(
        show(
            &[
                (".ruby-version", "3.4.1\n"),
                ("rustocop.toml", "ruby_version = \"4.0\"\n")
            ],
            "app"
        ),
        "Ruby 4.0 (ruby_version in ROOT/rustocop.toml)\n"
    );

    let dir = project(&[("a.rb", "x\n")]);
    for args in [
        vec!["--show-ruby-version", "--fix"],
        vec!["--show-ruby-version", "--stdin", "a.rb"],
    ] {
        let output = rustocop(dir.path(), &args, "");
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
    }
}

#[test]
fn ruby_4_logical_chains_are_syntax_errors_for_files_stdin_and_fix() {
    for operator in ["&&", "||"] {
        let source = format!("foo\n  {operator} bar\nfail\n");
        let dir = project(&[("a.rb", &source), ("other.rb", "fail\n")]);
        for args in [
            vec!["--only", "Style/SignalException", "a.rb"],
            vec!["--only", "Style/SignalException", "--stdin", "a.rb"],
            vec![
                "--only",
                "Style/SignalException",
                "--fix",
                "a.rb",
                "other.rb",
            ],
        ] {
            let output = rustocop(dir.path(), &args, &source);
            assert_eq!(output.status.code(), Some(2), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(stdout.contains("syntax error on line 2:"), "{stdout}");
            let token = if operator == "||" { "|" } else { "&&" };
            assert!(
                stdout.contains(&format!("unexpected '{token}'")),
                "{stdout}"
            );
            assert!(!stdout.contains("a.rb:3:"), "cops must not run: {stdout}");
            assert_eq!(
                std::fs::read(dir.path().join("a.rb")).unwrap(),
                source.as_bytes()
            );
            if args.contains(&"--fix") {
                assert_eq!(
                    std::fs::read(dir.path().join("other.rb")).unwrap(),
                    b"raise\n"
                );
            }
        }
    }
}

#[test]
fn ruby_34_it_forwarding_and_necessary_logical_continuations_survive_fix_validation() {
    let source = "items.map { it }\ndef forward(...); target(...); end\ndef anonymous(*, **, &); target(*, **, &); end\nfoo &&\n  bar\nfoo \\\n  || bar\ntarget(1, \\\n 2)\nfail\n";
    let expected = source
        .replace(", \\\n", ", \n")
        .replace("fail\n", "raise\n");
    let dir = project(&[("a.rb", source)]);
    let only = "Style/SignalException,Style/RedundantLineContinuation";
    let output = rustocop(dir.path(), &["--only", only, "--stdin", "a.rb"], source);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    for _ in 0..2 {
        let output = rustocop(dir.path(), &["--only", only, "--fix", "a.rb"], "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            std::fs::read(dir.path().join("a.rb")).unwrap(),
            expected.as_bytes()
        );
    }
}

#[test]
fn ruby34_nested_english_not_survives_files_stdin_and_fix_validation() {
    if crate::support::runtime::skips("asserts the Ruby 3.4 grammar, which Ruby 4.0 changed") {
        return;
    }
    let source =
        "def probe(x); x; end\np [not probe false]\nr = case\nwhen [not false]\n :yes\nend\np r\n";
    let expected = source.replace("case\nwhen [not false]", "if ([not false])");
    let dir = project(&[("a.rb", source)]);
    for args in [
        vec!["--only", "Style/EmptyCaseCondition", "a.rb"],
        vec!["--only", "Style/EmptyCaseCondition", "--stdin", "a.rb"],
    ] {
        let output = rustocop(dir.path(), &args, source);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("Style/EmptyCaseCondition")
        );
        assert_eq!(
            std::fs::read(dir.path().join("a.rb")).unwrap(),
            source.as_bytes()
        );
    }
    for _ in 0..2 {
        let output = rustocop(
            dir.path(),
            &["--only", "Style/EmptyCaseCondition", "--fix", "a.rb"],
            "",
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            std::fs::read(dir.path().join("a.rb")).unwrap(),
            expected.as_bytes()
        );
    }
    if let Some(ruby) = crate::support::runtime::ruby_os() {
        for program in [source, &expected] {
            let output = Command::new(&ruby)
                .env_remove("RUBYOPT")
                .env_remove("RUBYLIB")
                .args(["--parser=prism", "-e", program])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(output.stdout, b"[true]\n:yes\n");
        }
    }
}

#[test]
fn excessive_ast_depth_is_a_file_error_and_fix_never_writes_it() {
    let mut deep = String::from("# rustocop:disable all\n");
    for i in 0..400 {
        deep.push_str(&format!("def foo{i}(*args)\nbar(*args)  \n"));
    }
    deep.push_str(&"end\n".repeat(400));
    assert_eq!(ruby_prism::parse(deep.as_bytes()).errors().count(), 0);
    let dir = project(&[("deep.rb", &deep), ("other.rb", "binding.pry\n")]);
    for args in [
        vec!["--only", "Style/ArgumentsForwarding", "deep.rb"],
        vec!["--fix", "deep.rb", "other.rb"],
    ] {
        let output = rustocop(dir.path(), &args, "");
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("deep.rb: error: AST exceeds maximum depth of 64"),
            "{stdout}"
        );
        if args.contains(&"other.rb") {
            assert!(stdout.contains("Lint/Debugger"), "{stdout}");
        }
        assert_eq!(
            std::fs::read(dir.path().join("deep.rb")).unwrap(),
            deep.as_bytes()
        );
    }
    let output = rustocop(dir.path(), &["--stdin", "deep.rb"], &deep);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("AST exceeds maximum depth of 64")
    );
}

#[test]
fn parser_stack_is_bounded_on_default_parallel_workers() {
    for levels in [1000, 12_000] {
        let source = format!(
            "{}{}",
            "def foo(*args)\nbar(*args)  \n".repeat(levels),
            "end\n".repeat(levels)
        );
        let dir = project(&[
            ("a.rb", &source),
            ("b.rb", &source),
            ("c.rb", &source),
            ("d.rb", &source),
            ("other.rb", "binding.pry\nfail\n"),
        ]);
        for fix in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command
                .current_dir(dir.path())
                .env("RAYON_NUM_THREADS", "2")
                .args(["--only", "Lint/Debugger,Style/SignalException"]);
            if fix {
                command.arg("--fix");
            }
            let output = command
                .args(["a.rb", "b.rb", "c.rb", "d.rb", "other.rb"])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(stdout.contains("Lint/Debugger"), "{stdout}");
            let diagnostic = if levels == 1000 {
                "AST exceeds maximum depth of 64"
            } else {
                "syntax error on line"
            };
            for name in ["a.rb", "b.rb", "c.rb", "d.rb"] {
                assert!(stdout.contains(&format!("{name}: error: {diagnostic}")));
                assert_eq!(
                    std::fs::read(dir.path().join(name)).unwrap(),
                    source.as_bytes()
                );
            }
            assert_eq!(
                std::fs::read_to_string(dir.path().join("other.rb")).unwrap(),
                if fix {
                    "binding.pry\nraise\n"
                } else {
                    "binding.pry\nfail\n"
                }
            );
        }
        let output = rustocop(dir.path(), &["--stdin", "deep.rb"], &source);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn parser_budget_also_bounds_patterns_and_destructured_parameters() {
    for source in [
        // Escaped newlines preserve the recursive token sequence while keeping
        // diagnostic column mapping cheap for these large rejected inputs.
        format!("x in {}b\n", "a:\\\n".repeat(200_000)),
        format!("x in {}b\n", "**rest,a:\\\n".repeat(200_000)),
        format!(
            "value in {}capture{}\n",
            "[".repeat(12_000),
            "]".repeat(12_000)
        ),
        format!(
            "def foo({}arg{}); end\n",
            "(".repeat(12_000),
            ")".repeat(12_000)
        ),
    ] {
        let dir = project(&[("deep.rb", &source), ("other.rb", "binding.pry\n")]);
        let output = rustocop(dir.path(), &["--fix", "deep.rb"], "");
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("nesting too deep"),
            "{}",
            &stdout[..stdout.len().min(500)]
        );
        assert_eq!(
            std::fs::read(dir.path().join("deep.rb")).unwrap(),
            source.as_bytes()
        );

        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .env("RAYON_NUM_THREADS", "2")
            .args(["--only", "Lint/Debugger", "."])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("nesting too deep"), "{stdout}");
        assert!(stdout.contains("Lint/Debugger"), "{stdout}");
    }
}

#[test]
fn long_rejected_lines_map_all_diagnostics_within_a_deadline() {
    // Redirect to files, not pipes: the many diagnostics must not block the
    // child before the parent can enforce the deadline and kill/reap it.
    fn bounded_run(dir: &Path) -> String {
        use std::time::{Duration, Instant};

        let stdout = tempfile::NamedTempFile::new().unwrap();
        let stderr = tempfile::NamedTempFile::new().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir)
            .env("RAYON_NUM_THREADS", "2")
            .args([
                "--fix",
                "--format",
                "github",
                "--only",
                "Lint/Debugger",
                "deep.rb",
                "other.rb",
            ])
            .stdin(Stdio::null())
            .stdout(stdout.reopen().unwrap())
            .stderr(stderr.reopen().unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("long-line diagnostic mapping exceeded 20 seconds");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(status.code(), Some(2));
        assert!(std::fs::read(stderr.path()).unwrap().is_empty());
        std::fs::read_to_string(stdout.path()).unwrap()
    }

    const REPEATS: usize = 200_000;
    for prefix in ["a:", "**rest,a:"] {
        // These are the same Ruby tokens. The escaped physical newlines make
        // the short-line reference mapping cheap, without a second parser oracle
        // or a snapshot of thousands of version-dependent Prism messages.
        let reference = format!("x in {}b\n", format!("{prefix}\\\n").repeat(REPEATS));
        let source = format!("x in {}b\n", prefix.repeat(REPEATS));
        let dir = project(&[("deep.rb", &reference), ("other.rb", "binding.pry\n")]);
        let reference_output = bounded_run(dir.path());
        let mut expected = String::new();
        let mut diagnostics = 0;
        for line in reference_output.lines() {
            if let Some(rest) = line.strip_prefix("::error file=deep.rb,line=") {
                let (row, rest) = rest.split_once(",col=").unwrap();
                let (col, data) = rest.split_once(",title=rustocop::").unwrap();
                let row: usize = row.parse().unwrap();
                let col: usize = col.parse().unwrap();
                let (original_row, original_col) = if row == REPEATS + 2 {
                    (2, col)
                } else {
                    (1, col + (row - 1) * prefix.len() + usize::from(row > 1) * 5)
                };
                let message = data
                    .strip_prefix(&format!("deep.rb: error: syntax error on line {row}: "))
                    .unwrap();
                expected.push_str(&format!(
                    "::error file=deep.rb,line={original_row},col={original_col},title=rustocop::deep.rb: error: syntax error on line {original_row}: {message}\n"
                ));
                diagnostics += 1;
            } else {
                expected.push_str(line);
                expected.push('\n');
            }
        }
        drop(reference_output);
        assert!(diagnostics > 10_000, "must exercise many diagnostics");
        assert!(expected.contains("nesting too deep"));
        assert!(expected.contains("title=Lint/Debugger"));
        std::fs::write(dir.path().join("deep.rb"), &source).unwrap();
        let actual = bounded_run(dir.path());
        // Compare incrementally so a failure cannot dump megabytes of output.
        assert_eq!(actual.lines().count(), expected.lines().count());
        for (actual, expected) in actual.lines().zip(expected.lines()) {
            assert_eq!(actual, expected, "{prefix}");
        }
        assert_eq!(
            std::fs::read(dir.path().join("deep.rb")).unwrap(),
            source.as_bytes()
        );
        assert_eq!(
            std::fs::read(dir.path().join("other.rb")).unwrap(),
            b"binding.pry\n"
        );
    }
}

#[test]
fn rejected_receiver_chains_are_destroyed_safely_and_other_files_are_linted() {
    let deep = format!("root{}\n", ".foo".repeat(5000));
    // A trailing syntax error must retain syntax-diagnostic precedence without
    // overflowing during cleanup of the recovered receiver chain either.
    for source in [&deep, &format!("{deep}def\n")] {
        let dir = project(&[
            ("a.rb", source),
            ("b.rb", source),
            ("c.rb", source),
            ("d.rb", source),
            ("other.rb", "binding.pry\n"),
        ]);
        for fix in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command
                .current_dir(dir.path())
                .env("RAYON_NUM_THREADS", "2")
                .args(["--only", "Lint/Debugger"]);
            if fix {
                command.arg("--fix");
            }
            let output = command
                .args(["a.rb", "b.rb", "c.rb", "d.rb", "other.rb"])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let stdout = String::from_utf8(output.stdout).unwrap();
            let diagnostic = if source == &deep {
                "AST exceeds maximum depth of 64"
            } else {
                "syntax error on line"
            };
            assert!(stdout.contains("Lint/Debugger"), "{stdout}");
            for name in ["a.rb", "b.rb", "c.rb", "d.rb"] {
                assert!(
                    stdout.contains(&format!("{name}: error: {diagnostic}")),
                    "{stdout}"
                );
                assert_eq!(
                    std::fs::read(dir.path().join(name)).unwrap(),
                    source.as_bytes()
                );
            }
        }
        let output = rustocop(dir.path(), &["--stdin", "chain.rb"], source);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn definition_spacing_default_lint_is_read_only_and_fix_preserves_exact_bytes() {
    let cop = "Layout/EmptyLineBetweenDefs";
    for format in ["text", "github"] {
        let before = "\u{feff}# 日本語\r\ndef café\r\n  \"é\"\r\nend # café\r\n# thé documentation\r\ndef thé\r\n  \"thé\"\r\nend\r\n__END__\r\nraw\n\n";
        let after = before.replace("end # café\r\n", "end # café\r\n\r\n");
        let dir = project(&[("x.rb", before)]);
        let path = dir.path().join("x.rb");
        let plain = rustocop(dir.path(), &["--format", format, "x.rb"], "");
        assert_eq!(plain.status.code(), Some(1), "{plain:?}");
        assert!(plain.stderr.is_empty());
        assert!(stdout_lines(&plain).iter().any(|line| line.contains(cop)));
        assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", cop, "--format", format, "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert!(fixed.stderr.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 1 correction applied, 0 offenses remaining"
        );
        let again = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
        assert_eq!(again.status.code(), Some(0), "{again:?}");
        assert_eq!(
            stdout_lines(&again).last().unwrap(),
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        );
    }
    let before =
        b"# coding: ascii-8bit\r\ndef a\r\n  \"\xff\"\r\nend\r\ndef b\r\nend\r\n__END__\n\xff\n";
    let after = b"# coding: ascii-8bit\r\ndef a\r\n  \"\xff\"\r\nend\r\n\r\ndef b\r\nend\r\n__END__\n\xff\n";
    let dir = project(&[]);
    let path = dir.path().join("x.rb");
    std::fs::write(&path, before).unwrap();
    let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    assert_eq!(std::fs::read(&path).unwrap(), after);
}

#[test]
fn definition_spacing_configuration_selections_and_directives() {
    let cop = "Layout/EmptyLineBetweenDefs";
    // Nonempty definitions isolate Layout enablement from default-on EmptyMethod.
    let before = "def a\n  1\nend\ndef b\n  2\nend\n";
    let after = "def a\n  1\nend\n\ndef b\n  2\nend\n";
    for config in [
        "[cops.\"Layout\"]\nenabled = false\n".to_owned(),
        format!("[cops.\"{cop}\"]\nenabled = false\n"),
        format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
        "exclude = [\"x.rb\"]\n".to_owned(),
    ] {
        let dir = project(&[("x.rb", before), ("rustocop.toml", &config)]);
        let args = if config.contains("exclude") {
            vec!["--fix", "--only", cop, "x.rb"]
        } else {
            vec!["--fix", "x.rb"]
        };
        let fixed = rustocop(dir.path(), &args, "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            before
        );
        if !config.contains("exclude") {
            let except = rustocop(
                dir.path(),
                &["--fix", "--only", "Layout", "--except", cop, "x.rb"],
                "",
            );
            assert_eq!(except.status.code(), Some(0), "{except:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            let only = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(only.status.code(), Some(0), "{only:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                after
            );
        }
    }
    for prefix in ["rubocop", "rustocop"] {
        for target in [cop, "EmptyLineBetweenDefs", "Layout", "all"] {
            let source = format!(
                "# {prefix}:disable {target}\n{before}# {prefix}:enable {target}\np :separator\n{before}"
            );
            let expected = format!(
                "# {prefix}:disable {target}\n{before}# {prefix}:enable {target}\np :separator\n{after}"
            );
            let dir = project(&[("x.rb", &source)]);
            let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                expected
            );
        }
        let source = format!("def a\nend\n# {prefix}:disable-next {cop}\ndef b\nend\n");
        let dir = project(&[("x.rb", &source)]);
        let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            source
        );
    }
}

#[test]
fn definition_spacing_clean_and_withheld_files_are_not_written() {
    for (source, status, remaining) in [
        ("", 0, 0),
        ("__END__\ndef a\nend\ndef b\nend\n", 0, 0),
        ("def a; end\ndef b; end\n", 0, 0),
        ("def a\nend\ndef b\nend\np __LINE__\n", 1, 1),
    ] {
        let dir = project(&[("x.rb", source)]);
        let path = dir.path().join("x.rb");
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/EmptyLineBetweenDefs", "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        assert!(fixed.stderr.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(
            std::fs::metadata(path).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            &format!(
                "1 file inspected, 0 corrections applied, {remaining} {} remaining",
                if remaining == 1 {
                    "offense"
                } else {
                    "offenses"
                }
            )
        );
    }
}

#[test]
fn definition_spacing_preserves_controlled_runtime_values_and_line_numbers() {
    let cop = "Layout/EmptyLineBetweenDefs";
    let before = "def a = <<~H\n  héllo\n\n  world\nH\ndef b\n  ['a\n\nb', /a\n\nb/.source, %q(a\n\nb), %W(a#{1})]\nend\np [a, b]\n";
    let after = before.replace("H\ndef b", "H\n\ndef b");
    for (before, after, status) in [
        (before.to_owned(), after, 0),
        (
            "def a\n  __LINE__\nend\ndef b\n  __LINE__\nend\np [a, b, __LINE__]\n".to_owned(),
            "def a\n  __LINE__\nend\ndef b\n  __LINE__\nend\np [a, b, __LINE__]\n".to_owned(),
            1,
        ),
    ] {
        let run = |source: &str| {
            let output = Command::new("ruby").args(["-e", source]).output().unwrap();
            assert!(output.status.success(), "{output:?}");
            output.stdout
        };
        let expected = run(&before);
        let dir = project(&[("x.rb", &before)]);
        let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        let actual = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
        assert_eq!(actual, after);
        assert_eq!(run(&actual), expected);
    }
}

#[test]
fn definition_spacing_mixed_body_padding_fixes_converge_without_cycles() {
    let selected = "Layout/EmptyLineBetweenDefs,Layout/EmptyLines,Layout/EmptyLinesAroundClassBody,Layout/EmptyLinesAroundMethodBody";
    let before = "class C\n\n  def a\n\n    1\n\n  end\n\n\n\n  # b docs\n  def b\n\n    2\n\n  end\n\nend\n";
    let after = "class C\n  def a\n    1\n  end\n\n  # b docs\n  def b\n    2\n  end\nend\n";
    let dir = project(&[("x.rb", before)]);
    let path = dir.path().join("x.rb");
    let mut seen = std::collections::HashSet::new();
    for _ in 0..5 {
        let source = std::fs::read_to_string(&path).unwrap();
        assert!(seen.insert(source));
        let fixed = rustocop(dir.path(), &["--fix", "--only", selected, "x.rb"], "");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert!(matches!(fixed.status.code(), Some(0 | 1)), "{fixed:?}");
        if fixed.status.success() {
            break;
        }
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
    let stable = rustocop(dir.path(), &["--fix", "--only", selected, "x.rb"], "");
    assert_eq!(stable.status.code(), Some(0), "{stable:?}");
    assert_eq!(
        stdout_lines(&stable).last().unwrap(),
        "1 file inspected, 0 corrections applied, 0 offenses remaining"
    );
}

fn body_padding_cops() -> [(&'static str, &'static str); 3] {
    [
        ("Layout/EmptyLinesAroundClassBody", "class C"),
        ("Layout/EmptyLinesAroundModuleBody", "module M"),
        ("Layout/EmptyLinesAroundMethodBody", "def foo"),
    ]
}

#[test]
fn body_padding_default_lint_is_read_only_and_fixes_are_stable() {
    for (cop, header) in body_padding_cops() {
        for format in ["text", "github"] {
            let before = format!(
                "\u{feff}# 日本語\r\n{header}\r\n\r\n  puts \"é\"\r\n\r\nend\r\n__END__\r\n\r\nraw\r\n"
            );
            let after = before
                .replace("\r\n\r\n  puts", "\r\n  puts")
                .replace("\r\n\r\nend", "\r\nend");
            let dir = project(&[("x.rb", &before)]);
            let path = dir.path().join("x.rb");
            let plain = rustocop(dir.path(), &["--format", format, "x.rb"], "");
            assert_eq!(plain.status.code(), Some(1), "{plain:?}");
            assert!(plain.stderr.is_empty(), "{plain:?}");
            assert!(stdout_lines(&plain).iter().any(|line| line.contains(cop)));
            assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
            let fixed = rustocop(
                dir.path(),
                &["--fix", "--only", cop, "--format", format, "x.rb"],
                "",
            );
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert!(fixed.stderr.is_empty(), "{fixed:?}");
            assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
            assert_eq!(
                stdout_lines(&fixed).last().unwrap(),
                "1 file inspected, 2 corrections applied, 0 offenses remaining"
            );
            let again = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(again.status.code(), Some(0), "{again:?}");
            assert_eq!(
                stdout_lines(&again).last().unwrap(),
                "1 file inspected, 0 corrections applied, 0 offenses remaining"
            );
            assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
        }
    }
}

#[test]
fn body_padding_config_exclusions_selections_and_directives() {
    for (cop, header) in body_padding_cops() {
        let before = format!("# A container.\n{header}\n\n  puts 1\n\nend\n");
        let after = format!("# A container.\n{header}\n  puts 1\nend\n");
        for config in [
            "[cops.\"Layout\"]\nenabled = false\n".to_string(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
            "exclude = [\"x.rb\"]\n".to_string(),
        ] {
            let dir = project(&[("x.rb", &before), ("rustocop.toml", &config)]);
            let args = if config.contains("exclude") {
                vec!["--fix", "--only", cop, "x.rb"]
            } else {
                vec!["--fix", "x.rb"]
            };
            let fixed = rustocop(dir.path(), &args, "");
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            if !config.contains("exclude") {
                let except = rustocop(
                    dir.path(),
                    &["--fix", "--only", "Layout", "--except", cop, "x.rb"],
                    "",
                );
                assert_eq!(except.status.code(), Some(0), "{except:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    before
                );
                let only = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(only.status.code(), Some(0), "{only:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    after
                );
            }
        }
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Layout", "all"] {
                let before = format!(
                    "# {prefix}:disable {target}\n{header}\n\n  puts 1\n\nend\n# {prefix}:enable {target}\n{header}\n\n  puts 2\n\nend\n"
                );
                let after = before.replace("\n\n  puts 2\n\n", "\n  puts 2\n");
                let dir = project(&[("x.rb", &before)]);
                let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    after
                );
            }
        }
    }
}

#[test]
fn body_padding_clean_and_withheld_files_are_not_written() {
    for (cop, header) in body_padding_cops() {
        for (source, status, remaining) in [
            (String::new(), 0, 0),
            ("__END__\n\nraw\n".into(), 0, 0),
            (format!("{header}\n\n  puts 1\n\nend\np __LINE__\n"), 1, 2),
        ] {
            let dir = project(&[("x.rb", &source)]);
            let path = dir.path().join("x.rb");
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&path, permissions).unwrap();
            let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
            assert!(fixed.stderr.is_empty(), "{fixed:?}");
            assert_eq!(std::fs::read(path).unwrap(), source.as_bytes());
            assert_eq!(
                stdout_lines(&fixed).last().unwrap(),
                &format!(
                    "1 file inspected, 0 corrections applied, {remaining} {} remaining",
                    if remaining == 1 {
                        "offense"
                    } else {
                        "offenses"
                    }
                )
            );
        }
    }
}

#[test]
fn body_padding_preserves_raw_encoding_bytes() {
    for (cop, header) in body_padding_cops() {
        let before = [
            b"# coding: ascii-8bit\r\n".as_slice(),
            header.as_bytes(),
            b"\r\n\r\n  puts \"\xff\"\r\n\r\nend\r\n__END__\r\n\xff\n\n",
        ]
        .concat();
        let after = [
            b"# coding: ascii-8bit\r\n".as_slice(),
            header.as_bytes(),
            b"\r\n  puts \"\xff\"\r\nend\r\n__END__\r\n\xff\n\n",
        ]
        .concat();
        let dir = project(&[]);
        let path = dir.path().join("x.rb");
        std::fs::write(&path, &before).unwrap();
        let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(std::fs::read(path).unwrap(), after);
    }
}

#[test]
fn body_padding_preserves_controlled_runtime_literals_and_line_values() {
    let selected = "Layout/EmptyLinesAroundClassBody,Layout/EmptyLinesAroundModuleBody,Layout/EmptyLinesAroundMethodBody";
    let values = "['a\n\nb', \"a\n\nb\", /a\n\nb/.source, %q(a\n\nb), %Q(a#{1}b), %w(a\\\n\nb), %W(a#{1}), <<~H]\n  a\n\n  b\nH\n";
    let before = format!(
        "module M\n\n  class C\n\n    def self.values\n\n      {values}\n    end\n\n  end\n\nend\np M::C.values\n"
    );
    let after = format!(
        "module M\n  class C\n    def self.values\n      {values}    end\n  end\nend\np M::C.values\n"
    );
    for (before, after, status) in [
        (before, after, 0),
        ("module M\n\n  class C\n\n    def self.line\n\n      __LINE__\n\n    end\n\n  end\n\nend\np [M::C.line, __LINE__]\n".into(), "module M\n\n  class C\n\n    def self.line\n\n      __LINE__\n\n    end\n\n  end\n\nend\np [M::C.line, __LINE__]\n".into(), 1),
    ] {
        let run = |source: &str| {
            let output = Command::new("ruby").args(["-e", source]).output().unwrap();
            assert!(output.status.success(), "{output:?}");
            output.stdout
        };
        let expected = run(&before);
        let dir = project(&[("x.rb", &before)]);
        let fixed = rustocop(dir.path(), &["--fix", "--only", selected, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        let actual = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
        assert_eq!(actual, after);
        assert_eq!(run(&actual), expected);
    }
}

#[test]
fn body_padding_preserves_runtime_boundaries_after_explicit_continuations() {
    for (cop, source, expected) in [
        (
            "Layout/EmptyLinesAroundMethodBody",
            "def bar; :called; end\ndef foo \\\n\n  bar\nend\np [method(:foo).arity, foo]\n",
            "[0, :called]\n",
        ),
        (
            "Layout/EmptyLinesAroundClassBody",
            "def parent(*args); p args; Class.new; end\nclass C < parent \\\n\n  :body\nend\n",
            "[]\n",
        ),
    ] {
        for newline in ["\n", "\r\n"] {
            let before = source.replace('\n', newline);
            let run = |source: &str| {
                let output = Command::new("ruby").args(["-e", source]).output().unwrap();
                assert!(output.status.success(), "{output:?}");
                assert_eq!(output.stdout, expected.as_bytes());
            };
            run(&before);
            let dir = project(&[("x.rb", &before)]);
            for _ in 0..2 {
                let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(fixed.status.code(), Some(1), "{fixed:?}");
                assert!(fixed.stderr.is_empty(), "{fixed:?}");
                assert_eq!(
                    stdout_lines(&fixed).last().unwrap(),
                    "1 file inspected, 0 corrections applied, 1 offense remaining"
                );
                let actual = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
                assert_eq!(actual, before);
                run(&actual);
            }
        }
    }
}

#[test]
fn empty_lines_default_lint_is_read_only_and_single_pass_fix_is_stable() {
    let before =
        "\u{feff}# 日本語\r\nputs \"é\"\r\n\r\n\r\n# separator\nputs 2\n__END__\r\n\r\n\r\nraw\r\n";
    let after =
        "\u{feff}# 日本語\r\nputs \"é\"\r\n\r\n# separator\nputs 2\n__END__\r\n\r\n\r\nraw\r\n";
    for format in ["text", "github"] {
        let dir = project(&[("x.rb", before)]);
        let path = dir.path().join("x.rb");
        let plain = rustocop(dir.path(), &["--format", format, "x.rb"], "");
        assert_eq!(plain.status.code(), Some(1), "{plain:?}");
        assert!(plain.stderr.is_empty(), "{plain:?}");
        assert!(
            stdout_lines(&plain)
                .iter()
                .any(|line| line.contains("Layout/EmptyLines"))
        );
        assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
        let fixed = rustocop(
            dir.path(),
            &[
                "--fix",
                "--only",
                "Layout/EmptyLines",
                "--format",
                format,
                "x.rb",
            ],
            "",
        );
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 1 correction applied, 0 offenses remaining"
        );
        let again = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/EmptyLines", "x.rb"],
            "",
        );
        assert_eq!(again.status.code(), Some(0), "{again:?}");
        assert_eq!(
            stdout_lines(&again).last().unwrap(),
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        );
        assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
    }
}

#[test]
fn empty_lines_config_exclusions_selections_and_directives() {
    let cop = "Layout/EmptyLines";
    let before = "puts 1\n\n\nputs 2\n";
    for config in [
        "[cops.\"Layout\"]\nenabled = false\n".to_string(),
        format!("[cops.\"{cop}\"]\nenabled = false\n"),
        format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
        "exclude = [\"x.rb\"]\n".to_string(),
    ] {
        let dir = project(&[("x.rb", before), ("rustocop.toml", &config)]);
        let args = if config.contains("exclude") {
            vec!["--fix", "--only", cop, "x.rb"]
        } else {
            vec!["--fix", "x.rb"]
        };
        let fixed = rustocop(dir.path(), &args, "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            before
        );
        if !config.contains("exclude") {
            let except = rustocop(
                dir.path(),
                &["--fix", "--only", "Layout", "--except", cop, "x.rb"],
                "",
            );
            assert_eq!(except.status.code(), Some(0), "{except:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            let only = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(only.status.code(), Some(0), "{only:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                "puts 1\n\nputs 2\n"
            );
        }
    }
    for prefix in ["rustocop", "rubocop"] {
        for target in [cop, "Layout", "all"] {
            let before = format!(
                "# {prefix}:disable {target}\nputs 1\n\n\nputs 2\n# {prefix}:enable {target}\nputs 3\n\n\nputs 4\n"
            );
            let after = before.replace("puts 3\n\n\n", "puts 3\n\n");
            let dir = project(&[("x.rb", &before)]);
            let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                after
            );
        }
    }
}

#[test]
fn empty_lines_clean_data_and_withheld_only_files_are_not_written() {
    for (source, status) in [
        ("", 0),
        ("__END__\n\n\nraw\n", 0),
        ("x = 'a\n\n\nb'\n", 0),
        ("puts 1\n\n\nclass C\nLINE = __LINE__\nend\n", 1),
        ("x = %w(a\n\n\nb)\n", 1),
    ] {
        let dir = project(&[("x.rb", source)]);
        let path = dir.path().join("x.rb");
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/EmptyLines", "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert_eq!(std::fs::read(path).unwrap(), source.as_bytes());
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            &format!(
                "1 file inspected, 0 corrections applied, {status} {} remaining",
                if status == 1 { "offense" } else { "offenses" }
            )
        );
    }
}

#[test]
fn empty_lines_preserve_raw_encoding_and_exact_byte_ranges() {
    let dir = project(&[]);
    let path = dir.path().join("x.rb");
    let before = b"# coding: ascii-8bit\r\nputs \"\xff\"\r\n\r\n\r\nputs 2\n__END__\n\xff\n\n\n";
    let after = b"# coding: ascii-8bit\r\nputs \"\xff\"\r\n\r\nputs 2\n__END__\n\xff\n\n\n";
    std::fs::write(&path, before).unwrap();
    let plain = rustocop(dir.path(), &["--only", "Layout/EmptyLines", "x.rb"], "");
    assert_eq!(plain.status.code(), Some(1), "{plain:?}");
    assert!(
        stdout_lines(&plain)
            .iter()
            .any(|line| line.contains("x.rb:4:1: C: Extra blank line detected."))
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let fixed = rustocop(
        dir.path(),
        &["--fix", "--only", "Layout/EmptyLines", "x.rb"],
        "",
    );
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    assert_eq!(std::fs::read(path).unwrap(), after);
}

#[test]
fn empty_lines_preserve_controlled_runtime_values_and_later_line_numbers() {
    assert!(
        Command::new("ruby")
            .arg("--version")
            .output()
            .unwrap()
            .status
            .success()
    );
    let literals = "values = ['a\n\n\nb', \"a\n\n\nb\", /a\n\n\nb/.source, %q(a\n\n\nb), %Q(a#{1\n\n\n2}b), %w(a\\\n\n\nb), %W(a#{1\n\n\n2})]\nvalues << <<~H\n  a\n\n\n  b\nH\n";
    for (before, after, status) in [
        (
            format!("{literals}puts 1\n\n\np values\n"),
            format!(
                "{}puts 1\n\np values\n",
                literals.replacen("1\n\n\n2", "1\n\n2", 1)
            ),
            1,
        ),
        (
            "puts 1\n\n\nclass Lines\n  def self.line; __LINE__; end\nend\np Lines.line\n"
                .to_string(),
            "puts 1\n\n\nclass Lines\n  def self.line; __LINE__; end\nend\np Lines.line\n"
                .to_string(),
            1,
        ),
        (
            "p __LINE__\nputs 1\n\n\np 2\n".to_string(),
            "p __LINE__\nputs 1\n\np 2\n".to_string(),
            0,
        ),
    ] {
        let run = |source: &str| {
            let output = Command::new("ruby").args(["-e", source]).output().unwrap();
            assert!(output.status.success(), "{output:?}");
            output.stdout
        };
        let expected = run(&before);
        let dir = project(&[("x.rb", &before)]);
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/EmptyLines", "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        let actual = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
        assert_eq!(actual, after);
        assert_eq!(run(&actual), expected);
    }
}

#[test]
fn end_alignment_default_plain_lint_is_read_only_and_fix_is_stable() {
    let before = "\u{feff}# 日本語\r\nclass A\r\n  end # keep\r\n__END__\r\n  end\r\n\traw\r\n";
    let after = before.replacen("  end # keep", "end # keep", 1);
    for format in ["text", "github"] {
        let dir = project(&[("x.rb", before)]);
        let path = dir.path().join("x.rb");
        let plain = rustocop(dir.path(), &["--format", format, "x.rb"], "");
        assert_eq!(plain.status.code(), Some(1), "{plain:?}");
        assert!(plain.stderr.is_empty(), "{plain:?}");
        assert!(
            stdout_lines(&plain)
                .iter()
                .any(|l| l.contains("Layout/EndAlignment"))
        );
        assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
        let fixed = rustocop(
            dir.path(),
            &[
                "--fix",
                "--only",
                "Layout/EndAlignment",
                "--format",
                format,
                "x.rb",
            ],
            "",
        );
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 1 correction applied, 0 offenses remaining"
        );
        let again = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/EndAlignment", "x.rb"],
            "",
        );
        assert_eq!(again.status.code(), Some(0), "{again:?}");
        assert_eq!(
            stdout_lines(&again).last().unwrap(),
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        );
    }
}

#[test]
fn end_alignment_config_exclusions_and_cli_selections() {
    let cop = "Layout/EndAlignment";
    for config in [
        "[cops.\"Layout\"]\nenabled = false\n".to_string(),
        format!("[cops.\"{cop}\"]\nenabled = false\n"),
        format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
        "exclude = [\"x.rb\"]\n".to_string(),
    ] {
        let before = "if a\n  x\n  end\n";
        // Keep this Layout selection test independent of modifier-style detection.
        let config = format!("{config}[cops.\"Style/IfUnlessModifier\"]\nenabled = false\n");
        let dir = project(&[("x.rb", before), ("rustocop.toml", &config)]);
        let args = if config.contains("exclude") {
            vec!["--fix", "--only", cop, "x.rb"]
        } else {
            vec!["--fix", "x.rb"]
        };
        let output = rustocop(dir.path(), &args, "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            before
        );
        if !config.contains("exclude") {
            let except = rustocop(
                dir.path(),
                &["--fix", "--only", cop, "--except", cop, "x.rb"],
                "",
            );
            assert_eq!(except.status.code(), Some(0), "{except:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            let only = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(only.status.code(), Some(0), "{only:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                "if a\n  x\nend\n"
            );
        }
    }
}

#[test]
fn end_alignment_with_indentation_cops_converges_without_changing_tokens_or_directives() {
    for prefix in ["rustocop", "rubocop"] {
        let disabled =
            format!("# {prefix}:disable Layout\nclass Hidden\n  end\n# {prefix}:enable Layout\n");
        let before = disabled.clone()
            + "module M\nclass C\n\tdef self.value\n\t\tif true\n\t\t\t[\"end\", __LINE__]\n\t\t\tend # keep\n\tend\nend\n  end\np M::C.value\n";
        let after = disabled
            + "module M\n  class C\n    def self.value\n      if true\n        [\"end\", __LINE__]\n      end # keep\n    end\n  end\nend\np M::C.value\n";
        let dir = project(&[("x.rb", &before)]);
        let mut snapshots = vec![before.as_bytes().to_vec()];
        let mut clean = false;
        for _ in 0..5 {
            let fixed = rustocop(
                dir.path(),
                &[
                    "--fix",
                    "--only",
                    "Layout/EndAlignment,Layout/IndentationStyle,Layout/IndentationWidth",
                    "x.rb",
                ],
                "",
            );
            assert!(fixed.stderr.is_empty(), "{fixed:?}");
            assert_ne!(fixed.status.code(), Some(2), "{fixed:?}");
            let actual = std::fs::read(dir.path().join("x.rb")).unwrap();
            if fixed.status.code() == Some(0) {
                assert_eq!(actual, after.as_bytes());
                clean = true;
                break;
            }
            assert!(!snapshots.contains(&actual), "correction cycle: {actual:?}");
            snapshots.push(actual);
        }
        assert!(clean);
    }
}

#[test]
fn end_alignment_preserves_controlled_runtime_literal_values_and_line_numbers() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("skipping controlled runtime check: ruby unavailable");
        return;
    }
    // Controlled synthetic source only. No corpus/application code executes.
    let before = "x = if true\n  [\"é\\n end\", /end/.source, %w(a end), __LINE__]\nend\ny = <<~H\n  text\n#{if true\n    \"value\"\n    end}\n  tail\nH\np [x, y, __LINE__]\n";
    let after = "x = if true\n  [\"é\\n end\", /end/.source, %w(a end), __LINE__]\n    end\ny = <<~H\n  text\n#{if true\n    \"value\"\n  end}\n  tail\nH\np [x, y, __LINE__]\n";
    let run = |source: &str| {
        let output = Command::new("ruby").args(["-e", source]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        output.stdout
    };
    let expected = run(before);
    let dir = project(&[("x.rb", before)]);
    let fixed = rustocop(
        dir.path(),
        &["--fix", "--only", "Layout/EndAlignment", "x.rb"],
        "",
    );
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    assert!(fixed.stderr.is_empty(), "{fixed:?}");
    let actual = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
    assert_eq!(actual, after);
    assert_eq!(run(&actual), expected);
}

#[test]
fn indentation_width_default_plain_lint_is_read_only_and_fix_is_stable() {
    let before = "\u{feff}# 日本語\r\nclass A\r\n    def élève\r\n      \"é\"\r\n    end\r\nend\r\n__END__\r\n\traw\r\n";
    let after = "\u{feff}# 日本語\r\nclass A\r\n  def élève\r\n    \"é\"\r\n  end\r\nend\r\n__END__\r\n\traw\r\n";
    for format in ["text", "github"] {
        let dir = project(&[("x.rb", before)]);
        let path = dir.path().join("x.rb");
        let plain = rustocop(dir.path(), &["--format", format, "x.rb"], "");
        assert_eq!(plain.status.code(), Some(1), "{plain:?}");
        assert!(plain.stderr.is_empty(), "{plain:?}");
        assert!(
            stdout_lines(&plain)
                .iter()
                .any(|l| l.contains("Layout/IndentationWidth"))
        );
        assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
        let fixed = rustocop(
            dir.path(),
            &[
                "--fix",
                "--only",
                "Layout/IndentationWidth",
                "--format",
                format,
                "x.rb",
            ],
            "",
        );
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 1 correction applied, 0 offenses remaining"
        );
        let again = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/IndentationWidth", "x.rb"],
            "",
        );
        assert_eq!(again.status.code(), Some(0), "{again:?}");
        assert_eq!(
            stdout_lines(&again).last().unwrap(),
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        );
    }
}

#[test]
fn indentation_width_config_exclusions_and_cli_selection() {
    let cop = "Layout/IndentationWidth";
    for config in [
        "[cops.\"Layout\"]\nenabled = false\n".to_string(),
        format!("[cops.\"{cop}\"]\nenabled = false\n"),
        format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
        "exclude = [\"x.rb\"]\n".to_string(),
    ] {
        let before = "if a\n x\nend\n";
        // Keep this Layout selection test independent of modifier-style detection.
        let config = format!("{config}[cops.\"Style/IfUnlessModifier\"]\nenabled = false\n");
        let dir = project(&[("x.rb", before), ("rustocop.toml", &config)]);
        let args = if config.contains("exclude") {
            vec!["--fix", "--only", cop, "x.rb"]
        } else {
            vec!["--fix", "x.rb"]
        };
        let output = rustocop(dir.path(), &args, "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            before
        );
        if !config.contains("exclude") {
            let except = rustocop(
                dir.path(),
                &["--fix", "--only", cop, "--except", cop, "x.rb"],
                "",
            );
            assert_eq!(except.status.code(), Some(0), "{except:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            let only = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(only.status.code(), Some(0), "{only:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                "if a\n  x\nend\n"
            );
        }
    }
}

#[test]
fn indentation_width_and_style_converge_with_directives_in_five_passes() {
    for prefix in ["rustocop", "rubocop"] {
        let before = format!(
            "# {prefix}:disable Layout\nif a\n\tx\nend\n# {prefix}:enable Layout\nclass A\n  def x\n \t\t1\n  end\nend\n"
        );
        let after = before.replace(" \t\t1", "    1");
        let dir = project(&[("x.rb", &before)]);
        let mut snapshots = vec![before.as_bytes().to_vec()];
        let mut clean = false;
        for _ in 0..5 {
            let fixed = rustocop(
                dir.path(),
                &[
                    "--fix",
                    "--only",
                    "Layout/IndentationWidth,Layout/IndentationStyle",
                    "x.rb",
                ],
                "",
            );
            assert!(fixed.stderr.is_empty(), "{fixed:?}");
            assert_ne!(fixed.status.code(), Some(2), "{fixed:?}");
            let bytes = std::fs::read(dir.path().join("x.rb")).unwrap();
            if fixed.status.code() == Some(0) {
                assert_eq!(bytes, after.as_bytes());
                clean = true;
                break;
            }
            assert!(!snapshots.contains(&bytes), "correction cycle: {bytes:?}");
            snapshots.push(bytes);
        }
        assert!(clean);
    }
}

#[test]
fn indentation_width_safe_withholding_and_raw_encoding_preservation() {
    let cop = "Layout/IndentationWidth";
    for (before, after, status) in [
        (b"if a\r\n    puts 1\r\n  puts \"\xff\"\r\nend\r\n".as_slice(), b"if a\r\n  puts 1\r\n  puts \"\xff\"\r\nend\r\n".as_slice(), 0),
        (b"class A\nclass B\n  X = \"\xff\"\nend\nend\n", b"class A\nclass B\n  X = \"\xff\"\nend\nend\n", 1),
        // As in RuboCop, the enabled offense shifts the whole subtree,
        // including lines on which the cop is disabled.
        (b"module A\nclass B\n  # rubocop:disable Layout/IndentationWidth\n  def x\n  end\nend\nend\n", b"module A\n  class B\n    # rubocop:disable Layout/IndentationWidth\n    def x\n    end\n  end\nend\n", 0),
        (b"", b"", 0),
        (b"__END__\n if x\ny\nend\n", b"__END__\n if x\ny\nend\n", 0),
    ] {
        let dir = project(&[]);
        let path = dir.path().join("x.rb");
        let before = [b"# coding: ascii-8bit\r\n".as_slice(), before].concat();
        let after = [b"# coding: ascii-8bit\r\n".as_slice(), after].concat();
        std::fs::write(&path, &before).unwrap();
        let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert_eq!(std::fs::read(path).unwrap(), after);
    }
}

#[test]
fn indentation_width_preserves_controlled_runtime_literals_and_line_numbers() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("skipping controlled runtime check: ruby unavailable");
        return;
    }
    // Controlled synthetic source only, never application/corpus code.
    let before = "module M\nclass C\n  def self.values\n    [\"é\n raw\", %w(a\\\n escaped), /a\n raw/.source, <<~H, __LINE__]\n  héllo #{1}\n  H\n  end\nend\nend\np [M::C.values, __LINE__]\n";
    let after = "module M\n  class C\n    def self.values\n      [\"é\n raw\", %w(a\\\n escaped), /a\n raw/.source, <<~H, __LINE__]\n  héllo #{1}\n  H\n    end\n  end\nend\np [M::C.values, __LINE__]\n";
    let run = |text: &str| {
        let output = Command::new("ruby")
            .args(["-e", text])
            .output()
            .expect("Ruby required for controlled runtime test");
        assert!(output.status.success(), "{output:?}");
        output.stdout
    };
    let expected = run(before);
    let dir = project(&[("x.rb", before)]);
    let fixed = rustocop(
        dir.path(),
        &["--fix", "--only", "Layout/IndentationWidth", "x.rb"],
        "",
    );
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    assert!(fixed.stderr.is_empty(), "{fixed:?}");
    let actual = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
    assert_eq!(actual, after);
    assert_eq!(run(&actual), expected);
}

#[test]
fn indentation_style_plain_lint_is_read_only_and_fix_reports_withheld_tabs() {
    let before = "\u{feff}# 日本語\r\n\tputs \"é\"\r\n \t# comment\n\t  \r\nx = /a\n\tb/\n__END__\r\n\traw\r\n";
    let after = "\u{feff}# 日本語\r\n  puts \"é\"\r\n   # comment\n    \r\nx = /a\n\tb/\n__END__\r\n\traw\r\n";
    for format in ["text", "github"] {
        let dir = project(&[("x.rb", before)]);
        let plain = rustocop(dir.path(), &["--format", format, "x.rb"], "");
        assert_eq!(plain.status.code(), Some(1), "{plain:?}");
        assert!(plain.stderr.is_empty(), "{plain:?}");
        assert!(
            stdout_lines(&plain)
                .iter()
                .any(|line| line.contains("Layout/IndentationStyle"))
        );
        assert_eq!(
            std::fs::read(dir.path().join("x.rb")).unwrap(),
            before.as_bytes()
        );
        let fixed = rustocop(
            dir.path(),
            &[
                "--fix",
                "--only",
                "Layout/IndentationStyle",
                "--format",
                format,
                "x.rb",
            ],
            "",
        );
        assert_eq!(fixed.status.code(), Some(1), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert_eq!(
            std::fs::read(dir.path().join("x.rb")).unwrap(),
            after.as_bytes()
        );
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 3 corrections applied, 1 offense remaining"
        );
        let again = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/IndentationStyle", "x.rb"],
            "",
        );
        assert_eq!(again.status.code(), Some(1), "{again:?}");
        assert_eq!(
            stdout_lines(&again).last().unwrap(),
            "1 file inspected, 0 corrections applied, 1 offense remaining"
        );
        assert_eq!(
            std::fs::read(dir.path().join("x.rb")).unwrap(),
            after.as_bytes()
        );
    }
}

#[test]
fn indentation_style_honors_config_selections_exclusions_and_directives() {
    let cop = "Layout/IndentationStyle";
    for config in [
        "[cops.\"Layout\"]\nenabled = false\n".to_string(),
        format!("[cops.\"{cop}\"]\nenabled = false\n"),
        format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
        "exclude = [\"x.rb\"]\n".to_string(),
    ] {
        let before = "\tputs 1\n";
        let dir = project(&[("x.rb", before), ("rustocop.toml", &config)]);
        let args = if config.contains("exclude") {
            vec!["--fix", "--only", cop, "x.rb"]
        } else {
            // Default-on InitialIndentation would remove the
            // leading tab of this one-line file; it is not the subject here.
            vec!["--fix", "--except", "Layout/InitialIndentation", "x.rb"]
        };
        let output = rustocop(dir.path(), &args, "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            before
        );
        if !config.contains("exclude") {
            let except = rustocop(
                dir.path(),
                &["--fix", "--only", cop, "--except", cop, "x.rb"],
                "",
            );
            assert_eq!(except.status.code(), Some(0), "{except:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            let only = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(only.status.code(), Some(0), "{only:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                "  puts 1\n"
            );
            let again = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(
                stdout_lines(&again).last().unwrap(),
                "1 file inspected, 0 corrections applied, 0 offenses remaining"
            );
        }
    }
    for prefix in ["rustocop", "rubocop"] {
        for target in [cop, "Layout", "all"] {
            let before = format!(
                "# {prefix}:disable {target}\n\tputs 1\n# {prefix}:enable {target}\n\tputs 2 # {prefix}:disable {target}\n\tputs 3\n"
            );
            let after = before.strip_suffix("\tputs 3\n").unwrap().to_owned() + "  puts 3\n";
            let dir = project(&[("x.rb", &before)]);
            let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                after
            );
        }
    }
}

#[test]
fn indentation_style_does_not_write_clean_data_or_withheld_only_files() {
    for (source, status) in [
        ("", 0),
        ("__END__\n\tdata\n", 0),
        ("x = 'a\n\tb'\n", 0),
        ("x = /a\n\tb/\n", 1),
        ("\u{feff}\tputs 1\r\n", 0),
    ] {
        let dir = project(&[("x.rb", source)]);
        let path = dir.path().join("x.rb");
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/IndentationStyle", "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert_eq!(std::fs::read(path).unwrap(), source.as_bytes());
    }
}

#[test]
fn indentation_style_preserves_raw_encoding_bytes() {
    let dir = project(&[]);
    let path = dir.path().join("x.rb");
    let before = b"# coding: ascii-8bit\r\n\tputs \"\xff\"\r\n__END__\n\t\xff\n";
    let after = b"# coding: ascii-8bit\r\n  puts \"\xff\"\r\n__END__\n\t\xff\n";
    std::fs::write(&path, before).unwrap();
    let plain = rustocop(
        dir.path(),
        &["--only", "Layout/IndentationStyle", "x.rb"],
        "",
    );
    assert_eq!(plain.status.code(), Some(1), "{plain:?}");
    assert!(
        stdout_lines(&plain)
            .iter()
            .any(|line| line.contains("x.rb:2:1: C: Tab detected in indentation."))
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let fixed = rustocop(
        dir.path(),
        &["--fix", "--only", "Layout/IndentationStyle", "x.rb"],
        "",
    );
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    assert_eq!(std::fs::read(path).unwrap(), after);
}

#[test]
fn indentation_style_preserves_controlled_runtime_literals_and_line_numbers() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("skipping controlled runtime check: ruby unavailable");
        return;
    }
    // Synthetic code only. Include significant regexp/escaped-word tabs,
    // nested interpolation, all heredoc modes and __LINE__ in another scope.
    let before = "values = [\n\t'single\n\tline', \"double\n#{\n\t1\n}tail\", /a\n\tb/.source, %r(a\n\tb).source, %q(a\n\tb), %Q(a\n\tb), %w(a\\\n\tb), %W(a#{\n\t1\n}), %i(a\\\n\tb), %I(a#{\n\t1\n}), :\"a\n\tb\"\n]\nvalues << <<H\n\tliteral\nH\nvalues << <<-'H'\n\tliteral\n\tH\nvalues << <<~H\n\t#{\n\t2\n\t}tail\n\tH\n\tvalues << __LINE__\nclass Lines\n\tdef self.line\n\t\t__LINE__\n\tend\nend\n\tvalues << Lines.line\n\tp values\n";
    let after = before
        .replace("\t'single", "  'single")
        .replace("\tvalues <<", "  values <<")
        .replace("\tdef self.line", "  def self.line")
        .replace("\t\t__LINE__", "    __LINE__")
        .replace("\tend\n", "  end\n")
        .replace("\tp values", "  p values");
    let run = |text: &str| {
        let output = Command::new("ruby").args(["-e", text]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        output.stdout
    };
    let expected = run(before);
    let dir = project(&[("x.rb", before)]);
    let fixed = rustocop(
        dir.path(),
        &["--fix", "--only", "Layout/IndentationStyle", "x.rb"],
        "",
    );
    assert_eq!(fixed.status.code(), Some(1), "{fixed:?}");
    assert!(fixed.stderr.is_empty(), "{fixed:?}");
    let corrected = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
    assert_eq!(corrected, after);
    assert_ne!(corrected, before);
    assert_eq!(run(&corrected), expected);
    assert_eq!(corrected.lines().count(), before.lines().count());
}

#[test]
fn operator_spacing_plain_lint_default_enabled_and_fixes_are_stable() {
    let only = "Layout/SpaceAroundOperators,Layout/SpaceAroundEqualsInParameterDefault";
    for format in ["text", "github"] {
        let dir = project(&[]);
        let path = dir.path().join("x.rb");
        let before =
            b"# coding: ascii-8bit\r\nx=1+2\r\ndef f(x=\"\xff\"); x; end\r\n__END__\r\na+b\xff\r\n";
        let after = b"# coding: ascii-8bit\r\nx = 1 + 2\r\ndef f(x = \"\xff\"); x; end\r\n__END__\r\na+b\xff\r\n";
        std::fs::write(&path, before).unwrap();
        let plain = rustocop(dir.path(), &["--format", format, "x.rb"], "");
        assert_eq!(plain.status.code(), Some(1), "{plain:?}");
        for cop in only.split(',') {
            assert!(
                stdout_lines(&plain).iter().any(|s| s.contains(cop)),
                "{plain:?}"
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", only, "--format", format, "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after);
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 3 corrections applied, 0 offenses remaining"
        );
        let again = rustocop(dir.path(), &["--fix", "--only", only, "x.rb"], "");
        assert_eq!(again.status.code(), Some(0), "{again:?}");
        assert_eq!(
            stdout_lines(&again).last().unwrap(),
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        );
        assert_eq!(std::fs::read(&path).unwrap(), after);
    }
}

#[test]
fn operator_and_parameter_spacing_honor_directives_exclusions_and_selection() {
    for (cop, before, after) in [
        ("Layout/SpaceAroundOperators", "$x=1\n", "$x = 1\n"),
        (
            "Layout/SpaceAroundEqualsInParameterDefault",
            "def f(arg=1); arg; end\n",
            "def f(arg = 1); arg; end\n",
        ),
    ] {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Layout", "all"] {
                let source = format!(
                    "# {prefix}:disable {target}\n{before}# {prefix}:enable {target}\n{} # {prefix}:disable {target}\n{before}",
                    before.trim_end()
                );
                let expected = format!(
                    "# {prefix}:disable {target}\n{before}# {prefix}:enable {target}\n{} # {prefix}:disable {target}\n{after}",
                    before.trim_end()
                );
                let dir = project(&[("x.rb", &source)]);
                let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    expected
                );
            }
        }
        for config in [
            "[cops.\"Layout\"]\nenabled = false\n".to_string(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
            "exclude = [\"x.rb\"]\n".to_string(),
        ] {
            let dir = project(&[("x.rb", before), ("rustocop.toml", &config)]);
            let args = if config.contains("exclude") {
                vec!["--fix", "--only", cop, "x.rb"]
            } else {
                vec!["--fix", "--except", "Style", "x.rb"]
            };
            let output = rustocop(dir.path(), &args, "");
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            if !config.contains("exclude") {
                let except = rustocop(
                    dir.path(),
                    &["--fix", "--only", cop, "--except", cop, "x.rb"],
                    "",
                );
                assert_eq!(except.status.code(), Some(0), "{except:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    before
                );
                let only = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(only.status.code(), Some(0), "{only:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    after
                );
            }
        }
    }
}

#[test]
fn operator_spacing_withholds_line_joins_and_ambiguous_literal_delimiters() {
    for (cop, source) in [
        ("Layout/SpaceAroundOperators", "x = 2 **\n  3\np __LINE__\n"),
        (
            "Layout/SpaceAroundOperators",
            "x = 2.0 /\n  3r\np __LINE__\n",
        ),
        ("Layout/SpaceAroundOperators", "%q a +1\n"),
        (
            "Layout/SpaceAroundOperators",
            "x = 2 **\n# rubocop:disable Layout/SpaceAroundOperators\n  3\n",
        ),
        (
            "Layout/SpaceAroundEqualsInParameterDefault",
            "def f(x\\\n=1); end\n",
        ),
    ] {
        let dir = project(&[("x.rb", source)]);
        let result = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        assert_eq!(
            stdout_lines(&result).last().unwrap(),
            "1 file inspected, 0 corrections applied, 1 offense remaining"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            source
        );
    }
}

#[test]
fn operator_and_default_spacing_preserve_controlled_runtime_values_and_line_numbers() {
    let only = "Layout/SpaceAroundOperators,Layout/SpaceAroundEqualsInParameterDefault";
    let before = concat!(
        "def a; 12; end\n",
        "def b; 2; end\n",
        "def f(x=-1, y=+2, z=__LINE__); [x, y, z]; end\n",
        "x=2\nx**=3\nx/=2r\n",
        "p [a/b/2, 2 ** -2, 2.0 / 3r, x, f, ->(x=1) { x }.call]\n",
        "p [1, 2].map { |v=1| v+1 }\n",
        "p ['a'+\n'b', __LINE__, /a+b/.source, %q(a+b), \"a+b#{1+2}\"]\n",
        "p <<~H\n  a+b#{1+2}\nH\n",
        "p __LINE__\n",
    );
    let expected = concat!(
        "def a; 12; end\n",
        "def b; 2; end\n",
        "def f(x = -1, y = +2, z = __LINE__); [x, y, z]; end\n",
        "x = 2\nx **= 3\nx /= 2r\n",
        "p [a / b / 2, 2**-2, 2.0/3r, x, f, ->(x = 1) { x }.call]\n",
        "p [1, 2].map { |v = 1| v + 1 }\n",
        "p ['a' +\n'b', __LINE__, /a+b/.source, %q(a+b), \"a+b#{1 + 2}\"]\n",
        "p <<~H\n  a+b#{1 + 2}\nH\n",
        "p __LINE__\n",
    );
    for (before, expected) in [
        (before.to_string(), expected.to_string()),
        (before.replace('\n', "\r\n"), expected.replace('\n', "\r\n")),
    ] {
        let dir = project(&[("x.rb", &before)]);
        let result = rustocop(dir.path(), &["--fix", "--only", only, "x.rb"], "");
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        let after = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
        assert_eq!(after, expected);
        let execute = |source: &str| {
            let output = Command::new("ruby")
                .args(["-e", source])
                .output()
                .expect("Ruby required for controlled runtime fixtures");
            assert!(output.status.success(), "{output:?}");
            output.stdout
        };
        assert_eq!(execute(&before), execute(&after));
    }
}

fn brace_cops() -> [&'static str; 2] {
    [
        "Layout/SpaceInsideHashLiteralBraces",
        "Layout/SpaceInsideBlockBraces",
    ]
}

#[test]
fn brace_spacing_selective_attributed_fixtures_are_read_only_then_stable() {
    let before = include_str!("fixtures/fix/braces.before.rb");
    let expected = [
        include_str!("fixtures/fix/braces.hash.after.rb"),
        include_str!("fixtures/fix/braces.block.after.rb"),
    ];
    for (cop, after) in brace_cops().into_iter().zip(expected) {
        for format in ["text", "github"] {
            let dir = project(&[("x.rb", before)]);
            let path = dir.path().join("x.rb");
            let metadata = std::fs::metadata(&path).unwrap().modified().unwrap();
            let plain = rustocop(dir.path(), &["--only", cop, "--format", format, "x.rb"], "");
            assert_eq!(plain.status.code(), Some(1), "{plain:?}");
            assert!(stdout_lines(&plain).iter().any(|line| line.contains(cop)));
            let other = brace_cops().into_iter().find(|name| *name != cop).unwrap();
            assert!(
                stdout_lines(&plain)
                    .iter()
                    .all(|line| !line.contains(other))
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
            assert_eq!(
                std::fs::metadata(&path).unwrap().modified().unwrap(),
                metadata
            );
            let fixed = rustocop(
                dir.path(),
                &["--fix", "--only", cop, "--format", format, "x.rb"],
                "",
            );
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
            let metadata = std::fs::metadata(&path).unwrap().modified().unwrap();
            let again = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(again.status.code(), Some(0), "{again:?}");
            assert_eq!(
                stdout_lines(&again).last().unwrap(),
                "1 file inspected, 0 corrections applied, 0 offenses remaining"
            );
            assert_eq!(
                std::fs::metadata(&path).unwrap().modified().unwrap(),
                metadata
            );
        }
    }
    let dir = project(&[("x.rb", before)]);
    let plain = rustocop(dir.path(), &["x.rb"], "");
    for cop in brace_cops() {
        assert!(
            stdout_lines(&plain).iter().any(|line| line.contains(cop)),
            "{plain:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        before
    );
}

#[test]
fn brace_spacing_honors_directives_config_exclusions_and_selection() {
    for (cop, before, after) in [
        (brace_cops()[0], "{key: 1}\n", "{ key: 1 }\n"),
        (brace_cops()[1], "f {x}\n", "f { x }\n"),
    ] {
        for prefix in ["rubocop", "rustocop"] {
            let before = before.trim_end();
            let after = after.trim_end();
            for target in [cop, "Layout", "all"] {
                let source = format!(
                    "# {prefix}:disable {target}\n{before}\n# {prefix}:enable {target}\n{before} # {prefix}:disable {target}\n# {prefix}:disable-next {target}\n{before}\n{before}\n"
                );
                let expected = format!(
                    "# {prefix}:disable {target}\n{before}\n# {prefix}:enable {target}\n{before} # {prefix}:disable {target}\n# {prefix}:disable-next {target}\n{before}\n{after}\n"
                );
                let dir = project(&[("x.rb", &source)]);
                let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    expected
                );
            }
        }
        for config in [
            "[cops.\"Layout\"]\nenabled = false\n".to_string(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
            "exclude = [\"x.rb\"]\n".to_string(),
        ] {
            let dir = project(&[("x.rb", before), ("rustocop.toml", &config)]);
            let args = if config.contains("exclude") {
                vec!["--fix", "--only", cop, "x.rb"]
            } else {
                vec!["--fix", "--except", "Style", "x.rb"]
            };
            let fixed = rustocop(dir.path(), &args, "");
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            if !config.contains("exclude") {
                let except = rustocop(
                    dir.path(),
                    &["--fix", "--only", cop, "--except", cop, "x.rb"],
                    "",
                );
                assert_eq!(except.status.code(), Some(0), "{except:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    before
                );
                let forced = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(forced.status.code(), Some(0), "{forced:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    after
                );
            }
        }
    }
}

#[test]
fn brace_spacing_preserves_raw_bytes_crlf_bom_comments_and_data() {
    let selected = brace_cops().join(",");
    for (before, after) in [
        (b"# coding: ascii-8bit\r\nh = {key: \"\xff\"}\r\nf {h}\r\n__END__\r\n{key: 1} f {x}\xff\r\n".as_slice(), b"# coding: ascii-8bit\r\nh = { key: \"\xff\" }\r\nf { h }\r\n__END__\r\n{key: 1} f {x}\xff\r\n".as_slice()),
        ("\u{feff}h = {é: 1}\r\nf {h}\r\n# {key: 1} f {x}\r\n".as_bytes(), "\u{feff}h = { é: 1 }\r\nf { h }\r\n# {key: 1} f {x}\r\n".as_bytes()),
    ] {
        let dir = project(&[]);
        let path = dir.path().join("x.rb");
        std::fs::write(&path, before).unwrap();
        let plain = rustocop(dir.path(), &["--only", &selected, "x.rb"], "");
        assert_eq!(plain.status.code(), Some(1), "{plain:?}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let fixed = rustocop(dir.path(), &["--fix", "--only", &selected, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after);
    }
}

#[test]
fn brace_spacing_clean_and_safely_withheld_files_are_not_written() {
    let selected = brace_cops().join(",");
    for (source, status) in [
        ("", 0),
        ("__END__\n{a: 1} f {x}\n", 0),
        ("f {\n}\n-> {\n}\n", 0),
        ("x = {\n}\np __LINE__\n", 1),
        (
            "x = {\n  # rustocop:disable Layout/SpaceInsideHashLiteralBraces\n}\n",
            0,
        ),
        ("x = <<~H\n  #{{\n}}\nH\n", 1),
    ] {
        let dir = project(&[("x.rb", source)]);
        let path = dir.path().join("x.rb");
        let metadata = std::fs::metadata(&path).unwrap().modified().unwrap();
        let fixed = rustocop(dir.path(), &["--fix", "--only", &selected, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            metadata
        );
    }
}

#[test]
fn brace_spacing_and_style_edits_resolve_whole_conflicts_deterministically() {
    let before = "h = {:x => 1}\nf {h;}\nx = <<~H\n  {literal} #{[1].map {_1}}\nH\n";
    let after = "h = { x: 1 }\nf { h }\nx = <<~H\n  {literal} #{[1].map { _1 }}\nH\n";
    let selections = [
        "Layout/SpaceInsideHashLiteralBraces,Layout/SpaceInsideBlockBraces,Style/HashSyntax,Style/Semicolon",
        "Style/Semicolon,Style/HashSyntax,Layout/SpaceInsideBlockBraces,Layout/SpaceInsideHashLiteralBraces",
    ];
    let mut histories = Vec::new();
    for selected in selections {
        let dir = project(&[("x.rb", before)]);
        let path = dir.path().join("x.rb");
        let mut snapshots = vec![before.to_owned()];
        for _ in 0..5 {
            let fixed = rustocop(
                dir.path(),
                &["--fix", "--fix-passes", "1", "--only", selected, "x.rb"],
                "",
            );
            assert!(matches!(fixed.status.code(), Some(0 | 1)), "{fixed:?}");
            let current = std::fs::read_to_string(&path).unwrap();
            if fixed.status.code() == Some(0) {
                assert_eq!(current, after);
                snapshots.push(current);
                break;
            }
            assert!(!snapshots.contains(&current), "correction cycle: {current}");
            snapshots.push(current);
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
        // The insertion immediately before :x conflicts with HashSyntax's
        // contiguous symbol/rocket edit; defer the whole Style correction.
        assert!(snapshots[1].contains("{ :x => 1 }"), "{snapshots:?}");
        let stable = rustocop(
            dir.path(),
            &["--fix", "--fix-passes", "1", "--only", selected, "x.rb"],
            "",
        );
        assert_eq!(stable.status.code(), Some(0), "{stable:?}");
        assert_eq!(
            stdout_lines(&stable).last().unwrap(),
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        );
        histories.push(snapshots);
    }
    assert_eq!(histories[0], histories[1]);
}

#[test]
fn brace_spacing_preserves_controlled_runtime_values_association_and_line_numbers() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("skipping controlled runtime check: ruby unavailable");
        return;
    }
    let selected = brace_cops().join(",");
    for source in [
        "h = {a: 1, b: {c: 2}}\np h, [1,2].map {|n|n * 2}, ->(x) {x}.call(3), __LINE__\n",
        "def outer(x); [x, block_given?]; end\ndef inner; yield; end\np outer inner {[1,2].map {_1 * 2}}\np outer({a: 1}), outer(a: 1), __LINE__\n",
        "h = {é: %q x }\np h, \"{x} #{{a: 1}} #{[1].map {_1}}\", __LINE__\n",
        "h = {a: <<~H}\n  {literal} #{{a: 1}} #{[1].map {_1}}\nH\np h, __LINE__\n",
        "h = {\n}\np h, __LINE__\ndef line; __LINE__; end\np line\n",
        "p({\n})\n",
        "x = <<~H\n  #{{\n}.inspect}\n  tail\nH\np x\n",
        "{a: 1} => {a:}\np a, __LINE__\n",
    ] {
        let dir = project(&[("x.rb", source)]);
        let run = || {
            let output = Command::new("ruby")
                .arg("x.rb")
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(output.status.success(), "{source:?}: {output:?}");
            output.stdout
        };
        let before = run();
        let fixed = rustocop(dir.path(), &["--fix", "--only", &selected, "x.rb"], "");
        assert!(matches!(fixed.status.code(), Some(0 | 1)), "{fixed:?}");
        assert_eq!(run(), before, "{source}");
    }
}

fn bracket_cops() -> [&'static str; 2] {
    [
        "Layout/SpaceInsideArrayLiteralBrackets",
        "Layout/SpaceInsideReferenceBrackets",
    ]
}

#[test]
fn bracket_spacing_selective_attributed_fixtures_are_read_only_then_stable() {
    let before = include_str!("fixtures/fix/brackets.before.rb");
    let expected = [
        include_str!("fixtures/fix/brackets.array.after.rb"),
        include_str!("fixtures/fix/brackets.reference.after.rb"),
    ];
    for (cop, after) in bracket_cops().into_iter().zip(expected) {
        for format in ["text", "github"] {
            let dir = project(&[("x.rb", before)]);
            let path = dir.path().join("x.rb");
            let plain = rustocop(dir.path(), &["--only", cop, "--format", format, "x.rb"], "");
            assert_eq!(plain.status.code(), Some(1), "{plain:?}");
            assert!(stdout_lines(&plain).iter().any(|line| line.contains(cop)));
            let other = bracket_cops()
                .into_iter()
                .find(|name| *name != cop)
                .unwrap();
            assert!(
                stdout_lines(&plain)
                    .iter()
                    .all(|line| !line.contains(other))
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
            let fixed = rustocop(
                dir.path(),
                &["--fix", "--only", cop, "--format", format, "x.rb"],
                "",
            );
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
            let metadata = std::fs::metadata(&path).unwrap().modified().unwrap();
            let again = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(again.status.code(), Some(0), "{again:?}");
            assert_eq!(
                stdout_lines(&again).last().unwrap(),
                "1 file inspected, 0 corrections applied, 0 offenses remaining"
            );
            assert_eq!(
                std::fs::metadata(&path).unwrap().modified().unwrap(),
                metadata
            );
        }
    }
    let dir = project(&[("x.rb", before)]);
    let plain = rustocop(dir.path(), &["x.rb"], "");
    for cop in bracket_cops() {
        assert!(
            stdout_lines(&plain).iter().any(|line| line.contains(cop)),
            "{plain:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        before
    );
}

#[test]
fn bracket_spacing_constant_pattern_review_regressions_fix_in_one_pass() {
    let before = "x in A(a: ^([ 1 ]))\nx in A(^(obj.[](1)), [ a ])\nx in A([ a ], B[ b ])\n[ \n=begin\ncomment\n=end\n 1\n]\n";
    let after = "x in A(a: ^([1]))\nx in A(^(obj.[](1)), [a])\nx in A([a ], B[b])\n[ \n=begin\ncomment\n=end\n 1\n]\n";
    let dir = project(&[("x.rb", before)]);
    let path = dir.path().join("x.rb");
    let cop = bracket_cops()[0];
    let plain = rustocop(dir.path(), &["--only", cop, "x.rb"], "");
    assert_eq!(plain.status.code(), Some(1), "{plain:?}");
    assert_eq!(
        stdout_lines(&plain)
            .iter()
            .filter(|line| line.contains(cop))
            .count(),
        7
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
    assert_eq!(
        stdout_lines(&fixed).last().unwrap(),
        "1 file inspected, 7 corrections applied, 0 offenses remaining"
    );
    let again = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
    assert_eq!(again.status.code(), Some(0), "{again:?}");
    assert_eq!(
        stdout_lines(&again).last().unwrap(),
        "1 file inspected, 0 corrections applied, 0 offenses remaining"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
}

#[test]
fn bracket_spacing_honors_directives_config_exclusions_and_except() {
    for (cop, before, after) in [
        (bracket_cops()[0], "[ 1 ]\n", "[1]\n"),
        (bracket_cops()[1], "a[ 1 ]\n", "a[1]\n"),
    ] {
        for prefix in ["rubocop", "rustocop"] {
            let before = before.trim_end();
            let after = after.trim_end();
            for target in [cop, "Layout", "all"] {
                let source = format!(
                    "# {prefix}:disable {target}\n{before}\n# {prefix}:enable {target}\n{before} # {prefix}:disable {target}\n{before}\n"
                );
                let expected = format!(
                    "# {prefix}:disable {target}\n{before}\n# {prefix}:enable {target}\n{before} # {prefix}:disable {target}\n{after}\n"
                );
                let dir = project(&[("x.rb", &source)]);
                let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    expected
                );
            }
        }
        for config in [
            "[cops.\"Layout\"]\nenabled = false\n".to_string(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
            "exclude = [\"x.rb\"]\n".to_string(),
        ] {
            let dir = project(&[("x.rb", before), ("rustocop.toml", &config)]);
            let args = if config.contains("exclude") {
                vec!["--fix", "--only", cop, "x.rb"]
            } else {
                vec!["--fix", "--except", "Style", "x.rb"]
            };
            let fixed = rustocop(dir.path(), &args, "");
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            if !config.contains("exclude") {
                let except = rustocop(
                    dir.path(),
                    &["--fix", "--only", cop, "--except", cop, "x.rb"],
                    "",
                );
                assert_eq!(except.status.code(), Some(0), "{except:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    before
                );
                let forced = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(forced.status.code(), Some(0), "{forced:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    after
                );
            }
        }
    }
}

#[test]
fn bracket_spacing_preserves_raw_bytes_crlf_bom_and_data() {
    let selected = bracket_cops().join(",");
    for (before, after) in [
        (b"# coding: ascii-8bit\r\na = [ \"\xff\" ]\r\na[ 0 ]\r\n__END__\r\n[ x ] a[ x ]\xff\r\n".as_slice(), b"# coding: ascii-8bit\r\na = [\"\xff\"]\r\na[0]\r\n__END__\r\n[ x ] a[ x ]\xff\r\n".as_slice()),
        ("\u{feff}a = [ \"é\" ]\r\na[ 0 ]\r\n".as_bytes(), "\u{feff}a = [\"é\"]\r\na[0]\r\n".as_bytes()),
    ] {
        let dir = project(&[]);
        let path = dir.path().join("x.rb");
        std::fs::write(&path, before).unwrap();
        let plain = rustocop(dir.path(), &["--only", &selected, "x.rb"], "");
        assert_eq!(plain.status.code(), Some(1), "{plain:?}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let fixed = rustocop(dir.path(), &["--fix", "--only", &selected, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after);
    }
}

#[test]
fn bracket_spacing_clean_and_withheld_only_files_are_not_written() {
    let selected = bracket_cops().join(",");
    for (source, status) in [
        ("", 0),
        ("__END__\n[ x ] a[ x ]\n", 0),
        ("x = [\n]\na[\n]\np __LINE__\n", 1),
        ("x = [%q x ]\na[%q x ]\n", 1),
    ] {
        let dir = project(&[("x.rb", source)]);
        let path = dir.path().join("x.rb");
        let metadata = std::fs::metadata(&path).unwrap().modified().unwrap();
        let fixed = rustocop(dir.path(), &["--fix", "--only", &selected, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            metadata
        );
    }
}

#[test]
fn bracket_spacing_preserves_controlled_runtime_values_and_line_numbers() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("skipping controlled runtime check: ruby unavailable");
        return;
    }
    let selected = bracket_cops().join(",");
    for source in [
        "a = [ 1, [ 2, 3 ], *[4] ]\na[ 0 ] = 5\na[ 1 ][ 0 ] += 2\np a, a[ 1..2 ], a[ ..1 ], a[ 1... ], __LINE__\n",
        "a = [ \"é\", %q[ x ], %w[ a b ], /[ x ]/ ]\np a, \"[ x ] #{[ 1 ].inspect}\", __LINE__\n",
        "x = [ <<~H ]\n  [ x ] #{[ 1 ].inspect}\nH\np x, __LINE__\n",
        "a = [\n]\np a, __LINE__\ndef line; __LINE__; end\np line\n",
        "a = Object.new\ndef a.[](*args); args; end\np a[\n], __LINE__\n",
        "p [\n], [ \\\n ]\n",
        "p [%q x ], [ %q x  ]\n",
        "x = <<~H\n  #{[\n].inspect}\n  tail\nH\np x\n",
        "a = [ 1, 2 ]\na => [ first, last ]\np first, last, __LINE__\n",
        "a = [ <<~H, [ 2 ] ]\n  a[ x ] #{[ 3 ].inspect}\nH\np a, __LINE__\n",
    ] {
        let dir = project(&[("x.rb", source)]);
        let run = || {
            let output = Command::new("ruby")
                .arg("x.rb")
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(output.status.success(), "{source:?}: {output:?}");
            output.stdout
        };
        let before = run();
        let fixed = rustocop(dir.path(), &["--fix", "--only", &selected, "x.rb"], "");
        assert!(matches!(fixed.status.code(), Some(0 | 1)), "{fixed:?}");
        assert_eq!(run(), before, "{source}");
    }
}

#[test]
fn paren_spacing_plain_lint_is_read_only_default_enabled_and_fix_is_stable() {
    for format in ["text", "github"] {
        let dir = project(&[]);
        let path = dir.path().join("x.rb");
        let before = b"# coding: ascii-8bit\r\nf( \"\xff\" )\r\n__END__\r\nraw ( x )\xff\r\n";
        let after = b"# coding: ascii-8bit\r\nf(\"\xff\")\r\n__END__\r\nraw ( x )\xff\r\n";
        std::fs::write(&path, before).unwrap();
        let plain = rustocop(dir.path(), &["--format", format, "x.rb"], "");
        assert_eq!(plain.status.code(), Some(1), "{plain:?}");
        assert!(
            stdout_lines(&plain)
                .iter()
                .any(|s| s.contains("Layout/SpaceInsideParens"))
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let fixed = rustocop(
            dir.path(),
            &[
                "--fix",
                "--only",
                "Layout/SpaceInsideParens",
                "--format",
                format,
                "x.rb",
            ],
            "",
        );
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after);
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 2 corrections applied, 0 offenses remaining"
        );
        let again = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/SpaceInsideParens", "x.rb"],
            "",
        );
        assert_eq!(again.status.code(), Some(0), "{again:?}");
        assert_eq!(
            stdout_lines(&again).last().unwrap(),
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        );
        assert_eq!(std::fs::read(&path).unwrap(), after);
    }
}

#[test]
fn paren_spacing_pinned_token_exemptions_are_not_rewritten() {
    let source = "f ( x)\nf(\"a\\\nb\" )\n";
    let dir = project(&[("x.rb", source)]);
    for fix in [false, true] {
        let mut args = vec!["--only", "Layout/SpaceInsideParens", "x.rb"];
        if fix {
            args.push("--fix");
        }
        let output = rustocop(dir.path(), &args, "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            source
        );
        assert_eq!(
            stdout_lines(&output).last().unwrap(),
            if fix {
                "1 file inspected, 0 corrections applied, 0 offenses remaining"
            } else {
                "1 file inspected, no offenses detected"
            }
        );
    }
}

#[test]
fn paren_spacing_honors_directives_config_excludes_and_selection() {
    let cop = "Layout/SpaceInsideParens";
    for prefix in ["rustocop", "rubocop"] {
        for target in [cop, "Layout", "all"] {
            let before = format!(
                "# {prefix}:disable {target}\nf( 1 )\n# {prefix}:enable {target}\nf( 1 ) # {prefix}:disable {target}\nf( 1 )\n"
            );
            let after = format!(
                "# {prefix}:disable {target}\nf( 1 )\n# {prefix}:enable {target}\nf( 1 ) # {prefix}:disable {target}\nf(1)\n"
            );
            let dir = project(&[("x.rb", &before)]);
            let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                after
            );
        }
    }
    for config in [
        "[cops.\"Layout\"]\nenabled = false\n".to_string(),
        format!("[cops.\"{cop}\"]\nenabled = false\n"),
        format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
        "exclude = [\"x.rb\"]\n".to_string(),
    ] {
        let before = "f( 1 )\n";
        let dir = project(&[("x.rb", before), ("rustocop.toml", &config)]);
        let args = if config.contains("exclude") {
            vec!["--fix", "--only", cop, "x.rb"]
        } else {
            vec!["--fix", "--except", "Style", "x.rb"]
        };
        let output = rustocop(dir.path(), &args, "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            before
        );
        if !config.contains("exclude") {
            let except = rustocop(
                dir.path(),
                &["--fix", "--only", cop, "--except", cop, "x.rb"],
                "",
            );
            assert_eq!(except.status.code(), Some(0), "{except:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                before
            );
            let only = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(only.status.code(), Some(0), "{only:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                "f(1)\n"
            );
        }
    }
}

#[test]
fn paren_spacing_and_redundant_parentheses_have_deterministic_overlap_selection() {
    let before = "x = ( 1 )\ny = ( a + b ) * c\nz = ( # keep\n 3 ) # rustocop:disable Style/RedundantParentheses\n";
    let after =
        "x = 1\ny = (a + b) * c\nz = ( # keep\n 3) # rustocop:disable Style/RedundantParentheses\n";
    for only in [
        "Layout/SpaceInsideParens,Style/RedundantParentheses",
        "Style/RedundantParentheses,Layout/SpaceInsideParens",
    ] {
        let dir = project(&[("x.rb", before)]);
        let fixed = rustocop(dir.path(), &["--fix", "--only", only, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            after
        );
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 4 corrections applied, 0 offenses remaining"
        );
        let again = rustocop(dir.path(), &["--fix", "--only", only, "x.rb"], "");
        assert_eq!(again.status.code(), Some(0), "{again:?}");
        assert_eq!(
            stdout_lines(&again).last().unwrap(),
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            after
        );
    }
}

#[test]
fn paren_spacing_preserves_controlled_runtime_values_and_line_numbers() {
    // Controlled synthetic fixture only; never execute application/corpus code.
    let source = "values = [\"( x )\", /( x )/.source, %q( x ), %w( x ), ?(, ?)]; values << ( 1 + 2 )\nvalues << %Q( literal ( x ) #{( 3 )})\nvalues << <<~OUT\n  ( x ) #{<<~IN}\n    ( x ) #{( 4 )}\nIN\n  tail ( x )\nOUT\nvalues << ( 1; # keep\n  2 )\nvalues << __LINE__\nclass Lines\n  def self.line; __LINE__; end\nend\nvalues << Lines.line\np( values )\n__END__\nraw ( x )\n";
    let run = |source: &str| {
        let output = Command::new("ruby").args(["-e", source]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        output.stdout
    };
    let expected = run(source);
    for source in [source.to_string(), source.replace('\n', "\r\n")] {
        let dir = project(&[("x.rb", &source)]);
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/SpaceInsideParens", "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        let after = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
        assert_ne!(after, source);
        assert_eq!(run(&after), expected);
        assert_eq!(after.lines().count(), source.lines().count());
        assert!(after.contains("# keep"));
        assert!(after.ends_with(&source[source.find("__END__").unwrap()..]));
        assert_eq!(
            after.matches("\r\n").count(),
            source.matches("\r\n").count()
        );
    }
}

#[test]
fn separator_cops_plain_lint_is_read_only_and_only_fixes_independently() {
    for (cop, before, after) in [
        ("Layout/SpaceAfterComma", "f(1 ,2);g\r\n", "f(1 , 2);g\r\n"),
        ("Layout/SpaceBeforeComma", "f(1 ,2);g\r\n", "f(1,2);g\r\n"),
        (
            "Layout/SpaceAfterSemicolon",
            "f(1 ,2);g\r\n",
            "f(1 ,2); g\r\n",
        ),
    ] {
        for format in ["text", "github"] {
            let dir = project(&[("x.rb", before)]);
            let path = dir.path().join("x.rb");
            let plain = rustocop(dir.path(), &["--only", cop, "--format", format, "x.rb"], "");
            assert_eq!(plain.status.code(), Some(1), "{cop}: {plain:?}");
            assert!(plain.stderr.is_empty(), "{plain:?}");
            assert!(
                stdout_lines(&plain).iter().any(|s| s.contains(cop)),
                "{plain:?}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
            let fixed = rustocop(
                dir.path(),
                &["--fix", "--only", cop, "--format", format, "x.rb"],
                "",
            );
            assert_eq!(fixed.status.code(), Some(0), "{cop}: {fixed:?}");
            assert!(fixed.stderr.is_empty(), "{fixed:?}");
            assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
            assert_eq!(
                stdout_lines(&fixed).last().unwrap(),
                "1 file inspected, 1 correction applied, 0 offenses remaining"
            );
            let again = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
            assert_eq!(again.status.code(), Some(0), "{again:?}");
            assert_eq!(
                stdout_lines(&again).last().unwrap(),
                "1 file inspected, 0 corrections applied, 0 offenses remaining"
            );
            assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
        }
    }
}

#[test]
fn separator_cops_preserve_eof_payload_bytes() {
    let only = "Layout/SpaceAfterComma,Layout/SpaceBeforeComma,Layout/SpaceAfterSemicolon";
    for marker in ['\0', '\u{4}', '\u{1a}'] {
        for (code, corrected, count) in [
            ("p 1\n".to_string(), "p 1\n".to_string(), 0),
            (
                format!("f(\"{marker}\" ,2);g\r\n"),
                format!("f(\"{marker}\", 2); g\r\n"),
                3,
            ),
        ] {
            let source = format!("{code}{marker}f(1 ,2);g\r\n");
            let mut expected = format!("{corrected}{marker}f(1 ,2);g\r\n").into_bytes();
            let dir = project(&[("x.rb", &source)]);
            let path = dir.path().join("x.rb");
            // Ignored payload need not even be valid UTF-8.
            let mut bytes = source.into_bytes();
            bytes.extend_from_slice(b"\xff ,;payload\n");
            expected.extend_from_slice(b"\xff ,;payload\n");
            std::fs::write(&path, &bytes).unwrap();
            let plain = rustocop(dir.path(), &["--only", only, "x.rb"], "");
            assert_eq!(
                plain.status.code(),
                Some(i32::from(count != 0)),
                "{plain:?}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            let fixed = rustocop(dir.path(), &["--fix", "--only", only, "x.rb"], "");
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert_eq!(std::fs::read(&path).unwrap(), expected);
            assert_eq!(
                stdout_lines(&fixed).last().unwrap(),
                &format!(
                    "1 file inspected, {count} correction{} applied, 0 offenses remaining",
                    if count == 1 { "" } else { "s" }
                )
            );
        }
    }
}

#[test]
fn separator_cops_fix_nested_heredoc_code_but_preserve_literal_text() {
    let only = "Layout/SpaceAfterComma,Layout/SpaceBeforeComma,Layout/SpaceAfterSemicolon";
    // Multiple levels, two heredocs on one opener line, and literal lookalikes.
    let source = "x = <<~OUT\n  outer ,;\n  #{[<<~MID,<<~SIDE].join}\n    mid ,; #{<<~IN}\n      inner ,; #{[1 ,2].join(',');nil}\nIN\n    mid tail ,;\nMID\n    side ,; #{[3 ,4].join(',');nil}\nSIDE\n  outer tail ,;\nOUT\n";
    let expected = source
        .replace("<<~MID,<<~SIDE", "<<~MID, <<~SIDE")
        .replace("[1 ,2]", "[1, 2]")
        .replace("[3 ,4]", "[3, 4]")
        .replace(";nil", "; nil");
    for (source, expected) in [
        (source.to_string(), expected.clone()),
        (source.replace('\n', "\r\n"), expected.replace('\n', "\r\n")),
    ] {
        let dir = project(&[("x.rb", &source)]);
        let fixed = rustocop(dir.path(), &["--fix", "--only", only, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(
            std::fs::read(dir.path().join("x.rb")).unwrap(),
            expected.as_bytes()
        );
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 7 corrections applied, 0 offenses remaining"
        );
        let again = rustocop(dir.path(), &["--fix", "--only", only, "x.rb"], "");
        assert_eq!(again.status.code(), Some(0), "{again:?}");
        assert_eq!(
            stdout_lines(&again).last().unwrap(),
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        );
    }
}

#[test]
fn separator_cops_honor_config_excludes_and_selection_precedence() {
    for (cop, source) in [
        ("Layout/SpaceAfterComma", "f(1,2)\n"),
        ("Layout/SpaceBeforeComma", "f(1 , 2)\n"),
        ("Layout/SpaceAfterSemicolon", "f;g\n"),
    ] {
        for config in [
            "[cops.\"Layout\"]\nenabled = false\n".to_string(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
            "exclude = [\"x.rb\"]\n".to_string(),
        ] {
            let dir = project(&[("x.rb", source), ("rustocop.toml", &config)]);
            // --only bypasses disable flags, but not file exclusions.
            let args = if config.contains("exclude") {
                vec!["--fix", "--only", cop, "x.rb"]
            } else {
                vec!["--fix", "--except", "Style", "x.rb"]
            };
            let output = rustocop(dir.path(), &args, "");
            assert_eq!(output.status.code(), Some(0), "{config}: {output:?}");
            assert_eq!(
                std::fs::read(dir.path().join("x.rb")).unwrap(),
                source.as_bytes()
            );
        }
        let dir = project(&[
            ("x.rb", source),
            ("rustocop.toml", "[cops.\"Layout\"]\nenabled = false\n"),
        ]);
        let except = rustocop(
            dir.path(),
            &["--fix", "--only", cop, "--except", cop, "x.rb"],
            "",
        );
        assert_eq!(except.status.code(), Some(0), "{except:?}");
        assert_eq!(
            std::fs::read(dir.path().join("x.rb")).unwrap(),
            source.as_bytes()
        );
        let fixed = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_ne!(
            std::fs::read(dir.path().join("x.rb")).unwrap(),
            source.as_bytes()
        );
    }
}

#[test]
fn separator_cops_honor_cop_department_and_all_directives() {
    for (cop, before, after) in [
        ("Layout/SpaceAfterComma", "f(1,2)", "f(1, 2)"),
        ("Layout/SpaceBeforeComma", "f(1 , 2)", "f(1, 2)"),
        ("Layout/SpaceAfterSemicolon", "x = 1;y = 2", "x = 1; y = 2"),
    ] {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Layout", "all"] {
                let source = format!(
                    "# {prefix}:disable {target}\n{before}\n# {prefix}:enable {target}\n{before} # {prefix}:disable {target}\n{before}\n"
                );
                let expected = format!(
                    "# {prefix}:disable {target}\n{before}\n# {prefix}:enable {target}\n{before} # {prefix}:disable {target}\n{after}\n"
                );
                let dir = project(&[("x.rb", &source)]);
                let output = rustocop(dir.path(), &["--fix", "--only", cop, "x.rb"], "");
                assert_eq!(output.status.code(), Some(0), "{output:?}");
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                    expected
                );
                assert_eq!(
                    stdout_lines(&output).last().unwrap(),
                    "1 file inspected, 1 correction applied, 0 offenses remaining"
                );
            }
        }
    }
}

#[test]
fn separator_and_style_semicolon_conflicts_remain_deterministic_single_pass() {
    let before = "f(1 ,2);g(3,4)\n";
    for only in [
        "Style/Semicolon,Layout/SpaceAfterComma,Layout/SpaceBeforeComma,Layout/SpaceAfterSemicolon",
        "Layout/SpaceAfterSemicolon,Layout/SpaceBeforeComma,Layout/SpaceAfterComma,Style/Semicolon",
    ] {
        let dir = project(&[("x.rb", before)]);
        let fixed = rustocop(dir.path(), &["--fix", "--only", only, "x.rb"], "");
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            "f(1, 2)\ng(3, 4)\n"
        );
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 4 corrections applied, 0 offenses remaining"
        );
    }
    // With a heredoc opener or a later __LINE__, Style's newline-moving fix
    // is withheld, but Layout's same-line insertion remains safe and useful.
    for (before, after) in [
        ("x = <<~H;p x\ntext,;\nH\n", "x = <<~H; p x\ntext,;\nH\n"),
        ("x = 1;p x\np __LINE__\n", "x = 1; p x\np __LINE__\n"),
    ] {
        let dir = project(&[("x.rb", before)]);
        let fixed = rustocop(
            dir.path(),
            &[
                "--fix",
                "--only",
                "Style/Semicolon,Layout/SpaceAfterSemicolon",
                "x.rb",
            ],
            "",
        );
        assert_eq!(fixed.status.code(), Some(1), "{fixed:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            after
        );
        assert_eq!(
            stdout_lines(&fixed).last().unwrap(),
            "1 file inspected, 1 correction applied, 1 offense remaining"
        );
    }
}

#[test]
fn separator_fixes_preserve_controlled_runtime_literals_and_line_values() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("skipping controlled runtime check: ruby unavailable");
        return;
    }
    // Controlled synthetic code only; never execute application/corpus code.
    let source = "values = ['a,b;c',/a,b;c/.source,%q;a,b\\;c;,?,,?;, $,, $;];values << [1 ,2]\nvalues << \"#{[3 ,4].join(',')}\"\nvalues << %W;literal\\;word#{[5 ,6].join(',')};\nvalues << <<~H;values << __LINE__\n  literal,; #{[7 ,8].join(',')}\nH\nclass Lines\n  def self.line;__LINE__;end\nend\nvalues << Lines.line;p values\n__END__\nraw,;\n";
    let run = |source: &str| {
        let output = Command::new("ruby").args(["-e", source]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        output.stdout
    };
    let source = format!(
        "values = []\nvalues << <<~OUT\n  outer ,; #{{<<~IN}}\n    inner ,; #{{[9 ,10].join(',');nil}}\nIN\n  tail ,;\nOUT\np values\n{source}"
    );
    let expected = run(&source);
    let dir = project(&[("x.rb", &source)]);
    let fixed = rustocop(
        dir.path(),
        &[
            "--fix",
            "--only",
            "Layout/SpaceAfterComma,Layout/SpaceBeforeComma,Layout/SpaceAfterSemicolon",
            "x.rb",
        ],
        "",
    );
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    let corrected = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
    assert_ne!(corrected, source);
    assert_eq!(run(&corrected), expected);
    assert_eq!(corrected.lines().count(), source.lines().count());
    assert!(corrected.ends_with("__END__\nraw,;\n"));
}

#[test]
fn separator_cops_are_default_enabled_and_preserve_raw_encoding_bytes() {
    let dir = project(&[]);
    let path = dir.path().join("x.rb");
    let before = b"# coding: ascii-8bit\nf(\"\xff\" ,2);g\r\n__END__\r\nraw,;\xff\r\n";
    std::fs::write(&path, before).unwrap();
    let plain = rustocop(dir.path(), &["x.rb"], "");
    assert_eq!(plain.status.code(), Some(1), "{plain:?}");
    for cop in [
        "Layout/SpaceAfterComma",
        "Layout/SpaceBeforeComma",
        "Layout/SpaceAfterSemicolon",
    ] {
        assert!(
            stdout_lines(&plain).iter().any(|s| s.contains(cop)),
            "{plain:?}"
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let fixed = rustocop(
        dir.path(),
        &[
            "--fix",
            "--only",
            "Layout/SpaceAfterComma,Layout/SpaceBeforeComma,Layout/SpaceAfterSemicolon",
            "x.rb",
        ],
        "",
    );
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"# coding: ascii-8bit\nf(\"\xff\", 2); g\r\n__END__\r\nraw,;\xff\r\n"
    );
}

#[test]
fn trailing_empty_lines_plain_lint_is_read_only_and_fixes_exact_eof_bytes() {
    for (before, after) in [
        ("puts 1", "puts 1\n"),
        ("\n\n\n\n", "\n"),
        (
            "\u{feff}#!/usr/bin/ruby\r\n# frozen_string_literal: true\r\nputs \"é\"\r\n \t\r\n\r\n",
            "\u{feff}#!/usr/bin/ruby\r\n# frozen_string_literal: true\r\nputs \"é\"\r\n",
        ),
    ] {
        for format in ["text", "github"] {
            let dir = project(&[("x.rb", before)]);
            let path = dir.path().join("x.rb");
            let plain = rustocop(dir.path(), &["--format", format, "x.rb"], "");
            assert_eq!(plain.status.code(), Some(1), "{plain:?}");
            assert!(plain.stderr.is_empty(), "{plain:?}");
            assert!(
                stdout_lines(&plain)
                    .iter()
                    .any(|line| line.contains("Layout/TrailingEmptyLines"))
            );
            assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
            let fixed = rustocop(
                dir.path(),
                &[
                    "--fix",
                    "--only",
                    "Layout/TrailingEmptyLines",
                    "--format",
                    format,
                    "x.rb",
                ],
                "",
            );
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
            assert!(fixed.stderr.is_empty(), "{fixed:?}");
            assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
            assert_eq!(
                stdout_lines(&fixed).last().unwrap(),
                "1 file inspected, 1 correction applied, 0 offenses remaining"
            );
            let again = rustocop(
                dir.path(),
                &["--fix", "--only", "Layout/TrailingEmptyLines", "x.rb"],
                "",
            );
            assert_eq!(again.status.code(), Some(0), "{again:?}");
            assert_eq!(
                stdout_lines(&again).last().unwrap(),
                "1 file inspected, 0 corrections applied, 0 offenses remaining"
            );
            assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
        }
    }
}

#[test]
fn trailing_empty_lines_no_write_for_canonical_data_or_unsafe_only_files() {
    for (source, status) in [
        ("", 0),
        ("\n", 0),
        ("puts 1\r\n", 0),
        ("__END__\r\nraw \t\r\n\r\n", 0),
        ("p %q a ", 1),
        ("p value = %\n\n\n", 1),
        ("# rubocop:disable Layout/TrailingEmptyLines\nputs 1\n\n", 0),
    ] {
        let dir = project(&[("x.rb", source)]);
        let path = dir.path().join("x.rb");
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let output = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/TrailingEmptyLines", "x.rb"],
            "",
        );
        assert_eq!(output.status.code(), Some(status), "{source:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(
            stdout_lines(&output)
                .last()
                .unwrap()
                .contains("0 corrections applied")
        );
        assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes());
    }
}

#[test]
fn trailing_empty_lines_honors_config_and_selectors() {
    for config in [
        "[cops.\"Layout\"]\nenabled = false\n",
        "[cops.\"Layout/TrailingEmptyLines\"]\nenabled = false\n",
        "[cops.\"Layout/TrailingEmptyLines\"]\nexclude = [\"x.rb\"]\n",
        "exclude = [\"x.rb\"]\n",
    ] {
        let dir = project(&[("x.rb", "puts 1"), ("rustocop.toml", config)]);
        let output = rustocop(dir.path(), &["--fix", "x.rb"], "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(std::fs::read(dir.path().join("x.rb")).unwrap(), b"puts 1");
    }
    let dir = project(&[
        ("x.rb", "puts 1"),
        ("rustocop.toml", "[cops.\"Layout\"]\nenabled = false\n"),
    ]);
    let except = rustocop(
        dir.path(),
        &[
            "--fix",
            "--only",
            "Layout",
            "--except",
            "Layout/TrailingEmptyLines",
            "x.rb",
        ],
        "",
    );
    assert_eq!(except.status.code(), Some(0), "{except:?}");
    assert_eq!(std::fs::read(dir.path().join("x.rb")).unwrap(), b"puts 1");
    let fixed = rustocop(
        dir.path(),
        &["--fix", "--only", "Layout/TrailingEmptyLines", "x.rb"],
        "",
    );
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    assert_eq!(std::fs::read(dir.path().join("x.rb")).unwrap(), b"puts 1\n");
}

#[test]
fn trailing_empty_lines_and_trailing_whitespace_compose_in_explicit_passes() {
    let before = "puts \"é\" \t\r\n \t\r\n\r\n";
    let dir = project(&[("x.rb", before)]);
    let args = [
        "--fix",
        "--fix-passes",
        "1",
        "--only",
        "Layout/TrailingEmptyLines,Layout/TrailingWhitespace",
        "x.rb",
    ];
    let first = rustocop(dir.path(), &args, "");
    assert_eq!(first.status.code(), Some(1), "{first:?}");
    assert_eq!(
        std::fs::read(dir.path().join("x.rb")).unwrap(),
        b"puts \"\xc3\xa9\"\r\n\r\n\r\n"
    );
    let second = rustocop(dir.path(), &args, "");
    assert_eq!(second.status.code(), Some(0), "{second:?}");
    assert_eq!(
        std::fs::read(dir.path().join("x.rb")).unwrap(),
        "puts \"é\"\r\n".as_bytes()
    );
    let third = rustocop(dir.path(), &args, "");
    assert_eq!(third.status.code(), Some(0), "{third:?}");
    assert!(
        stdout_lines(&third)
            .last()
            .unwrap()
            .contains("0 corrections applied")
    );
}

#[test]
fn trailing_empty_lines_preserves_controlled_runtime_values_and_line_numbers() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("skipping controlled runtime check: ruby unavailable");
        return;
    }
    for (source, status) in [
        (
            "p ['a\\ \nb', \"a\\ \n#{1}b\", /a\nb/.source, %q(a\nb), %w(a b)]\nclass Lines\n  def self.line\n    __LINE__\n  end\nend\np [__LINE__, Lines.line]\np <<~H\n  body  \n \nH\n\n\n",
            0,
        ),
        ("p %q a ", 1),
        ("p value = %\n\n\n", 1),
    ] {
        let run = |text: &str| {
            let output = Command::new("ruby").args(["-e", text]).output().unwrap();
            assert!(output.status.success(), "{output:?}");
            output.stdout
        };
        let expected = run(source);
        let dir = project(&[("x.rb", source)]);
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/TrailingEmptyLines", "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(status), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        let corrected = std::fs::read_to_string(dir.path().join("x.rb")).unwrap();
        assert_eq!(run(&corrected), expected);
        if status == 0 {
            assert_eq!(corrected, source.trim_end_matches('\n').to_owned() + "\n");
        } else {
            assert_eq!(corrected, source);
        }
    }
}

#[test]
fn trailing_empty_lines_preserves_binary_source_and_payload_bytes() {
    for (before, after) in [
        (
            b"# coding: ASCII-8BIT\r\nputs \"\xff\"".as_slice(),
            b"# coding: ASCII-8BIT\r\nputs \"\xff\"\r\n".as_slice(),
        ),
        (
            b"# coding: ASCII-8BIT\r\nputs \"\xff\"\r\n__END__\r\n\xff\r\n\r\n".as_slice(),
            b"# coding: ASCII-8BIT\r\nputs \"\xff\"\r\n__END__\r\n\xff\r\n\r\n".as_slice(),
        ),
    ] {
        let dir = project(&[]);
        let path = dir.path().join("x.rb");
        std::fs::write(&path, before).unwrap();
        let output = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/TrailingEmptyLines", "x.rb"],
            "",
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after);
    }
}

#[test]
fn trailing_whitespace_honors_config_selections_and_directives() {
    for config in [
        "[cops.\"Layout\"]\nenabled = false\n",
        "[cops.\"Layout/TrailingWhitespace\"]\nenabled = false\n",
        "[cops.\"Layout/TrailingWhitespace\"]\nexclude = [\"x.rb\"]\n",
    ] {
        let dir = project(&[("x.rb", "puts 1 \n"), ("rustocop.toml", config)]);
        let output = rustocop(dir.path(), &["--fix", "x.rb"], "");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            std::fs::read(dir.path().join("x.rb")).unwrap(),
            b"puts 1 \n"
        );
    }
    for prefix in ["rustocop", "rubocop"] {
        let before = format!("puts 1 # {prefix}:disable Layout \nputs 2 \n");
        let after = format!("puts 1 # {prefix}:disable Layout \nputs 2\n");
        let dir = project(&[
            ("x.rb", &before),
            ("rustocop.toml", "[cops.\"Layout\"]\nenabled = false\n"),
        ]);
        let except = rustocop(
            dir.path(),
            &[
                "--fix",
                "--only",
                "Layout",
                "--except",
                "Layout/TrailingWhitespace",
                "x.rb",
            ],
            "",
        );
        assert_eq!(except.status.code(), Some(0), "{except:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            before
        );
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/TrailingWhitespace", "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            after
        );
    }
}

#[test]
fn trailing_whitespace_uses_declared_encoding_for_detection_and_byte_edits() {
    for (before, after, position) in [
        (
            b"# coding: ascii-8bit\n# comment\xe3\x80\x80\n".as_slice(),
            b"# coding: ascii-8bit\n# comment\xe3\x80\x80\n".as_slice(),
            None,
        ),
        (
            b"# coding: iso-8859-1\n# comment\xa0\n".as_slice(),
            b"# coding: iso-8859-1\n# comment\n".as_slice(),
            Some("x.rb:2:10:"),
        ),
    ] {
        let dir = project(&[]);
        let path = dir.path().join("x.rb");
        std::fs::write(&path, before).unwrap();
        let plain = rustocop(
            dir.path(),
            &["--only", "Layout/TrailingWhitespace", "x.rb"],
            "",
        );
        assert_eq!(
            plain.status.code(),
            Some(i32::from(position.is_some())),
            "{plain:?}"
        );
        assert!(plain.stderr.is_empty(), "{plain:?}");
        if let Some(position) = position {
            assert!(
                stdout_lines(&plain)
                    .iter()
                    .any(|line| line.contains(position)),
                "{plain:?}"
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let fixed = rustocop(
            dir.path(),
            &["--fix", "--only", "Layout/TrailingWhitespace", "x.rb"],
            "",
        );
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after);
    }
}

#[test]
fn fix_applies_real_style_corrections_and_reports_remaining_in_both_formats() {
    for format in ["text", "github"] {
        let dir = project(&[("x.rb", "puts 'abc';\nbinding.pry\nfail\n")]);
        let output = rustocop(dir.path(), &["--fix", "--format", format, "x.rb"], "");
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            "puts \"abc\"\nbinding.pry\nraise\n"
        );
        let lines = stdout_lines(&output);
        assert!(lines.iter().any(|line| line.contains("Lint/Debugger")));
        assert_eq!(
            lines.last().unwrap(),
            "1 file inspected, 3 corrections applied, 1 offense remaining"
        );
    }
}

#[test]
fn fix_style_honors_config_and_directives() {
    let before = "x = 'a' # rubocop:disable Style/StringLiterals\ny = 'b'\nfail\n";
    let dir = project(&[
        ("x.rb", before),
        (
            "rustocop.toml",
            "[cops.\"Style/SignalException\"]\nexclude = [\"x.rb\"]\n",
        ),
    ]);
    let detection = rustocop(
        dir.path(),
        &[
            "--only",
            "Style/StringLiterals,Style/SignalException",
            "x.rb",
        ],
        "",
    );
    assert_eq!(detection.status.code(), Some(1));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        before
    );
    let output = rustocop(
        dir.path(),
        &[
            "--fix",
            "--only",
            "Style/StringLiterals,Style/SignalException",
            "x.rb",
        ],
        "",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        "x = 'a' # rubocop:disable Style/StringLiterals\ny = \"b\"\nfail\n"
    );
    assert_eq!(
        stdout_lines(&output).last().unwrap(),
        "1 file inspected, 1 correction applied, 0 offenses remaining"
    );
}

#[test]
fn fix_reports_remaining_detection_only_offenses_in_both_formats() {
    let source = "binding.pry\n";
    let dir = project(&[("x.rb", source)]);
    for format in ["text", "github"] {
        let output = rustocop(
            dir.path(),
            &[
                "--fix",
                "--only",
                "Lint/Debugger",
                "--format",
                format,
                "x.rb",
            ],
            "",
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let lines = stdout_lines(&output);
        assert!(lines[0].contains("Remove debugger entry point `binding.pry`."));
        assert_eq!(
            lines.last().unwrap(),
            "1 file inspected, 0 corrections applied, 1 offense remaining"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            source
        );
    }
}

#[cfg(unix)]
#[test]
fn fix_preserves_path_specific_exclusions_across_symlink_aliases() {
    let dir = project(&[
        ("z.rb", "binding.pry\n"),
        (
            "rustocop.toml",
            "[cops.\"Lint/Debugger\"]\nexclude = [\"a.rb\"]\n",
        ),
    ]);
    std::os::unix::fs::symlink("z.rb", dir.path().join("a.rb")).unwrap();
    for fix in [false, true] {
        let mut args = vec!["--only", "Lint/Debugger", "a.rb", "z.rb"];
        if fix {
            args.push("--fix");
        }
        let output = rustocop(dir.path(), &args, "");
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let lines = stdout_lines(&output);
        assert!(lines[0].starts_with("z.rb:1:1: E: Remove debugger"));
        assert_eq!(
            lines.last().unwrap(),
            if fix {
                "2 files inspected, 0 corrections applied, 1 offense remaining"
            } else {
                "2 files inspected, 1 offense detected"
            }
        );
        assert_eq!(
            std::fs::read(dir.path().join("z.rb")).unwrap(),
            b"binding.pry\n"
        );
    }
}

#[test]
fn fix_honors_disabled_lines_and_does_not_rewrite_a_clean_file() {
    let source = "binding.pry # rubocop:disable Lint/Debugger\n";
    let dir = project(&[("x.rb", source)]);
    let path = dir.path().join("x.rb");
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    let output = rustocop(
        dir.path(),
        &["--fix", "--only", "Lint/Debugger", "x.rb"],
        "",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        stdout_lines(&output),
        [
            "",
            "1 file inspected, 0 corrections applied, 0 offenses remaining"
        ]
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn fix_rejects_stdin_without_touching_the_display_path() {
    let dir = project(&[("x.rb", "original\n")]);
    let output = rustocop(dir.path(), &["--fix", "--stdin", "x.rb"], "");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        "original\n"
    );
}

#[test]
fn fix_repeats_passes_until_nothing_changes_and_fix_passes_limits_them() {
    let source = "def func\n  return ({ :a => 'x' });\nend\n";
    let dir = project(&[("x.rb", source)]);
    let path = dir.path().join("x.rb");

    let output = rustocop(dir.path(), &["--fix", "--fix-passes", "1", "x.rb"], "");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "def func\n  { :a => 'x' }\nend\n"
    );

    std::fs::write(&path, source).unwrap();
    let output = rustocop(dir.path(), &["--fix", "x.rb"], "");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "def func\n  { a: \"x\" }\nend\n"
    );

    let output = rustocop(dir.path(), &["--fix", "x.rb"], "");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("0 corrections applied"),
        "{output:?}"
    );
}

#[test]
fn fix_passes_needs_fix_and_a_positive_number() {
    let dir = project(&[("x.rb", "original\n")]);
    for args in [
        vec!["--fix-passes", "2", "x.rb"],
        vec!["--fix", "--fix-passes", "0", "x.rb"],
        vec!["--fix", "--fix-passes", "many", "x.rb"],
    ] {
        let output = rustocop(dir.path(), &args, "");
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            "original\n",
            "{args:?}"
        );
    }
}

#[test]
fn fix_does_not_rewrite_parse_errors_and_continues_to_other_files() {
    let dir = project(&[("bad.rb", "def oops(\n"), ("good.rb", "binding.pry\n")]);
    let output = rustocop(dir.path(), &["--fix", "--only", "Lint/Debugger"], "");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("good.rb:1:1: E: Remove debugger"));
    assert!(text.contains("bad.rb: error: syntax error"));
    assert!(text.contains("2 files inspected, 0 corrections applied, 1 offense remaining"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("bad.rb")).unwrap(),
        "def oops(\n"
    );
}

#[test]
fn default_and_dot_paths_are_normalized_in_text_output() {
    let dir = project(&[("ok.rb", "binding.pry\n")]);
    let absolute = dir.path().join("ok.rb");
    for args in [
        vec![],
        vec!["."],
        vec!["ok.rb"],
        vec![absolute.to_str().unwrap()],
    ] {
        let output = rustocop(dir.path(), &args, "");
        let lines = stdout_lines(&output);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {lines:#?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            lines,
            [
                "ok.rb:1:1: E: Remove debugger entry point `binding.pry`. (Lint/Debugger)",
                "",
                "1 file inspected, 1 offense detected",
            ],
            "{args:?}"
        );
    }
}

#[test]
fn default_and_dot_paths_are_normalized_in_github_output() {
    let dir = project(&[("ok.rb", "binding.pry\n")]);
    let absolute = dir.path().join("ok.rb");
    for path in [None, Some("."), Some("ok.rb"), absolute.to_str()] {
        let mut args = vec!["--format", "github"];
        args.extend(path);
        let output = rustocop(dir.path(), &args, "");
        let lines = stdout_lines(&output);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {lines:#?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            lines,
            [
                "::error file=ok.rb,line=1,col=1,title=Lint/Debugger::ok.rb:1:1: \
                 Lint/Debugger: Remove debugger entry point `binding.pry`.",
                "1 file inspected, 1 offense detected",
            ],
            "{args:?}"
        );
    }
}

#[test]
fn github_format_annotates_syntax_errors_with_file_line_and_message() {
    let dir = project(&[
        ("lib/broken.rb", "x = 1\ndef oops(a,\n  b\n"),
        ("lib/fine.rb", "binding.pry\n"),
    ]);
    let output = rustocop(dir.path(), &["--format", "github", "lib"], "");
    let lines = stdout_lines(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{lines:#?}\n{stderr}");
    assert!(stderr.is_empty(), "{stderr}");

    // The offense in the healthy file is still reported.
    assert_eq!(
        lines[0],
        "::error file=lib/fine.rb,line=1,col=1,title=Lint/Debugger::lib/fine.rb:1:1: \
         Lint/Debugger: Remove debugger entry point `binding.pry`."
    );
    // Every syntax error is an annotation on its line, naming file and message.
    let errors = &lines[1..lines.len() - 1];
    assert!(!errors.is_empty(), "{lines:#?}");
    for error in errors {
        assert!(
            error.starts_with("::error file=lib/broken.rb,line="),
            "{error}"
        );
        assert!(
            error.contains(",title=rustocop::lib/broken.rb: error: syntax error on line "),
            "{error}"
        );
    }
    assert!(
        errors.contains(
            &"::error file=lib/broken.rb,line=2,col=1,title=rustocop::lib/broken.rb: error: \
              syntax error on line 2: expected an `end` to close the `def` statement (Lint/Syntax)"
                .to_string()
        ),
        "{errors:#?}"
    );
    assert_eq!(
        lines.last().unwrap(),
        &format!(
            "2 files inspected, 1 offense detected, {} errors",
            errors.len()
        )
    );
}

#[test]
fn github_format_annotates_syntax_errors_from_stdin() {
    let dir = project(&[]);
    let output = rustocop(
        dir.path(),
        &["--format", "github", "--stdin", "app/x.rb"],
        "puts(1, ]\n",
    );
    let lines = stdout_lines(&output);
    assert_eq!(output.status.code(), Some(2), "{lines:#?}");
    let errors = &lines[..lines.len() - 1];
    assert!(!errors.is_empty(), "{lines:#?}");
    for error in errors {
        assert!(
            error.starts_with(
                "::error file=app/x.rb,line=1,col=9,title=rustocop::app/x.rb: error: \
                 syntax error on line 1: "
            ),
            "{error}"
        );
    }
}

#[test]
fn line_length_defaults_to_120_columns() {
    let source = format!("x = 1\n{}\n{}\n", "a".repeat(120), "b".repeat(121));
    let dir = project(&[("app/x.rb", &source)]);
    let output = rustocop(dir.path(), &["--only", "Layout/LineLength", "app"], "");
    let lines = stdout_lines(&output);
    assert_eq!(output.status.code(), Some(1), "{lines:#?}");
    assert_eq!(
        lines[0],
        "app/x.rb:3:121: C: Line is too long. [121/120] (Layout/LineLength)"
    );
    assert_eq!(lines.len(), 3, "{lines:#?}");
}

#[test]
fn line_length_max_comes_from_rustocop_toml() {
    let source = format!("{}\n{}\n", "a".repeat(100), "b".repeat(101));
    let dir = project(&[
        ("rustocop.toml", "[cops.\"Layout/LineLength\"]\nmax = 100\n"),
        ("app/x.rb", &source),
    ]);
    let output = rustocop(dir.path(), &["app"], "");
    let lines = stdout_lines(&output);
    assert_eq!(output.status.code(), Some(1), "{lines:#?}");
    assert_eq!(
        lines[0],
        "app/x.rb:2:101: C: Line is too long. [101/100] (Layout/LineLength)"
    );
    assert_eq!(lines.len(), 3, "{lines:#?}");

    // `--only` runs the configured cop too, stdin included.
    let output = rustocop(
        dir.path(),
        &["--only", "Layout", "--stdin", "app/y.rb"],
        &source,
    );
    assert_eq!(
        stdout_lines(&output)[0],
        "app/y.rb:2:101: C: Line is too long. [101/100] (Layout/LineLength)"
    );
}

#[test]
fn invalid_line_length_max_is_a_config_error() {
    for (config, message) in [
        (
            "[cops.\"Layout/LineLength\"]\nmax = 0\n",
            "`max` for `Layout/LineLength` must be at least 1",
        ),
        (
            "[cops.\"Lint/Debugger\"]\nmax = 100\n",
            "`max` is not a valid setting for `Lint/Debugger`",
        ),
        (
            "[cops.\"Layout\"]\nmax = 100\n",
            "`max` is not a valid setting for `Layout`",
        ),
    ] {
        let dir = project(&[("rustocop.toml", config), ("x.rb", "x = 1\n")]);
        let output = rustocop(dir.path(), &[], "");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains(message), "{stderr}");
    }
}

#[test]
fn unknown_config_cop_is_an_error_with_a_suggestion() {
    let dir = project(&[(
        "rustocop.toml",
        "[cops.\"Lint/Debbuger\"]\nenabled = false\n",
    )]);
    let output = rustocop(dir.path(), &[], "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        stderr.contains("unknown cop or department `Lint/Debbuger`"),
        "{stderr}"
    );
    assert!(stderr.contains("rustocop.toml"), "{stderr}");
    assert!(stderr.contains("did you mean `Lint/Debugger`?"), "{stderr}");
}

#[test]
fn unmatched_cli_selectors_are_errors_even_in_an_empty_project() {
    let dir = project(&[]);
    for flag in ["--only", "--except"] {
        for name in [
            "Lint/Debbuger",
            "Lnit",
            "Style/NotImplemented",
            "Debugger",
            "Lint/*",
            "all",
            "Zzzzzzzzzzzzzzzz",
        ] {
            for selection in [name.to_string(), format!("Lint/Debugger,{name}")] {
                let output = rustocop(dir.path(), &[flag, &selection], "");
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert_eq!(
                    output.status.code(),
                    Some(2),
                    "{flag} {selection}: {stderr}"
                );
                assert!(output.stdout.is_empty(), "{output:?}");
                assert!(
                    stderr.contains(&format!("unknown cop or department `{name}` in {flag}")),
                    "{stderr}"
                );
                if name == "Lint/Debbuger" {
                    assert!(stderr.contains("did you mean `Lint/Debugger`?"), "{stderr}");
                }
            }
        }
    }
}

#[test]
fn cli_selectors_are_validated_before_discovery_and_for_stdin() {
    let dir = project(&[]);
    for args in [
        vec!["--only", "Lint/Debbuger", "missing.rb"],
        vec!["--except", "Lint/Debbuger", "--stdin", "x.rb"],
        vec!["--only", "Lint", "--except", "Lint/Debbuger"],
        vec!["--only", "Lint", "--only", "Lint/Debbuger"],
        vec!["--except", "Lint", "--except", "Lint/Debbuger"],
    ] {
        let output = rustocop(dir.path(), &args, "");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(
            stderr.contains("unknown cop or department `Lint/Debbuger`"),
            "{stderr}"
        );
        assert!(!stderr.contains("missing.rb"), "{stderr}");
    }
}

#[test]
fn valid_selectors_force_enable_and_except_still_wins() {
    let dir = project(&[
        ("rustocop.toml", "[cops.\"Lint\"]\nenabled = false\n"),
        ("x.rb", "binding.pry\n"),
    ]);
    for selection in ["Lint", "Lint/Debugger", "Lint,Security/Open"] {
        let output = rustocop(dir.path(), &["--only", selection], "");
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("Lint/Debugger"));

        for exception in ["Lint", "Lint/Debugger"] {
            let output = rustocop(
                dir.path(),
                &["--only", selection, "--except", exception],
                "",
            );
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
        }
    }

    // Off-by-default cops and selectors excluded by --only are still known.
    let output = rustocop(
        dir.path(),
        &[
            "--only",
            "Style/FrozenStringLiteralComment",
            "--except",
            "Lint",
        ],
        "",
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("Style/FrozenStringLiteralComment"));
}

#[test]
fn directory_walk_skips_license_and_other_extensionless_files() {
    let license = "MIT License\n\nCopyright (c) 2026 Someone\n\n\
                   Permission is hereby granted, free of charge, to any person obtaining a copy\n";
    let dir = project(&[
        ("gem/LICENSE", license),
        ("gem/bin/console", "#!/usr/bin/env ruby\ndef oops(a,\n"),
        ("gem/lib/gem.rb", "# Gem entry point.\nmodule Gem\nend\n"),
    ]);
    let output = rustocop(dir.path(), &["gem"], "");
    let lines = stdout_lines(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{lines:#?}\n{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(
        lines.last().map(String::as_str),
        Some("1 file inspected, no offenses detected"),
        "{lines:#?}"
    );
}

#[test]
fn explicitly_named_non_rb_files_are_still_linted() {
    let dir = project(&[
        ("Rakefile", "binding.pry\n"),
        ("lib/tasks/build.rake", "binding.pry\n"),
    ]);
    let output = rustocop(
        dir.path(),
        &[
            "--only",
            "Lint/Debugger",
            "Rakefile",
            "lib/tasks/build.rake",
        ],
        "",
    );
    let lines = stdout_lines(&output);
    assert_eq!(output.status.code(), Some(1), "{lines:#?}");
    assert!(
        lines.iter().any(|l| l.starts_with("Rakefile:1:1: ")),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("lib/tasks/build.rake:1:1: ")),
        "{lines:#?}"
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("2 files inspected, 2 offenses detected"),
        "{lines:#?}"
    );
}
