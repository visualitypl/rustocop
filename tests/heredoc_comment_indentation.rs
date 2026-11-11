//! Actual CLI lifecycle for the heredoc-comment-indentation
//! family.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

/// (cop, misindented source, canonical correction, first offense line:col)
const CASES: [(&str, &str, &str, &str); 4] = [
    (
        "Layout/HeredocIndentation",
        "x = <<~A\nfoo\nA\n",
        "x = <<~A\n  foo\nA\n",
        "2:1",
    ),
    (
        "Layout/ClosingHeredocIndentation",
        "def f\n  x = <<~A\n    y\nA\nend\n",
        "def f\n  x = <<~A\n    y\n  A\nend\n",
        "4:1",
    ),
    (
        "Layout/CommentIndentation",
        "def f\n# a\n  x\nend\n",
        "def f\n  # a\n  x\nend\n",
        "2:1",
    ),
    (
        "Layout/InitialIndentation",
        "  # a\n  x = 1\n",
        "  # a\nx = 1\n",
        "2:3",
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

#[test]
fn metadata_defaults_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    for (cop, before, _, position) in CASES {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(found.default_enabled() && found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
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
        // No public width, squish, line-length or alignment knobs exist.
        for key in [
            "IndentationWidth = 4",
            "ActiveSupportExtensionsEnabled = true",
            "AllowHeredoc = false",
            "AllowForAlignment = true",
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
            &["--only", cop, "--fix"],
            Some(0),
        );
        // CRLF line endings and a UTF-8 BOM are preserved around the edit.
        let crlf = before.replace('\n', "\r\n");
        fs::write(&path, &crlf).unwrap();
        let out = run(dir.path(), &["--only", cop, "--fix", "example.rb"]);
        assert_eq!(out.status.code(), Some(0), "{cop}: {out:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            after.replace('\n', "\r\n")
        );
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
fn heredoc_body_and_closing_edits_compose_across_fix_runs() {
    let dir = project();
    let path = dir.path().join("example.rb");
    let only = "Layout/HeredocIndentation,Layout/ClosingHeredocIndentation";
    for (before, after, runs) in [
        // Both cops rewrite the closing line: one edit wins per pass.
        (
            "def f\n  <<~A\nfoo\nA\nend\n",
            "def f\n  <<~A\n    foo\n  A\nend\n",
            2,
        ),
        // `<<-` becomes `<<~` and the closing line moves in the same pass;
        // the body is indented in the next.
        ("x = <<-A\nfoo\n  A\n", "x = <<~A\n  foo\nA\n", 2),
    ] {
        let out = unchanged(dir.path(), before.as_bytes(), &["--only", only], Some(1));
        assert_eq!(count(&out, "Layout/HeredocIndentation"), 1, "{out:?}");
        assert_eq!(
            count(&out, "Layout/ClosingHeredocIndentation"),
            1,
            "{out:?}"
        );
        fs::write(&path, before).unwrap();
        for _ in 0..runs {
            let out = run(dir.path(), &["--only", only, "--fix", "example.rb"]);
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", "Layout", "--fix"],
            Some(0),
        );
    }
    // A held, value-changing edit keeps the finding and the file's bytes.
    let tabs = "x = <<~A\n\tfoo\n        bar\nA\n";
    let out = unchanged(
        dir.path(),
        tabs.as_bytes(),
        &["--only", "Layout/HeredocIndentation", "--fix"],
        Some(1),
    );
    assert_eq!(count(&out, "Layout/HeredocIndentation"), 1, "{out:?}");
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

#[test]
fn heredoc_argument_closing_parenthesis_is_opt_in_and_yields_to_its_incompatible_peer() {
    const COP: &str = "Layout/HeredocArgumentClosingParenthesis";
    let before = "foo(<<~A, 1,\n  x\nA\n)\n";
    let after = "foo(<<~A, 1)\n  x\nA\n";
    let registry = rustocop::cops::Registry::new();
    let found = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(!found.default_enabled() && found.safe_fixes());
    assert_eq!(
        found.autocorrect_incompatible_with(),
        &["Style/TrailingCommaInArguments"]
    );
    let dir = project();
    let path = dir.path().join("example.rb");
    // Off by default; `--only` and `enabled = true` select it.
    let out = unchanged(dir.path(), before.as_bytes(), &[], None);
    assert_eq!(count(&out, COP), 0, "{out:?}");
    let out = unchanged(dir.path(), before.as_bytes(), &["--only", COP], Some(1));
    assert_eq!(count(&out, COP), 1, "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("example.rb:4:1: C: "));
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    let out = unchanged(
        dir.path(),
        before.as_bytes(),
        &["--except", "Style"],
        Some(1),
    );
    assert_eq!(count(&out, COP), 1, "{out:?}");
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    // The canonical relocation, once.
    let out = run(dir.path(), &["--only", COP, "--fix", "example.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    unchanged(
        dir.path(),
        after.as_bytes(),
        &["--only", COP, "--fix"],
        Some(0),
    );
    let crlf = before.replace('\n', "\r\n");
    fs::write(&path, &crlf).unwrap();
    let out = run(dir.path(), &["--only", COP, "--fix", "example.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        after.replace('\n', "\r\n")
    );
    // With the incompatible peer selected, only the peer's edit is applied
    // and this finding stays.
    let peer = format!("{COP},Style/TrailingCommaInArguments");
    fs::write(&path, before).unwrap();
    let out = run(dir.path(), &["--only", &peer, "--fix", "example.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out, COP), 1, "{out:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "foo(<<~A, 1\n  x\nA\n)\n"
    );
    // Directives, stdin and invalid input.
    for prefix in ["rubocop", "rustocop"] {
        let source = format!("# {prefix}:disable {COP}\n{before}");
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix"],
            Some(0),
        );
        assert_eq!(count(&out, COP), 0, "{out:?}");
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--stdin", "virtual.rb"])
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
    assert!(String::from_utf8_lossy(&out.stdout).contains("virtual.rb:4:1: C: "));
    let invalid = format!("{before}def\n");
    let out = unchanged(
        dir.path(),
        invalid.as_bytes(),
        &["--only", COP, "--fix"],
        Some(2),
    );
    assert_eq!(count(&out, COP), 0, "{out:?}");
}
