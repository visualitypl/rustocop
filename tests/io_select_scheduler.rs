//! Source-only pin parity, original-byte CLI and detection-only safety.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/IncompatibleIoSelectWithFiberScheduler";
const SOURCE: &[u8] = b"IO.select([io], [], [], timeout)\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<_> =
        serde_json::from_str(include_str!("fixtures/io_select_scheduler.json")).unwrap();
    assert_eq!(rows.len(), 730);
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
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(out.status.code(), Some(status), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
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
fn all_reference_original_byte_shapes_match_pipeline_exactly_without_corrections() {
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
fn all_reference_inputs_match_rebuilt_cli_exactly_and_fix_never_writes() {
    let dir = project();
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let expected: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
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
            assert_eq!(count(&out), expected.len(), "{row}: {out:?}");
            let stdout = String::from_utf8_lossy(&out.stdout);
            for (line, col, _, message) in &expected {
                assert!(
                    stdout.contains(&format!("lib/a.rb:{line}:{col}: E: {message} ({COP})\n")),
                    "{row}: {out:?}"
                );
            }
        }
    }
}
#[test]
fn defaults_metadata_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    let dir = project();
    for (config, mut args, expected) in [
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
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(dir.path(), SOURCE, &args, None)), expected);
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
}
#[test]
fn supplied_path_and_stdin_provenance_excludes_and_no_fixed_path_gate() {
    let dir = project();
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
    for config in [
        String::new(),
        "exclude = ['lib/a.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
    ] {
        let n = usize::from(config.is_empty());
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let out = unchanged(
            dir.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(n as i32),
        );
        assert_eq!(count(&out), n);
        assert_eq!(
            count(&run(&dir.path().join("lib"), &["--only", COP, "a.rb"])),
            n
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
        assert_eq!(count(&out), n);
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
fn both_directive_prefixes_multiline_next_push_and_enables() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Lint\nIO.select([io])\n"),
            format!("# {prefix}:disable-next {COP}\nIO.select(\n[io]\n)\n"),
            format!("# {prefix}:next -{COP}\nIO.select([io])\n"),
            format!("IO.select(\n[io]\n) # {prefix}:disable {COP}\n"),
            format!("# {prefix}:push -{COP}\nIO.select([io])\n# {prefix}:pop\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
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
            let source = format!(
                "# {prefix}:disable {COP}\nIO.select([io])\n{open}IO.select([io])\n{close}IO.select([io])\n"
            );
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(1)
                )),
                1
            );
        }
    }
}
#[test]
fn observers_parser_failures_composition_permissions_and_atomic_symlink_failure() {
    let dir = project();
    for source in [
        b"\xef\xbb\xbf# coding: utf-8\r\nIO.select([io])\r\np __LINE__\r\n__END__\r\nDATA\x00\r\n".as_slice(),
        b"# coding: Windows-1252\np '\xe9'; IO.select([io])\n",
        b"x = <<~TEXT\nIO.select([text])\nTEXT\nIO.select([io])\n",
        b"IO.select([io]); File.binread(__FILE__); DATA.pos\n__END__\nraw\n",
        b"IO.select([io]); RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
    ] { assert_eq!(count(&unchanged(dir.path(), source, &["--only", COP, "--fix", "lib/a.rb"], Some(1))), 1); }
    for source in [
        b"IO.select([io]); def broken(\n".as_slice(),
        b"def f; IO.select([io], [], (return), nil); end\n",
    ] {
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(2)
            )),
            0
        );
    }
    let peers = format!("{COP},Style/StringLiterals,Layout/SpaceAfterComma");
    let before = b"IO.select([io])\nx = ['a',2]\n";
    let after = b"IO.select([io])\nx = [\"a\", 2]\n";
    let path = dir.path().join("lib/a.rb");
    unchanged(dir.path(), before, &["--only", &peers, "lib/a.rb"], Some(1));
    assert_eq!(
        count(&run(dir.path(), &["--only", &peers, "--fix", "lib/a.rb"])),
        1
    );
    assert_eq!(fs::read(&path).unwrap(), after);
    unchanged(
        dir.path(),
        after,
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
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(count(&out), 1);
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&path, before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", &peers, "--fix", "link.rb"]);
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
fn native_ruby34_syntax_checks_all_original_bytes_without_executing_io() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("ruby 3.4."));
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let mut child = Command::new(&ruby)
            .args(["-c", "-W0"])
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
