//! Source-only pinned original-byte, rebuilt CLI and no-write controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/IneffectiveAccessModifier";
const SOURCE: &[u8] = b"class C\nprivate\ndef self.f;end\nend\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<_> =
        serde_json::from_str(include_str!("fixtures/ineffective_access_modifier.json")).unwrap();
    assert_eq!(rows.len(), 434);
    rows
}
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({COP})"))
        .count()
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, bytes).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(out.status.code(), Some(status), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
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
fn all_reference_original_bytes_match_pipeline_exactly_without_corrections() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let mut failures = Vec::new();
    for row in rows() {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}: {report:?}"
        );
        let mut found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Warning);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        found.sort();
        let expected: Vec<Finding> = crate::support::fixture::value(&row["findings"]);
        if found != expected {
            failures.push(format!(
                "{}: expected {expected:?}, found {found:?}",
                String::from_utf8_lossy(&bytes)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
#[test]
fn all_reference_original_bytes_match_rebuilt_cli_and_fix_never_writes() {
    let dir = project_with_lib();
    for row in rows() {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let expected: Vec<Finding> = crate::support::fixture::value(&row["findings"]);
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            let out = unchanged(
                dir.path(),
                &bytes,
                &args,
                Some(i32::from(!expected.is_empty())),
            );
            assert_eq!(count(&out), expected.len(), "{row}: {out:?}");
            let stdout = String::from_utf8_lossy(&out.stdout);
            for (line, col, _, message) in &expected {
                assert!(
                    stdout.contains(&format!("lib/a.rb:{line}:{col}: W: {message} ({COP})\n")),
                    "{row}: {out:?}"
                );
            }
        }
    }
}
#[test]
fn defaults_metadata_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    let dir = project_with_lib();
    for (config, mut args, expected) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint", "--except", COP], 0),
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
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(dir.path(), SOURCE, &args, None)), expected);
    }
    for key in [
        "max = 1",
        "Include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "TargetRubyVersion = 3.3",
        "SafeAutoCorrect = true",
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
}
#[test]
fn supplied_path_stdin_excludes_and_no_fixed_path_gate() {
    let dir = project_with_lib();
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.Lint]\nexclude = ['lib/a.rb']\n",
    )
    .unwrap();
    unchanged(
        dir.path(),
        SOURCE,
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    for config in [
        String::new(),
        "exclude = ['lib/a.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
    ] {
        let n = usize::from(config.is_empty());
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        assert_eq!(
            count(&unchanged(
                dir.path(),
                SOURCE,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(n as i32)
            )),
            n
        );
        assert_eq!(
            count(&run(&dir.path().join("lib"), &["--only", COP, "a.rb"])),
            n
        );
        fs::write(dir.path().join("lib/a.rb"), b"sentinel\n").unwrap();
        let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), n);
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
        run(dir.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for path in ["Gemfile", "lib/a.jb", "lib/.hidden.rb", "lib/a_spec.rb"] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        assert_eq!(count(&run(dir.path(), &["--only", COP, path])), 1);
    }
}
#[test]
fn both_prefixes_multiline_next_push_and_enable_scopes() {
    let dir = project_with_lib();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!(
                "# {prefix}:disable Lint\n{}",
                String::from_utf8_lossy(SOURCE)
            ),
            format!(
                "class C\nprivate\n# {prefix}:disable-next {COP}\ndef self.f(\nx\n);end\nend\n"
            ),
            format!("class C\nprivate\n# {prefix}:next -{COP}\ndef self.f;end\nend\n"),
            format!("class C\nprivate\ndef self.f( # {prefix}:disable {COP}\nx\n);end\nend\n"),
            format!(
                "# {prefix}:push -{COP}\n{}# {prefix}:pop\n",
                String::from_utf8_lossy(SOURCE)
            ),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        let late =
            format!("class C\nprivate\ndef self.f(\nx\n);end # {prefix}:disable {COP}\nend\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                late.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
        for (open, close) in [
            (
                format!("# {prefix}:push +{COP}\n"),
                format!("# {prefix}:pop\n"),
            ),
            (format!("# {prefix}:enable-next {COP}\n"), String::new()),
        ] {
            let source = format!(
                "# {prefix}:disable {COP}\nclass C\nprivate\ndef self.f;end\n{open}def self.g;end\n{close}def self.h;end\nend\n"
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
fn observer_bytes_invalid_syntax_composition_and_atomic_symlink_failure() {
    let dir = project_with_lib();
    for source in [
        b"\xef\xbb\xbf# coding: utf-8\r\nclass C;private;def self.f;end;end\r\np __LINE__\r\n__END__\r\nDATA\x00\r\n".as_slice(),
        b"# coding: Windows-1252\nclass C;private;def self.f;end;end; p '\xe9'\n",
        b"class C;private;def self.f;end;end;File.binread(__FILE__);DATA.pos\n__END__\nraw\n",
        b"class C;private;def self.f;end;end;RubyVM::InstructionSequence.of(method(:f));ObjectSpace.each_object {}\n",
    ] { assert_eq!(count(&unchanged(dir.path(), source, &["--only", COP, "--fix", "lib/a.rb"], Some(1))), 1); }
    unchanged(
        dir.path(),
        b"class C;private;def self.f;end;end;def broken(\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    let peers = format!("{COP},Style/StringLiterals,Layout/SpaceAfterComma");
    let before = b"class C;private;def self.f;end;end\nx = ['a',2]\n";
    let after = b"class C;private;def self.f;end;end\nx = [\"a\", 2]\n";
    let path = dir.path().join("lib/a.rb");
    unchanged(dir.path(), before, &["--only", &peers, "lib/a.rb"], Some(1));
    assert_eq!(
        count(&run(dir.path(), &["--only", &peers, "--fix", "lib/a.rb"])),
        1
    );
    assert_eq!(fs::read(&path).unwrap(), after);
    unchanged(
        dir.path(),
        after,
        &["--only", &peers, "--fix", "lib/a.rb"],
        Some(1),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        symlink(&path, dir.path().join("link.rb")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(count(&out), 1);
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&path, before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", &peers, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), 1);
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
fn native_ruby34_syntax_checks_original_bytes_without_executing_definitions() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let mut child = Command::new(&ruby)
            .args(["-c", "-W0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}: {out:?}");
    }
}
