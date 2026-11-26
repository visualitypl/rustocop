//! Original-byte source-only references; no fixture execution.
//! RuboCop examples MIT, Copyright 2012-26 Bozhidar Batsov.
use crate::support::cli::{project, run};
use std::{
    fs,
    process::{Command, Output},
};
fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.contains(&format!("({cop})")))
        .count()
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
#[test]
fn native_syntax_only_for_all_reference_valid_byte_rows() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let d = project();
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/to_enum_json.json")).unwrap();
    for row in rows
        .into_iter()
        .filter(|r| r.get("parser_rejection").is_none())
    {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
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
