//! pinned source-only parity and actual rebuilt CLI no-write controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/DuplicateMatchPattern";
const SOURCE: &[u8] = b"case x\nin foo\n  first\nin foo\n  second\nend\n";
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_match_pattern.json")).unwrap();
    assert_eq!(rows.len(), 148);
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
        assert_eq!(out.status.code(), Some(code), "{out:?}\n{source:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source);
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
fn every_reference_row_matches_cli_with_and_without_fix_and_never_writes() {
    let dir = project();
    for row in rows() {
        let source = row["source"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
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
    }
}
#[test]
fn legacy_controls_match_reference_without_changing_source_bytes() {
    byte_rows_match_cli(include_str!("fixtures/duplicate_match_pattern_legacy.json"));
}

#[test]
fn pinned_unicode15_inspection_matches_cli_without_writes() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/duplicate_match_pattern_projection.json"
    ))
    .unwrap();
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|r| r["kind"] == "unicode-version")
        .collect();
    assert_eq!(rows.len(), 6);
    byte_rows_match_cli(&serde_json::to_string(&rows).unwrap());
}

#[test]
fn projected_source_order_aliases_and_mac_composition_match_cli_without_writes() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/duplicate_match_pattern_projection.json"
    ))
    .unwrap();
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|r| r["kind"] != "unicode-version")
        .collect();
    assert!(rows.len() >= 4);
    byte_rows_match_cli(&serde_json::to_string(&rows).unwrap());
}

#[test]
fn projected_same_line_columns_match_reference_without_writes() {
    byte_rows_match_cli(include_str!(
        "fixtures/duplicate_match_pattern_positions.json"
    ));
}

#[test]
fn reference_buffer_header_projection_does_not_change_main_parse_metadata() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/duplicate_match_pattern_projection.json"
    ))
    .unwrap();
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|r| r["kind"] == "buffer-header-control")
        .collect();
    assert_eq!(rows.len(), 5);
    byte_rows_match_cli(&serde_json::to_string(&rows).unwrap());
}

fn byte_rows_match_cli(json: &str) {
    let rows: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
    let dir = project();
    for row in rows {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let findings = row["findings"].as_array().unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            let out = unchanged(
                dir.path(),
                &source,
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
    }
}

#[test]
fn defaults_metadata_selectors_version_gate_strict_toml_and_precedence() {
    use rustocop::cops::{Registry, policy::RubyVersion};
    let registry = Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && !cop.safe_fixes());
    assert_eq!(cop.ruby_versions().minimum, Some(RubyVersion(2, 7)));
    assert!(cop.ruby_versions().maximum.is_none());
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
        assert_eq!(count(&unchanged(dir.path(), SOURCE, &args, None)), n);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 2.7",
        "EnforcedStyle = 'semantic'",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    for config in [
        "exclude = ['lib/**']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn both_prefixes_multiline_pattern_range_excludes_guard_and_body_next_push_scopes() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!(
                "# {prefix}:disable {COP}\n{}",
                String::from_utf8_lossy(SOURCE)
            ),
            format!(
                "# {prefix}:push -{COP}\n{}# {prefix}:pop\n",
                String::from_utf8_lossy(SOURCE)
            ),
            format!("# {prefix}:disable-next {COP}\ncase x; in foo; first; in foo; second; end\n"),
            format!("# {prefix}:next -{COP}\ncase x; in foo; first; in foo; second; end\n"),
            format!(
                "case x\nin [a,\n b]; first\nin [a,\n b] # {prefix}:disable {COP}\n second\nend\n"
            ),
            format!(
                "case x\nin {{a: 1, b: 2}}; first\nin {{\n b: 2, # {prefix}:disable {COP}\n a: 1\n}}; second\nend\n"
            ),
            format!("case x\nin foo; first\nin foo # {prefix}:disable {COP}\n second\nend\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        for source in [
            format!("case x\nin foo; first\nin foo\n second # {prefix}:disable {COP}\nend\n"),
            format!(
                "case x\nin foo if\n test; first\nin foo if\n test # {prefix}:disable {COP}\n second\nend\n"
            ),
            format!(
                "# {prefix}:disable {COP}\n{}# {prefix}:push +{COP}\n{}# {prefix}:pop\n{}",
                String::from_utf8_lossy(SOURCE),
                String::from_utf8_lossy(SOURCE),
                String::from_utf8_lossy(SOURCE)
            ),
            format!(
                "# {prefix}:disable {COP}\n# {prefix}:enable-next {COP}\ncase x; in foo; first; in foo; second; end\n{}",
                String::from_utf8_lossy(SOURCE)
            ),
        ] {
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
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!(
            "# {prefix}:enable {COP}\n{}",
            String::from_utf8_lossy(SOURCE)
        );
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["lib/a.rb"],
                None
            )),
            1
        );
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn supplied_file_paths_stdin_provenance_excludes_and_rejected_stdin_fix() {
    let dir = project();
    let absolute = dir.path().join("lib/a.rb");
    for path in [
        "lib/a.rb",
        "./lib/a.rb",
        absolute.to_str().unwrap(),
        "lib/a.arb",
        "lib/a.jb",
        "Rakefile",
        "lib/.hidden.rb",
    ] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", path]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(count(&out), 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(":4:4: E:"));
        assert_eq!(fs::read(dir.path().join(path)).unwrap(), SOURCE);
    }
    for directory in ["vendor", "node_modules", "db"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        let path = dir.path().join(directory).join(if directory == "db" {
            "schema.rb"
        } else {
            "a.rb"
        });
        fs::write(&path, SOURCE).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", directory])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(fs::read(path).unwrap(), SOURCE);
    }
    for config in ["", "exclude = ['virtual/**']\n"] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "virtual/main.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        let n = usize::from(config.is_empty());
        assert_eq!(out.status.code(), Some(n as i32));
        assert_eq!(count(&out), n);
        if n == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/main.rb:4:4"));
        }
        assert!(!dir.path().join("virtual/main.rb").exists());
    }
    unchanged(
        dir.path(),
        SOURCE,
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
}
#[test]
fn bom_crlf_legacy_encodings_and_observers_preserve_bytes_metadata_and_findings() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(b"case x\r\nin foo; first\r\nin foo; second\r\nend\r\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
    }
    for observer in [
        "p __LINE__",
        "File.read(__FILE__)",
        "IO.read($0)",
        "RubyVM::InstructionSequence.of(method(:f))",
        "RubyVM::AbstractSyntaxTree.parse_file(__FILE__)",
        "ObjectSpace.each_object {}",
        "GC.stat",
        "binding.local_variables",
        "TracePoint.new(:line) {}.enable",
        "__END__\ndata\n",
        "DATA.read\n__END__\ndata\n",
        "eval('x = 1')",
        "<<~DOC\n#{case x; in foo; first; in foo; second; end}\nDOC\n",
    ] {
        let mut source = SOURCE.to_vec();
        source.extend_from_slice(observer.as_bytes());
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
fn shared_atomic_validation_readonly_symlink_failure_and_layout_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, SOURCE).unwrap();
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
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap(), old.permissions()),
            (new.ino(), new.modified().unwrap(), new.permissions())
        );
        assert_eq!(fs::read(&path).unwrap(), SOURCE);
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    for source in [
        b"case x; in foo; first; in foo; second; end\n(\n".to_vec(),
        format!("{}\n", "(".repeat(80)).into_bytes(),
    ] {
        assert_eq!(
            count(&unchanged(
                dir.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(2)
            )),
            0
        );
    }
    let before = b"case x; in [1,2]; first; in [1,2]; second; end\n";
    let after = b"case x; in [1, 2]; first; in [1, 2]; second; end\n";
    let only = "Lint/DuplicateMatchPattern,Layout/SpaceAfterComma";
    unchanged(dir.path(), before, &["--only", only, "lib/a.rb"], Some(1));
    let inode = fs::metadata(&path).unwrap().ino();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/link.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&path).unwrap(), after);
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
    unchanged(
        dir.path(),
        after,
        &["--only", only, "--fix", "lib/a.rb"],
        Some(1),
    );
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/link.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(count(&out), 1);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/dangling.rb"])
            .status
            .code(),
        Some(2)
    );
    assert!(!dir.path().join("lib/missing.rb").exists());
}
#[test]
fn every_reference_input_is_native_syntax_checked_only_never_executed() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let byte_rows: Vec<serde_json::Value> = [
        include_str!("fixtures/duplicate_match_pattern_legacy.json"),
        include_str!("fixtures/duplicate_match_pattern_projection.json"),
        include_str!("fixtures/duplicate_match_pattern_positions.json"),
    ]
    .into_iter()
    .flat_map(|json| serde_json::from_str::<Vec<serde_json::Value>>(json).unwrap())
    .collect();
    let sources = rows()
        .into_iter()
        .map(|r| (r["source"].as_str().unwrap().as_bytes().to_vec(), r))
        .chain(byte_rows.into_iter().map(|r| {
            (
                serde_json::from_value::<Vec<u8>>(r["bytes"].clone()).unwrap(),
                r,
            )
        }));
    for (source, row) in sources {
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-c", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&source).unwrap();
        let out = child.wait_with_output().unwrap();
        if let Some(error) = row["native_script_loader_error"].as_str() {
            // The pin and Rust still fully investigate this authored header
            // control. Native Ruby's script loader rejects non-Ruby shebangs
            // before syntax parsing: assert that real outcome, never skip it.
            assert_eq!(out.status.code(), Some(1), "{row}\n{out:?}");
            assert!(
                String::from_utf8_lossy(&out.stderr).contains(error),
                "{row}\n{out:?}"
            );
        } else {
            assert!(out.status.success(), "{row}\n{out:?}");
        }
    }
}
