//! Actual CLI behaviour of `Layout/IndentationWidth` negative
//! shifts over tab-containing indentation. Native Ruby 3.4 expands tabs to
//! 8-column stops, so removing trailing prefix bytes on a span's first line
//! and leading bytes on the others can move natively compared keywords by
//! different amounts. Such a shift keeps its finding and leaves the file
//! untouched; one that keeps every compared pair's native order stays
//! corrected. Native checks only syntax-check (`-w -c`, never run) these
//! authored controls.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const COP: &str = "Layout/IndentationWidth";

/// (source, findings, the bytes an unguarded shift would write): each
/// unguarded result changes the native warning output.
const WITHHELD: &[(&str, usize, &str)] = &[
    // The exact reproducer.
    (
        "if true\n  \tif true\n  \t  1\n  \tend\nend\n",
        1,
        "if true\n  if true\n \t  1\n \tend\nend\n",
    ),
    (
        "if true\n  \tif a\n  \t  1\n  \telse\n  \t  2\n  \tend\nend\n",
        1,
        "if true\n  if a\n \t  1\n \telse\n \t  2\n \tend\nend\n",
    ),
    (
        "if a\n  \t-> do\n  \t  1\n  \tend\nend\n",
        1,
        "if a\n  -> do\n \t  1\n \tend\nend\n",
    ),
    (
        "if a\n  \tcase b\n  \twhen c\n  \tend\nend\n",
        1,
        "if a\n  case b\n \twhen c\n \tend\nend\n",
    ),
    (
        "if a\n \t\tif b\n \t\tend\nend\n",
        1,
        "if a\n \tif b\n\t\tend\nend\n",
    ),
    (
        "def f\n  \t x\n \trescue\n  \t y\nend\n",
        1,
        "def f\n  x\nrescue\n\t y\nend\n",
    ),
    // Natively `end` pairs with the `else`, whose order alone would change.
    (
        "if true\n \t if x\n1\n\telse\n 2\n end\nend\n",
        3,
        "if true\n \tif x\n1\nelse\n2\nend\nend\n",
    ),
    // A space-only partial shift of a stabby lambda.
    (
        "if a\n      -> do\n  end\nend\n",
        1,
        "if a\n  -> do\n  end\nend\n",
    ),
];

/// (before, after) tab-containing shifts that keep the native warnings.
const CORRECTED: &[(&str, &str)] = &[
    (
        "if true\n  \t  if x\n  \t    1\n  \t  end\nend\n",
        "if true\n  if x\n    1\n  end\nend\n",
    ),
    (
        "def f\n   \tx\n  \trescue\n  \t  y\nend\n",
        "def f\n  x\n\trescue\n\t  y\nend\n",
    ),
    // A positive shift appends spaces after the whole prefix.
    (
        "if a\n\tif b\n\t  c\n\tend\nend\n",
        "if a\n\t if b\n\t   c\n\t end\nend\n",
    ),
];

const BINARY: &[u8] = b"# coding: BINARY\n# raw \xff\n";

/// The plain, CRLF, BOM and BINARY-with-raw-FF forms of `source`.
fn variants(source: &str) -> Vec<Vec<u8>> {
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend_from_slice(source.as_bytes());
    let mut binary = BINARY.to_vec();
    binary.extend_from_slice(source.as_bytes());
    vec![
        source.as_bytes().to_vec(),
        source.replace('\n', "\r\n").into_bytes(),
        bom,
        binary,
    ]
}

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
    let ruby = std::env::var("RUSTOCOP_RUBY34").ok()?;
    let out = Command::new(ruby)
        .current_dir(root)
        .args(["-w", "-c", "example.rb"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"Syntax OK\n");
    Some(String::from_utf8_lossy(&out.stderr).into_owned())
}

fn assert_untouched(root: &Path, source: &[u8], findings: usize, prefix: &[&str]) {
    let path = root.join("example.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    for fix in [false, true] {
        let mut args = prefix.to_vec();
        if fix {
            args.push("--fix");
        }
        args.push("example.rb");
        let out = run(root, &args);
        assert_eq!(out.status.code(), Some(1), "{source:?} {args:?} {out:?}");
        assert_eq!(count(&out), findings, "{source:?} {args:?} {out:?}");
        assert_eq!(fs::read(&path).unwrap(), source, "{source:?}");
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(old.ino(), new.ino());
        }
    }
}

#[test]
fn order_changing_tab_shifts_keep_findings_warnings_and_file_identity() {
    let dir = project();
    let path = dir.path().join("example.rb");
    for &(source, findings, unguarded) in WITHHELD {
        // The edit the guard withholds would change the native warnings.
        fs::write(&path, unguarded).unwrap();
        let changed = warnings(dir.path());
        for variant in variants(source) {
            fs::write(&path, &variant).unwrap();
            let native = warnings(dir.path());
            if variant.as_slice() == source.as_bytes() {
                assert_ne!(native, changed, "{source:?}");
            }
            assert_untouched(dir.path(), &variant, findings, &["--only", COP]);
            assert_eq!(warnings(dir.path()), native, "{variant:?}");
        }
    }
}

#[test]
fn order_keeping_tab_shifts_stay_corrected_with_identical_native_warnings() {
    let dir = project();
    let path = dir.path().join("example.rb");
    for &(before, after) in CORRECTED {
        for (before, after) in variants(before).into_iter().zip(variants(after)) {
            fs::write(&path, &before).unwrap();
            let native = warnings(dir.path());
            let out = run(dir.path(), &["--only", COP, "--fix", "example.rb"]);
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(" applied, "),
                "{before:?} {out:?}"
            );
            assert_eq!(fs::read(&path).unwrap(), after, "{before:?}");
            assert_eq!(warnings(dir.path()), native, "{before:?}");
            // The corrected file is stable: a second fix pass writes nothing.
            let old = fs::metadata(&path).unwrap();
            let out = run(dir.path(), &["--only", COP, "--fix", "example.rb"]);
            assert_eq!(out.status.code(), Some(0), "{after:?} {out:?}");
            assert_eq!(fs::read(&path).unwrap(), after);
            assert_eq!(
                old.modified().unwrap(),
                fs::metadata(&path).unwrap().modified().unwrap()
            );
        }
    }
}

#[test]
fn listed_source_observers_hold_tab_shift_corrections() {
    let dir = project();
    for observer in ["p __FILE__\n", "p $PROGRAM_NAME\n", "p caller\n"] {
        let source = format!("{observer}{}", CORRECTED[0].0);
        assert_untouched(dir.path(), source.as_bytes(), 1, &["--only", COP]);
    }
}

#[test]
fn whole_layout_composes_tab_shifts_to_a_stable_canonical_layout() {
    let dir = project();
    let path = dir.path().join("example.rb");
    for (source, canonical) in [
        (WITHHELD[0].0, "if true\n  if true\n    1\n  end\nend\n"),
        (
            WITHHELD[1].0,
            "if true\n  if a\n    1\n  else\n    2\n  end\nend\n",
        ),
        (WITHHELD[2].0, "if a\n  -> do\n    1\n  end\nend\n"),
        (CORRECTED[0].0, "if true\n  if x\n    1\n  end\nend\n"),
    ] {
        fs::write(&path, source).unwrap();
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
        assert_eq!(fs::read_to_string(&path).unwrap(), canonical, "{source:?}");
        if let Some(native) = warnings(dir.path()) {
            assert_eq!(native, "", "{source:?}");
        }
        // Stable zero: another pass reports nothing and writes nothing.
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", "Layout", "--fix", "example.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
    }
}
