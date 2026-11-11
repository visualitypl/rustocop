//! exact pinned detection and real CLI no-write controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use rustocop::offense::Offense;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

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
    assert_eq!(fs::read(&path).unwrap(), source, "{out:?}");
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}

fn stdin(root: &Path, source: &[u8], args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}

const EXPR: &str = "Lint/EmptyExpression";
const FILE: &str = "Lint/EmptyFile";

#[test]
fn every_pinned_spec_documentation_and_control_matches_pipeline_and_rebuilt_cli_without_edits() {
    let dir = project();
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/empty_expression_file.json")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 187);
    for row in rows.as_array().unwrap() {
        let cop = row["cop"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let mut found: Vec<_> = offenses(source, cop)
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
        assert_eq!(found, expected, "{row}");
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
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

#[test]
fn metadata_matches_actual_default_detection_only_policy() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 582);
    for name in [EXPR, FILE] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled());
        assert!(!cop.safe_fixes());
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
        assert!(cop.max_setting().is_none());
    }
    assert!(!registry.has_cop("Lint/EmptyRescue"));
}

#[test]
fn defaults_selectors_config_and_department_precedence_never_write_without_fix() {
    let dir = project();
    for (cop, source) in [(EXPR, "()\n"), (FILE, "")] {
        for (config, select, expected) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint", "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
            (
                format!("[cops.Lint]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (
                format!("[cops.Lint]\nenabled = true\n[cops.\"{cop}\"]\nenabled = false\n"),
                vec![],
                0,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let mut args = select;
            args.push("lib/a.rb");
            // Other default cops may report too; only selection's count matters.
            let path = dir.path().join("lib/a.rb");
            fs::write(&path, source).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = run(dir.path(), &args);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_eq!(count(&out, cop), expected, "{out:?}");
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            let new = fs::metadata(path).unwrap();
            assert_eq!(old.modified().unwrap(), new.modified().unwrap());
            assert_eq!(old.permissions(), new.permissions());
        }
    }
}

#[test]
fn strict_toml_exclusions_and_supplied_paths_apply_to_both_source_lifecycles() {
    let dir = project();
    for (cop, source) in [(EXPR, "()\n"), (FILE, "")] {
        for key in [
            "max = 1",
            "AllowComments = false",
            "include = ['**/*']",
            "enabled = 'true'",
            "exclude = ['[']",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                2,
            );
        }
        for config in [
            "exclude = ['lib/a.rb']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                0,
            );
            let out = run(&dir.path().join("lib"), &["--only", cop, "a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            assert_eq!(count(&out, cop), 0);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        fs::write(dir.path().join("lib/source.arb"), source).unwrap();
        let out = run(dir.path(), &["--only", cop, "lib/source.arb"]);
        assert_eq!(count(&out, cop), 1, "{out:?}");
    }
}

#[test]
fn both_directive_prefixes_multiline_next_push_and_source_enable_scopes() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Lint\n()\n"),
            format!("# {prefix}:disable-next {EXPR}\n(\n)\n"),
            format!("(\n) # {prefix}:disable {EXPR}\n"),
            format!("# {prefix}:push -{EXPR}\n()\n# {prefix}:pop\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", EXPR, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "# {prefix}:disable Lint\n()\n# {prefix}:push +{EXPR}\n(\n)\n# {prefix}:pop\n()\n"
        );
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", EXPR, "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!(count(&out, EXPR), 1, "{out:?}");
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{EXPR}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable-next {EXPR}\n()\n()\n");
        let out = unchanged(dir.path(), source.as_bytes(), &["--fix", "lib/a.rb"], 1);
        assert_eq!(count(&out, EXPR), 1, "{out:?}");
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        // A directive necessarily makes the file nonempty: no fabricated global
        // offense or source-enable on whitespace/comment-only files.
        let source = format!("# {prefix}:enable {FILE}\n");
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", FILE, "--fix", "lib/a.rb"],
            0,
        );
    }
}

#[test]
fn stdin_provenance_never_reads_or_writes_the_display_path_and_fix_is_rejected() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "sentinel\n").unwrap();
    let old = fs::metadata(&path).unwrap();
    for (cop, source) in [(EXPR, "()\n"), (FILE, "")] {
        for (config, expected) in [
            (String::new(), 1),
            ("exclude = ['lib/a.rb']\n".into(), 0),
            (format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"), 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = stdin(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--stdin", "lib/a.rb"],
            );
            assert_eq!(out.status.code(), Some(i32::from(expected > 0)), "{out:?}");
            assert_eq!(count(&out, cop), expected, "{out:?}");
            if expected > 0 {
                assert!(String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:1:1: E:"));
            }
        }
        let out = stdin(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--stdin", "lib/a.rb", "--fix"],
        );
        assert_eq!(out.status.code(), Some(2));
    }
    assert_eq!(fs::read(&path).unwrap(), b"sentinel\n");
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}

#[test]
fn source_envelopes_and_observer_text_keep_findings_and_never_receive_fabricated_edits() {
    let dir = project();
    for source in [
        b"\xef\xbb\xbf# encoding: utf-8\r\n()\r\np __LINE__\r\n__END__\r\nraw\x00\r\n".as_slice(),
        b"# encoding: Windows-1252\np '\xe9'; ()\n",
        b"x = <<~TEXT\n()\nTEXT\n()\n",
        b"(); File.binread(__FILE__); DATA.pos\n__END__\nraw\n",
        b"(); RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
    ] {
        let out = unchanged(
            dir.path(),
            source,
            &[
                "--only",
                "Lint/EmptyExpression,Lint/EmptyFile",
                "--fix",
                "lib/a.rb",
            ],
            1,
        );
        assert_eq!(count(&out, EXPR), 1, "{out:?}");
        assert_eq!(count(&out, FILE), 0, "{out:?}");
    }
}

#[test]
fn parser_failure_missing_inputs_and_readonly_symlinks_reuse_shared_no_write_controls() {
    let dir = project();
    let both = "Lint/EmptyExpression,Lint/EmptyFile";
    let out = unchanged(
        dir.path(),
        b"()\ndef broken(\n",
        &["--only", both, "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!((count(&out, EXPR), count(&out, FILE)), (0, 0));
    let out = run(dir.path(), &["--only", both, "--fix", "missing.rb"]);
    assert_eq!(out.status.code(), Some(2));
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = dir.path().join("lib/a.rb");
        symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
        for (cop, source) in [(EXPR, "()\n"), (FILE, "")] {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            fs::write(&path, source).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = run(
                dir.path(),
                &["--only", cop, "--fix", "lib/alias.rb", "lib/a.rb"],
            );
            assert_eq!(out.status.code(), Some(1), "{out:?}");
            assert!(count(&out, cop) >= 1, "{out:?}");
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
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
}

fn offenses(source: &str, cop: &str) -> Vec<Offense> {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[], &[]);
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
    report
        .result
        .offenses
        .into_iter()
        .filter(|o| o.cop == cop)
        .collect()
}

#[test]
fn empty_file_global_offense_runs_without_statements() {
    let found = offenses("", FILE);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
    assert_eq!(found[0].message, "Empty file detected.");
    assert!(found[0].correction.is_none());
    for source in [
        " ",
        "\n",
        "\r\n",
        "\t",
        "# comment\n",
        "foo.bar\n",
        "__END__\n",
        "\u{feff}",
    ] {
        assert!(offenses(source, FILE).is_empty(), "{source:?}");
    }
}

#[test]
fn empty_expression_exact_locations_in_all_pinned_contexts() {
    for (source, line, col) in [
        ("()", 1, 1),
        ("(())", 1, 2),
        ("if (); end", 1, 4),
        ("if foo\n  1\nelsif ()\n  2\nend", 3, 7),
        ("case ()\nwhen :foo then 1\nend", 1, 6),
        ("case foo\nwhen () then 1\nend", 2, 6),
        ("() ? true : false", 1, 1),
        ("foo ? () : bar", 1, 7),
        ("def foo\n  ()\nend", 2, 3),
        ("if foo\n  ()\nend", 2, 3),
        ("case foo\nwhen :bar then ()\nend", 2, 16),
        ("foo = ()", 1, 7),
    ] {
        let found = offenses(source, EXPR);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
        assert_eq!(found[0].message, "Avoid empty expressions.");
        assert!(found[0].correction.is_none());
    }
}
