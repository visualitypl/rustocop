//! Source-only pinned AST and CLI checks. Never execute fixture input.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = [
    "Lint/SafeNavigationConsistency",
    "Lint/SafeNavigationWithEmpty",
];
const SOURCES: [&[u8]; 2] = [b"x&.foo && x&.bar\n", b"return if x&.empty?\n"];
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/safe_navigation_consistency_empty.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 8264);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("project_after").is_some())
            .count(),
        25
    );
    rows
}
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
        .matches(&format!("({cop})"))
        .count()
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
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
fn pinned_main_source_original_bytes_and_safe_navigation_shapes_match_without_edits() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let mut failures = Vec::new();
    for row in rows() {
        let cop = row["cop"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
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
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        found.sort();
        let expected: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
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
fn mixed_newline_source_fixed_policy_rebuilt_cli_preserves_raw_bytes_and_metadata() {
    let dir = project();
    let cases: Vec<_> = rows()
        .into_iter()
        .filter(|r| r["origin"] == "newline")
        .collect();
    assert_eq!(cases.len(), 98);
    for row in cases {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let expected: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COPS[0]];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            let out = unchanged(
                dir.path(),
                &bytes,
                &args,
                Some(i32::from(!expected.is_empty())),
            );
            assert!(out.stderr.is_empty(), "{row}: {out:?}");
            assert_eq!(count(&out, COPS[0]), expected.len(), "{row}: {out:?}");
            let stdout = String::from_utf8(out.stdout).unwrap();
            for (line, col, _, message) in &expected {
                assert!(
                    stdout.contains(&format!(
                        "lib/a.rb:{line}:{col}: E: {message} ({})\n",
                        COPS[0]
                    )),
                    "{row}: {stdout}"
                );
            }
        }
    }
}
#[test]
fn defaults_selectors_toml_department_and_path_precedence() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
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
        assert_eq!(
            rustocop::offense::department_of(cop),
            rustocop::offense::Department::Lint
        );
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
                format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
                vec!["--only", cop],
                0,
            ),
            ("exclude = ['lib/a.rb']\n".into(), vec!["--only", cop], 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            args.push("lib/a.rb");
            assert_eq!(count(&unchanged(dir.path(), source, &args, None), cop), n);
        }
        for key in [
            "max = 1",
            "include = ['**/*']",
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
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(2),
            );
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            "[cops.Lint]\nexclude = ['lib/a.rb']\n",
        )
        .unwrap();
        unchanged(
            dir.path(),
            source,
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
}
#[cfg(unix)]
#[test]
fn actual_empty_canonical_atomic_readonly_symlink_and_idempotence() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let cop = COPS[1];
    let path = dir.path().join("lib/a.rb");
    let before = b"if false\n  if probe&.empty?\n  end\nend\n";
    fs::write(&path, before).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", cop, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(2)
    );
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", cop, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"if false\n  if probe && probe.empty?\n  end\nend\n"
    );
    assert_ne!(fs::metadata(&path).unwrap().ino(), old.ino());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(dir.path().join("lib/link.rb"))
            .unwrap()
            .is_symlink()
    );
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", cop, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(0)
    );
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap()),
        (new.ino(), new.modified().unwrap())
    );
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", cop, "--fix", "lib/dangling.rb"])
            .status
            .code(),
        Some(2)
    );
    assert!(!dir.path().join("lib/missing.rb").exists());
}

#[test]
fn authored_native_closed_canonical_preserves_vm_bindings_events_and_warnings() {
    if crate::support::runtime::skips(
        "pins compiled code or VM state that Ruby 4.0 builds differently",
    ) {
        return;
    }
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    // Authored bounded programs only. No fixture is evaluated. Both versions
    // compile the unexecuted predicate away; the full native instruction arrays,
    // locals, result and trace event surface agree, with no receiver dispatch.
    let driver = r#"
source = "if false\n  if probe&.empty?\n  end\nend\n"
source = source.sub('probe&.empty?', 'probe && probe.empty?') if ARGV[0] == 'after'
iseq = RubyVM::InstructionSequence.compile(source, 'closed.rb', 'closed.rb', 1)
events = []
trace = TracePoint.new(:line, :call, :return, :raise, :c_call, :c_return) { |t| events << [t.event, t.lineno, t.method_id] if t.path == 'closed.rb' }
trace.enable
result = iseq.eval
trace.disable
p [iseq.to_a, result, events, local_variables.sort]
"#;
    let evaluate = |mode: &str| {
        Command::new(&ruby)
            .args(["--parser=prism", "-w", "-e", driver, mode])
            .output()
            .unwrap()
    };
    let before = evaluate("before");
    let after = evaluate("after");
    assert!(
        before.status.success() && after.status.success(),
        "{before:?} {after:?}"
    );
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
    let dir = project();
    let text = b"if false\n  if probe&.empty?\n  end\nend\n";
    unchanged(dir.path(), text, &["--only", COPS[1], "lib/a.rb"], Some(1));
    let out = run(dir.path(), &["--only", COPS[1], "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read(dir.path().join("lib/a.rb")).unwrap(),
        b"if false\n  if probe && probe.empty?\n  end\nend\n"
    );
}

#[test]
fn native_ruby34_syntax_only_never_execute_reference_inputs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        for key in ["bytes", "project_after", "upstream_after"] {
            let Some(value) = row.get(key) else {
                continue;
            };
            let bytes: Vec<u8> = serde_json::from_value(value.clone()).unwrap();
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-W0", "-c"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                key != "bytes" || row.get("parser_rejection").is_none(),
                "{row}: {key}: {out:?}"
            );
        }
    }
}
