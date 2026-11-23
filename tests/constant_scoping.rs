//! source-only references and real rebuilt CLI controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const RESOLUTION: &str = "Lint/ConstantResolution";
const SCOPING: &str = "Lint/UselessConstantScoping";
const CASES: [(&str, &str); 2] = [
    (RESOLUTION, "value = MyConst\n"),
    (SCOPING, "class Foo\n private\n X = 1\nend\n"),
];
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
        .lines()
        .filter(|l| l.contains(&format!("({cop})")))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str]) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(fs::read(&path).unwrap(), source, "{out:?}");
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}

#[test]
fn every_pinned_spec_source_and_control_matches_pipeline_and_cli_with_no_edits() {
    let dir = project();
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/constant_scoping.json")).unwrap();
    assert_eq!(rows.len(), 1656);
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    for row in rows {
        let cop = row["cop"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let report = rustocop::pipeline::lint_one(
            &dir.path().join("lib/a.rb"),
            Path::new("lib/a.rb"),
            Some(source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}\n{report:?}"
        );
        let mut found: Vec<_> = report
            .result
            .offenses
            .into_iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message)
            })
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> =
            crate::support::fixture::value(&row["findings"]);
        assert_eq!(found, expected, "{row}");
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
        );
        assert_eq!(
            out.status.code(),
            Some(i32::from(!expected.is_empty())),
            "{row}\n{out:?}"
        );
        assert_eq!(count(&out, cop), expected.len(), "{row}\n{out:?}");
        for (line, col, _, msg) in expected {
            assert!(
                String::from_utf8_lossy(&out.stdout)
                    .contains(&format!("lib/a.rb:{line}:{col}: E: {msg} ({cop})")),
                "{row}\n{out:?}"
            );
        }
    }
}

#[test]
fn forwarded_private_constants_cli_keeps_exact_findings_and_source_bytes() {
    let dir = project();
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/forwarded_private_constants.json")).unwrap();
    assert_eq!(rows.len(), 49);
    // These fixtures audit detection-only constant rules. Newly landed literal
    // removals are independently covered by adjacent_strings composition tests.
    let except = format!(
        "{RESOLUTION},Lint/ImplicitStringConcatenation,Lint/TripleQuotes,Lint/LiteralInInterpolation,Lint/UselessAccessModifier"
    );
    for row in rows {
        let source = row["source"].as_str().unwrap();
        let expected: Vec<(u32, u32, u32, String)> =
            crate::support::fixture::value(&row["findings"]);
        for args in [
            vec!["--only", SCOPING, "lib/a.rb"],
            vec!["--only", SCOPING, "--fix", "lib/a.rb"],
            vec!["--only", "Lint", "--except", &except, "--fix", "lib/a.rb"],
        ] {
            let out = unchanged(dir.path(), source.as_bytes(), &args);
            assert_eq!(
                out.status.code(),
                Some(i32::from(!expected.is_empty())),
                "{row}\n{out:?}"
            );
            assert_eq!(count(&out, SCOPING), expected.len(), "{row}\n{out:?}");
            for (line, col, _, msg) in &expected {
                assert!(
                    String::from_utf8_lossy(&out.stdout)
                        .contains(&format!("lib/a.rb:{line}:{col}: E: {msg} ({SCOPING})")),
                    "{row}\n{out:?}"
                );
            }
        }
    }
}

#[test]
fn native_ruby34_accepts_fixture_syntax_without_executing_inputs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/constant_scoping.json")).unwrap();
    let forwarded: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/forwarded_private_constants.json")).unwrap();
    rows.extend(forwarded);
    for row in rows {
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
            .write_all(row["source"].as_str().unwrap().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
    }
}

#[test]
fn operator_recovery_source_projection_pipeline_cli_and_native_syntax_only() {
    let dir = project();
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[SCOPING.into()], &[]);
    let ruby = crate::support::runtime::ruby().ok();
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/constant_scoping_projection.json")).unwrap();
    assert_eq!(rows.len(), 532);
    let review: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/constant_scoping_boundaries.json")).unwrap();
    assert_eq!(review.len(), 330);
    rows.extend(review);
    for row in rows {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let expected: Vec<(u32, u32, u32, String)> =
            crate::support::fixture::value(&row["raw_findings"]);
        let report = rustocop::pipeline::lint_one(
            &dir.path().join("lib/a.rb"),
            Path::new("lib/a.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}\n{report:?}"
        );
        let actual: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        assert_eq!(actual, expected, "{row}");
        for args in [
            vec!["--only", SCOPING, "lib/a.rb"],
            vec!["--only", SCOPING, "--fix", "lib/a.rb"],
        ] {
            let out = unchanged(dir.path(), &bytes, &args);
            assert_eq!(
                out.status.code(),
                Some(i32::from(!expected.is_empty())),
                "{row}\n{out:?}"
            );
            assert_eq!(count(&out, SCOPING), expected.len(), "{row}\n{out:?}");
            for (line, col, _, msg) in &expected {
                assert!(
                    String::from_utf8_lossy(&out.stdout)
                        .contains(&format!("lib/a.rb:{line}:{col}: E: {msg} ({SCOPING})")),
                    "{row}\n{out:?}"
                );
            }
        }
        if let Some(ruby) = &ruby {
            // Parse controlled fixtures only; never execute their Ruby code.
            let out = Command::new(ruby)
                .current_dir(dir.path())
                .args(["--parser=prism", "-c", "lib/a.rb"])
                .output()
                .unwrap();
            assert!(out.status.success(), "{row}\n{out:?}");
        }
    }
}

#[test]
fn exact_live_metadata_default_opt_in_and_no_new_capabilities() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let lint: Vec<_> = registry
        .cops()
        .iter()
        .filter(|c| c.name().starts_with("Lint/"))
        .collect();
    assert_eq!(lint.len(), 160);
    assert_eq!(lint.iter().filter(|c| c.default_enabled()).count(), 155);
    let config = rustocop::config::ResolvedConfig::default();
    // Three version-inactive and four genuinely absent-index prerequisites;
    // registration/configuration is not the default-effective count (148).
    assert_eq!(
        rustocop::cops::select_active(&registry, &config, &[], &[])
            .iter()
            .filter(|c| c.name().starts_with("Lint/"))
            .count(),
        152
    );
    for (name, _) in CASES {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert_eq!(cop.default_enabled(), name == SCOPING);
        assert!(!cop.safe_fixes());
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
        assert!(cop.max_setting().is_none());
    }
    assert!(registry.has_cop("Lint/UnusedPrivateMethod"));
}

#[test]
fn selectors_config_precedence_strict_toml_exclusions_and_explicit_paths() {
    let dir = project();
    for (cop, source) in CASES {
        for (config, select, expected) in [
            (String::new(), vec![], usize::from(cop == SCOPING)),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint"], 1),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
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
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let mut args = select;
            args.push("lib/a.rb");
            let out = unchanged(dir.path(), source.as_bytes(), &args);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_eq!(count(&out, cop), expected, "{out:?}");
        }
        for key in [
            "Only = []",
            "Ignore = []",
            "max = 1",
            "include = ['**/*']",
            "enabled = 'true'",
            "target_ruby = 2.6",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            assert_eq!(
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"]
                )
                .status
                .code(),
                Some(2)
            );
        }
        for config in [
            "exclude = ['lib/a.rb']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            assert_eq!(
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"]
                )
                .status
                .code(),
                Some(0)
            );
            assert_eq!(
                run(&dir.path().join("lib"), &["--only", cop, "a.rb"])
                    .status
                    .code(),
                Some(0)
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for path in [
            "lib/source.arb",
            "lib/source.jb",
            "Rakefile",
            "lib/.hidden.rb",
        ] {
            fs::write(dir.path().join(path), source).unwrap();
            let out = run(dir.path(), &["--only", cop, path]);
            assert_eq!(out.status.code(), Some(1), "{out:?}");
            assert_eq!(count(&out, cop), 1);
        }
    }
}

#[test]
fn both_prefixes_multiline_next_push_and_opt_in_source_activation() {
    let dir = project();
    for (cop, source) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for decorated in [
                format!("# {prefix}:disable {cop}\n{source}"),
                format!("# {prefix}:todo Lint\n{source}"),
                format!("# {prefix}:push -{cop}\n{source}# {prefix}:pop\n"),
                if cop == RESOLUTION {
                    format!("# {prefix}:disable-next {cop}\nMyConst\n")
                } else {
                    format!("class Foo\n private\n# {prefix}:next -{cop}\n X =\n  1\nend\n")
                },
                if cop == RESOLUTION {
                    format!("MyConst # {prefix}:disable {cop}\n")
                } else {
                    format!("class Foo\n private\n X =\n  1 # {prefix}:disable {cop}\nend\n")
                },
            ] {
                let out = unchanged(
                    dir.path(),
                    decorated.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                );
                assert_eq!(out.status.code(), Some(0), "{out:?}");
            }
            let decorated = format!(
                "# {prefix}:disable {cop}\n{source}# {prefix}:push +{cop}\n{source}# {prefix}:pop\n{source}"
            );
            let out = unchanged(
                dir.path(),
                decorated.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
            );
            assert_eq!(count(&out, cop), 1, "{out:?}");
            fs::write(
                dir.path().join("rustocop.toml"),
                "[cops.Lint]\nenabled = false\n",
            )
            .unwrap();
            let decorated = format!("# {prefix}:enable {cop}\n{source}");
            let out = unchanged(dir.path(), decorated.as_bytes(), &["lib/a.rb"]);
            assert_eq!(count(&out, cop), 1, "{out:?}");
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            let next_source = if cop == RESOLUTION {
                format!(
                    "# {prefix}:disable {cop}\n# {prefix}:enable-next {cop}\nMyConst\nOtherConst\n"
                )
            } else {
                format!(
                    "# {prefix}:disable {cop}\nclass Foo\n private\n# {prefix}:enable-next {cop}\n X = 1\n Y = 2\nend\n"
                )
            };
            let out = unchanged(
                dir.path(),
                next_source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
            );
            assert_eq!(count(&out, cop), 1, "{out:?}");
        }
    }
}

#[test]
fn stdin_path_provenance_and_fix_rejection_preserve_existing_file() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, b"sentinel\n").unwrap();
    let old = fs::metadata(&path).unwrap();
    for (cop, source) in CASES {
        for (config, expected) in [
            (String::new(), 1),
            ("exclude = ['lib/a.rb']\n".into(), 0),
            (format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"), 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", cop, "--stdin", "lib/a.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(count(&out, cop), expected, "{out:?}");
            assert_eq!(out.status.code(), Some(i32::from(expected != 0)));
        }
        assert_eq!(
            run(dir.path(), &["--only", cop, "--stdin", "lib/a.rb", "--fix"])
                .status
                .code(),
            Some(2)
        );
    }
    assert_eq!(fs::read(&path).unwrap(), b"sentinel\n");
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}

#[test]
fn encodings_observers_control_flow_and_invalid_inputs_keep_bytes_and_findings() {
    let dir = project();
    for (cop, source) in CASES {
        for prefix in [
            b"\xef\xbb\xbf# encoding: utf-8\r\n".as_slice(),
            b"# encoding: Windows-1252\n# \xe9\n",
            b"# encoding: Shift_JIS\n# \x82\xa0\n",
        ] {
            let mut bytes = prefix.to_vec();
            bytes.extend_from_slice(source.replace('\n', "\r\n").as_bytes());
            let out = unchanged(dir.path(), &bytes, &["--only", cop, "--fix", "lib/a.rb"]);
            assert_eq!(out.status.code(), Some(1), "{out:?}");
            assert_eq!(count(&out, cop), 1);
        }
        for suffix in [
            "p __LINE__; p __ENCODING__; p __FILE__\n",
            "::File.binread(__FILE__); ::DATA.pos\n__END__\nraw\0\n",
            "::RubyVM::InstructionSequence.of(method(:f)); ::ObjectSpace.each_object {}\n",
            "binding.local_variables; begin; nil; ensure; nil; end\n",
            "s = <<~TEXT\nbody\nTEXT\n",
        ] {
            let bytes = format!("{source}{suffix}");
            let out = unchanged(
                dir.path(),
                bytes.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
            );
            assert_eq!(out.status.code(), Some(1), "{out:?}");
            assert_eq!(count(&out, cop), 1);
        }
        for suffix in [
            "def broken(\n".into(),
            format!("{}nil\n", "x = ".repeat(80)),
        ] {
            let bytes = format!("{source}{suffix}");
            let out = unchanged(
                dir.path(),
                bytes.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
            );
            assert_eq!(out.status.code(), Some(2), "{out:?}");
            assert_eq!(count(&out, cop), 0);
        }
        assert_eq!(
            run(dir.path(), &["--only", cop, "--fix", "missing.rb"])
                .status
                .code(),
            Some(2)
        );
    }
}

#[cfg(unix)]
#[test]
fn atomic_layout_composition_symlinks_and_readonly_no_edit_or_failure() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let alias = dir.path().join("lib/alias.rb");
    symlink("a.rb", &alias).unwrap();
    for (cop, source) in CASES {
        let before = format!("{source}values = [1,2]\n");
        let after = format!("{source}values = [1, 2]\n");
        let only = format!("{cop},Layout/SpaceAfterComma");
        fs::write(&path, &before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let mut original = fs::File::open(&path).unwrap();
        let inode = original.metadata().unwrap().ino();
        let out = run(dir.path(), &["--only", &only, "--fix", "lib/alias.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, cop), 1);
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        let mut old_bytes = Vec::new();
        original.read_to_end(&mut old_bytes).unwrap();
        assert_eq!(old_bytes, before.as_bytes());
        assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
        assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
        fs::write(&path, &before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", cop, "--fix", "lib/alias.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, cop), 1);
        assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            old.modified().unwrap()
        );
        let out = run(dir.path(), &["--only", &only, "--fix", "lib/alias.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out, cop), 1);
        assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            old.modified().unwrap()
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions(),
            old.permissions()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    }
}

#[test]
fn redundant_constant_base_follows_configured_resolution_not_selectors() {
    let dir = project();
    for (config, expected) in [
        (String::new(), 1),
        (format!("[cops.\"{RESOLUTION}\"]\nenabled = true\n"), 0),
        ("[cops.Lint]\nenabled = true\n".into(), 0),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{RESOLUTION}\"]\nenabled = false\n"),
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let out = unchanged(
            dir.path(),
            b"::Bar\n",
            &["--only", "Style/RedundantConstantBase", "lib/a.rb"],
        );
        assert_eq!(
            count(&out, "Style/RedundantConstantBase"),
            expected,
            "{out:?}"
        );
        if expected == 0 {
            let fixed = unchanged(
                dir.path(),
                b"::Bar\n",
                &["--only", "Style/RedundantConstantBase", "--fix", "lib/a.rb"],
            );
            assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        }
    }
}
