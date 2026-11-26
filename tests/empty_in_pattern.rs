//! pinned source-only reference examples and actual CLI no-write controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;

const COP: &str = "Lint/EmptyInPattern";
const SOURCE: &str = "case value\nin [a]\nend\n";

fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(COP))
        .count()
}

fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}\n{source:?}");
    assert_eq!(fs::read(&path).unwrap(), source, "{out:?}");
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}

/// `expect_early_error` only for runs whose asserted exit is the early
/// `--stdin --fix` rejection, which may exit before reading stdin.
fn stdin(root: &Path, source: &[u8], args: &[&str], expect_early_error: bool) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    stdin_support::write_stdin(child.stdin.take().unwrap(), source, expect_early_error).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn all_pinned_specs_source_examples_and_controls_match_pipeline_and_cli_without_writes() {
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/empty_in_pattern.json")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 188);
    check_reference_rows(&rows);
}

#[test]
fn lexical_block_comment_reference_matrix_matches_pipeline_and_cli_without_writes() {
    let rows: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/empty_in_pattern_block_comments.json"
    ))
    .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 128);
    check_reference_rows(&rows);
}

fn check_reference_rows(rows: &serde_json::Value) {
    let dir = project_with_lib();
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    for row in rows.as_array().unwrap() {
        let source = row["source"].as_str().unwrap();
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{report:?}"
        );
        let mut found: Vec<_> = report
            .result
            .offenses
            .into_iter()
            .map(|o| {
                assert_eq!(o.cop, COP);
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Warning);
                (o.line, o.col, o.end_line, o.message)
            })
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> =
            crate::support::fixture::value(&row["findings"]);
        assert_eq!(found, expected, "{row}");
        for fix in [false, true] {
            let mut args = vec!["--only", COP, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &args,
                i32::from(!expected.is_empty()),
            );
            assert_eq!(count(&out), expected.len(), "{row}\n{out:?}");
            let stdout = String::from_utf8_lossy(&out.stdout);
            for (line, col, _, msg) in &expected {
                assert!(
                    stdout.contains(&format!("lib/a.rb:{line}:{col}: W: {msg} ({COP})")),
                    "{stdout}"
                );
            }
        }
    }
}

#[test]
fn block_pseudo_directives_do_not_suppress_code_or_opt_in_config_disabled_cops() {
    let dir = project_with_lib();
    for prefix in ["rubocop", "rustocop"] {
        for (mode, close, pattern_count) in [
            ("push -Lint", "pop", 1),
            ("disable Lint", "enable Lint", 1),
            ("todo Lint", "enable Lint", 1),
            ("next -Lint", "pop", 0),
            ("disable-next Lint", "enable Lint", 0),
        ] {
            let source = format!(
                "case x\nin [a]\n=begin\n# {prefix}:{mode}\n=end\n# {prefix}:{close}\nend\nbinding.pry\n"
            );
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &[
                    "--only",
                    "Lint/Debugger,Lint/EmptyInPattern",
                    "--fix",
                    "lib/a.rb",
                ],
                1,
            );
            assert_eq!(count(&out), pattern_count, "{source}\n{out:?}");
            assert!(String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:8:1: W:"));
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        for mode in [
            format!("enable {COP}"),
            format!("enable-next {COP}"),
            format!("push +{COP}"),
            format!("next +{COP}"),
        ] {
            // As in RuboCop, Style/BlockComments turns the pseudo-directive
            // into a `# # …` line comment, which mobilizes nothing.
            let source = format!("=begin\n# {prefix}:{mode}\n=end\n{SOURCE}");
            let path = dir.path().join("lib/a.rb");
            fs::write(&path, &source).unwrap();
            let out = run(dir.path(), &["--fix", "lib/a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{source}\n{out:?}");
            assert_eq!(count(&out), 0, "{source}\n{out:?}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                format!("# # {prefix}:{mode}\n{SOURCE}")
            );
        }
        // Ordinary enables must still mobilize the same configured-disabled
        // cop; this is not a global rejection of source opt-ins.
        let source = format!("# {prefix}:enable {COP}\n{SOURCE}");
        let out = unchanged(dir.path(), source.as_bytes(), &["--fix", "lib/a.rb"], 1);
        assert_eq!(count(&out), 1, "{out:?}");
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn real_minimum_ruby_gate_and_detection_only_metadata() {
    use rustocop::cops::policy::{RubyVersion, RubyVersions};
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(
        cop.ruby_versions(),
        RubyVersions {
            minimum: Some(RubyVersion(2, 7)),
            maximum: None
        }
    );
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
    assert!(!registry.has_cop("Lint/EmptyRescue"));
}

#[test]
fn default_selectors_and_strict_toml_precedence() {
    let dir = project_with_lib();
    for (config, select, expected) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            1,
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let status = i32::from(expected > 0);
        let mut args = select;
        args.push("lib/a.rb");
        let out = unchanged(dir.path(), SOURCE.as_bytes(), &args, status);
        assert_eq!(count(&out), expected, "{out:?}");
    }
    for key in [
        "max = 1",
        "AllowComments = false",
        "include = ['**/*']",
        "enabled = 'true'",
        "exclude = ['[']",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            2,
        );
    }
}

#[test]
fn user_exclusions_supplied_paths_and_stdin_provenance() {
    let dir = project_with_lib();
    let sentinel = dir.path().join("lib/display.rb");
    fs::write(&sentinel, "sentinel\n").unwrap();
    let old = fs::metadata(&sentinel).unwrap();
    for config in [
        String::new(),
        "exclude = ['lib/*.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/*.rb']\n"),
    ] {
        let excluded = !config.is_empty();
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let out = unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            i32::from(!excluded),
        );
        assert_eq!(count(&out), usize::from(!excluded));
        let out = run(&dir.path().join("lib"), &["--only", COP, "a.rb"]);
        assert_eq!(out.status.code(), Some(i32::from(!excluded)), "{out:?}");
        let out = stdin(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--stdin", "lib/display.rb"],
            false,
        );
        assert_eq!(out.status.code(), Some(i32::from(!excluded)), "{out:?}");
        assert_eq!(count(&out), usize::from(!excluded));
        if !excluded {
            assert!(String::from_utf8_lossy(&out.stdout).contains("lib/display.rb:2:1: W:"));
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/source.arb"), SOURCE).unwrap();
    assert_eq!(
        count(&run(dir.path(), &["--only", COP, "lib/source.arb"])),
        1
    );
    let out = stdin(
        dir.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--stdin", "lib/display.rb", "--fix"],
        true,
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
        "{out:?}"
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel\n");
    let new = fs::metadata(sentinel).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}

#[test]
fn both_prefixes_multiline_next_push_and_source_enables() {
    let dir = project_with_lib();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Lint\n{SOURCE}"),
            format!("# {prefix}:disable-next {COP}\ncase x\nin [a]\nend\n"),
            format!("case x\nin [\n a,\n b\n] # {prefix}:disable {COP}\nend\n"),
            format!("# {prefix}:push -{COP}\n{SOURCE}# {prefix}:pop\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "# {prefix}:disable Lint\n{SOURCE}# {prefix}:push +{COP}\n{SOURCE}# {prefix}:pop\n{SOURCE}"
        );
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!(count(&out), 1, "{out:?}");
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable-next {COP}\n{SOURCE}{SOURCE}");
        let out = unchanged(dir.path(), source.as_bytes(), &["--fix", "lib/a.rb"], 1);
        assert_eq!(count(&out), 1, "{out:?}");
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn envelopes_observers_parser_failures_and_readonly_symlinks_never_write() {
    let dir = project_with_lib();
    for source in [
        b"\xef\xbb\xbf# encoding: utf-8\r\ncase x\r\nin [a]\r\nend\r\np __LINE__\r\n__END__\r\nraw\x00\r\n".as_slice(),
        b"# encoding: Windows-1252\np '\xe9'\ncase x\nin [a]\nend\n",
        b"x = <<~TEXT\nin [a]\nTEXT\ncase x\nin [a]\nend\n",
        b"case x\nin [a]\nend\nFile.binread(__FILE__); DATA.pos\n__END__\nraw\n",
        b"case x\nin [a]\nend\nRubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
    ] {
        let out = unchanged(dir.path(), source, &["--only", COP, "--fix", "lib/a.rb"], 1);
        assert_eq!(count(&out), 1);
    }
    let out = unchanged(
        dir.path(),
        b"case x\nin [a]\nend\ndef broken(\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!(count(&out), 0);
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "missing.rb"])
            .status
            .code(),
        Some(2)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, SOURCE).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(
            dir.path(),
            &["--only", COP, "--fix", "lib/alias.rb", "lib/a.rb"],
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(count(&out) >= 1);
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
        let new = fs::metadata(path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert!(
            fs::symlink_metadata(dir.path().join("lib/alias.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
