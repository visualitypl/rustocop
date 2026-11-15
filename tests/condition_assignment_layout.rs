//! Actual CLI lifecycle for default-on Layout/ConditionPosition and
//! opt-in Layout/MultilineAssignmentLayout (fixed `new_line`).
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const CONDITION: &str = "Layout/ConditionPosition";
const ASSIGNMENT: &str = "Layout/MultilineAssignmentLayout";

/// (cop, offending source, canonical correction, offense line:col)
const CASES: [(&str, &str, &str, &str); 2] = [
    (CONDITION, "if\n  a\n  b\nend\n", "if a\n  b\nend\n", "2:3"),
    (
        ASSIGNMENT,
        "x = if a\n  b\nend\n",
        "x =\n if a\n  b\nend\n",
        "1:1",
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

fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({cop})"))
        .count()
}

/// Run `args` on `example.rb` holding `source`; the file must keep its bytes,
/// mtime, permissions and inode.
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: Option<i32>) -> Output {
    let path = root.join("example.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let mut args = args.to_vec();
    args.push("example.rb");
    let out = run(root, &args);
    if code.is_some() {
        assert_eq!(out.status.code(), code, "{args:?} {out:?}");
    }
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

fn fix(root: &Path, source: &str, args: &[&str]) -> (Output, String) {
    let path = root.join("example.rb");
    fs::write(&path, source).unwrap();
    let mut args = args.to_vec();
    args.extend(["--fix", "example.rb"]);
    let out = run(root, &args);
    (out, fs::read_to_string(&path).unwrap())
}

#[test]
fn metadata_defaults_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    for (cop, before, _, position) in CASES {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        let default_on = cop == CONDITION;
        assert_eq!(found.default_enabled(), default_on, "{cop}");
        assert!(found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        assert!(found.autocorrect_incompatible_with().is_empty());
        let dir = project();
        let on = usize::from(default_on);
        for (config, selectors, n) in [
            (String::new(), vec![], on),
            (String::new(), vec!["--only", cop], 1),
            // A department selection runs an opt-in cop only when the
            // configuration enables it, as upstream.
            (String::new(), vec!["--only", "Layout"], on),
            (
                format!("[cops.\"{cop}\"]\nenabled = true\n"),
                vec!["--only", "Layout"],
                1,
            ),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (String::new(), vec!["--except", "Layout"], 0),
            (format!("[cops.\"{cop}\"]\nenabled = true\n"), vec![], 1),
            ("[cops.Layout]\nenabled = true\n".into(), vec![], on),
            (
                format!("[cops.Layout]\nenabled = true\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (
                format!("[cops.Layout]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(dir.path(), before.as_bytes(), &selectors, None);
            assert_eq!(count(&out, cop), n, "{cop} {selectors:?} {out:?}");
            if n == 1 {
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(
                    text.contains(&format!("example.rb:{position}: C: ")),
                    "{text}"
                );
            }
        }
        // No public style, type or threshold knobs exist.
        for key in [
            "EnforcedStyle = \"same_line\"",
            "SupportedTypes = [\"array\"]",
            "max = 1",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", cop, "--fix"],
                Some(2),
            );
        }
    }
}

#[test]
fn default_selection_converges_without_oscillation() {
    let dir = project();
    let (out, fixed) = fix(dir.path(), "while\n  a\n  b\n  c\nend\n", &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fixed, "while a\n  b\n  c\nend\n");
    unchanged(dir.path(), fixed.as_bytes(), &["--fix"], Some(0));
    // Department selections do not run the opt-in assignment cop.
    unchanged(
        dir.path(),
        b"x = if a\n      b\n    end\nx\n",
        &["--only", "Layout", "--fix"],
        Some(0),
    );
    // With it and every Layout cop selected, the assignment break is
    // followed by the realignment of its value.
    let selection = format!("{ASSIGNMENT},Layout");
    let mut source = "x = if a\n      b\n    end\nx\n".to_owned();
    for _ in 0..3 {
        let (_, fixed) = fix(dir.path(), &source, &["--only", &selection]);
        source = fixed;
    }
    assert_eq!(source, "x =\n  if a\n    b\n  end\nx\n");
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", &selection, "--fix"],
        Some(0),
    );
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for (cop, before, _, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for source in [
                format!("# {prefix}:disable {cop}\n{before}"),
                format!("# {prefix}:disable Layout\n{before}"),
                format!("# {prefix}:push -{cop}\n{before}# {prefix}:pop\n"),
            ] {
                let out = unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix"],
                    Some(0),
                );
                assert_eq!(count(&out, cop), 0, "{source}");
            }
        }
    }
}

#[test]
fn stdin_reports_positions_and_parse_errors_report_nothing() {
    let dir = project();
    for (cop, before, _, position) in CASES {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "virtual.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains(&format!("virtual.rb:{position}: C: ")),
            "{text}"
        );
        // An invalid file is an error, never a finding or a write.
        let invalid = format!("{before}def\n");
        let out = unchanged(
            dir.path(),
            invalid.as_bytes(),
            &["--only", cop, "--fix"],
            Some(2),
        );
        assert_eq!(count(&out, cop), 0, "{out:?}");
    }
}
