//! Original-byte source-only pinned parity and rebuilt CLI controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/InterpolationCheck";
const SOURCE: &[u8] = b"value = '#{one + two}'\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/interpolation_check.json")).unwrap();
    assert_eq!(rows.len(), 7538);
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
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({COP})"))
        .count()
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: i32) -> Output {
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
    assert_eq!(
        out.status.code(),
        Some(status),
        "{}: {out:?}",
        String::from_utf8_lossy(bytes)
    );
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
fn all_pinned_main_source_and_original_byte_controls_match_pipeline() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let mut failures = Vec::new();
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}: {report:?}"
        );
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
                "{}: expected {expected:?}, found {found:?}",
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
fn all_original_bytes_match_rebuilt_cli_and_fix_never_writes() {
    parallel_rows!(for row in rows() => {
        let dir = project();
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let expected: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            let out = unchanged(dir.path(), &bytes, &args, i32::from(!expected.is_empty()));
            assert_eq!(count(&out), expected.len(), "{row}: {out:?}");
            let mut stdout = String::new();
            for (line, col, _, message) in &expected {
                stdout.push_str(&format!("lib/a.rb:{line}:{col}: E: {message} ({COP})\n"));
            }
            let n = expected.len();
            let plural = if n == 1 { "" } else { "s" };
            let summary = if fix {
                format!("0 corrections applied, {n} offense{plural} remaining")
            } else if n == 0 {
                "no offenses detected".into()
            } else {
                format!("{n} offense{plural} detected")
            };
            stdout.push_str(&format!("\n1 file inspected, {summary}\n"));
            assert_eq!(String::from_utf8_lossy(&out.stdout), stdout, "{row}");
            assert!(out.stderr.is_empty(), "{row}: {out:?}");
        }
    });
}
#[test]
fn defaults_selectors_strict_toml_department_and_exclusion_precedence() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(
        cop.ruby_versions(),
        rustocop::cops::policy::RubyVersions::default()
    );
    let dir = project();
    let out = unchanged(dir.path(), SOURCE, &["lib/a.rb"], 1);
    assert_eq!(count(&out), 1);
    let out = unchanged(
        dir.path(),
        SOURCE,
        &["--only", COP, "--except", COP, "lib/a.rb"],
        0,
    );
    assert_eq!(count(&out), 0);
    for (toml, flags, expected) in [
        ("[cops.Lint]\nenabled = false\n", false, 0),
        (
            "[cops.Lint]\nenabled = false\n[cops.\"Lint/InterpolationCheck\"]\nenabled = true\n",
            false,
            1,
        ),
        (
            "[cops.Lint]\nenabled = true\n[cops.\"Lint/InterpolationCheck\"]\nenabled = false\n",
            false,
            0,
        ),
        (
            "[cops.\"Lint/InterpolationCheck\"]\nenabled = false\n",
            true,
            1,
        ),
        (
            "[cops.\"Lint/InterpolationCheck\"]\nexclude = [\"lib/**\"]\n",
            true,
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), toml).unwrap();
        let args = if flags {
            vec!["--only", COP, "lib/a.rb"]
        } else {
            vec!["lib/a.rb"]
        };
        let status = if toml.starts_with("[cops.Lint]\nenabled = true") {
            1
        } else {
            expected
        };
        let out = unchanged(dir.path(), SOURCE, &args, status);
        assert_eq!(count(&out), expected as usize, "{toml}: {out:?}");
    }
    for toml in [
        "[cops.\"Lint/InterpolationCheck\"]\nmax = 1\n",
        "[cops.\"Lint/InterpolationCheck\"]\nEnforcedStyle = 'double'\n",
        "[cops.\"Lint/InterpolationCheck\"]\ninclude = ['**']\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), toml).unwrap();
        unchanged(dir.path(), SOURCE, &["--only", COP, "--fix", "lib/a.rb"], 2);
    }
}
#[test]
fn both_directive_prefixes_multiline_next_and_push_scopes() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\nvalue = '#{{a}}'\n"),
            format!("value = '#{{a}}' # {prefix}:disable {COP}\n"),
            format!("# {prefix}:disable-next {COP}\nvalue = '#{{a}}'\n"),
            format!("# {prefix}:disable Lint\nvalue = '#{{a}}'\n"),
            format!(
                "# {prefix}:push\n# {prefix}:disable {COP}\nvalue = '#{{a}}'\n# {prefix}:pop\n"
            ),
            format!("value = '#{{a}}\nend' # {prefix}:disable {COP}\n"),
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!(count(&out), 0, "{source}: {out:?}");
        }
        let source = format!(
            "# {prefix}:push\n# {prefix}:disable {COP}\nvalue = '#{{a}}'\n# {prefix}:pop\nvalue = '#{{b}}'\n"
        );
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!(count(&out), 1);
    }
}
#[test]
fn stdin_virtual_paths_have_no_fixed_path_gate_and_never_write() {
    let dir = project();
    for path in [
        "virtual.rb",
        "lib/template.jb",
        "vendor/virtual.rb",
        "db/schema.rb",
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        // Built-in source path exclusions remain mandatory even with --only;
        // this cop adds no Include/Exclude policy of its own.
        let excluded = path.starts_with("vendor/") || path == "db/schema.rb";
        assert_eq!(
            out.status.code(),
            Some(i32::from(!excluded)),
            "{path}: {out:?}"
        );
        assert_eq!(count(&out), usize::from(!excluded));
        if !excluded {
            assert!(String::from_utf8_lossy(&out.stdout).starts_with(&format!("{path}:1:9:")));
        }
        assert!(!dir.path().join(path).exists());
    }
    let sentinel = dir.path().join("lib/a.rb");
    fs::write(&sentinel, b"sentinel\n").unwrap();
    let old = fs::metadata(&sentinel).unwrap();
    for excluded in [false, true] {
        fs::write(
            dir.path().join("rustocop.toml"),
            if excluded {
                "exclude = ['lib/a.rb']\n"
            } else {
                ""
            },
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), usize::from(!excluded));
        assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel\n");
        let new = fs::metadata(&sentinel).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
    }
    let out = run(
        dir.path(),
        &["--only", COP, "--stdin", "virtual.rb", "--fix"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(!dir.path().join("virtual.rb").exists());
}
#[test]
fn original_fixture_sources_pass_native_ruby34_syntax_only_never_execute_interpolation() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-W0", "-c"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}: {out:?}");
    }
}
#[test]
fn individual_when_not_conversions_are_fixed34_reference_grammar_differences() {
    if crate::support::runtime::skips("asserts the Ruby 3.4 grammar, which Ruby 4.0 changed") {
        return;
    }
    // NOT ordinary parity fixtures or baseline waivers: pinned Prism1.9 rejects
    // each exact converted literal, whereas native fixed Ruby3.4 accepts it.
    // The Ruby tool checks each main source, candidate and reference diagnostic.
    let exclusions: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/interpolation_check_parser_exclusions.json"
    ))
    .unwrap();
    assert_eq!(exclusions.len(), 12);
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let dir = project();
    for row in exclusions {
        let source = row["source"].as_str().unwrap().as_bytes();
        let expected: Vec<(u32, u32, String)> =
            serde_json::from_value(row["fixed_findings"].clone()).unwrap();
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(source),
            &active,
            &config,
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        let found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                (o.line, o.col, o.message.clone())
            })
            .collect();
        assert_eq!(found, expected, "{row}");
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            let out = unchanged(dir.path(), source, &args, 1);
            assert_eq!(count(&out), expected.len(), "{row}");
            assert!(out.stderr.is_empty());
            for (line, col, message) in &expected {
                assert!(
                    String::from_utf8_lossy(&out.stdout)
                        .contains(&format!("lib/a.rb:{line}:{col}: E: {message} ({COP})\n"))
                );
            }
        }
        if let Ok(ruby) = crate::support::runtime::ruby() {
            for bytes in [source, row["converted"].as_str().unwrap().as_bytes()] {
                let mut child = Command::new(&ruby)
                    .args(["--parser=prism", "-W0", "-c"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(bytes).unwrap();
                let out = child.wait_with_output().unwrap();
                assert!(out.status.success(), "{row}: {out:?}");
            }
        }
    }
}

#[test]
fn parser_failure_peer_composition_and_symlink_controls_keep_bytes() {
    let dir = project();
    let invalid = b"value = '#{a}'\ndef broken(\n";
    let out = unchanged(
        dir.path(),
        invalid,
        &[
            "--only",
            COP,
            "--except",
            "Lint/Syntax",
            "--fix",
            "lib/a.rb",
        ],
        2,
    );
    assert_eq!(count(&out), 0);
    let out = unchanged(
        dir.path(),
        SOURCE,
        &[
            "--only",
            "Lint/InterpolationCheck,Style/StringLiterals",
            "--fix",
            "lib/a.rb",
        ],
        1,
    );
    assert_eq!(count(&out), 1);
    #[cfg(unix)]
    {
        let target = dir.path().join("lib/a.rb");
        std::os::unix::fs::symlink(&target, dir.path().join("link.rb")).unwrap();
        let old = fs::metadata(&target).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(fs::read(&target).unwrap(), SOURCE);
        assert_eq!(
            fs::metadata(&target).unwrap().modified().unwrap(),
            old.modified().unwrap()
        );
        assert!(
            fs::symlink_metadata(dir.path().join("link.rb"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
