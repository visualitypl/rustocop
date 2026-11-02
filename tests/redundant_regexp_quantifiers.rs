//! Real CLI/pipeline and authored native correction-safety controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/RedundantRegexpQuantifiers";
const SAFE: &[u8] = b"[defined?(/(?:a+)+/)]\n";
const AFTER: &[u8] = b"[defined?(/(?:a+)/)]\n";
const ORDINARY: &[u8] = b"value = /(?:a+)+/\n";
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
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(code) = code {
        assert_eq!(out.status.code(), Some(code), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    let new = fs::metadata(&path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(
        (old.modified().unwrap(), old.permissions()),
        (new.modified().unwrap(), new.permissions())
    );
    out
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/redundant_regexp_quantifiers.json")).unwrap()
}
#[test]
fn complete_pinned_sources_and_adversarial_rows_exact_cli_and_no_writes() {
    let dir = project();
    let rows = rows();
    assert_eq!(rows.len(), 3789);
    let mut fixed = 0;
    for row in rows {
        let source = row["source"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        let code = if row["invalid_syntax"] == true && row["fixed_native_valid"] != true {
            2
        } else {
            i32::from(!findings.is_empty())
        };
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "lib/a.rb"],
            Some(code),
        );
        assert_eq!(count(&out), findings.len(), "{row}\n{out:?}");
        for f in findings {
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!(
                    "lib/a.rb:{}:{}: E: {} ({COP})",
                    f[0],
                    f[1],
                    f[3].as_str().unwrap()
                )),
                "{row}\n{out:?}"
            );
        }
        let after = row["project_after"].as_str().unwrap();
        if after == source {
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(code)
                )),
                findings.len()
            );
        } else {
            fixed += 1;
            let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
            let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{row}\n{out:?}");
            assert_eq!(
                fs::read(dir.path().join("lib/a.rb")).unwrap(),
                after.as_bytes()
            );
            assert_eq!(
                old.permissions(),
                fs::metadata(dir.path().join("lib/a.rb"))
                    .unwrap()
                    .permissions()
            );
            unchanged(
                dir.path(),
                after.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
    assert_eq!(fixed, 10);
}
#[cfg(unix)]
#[test]
fn shorthand_cli_lint_and_fix_preserve_open_fd_inode_mtime_and_mode() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt},
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let ruby = std::env::var("RUSTOCOP_RUBY34").ok();
    for part in ["#@value", "#@@value", "#$value"] {
        for options in ["", "x"] {
            for (pattern, cols) in [
                (format!("(?:a+)+{part}"), vec![6]),
                (format!("{part}(?:a+)+"), vec![5 + part.len()]),
                (format!("(?:a+)+ {part} (?:b+)+"), vec![6, 14 + part.len()]),
            ] {
                let source = format!("/{pattern}/{options}\n");
                if let Some(ruby) = &ruby {
                    assert!(syntax(ruby, source.as_bytes()).status.success(), "{source}");
                }
                fs::write(&path, &source).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
                for fix in [false, true] {
                    let mut fd = fs::File::open(&path).unwrap();
                    let old = fd.metadata().unwrap();
                    let mut args = vec!["--only", COP, "lib/a.rb"];
                    if fix {
                        args.push("--fix");
                    }
                    let out = run(dir.path(), &args);
                    assert_eq!(out.status.code(), Some(1), "{source}\n{out:?}");
                    assert_eq!(count(&out), cols.len(), "{source}\n{out:?}");
                    for col in &cols {
                        assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("lib/a.rb:1:{col}: E: Replace redundant quantifiers `+` and `+` with a single `+`. ({COP})")), "{source}\n{out:?}");
                    }
                    let new = fs::metadata(&path).unwrap();
                    assert_eq!(
                        (old.ino(), old.mtime(), old.mtime_nsec(), old.mode()),
                        (new.ino(), new.mtime(), new.mtime_nsec(), new.mode())
                    );
                    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                    let mut bytes = vec![];
                    fd.read_to_end(&mut bytes).unwrap();
                    assert_eq!(bytes, source.as_bytes());
                }
            }
        }
    }
}
#[test]
fn interpolation_type_percent_matchlastline_options_and_encoded_projection_controls() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/redundant_regexp_quantifier_interpolation.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 386);
    let dir = project();
    let ruby = std::env::var("RUSTOCOP_RUBY34").ok();
    let path = dir.path().join("lib/a.rb");
    for row in rows {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let findings = row["findings"].as_array().unwrap();
        if let Some(ruby) = &ruby {
            let out = syntax(ruby, &source);
            assert!(out.status.success(), "{row}\n{out:?}");
        }
        for fix in [false, true] {
            fs::write(&path, &source).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
            }
            let mut fd = fs::File::open(&path).unwrap();
            let old = fd.metadata().unwrap();
            let mut args = vec!["--only", COP, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = run(dir.path(), &args);
            assert_eq!(
                out.status.code(),
                Some(i32::from(!findings.is_empty())),
                "{row}\n{out:?}"
            );
            let expected: Vec<_> = findings
                .iter()
                .map(|f| {
                    format!(
                        "lib/a.rb:{}:{}: E: {} ({COP})",
                        f[0],
                        f[1],
                        f[3].as_str().unwrap()
                    )
                })
                .collect();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let actual: Vec<_> = stdout
                .lines()
                .filter(|l| l.ends_with(&format!("({COP})")))
                .collect();
            assert_eq!(actual, expected, "{row}\n{out:?}");
            let new = fs::metadata(&path).unwrap();
            assert_eq!(
                (old.modified().unwrap(), old.permissions()),
                (new.modified().unwrap(), new.permissions())
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!((old.ino(), old.mtime_nsec()), (new.ino(), new.mtime_nsec()));
            }
            assert_eq!(fs::read(&path).unwrap(), source);
            let mut bytes = vec![];
            std::io::Read::read_to_end(&mut fd, &mut bytes).unwrap();
            assert_eq!(bytes, source);
        }
    }
}
#[test]
fn octal_token_boundaries_preserve_cli_findings_and_no_write_holds() {
    let dir = project();
    let ruby = std::env::var("RUSTOCOP_RUBY34").ok();
    for (octal, expected) in [
        (r"\0000", 0),
        (r"\012345", 0),
        (r"\0777", 0),
        (r"\0000000", 0),
        (r"\0", 1),
        (r"\00", 1),
        (r"\000", 1),
        (r"\012", 1),
        (r"\077", 1),
    ] {
        let source = format!("/(?:{octal}+)+/\n");
        if let Some(ruby) = &ruby {
            let out = syntax(ruby, source.as_bytes());
            assert!(out.status.success(), "{source}\n{out:?}");
        }
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &args,
                Some(i32::from(expected != 0)),
            );
            assert_eq!(count(&out), expected, "{source}\n{out:?}");
        }
    }
}
#[test]
fn nested_overlapping_atomic_envelopes_retain_findings_and_converge() {
    let dir = project();
    for (before, after) in [
        ("[defined?(/(?:(?:a?)+)+/)]\n", "[defined?(/(?:(?:a*))/)]\n"),
        ("[defined?(/(?:(?:a+)?)+/)]\n", "[defined?(/(?:(?:a*))/)]\n"),
        ("[defined?(/(?:(?:a+)+)?/)]\n", "[defined?(/(?:(?:a*))/)]\n"),
        (
            "[defined?(/(?:a+)+(?:b+)+/)]\n",
            "[defined?(/(?:a+)(?:b+)/)]\n",
        ),
    ] {
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "lib/a.rb"],
            Some(1),
        );
        for _ in 0..3 {
            let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            if out.status.success() {
                break;
            }
            assert!(count(&out) > 0, "deferred finding was lost: {out:?}");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            after
        );
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn all_reference_inputs_and_upstream_edits_are_native_syntax_only() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        for key in ["source", "upstream_after"] {
            let out = syntax(&ruby, row[key].as_str().unwrap().as_bytes());
            assert_eq!(
                out.status.success(),
                row["invalid_syntax"] != true || row["fixed_native_valid"] == true,
                "{row}\n{key}\n{out:?}"
            );
        }
    }
}
fn syntax(ruby: &str, source: &[u8]) -> Output {
    let mut child = Command::new(ruby)
        .args(["--parser=prism", "-c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn legacy_original_bytes_projected_columns_and_no_edits() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/redundant_regexp_quantifier_legacy.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 24);
    let dir = project();
    for row in rows {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let findings = row["findings"].as_array().unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = unchanged(
                dir.path(),
                &bytes,
                &args,
                Some(i32::from(!findings.is_empty())),
            );
            assert_eq!(count(&out), findings.len(), "{row}\n{out:?}");
            for f in findings {
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains(&format!(
                        "lib/a.rb:{}:{}: E: {} ({COP})",
                        f[0],
                        f[1],
                        f[3].as_str().unwrap()
                    )),
                    "{row}\n{out:?}"
                );
            }
        }
        if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
            assert!(syntax(&ruby, &bytes).status.success(), "{row}");
        }
    }
}
#[test]
fn default_on_metadata_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 491);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    let dir = project();
    for (config, selectors, n) in [
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
        assert_eq!(count(&unchanged(dir.path(), ORDINARY, &args, None)), n);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 3.3",
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
fn both_directive_prefixes_multiline_next_push_pop_and_reenable() {
    let dir = project();
    for p in ["rubocop", "rustocop"] {
        for source in [
            format!("# {p}:disable {COP}\nvalue = /(?:a+)+/\n"),
            format!("# {p}:push -{COP}\nvalue = /(?:a+)+/\n# {p}:pop\n"),
            format!("# {p}:disable-next {COP}\nvalue = /(?:a+)+/\n"),
            format!("# {p}:next -{COP}\nvalue = /(?:a+)+/\n"),
            format!("[defined?(/(?:a+)+/)] # {p}:disable {COP}\n"),
            format!("value = /(?:a\n+)+/ # {p}:disable {COP}\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        let source = format!("value = /(?:a+)+\n/ # {p}:disable {COP}\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
        for open in [
            format!("# {p}:push +{COP}"),
            format!("# {p}:enable-next {COP}"),
        ] {
            let source = format!(
                "# {p}:disable {COP}\nvalue = /(?:a+)+/\n{open}\nvalue = /(?:a+)+/\n# {p}:pop\nvalue = /(?:a+)+/\n"
            );
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(1)
                )),
                1
            );
        }
    }
}
#[test]
fn supplied_paths_stdin_provenance_and_stdin_fix_rejection() {
    let dir = project();
    for path in [
        "lib/a.rb",
        "a.jb",
        "view.erb",
        "db/schema.rb",
        "vendor/a.rb",
        ".hidden.rb",
    ] {
        let p = dir.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, ORDINARY).unwrap();
        let n = usize::from(!path.starts_with("vendor/") && path != "db/schema.rb");
        assert_eq!(count(&run(dir.path(), &["--only", COP, path])), n, "{path}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(ORDINARY).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), n, "{path}\n{out:?}");
        if n == 1 {
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:1:14:")),
                "{out:?}"
            );
        }
    }
    assert_eq!(
        run(dir.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
}
#[test]
fn canonical_edits_preserve_encoding_bom_crlf_comments_and_withhold_observers() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
        b"# coding: UTF8-MAC\n# e\xcc\x81\n",
    ] {
        let source = [prefix, b"# keep\r\n[defined?(/(?:a+)+/)] # consume\r\n"].concat();
        let after = [prefix, b"# keep\r\n[defined?(/(?:a+)/)] # consume\r\n"].concat();
        unchanged(dir.path(), &source, &["--only", COP, "lib/a.rb"], Some(1));
        let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), after);
    }
    for observer in [
        "p __LINE__",
        "File.read(__FILE__)",
        "IO.read($0)",
        "RubyVM::InstructionSequence.compile_file(__FILE__)",
        "RubyVM::AbstractSyntaxTree.parse_file(__FILE__)",
        "ObjectSpace.each_object {}",
        "GC.stat",
        "binding.local_variables",
        "TracePoint.new(:line) {}.enable",
        "__END__\ndata\n",
        "DATA.read\n__END__\ndata\n",
        "<<~DOC\n#{/(?:a+)+/}\nDOC\n",
    ] {
        let source = [SAFE, observer.as_bytes(), b"\n"].concat();
        assert!(
            count(&unchanged(
                dir.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )) >= 1
        );
    }
}
#[cfg(unix)]
#[test]
fn atomic_validation_symlinks_readonly_permissions_and_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, ORDINARY).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for alias in ["lib/a.rb", "lib/link.rb"] {
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", alias])
                .status
                .code(),
            Some(1)
        );
    }
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
    assert_eq!(fs::read(&path).unwrap(), ORDINARY);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(2)
    );
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    unchanged(
        dir.path(),
        b"[defined?(/(?:a+)+/)]\n(\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    fs::write(&path, SAFE).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(0)
    );
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
    for (source, after, peers) in [
        (
            "[defined?(/(?:a+)+/)]  \n",
            "[defined?(/(?:a+)/)]\n",
            "Layout/TrailingWhitespace",
        ),
        (
            "[defined?(%r{(?:a+)+})]\n",
            "[defined?(/(?:a+)+/)]\n",
            "Style/RegexpLiteral",
        ),
        (
            "[defined?(/(?:a+)+/),'x']\n",
            "[defined?(/(?:a+)+/),\"x\"]\n",
            "Style/StringLiterals",
        ),
        (
            "value = [/(?:a+)+/,1]\n",
            "value = [/(?:a+)+/, 1]\n",
            "Layout/SpaceAfterComma",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let only = format!("{COP},{peers}");
        run(dir.path(), &["--only", &only, "--fix", "lib/a.rb"]);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        if peers == "Style/RegexpLiteral" {
            // The shared atomic overlap planner defers the nested envelope.
            assert_eq!(
                run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"])
                    .status
                    .code(),
                Some(0)
            );
            assert_eq!(fs::read(&path).unwrap(), AFTER);
        }
    }
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/dangling.rb"])
            .status
            .code(),
        Some(2)
    );
    assert!(!dir.path().join("lib/missing.rb").exists());
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 3);
}
fn native(ruby: &str, source: &str) -> Output {
    Command::new(ruby)
        .args(["--parser=prism", "-w", "-e", source])
        .output()
        .unwrap()
}
#[test]
fn authored_native_value_binding_warning_control_flow_and_heap_proofs() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    for (before, after) in [
        ("[defined?(/(?:a+)+/)]", "[defined?(/(?:a+)/)]"),
        ("[defined?(/(?:a+)?/)]", "[defined?(/(?:a*)/)]"),
        ("[defined?(/(?:a{1,}){,1}/)]", "[defined?(/(?:a*)/)]"),
    ] {
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "lib/a.rb"],
            Some(1),
        );
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            after
        );
        for (a, b) in [
            (before.to_string(), after.to_string()),
            (
                format!(
                    "value = (begin; {before}; end); p [value, value.class.name, value[0].encoding.name, TOPLEVEL_BINDING.local_variables, $~, $_]"
                ),
                format!(
                    "value = (begin; {after}; end); p [value, value.class.name, value[0].encoding.name, TOPLEVEL_BINDING.local_variables, $~, $_]"
                ),
            ),
            (
                format!(
                    "GC.start; n = ObjectSpace.each_object(Regexp).count; value = {before}; p [value, ObjectSpace.each_object(Regexp).count - n]"
                ),
                format!(
                    "GC.start; n = ObjectSpace.each_object(Regexp).count; value = {after}; p [value, ObjectSpace.each_object(Regexp).count - n]"
                ),
            ),
        ] {
            let a = native(&ruby, &a);
            let b = native(&ruby, &b);
            assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
            assert_eq!(a.stdout, b.stdout);
            assert_eq!(a.stderr, b.stderr);
            assert!(a.stderr.is_empty(), "{a:?}");
        }
    }
    let out = native(
        &ruby,
        "events = []; tp = TracePoint.new(:call, :c_call) { |t| events << t.method_id if t.defined_class == Regexp }; tp.enable; value = [defined?(/(?:a+)+/), defined?(/(?:a+)/)]; tp.disable; p [value, events]",
    );
    assert!(out.status.success() && out.stderr.is_empty(), "{out:?}");
    assert_eq!(out.stdout, b"[[\"expression\", \"expression\"], []]\n");
    // Ordinary evaluated literals expose changed #source even when matching
    // results happen to agree. They retain findings, not optimistic edits.
    for before in [
        "value = /(?:a+)+/; p [value.source, value.options, value.match?('aaa')]",
        "[defined?(/(?:a+)+/.match?(effect))]",
        "value = /(?:a+)+/; p [1, value.source]; raise 'stop'",
    ] {
        let after = before.replace("(?:a+)+", "(?:a+)");
        let a = native(&ruby, before);
        let b = native(&ruby, &after);
        assert_eq!(a.status.code(), b.status.code());
        assert!(a.stdout != b.stdout || a.stderr != b.stderr, "{a:?}\n{b:?}");
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
}
#[test]
fn authored_native_source_data_code_object_and_encoding_controls() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for observer in [
        "p File.binread(__FILE__).bytesize\n",
        "p IO.read($0).bytesize\n",
        "p RubyVM::InstructionSequence.compile_file(__FILE__).to_a\n",
        "p DATA.pos\n__END__\ndata\n",
    ] {
        let before = [SAFE, observer.as_bytes()].concat();
        let after = [AFTER, observer.as_bytes()].concat();
        unchanged(
            dir.path(),
            &before,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
        let a = Command::new(&ruby)
            .args(["--parser=prism", "-w"])
            .arg(&path)
            .output()
            .unwrap();
        fs::write(&path, after).unwrap();
        let b = Command::new(&ruby)
            .args(["--parser=prism", "-w"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert!(
            a.stdout != b.stdout || a.stderr != b.stderr,
            "{observer}\n{a:?}\n{b:?}"
        );
    }
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        for body in [SAFE, AFTER] {
            fs::write(&path, [prefix, b"# keep\r\n", body].concat()).unwrap();
            let out = Command::new(&ruby)
                .args(["--parser=prism", "-w"])
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
                "{out:?}"
            );
        }
    }
}
