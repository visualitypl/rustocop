//! Real CLI controls and separately authored native safety witnesses.
//! RuboCop spec fixtures: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/RedundantSplatExpansion";
const SAFE: &[u8] = b"begin\n nil\nrescue *[First, Second]\n handler\nend\n";
const AFTER: &[u8] = b"begin\n nil\nrescue First, Second\n handler\nend\n";
const LIVE: &[u8] = b"begin\n work\nrescue *[First, Second]\n handler\nend\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
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
fn count(o: &Output) -> usize {
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|s| s.ends_with(&format!("({COP})")))
        .count()
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, bytes).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(out.status.code(), Some(status), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
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
fn rows() -> Vec<serde_json::Value> {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/redundant_splat_expansion.json")).unwrap();
    data["rows"].as_array().unwrap().clone()
}
#[test]
fn fixed_metadata_and_actual_current_admission() {
    let registry = rustocop::cops::Registry::new();
    let c = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.safe_fixes() && c.default_enabled());
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(c.path_policy().is_none() && c.max_setting().is_none());
    assert!(c.autocorrect_incompatible_with().is_empty());
    assert!(registry.has_cop("Lint/UnusedPrivateMethod"));
}
#[test]
fn selectors_strict_toml_enablement_department_and_excludes() {
    let d = project();
    for (cfg, selectors, expected) in [
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
        fs::write(d.path().join("rustocop.toml"), cfg).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(d.path(), LIVE, &args, None)), expected);
    }
    for key in [
        "AllowPercentLiteralArrayArgument = false",
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "target_ruby = 2.7",
        "SafeAutoCorrect = true",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            SAFE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    for cfg in [
        "exclude = ['lib/**']\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), cfg).unwrap();
        unchanged(
            d.path(),
            SAFE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn supplied_paths_stdin_provenance_excludes_and_fix_rejection() {
    let d = project();
    for name in [
        "lib/source.arb",
        "lib/source.jb",
        "Rakefile",
        "lib/.hidden.rb",
    ] {
        fs::write(d.path().join(name), LIVE).unwrap();
        let out = run(d.path(), &["--only", COP, name]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(count(&out), 1);
    }
    for name in ["vendor", "node_modules"] {
        fs::create_dir(d.path().join(name)).unwrap();
        fs::write(d.path().join(name).join("a.rb"), SAFE).unwrap();
        assert_eq!(
            run(d.path(), &["--only", COP, "--fix", name]).status.code(),
            Some(0)
        );
        assert_eq!(fs::read(d.path().join(name).join("a.rb")).unwrap(), SAFE);
    }
    for excluded in [false, true] {
        fs::write(
            d.path().join("rustocop.toml"),
            if excluded {
                format!("[cops.\"{COP}\"]\nexclude = ['virtual/**']\n")
            } else {
                String::new()
            },
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", COP, "--stdin", "virtual/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(LIVE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(i32::from(!excluded)));
        if !excluded {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/a.rb:3:8"));
        }
    }
    unchanged(
        d.path(),
        SAFE,
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
}
#[cfg(unix)]
#[test]
fn atomic_validation_readonly_symlink_and_bounded_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink("a.rb", d.path().join("lib/link.rb")).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    assert_eq!(
        run(d.path(), &["--only", COP, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(fs::read(&path).unwrap(), AFTER);
    assert_ne!(inode, fs::metadata(&path).unwrap().ino());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(d.path().join("lib/link.rb"))
            .unwrap()
            .is_symlink()
    );
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    assert_eq!(
        run(d.path(), &["--only", COP, "--fix", "lib/a.rb"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let before = format!("{}p [1,2]\nx = 'a'\n", String::from_utf8_lossy(SAFE));
    fs::write(&path, before).unwrap();
    let out = run(
        d.path(),
        &[
            "--only",
            "Lint/RedundantSplatExpansion,Layout/SpaceAfterComma,Style/StringLiterals",
            "--fix",
            "lib/a.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("{}p [1, 2]\nx = \"a\"\n", String::from_utf8_lossy(AFTER))
    );
    assert_eq!(fs::read_dir(d.path().join("lib")).unwrap().count(), 2);
}
fn native(source: &[u8], syntax: bool) -> Option<Output> {
    let ruby = crate::support::runtime::ruby().ok()?;
    let mut cmd = Command::new(ruby);
    cmd.args(["--parser=prism", "-w"]);
    if syntax {
        cmd.arg("-c");
    }
    cmd.arg("-");
    let mut c = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(source).unwrap();
    Some(c.wait_with_output().unwrap())
}
#[test]
fn reference_inputs_are_only_syntax_checked_never_executed() {
    for row in rows() {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        if let Some(out) = native(&bytes, true) {
            let invalid = row.get("parser_rejection").is_some()
                || row.get("native_scope_rejection").is_some();
            assert_eq!(out.status.success(), !invalid, "{row}\n{out:?}");
        }
        if let Some(after) = row.get("project_after") {
            let bytes: Vec<u8> = crate::support::fixture::value(after);
            if let Some(out) = native(&bytes, true) {
                assert!(out.status.success(), "{row}\n{out:?}");
            }
        }
    }
}
