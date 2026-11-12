//! Source-only pinned fixtures, real CLI, no-edit applicability.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const FLIP: &str = "Lint/FlipFlop";
const EACH: &str = "Lint/EachWithObjectArgument";
const BOTH: &str = "Lint/FlipFlop,Lint/EachWithObjectArgument";
const SOURCE: &[u8] = b"obj.each_with_object(0)\nbody if a..b\n";

fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<_> =
        serde_json::from_str(include_str!("fixtures/flip_flop_accumulator.json")).unwrap();
    assert_eq!(rows.len(), 1385);
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
fn counts(out: &Output) -> (usize, usize) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let count = |cop| {
        stdout
            .lines()
            .filter(|l| l.ends_with(&format!("({cop})")))
            .count()
    };
    (count(FLIP), count(EACH))
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}\n{source:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
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
type Finding = (u32, u32, u32, String);
fn expected(row: &serde_json::Value, cop: &str) -> Vec<Finding> {
    serde_json::from_value(row["findings"][cop].clone()).unwrap()
}

#[test]
fn all_pinned_specs_documentation_syntactic_and_original_byte_controls_match_pipeline() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active =
        rustocop::cops::select_active(&registry, &config, &[FLIP.into(), EACH.into()], &[]);
    let mut failures = Vec::new();
    for row in rows() {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{report:?}: {row}"
        );
        for cop in [FLIP, EACH] {
            let mut found: Vec<_> = report
                .result
                .offenses
                .iter()
                .filter(|o| o.cop == cop)
                .map(|o| {
                    assert!(o.correction.is_none());
                    assert_eq!(o.severity, rustocop::offense::Severity::Error);
                    (o.line, o.col, o.end_line, o.message.clone())
                })
                .collect();
            found.sort();
            let expected = expected(&row, cop);
            if found != expected {
                failures.push(format!(
                    "{}: {cop}: expected {expected:?}, found {found:?}",
                    String::from_utf8_lossy(&source)
                ));
            }
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
fn every_fixture_matches_exact_rebuilt_cli_and_never_writes_even_when_fix_requested() {
    let dir = project();
    for row in rows() {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let has_findings = [FLIP, EACH].iter().any(|c| !expected(&row, c).is_empty());
        let out = unchanged(
            dir.path(),
            &source,
            &["--only", BOTH, "--fix", "lib/a.rb"],
            i32::from(has_findings),
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        for cop in [FLIP, EACH] {
            let mut expected: Vec<_> = expected(&row, cop)
                .into_iter()
                .map(|(line, col, _, msg)| format!("lib/a.rb:{line}:{col}: E: {msg} ({cop})"))
                .collect();
            let mut found: Vec<_> = stdout
                .lines()
                .filter(|l| l.ends_with(&format!("({cop})")))
                .map(str::to_owned)
                .collect();
            found.sort();
            expected.sort();
            assert_eq!(found, expected, "{row}\n{out:?}");
        }
    }
}

#[test]
fn detection_only_metadata_defaults_selectors_and_strict_toml_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for cop in registry
        .cops()
        .iter()
        .filter(|c| [FLIP, EACH].contains(&c.name()))
    {
        assert!(cop.default_enabled());
        assert!(!cop.safe_fixes());
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
        assert!(cop.max_setting().is_none());
    }
    let dir = project();
    for (config, selection, expected) in [
        (String::new(), vec![], (1, 1)),
        (String::new(), vec!["--only", BOTH], (1, 1)),
        (
            String::new(),
            vec!["--only", "Lint", "--except", FLIP],
            (0, 1),
        ),
        (
            String::new(),
            vec!["--only", BOTH, "--except", BOTH],
            (0, 0),
        ),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], (0, 0)),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{FLIP}\"]\nenabled = true\n"),
            vec![],
            (1, 0),
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{EACH}\"]\nenabled = false\n"),
            vec![],
            (1, 0),
        ),
        (
            format!("[cops.\"{FLIP}\"]\nenabled = false\n"),
            vec!["--only", FLIP],
            (1, 0),
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selection;
        args.push("lib/a.rb");
        // No --fix: ordinary lint is also read-only, including when peers find offenses.
        fs::write(dir.path().join("lib/a.rb"), SOURCE).unwrap();
        let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
        let out = run(dir.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(counts(&out), expected, "{out:?}");
        assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), SOURCE);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(dir.path().join("lib/a.rb"))
                .unwrap()
                .modified()
                .unwrap()
        );
    }
    for cop in [FLIP, EACH] {
        for key in [
            "max = 1",
            "include = ['**/*']",
            "enabled = 'true'",
            "TargetRubyVersion = 3.3",
            "Style = 'other'",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                SOURCE,
                &["--only", BOTH, "--fix", "lib/a.rb"],
                2,
            );
        }
    }
}

#[test]
fn supplied_path_exclusions_stdin_and_both_directive_prefixes_keep_provenance() {
    let dir = project();
    for (config, expected) in [
        (String::new(), (1, 1)),
        ("exclude = ['lib/a.rb']\n".into(), (0, 0)),
        (
            format!("[cops.\"{FLIP}\"]\nexclude = ['lib/a.rb']\n"),
            (0, 1),
        ),
        (
            format!(
                "[cops.\"{FLIP}\"]\nexclude = ['lib/a.rb']\n[cops.\"{EACH}\"]\nexclude = ['lib/a.rb']\n"
            ),
            (0, 0),
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let out = unchanged(
            dir.path(),
            SOURCE,
            &["--only", BOTH, "--fix", "lib/a.rb"],
            i32::from(expected != (0, 0)),
        );
        assert_eq!(counts(&out), expected);
        let out = run(&dir.path().join("lib"), &["--only", BOTH, "a.rb"]);
        assert_eq!(counts(&out), expected);
        fs::write(dir.path().join("lib/a.rb"), b"sentinel\n").unwrap();
        let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", BOTH, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(counts(&out), expected);
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
        run(dir.path(), &["--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for path in ["lib/a.jb", "Gemfile", "lib/.hidden.rb", "lib/a_spec.rb"] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        assert_eq!(counts(&run(dir.path(), &["--only", BOTH, path])), (1, 1));
    }
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Lint\nobj.each_with_object(0)\nbody if a..b\n"),
            format!(
                "# {prefix}:disable-next {EACH}\nobj.each_with_object(\n0\n)\n# {prefix}:disable-next {FLIP}\nif (a..\nb); end\n"
            ),
            format!(
                "obj.each_with_object(\n0\n) # {prefix}:disable {EACH}\nif (a..\nb); end # {prefix}:disable {FLIP}\n"
            ),
            format!(
                "# {prefix}:push -{FLIP} -{EACH}\nobj.each_with_object(0)\nbody if a..b\n# {prefix}:pop\n"
            ),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", BOTH, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "# {prefix}:disable Lint\nobj.each_with_object(0)\nbody if a..b\n# {prefix}:push +{FLIP} +{EACH}\nobj.each_with_object(0)\nbody if a..b\n# {prefix}:pop\nobj.each_with_object(0)\nbody if a..b\n"
        );
        assert_eq!(
            counts(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", BOTH, "--fix", "lib/a.rb"],
                1
            )),
            (1, 1)
        );
    }
}

#[test]
fn source_observers_encoding_data_heredocs_comments_and_atomic_peers_keep_findings() {
    let dir = project();
    for source in [
        b"\xef\xbb\xbf# encoding: utf-8\r\nobj.each_with_object(0)\r\nbody if a..b\r\np __LINE__\r\n__END__\r\nraw\x00\r\n".as_slice(),
        b"# encoding: Windows-1252\np '\xe9'; obj.each_with_object(0)\nbody if a..b\n",
        b"x = <<~TEXT\nobj.each_with_object(0)\nbody if a..b\nTEXT\nobj.each_with_object(0)\nbody if a..b\n",
        b"obj.each_with_object(0); body if a..b; File.binread(__FILE__); DATA.pos\n__END__\nraw\n",
        b"obj.each_with_object(0); body if a..b; RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
    ] { assert_eq!(counts(&unchanged(dir.path(), source, &["--only", BOTH, "--fix", "lib/a.rb"], 1)), (1, 1)); }
    let invalid = b"obj.each_with_object(0); body if a..b\ndef broken(\n";
    let out = unchanged(
        dir.path(),
        invalid,
        &["--only", BOTH, "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!(counts(&out), (0, 0));
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, b"obj.each_with_object(0)\nbody if a..b\nx = [1,2]\n").unwrap();
    let out = run(
        dir.path(),
        &[
            "--only",
            "Lint/FlipFlop,Lint/EachWithObjectArgument,Layout/SpaceAfterComma",
            "--fix",
            "lib/a.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(counts(&out), (1, 1));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"obj.each_with_object(0)\nbody if a..b\nx = [1, 2]\n"
    );
    let old = fs::metadata(&path).unwrap();
    let out = run(dir.path(), &["--only", BOTH, "--fix", "lib/a.rb"]);
    assert_eq!(counts(&out), (1, 1));
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let out = run(dir.path(), &["--only", BOTH, "--fix", "lib/a.rb"]);
        assert_eq!(counts(&out), (1, 1));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o444
        );
        symlink(&path, dir.path().join("link.rb")).unwrap();
        let out = run(dir.path(), &["--only", BOTH, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(counts(&out), (1, 1));
        assert!(
            fs::symlink_metadata(dir.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        // No edit attempt above; an actual peer edit now reaches the shared
        // atomic writer and must fail without dropping either lint finding.
        let before = b"obj.each_with_object(0)\nbody if a..b\nx = [1,2]\n";
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&path, before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(
            dir.path(),
            &[
                "--only",
                "Lint/FlipFlop,Lint/EachWithObjectArgument,Layout/SpaceAfterComma",
                "--fix",
                "link.rb",
            ],
        );
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(counts(&out), (1, 1));
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
fn native_ruby34_parses_every_reference_input_without_execution() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("ruby 3.4."));
    for row in rows() {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let mut child = Command::new(&ruby)
            .args(["-c", "-W0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&source).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
    }
}
