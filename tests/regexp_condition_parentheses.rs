//! Source-only pinned AST and CLI checks. Never execute reference inputs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = ["Lint/RegexpAsCondition", "Lint/RequireParentheses"];
const SOURCES: [&[u8]; 2] = [b"if /foo/; end\n", b"foo? a && b\n"];
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/regexp_condition_parentheses.json")).unwrap();
    assert_eq!(rows.len(), 3167);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        105
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
fn pinned_main_source_original_bytes_and_parent_shapes_match_without_edits() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let mut failures = Vec::new();
    for row in rows() {
        let cop = row["cop"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
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
        let expected: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
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
    let dir = project();
    for row in rows() {
        let cop = row["cop"].as_str().unwrap();
        if row.get("after").is_some() {
            continue;
        }
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
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
            let expected: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
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
    }
}
#[test]
fn defaults_selectors_toml_department_and_path_precedence() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let detector = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(detector.default_enabled());
        assert_eq!(detector.safe_fixes(), cop == COPS[0]);
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
            // Neither cop highlights the outer conditional's keyword line.
            let sample = if cop == COPS[0] {
                "!/foo/"
            } else {
                "foo? a && b"
            };
            let text = format!("if x # {prefix}:disable {cop}\n {sample}\nend\n");
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
            let multiline = if cop == COPS[0] {
                "if /foo\nbar/\nend\n"
            } else {
                "foo? a &&\n b\n"
            };
            for text in [
                format!("# {prefix}:disable-next {cop}\n{multiline}"),
                if cop == COPS[0] {
                    format!("if /foo\nbar/ # {prefix}:disable {cop}\nend\n")
                } else {
                    format!("foo? a &&\n b # {prefix}:disable {cop}\n")
                },
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
fn canonical_edits_are_real_atomic_byte_preserving_and_idempotent() {
    let dir = project();
    for row in rows().into_iter().filter(|r| r.get("after").is_some()) {
        let before: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let after: Vec<u8> = serde_json::from_value(row["after"].clone()).unwrap();
        let cop = COPS[0];
        unchanged(dir.path(), &before, &["--only", cop, "lib/a.rb"], Some(1));
        let path = dir.path().join("lib/a.rb");
        let old = fs::metadata(&path).unwrap();
        let mut held = fs::File::open(&path).unwrap();
        let out = run(dir.path(), &["--only", cop, "--fix", "lib/a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{row}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 correction applied"));
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        let mut original = Vec::new();
        held.read_to_end(&mut original).unwrap();
        assert_eq!(original, before);
        unchanged(
            dir.path(),
            &after,
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn correction_uncertainty_keeps_findings_and_bytes() {
    let dir = project();
    let cop = COPS[0];
    let canonical = "if false\n if !/foo/\n end\nend\n";
    for source in [
        "if /foo/\nend\n".into(),
        "if false\n if /foo/\n end\nend\n".into(),
        "if !/foo/\nend\n".into(),
        canonical.replace("/foo/", "/(foo)/"),
        canonical.replace("/foo/", "/#{side_effect}/"),
        canonical.replace("/foo/", "/foo/i"),
        canonical.replace("!/foo/", "! /foo/"),
        canonical.replace("!/foo/", "not(/foo/)"),
        canonical.replace("if false", "unless true"),
        canonical.replace("if false", "if true"),
        canonical.replace("if !/foo/", "if !/foo/; nil"),
        format!("{canonical}p __LINE__, __FILE__, __ENCODING__\n"),
        format!("{canonical}File.binread(__FILE__)\n"),
        format!("{canonical}RubyVM::InstructionSequence.compile_file(__FILE__)\n"),
        format!("{canonical}ObjectSpace.each_object(Regexp).count\n"),
        format!("{canonical}TracePoint.new(:line) {{}}.enable\n"),
        format!("{canonical}DATA.read\n__END__\nheld\n"),
        "if false\n if !/#{<<TEXT}/\nfoo\nTEXT\n end\nend\n".into(),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert!(count(&out, cop) > 0, "{source}");
    }
    for prefix in ["rustocop", "rubocop"] {
        let s = canonical.replace("if !/foo/", &format!("if !/foo/ # {prefix}:disable {cop}"));
        unchanged(
            dir.path(),
            s.as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, canonical).unwrap();
        symlink(&path, dir.path().join("alias.rb")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", cop, "--fix", "alias.rb"])
                .status
                .code(),
            Some(2)
        );
        assert_eq!(fs::read(&path).unwrap(), canonical.as_bytes());
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", cop, "--fix", "alias.rb"])
                .status
                .code(),
            Some(0)
        );
        assert!(
            fs::symlink_metadata(dir.path().join("alias.rb"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            canonical.replace("!/foo/", "!(/foo/ =~ $_)")
        );
    }
}
#[test]
fn authored_native_warning_dispatch_binding_value_and_inactive_controls() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    // These bounded programs are authored here, never loaded from the oracle.
    let native = |text: &str, syntax: bool| {
        let path = dir.path().join("witness.rb");
        fs::write(&path, text).unwrap();
        let mut command = Command::new(&ruby);
        command.args(["--parser=prism", "-w"]);
        if syntax {
            command.arg("-c");
        }
        command.arg(path).output().unwrap()
    };
    let canonical = "if false\n if !/foo/\n end\nend\n";
    for header in [
        "",
        "\u{feff}",
        "# coding: ISO-8859-1\n",
        "# coding: UTF8-MAC\n",
    ] {
        for eol in ["\n", "\r\n"] {
            let before = format!("{header}{canonical}").replace('\n', eol);
            let after = before.replace("!/foo/", "!(/foo/ =~ $_)");
            for syntax in [false, true] {
                let a = native(&before, syntax);
                let b = native(&after, syntax);
                assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
                assert_eq!(a.stdout, b.stdout);
                assert_eq!(a.stderr, b.stderr);
                assert!(a.stderr.is_empty(), "{a:?}");
            }
        }
    }
    // Bare regexps warn at parse time even when the branch never runs.
    let bare = canonical.replace("!/foo/", "/foo/");
    let a = native(&bare, true);
    let b = native(&bare.replace("/foo/", "/foo/ =~ $_"), true);
    assert!(String::from_utf8_lossy(&a.stderr).contains("regex literal in condition"));
    assert!(b.stderr.is_empty());
    // Native3.4 dispatches BOTH forms through Regexp#=~. Preserve the
    // actual exception evidence rather than assuming an intrinsic bypass.
    let live = "class Regexp\n def =~(_value); raise 'dispatch changed'; end\nend\n$_ = 'foo'\nif !/foo/\n puts :miss\nelse\n puts :match\nend\n";
    let a = native(live, false);
    let b = native(&live.replace("!/foo/", "!(/foo/ =~ $_)"), false);
    assert!(!a.status.success() && !b.status.success(), "{a:?} {b:?}");
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
    assert!(String::from_utf8_lossy(&b.stderr).contains("dispatch changed"));
    // Named captures introduce locals under explicit =~ even in inactive code.
    let named =
        "if false\n if !/(?<capture>foo)/\n end\nend\np local_variables.include?(:capture)\n";
    let a = native(named, false);
    let b = native(
        &named.replace("!/(?<capture>foo)/", "!(/(?<capture>foo)/ =~ $_)"),
        false,
    );
    assert_eq!(a.stdout, b"false\n");
    assert_eq!(b.stdout, b"true\n");
    // Inert candidates preserve scoped $_/$~/$1, values, control flow and
    // interpolation evaluation count even with independently authored probes.
    let body = "$_ = 'foo'\n/(f)/ =~ $_\nbefore = [$_, $~.to_a, $1, local_variables.sort]\n";
    let tail = "after = [$_, $~.to_a, $1, local_variables.reject { |x| x == :before }.sort]\np before.take(3) == after.take(3)\np local_variables.include?(:capture)\n";
    let s = format!("{body}{canonical}{tail}");
    let a = native(&s, false);
    let b = native(&s.replace("!/foo/", "!(/foo/ =~ $_)"), false);
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
    assert_eq!(a.stdout, b"true\nfalse\n");
    // Authored live probes separately establish dynamic evaluation and $_
    // method scope; they do not expand the project's closed edit boundary.
    let dynamic = "n = 0\n$_ = 'foo'\nif !/#{n += 1; 'foo'}/\n raise 'unreachable'\nend\np n\n";
    let scoped = "$_ = 'outer'\ndef probe\n $_ = 'foo'\n if !/(f)/\n  raise 'unreachable'\n end\n [$1, $_]\nend\np probe\np $_\n";
    for (source, from, to, expected) in [
        (
            dynamic,
            "!/#{n += 1; 'foo'}/",
            "!(/#{n += 1; 'foo'}/ =~ $_)",
            "1\n",
        ),
        (
            scoped,
            "!/(f)/",
            "!(/(f)/ =~ $_)",
            "[\"f\", \"foo\"]\n\"outer\"\n",
        ),
    ] {
        let a = native(source, false);
        let b = native(&source.replace(from, to), false);
        assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
        assert_eq!(a.stdout, expected.as_bytes());
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
    }
    let precedence = "$_ = 'foo'\nif !/foo/\n puts :miss\nelse\n puts :match\nend\n";
    let a = native(precedence, false);
    let b = native(&precedence.replace("!/foo/", "!(/foo/ =~ $_)"), false);
    let naive = native(&precedence.replace("!/foo/", "!/foo/ =~ $_"), false);
    assert_eq!(a.stdout, b"match\n");
    assert_eq!(a.stdout, b.stdout);
    assert!(!naive.status.success() || naive.stdout != a.stdout);
    // Dynamic interpolation/evaluation/exception and method-local $_ remain
    // held, including a closed branch with a source/code-object/heap observer.
    for source in [live.to_string(), "def probe\n $_ = 'foo'\n /(?<capture>foo)/ ? [$1, $_, local_variables] : nil\nend\np probe\n".into(), "n = 0\n$_ = 'foo'\nif !/#{n += 1; 'foo'}/\n raise 'unreachable'\nend\np n\n".into()] {
        unchanged(dir.path(), source.as_bytes(), &["--only", COPS[0], "--fix", "lib/a.rb"], Some(1));
    }
}

#[test]
fn native_ruby34_syntax_only_never_execute_reference_inputs() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("ruby 3.4."));
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let after = row.get("after").or_else(|| {
            (row["origin"] == "spec")
                .then(|| row.get("upstream_after"))
                .flatten()
        });
        if let Some(after) = after {
            let after: Vec<u8> = serde_json::from_value(after.clone()).unwrap();
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-w", "-c"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&after).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{row}: {out:?}");
        }
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
