//! Fixed-policy contract through the real CLI, using only implemented cops.
use crate::support::stdin;

use std::{
    fs,
    process::{Command, Output, Stdio},
};

fn cli(root: &std::path::Path, cwd: &str, args: &[&str], stdin: Option<&str>) -> Output {
    run_cli(root, cwd, args, stdin, false)
}

fn rejecting_stdin(root: &std::path::Path, args: &[&str], source: &str) -> Output {
    let output = run_cli(root, "", args, Some(source), true);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    // Usage, selector and config rejections all print one `error:` diagnostic.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("error: "), "{output:?}");
    if args.contains(&"--fix") {
        assert!(
            stderr.contains("cannot be used with") && stderr.contains("'--fix'"),
            "{output:?}"
        );
    }
    output
}

fn run_cli(
    root: &std::path::Path,
    cwd: &str,
    args: &[&str],
    stdin: Option<&str>,
    expect_early_error: bool,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    command.current_dir(root.join(cwd)).args(args);
    if let Some(source) = stdin {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        stdin::write_stdin(
            child.stdin.take().unwrap(),
            source.as_bytes(),
            expect_early_error,
        )
        .unwrap();
        child.wait_with_output().unwrap()
    } else {
        command.output().unwrap()
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

#[test]
fn existing_opt_in_is_default_off_and_only_or_per_cop_enabled() {
    let dir = project();
    // Keep the broad --only Style check clean for opt-in Copyright, while
    // still omitting the frozen-string header under test.
    fs::write(
        dir.path().join("lib/a.rb"),
        "# Copyright 2026 Test\nputs 1\n",
    )
    .unwrap();
    let run = cli(dir.path(), "", &["lib/a.rb"], None);
    assert_eq!(run.status.code(), Some(0));
    for config in [
        "",
        "[cops.Style]\nenabled = false\n[cops.\"Style/FrozenStringLiteralComment\"]\nenabled = true\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let args = if config.is_empty() {
            vec!["--only", "Style/FrozenStringLiteralComment", "lib/a.rb"]
        } else {
            vec!["lib/a.rb"]
        };
        let run = cli(dir.path(), "", &args, None);
        assert_eq!(run.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&run.stdout).contains("lib/a.rb:1:1"));
        assert!(String::from_utf8_lossy(&run.stdout).contains("Style/FrozenStringLiteralComment"));
    }
    let run = cli(
        dir.path(),
        "",
        &[
            "--only",
            "Style",
            "--except",
            "Style/FrozenStringLiteralComment",
            "lib/a.rb",
        ],
        None,
    );
    assert_eq!(run.status.code(), Some(0));
}

#[test]
fn method_call_parentheses_is_opt_in_and_except_wins() {
    let dir = project();
    let cop = "Style/MethodCallWithArgsParentheses";
    let original = "object.test 1\n";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, original).unwrap();
    let run = cli(dir.path(), "", &["lib/a.rb"], None);
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    for selector in [cop, "Style"] {
        let run = cli(dir.path(), "", &["--only", selector, "lib/a.rb"], None);
        assert_eq!(run.status.code(), Some(1), "{run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert!(output.contains("lib/a.rb:1:1"), "{output}");
        assert!(output.contains(cop), "{output}");
        let run = cli(
            dir.path(),
            "",
            &["--only", selector, "--except", cop, "lib/a.rb"],
            None,
        );
        assert!(
            !String::from_utf8_lossy(&run.stdout).contains(cop),
            "{run:?}"
        );
        assert_ne!(run.status.code(), Some(2), "{run:?}");
    }
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
fn method_call_parentheses_config_enables_and_selectors_override() {
    let dir = project();
    let cop = "Style/MethodCallWithArgsParentheses";
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
    )
    .unwrap();
    let source = "def f\n  test 1\nend\n";
    let run = cli(dir.path(), "", &["--stdin", "lib/a.rb"], Some(source));
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert!(String::from_utf8_lossy(&run.stdout).contains("lib/a.rb:2:3"));
    let run = cli(
        dir.path(),
        "",
        &["--except", cop, "--stdin", "lib/a.rb"],
        Some(source),
    );
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{cop}\"]\nenabled = false\n"),
    )
    .unwrap();
    let run = cli(
        dir.path(),
        "",
        &["--only", cop, "--stdin", "lib/a.rb"],
        Some(source),
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
}

#[test]
fn method_call_parentheses_excludes_apply_before_detection_and_fixing() {
    let dir = project();
    let cop = "Style/MethodCallWithArgsParentheses";
    let source = "object.test 1\n";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_string(),
        format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for args in [
            vec!["--only", cop, "a.rb"],
            vec!["--only", cop, "--fix", "a.rb"],
        ] {
            let run = cli(dir.path(), "lib", &args, None);
            assert_eq!(run.status.code(), Some(0), "{run:?}");
        }
        let run = cli(
            dir.path(),
            "lib",
            &["--only", cop, "--stdin", "a.rb"],
            Some(source),
        );
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn method_call_parentheses_only_writes_reviewed_safe_corrections() {
    let dir = project();
    let cop = "Style/MethodCallWithArgsParentheses";
    let path = dir.path().join("lib/a.rb");
    for (source, status) in [
        ("object.test 1\n", 1),
        ("object.test(1)\n", 0),
        (
            "object.test 1 # rustocop:disable Style/MethodCallWithArgsParentheses\n",
            0,
        ),
        (
            "# rubocop:disable Style/MethodCallWithArgsParentheses\nobject.test 1\n",
            0,
        ),
        ("object.test(\n", 2),
        ("yield\n", 2),
    ] {
        for fix in [false, true] {
            fs::write(&path, source).unwrap();
            let mut args = vec!["--only", cop];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            let run = cli(dir.path(), "", &args, None);
            let corrected = fix && status == 1;
            assert_eq!(
                run.status.code(),
                Some(if corrected { 0 } else { status }),
                "{source}: {run:?}"
            );
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                if corrected {
                    "object.test(1)\n"
                } else {
                    source
                }
            );
        }
    }
    let registry = rustocop::cops::Registry::new();
    let rule = registry
        .cops()
        .iter()
        .find(|rule| rule.name() == cop)
        .unwrap();
    assert!(!rule.default_enabled());
    assert!(rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(rule.path_policy().is_none());
}

#[test]
fn pinned_exclude_applies_to_explicit_file_and_stdin_not_just_directory_walks() {
    let dir = project();
    fs::write(dir.path().join("lib/a.arb"), "puts 1\n").unwrap();
    let cop = "Style/FrozenStringLiteralComment";
    for (cwd, path) in [("", "lib/a.arb"), ("lib", "a.arb")] {
        for stdin in [None, Some("puts 1\n")] {
            let args = if stdin.is_some() {
                vec!["--only", cop, "--stdin", path]
            } else {
                vec!["--only", cop, path]
            };
            let run = cli(dir.path(), cwd, &args, stdin);
            assert_eq!(run.status.code(), Some(0), "{run:?}");
        }
    }
    let absolute = dir.path().join("lib/a.arb");
    let run = cli(
        dir.path(),
        "",
        &["--only", cop, "--stdin", absolute.to_str().unwrap()],
        Some("puts 1\n"),
    );
    assert_eq!(run.status.code(), Some(0), "{run:?}");
}

#[test]
fn exclusions_are_config_relative_from_nested_cwd_for_files_and_stdin() {
    let dir = project();
    fs::write(dir.path().join("lib/a.rb"), "puts 1\n").unwrap();
    for config in [
        "exclude = [\"lib/a.rb\"]\n",
        "[cops.\"Style/FrozenStringLiteralComment\"]\nexclude = [\"lib/a.rb\"]\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let absolute = dir.path().join("lib/a.rb");
        for path in ["a.rb", "./a.rb", "../lib/a.rb", absolute.to_str().unwrap()] {
            for stdin in [None, Some("puts 1\n")] {
                let args = if stdin.is_some() {
                    vec![
                        "--only",
                        "Style/FrozenStringLiteralComment",
                        "--stdin",
                        path,
                    ]
                } else {
                    vec!["--only", "Style/FrozenStringLiteralComment", path]
                };
                let run = cli(dir.path(), "lib", &args, stdin);
                assert_eq!(
                    run.status.code(),
                    Some(0),
                    "config={config}, path={path}: {run:?}"
                );
            }
        }
    }
}

#[test]
fn nested_path_exclusions_prevent_corrections_as_well_as_reports() {
    let dir = project();
    let original = "puts 'x'\n";
    for config in [
        "exclude = [\"lib/a.rb\"]\n",
        "[cops.\"Style/StringLiterals\"]\nexclude = [\"lib/a.rb\"]\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for path in ["a.rb", "./a.rb", "../lib/a.rb"] {
            fs::write(dir.path().join("lib/a.rb"), original).unwrap();
            let run = cli(
                dir.path(),
                "lib",
                &["--only", "Style/StringLiterals", "--fix", path],
                None,
            );
            assert_eq!(run.status.code(), Some(0), "{config}: {run:?}");
            assert_eq!(
                fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
                original
            );
        }
    }
}

#[test]
fn endless_method_is_default_on_and_selectors_override_config() {
    let dir = project();
    let cop = "Style/EndlessMethod";
    let source = "def foo = [\n  1\n]\n";
    for (config, selectors, expected) in [
        (String::new(), vec![], true),
        (String::new(), vec!["--only", cop], true),
        (
            String::new(),
            vec!["--only", "Style", "--except", cop],
            false,
        ),
        (
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            vec![],
            false,
        ),
        (
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            vec!["--only", cop],
            true,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
            vec![],
            true,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.extend(["--stdin", "lib/a.rb"]);
        let run = cli(dir.path(), "", &args, Some(source));
        assert_ne!(run.status.code(), Some(2), "{run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(output.contains(cop), expected, "{run:?}");
        if expected {
            assert!(output.contains("lib/a.rb:1:1"), "{output}");
            assert!(output.contains("Avoid endless method definitions with multiple lines."));
        }
    }
}

#[test]
fn endless_method_exclusions_are_config_relative_and_apply_with_fix() {
    let dir = project();
    let cop = "Style/EndlessMethod";
    let source = "def foo = [\n  1]\n";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_string(),
        format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for args in [
            vec!["--only", cop, "a.rb"],
            vec!["--only", cop, "--fix", "a.rb"],
        ] {
            let run = cli(dir.path(), "lib", &args, None);
            assert_eq!(run.status.code(), Some(0), "{run:?}");
        }
        let run = cli(
            dir.path(),
            "lib",
            &["--only", cop, "--stdin", "a.rb"],
            Some(source),
        );
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn endless_method_report_only_honors_directive_spans() {
    let dir = project();
    let cop = "Style/EndlessMethod";
    let path = dir.path().join("lib/a.rb");
    let mut cases = vec![
        ("def foo = [\n  1]\n".to_string(), 1),
        ("def foo = 1\n".to_string(), 0),
        ("def foo = [\n  1\n".to_string(), 2),
        ("def foo=(value) = [\n  value]\n".to_string(), 2),
        ("def []=(index, value) = [\n  value]\n".to_string(), 2),
        // Report-only mode preserves literal values, line observations, CRLF and data.
        (
            "def foo = \"first\nsecond\"\np __LINE__\n__END__\nraw\n".to_string(),
            1,
        ),
        ("def foo = [\r\n  __LINE__]\r\n".to_string(), 1),
    ];
    for prefix in ["rustocop", "rubocop"] {
        cases.extend([
            (format!("# {prefix}:disable {cop}\ndef foo = [\n  1]\n"), 0),
            (format!("# {prefix}:disable-next {cop}\ndef foo = [\n  1]\n"), 0),
            (format!("def foo = [\n  1] # {prefix}:disable {cop}\n"), 0),
            (format!("# {prefix}:disable {cop}\ndef foo = [\n  1]\n# {prefix}:enable {cop}\ndef bar = [\n  2]\n"), 1),
        ]);
    }
    for (source, status) in cases {
        fs::write(&path, &source).unwrap();
        let run = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
        assert_eq!(run.status.code(), Some(status), "{source}: {run:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn arguments_forwarding_defaults_selectors_and_enabling_use_the_real_cli() {
    let dir = project();
    let cop = "Style/ArgumentsForwarding";
    let source = "def foo(*args)\n  bar(*args)\nend\n";
    for (config, selectors, expected) in [
        (String::new(), vec![], true),
        (String::new(), vec!["--only", cop], true),
        (
            String::new(),
            vec!["--only", "Style", "--except", cop],
            false,
        ),
        (
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            vec![],
            false,
        ),
        (
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            vec!["--only", cop],
            true,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
            vec![],
            true,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.extend(["--stdin", "lib/a.rb"]);
        let run = cli(dir.path(), "", &args, Some(source));
        assert_ne!(run.status.code(), Some(2), "{run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(output.contains(cop), expected, "{run:?}");
        if expected {
            assert!(output.contains("lib/a.rb:1:9"), "{output}");
            assert!(output.contains("lib/a.rb:2:7"), "{output}");
            assert!(output.contains("Use anonymous positional arguments forwarding (`*`)."));
        }
    }
}

#[test]
fn arguments_forwarding_respects_config_relative_exclusions_with_or_without_fix() {
    let dir = project();
    let cop = "Style/ArgumentsForwarding";
    let source = "def foo(*args); bar(*args); end\n";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for args in [
            vec!["--only", cop, "a.rb"],
            vec!["--only", cop, "--fix", "a.rb"],
        ] {
            let run = cli(dir.path(), "lib", &args, None);
            assert_eq!(run.status.code(), Some(0), "{run:?}");
        }
        let run = cli(
            dir.path(),
            "lib",
            &["--only", cop, "--stdin", "a.rb"],
            Some(source),
        );
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn arguments_forwarding_honors_directives_and_never_writes_without_fix() {
    let dir = project();
    let cop = "Style/ArgumentsForwarding";
    let path = dir.path().join("lib/a.rb");
    let bad = "def foo(*args); bar(*args); end\n";
    let mut cases = vec![
        (bad.to_owned(), 1),
        ("def foo(*items); bar(*items); end\n".to_owned(), 0),
        ("def foo(*args)\n  bar(*args)\n".to_owned(), 2),
        ("# encoding: UTF-8\r\ndef foo(*args); bar(*args); end\r\np __LINE__\r\n__END__\r\nraw data\r\n".to_owned(), 1),
    ];
    for prefix in ["rustocop", "rubocop"] {
        cases.extend([
            (format!("# {prefix}:disable {cop}\n{bad}"), 0),
            (format!("# {prefix}:disable-next {cop}\n{bad}"), 0),
            (format!("{} # {prefix}:disable {cop}\n", bad.trim_end()), 0),
            (format!("# {prefix}:disable Style\n{bad}"), 0),
            (
                format!("# {prefix}:disable {cop}\n{bad}# {prefix}:enable {cop}\n{bad}"),
                1,
            ),
        ]);
    }
    for (source, status) in cases {
        fs::write(&path, &source).unwrap();
        let mut args = vec!["--only", cop, "lib/a.rb"];
        let run = cli(dir.path(), "", &args, None);
        assert_eq!(run.status.code(), Some(status), "{source}: {run:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        if status != 1 {
            args.push("--fix");
            let run = cli(dir.path(), "", &args, None);
            assert_eq!(run.status.code(), Some(status), "{source}: {run:?}");
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
    }
    let bytes = fs::read(&path).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let run = rejecting_stdin(
        dir.path(),
        &["--only", cop, "--fix", "--stdin", "lib/a.rb"],
        bad,
    );
    assert!(
        String::from_utf8_lossy(&run.stderr)
            .contains("the argument '--fix' cannot be used with '--stdin <PATH>'"),
        "{run:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

#[test]
fn block_delimiters_defaults_selectors_and_config_use_the_real_cli() {
    let dir = project();
    let cop = "Style/BlockDelimiters";
    let source = "items.each do |item| item end\n";
    for (config, selectors, expected) in [
        (String::new(), vec![], true),
        (String::new(), vec!["--only", cop], true),
        (
            String::new(),
            vec!["--only", "Style", "--except", cop],
            false,
        ),
        (
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            vec![],
            false,
        ),
        (
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            vec!["--only", cop],
            true,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
            vec![],
            true,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.extend(["--stdin", "lib/a.rb"]);
        let run = cli(dir.path(), "", &args, Some(source));
        assert_ne!(run.status.code(), Some(2), "{run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(output.contains(cop), expected, "{run:?}");
        if expected {
            assert!(output.contains("lib/a.rb:1:12"), "{output}");
            assert!(output.contains("Prefer `{...}` over `do...end` for single-line blocks."));
        }
    }
    let registry = rustocop::cops::Registry::new();
    let rule = registry
        .cops()
        .iter()
        .find(|rule| rule.name() == cop)
        .unwrap();
    assert!(rule.default_enabled());
    assert!(rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(rule.path_policy().is_none());
}

#[test]
fn block_delimiters_exclusions_apply_from_nested_cwd_with_or_without_fix() {
    let dir = project();
    let cop = "Style/BlockDelimiters";
    let source = "items.each do |x| x end\n";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for args in [
            vec!["--only", cop, "a.rb"],
            vec!["--only", cop, "--fix", "a.rb"],
        ] {
            let run = cli(dir.path(), "lib", &args, None);
            assert_eq!(run.status.code(), Some(0), "{run:?}");
        }
        let run = cli(
            dir.path(),
            "lib",
            &["--only", cop, "--stdin", "a.rb"],
            Some(source),
        );
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn block_delimiters_directives_and_detection_mode_preserve_bytes() {
    let dir = project();
    let cop = "Style/BlockDelimiters";
    let path = dir.path().join("lib/a.rb");
    let bad = "each { |x|\n x\n}\n";
    let mut cases = vec![
        (bad.to_owned(), 1),
        ("each { |x| x }\n".to_owned(), 0),
        ("each do\n".to_owned(), 2),
        (
            "# encoding: UTF-8\r\neach do |x| next x end\r\np __LINE__\r\n__END__\r\nraw\r\n"
                .to_owned(),
            1,
        ),
        ("map do puts <<~MSG end\n data\nMSG\n".to_owned(), 1),
        // Detection alone must not apply even eligible peer corrections.
        ("object.map 1 do |x| return x end\n".to_owned(), 1),
    ];
    for prefix in ["rustocop", "rubocop"] {
        cases.extend([
            (format!("# {prefix}:disable {cop}\n{bad}"), 0),
            (format!("# {prefix}:disable-next {cop}\n{bad}"), 0),
            (format!("each {{ |x| # {prefix}:disable {cop}\n x\n}}\n"), 0),
            (format!("# {prefix}:disable Style\n{bad}"), 0),
            (
                format!("# {prefix}:disable {cop}\n{bad}# {prefix}:enable {cop}\n{bad}"),
                1,
            ),
            // Highlights cover only the opener, not the block body/closing line.
            (format!("each {{ |x|\n x\n}} # {prefix}:disable {cop}\n"), 1),
        ]);
    }
    for (source, status) in cases {
        fs::write(&path, &source).unwrap();
        let run = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
        assert_eq!(run.status.code(), Some(status), "{source}: {run:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

// all Eval fixtures are linted/parsed as source, never executed.
#[test]
fn security_eval_defaults_selectors_config_and_metadata() {
    let dir = project();
    let cop = "Security/Eval";
    let source = "binding.eval(code)\n";
    for (config, selectors, expected) in [
        (String::new(), vec![], true),
        (String::new(), vec!["--only", cop], true),
        (String::new(), vec!["--only", "Security"], true),
        (String::new(), vec!["--except", cop], false),
        (
            String::new(),
            vec!["--only", "Security", "--except", cop],
            false,
        ),
        (String::new(), vec!["--only", "Lint"], false),
        (
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            vec![],
            false,
        ),
        (
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            vec!["--only", cop],
            true,
        ),
        ("[cops.Security]\nenabled = false\n".into(), vec![], false),
        (
            format!("[cops.Security]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
            vec![],
            true,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.extend(["--stdin", "lib/a.rb"]);
        let run = cli(dir.path(), "", &args, Some(source));
        assert_ne!(run.status.code(), Some(2), "{run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(output.contains(cop), expected, "{run:?}");
        if expected {
            assert!(
                output.contains(
                    "lib/a.rb:1:9: E: The use of `eval` is a serious security risk. (Security/Eval)"
                ),
                "{output}"
            );
        }
    }
    let registry = rustocop::cops::Registry::new();
    let rule = registry
        .cops()
        .iter()
        .find(|rule| rule.name() == cop)
        .unwrap();
    assert!(rule.default_enabled());
    assert!(!rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(rule.path_policy().is_none());
}

#[test]
fn security_eval_fixtures_are_valid_host_and_native_ruby_source_without_execution() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    // Undefined inputs deliberately cannot be run. Ruby is invoked ONLY with
    // -c; neither interpolation nor any eval input is ever executed.
    let source = "eval(code)\nbinding.eval(\"#{code}\")\nKernel.eval(code, binding, __FILE__, __LINE__)\n::Kernel.eval(code)\neval(<<~CODE)\n  #{code}\nCODE\n";
    fs::write(&path, source).unwrap();
    for ruby in
        std::iter::once(std::ffi::OsString::from("ruby")).chain(crate::support::runtime::ruby_os())
    {
        let output = match Command::new(&ruby).arg("-c").arg(&path).output() {
            Ok(output) => output,
            Err(error) if ruby == "ruby" && error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("SKIP host Ruby syntax-only witness: ruby not found");
                continue;
            }
            Err(error) => panic!("{ruby:?} must be executable: {error}"),
        };
        assert!(output.status.success(), "{ruby:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Syntax OK"),
            "{output:?}"
        );
    }
    let run = cli(
        dir.path(),
        "",
        &["--only", "Security/Eval", "lib/a.rb"],
        None,
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout)
            .matches("(Security/Eval)")
            .count(),
        5
    );
}

#[test]
fn security_eval_exclusions_cover_nested_paths_stdin_and_fix() {
    let dir = project();
    let cop = "Security/Eval";
    let path = dir.path().join("lib/a.rb");
    let source = "Kernel.eval(code)\n";
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for args in [
            vec!["--only", cop, "a.rb"],
            vec!["--only", cop, "--fix", "a.rb"],
        ] {
            let run = cli(dir.path(), "lib", &args, None);
            assert_eq!(run.status.code(), Some(0), "{run:?}");
        }
        let run = cli(
            dir.path(),
            "lib",
            &["--only", cop, "--stdin", "a.rb"],
            Some(source),
        );
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn security_eval_detection_only_preserves_findings_bytes_and_mtime() {
    let dir = project();
    let cop = "Security/Eval";
    let path = dir.path().join("lib/a.rb");
    for (source, count) in [
        ("eval(code)\n", 1),
        ("binding.eval(\"#{code}\")\n", 1),
        ("eval(\"literal\")\n", 0),
        ("eval(<<~CODE)\n  #{code}\nCODE\n", 1),
        ("\u{feff}eval(code)\r\n", 1),
        (
            "# encoding: UTF-8\r\nKernel.eval(code)\r\np __LINE__\r\n__END__\r\nraw data\r\n",
            1,
        ),
        (
            "eval(first)\nbinding.eval(second)\n::Kernel.eval(third)\n",
            3,
        ),
    ] {
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let lint = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
        assert_eq!(
            lint.status.code(),
            Some(i32::from(count != 0)),
            "{source}: {lint:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&lint.stdout)
                .matches("(Security/Eval)")
                .count(),
            count
        );
        for _ in 0..2 {
            let fixed = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
            assert_eq!(
                fixed.status.code(),
                lint.status.code(),
                "{source}: {fixed:?}"
            );
            assert!(fixed.stderr.is_empty(), "{fixed:?}");
            let output = String::from_utf8_lossy(&fixed.stdout);
            assert!(output.contains("0 corrections applied"), "{output}");
            assert_eq!(output.matches("(Security/Eval)").count(), count);
            for diagnostic in String::from_utf8_lossy(&lint.stdout)
                .lines()
                .filter(|line| line.contains("(Security/Eval)"))
            {
                assert!(output.lines().any(|line| line == diagnostic), "{output}");
            }
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
    let bytes = fs::read(&path).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    rejecting_stdin(
        dir.path(),
        &["--only", cop, "--fix", "--stdin", "lib/a.rb"],
        "eval(code)\n",
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

#[test]
fn security_eval_directives_use_selector_only_ranges_and_cannot_bypass_except() {
    let dir = project();
    let cop = "Security/Eval";
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rustocop", "rubocop"] {
        for (source, count) in [
            (format!("eval(code) # {prefix}:disable {cop}\n"), 0),
            (format!("# {prefix}:disable Security\neval(code)\n"), 0),
            (
                format!("# {prefix}:disable-next {cop}\neval(code)\neval(other)\n"),
                1,
            ),
            (
                format!(
                    "# {prefix}:disable {cop}\neval(code)\n# {prefix}:enable {cop}\neval(other)\n"
                ),
                1,
            ),
            (format!("eval(\n  code # {prefix}:disable {cop}\n)\n"), 1),
        ] {
            fs::write(&path, &source).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            for fix in [false, true] {
                let mut args = vec!["--only", cop, "lib/a.rb"];
                if fix {
                    args.push("--fix");
                }
                let run = cli(dir.path(), "", &args, None);
                assert_eq!(
                    run.status.code(),
                    Some(i32::from(count != 0)),
                    "{source}: {run:?}"
                );
                assert_eq!(
                    String::from_utf8_lossy(&run.stdout)
                        .matches("(Security/Eval)")
                        .count(),
                    count
                );
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {cop}\neval(code)\n");
        let run = cli(dir.path(), "", &["--stdin", "lib/a.rb"], Some(&source));
        assert!(
            String::from_utf8_lossy(&run.stdout).contains(cop),
            "{run:?}"
        );
        let run = cli(
            dir.path(),
            "",
            &["--except", cop, "--stdin", "lib/a.rb"],
            Some(&source),
        );
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

fn ns02_cases() -> [(&'static str, &'static str, u32, &'static str); 3] {
    [
        (
            "Naming/AsciiIdentifiers",
            "fooé = 1\n",
            4,
            "Use only ascii symbols in identifiers.",
        ),
        (
            "Naming/BlockParameterName",
            "foo { |aB| }\n",
            8,
            "Only use lowercase characters for block parameter.",
        ),
        (
            "Naming/VariableNumber",
            "foo_1 = 1\n",
            1,
            "Use normalcase for variable numbers.",
        ),
    ]
}

fn ns03_cases() -> [(&'static str, &'static str, u32, &'static str); 2] {
    [
        (
            "Naming/AccessorMethodName",
            "def get_value; end\n",
            5,
            "Do not prefix reader method names with `get_`.",
        ),
        (
            "Naming/PredicatePrefix",
            "def does_work; end\n",
            5,
            "Rename `does_work` to `work?`.",
        ),
    ]
}

#[test]
fn ns04_all_pinned_corrections_run_through_cli_with_exact_bytes_and_no_write_holds() {
    // RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns04_pinned.json")).unwrap();
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for call in fixture["calls"].as_array().unwrap() {
        let cop = call["cop"].as_str().unwrap();
        let source = call["source"].as_str().unwrap();
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let lint = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
        let output = String::from_utf8_lossy(&lint.stdout);
        assert_eq!(
            output.matches(&format!("({cop})")).count(),
            call["positions"].as_array().unwrap().len(),
            "{call}: {lint:?}"
        );
        for (p, message) in call["positions"]
            .as_array()
            .unwrap()
            .iter()
            .zip(call["messages"].as_array().unwrap())
        {
            assert!(
                output.contains(&format!(
                    "lib/a.rb:{}:{}: C: {} ({cop})",
                    p[0],
                    p[1],
                    message.as_str().unwrap()
                )),
                "{output}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        let hold = cop.ends_with("RescuedExceptionsVariableName")
            && [372, 418, 441].contains(&call["spec_line"].as_u64().unwrap());
        let expected = if hold {
            source
        } else {
            call["correction"].as_str().unwrap_or(source)
        };
        let run = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
        assert_ne!(run.status.code(), Some(2), "{call}: {run:?}");
        assert!(run.stderr.is_empty(), "{call}: {run:?}");
        assert_eq!(fs::read(&path).unwrap(), expected.as_bytes(), "{call}");
        if source == expected {
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
    let docs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns04_source.json")).unwrap();
    for call in docs {
        let cop = call["cop"].as_str().unwrap();
        let source = call["source"].as_str().unwrap();
        fs::write(&path, source).unwrap();
        let run = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
        assert_ne!(run.status.code(), Some(2), "{call}: {run:?}");
        assert_eq!(
            String::from_utf8_lossy(&run.stdout)
                .matches(&format!("({cop})"))
                .count(),
            call["positions"].as_array().unwrap().len()
        );
    }
}

#[test]
fn ns04_directives_protect_the_entire_atomic_binding_and_stdin_never_writes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, statement, _, _) in ns04_cases() {
        for prefix in ["rustocop", "rubocop"] {
            for source in [
                format!("# {prefix}:disable Naming\n{statement}"),
                format!("{} # {prefix}:disable {cop}\n", statement.trim_end()),
                if cop.ends_with("BinaryOperatorParameterName") {
                    format!("def +(arg)\n arg # {prefix}:disable {cop}\nend\n")
                } else {
                    format!("begin; rescue => error\n error # {prefix}:disable {cop}\nend\n")
                },
            ] {
                fs::write(&path, &source).unwrap();
                let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                let run = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
                assert_ne!(run.status.code(), Some(2), "{source}: {run:?}");
                assert!(String::from_utf8_lossy(&run.stdout).contains("0 corrections applied"));
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled=false\n"),
            )
            .unwrap();
            let source = format!("# {prefix}:enable {cop}\n{statement}");
            fs::write(&path, &source).unwrap();
            let run = cli(
                dir.path(),
                "",
                &["--fix", "--only", "Naming", "--except", cop, "lib/a.rb"],
                None,
            );
            assert_ne!(run.status.code(), Some(2), "{run:?}");
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            let run = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
            assert_eq!(run.status.code(), Some(0), "{run:?}");
            assert_ne!(fs::read(&path).unwrap(), source.as_bytes());
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        }
        let bytes = fs::read(&path).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        rejecting_stdin(
            dir.path(),
            &["--only", cop, "--fix", "--stdin", "lib/a.rb"],
            statement,
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn ns04_legacy_encodings_preserve_identifier_bindings_and_withhold_invalid_interior_bytes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (encoding, letter) in [
        ("UTF-8", "é".as_bytes()),
        ("ISO-8859-1", b"\xe9".as_slice()),
        ("Windows-1252", b"\xe9".as_slice()),
        ("Shift_JIS", b"\x83\x5f".as_slice()),
        ("EUC-JP", b"\xa3\xc1".as_slice()),
    ] {
        for (cop, head, name, tail, preferred) in [
            (
                "Naming/BinaryOperatorParameterName",
                "def +(",
                "arg",
                "); ",
                "other",
            ),
            (
                "Naming/RescuedExceptionsVariableName",
                "begin; rescue => ",
                "error",
                "; ",
                "e",
            ),
        ] {
            let mut identifier = name.as_bytes().to_vec();
            identifier.extend(letter);
            let header = format!("# encoding: {encoding}\r\n");
            let mut source = header.as_bytes().to_vec();
            source.extend(head.as_bytes());
            source.extend(&identifier);
            source.extend(tail.as_bytes());
            source.extend(&identifier);
            source.extend(b"; end\r\n__END__\r\nraw\r\n");
            fs::write(&path, &source).unwrap();
            for ruby in ns04_runtimes() {
                let run = match Command::new(&ruby).arg("-c").arg(&path).output() {
                    Ok(run) => run,
                    Err(e) if ruby == "ruby" && e.kind() == std::io::ErrorKind::NotFound => {
                        continue;
                    }
                    Err(e) => panic!("{ruby:?}: {e}"),
                };
                assert!(run.status.success(), "{encoding}: {ruby:?}: {run:?}");
            }
            let run = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
            assert_eq!(run.status.code(), Some(0), "{encoding}: {run:?}");
            let expected =
                format!("{header}{head}{preferred}{tail}{preferred}; end\r\n__END__\r\nraw\r\n");
            assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
            // Untouched legacy literal bytes between coordinated edits cannot
            // be represented by Correction's UTF-8 string; no transcoding.
            if encoding != "UTF-8" {
                let mut source = header.as_bytes().to_vec();
                source.extend(head.as_bytes());
                source.extend(&identifier);
                source.extend(tail.as_bytes());
                source.extend(b"\"");
                source.extend(letter);
                source.extend(b"\"; ");
                source.extend(&identifier);
                source.extend(b"; end\r\n");
                fs::write(&path, &source).unwrap();
                let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                let run = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
                assert_eq!(run.status.code(), Some(1), "{encoding}: {run:?}");
                assert_eq!(fs::read(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
        }
    }
}

fn ns04_runtimes() -> Vec<std::ffi::OsString> {
    std::iter::once(std::ffi::OsString::from("ruby"))
        .chain(crate::support::runtime::ruby_os())
        .collect()
}
fn ns04_ruby(ruby: &std::ffi::OsStr, path: &std::path::Path) -> Option<std::process::Output> {
    match Command::new(ruby).arg(path).output() {
        Ok(out) => Some(out),
        Err(e) if ruby == "ruby" && e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("{ruby:?}: {e}"),
    }
}

#[test]
fn ns04_native_runtime_values_captures_defaults_shadowing_ensure_retry_and_control_flow() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "class Example; def +(arg); result=arg; [2,3].each { |arg| result+=arg }; result; end; end; p Example.new + 4\n",
        "class Example; def +(arg); f=->(value=arg) { arg += value }; [f.call, arg]; end; end; p Example.new + 4\n",
        "class Example; def +(arg); f=-> { arg }; [1].each { |;arg| arg=8 }; arg=5; f.call; end; end; p Example.new + 4\n",
        "class Parent; def +(value); value+1; end; end; class Example < Parent; def +(arg); arg+=2; super; end; end; p Example.new + 4\n",
        "class Example; def +(arg); {arg:}; end; end; p Example.new + 4\n",
        "class Example; def +(arg); <<~TXT\n#{arg} #{__LINE__} arg literal\nTXT\nend; end; p Example.new + 4\n",
        // An unread data section survives; reading `DATA` is a
        // listed observer that withholds every edit.
        "class Example; def +(arg); [arg, __LINE__]; end; end; p Example.new + 4; p :data\n__END__\nraw arg data\n",
        "begin; raise 'boom'; rescue => error; p [error.message, {error:}.keys]; ensure; p error.message; end; p error.message\n",
        "begin; raise 'boom'; rescue => error; f=->(v=error) { v.message }; p f.call; end; p f.call\n",
        "n=0; begin; n+=1; raise 'boom' if n<2; rescue => error; retry if n<2; ensure; p [n, error&.message]; end; p error.message\n",
        "def test; raise 'boom'; rescue => error; return error.message; ensure; p error.message; end; p test\n",
        "def test; [1,2].each { begin; raise 'boom'; rescue => error; next error.message; ensure; p error.message; end }; end; p test\n",
        "begin; raise ArgumentError, 'a'; rescue ArgumentError => first; p first.message; rescue RuntimeError => second; p second.message; end\n",
    ] {
        fs::write(&path, source).unwrap();
        let before: Vec<_> = ns04_runtimes()
            .into_iter()
            .filter_map(|r| ns04_ruby(&r, &path).map(|out| (r, out)))
            .collect();
        let run = cli(
            dir.path(),
            "",
            &[
                "--only",
                "Naming/BinaryOperatorParameterName,Naming/RescuedExceptionsVariableName",
                "--fix",
                "lib/a.rb",
            ],
            None,
        );
        assert_eq!(run.status.code(), Some(0), "{source}: {run:?}");
        assert_ne!(fs::read(&path).unwrap(), source.as_bytes());
        for (ruby, a) in before {
            let b = ns04_ruby(&ruby, &path).unwrap();
            assert!(
                a.status.success() && b.status.success(),
                "{ruby:?}: {source}: {a:?} {b:?}"
            );
            assert_eq!(a.stdout, b.stdout, "{ruby:?}: {source}");
            assert_eq!(a.stderr, b.stderr, "{ruby:?}: {source}");
        }
    }
}

#[test]
fn ns04_existing_canonical_fixes_and_style_layout_composition_remain_single_pass() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def +(arg); arg; end\nbegin; rescue => error; fail; error; end\n";
    fs::write(&path, source).unwrap();
    let run = cli(
        dir.path(),
        "",
        &[
            "--only",
            "Naming/BinaryOperatorParameterName,Naming/RescuedExceptionsVariableName,Style/SignalException",
            "--fix",
            "--fix-passes",
            "1",
            "lib/a.rb",
        ],
        None,
    );
    // Atomic exception rename includes the fail token; overlapping SignalException
    // is deferred, not lost or applied against stale byte offsets.
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def +(other); other; end\nbegin; rescue => e; fail; e; end\n"
    );
    let run = cli(
        dir.path(),
        "",
        &[
            "--only",
            "Naming/BinaryOperatorParameterName,Naming/RescuedExceptionsVariableName,Style/SignalException,Layout",
            "--fix",
            "--fix-passes",
            "1",
            "lib/a.rb",
        ],
        None,
    );
    assert_ne!(run.status.code(), Some(2), "{run:?}");
    assert!(fs::read_to_string(&path).unwrap().contains("raise"));
}

const FILE_NAME: &str = "Naming/FileName";

#[test]
fn ns10_default_selectors_config_and_metadata_use_actual_stdin_filename() {
    let dir = project();
    for (config, selectors, expected) in [
        (String::new(), vec![], true),
        (String::new(), vec!["--only", FILE_NAME], true),
        (String::new(), vec!["--only", "Naming"], true),
        (String::new(), vec!["--except", FILE_NAME], false),
        (
            String::new(),
            vec!["--only", "Naming", "--except", FILE_NAME],
            false,
        ),
        (String::new(), vec!["--only", "Lint"], false),
        (
            format!("[cops.\"{FILE_NAME}\"]\nenabled = false\n"),
            vec![],
            false,
        ),
        (
            format!("[cops.\"{FILE_NAME}\"]\nenabled = false\n"),
            vec!["--only", FILE_NAME],
            true,
        ),
        ("[cops.Naming]\nenabled = false\n".into(), vec![], false),
        (
            format!("[cops.Naming]\nenabled = false\n[cops.\"{FILE_NAME}\"]\nenabled = true\n"),
            vec![],
            true,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.extend(["--stdin", "lib/badName.rb"]);
        let run = cli(dir.path(), "", &args, Some("print 1\n"));
        assert_ne!(run.status.code(), Some(2), "{run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(
            output.contains(&format!("({FILE_NAME})")),
            expected,
            "{output}"
        );
        if expected {
            assert!(output.contains("lib/badName.rb:1:1: C: The name of this source file (`badName.rb`) should use snake_case."), "{output}");
        }
    }
    let registry = rustocop::cops::Registry::new();
    let rule = registry
        .cops()
        .iter()
        .find(|r| r.name() == FILE_NAME)
        .unwrap();
    assert!(rule.default_enabled());
    assert!(!rule.safe_fixes());
    assert!(rule.path_policy().is_some());
    assert_eq!(rule.ruby_versions(), Default::default());
    // No new public configuration surface, including unused definition options.
    for (key, value) in [
        ("ExpectMatchingDefinition", "true"),
        ("IgnoreExecutableScripts", "false"),
        ("AllowedAcronyms", "[]"),
        ("Regex", "\".*\""),
        ("max", "2"),
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{FILE_NAME}\"]\n{key} = {value}\n"),
        )
        .unwrap();
        rejecting_stdin(
            dir.path(),
            &["--only", FILE_NAME, "--stdin", "badName.rb"],
            "print 1\n",
        );
    }
}

#[test]
fn ns10_excludes_and_pinned_root_rakefile_policy_are_config_relative() {
    let dir = project();
    let source = "print 1\n";
    let path = dir.path().join("lib/badName.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    for config in [
        "exclude = [\"lib/badName.rb\"]\n".to_owned(),
        format!("[cops.\"{FILE_NAME}\"]\nexclude = [\"lib/badName.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for args in [
            vec!["--only", FILE_NAME, "badName.rb"],
            vec!["--only", FILE_NAME, "--fix", "badName.rb"],
        ] {
            assert_eq!(cli(dir.path(), "lib", &args, None).status.code(), Some(0));
        }
        assert_eq!(
            cli(
                dir.path(),
                "lib",
                &["--only", FILE_NAME, "--stdin", "badName.rb"],
                Some(source)
            )
            .status
            .code(),
            Some(0)
        );
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for (cwd, file, flagged) in [
        ("", "Rakefile.rb", false),
        ("", "lib/Rakefile.rb", true),
        ("lib", "../Rakefile.rb", false),
        ("lib", "Rakefile.rb", true),
        ("", "Rakefile.rb.extra", true),
        ("", "lib/../Rakefile.rb", true),
    ] {
        let run = cli(
            dir.path(),
            cwd,
            &["--only", FILE_NAME, "--stdin", file],
            Some(source),
        );
        assert_eq!(
            run.status.code(),
            Some(i32::from(flagged)),
            "{file}: {run:?}"
        );
    }
    // Absolute inputs inside/outside the root keep the exact basename, never
    // infer a class or inspect sibling files. Stdin need not name a real file.
    for path in [
        dir.path().join("Rakefile.rb"),
        dir.path().join("lib/Rakefile.rb"),
        dir.path().join("../Rakefile.rb"),
    ] {
        let expected = path != dir.path().join("Rakefile.rb");
        let run = cli(
            dir.path(),
            "",
            &["--only", FILE_NAME, "--stdin", path.to_str().unwrap()],
            Some(source),
        );
        assert_eq!(
            run.status.code(),
            Some(i32::from(expected)),
            "{path:?}: {run:?}"
        );
    }
}

#[test]
fn ns10_global_directives_are_at_line_one_and_respect_selector_precedence() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for (source, flagged) in [
            (format!("# {prefix}:disable {FILE_NAME}\nprint 1\n"), false),
            (format!("# {prefix}:disable Naming\nprint 1\n"), false),
            (format!("# {prefix}:disable all\nprint 1\n"), false),
            (format!("print 1 # {prefix}:disable {FILE_NAME}\n"), false),
            (format!("# comment\n# {prefix}:disable {FILE_NAME}\n"), true),
            (
                format!("# {prefix}:disable-next {FILE_NAME}\nprint 1\n"),
                true,
            ),
            (
                format!("# {prefix}:disable {FILE_NAME}\n# {prefix}:enable {FILE_NAME}\nprint 1\n"),
                false,
            ),
        ] {
            fs::write(dir.path().join("lib/badName.rb"), &source).unwrap();
            let run = cli(
                dir.path(),
                "",
                &["--only", FILE_NAME, "--fix", "lib/badName.rb"],
                None,
            );
            assert_eq!(
                run.status.code(),
                Some(i32::from(flagged)),
                "{source}: {run:?}"
            );
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{FILE_NAME}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {FILE_NAME}\nprint 1\n");
        let run = cli(dir.path(), "", &["--stdin", "badName.rb"], Some(&source));
        // Inherited directive contract: the enable line is still disabled.
        // A global line-one finding cannot be enabled by a later source line.
        assert!(
            !String::from_utf8_lossy(&run.stdout).contains(FILE_NAME),
            "{run:?}"
        );
        let run = cli(
            dir.path(),
            "",
            &["--except", FILE_NAME, "--stdin", "badName.rb"],
            Some(&source),
        );
        assert!(
            !String::from_utf8_lossy(&run.stdout).contains(FILE_NAME),
            "{run:?}"
        );
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn ns10_every_pinned_and_source_case_is_detection_only_through_real_fix() {
    // MIT attribution and exact pin completeness live in ns10_test.rb and the cop.
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns10_pinned.json")).unwrap();
    assert_ns02_calls_preserve_bytes_and_mtime(&rows);
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns10_source.json")).unwrap();
    assert_ns02_calls_preserve_bytes_and_mtime(&rows);
}

#[test]
fn ns10_path_matrix_uses_actual_stdin_paths_and_never_accesses_or_creates_them() {
    let dir = project();
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns10_paths.json")).unwrap();
    rows.extend(
        serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
            "fixtures/ns10_hidden_paths.json"
        ))
        .unwrap(),
    );
    rows.extend(
        serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
            "fixtures/ns10_root_paths.json"
        ))
        .unwrap(),
    );
    assert_eq!(rows.len(), 127);
    for row in rows {
        let file = row["file"].as_str().unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", FILE_NAME, "--stdin", file],
            Some(row["source"].as_str().unwrap()),
        );
        let positions = row
            .get("rustocop_positions")
            .unwrap_or(&row["positions"])
            .as_array()
            .unwrap();
        let messages = row
            .get("rustocop_messages")
            .unwrap_or(&row["messages"])
            .as_array()
            .unwrap();
        assert_eq!(
            run.status.code(),
            Some(i32::from(!positions.is_empty())),
            "{row}: {run:?}"
        );
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(
            output.matches(&format!("({FILE_NAME})")).count(),
            positions.len(),
            "{row}: {output}"
        );
        for message in messages {
            assert!(
                output.contains(&format!(
                    ":1:1: C: {} ({FILE_NAME})",
                    message.as_str().unwrap()
                )),
                "{output}"
            );
        }
    }
    // Empty --stdin is rejected by the existing argument parser, before input.
    rejecting_stdin(
        dir.path(),
        &["--only", FILE_NAME, "--stdin", ""],
        "print 1\n",
    );
    for file in [".", "./", "lib/..", "lib/../", "lib/./.."] {
        let run = cli(
            dir.path(),
            "",
            &["--only", FILE_NAME, "--stdin", file],
            Some("print 1\n"),
        );
        assert_eq!(run.status.code(), Some(0), "{file}: {run:?}");
        assert!(
            !String::from_utf8_lossy(&run.stdout).contains(FILE_NAME),
            "{run:?}"
        );
    }
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 0);
}

#[test]
fn ns10_root_stdin_reports_exact_basename_without_writes_and_rejects_fix() {
    let dir = project();
    let witness = dir.path().join("lib/BadName.rb");
    for source in [
        "print 1\n",
        "",
        "\u{feff}print 1\r\n",
        "# encoding: UTF-8\r\nprint __LINE__\r\n__END__\r\nraw data\r\n",
    ] {
        fs::write(&witness, source).unwrap();
        let before = fs::metadata(&witness).unwrap();
        let directory = fs::metadata(dir.path().join("lib")).unwrap();
        for file in ["/", "//", "////", "/./"] {
            let run = cli(
                dir.path(),
                "",
                &["--only", FILE_NAME, "--stdin", file],
                Some(source),
            );
            assert_eq!(run.status.code(), Some(1), "{file}: {run:?}");
            assert!(run.stderr.is_empty(), "{run:?}");
            assert_eq!(
                String::from_utf8(run.stdout).unwrap(),
                format!(
                    "/:1:1: C: The name of this source file (`/`) should use snake_case. ({FILE_NAME})\n\n1 file inspected, 1 offense detected\n"
                )
            );
            rejecting_stdin(
                dir.path(),
                &["--only", FILE_NAME, "--fix", "--stdin", file],
                source,
            );
        }
        assert_eq!(fs::read(&witness).unwrap(), source.as_bytes());
        let after = fs::metadata(&witness).unwrap();
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());
        assert_eq!(after.permissions(), before.permissions());
        assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 1);
        let after = fs::metadata(dir.path().join("lib")).unwrap();
        assert_eq!(after.modified().unwrap(), directory.modified().unwrap());
        assert_eq!(after.permissions(), directory.permissions());
        assert!(!dir.path().join("lib/bad_name.rb").exists());
    }
}

#[test]
fn ns10_fix_preserves_bytes_mtime_filename_permissions_and_findings() {
    let dir = project();
    let path = dir.path().join("lib/badName.rb");
    for source in [
        "print 1\n",
        "",
        "\u{feff}print 1\r\n",
        "# encoding: UTF-8\r\nprint __LINE__\r\n__END__\r\nraw data\r\n",
    ] {
        fs::write(&path, source).unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let lint = cli(
            dir.path(),
            "",
            &["--only", FILE_NAME, "lib/badName.rb"],
            None,
        );
        assert_eq!(lint.status.code(), Some(1), "{lint:?}");
        let diagnostic = String::from_utf8(lint.stdout)
            .unwrap()
            .lines()
            .find(|s| s.contains(FILE_NAME))
            .unwrap()
            .to_owned();
        for _ in 0..2 {
            let run = cli(
                dir.path(),
                "",
                &["--only", FILE_NAME, "--fix", "lib/badName.rb"],
                None,
            );
            assert_eq!(run.status.code(), Some(1), "{run:?}");
            let output = String::from_utf8_lossy(&run.stdout);
            assert!(
                output.contains(&diagnostic) && output.contains("0 corrections applied"),
                "{output}"
            );
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            let after = fs::metadata(&path).unwrap();
            assert_eq!(after.modified().unwrap(), metadata.modified().unwrap());
            assert_eq!(after.permissions(), metadata.permissions());
            assert!(!dir.path().join("lib/bad_name.rb").exists());
        }
    }
    rejecting_stdin(
        dir.path(),
        &["--only", FILE_NAME, "--fix", "--stdin", "lib/badName.rb"],
        "print 1\n",
    );
    // Directory discovery must still pass actual names (only .rb recursively).
    let run = cli(dir.path(), "", &["--only", FILE_NAME, "lib"], None);
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert!(String::from_utf8_lossy(&run.stdout).contains("lib/badName.rb:1:1"));
}

#[test]
fn ns10_diagnostics_survive_canonical_naming_style_and_layout_peer_fixes() {
    let dir = project();
    let path = dir.path().join("lib/badName.rb");
    fs::write(&path, "def +(arg); arg; end\nfail  \n").unwrap();
    let run = cli(
        dir.path(),
        "",
        &[
            "--only",
            "Naming/FileName,Naming/BinaryOperatorParameterName,Style/SignalException,Layout/TrailingWhitespace",
            "--fix",
            "lib/badName.rb",
        ],
        None,
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert!(String::from_utf8_lossy(&run.stdout).contains(FILE_NAME));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def +(other); other; end\nraise\n"
    );
    assert!(!dir.path().join("lib/bad_name.rb").exists());
}

#[cfg(unix)]
#[test]
fn ns10_supplied_symlink_names_not_targets_and_permission_bits_are_not_shebangs() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = project();
    let target = dir.path().join("lib/good.rb");
    fs::write(&target, "print 1\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    symlink("good.rb", dir.path().join("lib/badName.rb")).unwrap();
    symlink("badName.rb", dir.path().join("lib/also_good.rb")).unwrap();
    let before = fs::metadata(&target).unwrap().modified().unwrap();
    for (file, flagged) in [
        ("lib/good.rb", false),
        ("lib/badName.rb", true),
        ("lib/also_good.rb", false),
    ] {
        let run = cli(dir.path(), "", &["--only", FILE_NAME, "--fix", file], None);
        assert_eq!(
            run.status.code(),
            Some(i32::from(flagged)),
            "{file}: {run:?}"
        );
    }
    assert_eq!(fs::read(&target).unwrap(), b"print 1\n");
    assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), before);
    assert!(
        fs::symlink_metadata(dir.path().join("lib/badName.rb"))
            .unwrap()
            .is_symlink()
    );
    // Interior parent steps cannot be collapsed without changing aliases:
    // link/../.Fastfile is outside the root, not root .Fastfile. The basename
    // is identical; only inherited path eligibility differs from the pin's
    // lexical File.expand_path (also for lib/../.Fastfile in the matrix).
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(outside.path().join("child")).unwrap();
    symlink(outside.path().join("child"), dir.path().join("link")).unwrap();
    let local_fastfile = dir.path().join(".Fastfile");
    let outside_fastfile = outside.path().join(".Fastfile");
    for (path, source, input, flagged) in [
        (&local_fastfile, "print 1\r\n", ".Fastfile", true),
        (&outside_fastfile, "print 2\r\n", "link/../.Fastfile", false),
    ] {
        fs::write(path, source).unwrap();
        let before = fs::metadata(path).unwrap();
        let run = cli(dir.path(), "", &["--only", FILE_NAME, "--fix", input], None);
        assert_eq!(
            run.status.code(),
            Some(i32::from(flagged)),
            "{input}: {run:?}"
        );
        assert_eq!(fs::read(path).unwrap(), source.as_bytes());
        let after = fs::metadata(path).unwrap();
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());
        assert_eq!(after.permissions(), before.permissions());
        assert!(!path.with_file_name(".fastfile").exists());
    }
    // A shebang exempts even a non-executable file; mode bits never decide it.
    fs::write(&target, "#!/usr/bin/env ruby\nprint 1\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        cli(
            dir.path(),
            "",
            &["--only", FILE_NAME, "lib/badName.rb"],
            None
        )
        .status
        .code(),
        Some(0)
    );
}

#[test]
fn ns10_native_ruby34_syntax_only_evidence_for_all_pinned_sources() {
    if crate::support::runtime::skips(
        "Ruby 4.0 finds no script after a byte order mark and a shebang line",
    ) {
        return;
    }
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns10_pinned.json")).unwrap();
    let matrix: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns10_paths.json")).unwrap();
    let dir = project();
    let path = dir.path().join("lib/badName.rb");
    let mut seen = std::collections::BTreeSet::new();
    for row in rows.iter().chain(&matrix) {
        let source = row["source"].as_str().unwrap();
        if !seen.insert(source) {
            continue;
        }
        fs::write(&path, source).unwrap();
        for ruby in ns04_runtimes() {
            // Syntax-only: never execute source. Native Ruby's script loader
            // can reject a non-Ruby shebang even though Prism accepts it.
            let output = Command::new(&ruby).arg("-c").arg(&path).output();
            match output {
                Ok(out) if source == "#!anything\n" => {
                    assert!(!out.status.success());
                    assert!(
                        String::from_utf8_lossy(&out.stderr).contains("no Ruby script found"),
                        "{out:?}"
                    );
                }
                Ok(out) => assert!(out.status.success(), "{ruby:?}: {source}: {out:?}"),
                Err(e) if ruby == "ruby" && e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => panic!("{ruby:?}: {e}"),
            }
        }
    }
    assert!(seen.len() >= 20);
}

const INCLUSIVE_LANGUAGE: &str = "Naming/InclusiveLanguage";
fn ns09_cases() -> [(&'static str, &'static str, u32, &'static str); 1] {
    [(
        INCLUSIVE_LANGUAGE,
        "whitelist = 1\n",
        1,
        "Consider replacing 'whitelist' with 'allowlist' or 'permit'.",
    )]
}

#[test]
fn ns09_opt_in_selectors_config_and_real_metadata() {
    let dir = project();
    for (config, selectors, expected) in [
        (String::new(), vec![], false),
        (String::new(), vec!["--only", INCLUSIVE_LANGUAGE], true),
        (String::new(), vec!["--only", "Naming"], true),
        (
            String::new(),
            vec!["--only", INCLUSIVE_LANGUAGE, "--except", INCLUSIVE_LANGUAGE],
            false,
        ),
        (
            format!("[cops.\"{INCLUSIVE_LANGUAGE}\"]\nenabled = true\n"),
            vec![],
            true,
        ),
        (
            format!("[cops.\"{INCLUSIVE_LANGUAGE}\"]\nenabled = false\n"),
            vec!["--only", INCLUSIVE_LANGUAGE],
            true,
        ),
        ("[cops.Naming]\nenabled = true\n".into(), vec![], true),
        (
            format!(
                "[cops.Naming]\nenabled = true\n[cops.\"{INCLUSIVE_LANGUAGE}\"]\nenabled = false\n"
            ),
            vec![],
            false,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.extend(["--stdin", "lib/a.rb"]);
        let run = cli(dir.path(), "", &args, Some("whitelist = 1\n"));
        assert_ne!(run.status.code(), Some(2), "{run:?}");
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).contains(&format!("({INCLUSIVE_LANGUAGE})")),
            expected,
            "{run:?}"
        );
    }
    let registry = rustocop::cops::Registry::new();
    let cop = registry
        .cops()
        .iter()
        .find(|c| c.name() == INCLUSIVE_LANGUAGE)
        .unwrap();
    assert!(!cop.default_enabled());
    // All fixed suggestions are ambiguous. Do not advertise fabricated fixes.
    assert!(!cop.safe_fixes());
    assert!(cop.path_policy().is_none());
    assert_eq!(cop.ruby_versions(), Default::default());
}

#[test]
fn ns09_supplied_paths_are_diagnostic_only_and_never_renamed() {
    let dir = project();
    for (path, message) in [
        (
            "lib/whitelist.rb",
            "Consider replacing 'whitelist' in file path with 'allowlist' or 'permit'.",
        ),
        (
            "lib/slave/black-list.rb",
            "Consider replacing 'slave', 'black-list' in file path with other terms.",
        ),
        ("lib/Slave1.rb", ""),
        (
            "lib/WhiteList.rb",
            "Consider replacing 'WhiteList' in file path with 'allowlist' or 'permit'.",
        ),
    ] {
        let disk = dir.path().join(path);
        fs::create_dir_all(disk.parent().unwrap()).unwrap();
        fs::write(&disk, "").unwrap();
        let mtime = fs::metadata(&disk).unwrap().modified().unwrap();
        for args in [
            vec!["--only", INCLUSIVE_LANGUAGE, path],
            vec!["--only", INCLUSIVE_LANGUAGE, "--fix", path],
            vec!["--only", INCLUSIVE_LANGUAGE, "--stdin", path],
        ] {
            let run = cli(
                dir.path(),
                "",
                &args,
                if args.contains(&"--stdin") {
                    Some("")
                } else {
                    None
                },
            );
            assert_eq!(
                run.status.code(),
                Some(i32::from(!message.is_empty())),
                "{run:?}"
            );
            if !message.is_empty() {
                assert!(
                    String::from_utf8_lossy(&run.stdout)
                        .contains(&format!("{path}:1:1: C: {message} ({INCLUSIVE_LANGUAGE})")),
                    "{run:?}"
                );
            }
            assert_eq!(fs::read(&disk).unwrap(), b"");
            assert_eq!(fs::metadata(&disk).unwrap().modified().unwrap(), mtime);
        }
    }
}

#[test]
fn ns09_all_fixed_policy_spec_adaptations_and_source_examples_are_no_write() {
    for fixture in [
        include_str!("fixtures/ns09_pinned.json"),
        include_str!("fixtures/ns09_source.json"),
        include_str!("fixtures/ns09_syntax.json"),
        include_str!("fixtures/ns09_paths.json"),
    ] {
        let mut rows: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for row in &mut rows {
            if row["source"].as_str().unwrap().starts_with('\u{feff}') {
                for p in row["positions"].as_array_mut().unwrap() {
                    if p[0] == 1 {
                        p[1] = serde_json::json!(p[1].as_u64().unwrap() - 1);
                    }
                }
            }
        }
        assert_ns02_calls_preserve_bytes_and_mtime(&rows);
    }
}

#[test]
fn ns09_existing_canonical_naming_style_and_layout_fixes_survive() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "whitelist = 1  \ndef +(arg); arg; end\nfail\n";
    fs::write(&path, source).unwrap();
    let run = cli(
        dir.path(),
        "",
        &[
            "--only",
            "Naming/InclusiveLanguage,Naming/BinaryOperatorParameterName,Style/SignalException,Layout/TrailingWhitespace",
            "--fix",
            "lib/a.rb",
        ],
        None,
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert!(String::from_utf8_lossy(&run.stdout).contains(&format!("({INCLUSIVE_LANGUAGE})")));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "whitelist = 1\ndef +(other); other; end\nraise\n"
    );
    for setting in [
        "CheckStrings = true",
        "CheckFilepaths = false",
        "FlaggedTerms = {}",
        "Suggestions = ['allowlist']",
        "max = 3",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{INCLUSIVE_LANGUAGE}\"]\n{setting}\n"),
        )
        .unwrap();
        assert_eq!(
            cli(dir.path(), "", &["--fix", "lib/a.rb"], None)
                .status
                .code(),
            Some(2)
        );
    }
}

#[test]
fn ns09_native_syntax_encoded_boundaries_and_runtime_no_write_evidence() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    if let Some(ruby) = crate::support::runtime::ruby_os() {
        let version = Command::new(&ruby)
            .args([
                "-e",
                crate::support::runtime::script(
                    "abort RUBY_VERSION unless RUBY_VERSION.start_with?('3.4.')",
                )
                .as_str(),
            ])
            .output()
            .unwrap();
        assert!(version.status.success(), "{version:?}");
        for fixture in [
            include_str!("fixtures/ns09_pinned.json"),
            include_str!("fixtures/ns09_source.json"),
            include_str!("fixtures/ns09_syntax.json"),
        ] {
            let rows: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
            for row in rows {
                fs::write(&path, row["source"].as_str().unwrap()).unwrap();
                let run = Command::new(&ruby).arg("-c").arg(&path).output().unwrap();
                assert!(run.status.success(), "{row}: {run:?}");
            }
        }
    }
    for (encoding, letter) in [
        ("UTF-8", "é".as_bytes()),
        ("ISO-8859-1", b"\xe9".as_slice()),
        ("Shift_JIS", b"\x83\x5f".as_slice()),
        ("GB18030", b"\x81\x36\xbf\x32".as_slice()),
    ] {
        let mut source = format!("# encoding: {encoding}\r\n# ").into_bytes();
        source.extend(letter);
        source.extend(b"slave\r\nwhitelist = 1\r\n__END__\r\nblacklist\r\n");
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        if let Some(ruby) = crate::support::runtime::ruby_os() {
            assert!(
                Command::new(ruby)
                    .arg("-c")
                    .arg(&path)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        let run = cli(
            dir.path(),
            "",
            &["--only", INCLUSIVE_LANGUAGE, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(1), "{run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert!(output.contains("lib/a.rb:2:4: C:"), "{encoding}: {output}");
        assert_eq!(
            output.matches(&format!("({INCLUSIVE_LANGUAGE})")).count(),
            2
        );
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
    let source = "# whitelist slave\nwhitelist = [1, 2]\nputs [whitelist, binding.local_variable_get(:whitelist), __LINE__, DATA.read, File.binread(__FILE__)].inspect\n__END__\nblacklist data\n";
    fs::write(&path, source).unwrap();
    let before: Vec<_> = ns04_runtimes()
        .into_iter()
        .filter_map(|r| ns04_ruby(&r, &path).map(|o| (r, o)))
        .collect();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let run = cli(
        dir.path(),
        "",
        &["--only", INCLUSIVE_LANGUAGE, "--fix", "lib/a.rb"],
        None,
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    for (ruby, original) in before {
        assert!(original.status.success(), "{original:?}");
        let fixed = ns04_ruby(&ruby, &path).unwrap();
        assert_eq!(fixed.stdout, original.stdout);
        assert_eq!(fixed.stderr, original.stderr);
    }
}

#[test]
fn ns09_encoded_pin_projections_keep_diagnostics_and_never_rewrite() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns09_encoding.json")).unwrap();
    for row in rows {
        let hex = row["source_hex"].as_str().unwrap();
        let source: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        if let Some(ruby) = crate::support::runtime::ruby_os() {
            let native = Command::new(ruby).arg("-c").arg(&path).output().unwrap();
            assert!(native.status.success(), "{row}: {native:?}");
        }
        let run = cli(
            dir.path(),
            "",
            &["--only", INCLUSIVE_LANGUAGE, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(1), "{row}: {run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        let expected = row
            .get("rustocop_positions")
            .unwrap_or(&row["positions"])
            .as_array()
            .unwrap();
        assert_eq!(
            output.matches(&format!("({INCLUSIVE_LANGUAGE})")).count(),
            expected.len(),
            "{row}: {output}"
        );
        for p in expected {
            assert!(
                output.contains(&format!("lib/a.rb:{}:{}: C:", p[0], p[1])),
                "{row}: {output}"
            );
        }
        assert!(output.contains("0 corrections applied"));
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

fn ns08_cases() -> [(&'static str, &'static str, u32, &'static str); 1] {
    [(
        "Naming/PredicateMethod",
        "def foo; true; end\n",
        5,
        "Predicate method names should end with `?`.",
    )]
}

fn ns07_cases() -> [(&'static str, &'static str, u32, &'static str); 1] {
    [(
        "Naming/MemoizedInstanceVariableName",
        "def foo; @bad ||= 1; end\n",
        10,
        "Memoized variable `@bad` does not match method name `foo`. Use `@foo` instead.",
    )]
}

fn ns06_cases() -> [(&'static str, &'static str, u32, &'static str); 2] {
    [
        (
            "Naming/HeredocDelimiterCase",
            "p <<~sql\ntext\nsql\n",
            1,
            "Use uppercase heredoc delimiters.",
        ),
        (
            "Naming/HeredocDelimiterNaming",
            "p <<~END\ntext\nEND\n",
            1,
            "Use meaningful heredoc delimiters.",
        ),
    ]
}

fn ns05_cases() -> [(&'static str, &'static str, u32, &'static str); 1] {
    [(
        "Naming/BlockForwarding",
        "def foo(&callback); end\n",
        9,
        "Use anonymous block forwarding.",
    )]
}

fn ns04_cases() -> [(&'static str, &'static str, u32, &'static str); 2] {
    [
        (
            "Naming/BinaryOperatorParameterName",
            "def +(arg); arg; end\n",
            7,
            "When defining the `+` operator, name its argument `other`.",
        ),
        (
            "Naming/RescuedExceptionsVariableName",
            "begin; rescue => error; error; end\n",
            18,
            "Use `e` instead of `error`.",
        ),
    ]
}

#[test]
fn fixed_naming_defaults_selectors_config_and_metadata() {
    let dir = project();
    for (cop, source, col, message) in ns02_cases()
        .into_iter()
        .chain(ns03_cases())
        .chain(ns04_cases())
        .chain(ns05_cases())
        .chain(ns06_cases())
        .chain(ns07_cases())
        .chain(ns08_cases())
    {
        for (config, selectors, expected) in [
            (String::new(), vec![], true),
            (String::new(), vec!["--only", cop], true),
            (String::new(), vec!["--only", "Naming"], true),
            (String::new(), vec!["--except", cop], false),
            (
                String::new(),
                vec!["--only", "Naming", "--except", cop],
                false,
            ),
            (String::new(), vec!["--only", "Lint"], false),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec![],
                false,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                true,
            ),
            ("[cops.Naming]\nenabled = false\n".into(), vec![], false),
            (
                format!("[cops.Naming]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                true,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.extend(["--stdin", "lib/a.rb"]);
            let run = cli(dir.path(), "", &args, Some(source));
            assert_ne!(run.status.code(), Some(2), "{run:?}");
            let output = String::from_utf8_lossy(&run.stdout);
            assert_eq!(output.contains(&format!("({cop})")), expected, "{run:?}");
            if expected {
                let line = if cop.starts_with("Naming/HeredocDelimiter") {
                    3
                } else {
                    1
                };
                assert!(
                    output.contains(&format!("lib/a.rb:{line}:{col}: C: {message} ({cop})")),
                    "{output}"
                );
            }
        }
        let registry = rustocop::cops::Registry::new();
        let rule = registry.cops().iter().find(|r| r.name() == cop).unwrap();
        assert!(rule.default_enabled());
        assert_eq!(
            rule.safe_fixes(),
            ns04_cases()
                .iter()
                .chain(ns05_cases().iter())
                .any(|c| c.0 == cop)
                || cop == "Naming/HeredocDelimiterCase"
        );
        assert_eq!(
            rule.path_policy().is_some(),
            cop == "Naming/PredicatePrefix"
        );
        let minimum = if cop == "Naming/BlockForwarding" {
            Some(rustocop::cops::policy::RubyVersion(3, 1))
        } else {
            None
        };
        assert_eq!(rule.ruby_versions().minimum, minimum);
        assert_eq!(rule.ruby_versions().maximum, None);
    }
}

#[test]
fn fixed_naming_exclusions_are_config_relative_for_files_stdin_and_fix() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, source, _, _) in ns02_cases()
        .into_iter()
        .chain(ns03_cases())
        .chain(ns04_cases())
        .chain(ns05_cases())
        .chain(ns06_cases())
        .chain(ns07_cases())
        .chain(ns08_cases())
        .chain(ns09_cases())
    {
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        for config in [
            "exclude = [\"lib/a.rb\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            for args in [
                vec!["--only", cop, "a.rb"],
                vec!["--only", cop, "--fix", "a.rb"],
            ] {
                assert_eq!(cli(dir.path(), "lib", &args, None).status.code(), Some(0));
            }
            let run = cli(
                dir.path(),
                "lib",
                &["--only", cop, "--stdin", "a.rb"],
                Some(source),
            );
            assert_eq!(run.status.code(), Some(0), "{run:?}");
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
}

#[test]
fn fixed_naming_directives_and_source_enables_respect_selector_precedence() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, statement, _, _) in ns02_cases()
        .into_iter()
        .chain(ns03_cases())
        .chain(ns07_cases())
        .chain(ns08_cases())
        .chain(ns09_cases())
    {
        for prefix in ["rustocop", "rubocop"] {
            for (source, count) in [
                (
                    format!("{} # {prefix}:disable {cop}\n", statement.trim_end()),
                    0,
                ),
                (format!("# {prefix}:disable Naming\n{statement}"), 0),
                (
                    format!("# {prefix}:disable-next {cop}\n{statement}{statement}"),
                    1,
                ),
                (
                    format!(
                        "# {prefix}:disable {cop}\n{statement}# {prefix}:enable {cop}\n{statement}"
                    ),
                    1,
                ),
            ] {
                fs::write(&path, &source).unwrap();
                let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                for fix in [false, true] {
                    let mut args = vec!["--only", cop, "lib/a.rb"];
                    if fix {
                        args.push("--fix");
                    }
                    let run = cli(dir.path(), "", &args, None);
                    assert_eq!(run.status.code(), Some(i32::from(count != 0)), "{run:?}");
                    assert_eq!(
                        String::from_utf8_lossy(&run.stdout)
                            .matches(&format!("({cop})"))
                            .count(),
                        count
                    );
                    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                }
            }
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            let source = format!("# {prefix}:enable {cop}\n{statement}");
            let run = cli(dir.path(), "", &["--stdin", "lib/a.rb"], Some(&source));
            assert!(
                String::from_utf8_lossy(&run.stdout).contains(cop),
                "{run:?}"
            );
            let run = cli(
                dir.path(),
                "",
                &["--except", cop, "--stdin", "lib/a.rb"],
                Some(&source),
            );
            assert!(
                !String::from_utf8_lossy(&run.stdout).contains(cop),
                "{run:?}"
            );
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        }
    }
}

#[test]
fn fixed_naming_detection_only_preserves_envelopes_findings_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, statement, _, _) in ns02_cases()
        .into_iter()
        .chain(ns03_cases())
        .chain(ns07_cases())
        .chain(ns08_cases())
        .chain(ns09_cases())
    {
        for source in [
            statement.to_owned(),
            format!("\u{feff}{}", statement.replace('\n', "\r\n")),
            format!(
                "# encoding: UTF-8\r\n{}p __LINE__\r\n__END__\r\nraw föö_1\r\n",
                statement.replace('\n', "\r\n")
            ),
        ] {
            fs::write(&path, &source).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            let lint = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
            assert_eq!(lint.status.code(), Some(1), "{lint:?}");
            let diagnostics: Vec<_> = String::from_utf8(lint.stdout)
                .unwrap()
                .lines()
                .filter(|line| line.contains(&format!("({cop})")))
                .map(str::to_owned)
                .collect();
            assert_eq!(diagnostics.len(), 1);
            for _ in 0..2 {
                let fixed = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
                assert_eq!(fixed.status.code(), Some(1), "{fixed:?}");
                assert!(fixed.stderr.is_empty(), "{fixed:?}");
                let output = String::from_utf8_lossy(&fixed.stdout);
                assert!(output.contains("0 corrections applied"), "{output}");
                assert_eq!(output.matches(&format!("({cop})")).count(), 1);
                assert!(output.contains(&diagnostics[0]));
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
        }
        let before = fs::read(&path).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        rejecting_stdin(
            dir.path(),
            &["--only", cop, "--fix", "--stdin", "lib/a.rb"],
            statement,
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn ns02_fixed_options_are_not_public_configuration() {
    let dir = project();
    for (cop, key, value) in [
        ("Naming/AsciiIdentifiers", "AsciiConstants", "false"),
        ("Naming/BlockParameterName", "MinNameLength", "2"),
        ("Naming/VariableNumber", "EnforcedStyle", "\"snake_case\""),
        ("Naming/PredicatePrefix", "UseSorbetSigs", "true"),
        ("Naming/PredicatePrefix", "NamePrefix", "[\"looks_\"]"),
        (
            "Naming/RescuedExceptionsVariableName",
            "PreferredName",
            "\"exception\"",
        ),
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\n{key} = {value}\n"),
        )
        .unwrap();
        rejecting_stdin(
            dir.path(),
            &["--only", cop, "--stdin", "lib/a.rb"],
            "foo = 1\n",
        );
    }
}

#[test]
fn ns02_reports_survive_an_existing_canonical_neighbor_fix() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "föö_1 = 1\nfoo { |aB| }\nfail\n";
    fs::write(&path, source).unwrap();
    let run = cli(
        dir.path(),
        "",
        &[
            "--only",
            "Naming/AsciiIdentifiers,Naming/BlockParameterName,Naming/VariableNumber,Style/SignalException",
            "--fix",
            "lib/a.rb",
        ],
        None,
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    let output = String::from_utf8_lossy(&run.stdout);
    assert!(output.contains("1 correction applied"), "{output}");
    for (cop, _, _, _) in ns02_cases() {
        assert!(output.contains(cop), "{output}");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("fail", "raise")
    );
}

#[test]
fn ns02_destructured_highlight_uses_encoded_first_child_text_length() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let cop = "Naming/BlockParameterName";
    for (encoding, letter) in [
        ("UTF-8", "É".as_bytes()),
        ("Shift_JIS", b"\x82\x60".as_slice()),
    ] {
        let mut source = format!("# encoding: {encoding}\nfoo {{ |(a").into_bytes();
        source.extend_from_slice(letter);
        source.extend_from_slice(b",     \n b)| } # rubocop:disable Naming/BlockParameterName\n");
        fs::write(&path, &source).unwrap();
        let run = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
        // The range ends on the first physical line: a later directive must
        // not suppress it just because a source character occupied two bytes.
        assert_eq!(run.status.code(), Some(1), "{encoding}: {run:?}");
        assert!(
            String::from_utf8_lossy(&run.stdout).contains("lib/a.rb:2:8: C:"),
            "{run:?}"
        );
    }
}

#[test]
fn ns02_review_regressions_through_cli_fix_preserve_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let mut cases = Vec::new();
    for encoding in ["Shift_JIS", "Windows-31J"] {
        let mut source = format!("# encoding: {encoding}\nfoo").into_bytes();
        source.extend_from_slice(b"\x83\x5f1 = 1\nfoo\x83\x5f_1 = 1\n");
        cases.push(("Naming/VariableNumber", source, vec![(3, 1)]));
    }
    for source in [
        "if cond; def self.Foo_1; end; else; class Foo_1; end; end\n",
        "begin; def self.Foo_1; end; ensure; class Foo_1; end; end\n",
    ] {
        cases.push(("Naming/VariableNumber", source.as_bytes().to_vec(), vec![]));
    }
    for sigil in ["*", "**", "&"] {
        let source =
            format!("foo {{ |{sigil}\naB| # rubocop:disable Naming/BlockParameterName\n}}\n");
        cases.push(("Naming/BlockParameterName", source.into_bytes(), vec![]));
    }
    for (cop, source, positions) in cases {
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
        assert_eq!(
            run.status.code(),
            Some(i32::from(!positions.is_empty())),
            "{run:?}"
        );
        assert!(run.stderr.is_empty(), "{run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(
            output.matches(&format!("({cop})")).count(),
            positions.len(),
            "{output}"
        );
        for (line, col) in positions {
            assert!(
                output.contains(&format!("lib/a.rb:{line}:{col}: C:")),
                "{output}"
            );
        }
        assert!(output.contains("0 corrections applied"), "{output}");
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn ns02_review2_decoded_symbols_and_destructured_comments_are_detection_only() {
    let mut calls = Vec::new();
    for encoding in ["Shift_JIS", "Windows-31J", "EUC-JP", "ISO-8859-1"] {
        for (symbol, col) in [
            (":\"\\u{401}_1\"", 1),
            (":\"\\u0401_1\"", 1),
            ("%I[\\u{401}_1]", 4),
            ("{\"\\u{401}_1\": 0}", 2),
        ] {
            calls.push(serde_json::json!({
                "cop": "Naming/VariableNumber",
                "source": format!("# encoding: {encoding}\n{symbol}\n"),
                "positions": [[2, col]],
                "messages": ["Use normalcase for symbol numbers."]
            }));
        }
    }
    for (source, uppercase) in [
        ("foo { |((a, # UPPER\n b), c)| }\n", false),
        ("foo { |((a, # lower\n b), c)| }\n", false),
        ("foo { |((a, # lower\n bC), c)| }\n", true),
        ("foo { |((a, # UPPER\n bC), c)| }\n", true),
        ("foo { |((a, b), cD)| }\n", false),
        ("foo { |((a, *bC), c)| }\n", true),
    ] {
        calls.push(serde_json::json!({
            "cop": "Naming/BlockParameterName",
            "source": source,
            "positions": if uppercase { vec![[1, 8]] } else { vec![] },
            "messages": if uppercase {
                vec!["Only use lowercase characters for block parameter."]
            } else { vec![] }
        }));
    }
    if let Some(ruby) = crate::support::runtime::ruby_os() {
        let dir = project();
        let path = dir.path().join("lib/a.rb");
        for call in &calls {
            fs::write(&path, call["source"].as_str().unwrap()).unwrap();
            let native = Command::new(&ruby).arg("-c").arg(&path).output().unwrap();
            assert!(native.status.success(), "{call}: {native:?}");
            assert!(String::from_utf8_lossy(&native.stdout).contains("Syntax OK"));
        }
    }
    assert_ns02_calls_preserve_bytes_and_mtime(&calls);
}

#[test]
fn ns02_ascii_selector_ownership_preserves_nested_and_encoded_names_without_writes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let cop = "Naming/AsciiIdentifiers";
    for (encoding, letter) in [
        ("UTF-8", "ö".as_bytes()),
        ("ISO-8859-1", b"\xf6".as_slice()),
        ("Windows-1252", b"\xf6".as_slice()),
        ("Shift_JIS", b"\x83\x5f".as_slice()),
        ("Windows-31J", b"\x83\x5f".as_slice()),
        ("EUC-JP", b"\xa5\xc0".as_slice()),
    ] {
        let template = "obj[:fX]\nobj[\"fX\"]\nobj[:fX] = 1\nobj[fX]\nfX[:unused]\nobj[\"#{fX}\"]\nobj.[](:fX)\nobj.[]=(fX, 1)\nobj[fX] += 1\nobj[fX] ||= 1\nobj[fX] &&= 1\nobj.fX = 1\nobj.fX, other = values\n";
        let mut source = format!("# encoding: {encoding}\n").into_bytes();
        for byte in template.bytes() {
            if byte == b'X' {
                source.extend_from_slice(letter);
            } else {
                source.push(byte);
            }
        }
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        if let Some(ruby) = crate::support::runtime::ruby_os() {
            let native = Command::new(ruby).arg("-c").arg(&path).output().unwrap();
            assert!(native.status.success(), "{encoding}: {native:?}");
            assert!(String::from_utf8_lossy(&native.stdout).contains("Syntax OK"));
        }
        for args in [
            vec!["--only", cop, "lib/a.rb"],
            vec!["--only", cop, "--fix", "lib/a.rb"],
        ] {
            let run = cli(dir.path(), "", &args, None);
            assert_eq!(run.status.code(), Some(1), "{encoding}: {run:?}");
            assert!(run.stderr.is_empty(), "{run:?}");
            let output = String::from_utf8_lossy(&run.stdout);
            let expected = [
                (5, 6),
                (6, 2),
                (7, 9),
                (9, 10),
                (10, 6),
                (11, 6),
                (12, 6),
                (13, 6),
                (14, 6),
            ];
            assert_eq!(
                output.matches(&format!("({cop})")).count(),
                expected.len(),
                "{encoding}: {output}"
            );
            for (line, col) in expected {
                assert!(
                    output.contains(&format!(
                        "lib/a.rb:{line}:{col}: C: Use only ascii symbols in identifiers. ({cop})"
                    )),
                    "{encoding}: {output}"
                );
            }
            if args.contains(&"--fix") {
                assert!(output.contains("0 corrections applied"), "{output}");
            }
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
}

#[test]
fn ns02_all_applicable_pinned_calls_are_detection_only_through_real_fix() {
    // RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns02_pinned.json")).unwrap();
    assert_ns02_calls_preserve_bytes_and_mtime(fixture["calls"].as_array().unwrap());
}

#[test]
fn ns02_all_pinned_source_documentation_is_detection_only_through_real_fix() {
    // RuboCop source examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let calls: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns02_source.json")).unwrap();
    assert_eq!(calls.len(), 46);
    assert_ns02_calls_preserve_bytes_and_mtime(&calls);
}

#[test]
fn ns08_all_pinned_specs_source_and_scope_matrix_are_detection_only_through_real_fix() {
    // RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let pinned: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns08_pinned.json")).unwrap();
    assert_ns02_calls_preserve_bytes_and_mtime(pinned["calls"].as_array().unwrap());
    for fixture in [
        include_str!("fixtures/ns08_source.json"),
        include_str!("fixtures/ns08_syntax.json"),
    ] {
        let rows: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        assert_ns02_calls_preserve_bytes_and_mtime(&rows);
    }
}

#[test]
fn ns11_predicate_elsif_controls_are_native_valid_and_detection_only() {
    // RuboCop source: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns11_predicate_branches.json")).unwrap();
    for row in &mut rows {
        row["cop"] = "Naming/PredicateMethod".into();
        if let Some(ruby) = crate::support::runtime::ruby_os() {
            let dir = project();
            let path = dir.path().join("lib/a.rb");
            fs::write(&path, row["source"].as_str().unwrap()).unwrap();
            let run = Command::new(&ruby).arg("-c").arg(&path).output().unwrap();
            assert!(run.status.success(), "{row}: {run:?}");
        }
    }
    assert_ns02_calls_preserve_bytes_and_mtime(&rows);
}

#[test]
fn ns08_native_syntax_and_encoded_method_name_columns() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let pinned: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns08_pinned.json")).unwrap();
    let documented: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns08_source.json")).unwrap();
    let syntax: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns08_syntax.json")).unwrap();
    if let Some(ruby) = crate::support::runtime::ruby_os() {
        let version = Command::new(&ruby)
            .args([
                "-e",
                crate::support::runtime::script(
                    "abort RUBY_VERSION unless RUBY_VERSION.start_with?('3.4.')",
                )
                .as_str(),
            ])
            .output()
            .unwrap();
        assert!(version.status.success(), "{version:?}");
        let sources: std::collections::BTreeSet<_> = pinned["calls"]
            .as_array()
            .unwrap()
            .iter()
            .chain(&documented)
            .chain(&syntax)
            .map(|r| r["source"].as_str().unwrap())
            .collect();
        for source in sources {
            fs::write(&path, source).unwrap();
            let run = Command::new(&ruby).arg("-c").arg(&path).output().unwrap();
            assert!(run.status.success(), "{source}: {run:?}");
        }
    }
    for (encoding, letter) in [
        ("UTF-8", "é".as_bytes()),
        ("ISO-8859-1", b"\xe9".as_slice()),
        ("Shift_JIS", b"\x83\x5f".as_slice()),
    ] {
        let mut source = format!("# encoding: {encoding}\r\n\"").into_bytes();
        source.extend(letter);
        source.extend(b"\"; def f");
        source.extend(letter);
        source.extend(b"; true; end\r\n__END__\r\nraw\r\n");
        fs::write(&path, &source).unwrap();
        if let Some(ruby) = crate::support::runtime::ruby_os() {
            let run = Command::new(ruby).arg("-c").arg(&path).output().unwrap();
            assert!(run.status.success(), "{encoding}: {run:?}");
        }
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", "Naming/PredicateMethod", "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(1), "{run:?}");
        assert!(
            String::from_utf8_lossy(&run.stdout).contains("lib/a.rb:2:10: C:"),
            "{run:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn ns08_preserves_canonical_peer_fixes_and_rejects_new_settings() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def foo; true; end\nfail\n";
    fs::write(&path, source).unwrap();
    let run = cli(
        dir.path(),
        "",
        &[
            "--only",
            "Naming/PredicateMethod,Style/SignalException,Layout",
            "--fix",
            "lib/a.rb",
        ],
        None,
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert!(String::from_utf8_lossy(&run.stdout).contains("(Naming/PredicateMethod)"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("fail", "raise")
    );
    for setting in [
        "Mode = 'aggressive'",
        "AllowBangMethods = true",
        "AllowedMethods = ['foo']",
        "WaywardPredicates = []",
        "max = 2",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"Naming/PredicateMethod\"]\n{setting}\n"),
        )
        .unwrap();
        assert_eq!(
            cli(dir.path(), "", &["--fix", "lib/a.rb"], None)
                .status
                .code(),
            Some(2)
        );
    }
}

#[test]
fn ns07_all_pinned_specs_source_and_syntax_are_detection_only_through_real_fix() {
    // RuboCop source/spec examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let pinned: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns07_pinned.json")).unwrap();
    assert_eq!(
        pinned["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| !r["unsafe_correction"].is_null())
            .count(),
        16
    );
    assert_ns02_calls_preserve_bytes_and_mtime(pinned["calls"].as_array().unwrap());
    for fixture in [
        include_str!("fixtures/ns07_source.json"),
        include_str!("fixtures/ns07_syntax.json"),
        include_str!("fixtures/ns07_review.json"),
    ] {
        let rows: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        assert_ns02_calls_preserve_bytes_and_mtime(&rows);
    }
}

#[test]
fn ns07_native_syntax_pinned_shapes_and_encoded_identifier_positions() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let pinned: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns07_pinned.json")).unwrap();
    let documented: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns07_source.json")).unwrap();
    let syntax: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns07_syntax.json")).unwrap();
    let review: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns07_review.json")).unwrap();
    if let Some(ruby) = crate::support::runtime::ruby_os() {
        let version = Command::new(&ruby)
            .args([
                "-e",
                crate::support::runtime::script(
                    "abort RUBY_VERSION unless RUBY_VERSION.start_with?('3.4.')",
                )
                .as_str(),
            ])
            .output()
            .unwrap();
        assert!(version.status.success(), "{version:?}");
        let sources: std::collections::BTreeSet<_> = pinned["calls"]
            .as_array()
            .unwrap()
            .iter()
            .chain(&documented)
            .chain(&syntax)
            .chain(&review)
            .map(|r| r["source"].as_str().unwrap())
            .collect();
        for source in sources {
            fs::write(&path, source).unwrap();
            let run = Command::new(&ruby).arg("-c").arg(&path).output().unwrap();
            assert!(run.status.success(), "{source}: {run:?}");
        }
    }
    for (encoding, letter) in [
        ("UTF-8", "é".as_bytes()),
        ("ISO-8859-1", b"\xe9".as_slice()),
        ("Shift_JIS", b"\x83\x5f".as_slice()),
    ] {
        let mut source = format!("# encoding: {encoding}\r\ndef foo; \"").into_bytes();
        source.extend(letter);
        source.extend(b"\"; @bad ||= 1; end\r\ndef f");
        source.extend(letter);
        source.extend(b"; @bad ||= 1; end\r\n__END__\r\nraw\r\n");
        fs::write(&path, &source).unwrap();
        if let Some(ruby) = crate::support::runtime::ruby_os() {
            let run = Command::new(ruby).arg("-c").arg(&path).output().unwrap();
            assert!(run.status.success(), "{encoding}: {run:?}");
        }
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let cop = "Naming/MemoizedInstanceVariableName";
        let run = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
        assert_eq!(run.status.code(), Some(1), "{run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(output.matches(&format!("({cop})")).count(), 1, "{output}");
        assert!(output.contains("lib/a.rb:2:15: C:"), "{output}");
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn ns07_preserves_canonical_neighbor_fixes_and_rejects_style_settings() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def foo; @bad ||= 1; end\nfail\n";
    fs::write(&path, source).unwrap();
    let run = cli(
        dir.path(),
        "",
        &[
            "--only",
            "Naming/MemoizedInstanceVariableName,Style/SignalException,Layout",
            "--fix",
            "lib/a.rb",
        ],
        None,
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert!(String::from_utf8_lossy(&run.stdout).contains("(Naming/MemoizedInstanceVariableName)"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("fail", "raise")
    );
    for setting in [
        "EnforcedStyleForLeadingUnderscores = 'optional'",
        "enforced_style = 'required'",
        "max = 2",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"Naming/MemoizedInstanceVariableName\"]\n{setting}\n"),
        )
        .unwrap();
        assert_eq!(
            cli(dir.path(), "", &["--fix", "lib/a.rb"], None)
                .status
                .code(),
            Some(2)
        );
    }
}

#[test]
fn ns03_all_pinned_specs_and_source_examples_are_detection_only_through_real_fix() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns03_pinned.json")).unwrap();
    assert_ns02_calls_preserve_bytes_and_mtime(fixture["calls"].as_array().unwrap());
    let source: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns03_source.json")).unwrap();
    assert_ns02_calls_preserve_bytes_and_mtime(&source);
}

#[test]
fn ns03_predicate_spec_exclusion_is_project_relative_not_a_name_heuristic() {
    let dir = project();
    let cop = "Naming/PredicatePrefix";
    let source =
        "# rustocop:enable Naming/PredicatePrefix\ndef is_ready; false; end\ndef get_value; end\n";
    for (relative, count) in [
        ("spec/a.rb", 0),
        ("spec/nested/a.rb", 0),
        ("test/a.rb", 1),
        ("lib/spec/a.rb", 1),
        ("spec.rb", 1),
        ("lib/a_spec.rb", 1),
    ] {
        let path = dir.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        for selectors in [vec![], vec!["--only", cop], vec!["--only", "Naming"]] {
            for fix in [false, true] {
                if fix && selectors.is_empty() {
                    continue;
                }
                let mut args = selectors.clone();
                if fix {
                    args.push("--fix");
                }
                args.push(relative);
                let run = cli(dir.path(), "", &args, None);
                assert_ne!(run.status.code(), Some(2), "{run:?}");
                assert_eq!(
                    String::from_utf8_lossy(&run.stdout)
                        .matches(&format!("({cop})"))
                        .count(),
                    count,
                    "{run:?}"
                );
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
        }
        let dot_relative = format!("./{relative}");
        for (cwd, name) in [
            ("", relative),
            ("", dot_relative.as_str()),
            ("", path.to_str().unwrap()),
            (
                path.parent()
                    .unwrap()
                    .strip_prefix(dir.path())
                    .unwrap()
                    .to_str()
                    .unwrap(),
                path.file_name().unwrap().to_str().unwrap(),
            ),
        ] {
            let run = cli(
                dir.path(),
                cwd,
                &["--only", cop, "--stdin", name],
                Some(source),
            );
            assert_eq!(run.status.code(), Some(i32::from(count != 0)), "{run:?}");
        }
        let run = cli(
            dir.path(),
            "",
            &["--only", "Naming/AccessorMethodName", relative],
            None,
        );
        assert_eq!(run.status.code(), Some(1), "{run:?}");
    }
}

#[test]
fn ns03_pinned_and_legacy_sources_have_host_and_native_syntax_witnesses() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let pinned: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns03_pinned.json")).unwrap();
    let documented: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns03_source.json")).unwrap();
    let sources: std::collections::BTreeSet<_> = pinned["calls"]
        .as_array()
        .unwrap()
        .iter()
        .chain(&documented)
        .map(|r| r["source"].as_str().unwrap())
        .collect();
    for source in sources {
        fs::write(&path, source).unwrap();
        for ruby in std::iter::once(std::ffi::OsString::from("ruby"))
            .chain(crate::support::runtime::ruby_os())
        {
            let run = match Command::new(&ruby).arg("-c").arg(&path).output() {
                Ok(run) => run,
                Err(error) if ruby == "ruby" && error.kind() == std::io::ErrorKind::NotFound => {
                    continue;
                }
                Err(error) => panic!("{ruby:?}: {error}"),
            };
            assert!(run.status.success(), "{ruby:?}: {source}: {run:?}");
        }
    }
    for (encoding, letter) in [
        ("UTF-8", "é".as_bytes()),
        ("ISO-8859-1", b"\xe9".as_slice()),
        ("Windows-1252", b"\xe9".as_slice()),
        ("Shift_JIS", b"\x83\x5f".as_slice()),
        ("EUC-JP", b"\xa3\xc1".as_slice()),
    ] {
        let mut source = format!("# encoding: {encoding}\r\ndef get_").into_bytes();
        source.extend(letter);
        source.extend(b"; end\r\ndef is_");
        source.extend(letter);
        source.extend(b"; false; end\r\ndefine_singleton_method(:has_");
        source.extend(letter);
        source.extend(b") {}\r\n__END__\r\nraw\r\n");
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        for ruby in std::iter::once(std::ffi::OsString::from("ruby"))
            .chain(crate::support::runtime::ruby_os())
        {
            let run = match Command::new(&ruby).arg("-c").arg(&path).output() {
                Ok(run) => run,
                Err(error) if ruby == "ruby" && error.kind() == std::io::ErrorKind::NotFound => {
                    continue;
                }
                Err(error) => panic!("{ruby:?}: {error}"),
            };
            assert!(run.status.success(), "{encoding}: {ruby:?}: {run:?}");
        }
        for (cop, expected) in [
            ("Naming/AccessorMethodName", 1),
            ("Naming/PredicatePrefix", 2),
        ] {
            let lint = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
            let fixed = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
            for run in [&lint, &fixed] {
                assert_eq!(run.status.code(), Some(1), "{encoding}: {run:?}");
                let output = String::from_utf8_lossy(&run.stdout);
                assert_eq!(
                    output.matches(&format!("({cop})")).count(),
                    expected,
                    "{output}"
                );
                assert!(output.contains(if expected == 1 {
                    "lib/a.rb:2:5:"
                } else {
                    "lib/a.rb:3:5:"
                }));
            }
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
}

#[test]
fn ns03_detection_survives_an_existing_canonical_neighbor_fix() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def get_value; end\ndef does_work; false; end\nfail\n";
    fs::write(&path, source).unwrap();
    let run = cli(
        dir.path(),
        "",
        &[
            "--only",
            "Naming/AccessorMethodName,Naming/PredicatePrefix,Style/SignalException",
            "--fix",
            "lib/a.rb",
        ],
        None,
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    let output = String::from_utf8_lossy(&run.stdout);
    assert!(output.contains("1 correction applied"), "{output}");
    for (cop, _, _, _) in ns03_cases() {
        assert!(output.contains(cop), "{output}");
    }
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        source.replace("fail", "raise")
    );
}

fn assert_ns02_calls_preserve_bytes_and_mtime(calls: &[serde_json::Value]) {
    let dir = project();
    for call in calls {
        let cop = call["cop"].as_str().unwrap();
        let source = call["source"].as_str().unwrap();
        let display = if cop == INCLUSIVE_LANGUAGE || cop == FILE_NAME {
            call["file"]
                .as_str()
                .unwrap_or("lib/a.rb")
                .trim_start_matches('/')
        } else {
            "lib/a.rb"
        };
        let path = dir.path().join(display);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(dir.path(), "", &["--only", cop, "--fix", display], None);
        let expected = call["positions"].as_array().unwrap();
        assert_eq!(
            run.status.code(),
            Some(i32::from(!expected.is_empty())),
            "{call}: {run:?}"
        );
        assert!(run.stderr.is_empty(), "{call}: {run:?}");
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(
            output.matches(&format!("({cop})")).count(),
            expected.len(),
            "{output}"
        );
        assert!(output.contains("0 corrections applied"));
        let severity = if cop.starts_with("Security/") {
            "E"
        } else {
            "C"
        };
        for (index, position) in expected.iter().enumerate() {
            let prefix = format!("{display}:{}:{}: {severity}: ", position[0], position[1]);
            if let Some(messages) = call["messages"].as_array() {
                assert!(
                    output.contains(&format!(
                        "{prefix}{} ({cop})",
                        messages[index].as_str().unwrap()
                    )),
                    "{output}"
                );
            } else {
                assert!(
                    output.lines().any(
                        |line| line.starts_with(&prefix) && line.ends_with(&format!("({cop})"))
                    ),
                    "{output}"
                );
            }
        }
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn ns11_four_actual_canonical_fixes_preserve_controlled_native_runtime_together() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let before = "class Box\n  def +(arg); arg + 1; end\nend\ndef sink; yield 4; end\ndef run(&callback)\n  sink(&callback)\nend\nbegin\n  raise 'controlled'\nrescue => error\n  caught = error.message\nend\ntext = <<~sql\nvalue\nsql\np [Box.new + 2, run { |value| value * 2 }, caught, text]\n";
    let after = before
        .replace("(arg)", "(other)")
        .replace("arg +", "other +")
        .replace("&callback", "&")
        .replace("=> error", "=> e")
        .replace("error.message", "e.message")
        .replace("sql", "SQL");
    fs::write(&path, before).unwrap();
    let observations: Vec<_> = ns04_runtimes()
        .into_iter()
        .map(|ruby| {
            let output = ns04_ruby(&ruby, &path).unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(output.stdout, b"[3, 8, \"controlled\", \"value\\n\"]\n");
            (ruby, output)
        })
        .collect();
    #[cfg(unix)]
    let (mut original, inode) = {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        let handle = fs::File::open(&path).unwrap();
        let inode = handle.metadata().unwrap().ino();
        (handle, inode)
    };
    let entries = fs::read_dir(path.parent().unwrap()).unwrap().count();
    let only = "Naming/BinaryOperatorParameterName,Naming/BlockForwarding,Naming/HeredocDelimiterCase,Naming/RescuedExceptionsVariableName";
    let run = cli(dir.path(), "", &["--only", only, "--fix", "lib/a.rb"], None);
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert!(
        String::from_utf8_lossy(&run.stdout)
            .contains("4 corrections applied, 0 offenses remaining"),
        "{run:?}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    assert_eq!(
        fs::read_dir(path.parent().unwrap()).unwrap().count(),
        entries
    );
    #[cfg(unix)]
    {
        use std::{
            io::Read,
            os::unix::fs::{MetadataExt, PermissionsExt},
        };
        assert_ne!(
            fs::metadata(&path).unwrap().ino(),
            inode,
            "atomic replacement"
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o750
        );
        let mut old_bytes = String::new();
        original.read_to_string(&mut old_bytes).unwrap();
        assert_eq!(
            old_bytes, before,
            "open descriptor still sees original inode"
        );
    }
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let stable = cli(dir.path(), "", &["--only", only, "--fix", "lib/a.rb"], None);
    assert_eq!(stable.status.code(), Some(0), "{stable:?}");
    assert!(String::from_utf8_lossy(&stable.stdout).contains("0 corrections applied"));
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    for (ruby, before) in observations {
        let after = ns04_ruby(&ruby, &path).unwrap();
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        assert_eq!(before.status.code(), after.status.code());
    }
}

#[test]
fn ns11_yaml_version_inactivity_survives_selection_config_and_source_opt_ins() {
    let dir = project();
    for config in [
        "",
        "[cops.Security]\nenabled = true\n",
        "[cops.\"Security/YAMLLoad\"]\nenabled = true\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for prefix in ["rubocop", "rustocop"] {
            let source =
                format!("# {prefix}:enable Security/YAMLLoad\nYAML.load(code)\neval(code)\n");
            for selection in ["Security/YAMLLoad", "Security"] {
                let run = cli(
                    dir.path(),
                    "",
                    &["--only", selection, "--stdin", "nonexistent.rb"],
                    Some(&source),
                );
                assert_eq!(
                    run.status.code(),
                    Some(i32::from(selection == "Security")),
                    "{run:?}"
                );
                assert!(run.stderr.is_empty(), "{run:?}");
                let output = String::from_utf8_lossy(&run.stdout);
                assert!(!output.contains("(Security/YAMLLoad)"), "{output}");
                assert_eq!(
                    output.contains("(Security/Eval)"),
                    selection == "Security",
                    "{output}"
                );
            }
        }
    }
}

#[test]
fn ns11_all_eleven_inherited_cops_source_examples_are_detection_only_and_native_syntax_valid() {
    // MIT-attributed, completeness-checked unchanged pinned documentation,
    // including alternate-style sources evaluated under our fixed policy.
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns11_existing_source.json")).unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["cop"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        11
    );
    assert_ns02_calls_preserve_bytes_and_mtime(&rows);
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for row in rows {
        let cop = row["cop"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let config = Default::default();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let report =
            rustocop::pipeline::lint_one(&path, &path, Some(source.as_bytes()), &active, &config);
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}"
        );
        let positions: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert!(!o.message.is_empty());
                vec![o.line, o.col]
            })
            .collect();
        assert_eq!(serde_json::json!(positions), row["positions"], "{row}");
        fs::write(&path, source).unwrap();
        for ruby in ns04_runtimes() {
            let result = Command::new(ruby).arg("-c").arg(&path).output().unwrap();
            assert!(result.status.success(), "{row}: {result:?}");
        }
    }
}

#[test]
fn ns02_legacy_positions_uppercase_and_native_syntax_witnesses() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (encoding, letter) in [
        ("UTF-8", "É".as_bytes()),
        ("ISO-8859-1", b"\xc9".as_slice()),
        ("Windows-1252", b"\xc9".as_slice()),
        ("Shift_JIS", b"\x82\x60".as_slice()),
        ("EUC-JP", b"\xa3\xc1".as_slice()),
    ] {
        // The first non-ASCII character is column 9 despite earlier multibyte
        // literal text; block names include a real legacy uppercase character.
        let mut source = format!("# encoding: {encoding}\r\n\"").into_bytes();
        source.extend_from_slice(letter);
        source.extend_from_slice(b"\"; foo");
        source.extend_from_slice(letter);
        source.extend_from_slice(b" = 1\r\nfoo { |a");
        source.extend_from_slice(letter);
        source.extend_from_slice(b"| }\r\nfoo_1 = 1\r\np __LINE__\r\n__END__\r\nraw data\r\n");
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        for ruby in std::iter::once(std::ffi::OsString::from("ruby"))
            .chain(crate::support::runtime::ruby_os())
        {
            let output = match Command::new(&ruby).arg("-c").arg(&path).output() {
                Ok(output) => output,
                Err(error) if ruby == "ruby" && error.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!("SKIP host Ruby syntax witness");
                    continue;
                }
                Err(error) => panic!("{ruby:?}: {error}"),
            };
            assert!(output.status.success(), "{encoding} {ruby:?}: {output:?}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("Syntax OK"));
        }
        for (cop, expected) in [
            ("Naming/AsciiIdentifiers", vec![(2, 9), (3, 9)]),
            ("Naming/BlockParameterName", vec![(3, 8)]),
            ("Naming/VariableNumber", vec![(4, 1)]),
        ] {
            let lint = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
            let fixed = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
            for run in [&lint, &fixed] {
                assert_eq!(run.status.code(), Some(1), "{encoding}: {run:?}");
                assert!(run.stderr.is_empty(), "{run:?}");
                let output = String::from_utf8_lossy(&run.stdout);
                assert_eq!(
                    output.matches(&format!("({cop})")).count(),
                    expected.len(),
                    "{output}"
                );
                for (line, col) in &expected {
                    assert!(
                        output.contains(&format!("lib/a.rb:{line}:{col}: C:")),
                        "{output}"
                    );
                }
            }
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
}

const BLOCK_FORWARDING: &str = "Naming/BlockForwarding";
const BLOCK_FORWARDING_BEFORE: &str = "def foo(&callback)\n  bar(&callback)\nend\n";
const NS05_AFTER: &str = "def foo(&)\n  bar(&)\nend\n";

#[test]
fn ns05_pinned_specs_corrections_and_source_run_through_real_cli() {
    // RuboCop MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns05_pinned.json")).unwrap();
    let docs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns05_source.json")).unwrap();
    let syntax: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns05_syntax.json")).unwrap();
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for call in fixture["calls"]
        .as_array()
        .unwrap()
        .iter()
        .chain(&docs)
        .chain(&syntax)
    {
        let source = call["source"].as_str().unwrap();
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", BLOCK_FORWARDING, "lib/a.rb"],
            None,
        );
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(
            output.matches(&format!("({BLOCK_FORWARDING})")).count(),
            call["positions"].as_array().unwrap().len(),
            "{call}: {run:?}"
        );
        for p in call["positions"].as_array().unwrap() {
            assert!(
                output.contains(&format!(
                    "lib/a.rb:{}:{}: C: Use anonymous block forwarding.",
                    p[0], p[1]
                )),
                "{call}: {run:?}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        if !call["spec_line"].is_null() {
            let withheld = [47, 109].contains(&call["spec_line"].as_u64().unwrap());
            let expected = if withheld {
                source
            } else {
                call["correction"].as_str().unwrap_or(source)
            };
            let run = cli(
                dir.path(),
                "",
                &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
                None,
            );
            assert_ne!(run.status.code(), Some(2), "{call}: {run:?}");
            assert_eq!(
                fs::read(&path).unwrap(),
                expected.as_bytes(),
                "{call}: {run:?}"
            );
            if source == expected {
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
        }
    }
}

#[test]
fn ns05_atomic_directives_stdin_path_and_no_write_controls() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable Naming\n{BLOCK_FORWARDING_BEFORE}"),
            format!(
                "def foo(&callback) # {prefix}:disable {BLOCK_FORWARDING}\n  bar(&callback)\nend\n"
            ),
            format!(
                "def foo(&callback)\n  bar(&callback) # {prefix}:disable {BLOCK_FORWARDING}\nend\n"
            ),
            format!(
                "def foo(&callback)\n  # {prefix}:disable-next {BLOCK_FORWARDING}\n  bar(&callback)\nend\n"
            ),
        ] {
            fs::write(&path, &source).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            let run = cli(
                dir.path(),
                "",
                &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
                None,
            );
            assert_ne!(run.status.code(), Some(2), "{run:?}");
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{run:?}");
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{BLOCK_FORWARDING}\"]\nenabled=false\n"),
        )
        .unwrap();
        fs::write(
            &path,
            format!("# {prefix}:enable {BLOCK_FORWARDING}\n{BLOCK_FORWARDING_BEFORE}"),
        )
        .unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", "Naming", "--fix", "lib/a.rb"],
            None,
        );
        assert_ne!(run.status.code(), Some(2), "{run:?}");
        assert!(fs::read_to_string(&path).unwrap().contains("def foo(&)"));
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    fs::write(&path, BLOCK_FORWARDING_BEFORE).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let run = cli(
        dir.path(),
        "lib",
        &["--only", BLOCK_FORWARDING, "--stdin", "a.rb"],
        Some(BLOCK_FORWARDING_BEFORE),
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("a.rb:1:9:"),
        "{run:?}"
    );
    rejecting_stdin(
        dir.path(),
        &["--only", BLOCK_FORWARDING, "--fix", "--stdin", "lib/a.rb"],
        BLOCK_FORWARDING_BEFORE,
    );
    assert_eq!(fs::read(&path).unwrap(), BLOCK_FORWARDING_BEFORE.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

#[test]
fn ns05_source_comments_encoding_bom_crlf_line_and_data_envelopes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (before, after) in [
        (BLOCK_FORWARDING_BEFORE.to_owned(), NS05_AFTER.to_owned()),
        (
            "def foo(&café)\n # café &café stays\n bar(__LINE__, 'café', &café)\nend\n".into(),
            "def foo(&)\n # café &café stays\n bar(__LINE__, 'café', &)\nend\n".into(),
        ),
        (
            "def foo(arg = <<~TXT, &callback)\n&callback literal\nTXT\n bar(arg, &callback)\nend\n"
                .into(),
            "def foo(arg = <<~TXT, &)\n&callback literal\nTXT\n bar(arg, &)\nend\n".into(),
        ),
    ] {
        for (prefix, newline) in [
            ("", "\n"),
            ("\u{feff}", "\r\n"),
            ("# encoding: UTF-8\n", "\r\n"),
        ] {
            let source =
                format!("{prefix}{before}__END__\nraw &callback café\n").replace('\n', newline);
            let expected =
                format!("{prefix}{after}__END__\nraw &callback café\n").replace('\n', newline);
            fs::write(&path, &source).unwrap();
            let run = cli(
                dir.path(),
                "",
                &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
                None,
            );
            assert_eq!(run.status.code(), Some(0), "{source}: {run:?}");
            assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
        }
    }
    for (source, expected) in [
        (
            b"# encoding: ISO-8859-1\nx = '\xe9'\ndef foo(&callback); bar(&callback); end\n"
                .as_slice(),
            b"# encoding: ISO-8859-1\nx = '\xe9'\ndef foo(&); bar(&); end\n".as_slice(),
        ),
        (
            b"# encoding: ISO-8859-1\ndef foo(&caf\xe9); bar(&caf\xe9); end\n".as_slice(),
            b"# encoding: ISO-8859-1\ndef foo(&); bar(&); end\n".as_slice(),
        ),
        (
            b"# encoding: ISO-8859-1\ndef foo(&callback); text = '\xe9'; bar(&callback); end\n"
                .as_slice(),
            b"# encoding: ISO-8859-1\ndef foo(&callback); text = '\xe9'; bar(&callback); end\n"
                .as_slice(),
        ),
    ] {
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
            None,
        );
        assert_ne!(run.status.code(), Some(2), "{run:?}");
        assert_eq!(fs::read(&path).unwrap(), expected);
        if source == expected {
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
    for observer in [
        "p DATA.pos",
        "p File.read(__FILE__)",
        "p RubyVM::InstructionSequence",
        "p method(:foo).parameters",
        "callback.call",
    ] {
        let source = format!("{BLOCK_FORWARDING_BEFORE}{observer}\n__END__\nraw\n");
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(1), "{source}: {run:?}");
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn ns05_each_of_the_five_pinned_incompatible_peers_withholds_only_edits() {
    use rustocop::cops::Cop;
    let peers = rustocop::cops::naming::BlockForwarding.autocorrect_incompatible_with();
    assert_eq!(
        peers,
        [
            "Lint/AmbiguousOperator",
            "Style/ArgumentsForwarding",
            "Style/EndlessMethod",
            "Style/ExplicitBlockArgument",
            "Style/MethodDefParentheses"
        ]
    );
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for peer in peers {
        for only in [
            format!("{BLOCK_FORWARDING},{peer}"),
            format!("{peer},{BLOCK_FORWARDING}"),
        ] {
            fs::write(&path, BLOCK_FORWARDING_BEFORE).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            let run = cli(
                dir.path(),
                "",
                &["--only", &only, "--fix", "lib/a.rb"],
                None,
            );
            assert_eq!(run.status.code(), Some(1), "{only}: {run:?}");
            assert!(
                String::from_utf8_lossy(&run.stdout).contains(BLOCK_FORWARDING),
                "{run:?}"
            );
            assert_eq!(fs::read(&path).unwrap(), BLOCK_FORWARDING_BEFORE.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
}

#[test]
fn ns05_style_peers_keep_diagnostics_and_compose_in_bounded_explicit_passes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for peers in [
        "Naming/BlockForwarding,Style/ArgumentsForwarding",
        "Style/ArgumentsForwarding,Naming/BlockForwarding",
    ] {
        let before = "def foo(*args, **kwargs, &block)\n  bar(*args, **kwargs, &block)\nend\n";
        fs::write(&path, before).unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", peers, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "def foo(...)\n  bar(...)\nend\n"
        );
    }
    let peers = "Naming/BlockForwarding,Style/MethodDefParentheses,Layout";
    fs::write(&path, "def foo &callback\n  bar &callback  \nend\n").unwrap();
    cli(
        dir.path(),
        "",
        &["--only", peers, "--fix", "lib/a.rb"],
        None,
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def foo(&callback)\n  bar &callback\nend\n"
    );
    cli(
        dir.path(),
        "",
        &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
        None,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), NS05_AFTER);
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    cli(
        dir.path(),
        "",
        &["--only", peers, "--fix", "lib/a.rb"],
        None,
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

#[test]
fn ns05_native_ruby34_runtime_forwarding_evaluation_identity_control_flow_and_lines() {
    let Some(ruby) = crate::support::runtime::ruby_os() else {
        eprintln!("Set RUSTOCOP_RUBY34 for native anonymous forwarding evidence");
        return;
    };
    let version = Command::new(&ruby)
        .args([
            "-e",
            crate::support::runtime::script(
                "abort RUBY_VERSION unless RUBY_VERSION.start_with?('3.4.')",
            )
            .as_str(),
        ])
        .output()
        .unwrap();
    assert!(version.status.success(), "{version:?}");
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let prefix = r#"$events=[]
def sink(*args, **kw, &cb)
 $events << [args, kw, cb.equal?($cb)]
 value = cb ? cb.call(:called) : :nil_block
 raise 'controlled' if kw[:fail]
 value
end
def mark(x); $events << x; x; end
"#;
    let source = r#"def foo(arg = mark(:default), **kw, &callback)
 begin
  result = sink(mark(arg), __LINE__, **kw, &callback)
  [result, :value]
 rescue RuntimeError => error
  error.message
 ensure
  $events << :ensure
 end
end
def nested(&callback)
 [1,2].map { sink(it, &callback) }
end
def numbered(&callback)
 [3].map { sink(_1, &callback) }
end
def closure(&callback)
 -> { sink(&callback) }.call
end
def heredoc(&callback)
 <<~TEXT
#{sink(&callback)} #{__LINE__}
TEXT
end
def nested_definition(&callback)
 def inner(&callback); sink(&callback); end
end
def unused(&callback); :unused; end
def yielded(&callback); yield; end
class Parent
 def run(&cb); sink(&cb); end
end
class Child < Parent
 def run(&callback); super(&callback); end
end
"#;
    let suffix = r#"$cb=->(x=:default){ $events << x; :block_value }
p foo
p foo(:explicit, fail: true, &$cb)
p nested(&$cb)
p numbered(&$cb)
p closure(&$cb)
p heredoc(&$cb)
nested_definition(&$cb)
p inner(&$cb)
p unused(&$cb)
p yielded(&$cb)
p Child.new.run(&$cb)
p $events
"#;
    fs::write(&path, format!("{prefix}{source}{suffix}")).unwrap();
    let before = ns04_ruby(&ruby, &path).unwrap();
    assert!(before.status.success(), "{before:?}");
    // Keep the controlled external Proc harness outside the correction source.
    fs::write(&path, source).unwrap();
    let run = cli(
        dir.path(),
        "",
        &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
        None,
    );
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    let corrected = fs::read_to_string(&path).unwrap();
    assert!(!corrected.contains("&callback"), "{corrected}");
    fs::write(&path, format!("{prefix}{corrected}{suffix}")).unwrap();
    let after = ns04_ruby(&ruby, &path).unwrap();
    assert!(after.status.success(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}

#[test]
fn ns05_native_counterexamples_justify_reflection_and_channel_withholds() {
    let Some(ruby) = crate::support::runtime::ruby_os() else {
        return;
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, naive) in [
        (
            "def foo(&callback); p local_variables; end; foo {}\n",
            "def foo(&); p local_variables; end; foo {}\n",
        ),
        (
            "def foo(&callback); end; p method(:foo).parameters\n",
            "def foo(&); end; p method(:foo).parameters\n",
        ),
        (
            "def foo(&callback); p eval('defined?(callback)'); end; foo {}\n",
            "def foo(&); p eval('defined?(callback)'); end; foo {}\n",
        ),
        (
            "def foo(&callback); end; p DATA.pos\n__END__\nraw\n",
            "def foo(&); end; p DATA.pos\n__END__\nraw\n",
        ),
        (
            "def foo(&callback); end; p File.binread(__FILE__)\n",
            "def foo(&); end; p File.binread(__FILE__)\n",
        ),
        (
            "def sink; yield; end; def foo(&callback); [1].each { |&| sink(&callback) }; end; foo { p :outer }\n",
            "def sink; yield; end; def foo(&); [1].each { |&| sink(&) }; end; foo { p :outer }\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let before = ns04_ruby(&ruby, &path).unwrap();
        assert!(before.status.success(), "{before:?}");
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(1), "{source}: {run:?}");
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        fs::write(&path, naive).unwrap();
        let changed = ns04_ruby(&ruby, &path).unwrap();
        assert!(
            before.stdout != changed.stdout || before.status.code() != changed.status.code(),
            "counterexample must differ: {source}"
        );
    }
}

#[test]
fn ns05_native_syntactic_matches_never_authorize_unrelated_or_stranded_reads() {
    let Some(ruby) = crate::support::runtime::ruby_os() else {
        return;
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, prefix, suffix, naive) in [
        (
            "def foo(&callback); xs.each { |callback| bar(&callback) }; end",
            "def xs; [-> { :shadowed }]; end\ndef bar(&cb); p cb.call; end\n",
            "\nfoo { :outer }\n",
            "def foo(&); xs.each { |callback| bar(&) }; end",
        ),
        (
            "def foo(&callback); xs.each { |;callback| bar(&callback) }; end",
            "def xs; [1]; end\ndef bar(&cb); p(cb ? cb.call : :none); end\n",
            "\nfoo { :outer }\n",
            "def foo(&); xs.each { |;callback| bar(&) }; end",
        ),
        (
            "def outer(&callback); def inner(&other); foo(&callback); end; bar(&callback); end",
            "def callback; -> { :unrelated }; end\ndef foo(&cb); p cb.call; end\ndef bar(&cb); p cb.call; end\n",
            "\nouter { :outer }; inner { :inner }\n",
            "def outer(&); def inner(&other); foo(&); end; bar(&); end",
        ),
        (
            "def foo(&callback); baz(& callback); end",
            "def callback; -> { :fallback }; end\ndef baz(&cb); p cb.call; end\n",
            "\nfoo { :outer }\n",
            "def foo(&); baz(& callback); end",
        ),
        (
            "def foo(&callback); baz(&\n callback); end",
            "def callback; -> { :fallback }; end\ndef baz(&cb); p cb.call; end\n",
            "\nfoo { :outer }\n",
            "def foo(&); baz(&\n callback); end",
        ),
        (
            "def foo(& callback); callback; end",
            "def callback; -> { :fallback }; end\n",
            "\np foo { :outer }.call\n",
            "def foo(&); callback; end",
        ),
    ] {
        fs::write(&path, format!("{prefix}{source}{suffix}")).unwrap();
        let before = ns04_ruby(&ruby, &path).unwrap();
        assert!(before.status.success(), "{source}: {before:?}");
        // No harness observers/converters are present in the correction input.
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(1), "{source}: {run:?}");
        assert!(
            String::from_utf8_lossy(&run.stdout).contains(BLOCK_FORWARDING),
            "{run:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        fs::write(&path, format!("{prefix}{naive}{suffix}")).unwrap();
        let changed = ns04_ruby(&ruby, &path).unwrap();
        assert!(
            before.stdout != changed.stdout || before.status.code() != changed.status.code(),
            "must witness unsafe identity/spelling edit: {source}"
        );
    }
}

#[test]
fn ns05_native_external_conversion_witnesses_the_two_pinned_correction_holds() {
    let Some(ruby) = crate::support::runtime::ruby_os() else {
        return;
    };
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns05_pinned.json")).unwrap();
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (line, prefix, suffix) in [
        (
            47,
            "class Symbol; def to_proc; p method(:foo).parameters; ->(*) {}; end; end\ndef bar(&cb); cb.call; end\n",
            "foo {}\n",
        ),
        (
            109,
            "$delegate=Object.new\ndef $delegate.foo(&cb); [1]; end\ndef delegator; $delegate; end\nother=Object.new\ndef other.to_proc; p method(:foo).parameters; ->(*) {}; end\n",
            "foo(other) {}\n",
        ),
    ] {
        let source = fixture["calls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["spec_line"] == line)
            .unwrap()["source"]
            .as_str()
            .unwrap();
        fs::write(&path, format!("{prefix}{source}{suffix}")).unwrap();
        let before = ns04_ruby(&ruby, &path).unwrap();
        assert!(before.status.success(), "{before:?}");
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", BLOCK_FORWARDING, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(1), "{run:?}");
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        fs::write(
            &path,
            format!("{prefix}{}{suffix}", source.replace("&block", "&")),
        )
        .unwrap();
        let changed = ns04_ruby(&ruby, &path).unwrap();
        assert!(changed.status.success(), "{changed:?}");
        assert_ne!(
            before.stdout, changed.stdout,
            "pinned correction at {line} must actually change observed metadata"
        );
    }
}

const NS06_CASE: &str = "Naming/HeredocDelimiterCase";
const NS06_NAMING: &str = "Naming/HeredocDelimiterNaming";

#[test]
fn ns06_pinned_calls_source_and_syntax_matrix_run_through_cli() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ns06_pinned.json")).unwrap();
    let source: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns06_source.json")).unwrap();
    let syntax: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/ns06_syntax.json")).unwrap();
    for row in fixture["calls"]
        .as_array()
        .unwrap()
        .iter()
        .chain(&source)
        .chain(&syntax)
    {
        let cop = row["cop"].as_str().unwrap();
        let before = row["source"].as_str().unwrap();
        fs::write(&path, before).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(dir.path(), "", &["--only", cop, "lib/a.rb"], None);
        let positions = row["positions"].as_array().unwrap();
        assert_eq!(
            run.status.code(),
            Some(i32::from(!positions.is_empty())),
            "{row}: {run:?}"
        );
        let output = String::from_utf8_lossy(&run.stdout);
        assert_eq!(
            output.matches(&format!("({cop})")).count(),
            positions.len(),
            "{row}: {output}"
        );
        for (position, message) in positions.iter().zip(row["messages"].as_array().unwrap()) {
            assert!(
                output.contains(&format!(
                    "lib/a.rb:{}:{}: C: {} ({cop})",
                    position[0],
                    position[1],
                    message.as_str().unwrap()
                )),
                "{output}"
            );
        }
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        if let Some(after) = row["correction"].as_str() {
            let run = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
            assert_eq!(run.status.code(), Some(0), "{row}: {run:?}");
            assert_eq!(fs::read_to_string(&path).unwrap(), after);
        } else if cop == NS06_NAMING {
            let run = cli(dir.path(), "", &["--only", cop, "--fix", "lib/a.rb"], None);
            assert_ne!(run.status.code(), Some(2), "{run:?}");
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
}

#[test]
fn ns06_atomic_disabled_anchors_comments_stdin_provenance_and_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable Naming\np <<~sql\nvalue\nsql\n"),
            format!("# {prefix}:disable-next {NS06_CASE}\np <<~sql\nvalue\nsql\n"),
            format!("p <<~sql # {prefix}:disable {NS06_CASE}\nvalue\nsql\n"),
            format!("p <<~sql\nvalue\nsql\n# {prefix}:disable {NS06_CASE}\n"),
        ] {
            fs::write(&path, &source).unwrap();
            let run = cli(
                dir.path(),
                "",
                &["--only", NS06_CASE, "--fix", "lib/a.rb"],
                None,
            );
            // A header-only disable withholds the whole atomic rename even
            // though the terminator finding survives; a later disable cannot.
            let header_only = source.starts_with("p <<~sql #");
            assert_eq!(run.status.code(), Some(i32::from(header_only)), "{run:?}");
            let expected = if source.ends_with(&format!("# {prefix}:disable {NS06_CASE}\n")) {
                source
                    .replacen("<<~sql", "<<~SQL", 1)
                    .replacen("\nsql\n", "\nSQL\n", 1)
            } else {
                source.clone()
            };
            assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        }
        let source = format!(
            "# {prefix}:disable {NS06_CASE}\np <<~sql\nvalue\nsql\n# {prefix}:enable {NS06_CASE}\np <<~html\nvalue\nhtml\n"
        );
        fs::write(&path, &source).unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", NS06_CASE, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source.replace("html", "HTML")
        );
        for cop in [NS06_CASE, NS06_NAMING] {
            let label = if cop == NS06_CASE { "sql" } else { "END" };
            let source = format!("# {prefix}:enable {cop}\np <<~{label}\nvalue\n{label}\n");
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.Naming]\nenabled=false\n[cops.\"{cop}\"]\nenabled=false\n"),
            )
            .unwrap();
            let run = cli(dir.path(), "lib", &["--stdin", "a.rb"], Some(&source));
            assert!(
                String::from_utf8_lossy(&run.stdout).contains("lib/a.rb:4:1: C:"),
                "{run:?}"
            );
            let run = cli(
                dir.path(),
                "",
                &["--except", cop, "--stdin", "lib/a.rb"],
                Some(&source),
            );
            assert!(
                !String::from_utf8_lossy(&run.stdout).contains(cop),
                "{run:?}"
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    let source = "p <<~sql\nvalue\nsql\n";
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let run = cli(
        dir.path(),
        "lib",
        &["--only", NS06_CASE, "--stdin", "a.rb"],
        Some(source),
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("lib/a.rb:3:1"),
        "{run:?}"
    );
    rejecting_stdin(
        dir.path(),
        &["--only", NS06_CASE, "--fix", "--stdin", "lib/a.rb"],
        source,
    );
    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

#[test]
fn ns06_live_default_renaming_does_not_weaken_prior_body_to_terminator_guards() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, expected) in [
        // Once no peer writes the body (third pass), default-on
        // HeredocIndentation turns the plain opener into `<<~` (zero dedent;
        // the renamed label cannot match the body line).
        (
            "a = 1\nb = 2\np <<\"#{a + b}\"\n#{a+b}\n#{a + b}\n",
            "a = 1\nb = 2\np <<~\"#{A + B}\"\n#{a + b}\n#{A + B}\n",
        ),
        (
            "@x = 3\np <<-\"#{@x}\"\n\u{b}#@x\n#{@x}\n",
            "@x = 3\np <<-\"#{@X}\"\n\u{b}#@x\n#{@X}\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let ruby = crate::support::runtime::ruby_os();
        let before = ruby.as_ref().map(|r| ns04_ruby(r, &path).unwrap());
        for _ in 0..3 {
            let run = cli(
                dir.path(),
                "",
                &["--fix", "--fix-passes", "1", "lib/a.rb"],
                None,
            );
            assert_ne!(run.status.code(), Some(2), "{run:?}");
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        if let Some(before) = before {
            let after = ns04_ruby(ruby.as_ref().unwrap(), &path).unwrap();
            assert!(
                before.status.success() && after.status.success(),
                "{before:?} {after:?}"
            );
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
        }
    }
}

#[test]
fn ns06_legacy_label_bytes_ascii_only_renames_and_non_utf8_envelope_holds() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (encoding, lower, upper) in [
        ("ISO-8859-1", b"\xe9".as_slice(), b"\xc9".as_slice()),
        ("Windows-31J", b"\x83\x60", b"\x83\x60"), // trailing backtick is not a quote
        ("CP932", b"\xfa\x40", b"\xfa\x4a"),       // alternate uppercase Roman-numeral bytes
        ("EUC-JP", b"\xa4\xa2", b"\xa4\xa2"),
        ("Big5", b"\xa4\xa4", b"\xa4\xa4"),
        ("GBK", b"\xd6\xd0", b"\xd6\xd0"),
    ] {
        for collision in [false, true] {
            let mut source = format!("# coding: {encoding}\np <<~'").into_bytes();
            source.extend_from_slice(lower);
            source.extend_from_slice(b"sql'\n");
            if collision {
                source.extend_from_slice(upper);
                source.extend_from_slice(b"SQL\n");
            } else {
                source.extend_from_slice(b"value\n");
            }
            source.extend_from_slice(lower);
            source.extend_from_slice(b"sql\n");
            fs::write(&path, &source).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            let run = cli(
                dir.path(),
                "",
                &["--only", NS06_CASE, "--fix", "lib/a.rb"],
                None,
            );
            assert_eq!(
                run.status.code(),
                Some(i32::from(!collision)),
                "{encoding} {collision}: {run:?}"
            );
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
        // A raw ASCII-looking multibyte trail does not satisfy Ruby /\\w/.
        let mut source = format!("# coding: {encoding}\np <<~'").into_bytes();
        source.extend_from_slice(lower);
        source.extend_from_slice(b"'\nvalue\n");
        source.extend_from_slice(lower);
        source.push(b'\n');
        fs::write(&path, &source).unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", NS06_NAMING, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(1), "{encoding}: {run:?}");
        assert!(String::from_utf8_lossy(&run.stdout).contains("lib/a.rb:4:1"));
        assert_eq!(fs::read(&path).unwrap(), source);
        let before = format!("# coding: {encoding}\np <<~sql\nASCII\nsql\n");
        fs::write(&path, &before).unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", NS06_CASE, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before.replace("sql", "SQL")
        );
    }
    for (encoding, raw) in [
        ("SJIS-DoCoMo", b"\xf9\x87"),
        ("SJIS-KDDI", b"\xf6\xfb"),
        ("SJIS-SoftBank", b"\xf2\xe5"),
    ] {
        let mut source = format!("# coding: {encoding}\np <<~'").into_bytes();
        source.extend_from_slice(raw);
        source.extend_from_slice(b"'\ntext\n");
        source.extend_from_slice(raw);
        source.push(b'\n');
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let run = cli(
            dir.path(),
            "",
            &["--only", NS06_NAMING, "--fix", "lib/a.rb"],
            None,
        );
        assert_eq!(run.status.code(), Some(0), "{encoding}: {run:?}");
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
    // Body bytes cannot be copied through a lossy UTF-8 String replacement.
    let source = b"# coding: ISO-8859-1\np((<<~sql).bytes)\ncaf\xe9\nsql\n";
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let run = cli(
        dir.path(),
        "",
        &["--only", NS06_CASE, "--fix", "lib/a.rb"],
        None,
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    // Host Ruby has the installed legacy encoding tables; miniruby does not.
    if let Ok(before) = Command::new("ruby")
        .arg("--disable-gems")
        .arg(&path)
        .output()
    {
        assert!(before.status.success(), "{before:?}");
        fs::write(&path, String::from_utf8_lossy(source).replace("sql", "SQL")).unwrap();
        let after = Command::new("ruby")
            .arg("--disable-gems")
            .arg(&path)
            .output()
            .unwrap();
        assert!(after.status.success(), "{after:?}");
        assert_ne!(before.stdout, after.stdout);
    }
}

#[test]
fn unknown_cops_and_style_or_version_parameters_remain_errors() {
    let dir = project();
    for config in [
        "TargetRubyVersion = \"4.0\"\n",
        "[cops.Style]\nexclude = []\n",
        "[cops.\"Style/StringLiterals\"]\nEnforcedStyle = \"single_quotes\"\n",
        "[cops.\"Style/StringLiterals\"]\ninclude = [\"lib/**\"]\n",
        "[cops.\"Style/NotImplemented\"]\nenabled = true\n",
        "[cops.\"Naming/HeredocDelimiterCase\"]\nEnforcedStyle = \"lowercase\"\n",
        "[cops.\"Naming/HeredocDelimiterNaming\"]\nForbiddenDelimiters = []\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let run = rejecting_stdin(dir.path(), &["--stdin", "x.rb"], "puts 1\n");
        assert_eq!(run.status.code(), Some(2), "{config}: {run:?}");
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let run = rejecting_stdin(
        dir.path(),
        &["--only", "Style/NotImplemented", "--stdin", "x.rb"],
        "puts 1\n",
    );
    assert_eq!(run.status.code(), Some(2), "{run:?}");
}
