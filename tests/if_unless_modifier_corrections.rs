//! real single-pass correction pipeline/CLI and controlled Ruby witnesses.
//! Pinned examples in the companion JSON are ported from RuboCop (MIT License,
//! Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const COP: &str = "Style/IfUnlessModifier";
const BEFORE: &str = "if ready\n  work\nend\n";
const AFTER: &str = "work if ready\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], status: i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
}
fn correct(root: &Path, before: &[u8], after: &[u8], selectors: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(["--only", selectors, "--fix", "--fix-passes", "1", "a.rb"])
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(fs::read(root.join("a.rb")).unwrap(), after, "{output:?}");
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) {
    fs::write(root.join("a.rb"), source).unwrap();
    let time = fs::metadata(root.join("a.rb")).unwrap().modified().unwrap();
    cli(root, args, status);
    assert_eq!(fs::read(root.join("a.rb")).unwrap(), source);
    assert_eq!(
        fs::metadata(root.join("a.rb")).unwrap().modified().unwrap(),
        time
    );
}
#[test]
fn defaults_opt_in_config_selectors_and_incompatibilities() {
    let d = project();
    unchanged(d.path(), BEFORE.as_bytes(), &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        BEFORE.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    // Default selections include both declared incompatible peers; no findings
    // are hidden, and no implicit multipass or unsafe correction is attempted.
    unchanged(d.path(), BEFORE.as_bytes(), &["--fix", "a.rb"], 1);
    for peer in ["Style/Next", "Style/SoleNestedConditional"] {
        unchanged(
            d.path(),
            BEFORE.as_bytes(),
            &["--only", &format!("{COP},{peer}"), "--fix", "a.rb"],
            1,
        );
    }
    fs::write(d.path().join("a.rb"), BEFORE).unwrap();
    cli(
        d.path(),
        &[
            "--except",
            "Style/Next,Style/SoleNestedConditional",
            "--fix",
            "a.rb",
        ],
        0,
    );
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), AFTER);
    for config in [
        format!(
            "[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n[cops.\"{COP}\"]\nenabled=false\n"
        ),
        "[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n".into(),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), BEFORE.as_bytes(), &["--fix", "a.rb"], 0);
        correct(d.path(), BEFORE.as_bytes(), AFTER.as_bytes(), COP);
    }
    fs::write(d.path().join("rustocop.toml"), format!("[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n")).unwrap();
    fs::write(d.path().join("a.rb"), BEFORE).unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), AFTER);
}
#[test]
fn mixed_style_layout_edits_are_atomic_deterministic_single_passes() {
    let d = project();
    let selected = "Style/IfUnlessModifier,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/IndentationWidth,Layout/SpaceAroundOperators";
    let before = "if ready\n  call({ :key => 'value' })\nend\nx=1\n";
    let first = "call({ :key => 'value' }) if ready\nx = 1\n";
    correct(d.path(), before.as_bytes(), first.as_bytes(), selected);
    let reversed = selected.split(',').rev().collect::<Vec<_>>().join(",");
    correct(d.path(), before.as_bytes(), first.as_bytes(), &reversed);
    // The whole conditional owns the overlapping literal/hash edits, so they
    // wait for a later explicitly requested pass rather than a hidden reparse.
    let mut previous = fs::read(d.path().join("a.rb")).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", selected, "--fix", "--fix-passes", "1", "a.rb"])
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        let next = fs::read(d.path().join("a.rb")).unwrap();
        if next == previous {
            stable = true;
            break;
        }
        previous = next;
    }
    assert!(stable);
    assert_eq!(
        String::from_utf8(previous).unwrap(),
        "call({ key: \"value\" }) if ready\nx = 1\n"
    );
    for (source, first) in [
        (
            "def f(c)\n  if c\n    return ('value')\n  end\nend\n",
            "def f(c)\n  return ('value') if c\nend\n",
        ),
        (
            "def f(c)\n  if c\n    fail ('value');\n  end\nend\n",
            "def f(c)\n  fail ('value') if c\nend\n",
        ),
    ] {
        correct(d.path(), source.as_bytes(), first.as_bytes(), selected);
        correct(d.path(), source.as_bytes(), first.as_bytes(), &reversed);
        let mut previous = first.as_bytes().to_vec();
        let mut stable = false;
        for _ in 0..6 {
            let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(["--only", selected, "--fix", "--fix-passes", "1", "a.rb"])
                .output()
                .unwrap();
            assert_ne!(output.status.code(), Some(2), "{output:?}");
            let next = fs::read(d.path().join("a.rb")).unwrap();
            if next == previous {
                stable = true;
                break;
            }
            previous = next;
        }
        assert!(stable, "{source}");
    }
}
fn ruby(source: &[u8], program: &std::ffi::OsStr) -> std::process::Output {
    let mut child = Command::new(program)
        .arg("--disable-gems")
        .arg("-W0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn native_34_implicit_it_and_numbered_parameters_keep_their_binding() {
    let Some(program) = crate::support::runtime::ruby_os() else {
        return;
    };
    let d = project();
    for parameter in ["it", "_1"] {
        let source = format!("p [false,true].map {{\n if {parameter}\n  42\n end\n}}\n");
        let after = format!("p [false,true].map {{\n 42 if {parameter}\n}}\n");
        correct(d.path(), source.as_bytes(), after.as_bytes(), COP);
        let old = ruby(source.as_bytes(), &program);
        let new = ruby(after.as_bytes(), &program);
        assert!(
            old.status.success() && new.status.success(),
            "{old:?} {new:?}"
        );
        assert_eq!(old.stdout, new.stdout);
        assert_eq!(old.stdout, b"[nil, 42]\n");
    }
}
#[test]
fn long_modifier_expansion_preserves_observations_and_is_stable() {
    let d = project();
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Layout/LineLength\"]\nmax=30\n",
    )
    .unwrap();
    let source = "def f(condition,log)\n log << :body if (log << :condition; condition)\nend\nl=[];p [f(false,l),f(true,l),l]\n";
    let after = "def f(condition,log)\n if (log << :condition; condition)\n   log << :body\n end\nend\nl=[];p [f(false,l),f(true,l),l]\n";
    correct(d.path(), source.as_bytes(), after.as_bytes(), COP);
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    if Command::new("ruby").arg("--version").output().is_ok() {
        let old = ruby(source.as_bytes(), std::ffi::OsStr::new("ruby"));
        let new = ruby(after.as_bytes(), std::ffi::OsStr::new("ruby"));
        assert!(
            old.status.success() && new.status.success(),
            "{old:?} {new:?}"
        );
        assert_eq!(old.stdout, new.stdout);
    }
}
