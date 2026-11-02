//! Exact source-only pinned fixtures, rebuilt CLI and no-edit controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/NextWithoutAccumulator";
const SOURCE: &[u8] = b"xs.reduce(0) { |acc, x| next if test; acc + x }\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<_> =
        serde_json::from_str(include_str!("fixtures/next_without_accumulator.json")).unwrap();
    assert_eq!(rows.len(), 1311);
    rows
}
fn expected(row: &serde_json::Value) -> Vec<Finding> {
    serde_json::from_value(row["findings"].clone()).unwrap()
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
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
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
fn all_original_byte_fixtures_match_exact_pipeline_or_explicit_parser_rejections() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let mut failures = Vec::new();
    for row in rows() {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{report:?}: {row}");
        if row.get("parser_rejection").is_some() || row.get("native_parser_rejection").is_some() {
            assert!(!report.errors.is_empty(), "{report:?}: {row}");
            assert!(report.result.offenses.is_empty());
            continue;
        }
        assert!(report.errors.is_empty(), "{report:?}: {row}");
        let mut found: Vec<_> = report
            .result
            .offenses
            .iter()
            .filter(|o| o.cop == COP)
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        found.sort();
        if found != expected(&row) {
            failures.push(format!(
                "{}: expected {:?}, found {found:?}",
                String::from_utf8_lossy(&source),
                expected(&row)
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
fn every_fixture_matches_exact_rebuilt_cli_lint_and_fix_without_writes() {
    let dir = project();
    for row in rows() {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let invalid =
            row.get("parser_rejection").is_some() || row.get("native_parser_rejection").is_some();
        let findings = if invalid { vec![] } else { expected(&row) };
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            let out = unchanged(
                dir.path(),
                &source,
                &args,
                if invalid {
                    2
                } else {
                    i32::from(!findings.is_empty())
                },
            );
            let mut found: Vec<_> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| l.ends_with(&format!("({COP})")))
                .map(str::to_owned)
                .collect();
            let mut expected: Vec<_> = findings
                .iter()
                .map(|(line, col, _, msg)| format!("lib/a.rb:{line}:{col}: E: {msg} ({COP})"))
                .collect();
            found.sort();
            expected.sort();
            assert_eq!(found, expected, "{row}\n{out:?}");
        }
    }
}
#[test]
fn detection_only_defaults_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 491);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
    let dir = project();
    for (config, selection, expected) in [
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
        let mut args = selection;
        args.push("lib/a.rb");
        fs::write(dir.path().join("lib/a.rb"), SOURCE).unwrap();
        let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
        let out = run(dir.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), expected, "{out:?}");
        assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), SOURCE);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(dir.path().join("lib/a.rb"))
                .unwrap()
                .modified()
                .unwrap()
        );
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "enabled = 'true'",
        "TargetRubyVersion = 3.3",
        "Style = 'other'",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(dir.path(), SOURCE, &["--only", COP, "--fix", "lib/a.rb"], 2);
    }
}
#[test]
fn paths_stdin_exclusions_and_both_directive_prefixes() {
    let dir = project();
    for (config, expected) in [
        (String::new(), 1),
        ("exclude = ['lib/a.rb']\n".into(), 0),
        (format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"), 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let out = unchanged(
            dir.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
            expected,
        );
        assert_eq!(count(&out), expected as usize);
        assert_eq!(
            count(&run(&dir.path().join("lib"), &["--only", COP, "a.rb"])),
            expected as usize
        );
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
        assert_eq!(count(&out), expected as usize);
        if expected == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:1:25: E:"));
        }
        assert_eq!(
            fs::read(dir.path().join("lib/a.rb")).unwrap(),
            b"sentinel\n"
        );
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(dir.path().join("lib/a.rb"))
                .unwrap()
                .modified()
                .unwrap()
        );
    }
    assert_eq!(
        run(dir.path(), &["--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for path in ["lib/a.jb", "Gemfile", "lib/.hidden.rb", "lib/a_spec.rb"] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        assert_eq!(count(&run(dir.path(), &["--only", COP, path])), 1);
    }
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!(
                "# {prefix}:disable Lint\n{}",
                String::from_utf8_lossy(SOURCE)
            ),
            format!(
                "xs.reduce(0) {{\n# {prefix}:disable-next {COP}\nnext if (\ntest\n)\nacc\n}}\n"
            ),
            format!("xs.reduce(0) {{ next if test; acc }} # {prefix}:disable {COP}\n"),
            format!(
                "# {prefix}:push -{COP}\n{}# {prefix}:pop\n",
                String::from_utf8_lossy(SOURCE)
            ),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "# {prefix}:disable {COP}\n{}# {prefix}:push +{COP}\n{}# {prefix}:pop\n{}",
            String::from_utf8_lossy(SOURCE),
            String::from_utf8_lossy(SOURCE),
            String::from_utf8_lossy(SOURCE)
        );
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                1
            )),
            1
        );
    }
}
#[test]
fn invalid_source_observers_comments_encoding_and_atomic_composition() {
    let dir = project();
    for source in [
        b"\xef\xbb\xbf# coding: utf-8\r\nxs.reduce(0) { next if test; acc }\r\np __LINE__\r\n__END__\r\nraw\x00\r\n".as_slice(),
        b"# coding: Windows-1252\nxs.reduce(0) { '\xe9'; next if test; acc }\n",
        b"x = <<~TEXT\nreduce(0) { next; acc }\nTEXT\nxs.reduce(0) { next if test; acc }\n",
        b"xs.reduce(0) { next if test; acc }; File.binread(__FILE__); DATA.pos; ObjectSpace.each_object {}; RubyVM::InstructionSequence.of(method(:f))\n__END__\nraw\n",
    ] { assert_eq!(count(&unchanged(dir.path(), source, &["--only", COP, "--fix", "lib/a.rb"], 1)), 1); }
    let invalid = b"reduce(0) { next; acc }\ndef broken(\n";
    assert_eq!(
        count(&unchanged(
            dir.path(),
            invalid,
            &["--only", COP, "--fix", "lib/a.rb"],
            2
        )),
        0
    );
    let path = dir.path().join("lib/a.rb");
    let before = b"xs.reduce(0) { next if test; acc }\nx = [1,2]\ny = obj.kind_of?(Integer)\n";
    fs::write(&path, before).unwrap();
    let both = "Lint/NextWithoutAccumulator,Layout/SpaceAfterComma,Style/ClassCheck";
    let out = run(dir.path(), &["--only", both, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out), 1);
    assert_eq!(
        fs::read(&path).unwrap(),
        b"xs.reduce(0) { next if test; acc }\nx = [1, 2]\ny = obj.is_a?(Integer)\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        symlink(&path, dir.path().join("link.rb")).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(count(&out), 1);
        assert!(
            fs::symlink_metadata(dir.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o444
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&path, before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", both, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), 1);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        assert!(
            fs::symlink_metadata(dir.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
#[test]
fn native_ruby34_syntax_only_checks_all_reference_inputs() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("ruby 3.4."));
    for row in rows() {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let mut child = Command::new(&ruby)
            .args(["-c", "-W0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&source).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            out.status.success(),
            row.get("parser_rejection").is_none() && row.get("native_parser_rejection").is_none(),
            "{row}\n{out:?}"
        );
    }
}
