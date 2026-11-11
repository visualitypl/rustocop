//! Original-byte source-only references; no fixture execution.
//! RuboCop examples MIT, Copyright 2012-26 Bozhidar Batsov.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
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
        .filter(|line| line.contains(&format!("({cop})")))
        .count()
}
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
#[test]
fn original_byte_main_source_controls_match_pipeline_and_actual_cli() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/to_enum_json.json")).unwrap();
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let d = project();
    for row in rows {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let cop = row["cop"].as_str().unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let report = rustocop::pipeline::lint_one(
            &d.path().join("example.rb"),
            Path::new("example.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        if row.get("parser_rejection").is_some() {
            assert!(!report.errors.is_empty(), "{row}");
            continue;
        }
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}: {report:?}"
        );
        let mut found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> =
            serde_json::from_value(row["findings"].clone()).unwrap();
        assert_eq!(found, expected, "{row}");
        let path = d.path().join("example.rb");
        fs::write(&path, &bytes).unwrap();
        let before = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--only", cop, "--fix", "example.rb"]);
        let expected_bytes: Vec<u8> = row
            .get("project_after")
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .unwrap_or(bytes.clone());
        assert_eq!(fs::read(&path).unwrap(), expected_bytes, "{row}: {out:?}");
        if expected_bytes == bytes {
            assert_eq!(
                out.status.code(),
                Some(i32::from(!found.is_empty())),
                "{row}: {out:?}"
            );
            let after = fs::metadata(&path).unwrap();
            assert_eq!(before.modified().unwrap(), after.modified().unwrap());
            assert_eq!(before.permissions(), after.permissions());
        } else {
            assert_eq!(out.status.code(), Some(0), "{row}: {out:?}");
        }
    }
}
#[test]
fn fixed_defaults_selectors_toml_paths_and_directives() {
    let d = project();
    let path = d.path().join("example.rb");
    let source = "def to_json\nend\ndef m(x)\n to_enum(:m)\nend\n";
    fs::write(&path, source).unwrap();
    for args in [
        vec!["--only", "Lint/ToJSON", "example.rb"],
        vec!["--only", "Lint/ToEnumArguments", "example.rb"],
        vec!["--only", "Lint", "--except", "Lint/ToJSON", "example.rb"],
    ] {
        let out = run(d.path(), &args);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
    for (config, json, enums) in [
        ("", 1, 1),
        ("[cops.Lint]\nenabled=false\n", 0, 0),
        (
            "[cops.Lint]\nenabled=false\n[cops.\"Lint/ToJSON\"]\nenabled=true\n",
            1,
            0,
        ),
        (
            "[cops.Lint]\nenabled=true\n[cops.\"Lint/ToEnumArguments\"]\nenabled=false\n",
            1,
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let out = run(d.path(), &["example.rb"]);
        assert_eq!(count(&out, "Lint/ToJSON"), json, "{out:?}");
        assert_eq!(count(&out, "Lint/ToEnumArguments"), enums, "{out:?}");
        let out = run(d.path(), &["--only", "Lint/ToJSON", "example.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
    }
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Lint/ToJSON\"]\nexclude=[\"example.rb\"]\n",
    )
    .unwrap();
    assert_eq!(
        run(d.path(), &["--only", "Lint/ToJSON", "example.rb"])
            .status
            .code(),
        Some(0)
    );
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for s in [
            format!("# {prefix}:disable Lint/ToJSON\ndef to_json\nend\n"),
            format!("# {prefix}:disable-next Lint/ToJSON\ndef to_json\nend\n"),
            format!(
                "# {prefix}:push\n# {prefix}:disable Lint/ToJSON\ndef to_json\nend\n# {prefix}:pop\n"
            ),
        ] {
            fs::write(&path, &s).unwrap();
            let out = run(d.path(), &["--only", "Lint/ToJSON", "--fix", "example.rb"]);
            assert_eq!(out.status.code(), Some(0), "{s}: {out:?}");
            assert_eq!(fs::read_to_string(&path).unwrap(), s);
        }
    }
}
#[test]
fn stdin_provenance_no_fix_and_mandatory_parser_rejection() {
    use std::io::Write;
    use std::process::Stdio;
    let d = project();
    for cop in ["Lint/ToEnumArguments", "Lint/ToJSON"] {
        for path in ["lib/given.rb", "given.jb", "vendor/explicit.rb"] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(["--only", cop, "--stdin", path])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"def to_json\nend\ndef m(x)\n to_enum(:m)\nend\n")
                .unwrap();
            let out = child.wait_with_output().unwrap();
            // Selection never bypasses inherited built-in path exclusions.
            let expected = usize::from(!path.starts_with("vendor/"));
            assert_eq!(
                out.status.code(),
                Some(i32::from(expected != 0)),
                "{path}: {out:?}"
            );
            assert_eq!(count(&out, cop), expected);
            if expected != 0 {
                assert!(String::from_utf8_lossy(&out.stdout).contains(path));
            }
        }
    }
    // --stdin --fix is rejected before reading stdin; no racing writer needed.
    let out = run(
        d.path(),
        &["--only", "Lint/ToJSON", "--stdin", "virtual.rb", "--fix"],
    );
    assert_eq!(out.status.code(), Some(2));
    let path = d.path().join("example.rb");
    for s in [
        "if false\n def to_json\n end\nend\n",
        "def to_json\nend\ndef broken(\n",
    ] {
        fs::write(&path, s).unwrap();
        let meta = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--only", "Lint/ToJSON", "example.rb"]);
        assert_eq!(
            out.status.code(),
            Some(if s.contains("broken") { 2 } else { 1 }),
            "{out:?}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), s);
        assert_eq!(
            meta.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        if s.contains("broken") {
            let out = run(d.path(), &["--only", "Lint/ToJSON", "--fix", "example.rb"]);
            assert_eq!(out.status.code(), Some(2));
            assert_eq!(fs::read_to_string(&path).unwrap(), s);
        }
    }
}
#[test]
fn both_cops_multiline_suppression_and_push_restore_exact_findings() {
    let d = project();
    let path = d.path().join("example.rb");
    for prefix in ["rubocop", "rustocop"] {
        let s = format!(
            "# {prefix}:push -Lint/ToJSON -Lint/ToEnumArguments\ndef to_json\nend\ndef m(x)\n to_enum(\n  :m\n )\nend\n# {prefix}:pop\ndef to_json\nend\ndef m(x)\n to_enum(\n  :m\n )\nend\n"
        );
        fs::write(&path, &s).unwrap();
        let out = run(
            d.path(),
            &[
                "--only",
                "Lint/ToJSON,Lint/ToEnumArguments",
                "--fix",
                "example.rb",
            ],
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, "Lint/ToJSON"), 1, "{out:?}");
        assert_eq!(count(&out, "Lint/ToEnumArguments"), 1, "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), s);
    }
}
#[test]
fn strict_settings_and_fixed_metadata_remain_minimal() {
    let d = project();
    fs::write(d.path().join("example.rb"), "def to_json; end").unwrap();
    for cop in ["Lint/ToJSON", "Lint/ToEnumArguments"] {
        for option in [
            "max=1",
            "TargetRubyVersion=3.3",
            "EnforcedStyle=\"other\"",
            "enabled=\"pending\"",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{option}\n"),
            )
            .unwrap();
            assert_eq!(
                run(d.path(), &["--only", cop, "example.rb"]).status.code(),
                Some(2)
            );
        }
        let registry = rustocop::cops::Registry::new();
        let detector = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(detector.default_enabled());
        assert_eq!(detector.max_setting(), None);
        assert_eq!(detector.ruby_versions().minimum, None);
        assert_eq!(detector.ruby_versions().maximum, None);
        assert_eq!(detector.safe_fixes(), cop == "Lint/ToJSON");
    }
}
#[cfg(unix)]
#[test]
fn withheld_edits_preserve_held_inode_permissions_and_symlinks() {
    use std::{
        io::{Read, Seek},
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    let d = project();
    let path = d.path().join("example.rb");
    for s in ["def to_json\nend\n", "def m(x); to_enum(:m); end\n"] {
        fs::write(&path, s).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let before = fs::metadata(&path).unwrap();
        let mut held = fs::File::open(&path).unwrap();
        let out = run(
            d.path(),
            &[
                "--only",
                "Lint/ToJSON,Lint/ToEnumArguments",
                "--fix",
                "example.rb",
            ],
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let after = fs::metadata(&path).unwrap();
        assert_eq!(
            (before.ino(), before.mode(), before.modified().unwrap()),
            (after.ino(), after.mode(), after.modified().unwrap())
        );
        held.rewind().unwrap();
        let mut bytes = String::new();
        held.read_to_string(&mut bytes).unwrap();
        assert_eq!(bytes, s);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    let target = d.path().join("target.rb");
    let canonical = "if false\n def to_json\n end\nend\n";
    fs::write(&target, canonical).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o444)).unwrap();
    let before = fs::metadata(&target).unwrap();
    let out = run(d.path(), &["--only", "Lint/ToJSON", "--fix", "example.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), canonical);
    assert_eq!(
        before.modified().unwrap(),
        fs::metadata(&target).unwrap().modified().unwrap()
    );
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    let mut held = fs::File::open(&target).unwrap();
    let out = run(d.path(), &["--only", "Lint/ToJSON", "--fix", "example.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        canonical.replace("to_json", "to_json(*_args)")
    );
    assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o640);
    let mut original = String::new();
    held.read_to_string(&mut original).unwrap();
    assert_eq!(original, canonical);
}
#[test]
fn native_syntax_only_for_all_reference_valid_byte_rows() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = project();
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/to_enum_json.json")).unwrap();
    for row in rows
        .into_iter()
        .filter(|r| r.get("parser_rejection").is_none())
    {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        fs::write(d.path().join("example.rb"), bytes).unwrap();
        let out = Command::new("timeout")
            .args([
                "--kill-after=1s",
                "10s",
                "prlimit",
                "--as=536870912",
                "--fsize=65536",
                "--core=0",
                "--",
                &ruby,
                "--parser=prism",
                "-c",
                "example.rb",
            ])
            .current_dir(d.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{row}: {out:?}");
    }
}
#[test]
fn withheld_json_composes_with_real_style_and_layout_edits() {
    let d = project();
    let path = d.path().join("example.rb");
    let before = "def to_json\nend\na = ['a',2]\n";
    let after = "def to_json\nend\na = [\"a\", 2]\n";
    fs::write(&path, before).unwrap();
    let args = [
        "--only",
        "Lint/ToJSON,Style/StringLiterals,Layout/SpaceAfterComma",
        "--fix",
        "example.rb",
    ];
    let out = run(d.path(), &args);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out, "Lint/ToJSON"), 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    let meta = fs::metadata(&path).unwrap();
    let out = run(d.path(), &args);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    assert_eq!(
        meta.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
}
#[test]
fn canonical_composition_is_idempotent_and_observable_envelopes_are_held() {
    let d = project();
    let path = d.path().join("example.rb");
    let before = "if false\n  def to_json()\n  end  \nend\n";
    fs::write(&path, before).unwrap();
    let out = run(
        d.path(),
        &[
            "--only",
            "Lint/ToJSON,Layout/TrailingWhitespace",
            "--fix",
            "example.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let after = "if false\n  def to_json(*_args)\n  end\nend\n";
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    let meta = fs::metadata(&path).unwrap();
    assert_eq!(
        run(
            d.path(),
            &[
                "--only",
                "Lint/ToJSON,Layout/TrailingWhitespace",
                "--fix",
                "example.rb"
            ]
        )
        .status
        .code(),
        Some(0)
    );
    assert_eq!(
        meta.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    for suffix in [
        "p __LINE__\n",
        "p File.binread(__FILE__)\n",
        "p ObjectSpace.count_objects\n",
        "p GC.stat\n",
        "p RubyVM::InstructionSequence.compile_file(__FILE__).to_a\n",
        "p retained.source\n",
        "p retained.script_lines\n",
        "__END__\npayload\n",
    ] {
        let s = format!("if false\n def to_json\n end\nend\n{suffix}");
        fs::write(&path, &s).unwrap();
        let meta = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--only", "Lint/ToJSON", "--fix", "example.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), s);
        assert_eq!(
            meta.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
    }
}
