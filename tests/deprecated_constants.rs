//! pinned source-only fixtures, actual CLI and authored native witnesses.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/DeprecatedConstants";
const SAFE: &[u8] = b"NIL = nil\n[::NIL]\n";
const UNCERTAIN: &[u8] = b"NIL\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/deprecated_constants.json")).unwrap()
}
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
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
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(out.status.code(), Some(status), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}
#[test]
fn metadata_selectors_toml_department_precedence_and_exclusions() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(registry.has_cop("Lint/UnusedPrivateMethod"));
    let dir = project();
    for (config, selectors, expected) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint"], 1),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            1,
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(dir.path(), SAFE, &args, None)), expected);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 2.6",
        "DeprecatedConstants = {}",
        "SafeAutoCorrect = false",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SAFE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    for config in [
        "exclude = ['lib/**']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        assert_eq!(
            count(&unchanged(
                dir.path(),
                SAFE,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0)
            )),
            0
        );
    }
}
#[test]
fn supplied_paths_stdin_and_builtin_exclusions() {
    let dir = project();
    for path in ["lib/a.arb", "lib/a.jb", "Rakefile", "lib/.hidden.rb"] {
        fs::write(dir.path().join(path), UNCERTAIN).unwrap();
        let out = run(dir.path(), &["--only", COP, path]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out), 1);
    }
    for directory in ["vendor", "node_modules"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        fs::write(dir.path().join(directory).join("a.rb"), SAFE).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", directory])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(
            fs::read(dir.path().join(directory).join("a.rb")).unwrap(),
            SAFE
        );
    }
    for (config, expected) in [(String::new(), 1), ("exclude = ['virtual/**']\n".into(), 0)] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "virtual/main.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SAFE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(expected as i32), "{out:?}");
        assert_eq!(count(&out), expected);
        if expected == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/main.rb:2:2"));
        }
    }
    unchanged(
        dir.path(),
        SAFE,
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
}
#[test]
fn canonical_byte_envelopes_preserve_comments_encoding_bom_and_crlf() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(b"# alias\r\nNIL = nil # binding\r\n[::NIL] # read\r\n");
        let mut after = prefix.to_vec();
        after.extend_from_slice(b"# alias\r\nNIL = nil # binding\r\n[nil] # read\r\n");
        unchanged(dir.path(), &source, &["--only", COP, "lib/a.rb"], Some(1));
        let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), after);
    }
}
#[test]
fn all_reference_and_corrected_sources_are_native_syntax_only() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        for key in ["source", "upstream_after", "project_after"] {
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-c", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(row[key].as_str().unwrap().as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                key != "upstream_after" || row["upstream_valid_syntax"].as_bool().unwrap(),
                "{row}\n{key}\n{out:?}"
            );
        }
    }
}
