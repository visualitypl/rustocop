//! Actual supplied-input CLI/native/atomic/no-write controls.
//! No new cops are registered by this prerequisite.
use crate::support::deprecated_constructor_native as native;
use crate::support::stdin as stdin_support;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], source: Option<&[u8]>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(if source.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(bytes) = source {
        stdin_support::write_stdin(child.stdin.take().unwrap(), bytes, args.contains(&"--fix"))
            .unwrap();
    }
    child.wait_with_output().unwrap()
}
fn status(out: &Output, code: i32) {
    assert_eq!(out.status.code(), Some(code), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
}
fn unchanged(path: &Path, bytes: &[u8], old: &fs::Metadata) {
    assert_eq!(fs::read(path).unwrap(), bytes);
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
}

#[test]
fn directory_discovery_keeps_ignore_hidden_config_root_and_explicit_file_policies() {
    let d = project();
    for name in [
        "Gemfile",
        "ci.gemfile",
        "gems.rb",
        "pkg.gemspec",
        "nested/Gemfile",
        ".hidden/Gemfile",
        "vendor/Gemfile",
        "ignored/Gemfile",
        "excluded/Gemfile",
        "lookalike/Gemfile.backup",
        "other.lock",
    ] {
        let path = d.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "binding.pry\n").unwrap();
    }
    fs::write(d.path().join(".gitignore"), "ignored/\n").unwrap();
    fs::write(
        d.path().join("rustocop.toml"),
        "exclude = [\"excluded/**\"]\n",
    )
    .unwrap();
    for name in ["Gemfile.lock", "gems.locked"] {
        fs::write(d.path().join(name), b"\xff\0def (((").unwrap();
    }
    let out = cli(d.path(), &["--only", "Lint/Debugger", "."], None);
    status(&out, 1);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.ends_with("7 files inspected, 5 offenses detected\n"),
        "{text}"
    );
    for name in [
        "Gemfile",
        "ci.gemfile",
        "gems.rb",
        "pkg.gemspec",
        "nested/Gemfile",
    ] {
        assert!(
            text.contains(&format!(
                "{name}:1:1: E: Remove debugger entry point `binding.pry`. (Lint/Debugger)"
            )),
            "{text}"
        );
    }
    for name in [
        ".hidden/Gemfile",
        "ignored/Gemfile",
        "other.lock",
        "lookalike/Gemfile.backup",
    ] {
        // Explicit files retain the existing extension/hidden/gitignore bypass.
        status(&cli(d.path(), &["--only", "Lint/Debugger", name], None), 1);
    }
    for name in ["vendor/Gemfile", "excluded/Gemfile"] {
        let out = cli(d.path(), &["--only", "Lint/Debugger", name], None);
        status(&out, 0);
        assert_eq!(out.stdout, b"\n0 files inspected, no offenses detected\n");
    }
    // Eligibility is relative to the discovered config, not the nested cwd.
    let out = cli(
        &d.path().join("nested"),
        &["--only", "Lint/Debugger", "../excluded/Gemfile"],
        None,
    );
    status(&out, 0);
    assert_eq!(out.stdout, b"\n0 files inspected, no offenses detected\n");
}

#[test]
fn opaque_locks_are_clean_except_filename_cop_with_exact_output_and_never_written() {
    let d = project();
    let hostile = [
        vec![b'('; 100_000],
        b"\xff\0# encoding: unknown\n# rubocop:enable all\nbinding.pry\n".to_vec(),
    ];
    for name in ["Gemfile.lock", "gems.locked"] {
        for bytes in &hostile {
            let path = d.path().join(name);
            fs::write(&path, bytes).unwrap();
            let old = fs::metadata(&path).unwrap();
            for format in ["text", "github"] {
                for fix in [false, true] {
                    let mut args = vec!["--except", "Bundler", "--format", format, name];
                    if fix {
                        args.push("--fix");
                    }
                    let out = cli(d.path(), &args, None);
                    status(&out, 0);
                    let expected = match (format, fix) {
                        ("text", false) => "\n1 file inspected, no offenses detected\n",
                        ("text", true) => {
                            "\n1 file inspected, 0 corrections applied, 0 offenses remaining\n"
                        }
                        (_, false) => "1 file inspected, no offenses detected\n",
                        (_, true) => {
                            "1 file inspected, 0 corrections applied, 0 offenses remaining\n"
                        }
                    };
                    assert_eq!(out.stdout, expected.as_bytes());
                    unchanged(&path, bytes, &old);
                }
            }
            let out = cli(
                d.path(),
                &["--except", "Bundler", "--stdin", name],
                Some(bytes),
            );
            status(&out, 0);
            assert_eq!(out.stdout, b"\n1 file inspected, no offenses detected\n");
            unchanged(&path, bytes, &old);
        }
    }
    assert_eq!(
        fs::read_dir(d.path()).unwrap().count(),
        4,
        "no temp/companion files"
    );
}

#[test]
fn ruby_gem_inputs_keep_fatal_syntax_encoding_resource_and_status_contracts() {
    let d = project();
    let deep = format!("{}1{}", "[".repeat(100), "]".repeat(100));
    for name in [
        "Gemfile",
        "ci.gemfile",
        "gems.rb",
        "pkg.gemspec",
        "Gemfile.lock.rb",
        "other.lock",
    ] {
        for bytes in [
            &b"def ("[..],
            b"\xff",
            b"# encoding: unknown\n",
            deep.as_bytes(),
        ] {
            let path = d.path().join(name);
            fs::write(&path, bytes).unwrap();
            let old = fs::metadata(&path).unwrap();
            for fix in [false, true] {
                let mut args = vec!["--only", "Lint/Debugger", name];
                if fix {
                    args.push("--fix");
                }
                let out = cli(d.path(), &args, None);
                status(&out, 2);
                let text = String::from_utf8(out.stdout).unwrap();
                assert!(text.contains(&format!("{name}: error:")), "{text}");
                assert!(!text.contains("(Lint/Debugger)"));
                unchanged(&path, bytes, &old);
            }
            status(
                &cli(
                    d.path(),
                    &["--only", "Lint/Debugger", "--stdin", name],
                    Some(bytes),
                ),
                2,
            );
        }
    }
    fs::write(d.path().join("gems.rb"), b"binding.pry\n").unwrap();
    let out = cli(
        d.path(),
        &["--only", "Lint/Debugger", "gems.rb", "pkg.gemspec"],
        None,
    );
    status(&out, 2);
    assert!(String::from_utf8_lossy(&out.stdout).contains("gems.rb:1:1: E:"));
}

#[test]
fn ruby_encoding_bom_crlf_data_directives_and_stdin_keep_existing_positions() {
    let d = project();
    for name in ["Gemfile", "ci.gemfile", "gems.rb", "pkg.gemspec"] {
        for (bytes, line, expected) in [
            (&b"binding.pry\n"[..], 1, 1),
            (&b"\xef\xbb\xbfbinding.pry\r\n"[..], 1, 1),
            (
                &b"# encoding: Windows-1252\r\n# \xe9\r\nbinding.pry\r\n"[..],
                3,
                1,
            ),
            (&b"__END__\r\nbinding.pry\r\n\xff"[..], 0, 0),
        ] {
            fs::write(d.path().join(name), bytes).unwrap();
            for stdin in [false, true] {
                let mut args = vec!["--only", "Lint/Debugger"];
                if stdin {
                    args.push("--stdin");
                }
                args.push(name);
                let out = cli(d.path(), &args, stdin.then_some(bytes));
                status(&out, expected);
                if expected == 1 {
                    assert!(
                        String::from_utf8_lossy(&out.stdout)
                            .contains(&format!("{name}:{line}:1: E:")),
                        "{out:?}"
                    );
                }
            }
        }
        for prefix in ["rubocop", "rustocop"] {
            let bytes = format!("# {prefix}:disable Lint/Debugger\r\nbinding.pry\r\n");
            status(
                &cli(
                    d.path(),
                    &["--only", "Lint/Debugger", "--stdin", name],
                    Some(bytes.as_bytes()),
                ),
                0,
            );
        }
    }
}

#[test]
fn supplied_files_do_not_search_siblings_resolve_dependencies_or_execute_inputs() {
    let d = project();
    fs::write(d.path().join("Gemfile.lock"), b"\xffnot Ruby").unwrap();
    fs::write(d.path().join("bad.gemspec"), b"def (").unwrap();
    for name in ["Gemfile", "ci.gemfile", "gems.rb", "pkg.gemspec"] {
        let source = b"File.write(\"EXECUTED\", \"never\")\nraise \"must not run\"\n";
        fs::write(d.path().join(name), source).unwrap();
        status(&cli(d.path(), &["--only", "Lint/Debugger", name], None), 0);
        assert!(!d.path().join("EXECUTED").exists());
        assert!(!d.path().join("gems.locked").exists());
    }
    let out = cli(d.path(), &["--only", "Lint/Debugger", "Gemfile"], None);
    assert_eq!(out.stdout, b"\n1 file inspected, no offenses detected\n");
}

#[cfg(unix)]
#[test]
fn known_opaque_symlink_alias_vetoes_all_source_writes_but_keeps_ruby_findings() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("gems.rb");
    let bytes = b"x=1\n";
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    symlink("gems.rb", d.path().join("gems.locked")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for order in [["gems.rb", "gems.locked"], ["gems.locked", "gems.rb"]] {
        let out = cli(
            d.path(),
            &[
                "--fix",
                "--only",
                "Layout/SpaceAroundOperators",
                order[0],
                order[1],
            ],
            None,
        );
        status(&out, 1);
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .contains("2 files inspected, 0 corrections applied, 1 offense remaining")
        );
        unchanged(&path, bytes, &old);
        assert!(
            fs::symlink_metadata(d.path().join("gems.locked"))
                .unwrap()
                .is_symlink()
        );
    }
    // Supplied names, not canonical targets, govern Ruby/opaque classification.
    let out = cli(
        d.path(),
        &["--only", "Layout/SpaceAroundOperators", "gems.locked"],
        None,
    );
    status(&out, 0);
    assert_eq!(out.stdout, b"\n1 file inspected, no offenses detected\n");
    unchanged(&path, bytes, &old);
}

#[cfg(unix)]
#[test]
fn ruby_alias_cannot_source_correct_a_known_opaque_disk_destination() {
    use std::os::unix::fs::symlink;
    let d = project();
    let path = d.path().join("Gemfile.lock");
    let before = b"x=1\n";
    fs::write(&path, before).unwrap();
    symlink("Gemfile.lock", d.path().join("alias.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = cli(
        d.path(),
        &["--fix", "--only", "Layout/SpaceAroundOperators", "alias.rb"],
        None,
    );
    status(&out, 1);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("0 corrections applied, 1 offense remaining")
    );
    unchanged(&path, before, &old);
    assert!(
        fs::symlink_metadata(d.path().join("alias.rb"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn ruby_gem_sources_retain_atomic_canonical_corrections_and_idempotence() {
    let d = project();
    for name in ["Gemfile", "ci.gemfile", "gems.rb", "pkg.gemspec"] {
        let path = d.path().join(name);
        let before = "value=7\nputs value\n";
        fs::write(&path, before).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = cli(
            d.path(),
            &["--fix", "--only", "Layout/SpaceAroundOperators", name],
            None,
        );
        status(&out, 0);
        assert_eq!(
            out.stdout,
            b"\n1 file inspected, 1 correction applied, 0 offenses remaining\n"
        );
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(after, "value = 7\nputs value\n");
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_ne!(old.ino(), fs::metadata(&path).unwrap().ino());
        }
        let old = fs::metadata(&path).unwrap();
        status(
            &cli(
                d.path(),
                &["--fix", "--only", "Layout/SpaceAroundOperators", name],
                None,
            ),
            0,
        );
        unchanged(&path, after.as_bytes(), &old);
        if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
            // Only this finite, authored witness executes, externally bounded.
            let a = native::run(&ruby, before);
            let b = native::run(&ruby, &after);
            assert_eq!(
                (a.status.code(), a.stdout, a.stderr),
                (b.status.code(), b.stdout, b.stderr)
            );
        }
    }
}

#[test]
fn future_selectors_config_and_stdin_fix_are_rejected_without_writes() {
    let d = project();
    let path = d.path().join("gems.locked");
    let bytes = b"\xff\0opaque";
    fs::write(&path, bytes).unwrap();
    let old = fs::metadata(&path).unwrap();
    for name in ["Bundler/Future", "Gemspec/Future"] {
        for flag in ["--only", "--except"] {
            let out = cli(d.path(), &[flag, name, "gems.locked"], None);
            assert_eq!(out.status.code(), Some(2));
            assert!(out.stdout.is_empty());
            assert!(String::from_utf8_lossy(&out.stderr).contains("unknown cop or department"));
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{name}\"]\nenabled = true\n"),
        )
        .unwrap();
        assert_eq!(cli(d.path(), &["gems.locked"], None).status.code(), Some(2));
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
    let out = cli(
        d.path(),
        &["--stdin", "gems.locked", "--fix"],
        Some(&vec![b'('; 100_000]),
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the argument '--stdin <PATH>' cannot be used with '--fix'")
    );
    unchanged(&path, bytes, &old);
}

#[test]
fn native_ruby34_checks_supplied_ruby_bytes_as_syntax_only() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for bytes in [
        &b"File.write(\"NEVER_EXECUTE_REFERENCE\", \"no\")\n"[..],
        b"# metadata only\n",
        b"",
        b"\xef\xbb\xbfputs 1\r\n",
        b"# encoding: Windows-1252\r\n# \xe9\r\nputs 1\r\n",
        b"__END__\n\xff",
    ] {
        let mut child = Command::new("timeout")
            .args([
                "--kill-after=1s",
                "10",
                &ruby,
                "--parser=prism",
                "-W0",
                "-c",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        stdin_support::write_stdin(child.stdin.take().unwrap(), bytes, false).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{bytes:?}: {out:?}");
    }
}
