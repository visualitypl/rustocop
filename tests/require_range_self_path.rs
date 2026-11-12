//! Source-only pinned AST and CLI checks. Never execute reference inputs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = [
    "Lint/RequireRangeParentheses",
    "Lint/RequireRelativeSelfPath",
];
const SOURCES: [&[u8]; 2] = [b"x = 42..\nvalue\n", b"require_relative 'foo'\n"];
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/require_range_self_path.json")).unwrap();
    assert_eq!(rows.len(), 6457);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        34
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
    let path = root.join("lib/foo.rb");
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
            Path::new(row["path"].as_str().unwrap()),
            Path::new(row["path"].as_str().unwrap()),
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
fn binary_message_bytes_are_explicit_not_lossy_replacements() {
    let source = b"# coding: ASCII-8BIT\nx = foo(# \xe9\n bar)..\nvalue\n";
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COPS[0].into()], &[]);
    let report = rustocop::pipeline::lint_one(
        Path::new("foo.rb"),
        Path::new("foo.rb"),
        Some(source),
        &active,
        &config,
    );
    assert!(report.errors.is_empty() && report.internal_errors.is_empty());
    assert_eq!(report.result.offenses.len(), 1);
    let finding = &report.result.offenses[0];
    assert_eq!((finding.line, finding.col, finding.end_line), (2, 5, 4));
    assert_eq!(
        finding.message,
        "Wrap the range literal `foo(# \\xE9\n bar)..` in parentheses to avoid confusion with an endless range."
    );
    assert!(!finding.message.contains('\u{fffd}'));
    let dir = project();
    unchanged(
        dir.path(),
        source,
        &["--only", COPS[0], "--fix", "lib/foo.rb"],
        Some(1),
    );
}
fn stdin(root: &Path, cop: &str, path: &str, source: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(["--only", cop, "--stdin", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn original_byte_rows_rebuilt_cli_supplied_stdin_paths_and_file_fix_controls() {
    parallel_rows!(for row in rows() => {
        let dir = project();
        let cop = row["cop"].as_str().unwrap();
        let path = row["path"].as_str().unwrap();
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let expected = row["findings"].as_array().map_or(0, Vec::len);
        let status = if row.get("parser_rejection").is_some() {
            2
        } else {
            i32::from(expected > 0)
        };
        let out = stdin(dir.path(), cop, path, &bytes);
        assert_eq!(out.status.code(), Some(status), "{row}: {out:?}");
        assert_eq!(count(&out, cop), expected, "{row}: {out:?}");
        if status != 2 {
            let findings: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
            for (line, col, _, message) in findings {
                // Text formatting escapes line breaks; pipeline above checks raw messages.
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains(&format!(":{line}:{col}: E:")),
                    "{row}: {out:?}"
                );
                assert!(
                    String::from_utf8_lossy(&out.stdout)
                        .contains(message.split('\n').next().unwrap()),
                    "{row}: {out:?}"
                );
            }
        }
        if path != "foo.rb" || row.get("after").is_some() {
            continue;
        }
        // All ordinary rows request corrections as well; no flag/whole detector suppression.
        for fix in [false, true] {
            let mut args = vec!["--only", cop];
            if fix {
                args.push("--fix");
            }
            args.push("lib/foo.rb");
            // __FILE__ is actual supplied provenance, not the diagnostic path.
            let n = if cop == COPS[1] && bytes.windows(8).any(|w| w == b"__FILE__") {
                0
            } else {
                expected
            };
            let status = if status == 2 { 2 } else { i32::from(n > 0) };
            let out = unchanged(dir.path(), &bytes, &args, Some(status));
            assert_eq!(count(&out, cop), n, "{row}: {out:?}");
        }
    });
}
#[test]
fn defaults_selectors_strict_toml_department_exclusions_and_version_policy() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let detector = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(detector.default_enabled());
        assert_eq!(detector.safe_fixes(), cop == COPS[1]);
        assert_eq!(
            detector.ruby_versions(),
            rustocop::cops::policy::RubyVersions {
                minimum: None,
                maximum: None
            }
        );
        assert!(detector.path_policy().is_none() && detector.max_setting().is_none());
        assert!(detector.autocorrect_incompatible_with().is_empty());
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
                format!("[cops.\"{cop}\"]\nexclude = ['lib/foo.rb']\n"),
                vec!["--only", cop],
                0,
            ),
            ("exclude = ['lib/foo.rb']\n".into(), vec!["--only", cop], 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            args.push("lib/foo.rb");
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
                &["--only", cop, "--fix", "lib/foo.rb"],
                Some(2),
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for path in ["vendor/foo.rb", "db/schema.rb"] {
            let file = dir.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, source).unwrap();
            assert_eq!(
                count(&run(dir.path(), &["--only", cop, "--fix", path]), cop),
                0
            );
            assert_eq!(fs::read(file).unwrap(), source);
        }
    }
}
#[test]
fn supplied_file_and_stdin_paths_are_provenance_not_required_file_reads() {
    let dir = project();
    let cop = COPS[1];
    for (path, n) in [
        ("foo.rb", 1),
        ("lib/foo.rb", 1),
        ("other.rb", 0),
        ("foo.rake", 0),
        ("Rakefile", 0),
    ] {
        let file = dir.path().join(path);
        fs::write(&file, b"sentinel\n").unwrap();
        let old = fs::metadata(&file).unwrap();
        assert_eq!(count(&stdin(dir.path(), cop, path, SOURCES[1]), cop), n);
        assert_eq!(fs::read(&file).unwrap(), b"sentinel\n");
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&file).unwrap().modified().unwrap()
        );
        fs::write(&file, SOURCES[1]).unwrap();
        assert_eq!(
            count(&run(dir.path(), &["--only", cop, "--fix", path]), cop),
            n
        );
        assert_eq!(fs::read(&file).unwrap(), SOURCES[1]);
    }
    for cop in COPS {
        assert_eq!(
            run(
                dir.path(),
                &["--only", cop, "--stdin", "lib/foo.rb", "--fix"]
            )
            .status
            .code(),
            Some(2)
        );
    }
}
#[test]
fn directives_both_prefixes_multiline_next_push_and_source_activation() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let source = std::str::from_utf8(source).unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for text in [
                format!("# {prefix}:disable Lint\n{source}"),
                format!("# {prefix}:disable-next {cop}\n{source}"),
                format!("# {prefix}:next -{cop}\n{source}"),
                format!("# {prefix}:push -{cop}\n{source}# {prefix}:pop\n"),
                format!("{} # {prefix}:disable {cop}\n", source.trim_end()),
            ] {
                unchanged(
                    dir.path(),
                    text.as_bytes(),
                    &["--only", cop, "--fix", "lib/foo.rb"],
                    Some(0),
                );
            }
            let text = format!(
                "# {prefix}:disable {cop}\n{source}# {prefix}:push +{cop}\n{source}# {prefix}:pop\n{source}"
            );
            assert_eq!(
                count(
                    &unchanged(
                        dir.path(),
                        text.as_bytes(),
                        &["--only", cop, "--fix", "lib/foo.rb"],
                        Some(1)
                    ),
                    cop
                ),
                1
            );
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            let text = format!("# {prefix}:enable {cop}\n{source}");
            assert_eq!(
                count(
                    &unchanged(dir.path(), text.as_bytes(), &["lib/foo.rb"], None),
                    cop
                ),
                1
            );
            assert_eq!(
                count(
                    &unchanged(
                        dir.path(),
                        text.as_bytes(),
                        &["--except", cop, "lib/foo.rb"],
                        None
                    ),
                    cop
                ),
                0
            );
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            let multiline = if cop == COPS[0] {
                "x = 42..\n value\n"
            } else {
                "require_relative(\n 'foo'\n)\n"
            };
            for text in [
                format!("# {prefix}:disable-next {cop}\n{multiline}"),
                format!("{} # {prefix}:disable {cop}\n", multiline.trim_end()),
            ] {
                unchanged(
                    dir.path(),
                    text.as_bytes(),
                    &["--only", cop, "--fix", "lib/foo.rb"],
                    Some(0),
                );
            }
        }
    }
}
#[test]
fn canonical_whole_line_removal_is_atomic_byte_preserving_and_idempotent() {
    let dir = project();
    let cop = COPS[1];
    for row in rows().into_iter().filter(|r| r.get("after").is_some()) {
        let before: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let after: Vec<u8> = serde_json::from_value(row["after"].clone()).unwrap();
        unchanged(dir.path(), &before, &["--only", cop, "lib/foo.rb"], Some(1));
        let path = dir.path().join("lib/foo.rb");
        let old = fs::metadata(&path).unwrap();
        let mut held = fs::File::open(&path).unwrap();
        let out = run(dir.path(), &["--only", cop, "--fix", "lib/foo.rb"]);
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
            &["--only", cop, "--fix", "lib/foo.rb"],
            Some(0),
        );
    }
}
#[test]
fn uncertain_calls_observers_comments_bindings_and_data_keep_findings_and_bytes() {
    let dir = project();
    let cop = COPS[1];
    let canonical = "if false\n require_relative 'foo'\nend\n";
    for source in [
        "require_relative 'foo'\n".into(),
        canonical.replace("if false", "if true"),
        canonical.replace("if false", "unless true"),
        canonical.replace("require_relative", "obj.require_relative"),
        canonical.replace("'foo'", "'foo', effect"),
        canonical.replace("'foo'", "'foo' do; x = 1; end"),
        canonical.replace("'foo'", "'foo' # retained"),
        canonical.replace("'foo'", "'foo'; nil"),
        canonical.replace("'foo'", "%q(foo)"),
        canonical.replace("'foo'", "\"f\\x6fo\""),
        canonical.replace("require_relative", "value = require_relative"),
        canonical.replace("'foo'", "'fo' 'o'"),
        format!("{canonical}p __LINE__, __FILE__, __ENCODING__\n"),
        format!("{canonical}File.binread(__FILE__)\n"),
        format!("{canonical}RubyVM::InstructionSequence.compile_file(__FILE__)\n"),
        format!("{canonical}ObjectSpace.each_object(String).count\n"),
        format!("{canonical}TracePoint.new(:line) {{}}.enable\n"),
        format!("{canonical}DATA.read\n__END__\nraw\0\n"),
        format!("def f\n{canonical}end\n"),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "lib/foo.rb"],
            Some(1),
        );
        assert!(count(&out, cop) > 0, "{source}");
    }
    for prefix in ["rubocop", "rustocop"] {
        let text = canonical.replace("'foo'", &format!("'foo' # {prefix}:disable {cop}"));
        unchanged(
            dir.path(),
            text.as_bytes(),
            &["--only", cop, "--fix", "lib/foo.rb"],
            Some(0),
        );
    }
}
#[test]
fn parser_errors_composition_and_symlink_atomic_failure_preserve_contracts() {
    let dir = project();
    let cops = COPS.join(",");
    for (cop, s) in COPS.into_iter().zip(SOURCES) {
        let mut invalid = s.to_vec();
        invalid.extend_from_slice(b"def broken(\n");
        unchanged(
            dir.path(),
            &invalid,
            &["--only", cop, "--fix", "lib/foo.rb"],
            Some(2),
        );
    }
    let peers = format!("{cops},Style/StringLiterals,Layout/SpaceAfterComma");
    let before = b"x = 42..\nvalue\nrequire_relative 'foo'\na = ['a',2]\n";
    let after = b"x = 42..\nvalue\nrequire_relative \"foo\"\na = [\"a\", 2]\n";
    unchanged(
        dir.path(),
        before,
        &["--only", &peers, "lib/foo.rb"],
        Some(1),
    );
    let out = run(dir.path(), &["--only", &peers, "--fix", "lib/foo.rb"]);
    assert_eq!(out.status.code(), Some(1));
    for cop in COPS {
        assert_eq!(count(&out, cop), 1);
    }
    assert_eq!(fs::read(dir.path().join("lib/foo.rb")).unwrap(), after);
    unchanged(
        dir.path(),
        after,
        &["--only", &peers, "--fix", "lib/foo.rb"],
        Some(1),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = dir.path().join("lib/foo.rb");
        let alias = dir.path().join("foo.rb");
        let canonical = b"if false\n require_relative 'foo'\nend\n";
        fs::write(&path, canonical).unwrap();
        symlink(&path, &alias).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", COPS[1], "--fix", "foo.rb"]);
        assert_eq!(out.status.code(), Some(2));
        assert_eq!(fs::read(&path).unwrap(), canonical);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", COPS[1], "--fix", "foo.rb"])
                .status
                .code(),
            Some(0)
        );
        assert!(fs::symlink_metadata(alias).unwrap().is_symlink());
        assert_eq!(fs::read(path).unwrap(), b"if false\nend\n");
    }
}
#[test]
fn native_syntax_only_all_original_inputs_never_execute_snippets() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        for source in std::iter::once(bytes).chain(
            row.get("after")
                .map(|v| serde_json::from_value::<Vec<u8>>(v.clone()).unwrap()),
        ) {
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-c", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                row.get("parser_rejection").is_none(),
                "{row}: {out:?}"
            );
        }
    }
}
#[test]
fn authored_bounded_native_warning_value_binding_dispatch_and_observation_controls() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let execute = |s: &str, syntax: bool| {
        let mut command = Command::new(&ruby);
        command.arg("--parser=prism").arg("-w");
        if syntax {
            command.arg("-c");
        }
        command
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    };
    // Authored controls, NOT reference/spec snippets. These paired controls
    // never invoke the loader; the final bounded self-loader uses only its own
    // separately authored scratch file.
    let before = "if false\n  require_relative 'native_control'\nend\n";
    let after = "if false\nend\n";
    for syntax in [false, true] {
        let a = execute(before, syntax);
        let b = execute(after, syntax);
        assert!(a.status.success() && b.status.success());
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
    }
    let wrap = |s: &str| {
        format!(
            "$events = []\ndef require_relative(*args); $events << args; raise 'must not dispatch'; end\nvalue = begin\n{s}end\np [value, local_variables.sort, $events, $LOADED_FEATURES.length]\n"
        )
    };
    let a = execute(&wrap(before), false);
    let b = execute(&wrap(after), false);
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
    // Live calls may be user-defined: deleting one changes effects and value.
    let a = execute(
        "def require_relative(x); puts x; false; end\np require_relative('authored')\n",
        false,
    );
    let b = execute(
        "def require_relative(x); puts x; false; end\np nil\n",
        false,
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    // Source line/code object observation distinguishes even inert deletion.
    let a = execute(&format!("{before}p __LINE__\n"), false);
    let b = execute(&format!("{after}p __LINE__\n"), false);
    assert_ne!(a.stdout, b.stdout);
    let probe = |s: &str| {
        format!(
            "RubyVM.keep_script_lines = true\np RubyVM::InstructionSequence.compile({s:?}).script_lines\n"
        )
    };
    let a = execute(&probe(before), false);
    let b = execute(&probe(after), false);
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    // A real authored self-load can recurse and changes require's Boolean
    // return and loaded features. No reference source or application file runs.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authored_self.rb");
    let source = "if $rustocop_authored_self_seen\n puts :nested\nelse\n $rustocop_authored_self_seen = true\n p require_relative 'authored_self'\nend\np $LOADED_FEATURES.grep(/authored_self.rb$/).length\n";
    fs::write(&path, source).unwrap();
    let a = Command::new(&ruby)
        .arg("--parser=prism")
        .arg(&path)
        .output()
        .unwrap();
    fs::write(
        &path,
        source.replace(" p require_relative 'authored_self'\n", ""),
    )
    .unwrap();
    let b = Command::new(&ruby)
        .arg("--parser=prism")
        .arg(&path)
        .output()
        .unwrap();
    assert!(a.status.success() && b.status.success(), "{a:?}: {b:?}");
    assert_eq!(String::from_utf8_lossy(&a.stdout), "nested\n0\ntrue\n1\n");
    assert_eq!(String::from_utf8_lossy(&b.stdout), "0\n");
}
