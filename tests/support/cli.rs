//! The rustocop binary, run in a throwaway project.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

/// A temporary project root: a `.git` marker and an empty `rustocop.toml`.
pub fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

/// A [`project`] with an empty `lib` directory.
pub fn project_with_lib() -> tempfile::TempDir {
    let dir = project();
    fs::create_dir(dir.path().join("lib")).unwrap();
    dir
}

/// Run rustocop in `root` with `args`, stdin inherited.
pub fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

/// The number of text-format offenses `out` reports for `cop`.
pub fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({cop})"))
        .count()
}
