//! real CLI, source-only pinned fixtures and authored native witnesses.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/ConstantOverwrittenInRescue";
const SAFE: &[u8] = b"begin\n  nil\nrescue => Error\nend\n";
const AFTER: &[u8] = b"begin\n  nil\nrescue Error\nend\n";
const REACHABLE: &[u8] = b"begin\n  something\nrescue => Error\nend\n";
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
fn metadata_and_current_only_admission_preserve_fixed_policy() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert_eq!(
        cop.autocorrect_incompatible_with(),
        [
            "Naming/RescuedExceptionsVariableName",
            "Style/RescueStandardError"
        ]
    );
    assert!(registry.has_cop("Lint/UnusedPrivateMethod"));
}
#[test]
fn selectors_strict_toml_department_precedence_and_exclusions() {
    let dir = project();
    for (config, select, expected) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint"], 1),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        (String::new(), vec!["--only", "Lint/Debugger"], 0),
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
        let mut args = select;
        args.push("lib/a.rb");
        let out = unchanged(dir.path(), SAFE, &args, None);
        assert_eq!(count(&out), expected, "{out:?}");
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 2.6",
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
        let out = unchanged(
            dir.path(),
            SAFE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
        assert_eq!(count(&out), 0);
    }
}
#[test]
fn supplied_paths_have_no_cop_gate_and_builtin_directory_exclusions_remain() {
    let dir = project();
    for path in [
        "lib/source.arb",
        "lib/source.jb",
        "Rakefile",
        "lib/.hidden.rb",
    ] {
        fs::write(dir.path().join(path), REACHABLE).unwrap();
        let out = run(dir.path(), &["--only", COP, path]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out), 1);
    }
    for directory in ["vendor", "node_modules"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        fs::write(dir.path().join(directory).join("a.rb"), SAFE).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", directory]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(
            fs::read(dir.path().join(directory).join("a.rb")).unwrap(),
            SAFE
        );
    }
}
#[test]
fn stdin_path_provenance_exclusions_and_fix_rejection() {
    let dir = project();
    fs::write(dir.path().join("lib/a.rb"), b"untouched\n").unwrap();
    let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
    for (config, expected) in [
        (String::new(), 1),
        (format!("[cops.\"{COP}\"]\nexclude = ['virtual/**']\n"), 0),
    ] {
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
        assert_eq!(out.status.code(), Some(i32::from(expected > 0)), "{out:?}");
        assert_eq!(count(&out), expected);
        if expected > 0 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/main.rb:3:8"));
        }
    }
    let out = run(dir.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        fs::read(dir.path().join("lib/a.rb")).unwrap(),
        b"untouched\n"
    );
    assert_eq!(
        fs::metadata(dir.path().join("lib/a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        old.modified().unwrap()
    );
}
#[cfg(unix)]
#[test]
fn atomic_symlink_readonly_validation_and_composition_share_existing_controls() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    let out = run(dir.path(), &["--only", COP, "--fix", "lib/link.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), AFTER);
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(dir.path().join("lib/link.rb"))
            .unwrap()
            .is_symlink()
    );
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        old.modified().unwrap()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    unchanged(
        dir.path(),
        b"begin; nil; rescue => Error; end\n(\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    fs::write(&path, b"begin; nil; rescue => Error; end\np [1,2]\n").unwrap();
    let out = run(
        dir.path(),
        &[
            "--only",
            "Lint/ConstantOverwrittenInRescue,Layout/SpaceAfterComma",
            "--fix",
            "lib/a.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read(&path).unwrap(),
        b"begin; nil; rescue Error; end\np [1, 2]\n"
    );
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
}

#[test]
fn native_reference_fixture_validation_is_syntax_only() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/constant_overwritten_in_rescue.json")).unwrap();
    for row in rows {
        for field in ["source", "project_after", "upstream_after"] {
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
                .write_all(row[field].as_str().unwrap().as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{row}\n{out:?}");
        }
    }
}
