//! Source-only pinned RescueType checks. Reference inputs never execute.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/RescueType";
const BEFORE: &str =
    "begin\n  raise StandardError\nrescue StandardError, nil\n  puts :caught\nend\n";
const ORDINARY: &[u8] = b"begin\n  foo\nrescue nil\n  bar\nend\n";
#[test]
fn defaults_selectors_strict_config_paths_and_supplied_provenance() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert_eq!(
        cop.ruby_versions(),
        rustocop::cops::policy::RubyVersions {
            minimum: None,
            maximum: None
        }
    );
    let dir = project();
    for (config, mut args, n) in [
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
        (
            format!("[cops.\"{COP}\"]\nexclude = ['lib/foo.rb']\n"),
            vec!["--only", COP],
            0,
        ),
        ("exclude = ['lib/foo.rb']\n".into(), vec!["--only", COP], 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        args.push("lib/foo.rb");
        assert_eq!(count(&unchanged(dir.path(), ORDINARY, &args, None)), n);
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
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            BEFORE.as_bytes(),
            &["--only", COP, "--fix", "lib/foo.rb"],
            Some(2),
        );
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for path in ["vendor/foo.rb", "db/schema.rb", "node_modules/foo.rb"] {
        let file = dir.path().join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, BEFORE).unwrap();
        assert_eq!(count(&run(dir.path(), &["--only", COP, "--fix", path])), 0);
        assert_eq!(fs::read(&file).unwrap(), BEFORE.as_bytes());
        assert_eq!(count(&stdin(dir.path(), path, BEFORE.as_bytes())), 0);
    }
    for path in ["foo.rb", "lib/foo.rb", "foo.rake", "Rakefile"] {
        let file = dir.path().join(path);
        fs::write(&file, b"sentinel\n").unwrap();
        let old = fs::metadata(&file).unwrap();
        assert_eq!(count(&stdin(dir.path(), path, ORDINARY)), 1);
        assert_eq!(fs::read(&file).unwrap(), b"sentinel\n");
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&file).unwrap().modified().unwrap()
        );
    }
    unchanged(
        dir.path(),
        BEFORE.as_bytes(),
        &["--only", COP, "--stdin", "lib/foo.rb", "--fix"],
        Some(2),
    );
    assert_eq!(
        String::from_utf8_lossy(&run(dir.path(), &["--version"]).stdout).trim(),
        "rustocop 1.1.0"
    );
}
#[test]
fn directives_both_prefixes_multiline_next_push_and_activation() {
    let dir = project();
    let ordinary = std::str::from_utf8(ORDINARY).unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for text in [
            format!("# {prefix}:disable Lint\n{BEFORE}"),
            format!("# {prefix}:disable-next {COP}\n{BEFORE}"),
            format!("# {prefix}:next -{COP}\n{BEFORE}"),
            format!("# {prefix}:push -{COP}\n{BEFORE}# {prefix}:pop\n"),
            BEFORE.replace(", nil", &format!(", nil # {prefix}:disable {COP}")),
        ] {
            unchanged(
                dir.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/foo.rb"],
                Some(0),
            );
        }
        let text = format!(
            "# {prefix}:disable {COP}\n{ordinary}# {prefix}:push +{COP}\n{ordinary}# {prefix}:pop\n{ordinary}"
        );
        assert_eq!(
            count(&unchanged(
                dir.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/foo.rb"],
                Some(1)
            )),
            1
        );
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let text = format!("# {prefix}:enable {COP}\n{ordinary}");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                text.as_bytes(),
                &["lib/foo.rb"],
                None
            )),
            1
        );
        assert_eq!(
            count(&unchanged(
                dir.path(),
                text.as_bytes(),
                &["--except", COP, "lib/foo.rb"],
                None
            )),
            0
        );
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        let multi = "begin\nfoo\nrescue nil,\n false\nbar\nend\n";
        for text in [
            format!("# {prefix}:disable-next {COP}\n{multi}"),
            multi.replace(" false", &format!(" false # {prefix}:disable {COP}")),
        ] {
            unchanged(
                dir.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/foo.rb"],
                Some(0),
            );
        }
    }
}
#[test]
fn canonical_edits_atomic_permissions_comments_and_symlink_failures() {
    let dir = project();
    for row in rows().into_iter().filter(|r| r.get("after").is_some()) {
        let before: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let after: Vec<u8> = serde_json::from_value(row["after"].clone()).unwrap();
        unchanged(dir.path(), &before, &["--only", COP, "lib/foo.rb"], Some(1));
        let path = dir.path().join("lib/foo.rb");
        let old = fs::metadata(&path).unwrap();
        let mut held = fs::File::open(&path).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", "lib/foo.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 correction applied"));
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        let mut original = Vec::new();
        held.read_to_end(&mut original).unwrap();
        assert_eq!(original, before);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = dir.path().join("lib/foo.rb");
        let alias = dir.path().join("alias.rb");
        fs::write(&path, BEFORE).unwrap();
        symlink(&path, &alias).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", "alias.rb"])
                .status
                .code(),
            Some(2)
        );
        assert_eq!(fs::read(&path).unwrap(), BEFORE.as_bytes());
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
            run(
                dir.path(),
                &["--only", COP, "--fix", "alias.rb", "lib/foo.rb"]
            )
            .status
            .code(),
            Some(0)
        );
        assert!(fs::symlink_metadata(alias).unwrap().is_symlink());
        assert_eq!(
            fs::read(path).unwrap(),
            BEFORE.replace(", nil", "").as_bytes()
        );
    }
}
#[test]
fn uncertain_dispatch_bindings_runtime_observers_and_composition() {
    let dir = project();
    for source in [
        BEFORE.replace("raise StandardError", "foo"),
        BEFORE.replace("raise StandardError", "raise Exception"),
        BEFORE.replace("StandardError, nil", "nil, StandardError"),
        BEFORE.replace("StandardError, nil", "StandardError, effect(), nil"),
        BEFORE.replace("puts :caught", "effect()"),
        BEFORE.replace("puts :caught", "puts :caught\nensure\n effect()"),
        BEFORE.replace("StandardError, nil", "StandardError, nil => e"),
        BEFORE.replace("raise StandardError", "Kernel.raise StandardError"),
        BEFORE.replace("StandardError, nil", "StandardError, # retained\n nil"),
        BEFORE.replace("StandardError, nil", "StandardError,\n nil"),
        format!("def raise(*); end\n{BEFORE}"),
        format!("StandardError = Exception\n{BEFORE}"),
        format!("class Module; def ===(*); false; end; end\n{BEFORE}"),
        format!("module M; refine Class do; def ===(*); false; end; end; end\nusing M\n{BEFORE}"),
        format!("{BEFORE}p binding, local_variables\n"),
        format!("{BEFORE}File.binread(__FILE__)\n"),
        format!("{BEFORE}RubyVM::InstructionSequence.compile_file(__FILE__)\n"),
        format!("{BEFORE}ObjectSpace.each_object(String).count\n"),
        format!("TracePoint.new(:raise, :line) {{}}.enable\n{BEFORE}"),
        format!("{BEFORE}DATA.read\n__END__\npayload\n"),
        format!("{BEFORE}p __LINE__, __ENCODING__\n"),
        format!("consume(<<TEXT)\n{BEFORE}TEXT\n{BEFORE}"),
        format!("def f\n{BEFORE}end\n"),
    ] {
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/foo.rb"],
                Some(1)
            )),
            1,
            "{source}"
        );
    }
    let invalid = format!("{BEFORE}def broken(\n");
    unchanged(
        dir.path(),
        invalid.as_bytes(),
        &["--only", COP, "--fix", "lib/foo.rb"],
        Some(2),
    );
    let peers = format!("{COP},Style/StringLiterals,Layout/SpaceAfterComma");
    let source = format!("{}a = ['a',2]\n", std::str::from_utf8(ORDINARY).unwrap());
    let expected = source.replace("'a',2", "\"a\", 2");
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", &peers, "lib/foo.rb"],
        Some(1),
    );
    assert_eq!(
        run(dir.path(), &["--only", &peers, "--fix", "lib/foo.rb"])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        fs::read(dir.path().join("lib/foo.rb")).unwrap(),
        expected.as_bytes()
    );
    unchanged(
        dir.path(),
        expected.as_bytes(),
        &["--only", &peers, "--fix", "lib/foo.rb"],
        Some(1),
    );
    // A disjoint layout edit composes with the actual live rescue correction.
    let source = BEFORE.replace("  puts", "\tputs");
    let peers = format!("{COP},Layout/IndentationStyle");
    fs::write(dir.path().join("lib/foo.rb"), source).unwrap();
    let out = run(dir.path(), &["--only", &peers, "--fix", "lib/foo.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read(dir.path().join("lib/foo.rb")).unwrap(),
        BEFORE.replace(", nil", "").as_bytes()
    );
}
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/rescue_type.json")).unwrap();
    assert_eq!(rows.len(), 1463);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        28
    );
    rows
}
fn native(ruby: &str, source: &[u8], syntax: bool) -> Output {
    let mut command = Command::new(ruby);
    command.args(["--parser=prism", "-w"]);
    if syntax {
        command.arg("-c");
    }
    let mut child = command
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn binary_diagnostics_are_lossless_without_edits() {
    let dir = project();
    let source = b"# coding: ASCII-8BIT\nbegin; foo; rescue '\xE9'; bar; end\n";
    let out = unchanged(
        dir.path(),
        source,
        &["--only", COP, "--fix", "lib/foo.rb"],
        Some(1),
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Rescuing from `'\\xE9'`"), "{text}");
    assert!(!text.contains('\u{fffd}'));
}
#[test]
fn native_syntax_only_all_reference_bytes_and_canonical_outputs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        for source in std::iter::once(bytes).chain(
            row.get("after")
                .map(|v| serde_json::from_value::<Vec<u8>>(v.clone()).unwrap()),
        ) {
            let out = native(&ruby, &source, true);
            assert_eq!(
                out.status.success(),
                row.get("parser_rejection").is_none(),
                "{row}: {out:?}"
            );
        }
    }
}
#[test]
fn authored_native_live_handler_warnings_events_values_binding_and_exception_controls() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    // These bounded programs are authored runtime controls, not executed
    // reference/corpus inputs. Standard dispatch actually raises and catches.
    let before = "begin\n raise StandardError\nrescue StandardError, nil\n puts :caught\nend\n";
    let after = "begin\n raise StandardError\nrescue StandardError\n puts :caught\nend\n";
    for syntax in [false, true] {
        let a = native(&ruby, before.as_bytes(), syntax);
        let b = native(&ruby, after.as_bytes(), syntax);
        assert!(a.status.success() && b.status.success(), "{a:?}: {b:?}");
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
        if !syntax {
            assert_eq!(a.stdout, b"caught\n");
        }
    }
    // The handler is live. Capture events, exception identity/class/message,
    // value and the lexical local table without changing any source line.
    let wrap = |s: &str| {
        format!(
            "events = []\ntrace = TracePoint.new(:line, :raise, :c_call, :c_return) {{ |t| events << [t.event, t.method_id, t.lineno, t.event == :raise ? [t.raised_exception.class, t.raised_exception.message, t.raised_exception.backtrace] : nil] if t.path == '-'  }}\ntrace.enable\nvalue = {s}trace.disable\np [value, local_variables.sort, events, $!]\n"
        )
    };
    let a = native(&ruby, wrap(before).as_bytes(), false);
    let b = native(&ruby, wrap(after).as_bytes(), false);
    assert!(a.status.success() && b.status.success(), "{a:?}: {b:?}");
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
    assert!(String::from_utf8_lossy(&a.stdout).contains(":raise"));
    assert!(String::from_utf8_lossy(&a.stdout).contains(":==="), "{a:?}");
    // Authored binding/ensure/value/exception controls retain evaluation count.
    let wrap = |s: &str| {
        format!(
            "events = []\nvalue = begin\n{s}ensure\n events << :ensured\nend\np [value, local_variables.sort, events, $!]\n"
        )
    };
    let a = native(&ruby, wrap(before).as_bytes(), false);
    let b = native(&ruby, wrap(after).as_bytes(), false);
    assert!(a.status.success() && b.status.success(), "{a:?}: {b:?}");
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
    // Arbitrary exception selection is not a safe repair: a trailing nil can
    // raise TypeError instead of propagating an unmatched Exception.
    let wrap =
        |s: &str| format!("begin\n{s}rescue Exception => failure\n puts failure.class\nend\n");
    let a = native(
        &ruby,
        wrap("begin\n raise Exception\nrescue StandardError, nil\n puts :caught\nend\n").as_bytes(),
        false,
    );
    let b = native(
        &ruby,
        wrap("begin\n raise Exception\nrescue StandardError\n puts :caught\nend\n").as_bytes(),
        false,
    );
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b"TypeError\n");
    assert_eq!(b.stdout, b"Exception\n");
    // A matching hook may change matching dispatch even for StandardError.
    let prefix = "def StandardError.===(other); false; end\n";
    let a = native(&ruby, format!("{prefix}{}", wrap(before)).as_bytes(), false);
    let b = native(&ruby, format!("{prefix}{}", wrap(after)).as_bytes(), false);
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b"TypeError\n");
    assert_eq!(b.stdout, b"StandardError\n");
    // Code/source observation distinguishes the edit and is not waived.
    let probe = |s: &str| {
        format!(
            "RubyVM.keep_script_lines = true\np RubyVM::InstructionSequence.compile({s:?}).script_lines\n"
        )
    };
    let a = native(&ruby, probe(before).as_bytes(), false);
    let b = native(&ruby, probe(after).as_bytes(), false);
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
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
        .matches(&format!("({COP})"))
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
    if let Some(s) = status {
        assert_eq!(out.status.code(), Some(s), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let mut buf = Vec::new();
    held.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, bytes);
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
fn stdin(root: &Path, path: &str, source: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(["--only", COP, "--stdin", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn all_original_byte_reference_rows_match_pipeline_and_canonical_edits() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let mut failures = Vec::new();
    for row in rows() {
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
            assert!(!report.errors.is_empty(), "{row}");
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
fn rebuilt_cli_every_reference_row_stdin_file_fix_and_no_write_controls() {
    parallel_rows!(for row in rows() => {
        let dir = project();
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let n = row["findings"].as_array().map_or(0, Vec::len);
        let status = if row.get("parser_rejection").is_some() {
            2
        } else {
            i32::from(n > 0)
        };
        let out = stdin(dir.path(), "example.rb", &bytes);
        assert_eq!(out.status.code(), Some(status), "{row}: {out:?}");
        assert_eq!(count(&out), n, "{row}: {out:?}");
        unchanged(
            dir.path(),
            &bytes,
            &["--only", COP, "lib/foo.rb"],
            Some(status),
        );
        if let Some(after) = row.get("after") {
            let expected: Vec<u8> = serde_json::from_value(after.clone()).unwrap();
            let out = run(dir.path(), &["--only", COP, "--fix", "lib/foo.rb"]);
            assert_eq!(out.status.code(), Some(0), "{row}: {out:?}");
            assert_eq!(
                fs::read(dir.path().join("lib/foo.rb")).unwrap(),
                expected,
                "{row}"
            );
            unchanged(
                dir.path(),
                &expected,
                &["--only", COP, "--fix", "lib/foo.rb"],
                Some(0),
            );
        } else {
            let out = unchanged(
                dir.path(),
                &bytes,
                &["--only", COP, "--fix", "lib/foo.rb"],
                Some(status),
            );
            assert_eq!(count(&out), n, "{row}: {out:?}");
        }
    });
}
