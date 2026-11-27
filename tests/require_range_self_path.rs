//! Source-only pinned AST and CLI checks. Never execute reference inputs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{count, project_with_lib, run};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = [
    "Lint/RequireRangeParentheses",
    "Lint/RequireRelativeSelfPath",
];
const SOURCES: [&[u8]; 2] = [b"x = 42..\nvalue\n", b"require_relative 'foo'\n"];
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/require_range_self_path.json")).unwrap();
    assert_eq!(rows.len(), 6457);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        34
    );
    rows
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/foo.rb");
    fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    }
    let mut held = fs::File::open(&path).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(
            out.status.code(),
            Some(status),
            "{}: {out:?}",
            String::from_utf8_lossy(bytes)
        );
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let mut held_bytes = Vec::new();
    held.read_to_end(&mut held_bytes).unwrap();
    assert_eq!(held_bytes, bytes);
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
fn pinned_main_source_original_bytes_and_parent_shapes_match_without_edits() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let mut failures = Vec::new();
    for row in rows() {
        let cop = row["cop"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let report = rustocop::pipeline::lint_one(
            Path::new(row["path"].as_str().unwrap()),
            Path::new(row["path"].as_str().unwrap()),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        if row.get("parser_rejection").is_some() {
            assert!(
                !report.errors.is_empty(),
                "reference rejection unexpectedly accepted: {row}: {report:?}"
            );
            assert!(report.result.offenses.is_empty());
            continue;
        }
        assert!(report.errors.is_empty(), "{row}: {report:?}");
        let mut found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Warning);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        found.sort();
        let expected: Vec<Finding> = crate::support::fixture::value(&row["findings"]);
        if found != expected {
            failures.push(format!(
                "{cop} {}: expected {expected:?}, found {found:?}",
                String::from_utf8_lossy(&bytes)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
#[test]
fn binary_message_bytes_are_explicit_not_lossy_replacements() {
    let source = b"# coding: ASCII-8BIT\nx = foo(# \xe9\n bar)..\nvalue\n";
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COPS[0].into()], &[]);
    let report = rustocop::pipeline::lint_one(
        Path::new("foo.rb"),
        Path::new("foo.rb"),
        Some(source),
        &active,
        &config,
    );
    assert!(report.errors.is_empty() && report.internal_errors.is_empty());
    assert_eq!(report.result.offenses.len(), 1);
    let finding = &report.result.offenses[0];
    assert_eq!((finding.line, finding.col, finding.end_line), (2, 5, 4));
    assert_eq!(
        finding.message,
        "Wrap the range literal `foo(# \\xE9\n bar)..` in parentheses to avoid confusion with an endless range."
    );
    assert!(!finding.message.contains('\u{fffd}'));
    let dir = project_with_lib();
    unchanged(
        dir.path(),
        source,
        &["--only", COPS[0], "--fix", "lib/foo.rb"],
        Some(1),
    );
}
#[test]
fn defaults_selectors_strict_toml_department_exclusions_and_version_policy() {
    let registry = rustocop::cops::Registry::new();
    let dir = project_with_lib();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let detector = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(detector.default_enabled());
        assert_eq!(detector.safe_fixes(), cop == COPS[1]);
        assert_eq!(
            detector.ruby_versions(),
            rustocop::cops::policy::RubyVersions {
                minimum: None,
                maximum: None
            }
        );
        assert!(detector.path_policy().is_none() && detector.max_setting().is_none());
        assert!(detector.autocorrect_incompatible_with().is_empty());
        for (config, mut args, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint", "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", "Lint"], 0),
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
            (
                format!("[cops.\"{cop}\"]\nexclude = ['lib/foo.rb']\n"),
                vec!["--only", cop],
                0,
            ),
            ("exclude = ['lib/foo.rb']\n".into(), vec!["--only", cop], 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            args.push("lib/foo.rb");
            assert_eq!(count(&unchanged(dir.path(), source, &args, None), cop), n);
        }
        for key in [
            "max = 1",
            "Include = ['**/*']",
            "Enabled = true",
            "enabled = 'true'",
            "TargetRubyVersion = 3.3",
            "SafeAutoCorrect = true",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                source,
                &["--only", cop, "--fix", "lib/foo.rb"],
                Some(2),
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for path in ["vendor/foo.rb", "db/schema.rb"] {
            let file = dir.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, source).unwrap();
            assert_eq!(
                count(&run(dir.path(), &["--only", cop, "--fix", path]), cop),
                0
            );
            assert_eq!(fs::read(file).unwrap(), source);
        }
    }
}
#[test]
fn canonical_whole_line_removal_is_atomic_byte_preserving_and_idempotent() {
    let dir = project_with_lib();
    let cop = COPS[1];
    for row in rows().into_iter().filter(|r| r.get("after").is_some()) {
        let before: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let after: Vec<u8> = crate::support::fixture::value(&row["after"]);
        unchanged(dir.path(), &before, &["--only", cop, "lib/foo.rb"], Some(1));
        let path = dir.path().join("lib/foo.rb");
        let old = fs::metadata(&path).unwrap();
        let mut held = fs::File::open(&path).unwrap();
        let out = run(dir.path(), &["--only", cop, "--fix", "lib/foo.rb"]);
        assert_eq!(out.status.code(), Some(0), "{row}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 correction applied"));
        assert_eq!(fs::read(&path).unwrap(), crate::support::lf::lf(&after));
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        let mut original = Vec::new();
        held.read_to_end(&mut original).unwrap();
        assert_eq!(original, crate::support::lf::lf(&after));
        unchanged(
            dir.path(),
            &after,
            &["--only", cop, "--fix", "lib/foo.rb"],
            Some(0),
        );
    }
}
#[test]
fn native_syntax_only_all_original_inputs_never_execute_snippets() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        for source in std::iter::once(bytes).chain(
            row.get("after")
                .map(crate::support::fixture::value::<Vec<u8>>),
        ) {
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-c", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                row.get("parser_rejection").is_none(),
                "{row}: {out:?}"
            );
        }
    }
}
#[test]
fn authored_bounded_native_warning_value_binding_dispatch_and_observation_controls() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let execute = |s: &str, syntax: bool| {
        let mut command = Command::new(&ruby);
        command.arg("--parser=prism").arg("-w");
        if syntax {
            command.arg("-c");
        }
        command
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    };
    // Authored controls, NOT reference/spec snippets. These paired controls
    // never invoke the loader; the final bounded self-loader uses only its own
    // separately authored scratch file.
    let before = "if false\n  require_relative 'native_control'\nend\n";
    let after = "if false\nend\n";
    for syntax in [false, true] {
        let a = execute(before, syntax);
        let b = execute(after, syntax);
        assert!(a.status.success() && b.status.success());
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
    }
    let wrap = |s: &str| {
        format!(
            "$events = []\ndef require_relative(*args); $events << args; raise 'must not dispatch'; end\nvalue = begin\n{s}end\np [value, local_variables.sort, $events, $LOADED_FEATURES.length]\n"
        )
    };
    let a = execute(&wrap(before), false);
    let b = execute(&wrap(after), false);
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
    // Live calls may be user-defined: deleting one changes effects and value.
    let a = execute(
        "def require_relative(x); puts x; false; end\np require_relative('authored')\n",
        false,
    );
    let b = execute(
        "def require_relative(x); puts x; false; end\np nil\n",
        false,
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    // Source line/code object observation distinguishes even inert deletion.
    let a = execute(&format!("{before}p __LINE__\n"), false);
    let b = execute(&format!("{after}p __LINE__\n"), false);
    assert_ne!(a.stdout, b.stdout);
    let probe = |s: &str| {
        format!(
            "RubyVM.keep_script_lines = true\np RubyVM::InstructionSequence.compile({s:?}).script_lines\n"
        )
    };
    let a = execute(&probe(before), false);
    let b = execute(&probe(after), false);
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    // A real authored self-load can recurse and changes require's Boolean
    // return and loaded features. No reference source or application file runs.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authored_self.rb");
    let source = "if $rustocop_authored_self_seen\n puts :nested\nelse\n $rustocop_authored_self_seen = true\n p require_relative 'authored_self'\nend\np $LOADED_FEATURES.grep(/authored_self.rb$/).length\n";
    fs::write(&path, source).unwrap();
    let a = Command::new(&ruby)
        .arg("--parser=prism")
        .arg(&path)
        .output()
        .unwrap();
    fs::write(
        &path,
        source.replace(" p require_relative 'authored_self'\n", ""),
    )
    .unwrap();
    let b = Command::new(&ruby)
        .arg("--parser=prism")
        .arg(&path)
        .output()
        .unwrap();
    assert!(a.status.success() && b.status.success(), "{a:?}: {b:?}");
    assert_eq!(String::from_utf8_lossy(&a.stdout), "nested\n0\ntrue\n1\n");
    assert_eq!(String::from_utf8_lossy(&b.stdout), "0\n");
}
