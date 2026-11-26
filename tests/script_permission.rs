//! ScriptPermission fixtures ported from RuboCop's lint/script_permission_spec.rb
//! (MIT, Copyright (c) 2012-26 Bozhidar Batsov). Never execute fixture source.
#![cfg(unix)]
use crate::support::cli::project;

use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Lint/ScriptPermission";
const SOURCE: &[u8] = b"#!/usr/bin/ruby\n";

fn fixture(root: &Path, name: &str, source: &[u8], mode: u32) -> fs::Metadata {
    let path = root.join(name);
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    fs::metadata(path).unwrap()
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

fn findings(out: &Output) -> Vec<String> {
    assert_ne!(out.status.code(), Some(2), "{out:?}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.ends_with(&format!("({COP})")))
        .map(str::to_owned)
        .collect()
}

fn unchanged(path: &Path, before: &fs::Metadata, source: &[u8]) {
    let after = fs::metadata(path).unwrap();
    assert_eq!(fs::read(path).unwrap(), source);
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(after.mode(), before.mode());
    assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
}

#[test]
fn defaults_selectors_and_strict_toml_keep_metadata_and_source_policy_separate() {
    let dir = project();
    let before = fixture(dir.path(), "script.rb", SOURCE, 0o644);
    for (config, args, expected) in [
        ("", vec!["script.rb"], 1),
        ("", vec!["--only", "Lint", "script.rb"], 1),
        ("", vec!["--except", COP, "script.rb"], 0),
        ("", vec!["--only", COP, "--except", "Lint", "script.rb"], 0),
        ("", vec!["--only", "Lint/Debugger", "script.rb"], 0),
        ("[cops.Lint]\nenabled = false\n", vec!["script.rb"], 0),
        (
            "[cops.Lint]\nenabled = false\n[cops.\"Lint/ScriptPermission\"]\nenabled = true\n",
            vec!["script.rb"],
            1,
        ),
        (
            "[cops.\"Lint/ScriptPermission\"]\nenabled = false\n",
            vec!["script.rb"],
            0,
        ),
        (
            "[cops.\"Lint/ScriptPermission\"]\nenabled = false\n",
            vec!["--only", COP, "script.rb"],
            1,
        ),
        (
            "[cops.\"Lint/ScriptPermission\"]\nexclude = [\"script.rb\"]\n",
            vec!["--only", COP, "script.rb"],
            0,
        ),
        (
            "exclude = [\"script.rb\"]\n",
            vec!["--only", COP, "script.rb"],
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for fix in [false, true] {
            let mut args = args.clone();
            if fix {
                args.push("--fix");
            }
            assert_eq!(
                findings(&cli(dir.path(), &args, None)).len(),
                expected,
                "{config} {args:?}"
            );
            unchanged(&dir.path().join("script.rb"), &before, SOURCE);
        }
    }
    for key in [
        "max = 1",
        "include = [\"*\"]",
        "permission = true",
        "autocorrect = false",
        "target_ruby = \"3.4\"",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        let out = cli(dir.path(), &["--fix", "script.rb"], None);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        unchanged(&dir.path().join("script.rb"), &before, SOURCE);
    }
}

#[test]
fn stdin_existing_missing_and_symlink_names_never_acquire_disk_metadata() {
    let dir = project();
    let before = fixture(dir.path(), "script.rb", SOURCE, 0o644);
    symlink("script.rb", dir.path().join("alias.rb")).unwrap();
    for name in ["script.rb", "alias.rb", "missing.rb", "virtual.jb"] {
        assert!(
            findings(&cli(
                dir.path(),
                &["--only", COP, "--stdin", name],
                Some(SOURCE)
            ))
            .is_empty()
        );
        assert_eq!(
            cli(dir.path(), &["--only", COP, "--stdin", name, "--fix"], None)
                .status
                .code(),
            Some(2)
        );
        unchanged(&dir.path().join("script.rb"), &before, SOURCE);
    }
    assert!(!dir.path().join("missing.rb").exists());
}

#[test]
fn executable_empty_noninitial_and_encoded_sources_obey_first_byte_gate() {
    let dir = project();
    for mode in [0o644, 0o755] {
        for (source, expected) in [
            (SOURCE, true),
            (b"#!", true),
            (b"#!/bin/sh\r\n", true),
            (
                b"#!/usr/bin/ruby\n# encoding: ISO-8859-1\n# caf\xe9\n",
                true,
            ),
            (
                b"#!/usr/bin/ruby\n# encoding: Shift_JIS\n# \x82\xa0\n",
                true,
            ),
            (
                b"#!/usr/bin/ruby\nx = <<~TEXT\nhello\nTEXT\n__LINE__\n__END__\nraw\n",
                true,
            ),
            (b"", false),
            (b"puts \"hello\"\n", false),
            (b"\n#!/usr/bin/ruby\n", false),
            (b" #!/usr/bin/ruby\n", false),
            (b"# ordinary\n#!/usr/bin/ruby\n", false),
            (b"\xef\xbb\xbf#!/usr/bin/ruby\n", false),
        ] {
            // Explicit names have no Include/Exclude or extension restriction.
            for name in ["script.rb", "script.jb", "bin-script"] {
                let before = fixture(dir.path(), name, source, mode);
                for fix in [false, true] {
                    let mut args = vec!["--only", COP, name];
                    if fix {
                        args.push("--fix");
                    }
                    assert_eq!(
                        findings(&cli(dir.path(), &args, None)).len(),
                        usize::from(expected && mode == 0o644),
                        "{name} {mode:o} {source:?}"
                    );
                    unchanged(&dir.path().join(name), &before, source);
                }
            }
        }
    }
}

#[test]
fn directive_scopes_do_not_retroactively_disable_the_header() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for (source, expected) in [
            ("#!/usr/bin/ruby # P:disable Lint/ScriptPermission\n", 0),
            ("#!/usr/bin/ruby # P:todo ScriptPermission\n", 0),
            ("#!/usr/bin/ruby # P:disable Lint\n", 0),
            ("#!/usr/bin/ruby # P:disable all\n", 0),
            (
                "#!/usr/bin/ruby # P:push -Lint/ScriptPermission\nvalue\n# P:pop\n",
                0,
            ),
            (
                "#!/usr/bin/ruby\n# P:disable Lint/ScriptPermission\nvalue\n# P:enable Lint/ScriptPermission\n",
                1,
            ),
            (
                "#!/usr/bin/ruby # P:disable-next Lint/ScriptPermission\nconsume(\n  value\n)\n",
                1,
            ),
            (
                "#!/usr/bin/ruby # P:next -Lint/ScriptPermission\nconsume(\n  value\n)\n",
                1,
            ),
            (
                "#!/usr/bin/ruby\n# P:push -Lint/ScriptPermission\nconsume(\n  value\n)\n# P:pop\n",
                1,
            ),
        ] {
            let source = source.replace("P:", &format!("{prefix}:"));
            let before = fixture(dir.path(), "script.rb", source.as_bytes(), 0o644);
            for fix in [false, true] {
                let mut args = vec!["--only", COP, "script.rb"];
                if fix {
                    args.push("--fix");
                }
                assert_eq!(
                    findings(&cli(dir.path(), &args, None)).len(),
                    expected,
                    "{source}"
                );
                unchanged(&dir.path().join("script.rb"), &before, source.as_bytes());
            }
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            "[cops.\"Lint/ScriptPermission\"]\nenabled = false\n",
        )
        .unwrap();
        let source = format!("#!/usr/bin/ruby # {prefix}:enable Lint/ScriptPermission\n");
        let before = fixture(dir.path(), "script.rb", source.as_bytes(), 0o644);
        // A comment-only enable starts AFTER its line. It cannot opt this
        // first-line-only finding in retroactively (unlike a disable).
        assert!(findings(&cli(dir.path(), &["script.rb"], None)).is_empty());
        assert!(findings(&cli(dir.path(), &["--except", COP, "script.rb"], None)).is_empty());
        unchanged(&dir.path().join("script.rb"), &before, source.as_bytes());
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn aliases_report_their_supplied_basename_but_never_change_identity_or_mode() {
    let dir = project();
    let before = fixture(dir.path(), "script.rb", SOURCE, 0o644);
    symlink("script.rb", dir.path().join("alias.rb")).unwrap();
    fs::hard_link(dir.path().join("script.rb"), dir.path().join("hard.rb")).unwrap();
    for fix in [false, true] {
        let mut args = vec!["--only", COP, "script.rb", "alias.rb", "hard.rb"];
        if fix {
            args.push("--fix");
        }
        let found = findings(&cli(dir.path(), &args, None));
        assert_eq!(found.len(), 3);
        for name in ["script.rb", "alias.rb", "hard.rb"] {
            assert!(found.iter().any(|s| s
                == &format!(
                    "{name}:1:1: W: Script file {name} doesn't have execute permission. ({COP})"
                )));
            unchanged(&dir.path().join(name), &before, SOURCE);
        }
        assert!(
            fs::symlink_metadata(dir.path().join("alias.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}

#[test]
fn fatal_inputs_and_read_errors_never_turn_into_permission_diagnostics() {
    let dir = project();
    let source = b"#!/usr/bin/ruby\nx=1\ndef (\n";
    let before = fixture(dir.path(), "script.rb", source, 0o644);
    symlink("missing.rb", dir.path().join("dangling.rb")).unwrap();
    for name in ["script.rb", "missing.rb", "dangling.rb"] {
        let out = cli(dir.path(), &["--only", COP, "--fix", name], None);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert!(!String::from_utf8_lossy(&out.stdout).contains("doesn't have execute permission"));
        unchanged(&dir.path().join("script.rb"), &before, source);
    }
}

#[test]
fn source_atomic_correction_composes_without_a_permission_correction() {
    let dir = project();
    let source = b"#!/usr/bin/ruby\nx=1\n";
    let before = fixture(dir.path(), "script.rb", source, 0o644);
    symlink("script.rb", dir.path().join("alias.rb")).unwrap();
    let out = cli(
        dir.path(),
        &[
            "--only",
            "Lint/ScriptPermission,Layout/SpaceAroundOperators",
            "--fix",
            "script.rb",
            "alias.rb",
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(findings(&out).len(), 2);
    let after = fs::metadata(dir.path().join("script.rb")).unwrap();
    assert_eq!(
        fs::read(dir.path().join("script.rb")).unwrap(),
        b"#!/usr/bin/ruby\nx = 1\n"
    );
    assert_eq!(after.mode(), before.mode());
    assert_eq!(after.ino(), before.ino());
    assert!(
        fs::symlink_metadata(dir.path().join("alias.rb"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn actual_nonexecutable_shebang_reports_exact_error_and_never_chmods() {
    let dir = project();
    let before = fixture(dir.path(), "script.rb", SOURCE, 0o644);
    for fix in [false, true] {
        let mut args = vec!["--only", COP, "script.rb"];
        if fix {
            args.push("--fix");
        }
        let out = cli(dir.path(), &args, None);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(
            findings(&out),
            vec![
                "script.rb:1:1: W: Script file script.rb doesn't have execute permission. (Lint/ScriptPermission)"
            ]
        );
        unchanged(&dir.path().join("script.rb"), &before, SOURCE);
    }
}
