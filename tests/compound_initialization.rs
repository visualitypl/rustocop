//! pinned source-only fixtures and real detection-only CLI controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
//! Upstream unsafe edits in the fixture are evidence, not project corrections.
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, select_active},
    pipeline::lint_one,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const CONSTANT: &str = "Lint/OrAssignmentToConstant";
const CONSTRUCTOR: &str = "Lint/DisjunctiveAssignmentInConstructor";
const BOTH: &str = "Lint/OrAssignmentToConstant,Lint/DisjunctiveAssignmentInConstructor";
const SOURCE: &[u8] = b"X ||= 1\ndef initialize\n @x ||= 1\nend\n";
#[test]
fn live_metadata_is_default_on_ruby34_ungated_and_detection_only() {
    let registry = Registry::new();
    assert_eq!(registry.cops().len(), 491);
    for name in [CONSTANT, CONSTRUCTOR] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled() && !cop.safe_fixes());
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
        assert!(cop.autocorrect_incompatible_with().is_empty());
    }
    assert!(registry.has_cop("Lint/UnusedPrivateMethod"));
}

#[test]
fn selectors_toml_precedence_and_strict_validation() {
    let dir = project();
    for cop in [CONSTANT, CONSTRUCTOR] {
        for (config, selectors, expected) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint"], 1),
            (String::new(), vec!["--only", BOTH, "--except", cop], 0),
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
            let mut args = selectors;
            args.push("lib/a.rb");
            let out = unchanged(dir.path(), SOURCE, &args, None);
            assert_eq!(count(&out, cop), expected, "{out:?}");
        }
        for key in [
            "max = 1",
            "include = ['**/*']",
            "Enabled = true",
            "enabled = 'true'",
            "target_ruby = 2.6",
            "SafeAutoCorrect = false",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                SOURCE,
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(2),
            );
        }
        for config in [
            "exclude = ['lib/**']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(
                dir.path(),
                SOURCE,
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
            );
            assert_eq!(count(&out, cop), 0);
        }
    }
}

#[test]
fn both_directive_prefixes_scope_only_the_operator_not_multiline_rhs() {
    let dir = project();
    for (cop, assignment, source) in [
        (CONSTANT, "X ||= 1", "X ||=\n  1\n"),
        (
            CONSTRUCTOR,
            "def initialize; @x ||= 1; end",
            "def initialize\n @x ||=\n  1\nend\n",
        ),
    ] {
        for prefix in ["rustocop", "rubocop"] {
            for body in [
                format!("# {prefix}:disable {cop}\n{assignment}\n"),
                format!("# {prefix}:push -{cop}\n{assignment}\n# {prefix}:pop\n"),
                format!("# {prefix}:disable-next {cop}\n{assignment}\n"),
                format!("# {prefix}:next -{cop}\n{assignment}\n"),
                format!("{assignment} # {prefix}:disable {cop}\n"),
            ] {
                let out = unchanged(
                    dir.path(),
                    body.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(0),
                );
                assert_eq!(count(&out, cop), 0);
            }
            let body = format!(
                "# {prefix}:disable {cop}\n{assignment}\n# {prefix}:push +{cop}\n{assignment}\n# {prefix}:pop\n{assignment}\n"
            );
            let out = unchanged(
                dir.path(),
                body.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(1),
            );
            assert_eq!(count(&out, cop), 1);
            let body = source.replace("  1", &format!("  1 # {prefix}:disable {cop}"));
            let out = unchanged(
                dir.path(),
                body.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(1),
            );
            assert_eq!(count(&out, cop), 1);
            let body = source.replace("||=", &format!("||= # {prefix}:disable {cop}"));
            let out = unchanged(
                dir.path(),
                body.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
            );
            assert_eq!(count(&out, cop), 0);
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            let body = format!("# {prefix}:enable {cop}\n{assignment}\n");
            let out = unchanged(dir.path(), body.as_bytes(), &["lib/a.rb"], None);
            assert_eq!(count(&out, cop), 1);
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        }
    }
}

#[test]
fn supplied_paths_stdin_provenance_and_builtin_exclusions() {
    let dir = project();
    for path in ["lib/a.arb", "lib/a.jb", "Rakefile", "lib/.hidden.rb"] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        let out = run(dir.path(), &["--only", BOTH, path]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!((count(&out, CONSTANT), count(&out, CONSTRUCTOR)), (1, 1));
    }
    for directory in ["vendor", "node_modules"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        fs::write(dir.path().join(directory).join("a.rb"), SOURCE).unwrap();
        let out = run(dir.path(), &["--only", BOTH, "--fix", directory]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(
            fs::read(dir.path().join(directory).join("a.rb")).unwrap(),
            SOURCE
        );
    }
    for (config, expected) in [(String::new(), 1), ("exclude = ['virtual/**']\n".into(), 0)] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", BOTH, "--stdin", "virtual/main.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(expected as i32), "{out:?}");
        assert_eq!(
            (count(&out, CONSTANT), count(&out, CONSTRUCTOR)),
            (expected, expected)
        );
        if expected == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/main.rb:1:3"));
        }
    }
    let out = unchanged(
        dir.path(),
        SOURCE,
        &["--only", BOTH, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("--stdin"));
}

#[test]
fn byte_envelopes_and_source_runtime_observers_never_enable_unsafe_fixes() {
    let dir = project();
    for prefix in [
        b"\xef\xbb\xbf".as_slice(),
        b"# coding: ISO-8859-1\n# \xe9\n",
        b"# coding: Shift_JIS\n# \x82\xa0\n",
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(
            &String::from_utf8_lossy(SOURCE)
                .replace('\n', "\r\n")
                .into_bytes(),
        );
        let out = unchanged(
            dir.path(),
            &source,
            &["--only", BOTH, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert_eq!((count(&out, CONSTANT), count(&out, CONSTRUCTOR)), (1, 1));
    }
    for tail in [
        "p __LINE__",
        "p DATA.pos\n__END__\npayload",
        "File.read(__FILE__)",
        "RubyVM::InstructionSequence.compile('nil')",
        "GC.stat",
        "TracePoint.new(:line) {}",
    ] {
        let source = format!("{}{tail}\n", String::from_utf8_lossy(SOURCE));
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", BOTH, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert_eq!((count(&out, CONSTANT), count(&out, CONSTRUCTOR)), (1, 1));
    }
    let source = b"X ||= <<~DOC\ntext\nDOC\ndef initialize\n @x ||= <<~DOC\ntext\nDOC\nend\n";
    let out = unchanged(
        dir.path(),
        source,
        &["--only", BOTH, "--fix", "lib/a.rb"],
        Some(1),
    );
    assert_eq!((count(&out, CONSTANT), count(&out, CONSTRUCTOR)), (1, 1));
}

#[cfg(unix)]
#[test]
fn symlink_readonly_invalid_and_atomic_composition_reuse_shared_controls() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, SOURCE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for name in ["lib/a.rb", "lib/link.rb"] {
        let out = run(dir.path(), &["--only", BOTH, "--fix", name]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), SOURCE);
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.ino(), new.ino());
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert!(
            fs::symlink_metadata(dir.path().join("lib/link.rb"))
                .unwrap()
                .is_symlink()
        );
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    unchanged(
        dir.path(),
        b"X ||= 1\n(\n",
        &["--only", BOTH, "--fix", "lib/a.rb"],
        Some(2),
    );
    fs::write(&path, b"X ||= 1\ndef initialize\n @x ||= 1\nend\np [1,2]\n").unwrap();
    let out = run(
        dir.path(),
        &[
            "--only",
            "Lint/OrAssignmentToConstant,Lint/DisjunctiveAssignmentInConstructor,Layout/SpaceAfterComma",
            "--fix",
            "lib/link.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!((count(&out, CONSTANT), count(&out, CONSTRUCTOR)), (1, 1));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"X ||= 1\ndef initialize\n @x ||= 1\nend\np [1, 2]\n"
    );
    assert_ne!(old.ino(), fs::metadata(&path).unwrap().ino());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
}

#[test]
fn native_fixture_syntax_only_and_authored_semantic_counterexamples() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        for key in ["source", "upstream_after"] {
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
                .write_all(row[key].as_str().unwrap().as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{row}\n{out:?}");
        }
    }
    // Only these authored scripts are executed, never reference/corpus sources.
    let dir = project();
    for source in [
        "X = :original\nX ||= :replacement\np X\n",
        "class Base\n def initialize\n  @x ||= :base\n end\n attr_reader :x\nend\nclass Derived < Base\n def initialize\n  @x = :derived\n  super\n end\nend\np Derived.new.x\n",
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", BOTH, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert_eq!(count(&out, CONSTANT) + count(&out, CONSTRUCTOR), 1);
        let execute = |s: &str| {
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-W0", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{out:?}");
            out.stdout
        };
        assert_ne!(execute(source), execute(&source.replace("||=", "=")));
    }
}

fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/compound_initialization.json")).unwrap()
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
        .lines()
        .filter(|l| l.ends_with(&format!("({cop})")))
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
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}
#[test]
fn every_pinned_spec_documentation_and_control_has_exact_pipeline_diagnostics() {
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    let dir = project();
    let rows = rows();
    assert_eq!(rows.len(), 522);
    for row in rows {
        let source = row["source"].as_str().unwrap();
        let cop = row["cop"].as_str().unwrap();
        let active = select_active(&registry, &config, &[cop.into()], &[]);
        let report = lint_one(
            &dir.path().join("lib/a.rb"),
            Path::new("lib/a.rb"),
            Some(source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}: {report:?}"
        );
        let actual: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                serde_json::json!([o.line, o.col, o.end_line, o.message])
            })
            .collect();
        assert_eq!(serde_json::json!(actual), row["findings"], "{row}");
        for fix in [false, true] {
            let mut args = vec!["--only", cop];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &args,
                Some(i32::from(!actual.is_empty())),
            );
            assert_eq!(count(&out, cop), actual.len(), "{row}\n{out:?}");
            for finding in &actual {
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains(&format!(
                        "lib/a.rb:{}:{}: E: {} ({cop})",
                        finding[0],
                        finding[1],
                        finding[3].as_str().unwrap()
                    )),
                    "{row}\n{out:?}"
                );
            }
        }
    }
}
