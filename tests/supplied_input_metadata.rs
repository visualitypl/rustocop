//! Rebuilt CLI regressions: existing detectors/write contract stay unchanged.
//! Reference shebang/directive forms: RuboCop MIT (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(bytes) = stdin {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    child.wait_with_output().unwrap()
}
fn count(out: &Output, cop: &str) -> usize {
    assert_ne!(out.status.code(), Some(2), "{out:?}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.ends_with(&format!("({cop})")))
        .count()
}
fn unchanged(path: &Path, before: &fs::Metadata, bytes: &[u8]) {
    let after = fs::metadata(path).unwrap();
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(before.permissions(), after.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(before.ino(), after.ino());
    }
}

#[test]
fn supplied_file_stdin_config_and_selection_remain_existing_policy_only() {
    let d = project();
    let path = d.path().join("script.rb");
    let source = b"#!/usr/bin/ruby\nbinding.pry\n";
    fs::write(&path, source).unwrap();
    let before = fs::metadata(&path).unwrap();
    for (args, expected) in [
        (vec!["script.rb"], 1),
        (vec!["--only", "Lint/Debugger", "script.rb"], 1),
        (
            vec!["--only", "Lint", "--except", "Lint/Debugger", "script.rb"],
            0,
        ),
        (vec!["--only", "Lint/Debugger", "--fix", "script.rb"], 1),
    ] {
        assert_eq!(
            count(&cli(d.path(), &args, None), "Lint/Debugger"),
            expected
        );
        unchanged(&path, &before, source);
    }
    for (config, expected) in [
        ("[cops.Lint]\nenabled = false\n", 0),
        (
            "[cops.Lint]\nenabled = false\n[cops.\"Lint/Debugger\"]\nenabled = true\n",
            1,
        ),
        ("[cops.\"Lint/Debugger\"]\nenabled = false\n", 0),
        ("[cops.\"Lint/Debugger\"]\nexclude = [\"script.rb\"]\n", 0),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        assert_eq!(
            count(&cli(d.path(), &["script.rb"], None), "Lint/Debugger"),
            expected
        );
        assert_eq!(
            count(
                &cli(d.path(), &["--stdin", "script.rb"], Some(source)),
                "Lint/Debugger"
            ),
            expected
        );
        unchanged(&path, &before, source);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    assert_eq!(
        cli(
            d.path(),
            &["--only", "Lint/NotAnImplementedCop", "script.rb"],
            None
        )
        .status
        .code(),
        Some(2)
    );
    assert_eq!(
        cli(d.path(), &["--stdin", "script.rb", "--fix"], None)
            .status
            .code(),
        Some(2)
    );
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Lint/Debugger\"]\npermission = true\n",
    )
    .unwrap();
    assert_eq!(cli(d.path(), &["script.rb"], None).status.code(), Some(2));
    unchanged(&path, &before, source);
}

#[test]
fn shebang_directive_scopes_and_fixed_path_gates_preserve_bytes() {
    let d = project();
    let path = d.path().join("script.rb");
    for prefix in ["rubocop", "rustocop"] {
        for body in [
            "# P:disable Lint/Debugger\nbinding.pry\n# P:enable Lint/Debugger\n",
            "# P:disable-next Lint/Debugger\nbinding.pry(\n value\n)\n",
            "# P:next -Lint/Debugger\nbinding.pry(value)\n",
            "# P:push -Lint/Debugger\nbinding.pry\n# P:pop\n",
        ] {
            let source = format!(
                "#!/usr/bin/ruby\n{}",
                body.replace("P:", &format!("{prefix}:"))
            );
            fs::write(&path, &source).unwrap();
            let before = fs::metadata(&path).unwrap();
            for fix in [false, true] {
                let mut args = vec!["--only", "Lint/Debugger", "script.rb"];
                if fix {
                    args.push("--fix");
                }
                assert_eq!(count(&cli(d.path(), &args, None), "Lint/Debugger"), 0);
                unchanged(&path, &before, source.as_bytes());
            }
            assert_eq!(
                count(
                    &cli(
                        d.path(),
                        &["--only", "Lint/Debugger", "--stdin", "script.rb"],
                        Some(source.as_bytes())
                    ),
                    "Lint/Debugger"
                ),
                0
            );
        }
    }
    let source = b"#!/usr/bin/ruby\nreturn 1\n";
    for (path, expected) in [("script.rb", 1), ("script.jb", 0)] {
        fs::write(d.path().join(path), source).unwrap();
        for from_stdin in [false, true] {
            let mut args = vec!["--only", "Lint/TopLevelReturnWithArgument"];
            if from_stdin {
                args.push("--stdin");
            }
            args.push(path);
            assert_eq!(
                count(
                    &cli(d.path(), &args, from_stdin.then_some(source)),
                    "Lint/TopLevelReturnWithArgument"
                ),
                expected
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn metadata_capture_does_not_change_atomic_alias_write_or_mode_preservation() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("script.rb");
    let alias = d.path().join("alias.rb");
    let source = b"#!/usr/bin/ruby\nx=1\n";
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    symlink("script.rb", &alias).unwrap();
    let before = fs::metadata(&path).unwrap();
    let out = cli(
        d.path(),
        &[
            "--only",
            "Layout/SpaceAroundOperators",
            "script.rb",
            "alias.rb",
        ],
        None,
    );
    assert_eq!(count(&out, "Layout/SpaceAroundOperators"), 2);
    unchanged(&path, &before, source);
    let out = cli(
        d.path(),
        &[
            "--only",
            "Layout/SpaceAroundOperators",
            "--fix",
            "script.rb",
            "alias.rb",
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), b"#!/usr/bin/ruby\nx = 1\n");
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o751);
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
    assert_ne!(fs::metadata(&path).unwrap().ino(), before.ino());
}
