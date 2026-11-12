//! Actual CLI behaviour of `Layout/IndentationWidth` partial
//! negative shifts. A shift that leaves a less-indented keyword line behind
//! while moving its native partner would erase Ruby 3.4's
//! mismatched-indentation warning, so the finding stays and the file is not
//! written. Native checks only syntax-check (`-w -c`, never run) these
//! authored controls.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const COP: &str = "Layout/IndentationWidth";

/// (source, findings) whose correction is withheld; each warns natively.
const WITHHELD: &[(&str, usize)] = &[
    // The exact reproducer.
    ("if true\n          if true\n  1\n  end\nend\n", 2),
    ("class A\n      def x\n  end\nend\n", 1),
    ("unless a\n      case b\n  when c\n  end\nend\n", 1),
    (
        "if a\n      if b\n        c\n  elsif d\n        e\n      end\nend\n",
        2,
    ),
    ("if a\n      x = [\n  if y\n      end]\nend\n", 1),
    ("def x\n      a\n    rescue\n  b\nend\n", 2),
];

/// (before, after) partial or complete shifts that stay corrected and keep
/// the native warning output.
const CORRECTED: &[(&str, &str)] = &[
    (
        "if true\n          if true\n            1\n          end\nend\n",
        "if true\n  if true\n    1\n  end\nend\n",
    ),
    (
        "def f\n      if x\n  y\n      end\nend\n",
        "def f\n  if x\n  y\n  end\nend\n",
    ),
    (
        "def x\n      a\n  rescue\n      b\nend\n",
        "def x\n  a\n  rescue\n  b\nend\n",
    ),
    (
        "do_something do\n    foo\nensure\n  handle_error\nend\n",
        "do_something do\n  foo\nensure\nhandle_error\nend\n",
    ),
];

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
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

/// Native Ruby 3.4 `-w -c` diagnostics of `example.rb`, when `RUSTOCOP_RUBY34`
/// is configured.
fn warnings(root: &Path) -> Option<String> {
    let ruby = crate::support::runtime::ruby().ok()?;
    let out = Command::new(ruby)
        .current_dir(root)
        .args(["-w", "-c", "example.rb"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"Syntax OK\n");
    Some(String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn withheld_partial_shifts_keep_findings_warnings_and_file_identity() {
    let dir = project();
    let path = dir.path().join("example.rb");
    for &(source, findings) in WITHHELD {
        fs::write(&path, source).unwrap();
        let native = warnings(dir.path());
        if let Some(native) = &native {
            assert!(
                native.contains("warning: mismatched indentations"),
                "{source:?}: {native}"
            );
        }
        let old = fs::metadata(&path).unwrap();
        for args in [
            &["--only", COP, "example.rb"][..],
            &["--only", COP, "--fix", "example.rb"],
        ] {
            let out = run(dir.path(), args);
            assert_eq!(out.status.code(), Some(1), "{source:?} {args:?} {out:?}");
            assert_eq!(count(&out), findings, "{source:?} {args:?} {out:?}");
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source:?}");
            let new = fs::metadata(&path).unwrap();
            assert_eq!(old.modified().unwrap(), new.modified().unwrap());
            assert_eq!(old.permissions(), new.permissions());
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(old.ino(), new.ino());
            }
        }
        assert_eq!(warnings(dir.path()), native, "{source:?}");
    }
}

#[test]
fn warning_neutral_shifts_stay_corrected_with_identical_native_warnings() {
    let dir = project();
    let path = dir.path().join("example.rb");
    for &(before, after) in CORRECTED {
        fs::write(&path, before).unwrap();
        let native = warnings(dir.path());
        let out = run(
            dir.path(),
            &["--only", COP, "--fix", "--fix-passes", "1", "example.rb"],
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(" applied, "),
            "{before:?} {out:?}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), after, "{before:?}");
        assert_eq!(warnings(dir.path()), native, "{before:?}");
    }
}

#[test]
fn the_reproducer_composes_to_the_canonical_layout() {
    let dir = project();
    let path = dir.path().join("example.rb");
    fs::write(&path, WITHHELD[0].0).unwrap();
    let mut runs = 0;
    loop {
        runs += 1;
        assert!(
            runs <= 6,
            "no fixed point: {}",
            fs::read_to_string(&path).unwrap()
        );
        let out = run(dir.path(), &["--only", "Layout", "--fix", "example.rb"]);
        if out.status.code() == Some(0) {
            break;
        }
        assert_eq!(out.status.code(), Some(1), "{out:?}");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "if true\n  if true\n    1\n  end\nend\n"
    );
    if let Some(native) = warnings(dir.path()) {
        assert_eq!(native, "");
    }
}
