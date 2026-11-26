//! Actual CLI lifecycle for Layout/DotPosition (fixed `leading`).
use crate::support::cli::{count, project, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Layout/DotPosition";
const BEFORE: &str = "a.\n  b\n";

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
    args.extend(["--fix", "--fix-passes", "1", "example.rb"]);
    let out = run(root, &args);
    (out, fs::read_to_string(&path).unwrap())
}

#[test]
fn metadata_defaults_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    let found = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(found.default_enabled() && found.safe_fixes());
    assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
    assert!(found.path_policy().is_none() && found.max_setting().is_none());
    assert!(found.autocorrect_incompatible_with().is_empty());
    let dir = project();
    for (config, selectors, n) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Layout"], 1),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--except", "Layout"], 0),
        (format!("[cops.\"{COP}\"]\nenabled = false\n"), vec![], 0),
        ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let out = unchanged(dir.path(), BEFORE.as_bytes(), &selectors, None);
        assert_eq!(count(&out, COP), n, "{selectors:?} {out:?}");
        if n == 1 {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains("example.rb:1:2: C: "), "{text}");
            assert!(
                text.contains("Place the . on the next line, together with the method name."),
                "{text}"
            );
        }
    }
    // No public style knob exists.
    for key in [
        "EnforcedStyle = \"trailing\"",
        "EnforcedStyle = \"leading\"",
        "max = 1",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            BEFORE.as_bytes(),
            &["--only", COP, "--fix"],
            Some(2),
        );
    }
}

#[test]
fn default_selection_converges_without_oscillation() {
    let dir = project();
    let (out, after) = fix(dir.path(), "foo.\n  bar.\n  baz\n", &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(after, "foo\n  .bar\n  .baz\n");
    unchanged(
        dir.path(),
        after.as_bytes(),
        &["--fix", "--fix-passes", "1"],
        Some(0),
    );
    // Layout/MultilineMethodCallIndentation realigns the moved dot next pass.
    let (out, after) = fix(dir.path(), "x = foo.\n      bar\nx\n", &[]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(after, "x = foo\n      .bar\nx\n");
    let (out, after) = fix(dir.path(), &after, &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(after, "x = foo\n    .bar\nx\n");
    unchanged(
        dir.path(),
        after.as_bytes(),
        &["--fix", "--fix-passes", "1"],
        Some(0),
    );
}

#[test]
fn selected_dot_position_holds_redundant_self_corrections() {
    // Style/RedundantSelf's accepted contract: it is
    // autocorrect-incompatible with this cop, and a selected DotPosition
    // withholds its edits without disabling either cop's detection.
    let dir = project();
    let (out, after) = fix(
        dir.path(),
        "self.object_id\na.\n  b\n",
        &["--only", "Style/RedundantSelf"],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(after, "object_id\na.\n  b\n");
    for selection in [
        "Style/RedundantSelf,Layout/DotPosition",
        "Layout/DotPosition,Style/RedundantSelf",
    ] {
        let (out, after) = fix(
            dir.path(),
            "self.object_id\na.\n  b\n",
            &["--only", selection],
        );
        assert_eq!(out.status.code(), Some(1), "{selection} {out:?}");
        assert_eq!(after, "self.object_id\na\n  .b\n", "{selection}");
        let out = unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", selection, "--fix"],
            Some(1),
        );
        assert_eq!(count(&out, "Style/RedundantSelf"), 1, "{out:?}");
        assert_eq!(count(&out, COP), 0, "{out:?}");
    }
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{BEFORE}"),
            format!("# {prefix}:disable Layout\n{BEFORE}"),
            format!("a. # {prefix}:disable {COP}\n  b\n"),
            format!("# {prefix}:push -{COP}\n{BEFORE}# {prefix}:pop\n"),
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out, COP), 0, "{source}");
        }
    }
}

#[test]
fn stdin_reports_positions_and_parse_errors_report_nothing() {
    let dir = project();
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
        .write_all(BEFORE.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("virtual.rb:1:2: C: "), "{text}");
    // An invalid file is an error, never a finding or a write.
    let invalid = format!("{BEFORE}def\n");
    let out = unchanged(
        dir.path(),
        invalid.as_bytes(),
        &["--only", COP, "--fix"],
        Some(2),
    );
    assert_eq!(count(&out, COP), 0, "{out:?}");
}
