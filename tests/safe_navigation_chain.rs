//! Source-only pinned AST and CLI checks. Never execute fixture input.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 1] = ["Lint/SafeNavigationChain"];
const SOURCES: [&[u8]; 1] = [b"x&.foo.bar\n"];
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/safe_navigation_chain.json")).unwrap();
    assert_eq!(rows.len(), 3951);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        1
    );
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
fn pinned_main_source_original_bytes_and_safe_navigation_shapes_match_without_edits() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let mut failures = Vec::new();
    for row in rows() {
        let cop = COPS[0];
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        if row.get("parser_rejection").is_some() {
            assert!(
                !report.errors.is_empty(),
                "reference rejection unexpectedly accepted: {row}: {report:?}"
            );
            assert!(report.result.offenses.is_empty());
            continue;
        }
        assert!(report.errors.is_empty(), "{row}: {report:?}");
        let mut found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        found.sort();
        let expected: Vec<Finding> = crate::support::fixture::value(&row["findings"]);
        if found != expected {
            failures.push(format!(
                "{cop} {}: expected {expected:?}, found {found:?}",
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
fn all_original_bytes_match_rebuilt_cli_and_fix_never_writes() {
    parallel_rows!(for row in rows() => {
        let dir = project();
        let cop = COPS[0];
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        for fix in [false, true] {
            let mut args = vec!["--only", cop];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            if row.get("parser_rejection").is_some() {
                let out = unchanged(dir.path(), &bytes, &args, Some(2));
                assert_eq!(count(&out, cop), 0);
                continue;
            }
            let expected: Vec<Finding> = crate::support::fixture::value(&row["findings"]);
            let out = unchanged(
                dir.path(),
                &bytes,
                &args,
                Some(i32::from(!expected.is_empty())),
            );
            assert_eq!(count(&out, cop), expected.len(), "{row}: {out:?}");
            let mut stdout = String::new();
            for (line, col, _, message) in &expected {
                stdout.push_str(&format!("lib/a.rb:{line}:{col}: E: {message} ({cop})\n"));
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
    });
}
#[test]
fn defaults_selectors_toml_department_and_path_precedence() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let detector = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(detector.default_enabled());
        assert!(!detector.safe_fixes());
        assert_eq!(
            detector.ruby_versions(),
            rustocop::cops::policy::RubyVersions {
                minimum: Some(rustocop::cops::policy::RubyVersion(2, 3)),
                maximum: None
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
            // A directive on the receiver line does not suppress a later dot.
            let text = format!("x&.foo # {prefix}:disable {cop}\n  .bar\n");
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
                format!("x&.foo.bar(\n  1) # {prefix}:disable {cop}\n"),
                format!("# {prefix}:disable-next {cop}\nx&.foo.bar(\n  1)\n"),
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
fn authored_native_value_changes_are_not_intention_repairs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    // Separately authored bounded witnesses; never execute reference fixtures.
    let dir = project();
    let cop = COPS[0];
    for (source, rewritten) in [
        (
            "v = nil; begin; p v&.foo.bar; rescue NoMethodError; puts :raised; end\n",
            "v = nil; begin; p v&.foo&.bar; rescue NoMethodError; puts :raised; end\n",
        ),
        (
            "v = nil; begin; p v&.foo[0]; rescue NoMethodError; puts :raised; end\n",
            "v = nil; begin; p v&.foo&.[](0); rescue NoMethodError; puts :raised; end\n",
        ),
        (
            "v = nil; begin; p v&.foo + 1; rescue NoMethodError; puts :raised; end\n",
            "v = nil; begin; p v&.foo&.+(1); rescue NoMethodError; puts :raised; end\n",
        ),
        (
            "class Probe; def foo; nil; end; end\nbegin; p Probe.new&.foo.bar; rescue NoMethodError; puts :raised; end\n",
            "class Probe; def foo; nil; end; end\nbegin; p Probe.new&.foo&.bar; rescue NoMethodError; puts :raised; end\n",
        ),
    ] {
        let evaluate = |text: &str| {
            Command::new(&ruby)
                .args(["--parser=prism", "-W0", "-e", text])
                .output()
                .unwrap()
        };
        let before = evaluate(source);
        let after = evaluate(rewritten);
        assert!(
            before.status.success() && after.status.success(),
            "{before:?} {after:?}"
        );
        assert_ne!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(1),
        );
        let retained = evaluate(&fs::read_to_string(dir.path().join("lib/a.rb")).unwrap());
        assert_eq!(before.stdout, retained.stdout);
    }
}

#[test]
fn native_ruby34_syntax_only_never_execute_reference_inputs() {
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
            row.get("parser_rejection").is_none(),
            "{row}: {out:?}"
        );
    }
}
