//! SOURCE ONLY. RuboCop examples: MIT, (c) 2012-26 Bozhidar Batsov.
#[macro_use]
#[path = "support/parallel_rows.rs"]
mod parallel_rows;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/UselessNumericOperation";
const SOURCE: &[u8] = b"x + 0\n";
const SAFE: &[u8] = b"if false\n  x + 0\nend\n";
const AFTER: &[u8] = b"if false\n  x\nend\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/useless_numeric_operation.json")).unwrap()
}
fn bytes(r: &serde_json::Value, key: &str) -> Vec<u8> {
    serde_json::from_value(r[key].clone()).unwrap()
}
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
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
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    }
    let mut held = fs::File::open(&path).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(n) = status {
        assert_eq!(out.status.code(), Some(n), "{out:?}\n{source:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source);
    let mut data = Vec::new();
    held.read_to_end(&mut data).unwrap();
    assert_eq!(data, source);
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
fn all_original_bytes_exact_pipeline_findings_ranges_messages_and_no_detection_edits() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let mut errors = Vec::new();
    for row in rows() {
        let source = bytes(&row, "bytes");
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        if row.get("parser_rejection").is_some() {
            assert!(!report.errors.is_empty(), "{row}");
            assert!(report.result.offenses.is_empty());
            continue;
        }
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}\n{report:?}"
        );
        let mut got: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        got.sort();
        let want: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
        if got != want {
            errors.push(format!(
                "{}: {got:?} != {want:?}",
                String::from_utf8_lossy(&source)
            ));
        }
    }
    assert!(
        errors.is_empty(),
        "{} mismatches\n{}",
        errors.len(),
        errors.join("\n")
    );
}
#[test]
fn all_original_bytes_actual_cli_lint_fix_no_write_or_real_canonical_idempotence() {
    parallel_rows!(for row in rows() => {
        let d = project();
        let path = d.path().join("lib/a.rb");
        let source = bytes(&row, "bytes");
        if row.get("parser_rejection").is_some() {
            for args in [
                vec!["--only", COP, "lib/a.rb"],
                vec!["--only", COP, "--fix", "lib/a.rb"],
            ] {
                unchanged(d.path(), &source, &args, Some(2));
            }
            continue;
        }
        let after = bytes(&row, "project_after");
        let want: Vec<Finding> = serde_json::from_value(row["findings"].clone()).unwrap();
        let status = i32::from(!want.is_empty());
        let out = unchanged(
            d.path(),
            &source,
            &["--only", COP, "lib/a.rb"],
            Some(status),
        );
        let mut got: Vec<_> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.ends_with(&format!("({COP})")))
            .map(str::to_owned)
            .collect();
        got.sort();
        let mut expected: Vec<_> = want
            .iter()
            .map(|(l, c, _, m)| format!("lib/a.rb:{l}:{c}: E: {m} ({COP})"))
            .collect();
        expected.sort();
        assert_eq!(got, expected, "{row}");
        if source == after {
            unchanged(
                d.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(status),
            );
        } else {
            let mut held = fs::File::open(&path).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = run(d.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert!(out.status.success(), "{row}\n{out:?}");
            assert_eq!(fs::read(&path).unwrap(), after, "{row}");
            assert_eq!(
                old.permissions(),
                fs::metadata(&path).unwrap().permissions()
            );
            let mut data = Vec::new();
            held.read_to_end(&mut data).unwrap();
            assert_eq!(data, source); // atomic replacement, old FD unchanged
            unchanged(
                d.path(),
                &after,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    });
}
#[test]
fn defaults_selectors_strict_toml_department_precedence_and_path_stdin_provenance() {
    let registry = rustocop::cops::Registry::new();
    let rule = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(rule.default_enabled() && rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(
        rule.path_policy().is_none()
            && rule.max_setting().is_none()
            && rule.autocorrect_incompatible_with().is_empty()
    );
    let d = project();
    for (config, select, want) in [
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
        ("exclude = ['lib/a.rb']\n".into(), vec!["--only", COP], 0),
        (
            format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
            vec!["--only", COP],
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = select.clone();
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(d.path(), SOURCE, &args, None)), want);
        args.pop();
        args.push("a.rb");
        assert_eq!(count(&run(&d.path().join("lib"), &args)), want);
        let mut args = select;
        args.extend(["--stdin", "lib/a.rb"]);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), want);
        if want > 0 {
            assert!(String::from_utf8_lossy(&out.stdout).starts_with("lib/a.rb:"));
        }
        assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), SOURCE);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "TargetRubyVersion = 3.3",
        "SafeAutoCorrect = true",
        "AllowedMethods = ['x']",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            SAFE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for path in ["Gemfile", "lib/.hidden.rb", "lib/a.jb", "lib/a_spec.rb"] {
        fs::write(d.path().join(path), SOURCE).unwrap();
        assert_eq!(count(&run(d.path(), &["--only", COP, path])), 1);
    }
    assert_eq!(
        run(d.path(), &["--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
}
#[test]
fn both_prefixes_multiline_next_push_and_selection_scopes() {
    let d = project();
    let source = String::from_utf8_lossy(SOURCE);
    for prefix in ["rubocop", "rustocop"] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        for directive in [
            format!("enable {COP}"),
            format!("enable-next {COP}"),
            format!("next +{COP}"),
            format!("push +{COP}"),
        ] {
            let text = format!("# {prefix}:{directive}\n{source}");
            assert_eq!(
                count(&unchanged(d.path(), text.as_bytes(), &["lib/a.rb"], None)),
                1
            );
            unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", "Lint/Debugger", "lib/a.rb"],
                Some(0),
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for text in [
            format!("# {prefix}:disable Lint\n{source}"),
            format!("# {prefix}:push -{COP}\n{source}# {prefix}:pop\n"),
            format!("# {prefix}:disable-next {COP}\n{source}"),
            format!("x +\n 0 # {prefix}:disable {COP}\n"),
            format!(
                "# {prefix}:disable {COP}\n{}",
                String::from_utf8_lossy(SAFE)
            ),
        ] {
            unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        let text = format!(
            "# {prefix}:disable {COP}\n{source}# {prefix}:enable {COP}\n{source}# {prefix}:push -{COP}\n{source}# {prefix}:pop\n{source}"
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "lib/a.rb"],
                Some(1)
            )),
            2
        );
    }
}
#[test]
fn source_runtime_comment_binding_and_delayed_controls_plus_style_layout_composition() {
    let d = project();
    for suffix in [
        "p __LINE__",
        "File.binread(__FILE__)",
        "ObjectSpace.each_object {}",
        "RubyVM::InstructionSequence.compile('1')",
        "DATA.read\n__END__\ndata\n",
        "value = <<~DOC\ntext\nDOC\n",
        "TracePoint.new(:line) {}.enable",
        "GC.stat",
        "binding",
    ] {
        let text = format!("{}{suffix}\n", String::from_utf8_lossy(SAFE));
        unchanged(
            d.path(),
            text.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    for wrapper in [
        "def deferred\n%s\nend\n",
        "-> {\n%s\n}\n",
        "if true\n%s\nend\n",
    ] {
        let text = wrapper.replace("%s", &String::from_utf8_lossy(SAFE));
        unchanged(
            d.path(),
            text.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    for source in [
        b"if false\n x += 0\nend\n".as_slice(),
        b"x = 1\nx + 0\n",
        b"if false\n x + # keep\n 0\nend\n",
        b"if false\n x + 0\nend\ndef broken(\n",
    ] {
        unchanged(
            d.path(),
            source,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(if source.ends_with(b"broken(\n") { 2 } else { 1 }),
        );
    }
    let mixed = format!("{}x = ['a',2]\n", String::from_utf8_lossy(SAFE));
    fs::write(d.path().join("lib/a.rb"), &mixed).unwrap();
    let args = [
        "--only",
        "Lint/UselessNumericOperation,Layout/SpaceAfterComma,Style/StringLiterals",
        "--fix",
        "lib/a.rb",
    ];
    let out = run(d.path(), &args);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out), 1);
    let after = mixed.replace("'a',2", "\"a\", 2");
    assert_eq!(
        fs::read(d.path().join("lib/a.rb")).unwrap(),
        after.as_bytes()
    );
    unchanged(d.path(), after.as_bytes(), &args, Some(1));
}
#[cfg(unix)]
#[test]
fn atomic_permissions_symlink_and_failure_controls() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&path, d.path().join("link.rb")).unwrap();
    let out = run(d.path(), &["--only", COP, "--fix", "link.rb"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), AFTER);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(d.path().join("link.rb"))
            .unwrap()
            .is_symlink()
    );
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(d.path(), &["--only", COP, "--fix", "link.rb"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    assert_eq!(
        old.permissions(),
        fs::metadata(&path).unwrap().permissions()
    );
}
#[test]
fn native_ruby34_syntax_only_for_every_reference_input_and_canonical_output() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        let source = bytes(&row, "bytes");
        let mut inputs = vec![source.clone()];
        if row.get("parser_rejection").is_none() && source != bytes(&row, "project_after") {
            inputs.push(bytes(&row, "project_after"));
        }
        for source in inputs {
            let mut child = Command::new(&ruby)
                .args(["-c", "-W0"])
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
                "{row}\n{out:?}"
            );
        }
    }
}
