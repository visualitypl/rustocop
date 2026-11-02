//! Source-only reference, pipeline and rebuilt CLI checks.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
//! Never execute constructors, callbacks or other fixture input.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/MissingSuper";
const SOURCE: &[u8] = b"class Child < Parent\n  def initialize\n  end\nend\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/missing_super.json")).unwrap();
    assert_eq!(rows.len(), 3639);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        198
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
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({COP})"))
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
fn pinned_main_source_and_adversarial_original_bytes_match_pipeline_without_edits() {
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
    let dir = project();
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            if row.get("parser_rejection").is_some() {
                assert_eq!(count(&unchanged(dir.path(), &bytes, &args, Some(2))), 0);
                continue;
            }
            let expected: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
            let n = expected.len();
            let out = unchanged(dir.path(), &bytes, &args, Some(i32::from(n != 0)));
            let mut stdout = String::new();
            for (line, col, _, message) in &expected {
                stdout.push_str(&format!("lib/a.rb:{line}:{col}: E: {message} ({COP})\n"));
            }
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
    }
}
#[test]
fn default_metadata_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    let detector = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(detector.default_enabled());
    assert!(!detector.safe_fixes());
    assert_eq!(detector.ruby_versions(), Default::default());
    assert!(detector.path_policy().is_none() && detector.max_setting().is_none());
    assert!(detector.autocorrect_incompatible_with().is_empty());
    let dir = project();
    for (config, mut args, n) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
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
        (
            format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
            vec!["--only", COP],
            0,
        ),
        ("exclude = ['lib/a.rb']\n".into(), vec!["--only", COP], 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(dir.path(), SOURCE, &args, None)), n);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "TargetRubyVersion = 3.3",
        "SafeAutoCorrect = true",
        "AllowedParentClasses = ['Array']",
        "UseProjectIndex = true",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
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
        SOURCE,
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
}
#[test]
fn supplied_paths_stdin_exclusions_and_no_writes() {
    let dir = project();
    for config in [
        String::new(),
        "exclude = ['lib/a.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
    ] {
        let n = usize::from(config.is_empty());
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        assert_eq!(
            count(&unchanged(
                dir.path(),
                SOURCE,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(n as i32)
            )),
            n
        );
        let out = run(&dir.path().join("lib"), &["--only", COP, "a.rb"]);
        assert_eq!(count(&out), n);
        if n != 0 {
            assert!(String::from_utf8_lossy(&out.stdout).starts_with("lib/a.rb:"));
        }
        fs::write(dir.path().join("lib/a.rb"), b"sentinel\n").unwrap();
        let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), n);
        if n != 0 {
            assert!(String::from_utf8_lossy(&out.stdout).starts_with("lib/a.rb:"));
        }
        assert_eq!(
            fs::read(dir.path().join("lib/a.rb")).unwrap(),
            b"sentinel\n"
        );
        let new = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
    }
    assert_eq!(
        run(dir.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for path in ["Gemfile", "lib/a.jb", "lib/.hidden.rb", "lib/a_spec.rb"] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        assert_eq!(count(&run(dir.path(), &["--only", COP, path])), 1);
    }
}
#[test]
fn both_directive_prefixes_multiline_next_push_and_whole_def_end_range() {
    let dir = project();
    let source = std::str::from_utf8(SOURCE).unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for text in [
            format!("# {prefix}:disable Lint\n{source}"),
            format!("# {prefix}:disable-next {COP}\n{source}"),
            format!("# {prefix}:next -{COP}\n{source}"),
            format!("# {prefix}:push -{COP}\n{source}# {prefix}:pop\n"),
            format!(
                "class C < Parent\n def initialize(\n a,\n b: 1 # {prefix}:disable {COP}\n )\n body\n end\nend"
            ),
            format!("class C < Parent\n def initialize\n body\n end # {prefix}:disable {COP}\nend"),
        ] {
            unchanged(
                dir.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        for (open, close) in [
            (
                format!("# {prefix}:push +{COP}\n"),
                format!("# {prefix}:pop\n"),
            ),
            (format!("# {prefix}:enable-next {COP}\n"), String::new()),
        ] {
            let text = format!("# {prefix}:disable {COP}\n{source}{open}{source}{close}{source}");
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    text.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(1)
                )),
                1
            );
        }
        let late =
            format!("class C < Parent\n def initialize\n end\nend # {prefix}:disable {COP}\n");
        unchanged(
            dir.path(),
            late.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
}
#[test]
fn observer_bytes_parser_gate_peer_composition_and_symlink_atomic_failure() {
    let dir = project();
    for suffix in [
        "p __LINE__; File.binread(__FILE__); DATA.pos\n__END__\nraw\0\n",
        "RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
        "p __FILE__, __ENCODING__\n",
    ] {
        let text = format!(
            "\u{feff}# coding: utf-8\r\n{}{}",
            std::str::from_utf8(SOURCE).unwrap().replace('\n', "\r\n"),
            suffix.replace('\n', "\r\n")
        );
        assert_eq!(
            count(&unchanged(
                dir.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
    }
    let mut invalid = SOURCE.to_vec();
    invalid.extend_from_slice(b"def broken(\n");
    unchanged(
        dir.path(),
        &invalid,
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    let peers = format!("{COP},Style/StringLiterals,Layout/SpaceAfterComma");
    let mut before = SOURCE.to_vec();
    before.extend_from_slice(b"a = ['a',2]\n");
    let mut after = SOURCE.to_vec();
    after.extend_from_slice(b"a = [\"a\", 2]\n");
    unchanged(
        dir.path(),
        &before,
        &["--only", &peers, "lib/a.rb"],
        Some(1),
    );
    let out = run(dir.path(), &["--only", &peers, "--fix", "lib/a.rb"]);
    assert_eq!(count(&out), 1);
    let path = dir.path().join("lib/a.rb");
    assert_eq!(fs::read(&path).unwrap(), after);
    unchanged(
        dir.path(),
        &after,
        &["--only", &peers, "--fix", "lib/a.rb"],
        Some(1),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        symlink(&path, dir.path().join("link.rb")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", "link.rb"]);
        assert_eq!(count(&out), 1);
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&path, &before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", &peers, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), 1);
        assert_eq!(fs::read(&path).unwrap(), before);
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert!(
            fs::symlink_metadata(dir.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
#[test]
fn native_ruby34_syntax_only_never_execute_constructors_or_callbacks() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("ruby 3.4."));
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
        assert_eq!(
            out.status.success(),
            row.get("parser_rejection").is_none(),
            "{row}: {out:?}"
        );
    }
}
