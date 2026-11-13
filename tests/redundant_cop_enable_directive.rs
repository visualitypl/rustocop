//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, lint::RedundantCopEnableDirective},
    pipeline::lint_one,
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;
const COP: &str = "Lint/RedundantCopEnableDirective";
const BEFORE: &str = "foo\n# rubocop:enable Layout/LineLength\n";
const STDIN_FIX_ERROR: &str = concat!(
    "error: the argument '--stdin <PATH>' cannot be used with '--fix'\n\n",
    "Usage: rustocop --only <ONLY> --stdin <PATH> [PATHS]...\n\n",
    "For more information, try '--help'.\n"
);
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], code: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    out
}
fn fix(root: &Path, before: &[u8], after: &[u8]) {
    let path = root.join("lib/a.rb");
    fs::write(&path, before).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(["--only", COP, "--fix", "lib/a.rb"])
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(2), "{before:?}: {out:?}");
    assert_eq!(fs::read(&path).unwrap(), after, "{before:?}: {out:?}");
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.permissions(), new.permissions());
    if before == after {
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    }
}
#[test]
fn all_28_pinned_calls_and_24_correction_dispositions_including_real_config() {
    let rows: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/redundant_cop_enable_directive_pinned.json"
    ))
    .unwrap();
    let d = project();
    let mut corrections = 0;
    for row in rows.as_array().unwrap() {
        let source = row["source"].as_str().unwrap();
        let disabled = row["disabled"] == true;
        // RuboCop's examples name Metrics cops, which are on there and
        // opt-in here: switch the department on, as the examples assume.
        fs::write(
            d.path().join("rustocop.toml"),
            if disabled {
                "[cops.Metrics]\nenabled = true\n[cops.\"Layout/LineLength\"]\nenabled = false\n"
            } else {
                "[cops.Metrics]\nenabled = true\n"
            },
        )
        .unwrap();
        // Ordinary pipeline diagnostics, with configured-off lifecycle cases
        // additionally checked through the real CLI's full configured universe.
        let mut config = ResolvedConfig::default();
        config.cop_settings.insert(
            "Metrics".into(),
            rustocop::config::CopSettings {
                enabled: Some(true),
                ..Default::default()
            },
        );
        if disabled {
            config.cop_settings.insert(
                "Layout/LineLength".into(),
                rustocop::config::CopSettings {
                    enabled: Some(false),
                    ..Default::default()
                },
            );
        }
        let report = lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(source.as_bytes()),
            &[&RedundantCopEnableDirective],
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row:?}"
        );
        {
            let actual: Vec<_> = report
                .result
                .offenses
                .iter()
                .map(|o| (o.line, o.col, o.message.as_str()))
                .collect();
            let expected: Vec<_> = row["positions"]
                .as_array()
                .unwrap()
                .iter()
                .zip(row["messages"].as_array().unwrap())
                .map(|(p, m)| {
                    (
                        p[0].as_u64().unwrap() as u32,
                        p[1].as_u64().unwrap() as u32,
                        m.as_str().unwrap(),
                    )
                })
                .collect();
            assert_eq!(actual, expected, "{row:?}");
        }
        fs::write(d.path().join("lib/a.rb"), source).unwrap();
        let out = cli(
            d.path(),
            &["--only", COP, "lib/a.rb"],
            if row["positions"].as_array().unwrap().is_empty() {
                0
            } else {
                1
            },
        );
        let output = String::from_utf8_lossy(&out.stdout);
        for (pos, msg) in row["positions"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["messages"].as_array().unwrap())
        {
            assert!(
                output.contains(&format!(
                    "lib/a.rb:{}:{}: E: {}",
                    pos[0],
                    pos[1],
                    msg.as_str().unwrap()
                )),
                "{output}"
            );
        }
        let mut after = row["correction"].as_str().unwrap_or(source).to_owned();
        if row["correction"].is_string() {
            corrections += 1;
        }
        match row["spec_line"].as_u64().unwrap() {
            // Necessary partial-department operation; same-line prior enable
            // not separately proven, so conservatively hold this one edit.
            364 | 379 => after = source.into(),
            // Keep subsequent executable/backtrace line positions unchanged.
            66 | 115 | 308 | 340 => {
                let lines: Vec<_> = row["positions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p[0].as_u64().unwrap() as usize)
                    .collect();
                after = source
                    .lines()
                    .enumerate()
                    .map(|(i, line)| {
                        if lines.contains(&(i + 1)) {
                            "\n".into()
                        } else {
                            format!("{line}\n")
                        }
                    })
                    .collect();
            }
            260 => after = "foo\n\n\n".into(),
            _ => {}
        }
        fix(d.path(), source.as_bytes(), after.as_bytes());
    }
    assert_eq!(corrections, 24);
}
#[test]
fn all_six_pinned_source_examples_use_actual_comment_lifecycle() {
    let rows: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/redundant_cop_enable_directive_source.json"
    ))
    .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 6);
    for row in rows.as_array().unwrap() {
        let source = row["source"].as_str().unwrap();
        let report = lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(source.as_bytes()),
            &[&RedundantCopEnableDirective],
            &ResolvedConfig::default(),
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        let actual: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| (o.line, o.col, o.message.as_str()))
            .collect();
        let expected: Vec<_> = row["positions"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["messages"].as_array().unwrap())
            .map(|(p, m)| {
                (
                    p[0].as_u64().unwrap() as u32,
                    p[1].as_u64().unwrap() as u32,
                    m.as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(actual, expected, "{row:?}");
    }
}
#[test]
fn defaults_selectors_strict_toml_and_supplied_stdin_path() {
    let r = Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled() && c.safe_fixes());
    assert!(c.path_policy().is_none() && c.max_setting().is_none());
    assert_eq!(c.ruby_versions(), Default::default());
    let d = project();
    let p = d.path().join("lib/a.rb");
    fs::write(&p, BEFORE).unwrap();
    let old = fs::metadata(&p).unwrap();
    assert!(String::from_utf8_lossy(&cli(d.path(), &["lib/a.rb"], 1).stdout).contains(COP));
    for (config, args, code) in [
        (
            "",
            vec!["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
            0,
        ),
        (
            "[cops.Lint]\nenabled = false\n",
            vec!["--only", COP, "lib/a.rb"],
            1,
        ),
        (
            "[cops.Lint]\nenabled = false\n[cops.\"Lint/RedundantCopEnableDirective\"]\nenabled = true\n",
            vec!["lib/a.rb"],
            1,
        ),
        (
            "[cops.Lint]\nenabled = true\n[cops.\"Lint/RedundantCopEnableDirective\"]\nenabled = false\n",
            vec!["lib/a.rb"],
            0,
        ),
        (
            "[cops.\"Lint/RedundantCopEnableDirective\"]\nexclude = [\"lib/**\"]\n",
            vec!["--only", COP, "--fix", "lib/a.rb"],
            0,
        ),
        (
            "[cops.\"Lint/RedundantCopEnableDirective\"]\nmax = 3\n",
            vec!["--only", COP, "lib/a.rb"],
            2,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let out = cli(d.path(), &args, code);
        if args == ["lib/a.rb"] && code == 0 {
            assert!(!String::from_utf8_lossy(&out.stdout).contains(COP));
        }
        assert_eq!(fs::read(&p).unwrap(), BEFORE.as_bytes());
        assert_eq!(
            fs::metadata(&p).unwrap().modified().unwrap(),
            old.modified().unwrap()
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for (args, code) in [
        (vec!["--only", COP, "--stdin", "virtual/custom.rb"], 1),
        (vec!["--only", COP, "--stdin", "lib/a.rb", "--fix"], 2),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Only the asserted `--stdin --fix` rejection (exit 2) may close stdin
        // before reading; accepted stdin writes must succeed, with even
        // BrokenPipe remaining fatal.
        stdin_support::write_stdin(child.stdin.take().unwrap(), BEFORE.as_bytes(), code == 2)
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(code), "{out:?}");
        if code == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/custom.rb:2:18"));
            assert!(out.stderr.is_empty(), "{out:?}");
            assert!(!d.path().join("virtual/custom.rb").exists());
        } else {
            assert!(out.stdout.is_empty(), "{out:?}");
            assert_eq!(String::from_utf8(out.stderr).unwrap(), STDIN_FIX_ERROR);
        }
        assert_eq!(fs::read(&p).unwrap(), BEFORE.as_bytes());
        let new = fs::metadata(&p).unwrap();
        assert_eq!(new.modified().unwrap(), old.modified().unwrap());
        assert_eq!(new.permissions(), old.permissions());
    }
}
#[test]
fn stdin_fix_rejection_handles_a_deterministically_closed_pipe() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, BEFORE).unwrap();
    let old = fs::metadata(&path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "lib/a.rb", "--fix"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // Waiting with our pipe still open proves rejection needs neither input nor
    // EOF, and guarantees the later write hits a closed reader (no sleeps).
    let out = child.wait_with_output().unwrap();
    let write = stdin.write_all(BEFORE.as_bytes());
    assert_eq!(
        write.as_ref().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    stdin_support::write_stdin(stdin, BEFORE.as_bytes(), true).unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert_eq!(String::from_utf8(out.stderr).unwrap(), STDIN_FIX_ERROR);
    assert_eq!(fs::read(&path).unwrap(), BEFORE.as_bytes());
    let new = fs::metadata(&path).unwrap();
    assert_eq!(new.modified().unwrap(), old.modified().unwrap());
    assert_eq!(new.permissions(), old.permissions());
}
#[test]
fn shared_stdin_guard_preserves_other_errors_and_success() {
    /// Fails every write with one error kind, as a closed or broken stdin would.
    struct Failing(io::ErrorKind);
    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "stdin control"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut delivered = Vec::new();
    stdin_support::write_stdin(&mut delivered, BEFORE.as_bytes(), true).unwrap();
    assert_eq!(delivered, BEFORE.as_bytes());
    stdin_support::write_stdin(Failing(io::ErrorKind::BrokenPipe), b"x", true).unwrap();
    let err =
        stdin_support::write_stdin(Failing(io::ErrorKind::BrokenPipe), b"x", false).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    for kind in [
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::WriteZero,
        io::ErrorKind::Other,
    ] {
        let err = stdin_support::write_stdin(Failing(kind), b"x", true).unwrap_err();
        assert_eq!(err.kind(), kind);
        assert_eq!(err.to_string(), "stdin control");
    }
}
#[test]
fn configured_off_and_opt_in_enables_remain_necessary_even_when_unselected_or_clean() {
    let d = project();
    for target in [
        "Layout/LineLength",
        "Style/FrozenStringLiteralComment",
        "Style/IpAddresses",
        COP,
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{target}\"]\nenabled = false\n"),
        )
        .unwrap();
        for prefix in ["rubocop", "rustocop"] {
            let source = format!("# {prefix}:enable {target}\n0\n");
            fix(d.path(), source.as_bytes(), source.as_bytes());
            cli(d.path(), &["--only", COP, "lib/a.rb"], 0);
            let duplicate = format!("{source}# {prefix}:enable {target}\n");
            fix(d.path(), duplicate.as_bytes(), duplicate.as_bytes());
            cli(d.path(), &["--only", COP, "lib/a.rb"], 1);
        }
    }
    // A later opt-in mobilizes the cop throughout the file: an earlier all
    // can close its initial disabled range, despite the cop being unselected.
    let source = "# rubocop:enable all\n0\n# rubocop:enable-next Style/IpAddresses\n0\n";
    fix(d.path(), source.as_bytes(), source.as_bytes());
}
#[test]
fn inactive_cop_pairing_uses_configuration_before_cli_and_version_gates() {
    let d = project();
    let registry = Registry::new();
    for target in ["Style/ReverseFind", "Security/YAMLLoad"] {
        let department = target.split('/').next().unwrap();
        for (toml, configured_on) in [
            (String::new(), true),
            (format!("[cops.\"{target}\"]\nenabled = false\n"), false),
            (format!("[cops.{department}]\nenabled = false\n"), false),
            (
                format!(
                    "[cops.{department}]\nenabled = false\n[cops.\"{target}\"]\nenabled = true\n"
                ),
                true,
            ),
            (
                format!(
                    "[cops.{department}]\nenabled = true\n[cops.\"{target}\"]\nenabled = false\n"
                ),
                false,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), &toml).unwrap();
            let config =
                ResolvedConfig::resolve(toml::from_str(&toml).unwrap(), None, &registry).unwrap();
            // Neither explicit configuration nor --only nor source enables
            // can activate the real min/max-inactive runtime matchers.
            assert!(
                rustocop::cops::select_active(&registry, &config, &[target.into()], &[]).is_empty()
            );
            for prefix in ["rubocop", "rustocop"] {
                let source = format!("# {prefix}:enable all\n# {prefix}:enable {target}\n0\n");
                let column = 11 + prefix.len() as u32;
                let mut expected = vec![];
                if configured_on {
                    expected.push((1, column, "Unnecessary enabling of all cops.".to_owned()));
                }
                expected.push((2, column, format!("Unnecessary enabling of {target}.")));
                let report = lint_one(
                    Path::new("example.rb"),
                    Path::new("example.rb"),
                    Some(source.as_bytes()),
                    &[&RedundantCopEnableDirective],
                    &config,
                );
                assert!(report.errors.is_empty() && report.internal_errors.is_empty());
                assert_eq!(
                    report
                        .result
                        .offenses
                        .iter()
                        .map(|o| (o.line, o.col, o.message.clone()))
                        .collect::<Vec<_>>(),
                    expected,
                    "{toml}: {source}"
                );
                let path = d.path().join("lib/a.rb");
                fs::write(&path, &source).unwrap();
                let old = fs::metadata(&path).unwrap();
                let forced = format!("{COP},{target}");
                for args in [
                    vec!["--only", COP, "lib/a.rb"],
                    vec!["--only", &forced, "lib/a.rb"],
                    vec!["--only", &forced, "--except", target, "lib/a.rb"],
                    vec!["--only", "Lint", "--except", department, "lib/a.rb"],
                ] {
                    let out = cli(d.path(), &args, 1);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let actual: Vec<_> = stdout
                        .lines()
                        .filter(|line| line.contains(": E: Unnecessary enabling"))
                        .collect();
                    let expected: Vec<_> = expected
                        .iter()
                        .map(|(line, col, msg)| format!("lib/a.rb:{line}:{col}: E: {msg} ({COP})"))
                        .collect();
                    assert_eq!(actual, expected, "{toml}: {args:?}");
                    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                    assert_eq!(
                        fs::metadata(&path).unwrap().modified().unwrap(),
                        old.modified().unwrap()
                    );
                }
                if configured_on {
                    fix(d.path(), source.as_bytes(), b"\n\n0\n");
                } else {
                    // The configured-off enable remains a protected opt-in,
                    // even when enable-all already consumed its pairing count.
                    fix(d.path(), source.as_bytes(), source.as_bytes());
                    let single = format!("# {prefix}:enable {target}\n0\n");
                    fix(d.path(), single.as_bytes(), single.as_bytes());
                    cli(d.path(), &["--only", COP, "lib/a.rb"], 0);
                }
                let matching_source = format!(
                    "# {prefix}:enable {target}\narray.reverse.find(&:even?)\nYAML.load(data)\n"
                );
                fs::write(&path, &matching_source).unwrap();
                cli(d.path(), &["--only", target, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read(&path).unwrap(), matching_source.as_bytes());
            }
        }
    }
}
#[test]
fn reserved_syntax_diagnostic_exclusion_does_not_hide_unknowns_or_parser_errors() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        let source = format!("# {prefix}:enable Lint/Syntax\n");
        fix(d.path(), source.as_bytes(), source.as_bytes());
        cli(d.path(), &["--only", COP, "lib/a.rb"], 0);
        for selector in ["Lint/UnknownCop", "Lint/SyntaxError", "Unknown/Syntax"] {
            let before = format!("# {prefix}:enable Lint/Syntax, {selector}\n");
            fs::write(d.path().join("lib/a.rb"), &before).unwrap();
            let out = cli(d.path(), &["--only", COP, "lib/a.rb"], 1);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert_eq!(
                stdout.lines().filter(|line| line.contains(": E:")).count(),
                1,
                "{stdout}"
            );
            assert!(
                stdout.contains(&format!(
                    "lib/a.rb:1:{}: E: Unnecessary enabling of {selector}.",
                    24 + prefix.len()
                )),
                "{stdout}"
            );
            fix(d.path(), before.as_bytes(), source.as_bytes());
        }
        let broken = format!("# {prefix}:enable Lint/Syntax\ndef broken(\n");
        let path = d.path().join("lib/a.rb");
        fs::write(&path, &broken).unwrap();
        let old = fs::metadata(&path).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 2);
        assert_eq!(fs::read(&path).unwrap(), broken.as_bytes());
        let new = fs::metadata(&path).unwrap();
        assert_eq!(new.modified().unwrap(), old.modified().unwrap());
        assert_eq!(new.permissions(), old.permissions());
    }
    assert!(
        Registry::new()
            .cops()
            .iter()
            .any(|c| c.name() == "Lint/Syntax")
    );
}
#[test]
fn review_regressions_use_actual_cli_selection_and_configured_state() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for (selectors, col) in [
            ("Layout/LineLength, Layout", 18),
            ("Layout, Layout/LineLength", 26),
        ] {
            let source = format!("# {prefix}:disable Layout\n# {prefix}:enable {selectors}\nfoo\n");
            fix(d.path(), source.as_bytes(), source.as_bytes());
            let out = cli(d.path(), &["--only", COP, "lib/a.rb"], 1);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                stdout.contains(&format!(
                    "lib/a.rb:2:{}: E: Unnecessary enabling of Layout/LineLength.",
                    col + prefix.len() - 7
                )),
                "{stdout}"
            );
        }
        for (target, config) in [
            ("Style/IpAddresses", ""),
            (
                "Layout/LineLength",
                "[cops.\"Layout/LineLength\"]\nenabled = false\n",
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let source =
                format!("=begin\n# {prefix}:enable {target}\n=end\n# {prefix}:enable all\n0\n");
            fs::write(d.path().join("lib/a.rb"), &source).unwrap();
            let out = cli(d.path(), &["--only", COP, "lib/a.rb"], 1);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                stdout.contains(&format!(
                    "lib/a.rb:4:{}: E: Unnecessary enabling of all cops.",
                    11 + prefix.len()
                )),
                "{stdout}"
            );
            let after = source.replace(&format!("# {prefix}:enable all"), "");
            fix(d.path(), source.as_bytes(), after.as_bytes());
            cli(d.path(), &["--only", COP, "lib/a.rb"], 0);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn both_prefixes_multiline_next_push_scopes_and_withheld_bytes() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for scope in [
            format!("# {prefix}:disable {COP}\n{BEFORE}"),
            format!("# {prefix}:push -Lint\n{BEFORE}# {prefix}:pop\n"),
            format!(
                "# {prefix}:disable-next {COP}\nfoo(\n# {prefix}:enable Layout/LineLength\nbar\n)\n"
            ),
            format!("# {prefix}:next -{COP}\nfoo(\n# {prefix}:enable Layout/LineLength\nbar\n)\n"),
        ] {
            fix(d.path(), scope.as_bytes(), scope.as_bytes());
        }
        let s = format!(
            "# {prefix}:disable-next Layout/LineLength\n# {prefix}:enable Style/For\nfoo(\n  bar\n)\n"
        );
        fix(d.path(), s.as_bytes(), s.as_bytes());
        let s = format!(
            "# {prefix}:push -Layout/LineLength\n# {prefix}:enable Layout/LineLength\nfoo\n# {prefix}:pop\n"
        );
        fix(d.path(), s.as_bytes(), s.as_bytes());
    }
}
#[test]
fn source_envelopes_and_shared_source_code_heap_position_barriers() {
    let d = project();
    for (before,after) in [
        (b"\xEF\xBB\xBF0\r\n# rustocop:enable Layout/LineLength\r\n".as_slice(), b"\xEF\xBB\xBF0\r\n".as_slice()),
        (b"# coding: shift_jis\n'\x82\xA0'\n# rubocop:enable Layout/LineLength\n", b"# coding: shift_jis\n'\x82\xA0'\n"),
        (b"x = <<~H\n# rubocop:enable Layout/LineLength\nH\n# rubocop:enable Layout/LineLength\n", b"x = <<~H\n# rubocop:enable Layout/LineLength\nH\n"),
        (b"__END__\n# rubocop:enable Layout/LineLength\n", b"__END__\n# rubocop:enable Layout/LineLength\n"),
        (b"# rubocop:enable Layout/LineLength\np __LINE__\n", b"\np __LINE__\n"),
    ] { fix(d.path(), before, after); }
    for observer in [
        "p File.binread(__FILE__)",
        "p DATA.pos\n__END__\nraw",
        "p ObjectSpace.count_objects",
        "p GC.stat",
        "p RubyVM::InstructionSequence.of(-> {}).to_a",
        "p RubyVM::AbstractSyntaxTree.of(-> {}).first_column",
        "RubyVM.keep_script_lines = true; p retained.script_lines",
    ] {
        let s = format!("# rubocop:enable Layout/LineLength\n{observer}\n");
        fix(d.path(), s.as_bytes(), s.as_bytes());
        cli(d.path(), &["--only", COP, "lib/a.rb"], 1);
    }
    let broken = b"# rubocop:enable Layout/LineLength\ndef broken(\n";
    let path = d.path().join("lib/a.rb");
    fs::write(&path, broken).unwrap();
    let old = fs::metadata(&path).unwrap();
    cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 2);
    assert_eq!(fs::read(&path).unwrap(), broken);
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        old.modified().unwrap()
    );
}
#[test]
fn controlled_native_ruby34_bindings_values_control_and_comments() {
    let Some(ruby) = crate::support::runtime::ruby_os() else {
        return;
    };
    let d = project();
    for source in [
        "x = 3; p [x, local_variables]\n# rubocop:enable Layout/LineLength\n",
        "def f; x = 1; begin; return x; ensure; puts 'done'; end; end; p f\n# rubocop:pop\n",
        "# rubocop:enable Layout/LineLength\np __LINE__\n",
        "x = <<~H\ntext\nH\np x\n# rubocop:enable Layout/LineLength\n",
    ] {
        let path = d.path().join("lib/a.rb");
        fs::write(&path, source).unwrap();
        let before = Command::new(&ruby).arg(&path).output().unwrap();
        assert!(before.status.success());
        cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        let after = Command::new(&ruby).arg(&path).output().unwrap();
        assert_eq!(
            (before.status.code(), before.stdout, before.stderr),
            (after.status.code(), after.stdout, after.stderr)
        );
    }
}
