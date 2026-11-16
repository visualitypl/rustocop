//! controlled Ruby semantics and real CLI correction boundaries.
use std::{fs, path::Path, process::Command};

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("file"), b"a\r\nb\0\xff\xc3\xa9").unwrap();
    dir
}
fn fix(dir: &Path, only: &str, status: i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(["--only", only, "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{out:?}");
}
#[test]
fn disabled_multiline_spans_never_apply_a_partial_read_rewrite() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("File.open('file') do |f|\n f.read # {prefix}:disable Style/FileRead\nend\n"),
            format!("File.open('file') do |f|\n # {prefix}:disable Style/FileRead\n f.read\nend\n"),
            format!("File.open(\n 'file' # {prefix}:disable Style/FileRead\n).read\n"),
        ] {
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            fix(dir.path(), "Style/FileRead", 0);
            assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        }
    }
}

#[test]
fn mixed_read_string_parentheses_return_semicolon_and_layout_edits_converge() {
    let dir = project();
    fs::write(
        dir.path().join("a.rb"),
        "def f\n  return (File.open('file', 'rb', &:read));  \nend\n",
    )
    .unwrap();
    let only = "Style/FileRead,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout";
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        "def f\n  File.binread(\"file\")\nend\n"
    );
    fix(dir.path(), only, 0);
}
