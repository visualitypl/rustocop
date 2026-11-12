//! Source-only pinned fixtures, rebuilt CLI and no-write policy.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COMPARISON: &str = "Lint/FloatComparison";
const RANGE: &str = "Lint/FloatOutOfRange";
const BOTH: &str = "Lint/FloatComparison,Lint/FloatOutOfRange";
const SOURCE: &[u8] = b"x == 0.1\nvalue = 1e400\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(cop))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}\n{source:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
    out
}

#[test]
fn all_pinned_specs_source_chunks_and_numeric_controls_match_pipeline_without_edits() {
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/float_literals.json")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 4046);
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let mut mismatches = Vec::new();
    for row in rows.as_array().unwrap() {
        let cop = row["cop"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{report:?}: {row}"
        );
        let mut found: Vec<_> = report
            .result
            .offenses
            .into_iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message)
            })
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> =
            serde_json::from_value(row["findings"].clone()).unwrap();
        if found != expected {
            mismatches.push(format!(
                "{source:?}: {cop}: expected {expected:?}, found {found:?}"
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

#[test]
fn every_reference_fixture_matches_exact_rebuilt_cli_diagnostics_without_writes() {
    let dir = project();
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/float_literals.json")).unwrap();
    for row in rows.as_array().unwrap() {
        let cop = row["cop"].as_str().unwrap();
        let expected: Vec<(u32, u32, u32, String)> =
            serde_json::from_value(row["findings"].clone()).unwrap();
        let out = unchanged(
            dir.path(),
            row["source"].as_str().unwrap().as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            i32::from(!expected.is_empty()),
        );
        assert_eq!(count(&out, cop), expected.len(), "{row}\n{out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        for (line, col, _, msg) in expected {
            assert!(
                stdout.contains(&format!("lib/a.rb:{line}:{col}: E: {msg} ({cop})")),
                "{stdout}"
            );
        }
    }
}

fn encoded_rows() -> Vec<serde_json::Value> {
    let rows: Vec<_> =
        serde_json::from_str(include_str!("fixtures/float_literal_positions.json")).unwrap();
    assert_eq!(rows.len(), 461);
    rows
}

fn encoded_source(row: &serde_json::Value) -> Vec<u8> {
    serde_json::from_value(row["bytes"].clone()).unwrap()
}

fn assert_encoded_diagnostics(out: &Output, row: &serde_json::Value, path: &str, enabled: &[&str]) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    for cop in [COMPARISON, RANGE] {
        let findings: Vec<(u32, u32, u32, String)> =
            serde_json::from_value(row["findings"][cop].clone()).unwrap();
        let mut expected: Vec<_> = findings
            .into_iter()
            .filter(|_| enabled.contains(&cop))
            .map(|(line, col, _, message)| format!("{path}:{line}:{col}: E: {message} ({cop})"))
            .collect();
        let mut found: Vec<_> = stdout
            .lines()
            .filter(|l| l.ends_with(&format!("({cop})")))
            .map(str::to_owned)
            .collect();
        found.sort();
        expected.sort();
        assert_eq!(found, expected, "{row}\n{out:?}");
    }
}

#[test]
fn encoded_original_bytes_match_exact_pinned_pipeline_positions_without_edits() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active =
        rustocop::cops::select_active(&registry, &config, &[COMPARISON.into(), RANGE.into()], &[]);
    for row in encoded_rows() {
        let source = encoded_source(&row);
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}: {report:?}"
        );
        for cop in [COMPARISON, RANGE] {
            let mut found: Vec<_> = report
                .result
                .offenses
                .iter()
                .filter(|o| o.cop == cop)
                .map(|o| {
                    assert!(o.correction.is_none());
                    assert_eq!(o.severity, rustocop::offense::Severity::Error);
                    (o.line, o.col, o.end_line, o.message.clone())
                })
                .collect();
            found.sort();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"][cop].clone()).unwrap();
            assert_eq!(found, expected, "{row}");
        }
    }
}

#[test]
fn native_ruby34_checks_encoded_position_controls_without_executing_source() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for row in encoded_rows() {
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-c", "-W0", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&encoded_source(&row))
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("Syntax OK"),
            "{out:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn encoded_cli_lint_and_fix_preserve_open_fd_inode_mtime_mode751_and_exact_positions() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt},
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for row in encoded_rows() {
        let source = encoded_source(&row);
        for fix in [false, true] {
            fs::write(&path, &source).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
            let mut original = fs::File::open(&path).unwrap();
            let old = original.metadata().unwrap();
            let mut args = vec!["--only", BOTH];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            let out = run(dir.path(), &args);
            let has_findings = [COMPARISON, RANGE]
                .iter()
                .any(|cop| !row["findings"][cop].as_array().unwrap().is_empty());
            assert_eq!(out.status.code(), Some(i32::from(has_findings)), "{out:?}");
            assert_encoded_diagnostics(&out, &row, "lib/a.rb", &[COMPARISON, RANGE]);
            let mut fd_bytes = Vec::new();
            original.read_to_end(&mut fd_bytes).unwrap();
            assert_eq!(fd_bytes, source);
            assert_eq!(fs::read(&path).unwrap(), source);
            let new = fs::metadata(&path).unwrap();
            assert_eq!(
                (old.ino(), old.mtime(), old.mtime_nsec(), old.mode()),
                (new.ino(), new.mtime(), new.mtime_nsec(), new.mode())
            );
            assert_eq!(new.permissions().mode() & 0o777, 0o751);
            assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 1);
        }
    }
}

#[test]
fn encoded_defaults_config_paths_directives_and_stdin_keep_projected_positions() {
    let dir = project();
    for row in encoded_rows()
        .into_iter()
        .filter(|r| r["origin"] == "counterexample")
    {
        let source = encoded_source(&row);
        for (config, enabled) in [
            (String::new(), vec![COMPARISON, RANGE]),
            (
                format!("[cops.Lint]\nenabled = false\n[cops.\"{COMPARISON}\"]\nenabled = true\n"),
                vec![COMPARISON],
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(dir.path(), &source, &["lib/a.rb"], 1);
            assert_encoded_diagnostics(&out, &row, "lib/a.rb", &enabled);
            // Explicit non-Ruby extension, subdirectory cwd and stdin's supplied
            // path all retain source provenance without becoming write targets.
            let path = dir.path().join("lib/encoded.jb");
            fs::write(&path, &source).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = run(&dir.path().join("lib"), &["encoded.jb"]);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_encoded_diagnostics(&out, &row, "lib/encoded.jb", &enabled);
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--stdin", "lib/encoded.jb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_encoded_diagnostics(&out, &row, "lib/encoded.jb", &enabled);
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(
                old.modified().unwrap(),
                fs::metadata(&path).unwrap().modified().unwrap()
            );
            assert_eq!(
                old.permissions(),
                fs::metadata(&path).unwrap().permissions()
            );
        }
        for config in [
            "exclude = ['lib/a.rb']\n".to_owned(),
            format!(
                "[cops.\"{COMPARISON}\"]\nexclude = ['lib/a.rb']\n[cops.\"{RANGE}\"]\nexclude = ['lib/a.rb']\n"
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(
                dir.path(),
                &source,
                &["--only", BOTH, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!((count(&out, COMPARISON), count(&out, RANGE)), (0, 0));
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for body in [
            format!("# {prefix}:disable Lint\n'e\u{301}'; x == 0.1; -1e400\n"),
            format!("# {prefix}:disable-next {BOTH}\n'e\u{301}'; x == 0.1; -1e400\n"),
            format!(
                "# {prefix}:push -{COMPARISON} -{RANGE}\n'e\u{301}'; x == 0.1; -1e400\n# {prefix}:pop\n"
            ),
            format!(
                "'e\u{301}'; x.eql?(\n0.1\n) # {prefix}:disable {COMPARISON}\n'e\u{301}'; -1e400 # {prefix}:disable {RANGE}\n"
            ),
        ] {
            let source = format!("# coding: UTF8-MAC\r\n{body}");
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", BOTH, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!((count(&out, COMPARISON), count(&out, RANGE)), (0, 0));
        }
    }
    let path = dir.path().join("lib/a.rb");
    let old = fs::metadata(&path).unwrap();
    let bytes = fs::read(&path).unwrap();
    let out = run(dir.path(), &["--stdin", "lib/a.rb", "--fix"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
}

#[test]
fn metadata_matches_real_detection_only_policy() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for name in [COMPARISON, RANGE] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled());
        assert!(!cop.safe_fixes());
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
        assert!(cop.max_setting().is_none());
    }
}

#[test]
fn defaults_selectors_and_strict_config_precedence_preserve_bytes_even_without_fix() {
    let dir = project();
    for (config, selection, expected) in [
        (String::new(), vec![], (1, 1)),
        (String::new(), vec!["--only", BOTH], (1, 1)),
        (
            String::new(),
            vec!["--only", "Lint", "--except", COMPARISON],
            (0, 1),
        ),
        (
            String::new(),
            vec!["--only", BOTH, "--except", BOTH],
            (0, 0),
        ),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], (0, 0)),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COMPARISON}\"]\nenabled = true\n"),
            vec![],
            (1, 0),
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{RANGE}\"]\nenabled = false\n"),
            vec![],
            (1, 0),
        ),
        (
            format!("[cops.\"{COMPARISON}\"]\nenabled = false\n"),
            vec!["--only", COMPARISON],
            (1, 0),
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(dir.path().join("lib/a.rb"), SOURCE).unwrap();
        let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
        let mut args = selection;
        args.push("lib/a.rb");
        let out = run(dir.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(
            (count(&out, COMPARISON), count(&out, RANGE)),
            expected,
            "{out:?}"
        );
        assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), SOURCE);
        assert_eq!(
            fs::metadata(dir.path().join("lib/a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            old.modified().unwrap()
        );
    }
    for cop in [COMPARISON, RANGE] {
        for key in [
            "max = 1",
            "include = ['**/*']",
            "enabled = 'true'",
            "TargetRubyVersion = 3.3",
            "Style = 'epsilon'",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                SOURCE,
                &["--only", BOTH, "--fix", "lib/a.rb"],
                2,
            );
        }
    }
}

#[test]
fn supplied_paths_exclusions_and_stdin_are_provenance_not_write_targets() {
    let dir = project();
    for config in [
        "exclude = ['lib/a.rb']\n".into(),
        format!(
            "[cops.\"{COMPARISON}\"]\nexclude = ['lib/a.rb']\n[cops.\"{RANGE}\"]\nexclude = ['lib/a.rb']\n"
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            SOURCE,
            &["--only", BOTH, "--fix", "lib/a.rb"],
            0,
        );
        assert_eq!(
            run(&dir.path().join("lib"), &["--only", BOTH, "a.rb"])
                .status
                .code(),
            Some(0)
        );
    }
    for (config, expected) in [
        (String::new(), (1, 1)),
        ("exclude = ['lib/a.rb']\n".into(), (0, 0)),
        (
            format!("[cops.\"{RANGE}\"]\nexclude = ['lib/a.rb']\n"),
            (1, 0),
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(dir.path().join("lib/a.rb"), b"sentinel\n").unwrap();
        let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", BOTH, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            (count(&out, COMPARISON), count(&out, RANGE)),
            expected,
            "{out:?}"
        );
        assert_eq!(
            fs::read(dir.path().join("lib/a.rb")).unwrap(),
            b"sentinel\n"
        );
        assert_eq!(
            fs::metadata(dir.path().join("lib/a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            old.modified().unwrap()
        );
    }
    assert_eq!(
        run(
            dir.path(),
            &["--only", BOTH, "--stdin", "lib/a.rb", "--fix"]
        )
        .status
        .code(),
        Some(2)
    );
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for path in ["lib/a.jb", "Gemfile", "lib/.hidden.rb", "lib/a_spec.rb"] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        let out = run(dir.path(), &["--only", BOTH, path]);
        assert_eq!(
            (count(&out, COMPARISON), count(&out, RANGE)),
            (1, 1),
            "{out:?}"
        );
    }
}

#[test]
fn both_directive_prefixes_multiline_next_and_push_scopes() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Lint\nx == 0.1\n1e400\n"),
            format!(
                "# {prefix}:disable-next {COMPARISON}\nx.eql?(\n0.1\n)\n# {prefix}:disable-next {RANGE}\n1e400\n"
            ),
            format!(
                "x.eql?(\n0.1\n) # {prefix}:disable {COMPARISON}\n1e400 # {prefix}:disable {RANGE}\n"
            ),
            format!("# {prefix}:push -{COMPARISON} -{RANGE}\nx == 0.1\n1e400\n# {prefix}:pop\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", BOTH, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "# {prefix}:disable Lint\nx == 0.1\n1e400\n# {prefix}:push +{COMPARISON} +{RANGE}\nx == 0.1\n1e400\n# {prefix}:pop\nx == 0.1\n1e400\n"
        );
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", BOTH, "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!((count(&out, COMPARISON), count(&out, RANGE)), (1, 1));
    }
}

#[test]
fn encoding_envelopes_and_runtime_observer_source_text_never_get_fabricated_edits() {
    let dir = project();
    for source in [
        b"\xef\xbb\xbf# encoding: utf-8\r\nx == 0.1\r\n1e400\r\np __LINE__\r\n__END__\r\nraw\x00\r\n".as_slice(),
        b"# encoding: Windows-1252\np '\xe9'; x == 0.1\n1e400\n",
        b"x = <<~TEXT\nx == 0.1\n1e400\nTEXT\nx == 0.1\n1e400\n",
        b"x == 0.1; 1e400; File.binread(__FILE__); DATA.pos\n__END__\nraw\n",
        b"x == 0.1; 1e400; RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
    ] {
        let out = unchanged(dir.path(), source, &["--only", BOTH, "--fix", "lib/a.rb"], 1);
        assert_eq!((count(&out, COMPARISON), count(&out, RANGE)), (1, 1), "{out:?}");
    }
}

#[test]
fn native_ruby34_checks_every_reference_fixture_syntax_only_never_executes_inputs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/float_literals.json")).unwrap();
    for row in rows.as_array().unwrap() {
        let mut child = Command::new(&ruby)
            .args(["-c", "-W0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(row["source"].as_str().unwrap().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("Syntax OK"),
            "{out:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn shared_layout_edits_remain_atomic_and_readonly_failure_keeps_both_findings() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    let dir = project();
    let before = b"x == 0.1\n1e400\nvalues = [1,2]\n";
    let after = b"x == 0.1\n1e400\nvalues = [1, 2]\n";
    let only = "Lint/FloatComparison,Lint/FloatOutOfRange,Layout/SpaceAfterComma";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
    let mut original = fs::File::open(&path).unwrap();
    let inode = original.metadata().unwrap().ino();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/alias.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!((count(&out, COMPARISON), count(&out, RANGE)), (1, 1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 correction applied"));
    assert_eq!(fs::read(&path).unwrap(), after);
    let mut old_bytes = Vec::new();
    original.read_to_end(&mut old_bytes).unwrap();
    assert_eq!(old_bytes, before);
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(dir.path().join("lib/alias.rb"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/alias.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!((count(&out, COMPARISON), count(&out, RANGE)), (1, 1));
    assert_eq!(fs::read(&path).unwrap(), before);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}

#[test]
fn invalid_source_failure_and_readonly_symlinks_preserve_bytes_permissions_and_links() {
    let dir = project();
    let out = unchanged(
        dir.path(),
        b"x == 0.1\n1e400\ndef broken(\n",
        &["--only", BOTH, "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!((count(&out, COMPARISON), count(&out, RANGE)), (0, 0));
    assert_eq!(
        run(dir.path(), &["--only", BOTH, "--fix", "missing.rb"])
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
            &["--only", BOTH, "--fix", "lib/alias.rb", "lib/a.rb"],
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(count(&out, COMPARISON) >= 1 && count(&out, RANGE) >= 1);
        assert_eq!(fs::read(&path).unwrap(), SOURCE);
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert!(
            fs::symlink_metadata(dir.path().join("lib/alias.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
