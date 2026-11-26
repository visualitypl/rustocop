//! fixed-policy reference examples and real CLI no-write controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const BLOCK: &str = "Lint/EmptyBlock";
const CLASS: &str = "Lint/EmptyClass";
const BOTH: &str = "Lint/EmptyBlock,Lint/EmptyClass";

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
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}

#[test]
fn all_pinned_specs_documentation_and_controls_match_pipeline_and_rebuilt_cli_without_edits() {
    let dir = project_with_lib();
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/empty_contexts.json")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 187);
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    for row in rows.as_array().unwrap() {
        let cop = row["cop"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let report = rustocop::pipeline::lint_one(
            &dir.path().join("lib/a.rb"),
            Path::new("lib/a.rb"),
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
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Warning);
                (o.line, o.col, o.end_line, o.message)
            })
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> =
            crate::support::fixture::value(&row["findings"]);
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
                stdout.contains(&format!("lib/a.rb:{line}:{col}: W: {msg} ({cop})")),
                "{stdout}"
            );
        }
    }
}

#[test]
fn metadata_matches_actual_detection_only_default_policy() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for name in [BLOCK, CLASS] {
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
fn defaults_selectors_and_department_precedence_preserve_source_without_fix() {
    let dir = project_with_lib();
    let source = b"each {}\nclass Foo\nend\n";
    for (config, select, expected) in [
        (String::new(), vec![], (1, 1)),
        (String::new(), vec!["--only", BOTH], (1, 1)),
        (
            String::new(),
            vec!["--only", "Lint", "--except", BLOCK],
            (0, 1),
        ),
        (
            String::new(),
            vec!["--only", BOTH, "--except", BOTH],
            (0, 0),
        ),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], (0, 0)),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{BLOCK}\"]\nenabled = true\n"),
            vec![],
            (1, 0),
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{CLASS}\"]\nenabled = false\n"),
            vec![],
            (1, 0),
        ),
        (
            format!("[cops.\"{BLOCK}\"]\nenabled = false\n"),
            vec!["--only", BLOCK],
            (1, 0),
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = select;
        args.push("lib/a.rb");
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(
            (count(&out, BLOCK), count(&out, CLASS)),
            expected,
            "{out:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), source);
        let new = fs::metadata(path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
    }
}

#[test]
fn strict_toml_and_exclusions_use_supplied_path_without_invented_cop_gates() {
    let dir = project_with_lib();
    let source = b"each {}\nclass Foo; end\n";
    for cop in [BLOCK, CLASS] {
        for key in [
            "max = 1",
            "AllowComments = true",
            "AllowEmptyLambdas = false",
            "include = ['**/*']",
            "enabled = 'true'",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                source,
                &["--only", BOTH, "--fix", "lib/a.rb"],
                2,
            );
        }
    }
    for config in [
        "exclude = ['lib/a.rb']\n".into(),
        format!(
            "[cops.\"{BLOCK}\"]\nexclude = ['lib/a.rb']\n[cops.\"{CLASS}\"]\nexclude = ['lib/a.rb']\n"
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            source,
            &["--only", BOTH, "--fix", "lib/a.rb"],
            0,
        );
        let out = run(&dir.path().join("lib"), &["--only", BOTH, "a.rb"]);
        assert_eq!((count(&out, BLOCK), count(&out, CLASS)), (0, 0));
        assert_eq!(out.status.code(), Some(0), "{out:?}");
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/source.arb"), source).unwrap();
    let out = run(dir.path(), &["--only", BOTH, "lib/source.arb"]);
    assert_eq!((count(&out, BLOCK), count(&out, CLASS)), (1, 1), "{out:?}");
}

#[test]
fn both_prefixes_multiline_next_and_push_directive_scopes() {
    let dir = project_with_lib();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Lint\neach {{}}\nclass Foo; end\n"),
            format!(
                "# {prefix}:disable-next {BLOCK}\neach do\nend\n# {prefix}:disable-next {CLASS}\nclass Foo\nend\n"
            ),
            format!(
                "each do\nend # {prefix}:disable {BLOCK}\nclass Foo\nend # {prefix}:disable {CLASS}\n"
            ),
            format!(
                "# {prefix}:push -{BLOCK} -{CLASS}\neach {{}}\nclass Foo; end\n# {prefix}:pop\n"
            ),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", BOTH, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "# {prefix}:disable Lint\neach {{}}\nclass Foo; end\n# {prefix}:push +{BLOCK} +{CLASS}\neach {{}}\nclass Foo; end\n# {prefix}:pop\neach {{}}\nclass Foo; end\n"
        );
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", BOTH, "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!((count(&out, BLOCK), count(&out, CLASS)), (1, 1), "{out:?}");
    }
}

#[test]
fn stdin_path_is_provenance_never_a_write_target_and_stdin_fix_is_rejected() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, b"sentinel\n").unwrap();
    let old = fs::metadata(&path).unwrap();
    for (config, expected) in [
        (String::new(), (1, 1)),
        ("exclude = ['lib/a.rb']\n".into(), (0, 0)),
        (
            format!("[cops.\"{CLASS}\"]\nexclude = ['lib/a.rb']\n"),
            (1, 0),
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", BOTH, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"each {}\nclass Foo; end\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            (count(&out, BLOCK), count(&out, CLASS)),
            expected,
            "{out:?}"
        );
        if expected.0 == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:1:1: W:"));
        }
    }
    let out = run(
        dir.path(),
        &["--only", BOTH, "--stdin", "lib/a.rb", "--fix"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), b"sentinel\n");
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}

#[test]
fn source_envelopes_and_runtime_observer_text_never_receive_fabricated_edits() {
    let dir = project_with_lib();
    for source in [
        b"\xef\xbb\xbf# encoding: utf-8\r\neach {}\r\nclass Foo\r\nend\r\np __LINE__\r\n__END__\r\nraw\x00\r\n".as_slice(),
        b"# encoding: Windows-1252\np '\xe9'; each {}\nclass Foo; end\n",
        b"x = <<~TEXT\neach {}\nclass Foo; end\nTEXT\neach {}\nclass Foo; end\n",
        b"each {}; class Foo; end; File.binread(__FILE__); DATA.pos\n__END__\nraw\n",
        b"each {}; class Foo; end; RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
    ] {
        let out = unchanged(dir.path(), source, &["--only", BOTH, "--fix", "lib/a.rb"], 1);
        assert!(count(&out, BLOCK) >= 1, "{out:?}");
        assert_eq!(count(&out, CLASS), 1, "{out:?}");
    }
}

#[test]
fn invalid_source_atomic_failure_and_readonly_symlink_inputs_preserve_bytes_and_permissions() {
    let dir = project_with_lib();
    let invalid = b"each {}\nclass Foo; end\ndef broken(\n";
    let out = unchanged(
        dir.path(),
        invalid,
        &["--only", BOTH, "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!((count(&out, BLOCK), count(&out, CLASS)), (0, 0));
    let missing = run(dir.path(), &["--only", BOTH, "--fix", "missing.rb"]);
    assert_eq!(missing.status.code(), Some(2));
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, b"each {}\nclass Foo; end\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(
            dir.path(),
            &["--only", BOTH, "--fix", "lib/alias.rb", "lib/a.rb"],
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(count(&out, BLOCK) >= 1 && count(&out, CLASS) >= 1);
        assert_eq!(fs::read(&path).unwrap(), b"each {}\nclass Foo; end\n");
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
