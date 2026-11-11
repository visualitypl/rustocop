//! Source-only pinned AST and CLI checks. Never execute reference inputs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
#[macro_use]
#[path = "support/parallel_rows.rs"]
mod parallel_rows;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 1] = ["Lint/SendWithMixinArgument"];
const SOURCES: [&[u8]; 1] = [b"Foo.send(:include, Bar)\n"];
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/send_with_mixin_argument.json")).unwrap();
    rows
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
fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({cop})"))
        .count()
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    }
    let mut held = fs::File::open(&path).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(
            out.status.code(),
            Some(status),
            "{}: {out:?}",
            String::from_utf8_lossy(bytes)
        );
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let mut held_bytes = Vec::new();
    held.read_to_end(&mut held_bytes).unwrap();
    assert_eq!(held_bytes, bytes);
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
#[test]
fn defaults_selectors_toml_department_and_path_precedence() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let detector = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(detector.default_enabled());
        assert!(detector.safe_fixes());
        assert_eq!(
            detector.ruby_versions(),
            rustocop::cops::policy::RubyVersions {
                minimum: None,
                maximum: None,
            }
        );
        assert!(detector.path_policy().is_none() && detector.max_setting().is_none());
        assert!(detector.autocorrect_incompatible_with().is_empty());
        assert_eq!(
            rustocop::offense::department_of(cop),
            rustocop::offense::Department::Lint
        );
        for (config, mut args, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint", "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", "Lint"], 0),
            ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
            (
                format!("[cops.Lint]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (
                format!("[cops.Lint]\nenabled = true\n[cops.\"{cop}\"]\nenabled = false\n"),
                vec![],
                0,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
                vec!["--only", cop],
                0,
            ),
            ("exclude = ['lib/a.rb']\n".into(), vec!["--only", cop], 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            args.push("lib/a.rb");
            assert_eq!(count(&unchanged(dir.path(), source, &args, None), cop), n);
        }
        for key in [
            "max = 1",
            "include = ['**/*']",
            "Enabled = true",
            "enabled = 'true'",
            "TargetRubyVersion = 3.3",
            "SafeAutoCorrect = true",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                source,
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(2),
            );
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            "[cops.Lint]\nexclude = ['lib/a.rb']\n",
        )
        .unwrap();
        unchanged(
            dir.path(),
            source,
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
}
#[test]
fn supplied_file_stdin_path_provenance_and_no_writes() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for config in [
            String::new(),
            "exclude = ['lib/a.rb']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
        ] {
            let n = usize::from(config.is_empty());
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            assert_eq!(
                count(
                    &unchanged(
                        dir.path(),
                        source,
                        &["--only", cop, "--fix", "lib/a.rb"],
                        Some(n as i32)
                    ),
                    cop
                ),
                n
            );
            let out = run(&dir.path().join("lib"), &["--only", cop, "a.rb"]);
            assert_eq!(count(&out, cop), n);
            if n != 0 {
                assert!(String::from_utf8_lossy(&out.stdout).starts_with("lib/a.rb:"));
            }
            fs::write(dir.path().join("lib/a.rb"), b"sentinel\n").unwrap();
            let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", cop, "--stdin", "lib/a.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(count(&out, cop), n);
            if n != 0 {
                assert!(String::from_utf8_lossy(&out.stdout).starts_with("lib/a.rb:"));
            }
            assert_eq!(
                fs::read(dir.path().join("lib/a.rb")).unwrap(),
                b"sentinel\n"
            );
            assert_eq!(
                old.modified().unwrap(),
                fs::metadata(dir.path().join("lib/a.rb"))
                    .unwrap()
                    .modified()
                    .unwrap()
            );
        }
        assert_eq!(
            run(dir.path(), &["--only", cop, "--stdin", "lib/a.rb", "--fix"])
                .status
                .code(),
            Some(2)
        );
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for (path, expected) in [
            ("Gemfile", 1),
            ("lib/a.jb", 1),
            ("lib/.hidden.rb", 1),
            ("lib/a_spec.rb", 1),
            ("vendor/a.rb", 0),
            ("db/schema.rb", 0),
        ] {
            let file = dir.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, source).unwrap();
            let old = fs::metadata(&file).unwrap();
            assert_eq!(
                count(&run(dir.path(), &["--only", cop, "--fix", path]), cop),
                expected
            );
            assert_eq!(fs::read(&file).unwrap(), source);
            let new = fs::metadata(&file).unwrap();
            assert_eq!(old.modified().unwrap(), new.modified().unwrap());
            assert_eq!(old.permissions(), new.permissions());
        }
    }
}
#[test]
fn both_directive_prefixes_multiline_next_push_and_range_end() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let source = std::str::from_utf8(source).unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for text in [
                format!("# {prefix}:disable Lint\n{source}"),
                format!("# {prefix}:disable-next {cop}\n{source}"),
                format!("# {prefix}:next -{cop}\n{source}"),
                format!("{} # {prefix}:disable {cop}\n", source.trim_end()),
                format!("# {prefix}:push -{cop}\n{source}# {prefix}:pop\n"),
            ] {
                unchanged(
                    dir.path(),
                    text.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(0),
                );
            }
            for (open, close) in [
                (
                    format!("# {prefix}:push +{cop}\n"),
                    format!("# {prefix}:pop\n"),
                ),
                (format!("# {prefix}:enable-next {cop}\n"), String::new()),
            ] {
                let text =
                    format!("# {prefix}:disable {cop}\n{source}{open}{source}{close}{source}");
                assert_eq!(
                    count(
                        &unchanged(
                            dir.path(),
                            text.as_bytes(),
                            &["--only", cop, "--fix", "lib/a.rb"],
                            Some(1)
                        ),
                        cop
                    ),
                    1
                );
            }
        }
    }
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        )
        .unwrap();
        for prefix in ["rubocop", "rustocop"] {
            let text = format!(
                "# {prefix}:enable {cop}\n{}",
                std::str::from_utf8(source).unwrap()
            );
            let out = unchanged(dir.path(), text.as_bytes(), &["lib/a.rb"], None);
            assert_eq!(count(&out, cop), 1);
            let out = unchanged(
                dir.path(),
                text.as_bytes(),
                &["--except", cop, "lib/a.rb"],
                None,
            );
            assert_eq!(count(&out, cop), 0);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    for cop in COPS {
        for prefix in ["rubocop", "rustocop"] {
            // A comment on a receiver line precedes the selector's range.
            let text = format!("Foo. # {prefix}:disable {cop}\n send(:include, Bar)\n");
            assert_eq!(
                count(
                    &unchanged(
                        dir.path(),
                        text.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        Some(1)
                    ),
                    cop
                ),
                1
            );
            for text in [
                format!("Foo.send(:include, # {prefix}:disable {cop}\n Bar\n)\n"),
                format!("# {prefix}:disable-next {cop}\nFoo.send(:include,\n Bar\n)\n"),
            ] {
                unchanged(
                    dir.path(),
                    text.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(0),
                );
            }
        }
    }
}
#[test]
fn observer_bytes_parser_gate_peer_composition_and_symlink_atomic_failure() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for suffix in [
            "p __LINE__; File.binread(__FILE__); DATA.pos\n__END__\nraw\0\n",
            "RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
            "p __FILE__, __ENCODING__\n",
        ] {
            let text = format!(
                "\u{feff}# coding: utf-8\r\n{}{}",
                std::str::from_utf8(source).unwrap().replace('\n', "\r\n"),
                suffix.replace('\n', "\r\n")
            );
            assert_eq!(
                count(
                    &unchanged(
                        dir.path(),
                        text.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        Some(1)
                    ),
                    cop
                ),
                1
            );
        }
        let mut invalid = source.to_vec();
        invalid.extend_from_slice(b"def broken(\n");
        unchanged(
            dir.path(),
            &invalid,
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    let cops = COPS.join(",");
    let peers = format!("{cops},Style/StringLiterals,Layout/SpaceAfterComma");
    let mut before = SOURCES.concat();
    before.extend_from_slice(b"a = ['a',2]\n");
    let mut after = SOURCES.concat();
    after.extend_from_slice(b"a = [\"a\", 2]\n");
    unchanged(
        dir.path(),
        &before,
        &["--only", &peers, "lib/a.rb"],
        Some(1),
    );
    let out = run(dir.path(), &["--only", &peers, "--fix", "lib/a.rb"]);
    for cop in COPS {
        assert_eq!(count(&out, cop), 1);
    }
    let path = dir.path().join("lib/a.rb");
    assert_eq!(fs::read(&path).unwrap(), after);
    unchanged(
        dir.path(),
        &after,
        &["--only", &peers, "--fix", "lib/a.rb"],
        Some(1),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        symlink(&path, dir.path().join("link.rb")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", &cops, "--fix", "link.rb"]);
        for cop in COPS {
            assert_eq!(count(&out, cop), 1);
        }
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&path, &before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", &peers, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        for cop in COPS {
            assert_eq!(count(&out, cop), 1);
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        assert!(
            fs::symlink_metadata(dir.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}

#[test]
fn all_original_byte_rows_rebuilt_cli_lint_fix_and_idempotence() {
    parallel_rows!(for row in rows() => {
        let dir = project();
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let relative = row["path"].as_str().unwrap();
        let path = dir.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for fix in [false, true] {
            fs::write(&path, &bytes).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
            }
            let old = fs::metadata(&path).unwrap();
            let mut held = fs::File::open(&path).unwrap();
            let mut args = vec!["--only", COPS[0]];
            if fix {
                args.push("--fix");
            }
            args.push(relative);
            let out = run(dir.path(), &args);
            let invalid = row.get("parser_rejection").is_some();
            let expected: Vec<(u32, u32, u32, String)> = if invalid {
                Vec::new()
            } else {
                serde_json::from_value(row["findings"].clone()).unwrap()
            };
            let after: Option<Vec<u8>> = if fix {
                row.get("project_after")
                    .map(|v| serde_json::from_value(v.clone()).unwrap())
            } else {
                None
            };
            let status = if invalid {
                2
            } else {
                i32::from(after.is_none() && !expected.is_empty())
            };
            assert_eq!(out.status.code(), Some(status), "{row}: {out:?}");
            assert_eq!(
                fs::read(&path).unwrap(),
                after.as_ref().unwrap_or(&bytes).as_slice(),
                "{row}: {out:?}"
            );
            let new = fs::metadata(&path).unwrap();
            assert_eq!(old.permissions(), new.permissions(), "{row}");
            let mut original = Vec::new();
            held.read_to_end(&mut original).unwrap();
            assert_eq!(original, bytes);
            if after.is_none() {
                assert_eq!(old.modified().unwrap(), new.modified().unwrap(), "{row}");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    assert_eq!(old.ino(), new.ino(), "{row}");
                }
                if !invalid {
                    let mut stdout = String::new();
                    for (line, col, _, message) in &expected {
                        stdout.push_str(&format!(
                            "{relative}:{line}:{col}: E: {message} ({})\n",
                            COPS[0]
                        ));
                    }
                    let n = expected.len();
                    let plural = if n == 1 { "" } else { "s" };
                    let summary = if fix {
                        format!("0 corrections applied, {n} offense{plural} remaining")
                    } else if n == 0 {
                        "no offenses detected".into()
                    } else {
                        format!("{n} offense{plural} detected")
                    };
                    stdout.push_str(&format!("\n1 file inspected, {summary}\n"));
                    assert_eq!(String::from_utf8_lossy(&out.stdout), stdout, "{row}");
                    assert!(out.stderr.is_empty(), "{row}: {out:?}");
                }
            } else {
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains("1 correction applied"),
                    "{out:?}"
                );
                let out = run(dir.path(), &["--only", COPS[0], "--fix", relative]);
                assert_eq!(out.status.code(), Some(0), "{row}: {out:?}");
                let twice = fs::metadata(&path).unwrap();
                assert_eq!(new.modified().unwrap(), twice.modified().unwrap());
            }
        }
    });
}
#[cfg(unix)]
#[test]
fn canonical_atomic_symlink_failure_and_idempotence() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let before = b"if false\n  Foo.send(:include, Bar)\nend\n";
    fs::write(&path, before).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COPS[0], "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(2)
    );
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COPS[0], "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"if false\n  Foo.include Bar\nend\n"
    );
    assert_ne!(old.ino(), fs::metadata(&path).unwrap().ino());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(dir.path().join("lib/link.rb"))
            .unwrap()
            .is_symlink()
    );
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COPS[0], "--fix", "lib/dangling.rb"])
            .status
            .code(),
        Some(2)
    );
    assert!(!dir.path().join("lib/missing.rb").exists());
}
#[test]
fn canonical_directives_observers_and_uncertain_edits_keep_bytes() {
    let dir = project();
    let cop = COPS[0];
    let core = "if false\n  Foo.send(:include, Bar)\nend\n";
    for prefix in ["rustocop", "rubocop"] {
        for text in [
            format!("# {prefix}:disable {cop}\n{core}"),
            format!("# {prefix}:push -{cop}\n{core}# {prefix}:pop\n"),
            core.replace("  Foo", &format!("  # {prefix}:disable-next {cop}\n  Foo")),
            core.replace("Bar)", &format!("Bar) # {prefix}:disable {cop}")),
        ] {
            unchanged(
                dir.path(),
                text.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
    for suffix in [
        "p __LINE__\n",
        "p File.binread(__FILE__)\n",
        "p RubyVM::InstructionSequence.compile_file(__FILE__).to_a\n",
        "p ObjectSpace.count_objects\n",
        "p binding.local_variables\n",
        "DATA.rewind; p DATA.read\n__END__\npayload\n",
        "__END__\npayload\n",
        "p reader.source\n",
        "p retained.script_lines\n",
    ] {
        unchanged(
            dir.path(),
            format!("{core}{suffix}").as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    for replacement in [
        "send('include', Bar)",
        "send(:include, # kept\n Bar)",
        "send(:include, __ENCODING__)",
        "send(:include, factory::Bar)",
        "send(:include, self::Bar)",
        "send(:include, Bar) {}",
        "send(:include, Bar) { _1 }",
        "send(:include, Bar) { it }",
    ] {
        unchanged(
            dir.path(),
            core.replace("send(:include, Bar)", replacement).as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    let text = core.replace("Bar)", "Bar) # retained comment");
    unchanged(
        dir.path(),
        text.as_bytes(),
        &["--only", cop, "lib/a.rb"],
        Some(1),
    );
    assert_eq!(
        run(dir.path(), &["--only", cop, "--fix", "lib/a.rb"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
        text.replace("send(:include, Bar)", "include Bar")
    );
}
#[test]
fn source_file_literal_uses_supplied_stdin_path_not_display_or_disk_contents() {
    let dir = project();
    for (path, n) in [
        ("include", 1),
        ("prepend", 1),
        ("extend", 1),
        ("lib/include", 0),
        ("example.rb", 0),
    ] {
        fs::write(
            dir.path().join(path).parent().unwrap().join("sentinel"),
            b"untouched",
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COPS[0], "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"Foo.send(__FILE__, Bar)\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(n));
        assert_eq!(count(&out, COPS[0]), n as usize);
        assert_eq!(
            fs::read(dir.path().join(path).parent().unwrap().join("sentinel")).unwrap(),
            b"untouched"
        );
    }
}
#[test]
fn native_syntax_only_original_and_canonical_bytes() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&version.stdout).contains("ruby 3.4."));
    for row in rows() {
        for key in ["bytes", "project_after", "upstream_after"] {
            let Some(value) = row.get(key) else {
                continue;
            };
            let bytes: Vec<u8> = serde_json::from_value(value.clone()).unwrap();
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-W0", "-c"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                key != "bytes" || row.get("parser_rejection").is_none(),
                "{row}: {key}: {out:?}"
            );
        }
    }
}
