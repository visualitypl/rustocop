//! source-only parity and real CLI controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/ConstantReassignment";
const SOURCE: &[u8] = b"X = 1\nX = 2\n";
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
        assert_eq!(out.status.code(), Some(status), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/constant_reassignment.json")).unwrap()
}
#[test]
fn every_pinned_spec_source_and_control_matches_pipeline_and_cli_no_edits() {
    let dir = project();
    let rows = rows();
    assert_eq!(rows.len(), 408);
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    for row in rows {
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
        for (line, col, _, message) in expected {
            assert!(
                String::from_utf8_lossy(&out.stdout)
                    .contains(&format!("lib/a.rb:{line}:{col}: E: {message} ({COP})")),
                "{out:?}"
            );
        }
    }
}
#[test]
fn native_ruby34_syntax_only_all_sources() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
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
fn metadata_and_selectors_config_paths_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(registry.has_cop("Lint/UnusedPrivateMethod"));
    let dir = project();
    for (config, select, expected) in [
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
        let mut args = select;
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(dir.path(), SOURCE, &args, None)), expected);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "UseProjectIndex = true",
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
        assert_eq!(count(&run(dir.path(), &["--only", COP, path])), 1);
    }
    fs::create_dir(dir.path().join("vendor")).unwrap();
    fs::write(dir.path().join("vendor/a.rb"), SOURCE).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "vendor"]).status.code(),
        Some(0)
    );
    // No cross-file state or automatic index, even in one multi-file invocation.
    fs::write(dir.path().join("lib/a.rb"), "X = 1\n").unwrap();
    fs::write(dir.path().join("lib/b.rb"), "X = 2\n").unwrap();
    assert_eq!(
        count(&run(dir.path(), &["--only", COP, "lib/a.rb", "lib/b.rb"])),
        0
    );
}
#[test]
fn both_directive_prefixes_all_scopes_preserve_ordered_definition_facts() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\nX = 1\nX = 2\n"),
            format!("X = 1\n# {prefix}:disable-next {COP}\nX =\n 2\n"),
            format!("X = 1\n# {prefix}:next -{COP}\nX = 2\n"),
            format!("X = 1\n# {prefix}:push -{COP}\nX = 2\n# {prefix}:pop\n"),
            format!("X = 1\nX = 2 # {prefix}:todo Lint\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        let source = format!(
            "# {prefix}:disable {COP}\nX = 1\nX = 2\n# {prefix}:push +{COP}\nX = 3\n# {prefix}:pop\nX = 4\n"
        );
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "lib/a.rb"],
                Some(1)
            )),
            1
        );
        fs::write(
            dir.path().join("rustocop.toml"),
            "[cops.Lint]\nenabled = false\n",
        )
        .unwrap();
        let source = format!("X = 1\n# {prefix}:enable-next {COP}\nX = 2\nX = 3\n");
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
fn stdin_provenance_no_fix_and_no_index() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "sentinel\n").unwrap();
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
            assert!(String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:2:1: E:"));
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
fn envelopes_observers_control_flow_errors_and_no_fix_never_write() {
    let dir = project();
    for source in [
        b"\xef\xbb\xbf# encoding: utf-8\r\nX = 1\r\nX = 2\r\np __LINE__\r\n__END__\r\nraw\x00\r\n"
            .as_slice(),
        b"# encoding: Windows-1252\n# \xe9\nX = 1; X = 2\n",
        b"# encoding: Shift_JIS\n# \x82\xa0\nX = 1; X = 2\n",
        b"X = 1\nX = <<~TEXT\ntext\nTEXT\n",
        b"X = 1; X = 2; File.binread(__FILE__); DATA.pos\n__END__\nraw\n",
        b"X = 1; X = 2; RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
        b"X = 1; X = 2; binding.local_variables; p __ENCODING__; p __FILE__\n",
        b"class A; X = 1; X = 2; end if condition\n",
    ] {
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
    }
    unchanged(
        dir.path(),
        b"X = 1; X = 2\nvalues = [1,2]\n",
        &[
            "--only",
            "Lint/ConstantReassignment,Layout/SpaceAfterComma",
            "lib/a.rb",
        ],
        Some(1),
    );
    for source in [
        "X = 1; X = 2\ndef broken(\n".to_string(),
        format!("X = 1; X = 2\n{}nil\n", "x = ".repeat(80)),
    ] {
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(2)
            )),
            0
        );
    }
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "missing.rb"])
            .status
            .code(),
        Some(2)
    );
}
#[cfg(unix)]
#[test]
fn atomic_layout_composition_symlinks_readonly_and_failure() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    let dir = project();
    let before = b"X = 1; X = 2\nvalues = [1,2]\n";
    let after = b"X = 1; X = 2\nvalues = [1, 2]\n";
    let only = "Lint/ConstantReassignment,Layout/SpaceAfterComma";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
    let mut original = fs::File::open(&path).unwrap();
    let inode = original.metadata().unwrap().ino();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/alias.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out), 1);
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
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        old.modified().unwrap()
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions(),
        old.permissions()
    );
    let out = run(dir.path(), &["--only", COP, "--fix", "lib/alias.rb"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        old.modified().unwrap()
    );
}
