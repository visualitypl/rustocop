//! source-only pinned examples, rebuilt CLI and no-write controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/DeprecatedOpenSSLConstant";
const SOURCE: &[u8] = b"OpenSSL::Digest::SHA256.digest('foo')\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
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
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("example.rb");
    fs::write(&path, bytes).unwrap();
    let before = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(out.status.code(), Some(status), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let after = fs::metadata(&path).unwrap();
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(before.permissions(), after.permissions());
    out
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/deprecated_open_ssl_constant.json")).unwrap()
}
#[test]
fn metadata_selectors_strict_toml_department_precedence_and_exclusions() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(
        cop.safe_fixes(),
        "bounded native-proven SHA256 literal digest capability"
    );
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
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
        args.push("example.rb");
        assert_eq!(count(&unchanged(dir.path(), SOURCE, &args, None)), expected);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 2.6",
        "NO_ARG_ALGORITHM = []",
        "SafeAutoCorrect = false",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SOURCE,
            &["--only", COP, "--fix", "example.rb"],
            Some(2),
        );
    }
    for config in [
        "exclude = ['example.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['example.rb']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        assert_eq!(
            count(&unchanged(
                dir.path(),
                SOURCE,
                &["--only", COP, "--fix", "example.rb"],
                Some(0)
            )),
            0
        );
    }
}
#[test]
fn supplied_file_stdin_provenance_and_builtin_exclusions() {
    let dir = project();
    for path in ["a.arb", "a.jb", "Rakefile", ".hidden.rb"] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        let out = run(dir.path(), &["--only", COP, path]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out), 1);
    }
    for directory in ["vendor", "node_modules"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        let path = dir.path().join(directory).join("a.rb");
        fs::write(&path, SOURCE).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", directory])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(fs::read(path).unwrap(), SOURCE);
    }
    for (config, expected) in [("", 1), ("exclude = ['virtual/**']\n", 0)] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "virtual/main.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"OpenSSL::Cipher::AES.new(__FILE__)\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(expected as i32), "{out:?}");
        assert_eq!(count(&out), expected);
        if expected == 1 {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains("virtual/main.rb:1:1"), "{out:?}");
            assert!(text.contains("aes-virtual/main.rb"), "{out:?}");
        }
    }
    unchanged(
        dir.path(),
        SOURCE,
        &["--only", COP, "--stdin", "example.rb", "--fix"],
        Some(2),
    );
}
#[test]
fn native_34_parses_reference_and_upstream_edits_but_never_executes_them() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        for key in ["source", "upstream_after"] {
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
                key == "source" || row["upstream_valid_syntax"].as_bool().unwrap(),
                "{row}\n{key}\n{out:?}"
            );
        }
    }
}
