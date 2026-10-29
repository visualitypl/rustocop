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
fn all_pinned_sources_have_exact_cli_diagnostics_and_withheld_fixes_never_write() {
    let dir = project();
    for row in rows() {
        let source = row["source"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        for fix in [false, true] {
            let args = if fix {
                vec!["--only", COP, "--fix", "example.rb"]
            } else {
                vec!["--only", COP, "example.rb"]
            };
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &args,
                Some(i32::from(!findings.is_empty())),
            );
            assert_eq!(count(&out), findings.len(), "{row}\n{out:?}");
            let text = String::from_utf8_lossy(&out.stdout);
            for f in findings {
                assert!(
                    text.contains(&format!(
                        "example.rb:{}:{}: E: {} ({COP})",
                        f[0],
                        f[1],
                        f[3].as_str().unwrap()
                    )),
                    "{row}\n{out:?}"
                );
            }
        }
    }
}
#[test]
fn legacy_argument_diagnostics_preserve_bytes_without_lossy_replacement_characters() {
    let dir = project();
    for (encoding, argument, displayed) in [
        ("ISO-8859-1", b"caf\xe9".as_slice(), r"caf\xe9"),
        ("Windows-31J", b"\x83\x60".as_slice(), r"\x83\x60"),
        // Valid UTF-8 bytes are still two separate characters in Latin-1.
        ("ISO-8859-1", b"\xc3\xa9".as_slice(), r"\xc3\xa9"),
        ("UTF-8", "café".as_bytes(), "café"),
    ] {
        for (before, after) in [
            (
                "OpenSSL::Digest::SHA256.digest",
                "OpenSSL::Digest.digest('SHA256', ",
            ),
            ("OpenSSL::Cipher::Cipher.new", "OpenSSL::Cipher.new("),
        ] {
            let mut source = format!("# encoding: {encoding}\n{before}(\"").into_bytes();
            source.extend_from_slice(argument);
            source.extend_from_slice(b"\")\n");
            let message = format!(
                "example.rb:2:1: E: Use `{after}\"{displayed}\")` instead of `{before}(\"{displayed}\")`. ({COP})"
            );
            for fix in [false, true] {
                let mut args = vec!["--only", COP];
                if fix {
                    args.push("--fix");
                }
                args.push("example.rb");
                let out = unchanged(dir.path(), &source, &args, Some(1));
                assert!(out.stderr.is_empty(), "{out:?}");
                assert_eq!(count(&out), 1);
                let text = String::from_utf8(out.stdout).unwrap();
                assert!(text.contains(&message), "{encoding}: {text}");
                assert!(!text.contains('\u{fffd}'));
            }
        }
    }
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
fn both_prefixes_multiline_next_push_pop_and_source_enable() {
    let dir = project();
    let call = String::from_utf8_lossy(SOURCE);
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{call}"),
            format!("# {prefix}:push -{COP}\n{call}# {prefix}:pop\n"),
            format!("# {prefix}:disable-next {COP}\n{call}"),
            format!("# {prefix}:next -{COP}\n{call}"),
            format!("OpenSSL::Cipher::AES.new(\n128,\n:GCM # {prefix}:disable {COP}\n)\n"),
        ] {
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "example.rb"],
                    Some(0)
                )),
                0
            );
        }
        for (open, close) in [
            (
                format!("# {prefix}:push +{COP}\n"),
                format!("# {prefix}:pop\n"),
            ),
            (format!("# {prefix}:enable-next {COP}\n"), String::new()),
        ] {
            let source = format!("# {prefix}:disable {COP}\n{call}{open}{call}{close}{call}");
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "example.rb"],
                    Some(1)
                )),
                1
            );
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\n{call}");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["example.rb"],
                None
            )),
            1
        );
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
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
fn byte_envelopes_and_observers_preserve_every_byte() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let mut bytes = prefix.to_vec();
        bytes.extend_from_slice(b"# preserve\r\nOpenSSL::Cipher::AES.new(128, :GCM) # call\r\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                &bytes,
                &["--only", COP, "--fix", "example.rb"],
                Some(1)
            )),
            1
        );
    }
    for suffix in [
        "; p __LINE__",
        "; File.read(__FILE__)",
        "; DATA.read\n__END__\ndata",
        "; ObjectSpace.each_object {}",
        "; RubyVM::InstructionSequence.compile_file(__FILE__)",
    ] {
        let source = format!("OpenSSL::Digest::SHA256.digest('foo'){suffix}\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "example.rb"],
                Some(1)
            )),
            1
        );
    }
}
#[cfg(unix)]
#[test]
fn readonly_symlink_parser_failure_and_existing_atomic_peer_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("example.rb");
    fs::write(&path, SOURCE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("example.rb", dir.path().join("link.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for name in ["example.rb", "link.rb"] {
        let out = run(dir.path(), &["--only", COP, "--fix", name]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.ino(), new.ino());
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert_eq!(fs::read(&path).unwrap(), SOURCE);
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    unchanged(
        dir.path(),
        b"OpenSSL::Digest::SHA256.new\n(\n",
        &["--only", COP, "--fix", "example.rb"],
        Some(2),
    );
    let before = b"OpenSSL::Digest::SHA256.digest('foo')\np [1,2]\n";
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let args = [
        "--only",
        "Lint/DeprecatedOpenSSLConstant,Layout/SpaceAfterComma",
        "--fix",
        "link.rb",
    ];
    assert_eq!(run(dir.path(), &args).status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        old.modified().unwrap()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(run(dir.path(), &args).status.code(), Some(1));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"OpenSSL::Digest::SHA256.digest('foo')\np [1, 2]\n"
    );
    assert_ne!(fs::metadata(&path).unwrap().ino(), old.ino());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(dir.path().join("link.rb"))
            .unwrap()
            .is_symlink()
    );
    symlink("missing.rb", dir.path().join("dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "dangling.rb"])
            .status
            .code(),
        Some(2)
    );
    assert!(!dir.path().join("missing.rb").exists());
}
#[test]
fn native_34_parses_reference_and_upstream_edits_but_never_executes_them() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
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
