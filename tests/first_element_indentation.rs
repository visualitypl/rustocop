//! Actual CLI lifecycle for the first-element-indentation
//! family (ClosingParenthesisIndentation, FirstArgumentIndentation,
//! FirstArrayElementIndentation, FirstHashElementIndentation).
use crate::support::cli::{count, project, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

/// (cop, misaligned source, canonical correction, first offense line:col)
const CASES: [(&str, &str, &str, &str); 4] = [
    (
        "Layout/ClosingParenthesisIndentation",
        "p(\n  1\n  )\n",
        "p(\n  1\n)\n",
        "3:3",
    ),
    (
        "Layout/FirstArgumentIndentation",
        // A symmetrical closing parenthesis keeps the result clean for the
        // default-on Layout/MultilineMethodCallBraceLayout.
        "p(\n      1\n)\n",
        "p(\n  1\n)\n",
        "2:7",
    ),
    (
        "Layout/FirstArrayElementIndentation",
        "x = [\n    1\n]\np x\n",
        "x = [\n  1\n]\np x\n",
        "2:5",
    ),
    (
        "Layout/FirstHashElementIndentation",
        "h = {\n    a: 1\n}\np h\n",
        "h = {\n  a: 1\n}\np h\n",
        "2:5",
    ),
];

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

#[test]
fn metadata_defaults_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    for (cop, before, _, position) in CASES {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(found.default_enabled() && found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert_eq!(found.ruby_versions(), Default::default());
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        assert!(found.autocorrect_incompatible_with().is_empty());
        let dir = project();
        for (config, selectors, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Layout"], 1),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (String::new(), vec!["--except", "Layout"], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            // Other default cops may also report, so only the count is exact.
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
        // No public style, width or threshold knobs exist for these cops.
        for key in [
            "EnforcedStyle = 'consistent'",
            "EnforcedStyle = 'special_inside_parentheses'",
            "IndentationWidth = 4",
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
fn lint_never_writes_and_fix_writes_the_canonical_edit_once() {
    let dir = project();
    let path = dir.path().join("example.rb");
    for (cop, before, after, _) in CASES {
        let out = unchanged(dir.path(), before.as_bytes(), &["--only", cop], Some(1));
        assert_eq!(count(&out, cop), 1, "{out:?}");
        let out = run(dir.path(), &["--only", cop, "--fix", "example.rb"]);
        assert_eq!(out.status.code(), Some(0), "{cop}: {out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), after, "{cop}");
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", "Layout", "--fix"],
            Some(0),
        );
        // As in RuboCop, a corrected file gets LF line endings; a BOM stays.
        let crlf = before.replace('\n', "\r\n");
        fs::write(&path, &crlf).unwrap();
        let out = run(dir.path(), &["--only", cop, "--fix", "example.rb"]);
        assert_eq!(out.status.code(), Some(0), "{cop}: {out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        fs::write(&path, format!("\u{feff}{before}")).unwrap();
        let out = run(dir.path(), &["--only", cop, "--fix", "example.rb"]);
        assert_eq!(out.status.code(), Some(0), "{cop}: {out:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("\u{feff}{after}")
        );
    }
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
        // An invalid file is an error, never an alignment finding or a write.
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
