//! pinned source-only examples, fixed policy and rebuilt CLI no-write.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COPS: [&str; 2] = ["Lint/DataDefineOverride", "Lint/SharedMutableDefault"];
const SOURCES: [&str; 2] = ["Data.define(:members)\n", "Hash.new([])\n"];

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
        .filter(|l| l.contains(cop))
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
        serde_json::from_str(include_str!("fixtures/data_defaults.json")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 489);
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    for row in rows.as_array().unwrap() {
        let cop = row["cop"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
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
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(i32::from(!expected.is_empty())),
        );
        assert_eq!(count(&out, cop), expected.len(), "{row}\n{out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        for (line, col, _, msg) in expected {
            assert!(
                stdout.contains(&format!("lib/a.rb:{line}:{col}: E: {msg} ({cop})")),
                "{stdout}"
            );
        }
    }
}

#[test]
fn metadata_default_on_ruby34_detection_only_without_paths_settings_or_future_admissions() {
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
    for name in COPS {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled());
        assert!(!cop.safe_fixes());
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
        assert!(cop.max_setting().is_none());
    }
    assert!(registry.has_cop("Lint/UnusedPrivateMethod"));
}

#[test]
fn selectors_department_precedence_and_source_activation_without_fix_never_write() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for (config, select, expected) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint"], 1),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", "Lint"], 0),
            (String::new(), vec!["--only", "Lint/Debugger"], 0),
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
            let out = unchanged(dir.path(), source.as_bytes(), &args, None);
            assert_eq!(count(&out, cop), expected, "{out:?}");
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            "[cops.Lint]\nenabled = false\n",
        )
        .unwrap();
        for prefix in ["rustocop", "rubocop"] {
            let src = format!("# {prefix}:enable {cop}\n{source}");
            let out = unchanged(dir.path(), src.as_bytes(), &["lib/a.rb"], None);
            assert_eq!(count(&out, cop), 1);
        }
    }
}

#[test]
fn strict_toml_exclusions_and_supplied_file_paths() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for key in [
            "max = 1",
            "include = ['**/*']",
            "Enabled = true",
            "enabled = 'true'",
            "target_ruby = 2.6",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(2),
            );
        }
        for config in [
            "exclude = ['lib/a.rb']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
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
        // Built-in excludes remain independent from the absence of cop path gates.
        fs::create_dir_all(dir.path().join("vendor")).unwrap();
        fs::write(dir.path().join("vendor/a.rb"), source).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", cop, "vendor"]).status.code(),
            Some(0)
        );
    }
}

#[test]
fn both_prefixes_multiline_ranges_next_and_push_scopes() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let multiline = if cop == COPS[0] {
            "Data.define(\n :\"members\"\n)\n"
        } else {
            "Hash.new(\n []\n)\n"
        };
        for prefix in ["rubocop", "rustocop"] {
            for src in [
                format!("# {prefix}:disable {cop}\n{source}"),
                format!("# {prefix}:todo Lint\n{source}"),
                format!("# {prefix}:disable-next {cop}\n{multiline}"),
                format!("# {prefix}:next -{cop}\n{multiline}"),
                format!("# {prefix}:push -{cop}\n{multiline}# {prefix}:pop\n"),
            ] {
                unchanged(
                    dir.path(),
                    src.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(0),
                );
            }
            for src in [
                format!(
                    "# {prefix}:disable {cop}\n{source}# {prefix}:push +{cop}\n{source}# {prefix}:pop\n{source}"
                ),
                format!(
                    "# {prefix}:disable {cop}\n# {prefix}:enable-next {cop}\n{multiline}{source}"
                ),
            ] {
                let out = unchanged(
                    dir.path(),
                    src.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(1),
                );
                assert_eq!(count(&out, cop), 1, "{out:?}");
            }
        }
    }
    // Data highlights only the member argument; a Hash offense covers its full send.
    for prefix in ["rubocop", "rustocop"] {
        for (cop, src) in [
            (
                COPS[0],
                format!(
                    "Data.define( # {prefix}:disable {}\n :members\n)\n",
                    COPS[0]
                ),
            ),
            (
                COPS[1],
                format!("Hash.new(\n [] # {prefix}:disable {}\n)\n", COPS[1]),
            ),
        ] {
            let status = if cop == COPS[0] { 1 } else { 0 };
            unchanged(
                dir.path(),
                src.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(status),
            );
        }
    }
}

#[test]
fn stdin_path_is_provenance_and_fix_is_rejected() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, b"sentinel\n").unwrap();
    let old = fs::metadata(&path).unwrap();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
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
            assert_eq!(count(&out, cop), expected);
            assert_eq!(out.status.code(), Some(i32::from(expected != 0)));
            if expected == 1 {
                let col = if cop == COPS[0] { 13 } else { 1 };
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains(&format!("lib/a.rb:1:{col}: E:"))
                );
            }
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
fn source_envelopes_observers_and_non_utf8_never_receive_edits() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for src in [
            format!(
                "\u{feff}# encoding: utf-8\r\n{}p __LINE__\r\n__END__\r\nraw\0\r\n",
                source.replace('\n', "\r\n")
            ),
            format!("x = <<~TEXT\n{source}TEXT\n{source}"),
            format!("{source}File.binread(__FILE__); DATA.pos\n__END__\nraw\n"),
            format!(
                "{source}RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {{}}\n"
            ),
            format!("def f(x = 1); {source}binding.local_variables; end\n"),
            format!("begin\n {source}ensure\n p :done\nend\n"),
            format!("{source}p __ENCODING__; p __FILE__\n"),
        ] {
            let out = unchanged(
                dir.path(),
                src.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(1),
            );
            assert_eq!(count(&out, cop), 1, "{out:?}");
        }
        let mut bytes = b"# encoding: Windows-1252\n# \xe9\n".to_vec();
        bytes.extend_from_slice(source.as_bytes());
        unchanged(
            dir.path(),
            &bytes,
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
}

#[cfg(unix)]
#[test]
fn existing_layout_correction_composes_atomically_and_readonly_failure_retains_findings() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let dir = project();
        let before = format!("{source}values = [1,2]\n");
        let after = format!("{source}values = [1, 2]\n");
        let only = format!("{cop},Layout/SpaceAfterComma");
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, &before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
        let mut original = fs::File::open(&path).unwrap();
        let inode = original.metadata().unwrap().ino();
        let out = run(dir.path(), &["--only", &only, "--fix", "lib/alias.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, cop), 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 correction applied"));
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        let mut old_bytes = Vec::new();
        original.read_to_end(&mut old_bytes).unwrap();
        assert_eq!(old_bytes, before.as_bytes());
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
        fs::write(&path, &before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", &only, "--fix", "lib/alias.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out, cop), 1);
        assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
    }
}

#[test]
fn invalid_resource_io_and_readonly_symlink_controls_preserve_originals() {
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let dir = project();
        for src in [
            format!("{source}def broken(\n"),
            format!("{source}{}nil\n", "x = ".repeat(80)),
        ] {
            let out = unchanged(
                dir.path(),
                src.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(2),
            );
            assert_eq!(count(&out, cop), 0);
        }
        assert_eq!(
            run(dir.path(), &["--only", cop, "--fix", "missing.rb"])
                .status
                .code(),
            Some(2)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let path = dir.path().join("lib/a.rb");
            fs::write(&path, source).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
            symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = run(
                dir.path(),
                &["--only", cop, "--fix", "lib/alias.rb", "lib/a.rb"],
            );
            assert_eq!(out.status.code(), Some(1), "{out:?}");
            assert!(count(&out, cop) >= 1);
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
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
}
