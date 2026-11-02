//! pinned source-only examples and real rebuilt CLI controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Lint/ConstantDefinitionInBlock";
const SOURCE: &[u8] = b"task { FOO = 1 }\n";

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
        .filter(|l| l.contains(COP))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(out.status.code(), Some(status), "{out:?}\n{source:?}");
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
fn all_pinned_specs_source_chunks_and_controls_match_pipeline_and_cli_without_edits() {
    let dir = project();
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/constant_definition_in_block.json")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 397);
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    for row in rows.as_array().unwrap() {
        let source = row["source"].as_str().unwrap();
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
            serde_json::from_value(row["findings"].clone()).unwrap();
        assert_eq!(found, expected, "{row}");
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(i32::from(!expected.is_empty())),
        );
        assert_eq!(count(&out), expected.len(), "{row}\n{out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        for (line, col, _, msg) in expected {
            assert!(
                stdout.contains(&format!("lib/a.rb:{line}:{col}: E: {msg} ({COP})")),
                "{stdout}"
            );
        }
    }
}

#[test]
fn native_ruby34_accepts_every_reference_fixture_syntax_only() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/constant_definition_in_block.json")).unwrap();
    for row in rows.as_array().unwrap() {
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
fn metadata_default_on_ruby34_eligible_detection_only_without_paths_or_settings() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 491);
    assert_eq!(
        registry
            .cops()
            .iter()
            .filter(|c| c.name().starts_with("Lint/"))
            .count(),
        145
    );
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
    assert!(registry.has_cop("Lint/UnusedPrivateMethod"));
}

#[test]
fn selectors_config_department_precedence_strict_toml_and_paths() {
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
        let out = unchanged(dir.path(), SOURCE, &args, None);
        assert_eq!(count(&out), expected, "{out:?}");
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "AllowedMethods = []",
        "target_ruby = 2.6",
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
        "exclude = ['lib/a.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
        assert_eq!(
            run(&dir.path().join("lib"), &["--only", COP, "a.rb"])
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
        fs::write(dir.path().join(path), SOURCE).unwrap();
        let out = run(dir.path(), &["--only", COP, path]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out), 1);
    }
    fs::create_dir_all(dir.path().join("vendor")).unwrap();
    fs::write(dir.path().join("vendor/a.rb"), SOURCE).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "vendor"]).status.code(),
        Some(0)
    );
}

#[test]
fn both_prefixes_multiline_next_push_and_source_activation() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\ntask {{ FOO = 1 }}\n"),
            format!("# {prefix}:todo Lint\ntask {{ FOO = 1 }}\n"),
            format!("task do\n# {prefix}:disable-next {COP}\n FOO =\n  1\nend\n"),
            format!("task do\n# {prefix}:next -{COP}\n class Foo\n end\nend\n"),
            format!("# {prefix}:push -{COP}\ntask {{ FOO = 1 }}\n# {prefix}:pop\n"),
            format!("task do\n class Foo\n end # {prefix}:disable {COP}\nend\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        for source in [
            format!(
                "# {prefix}:disable {COP}\ntask {{ FOO = 1 }}\n# {prefix}:push +{COP}\ntask {{ BAR = 1 }}\n# {prefix}:pop\ntask {{ BAZ = 1 }}\n"
            ),
            format!(
                "# {prefix}:disable {COP}\n# {prefix}:enable-next {COP}\ntask {{ class Foo; end }}\ntask {{ BAR = 1 }}\n"
            ),
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1),
            );
            assert_eq!(count(&out), 1, "{out:?}");
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            "[cops.Lint]\nenabled = false\n",
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\ntask {{ FOO = 1 }}\n");
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
fn stdin_path_is_provenance_and_fix_is_rejected() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, b"sentinel\n").unwrap();
    let old = fs::metadata(&path).unwrap();
    for (config, expected) in [
        (String::new(), 1),
        ("exclude = ['lib/a.rb']\n".into(), 0),
        (format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"), 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), expected);
        assert_eq!(out.status.code(), Some(i32::from(expected != 0)));
        if expected == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:1:8: E:"));
        }
    }
    assert_eq!(
        run(dir.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(&path).unwrap(), b"sentinel\n");
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}

#[test]
fn source_envelopes_observers_encodings_and_control_flow_never_receive_edits() {
    let dir = project();
    for source in [
        b"\xef\xbb\xbf# encoding: utf-8\r\ntask { FOO = 1 }\r\np __LINE__\r\n__END__\r\nraw\x00\r\n".as_slice(),
        b"# encoding: Windows-1252\n# \xe9\ntask { FOO = 1 }\n",
        b"# encoding: Shift_JIS\n# \x82\xa0\ntask { FOO = 1 }\n",
        b"task { FOO = <<~TEXT\ntext\nTEXT\n}\n",
        b"x = <<~TEXT\ntask { FOO = 1 }\nTEXT\ntask { FOO = 1 }\n",
        b"task { FOO = 1 }; File.binread(__FILE__); DATA.pos\n__END__\nraw\n",
        b"task { FOO = 1 }; RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
        b"task { FOO = 1; binding.local_variables; return }; p __ENCODING__; p __FILE__\n",
        b"begin; task { FOO = 1 }; ensure; p :done; end\n",
    ] {
        let out = unchanged(dir.path(), source, &["--only", COP, "--fix", "lib/a.rb"], Some(1));
        assert_eq!(count(&out), 1, "{out:?}");
    }
}

#[cfg(unix)]
#[test]
fn layout_composes_atomically_and_readonly_failure_retains_findings() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    let dir = project();
    let before = b"task { FOO = 1 }\nvalues = [1,2]\n";
    let after = b"task { FOO = 1 }\nvalues = [1, 2]\n";
    let only = "Lint/ConstantDefinitionInBlock,Layout/SpaceAfterComma";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
    let mut original = fs::File::open(&path).unwrap();
    let inode = original.metadata().unwrap().ino();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/alias.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out), 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 correction applied"));
    assert_eq!(fs::read(&path).unwrap(), after);
    let mut old_bytes = Vec::new();
    original.read_to_end(&mut old_bytes).unwrap();
    assert_eq!(old_bytes, before);
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(dir.path().join("lib/alias.rb"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/alias.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&path).unwrap(), before);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}

#[test]
fn invalid_resource_io_and_detection_only_readonly_symlinks_preserve_originals() {
    let dir = project();
    for source in [
        "task { FOO = 1 }\ndef broken(\n".to_string(),
        format!("task {{ FOO = 1 }}\n{}nil\n", "x = ".repeat(80)),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
        assert_eq!(count(&out), 0);
    }
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "missing.rb"])
            .status
            .code(),
        Some(2)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, SOURCE).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(
            dir.path(),
            &["--only", COP, "--fix", "lib/alias.rb", "lib/a.rb"],
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(count(&out) >= 1);
        assert_eq!(fs::read(&path).unwrap(), SOURCE);
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert!(
            fs::symlink_metadata(dir.path().join("lib/alias.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
