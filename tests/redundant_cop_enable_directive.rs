//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, lint::RedundantCopEnableDirective},
    pipeline::lint_one,
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;
const COP: &str = "Lint/RedundantCopEnableDirective";
const BEFORE: &str = "foo\n# rubocop:enable Layout/LineLength\n";
const STDIN_FIX_ERROR: &str = concat!(
    "error: the argument '--stdin <PATH>' cannot be used with '--fix'\n\n",
    "Usage: rustocop --only <ONLY> --stdin <PATH> [PATHS]...\n\n",
    "For more information, try '--help'.\n"
);
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], code: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    out
}
#[test]
fn all_six_pinned_source_examples_use_actual_comment_lifecycle() {
    let rows: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/redundant_cop_enable_directive_source.json"
    ))
    .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 6);
    for row in rows.as_array().unwrap() {
        let source = row["source"].as_str().unwrap();
        let report = lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(source.as_bytes()),
            &[&RedundantCopEnableDirective],
            &ResolvedConfig::default(),
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        let actual: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| (o.line, o.col, o.message.as_str()))
            .collect();
        let expected: Vec<_> = row["positions"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["messages"].as_array().unwrap())
            .map(|(p, m)| {
                (
                    p[0].as_u64().unwrap() as u32,
                    p[1].as_u64().unwrap() as u32,
                    m.as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(actual, expected, "{row:?}");
    }
}
#[test]
fn defaults_selectors_strict_toml_and_supplied_stdin_path() {
    let r = Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled() && c.safe_fixes());
    assert!(c.path_policy().is_none() && c.max_setting().is_none());
    assert_eq!(c.ruby_versions(), Default::default());
    let d = project();
    let p = d.path().join("lib/a.rb");
    fs::write(&p, BEFORE).unwrap();
    let old = fs::metadata(&p).unwrap();
    assert!(String::from_utf8_lossy(&cli(d.path(), &["lib/a.rb"], 1).stdout).contains(COP));
    for (config, args, code) in [
        (
            "",
            vec!["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
            0,
        ),
        (
            "[cops.Lint]\nenabled = false\n",
            vec!["--only", COP, "lib/a.rb"],
            1,
        ),
        (
            "[cops.Lint]\nenabled = false\n[cops.\"Lint/RedundantCopEnableDirective\"]\nenabled = true\n",
            vec!["lib/a.rb"],
            1,
        ),
        (
            "[cops.Lint]\nenabled = true\n[cops.\"Lint/RedundantCopEnableDirective\"]\nenabled = false\n",
            vec!["lib/a.rb"],
            0,
        ),
        (
            "[cops.\"Lint/RedundantCopEnableDirective\"]\nexclude = [\"lib/**\"]\n",
            vec!["--only", COP, "--fix", "lib/a.rb"],
            0,
        ),
        (
            "[cops.\"Lint/RedundantCopEnableDirective\"]\nmax = 3\n",
            vec!["--only", COP, "lib/a.rb"],
            2,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let out = cli(d.path(), &args, code);
        if args == ["lib/a.rb"] && code == 0 {
            assert!(!String::from_utf8_lossy(&out.stdout).contains(COP));
        }
        assert_eq!(fs::read(&p).unwrap(), BEFORE.as_bytes());
        assert_eq!(
            fs::metadata(&p).unwrap().modified().unwrap(),
            old.modified().unwrap()
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for (args, code) in [
        (vec!["--only", COP, "--stdin", "virtual/custom.rb"], 1),
        (vec!["--only", COP, "--stdin", "lib/a.rb", "--fix"], 2),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Only the asserted `--stdin --fix` rejection (exit 2) may close stdin
        // before reading; accepted stdin writes must succeed, with even
        // BrokenPipe remaining fatal.
        stdin_support::write_stdin(child.stdin.take().unwrap(), BEFORE.as_bytes(), code == 2)
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(code), "{out:?}");
        if code == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/custom.rb:2:18"));
            assert!(out.stderr.is_empty(), "{out:?}");
            assert!(!d.path().join("virtual/custom.rb").exists());
        } else {
            assert!(out.stdout.is_empty(), "{out:?}");
            assert_eq!(String::from_utf8(out.stderr).unwrap(), STDIN_FIX_ERROR);
        }
        assert_eq!(fs::read(&p).unwrap(), BEFORE.as_bytes());
        let new = fs::metadata(&p).unwrap();
        assert_eq!(new.modified().unwrap(), old.modified().unwrap());
        assert_eq!(new.permissions(), old.permissions());
    }
}
#[test]
fn stdin_fix_rejection_handles_a_deterministically_closed_pipe() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, BEFORE).unwrap();
    let old = fs::metadata(&path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "lib/a.rb", "--fix"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // Waiting with our pipe still open proves rejection needs neither input nor
    // EOF, and guarantees the later write hits a closed reader (no sleeps).
    let out = child.wait_with_output().unwrap();
    let write = stdin.write_all(BEFORE.as_bytes());
    assert_eq!(
        write.as_ref().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    stdin_support::write_stdin(stdin, BEFORE.as_bytes(), true).unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert_eq!(String::from_utf8(out.stderr).unwrap(), STDIN_FIX_ERROR);
    assert_eq!(fs::read(&path).unwrap(), BEFORE.as_bytes());
    let new = fs::metadata(&path).unwrap();
    assert_eq!(new.modified().unwrap(), old.modified().unwrap());
    assert_eq!(new.permissions(), old.permissions());
}
#[test]
fn shared_stdin_guard_preserves_other_errors_and_success() {
    /// Fails every write with one error kind, as a closed or broken stdin would.
    struct Failing(io::ErrorKind);
    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "stdin control"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut delivered = Vec::new();
    stdin_support::write_stdin(&mut delivered, BEFORE.as_bytes(), true).unwrap();
    assert_eq!(delivered, BEFORE.as_bytes());
    stdin_support::write_stdin(Failing(io::ErrorKind::BrokenPipe), b"x", true).unwrap();
    let err =
        stdin_support::write_stdin(Failing(io::ErrorKind::BrokenPipe), b"x", false).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    for kind in [
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::WriteZero,
        io::ErrorKind::Other,
    ] {
        let err = stdin_support::write_stdin(Failing(kind), b"x", true).unwrap_err();
        assert_eq!(err.kind(), kind);
        assert_eq!(err.to_string(), "stdin control");
    }
}
