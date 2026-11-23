//! Source-only pinned replay and actual CLI/native controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/ParenthesesAsGroupedExpression";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/parentheses_as_grouped_expression.json"
    ))
    .unwrap();
    let rows = data["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2179);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        498
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("native_scope_rejection").is_some())
            .count(),
        41
    );
    rows.clone()
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
        assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
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
fn all_pinned_main_source_and_original_byte_controls_match_pipeline() {
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
        if row.get("parser_rejection").is_some() || row.get("native_scope_rejection").is_some() {
            assert!(!report.errors.is_empty(), "{row}: {report:?}");
            assert!(report.result.offenses.is_empty());
            continue;
        }
        if !report.errors.is_empty() {
            failures.push(format!(
                "parse {}: {:?}",
                String::from_utf8_lossy(&bytes),
                report.errors
            ));
            continue;
        }
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
fn canonical_bytes_atomic_idempotence_and_encoding() {
    let dir = project();
    for (before, after) in [
        (b"a.func (x)\n".as_slice(), b"a.func(x)\n".as_slice()),
        (b"is? \t (x)\n", b"is?(x)\n"),
        (b"a&.func (x)\n", b"a&.func(x)\n"),
        (
            b"x = 42; a.func (x); p __LINE__\n",
            b"x = 42; a.func(x); p __LINE__\n",
        ),
        (b"a.func (42) # keep\n", b"a.func(42) # keep\n"),
        (b"\xef\xbb\xbfa.func (x)\r\n", b"\xef\xbb\xbfa.func(x)\r\n"),
        (
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\na.func (x)\r\n",
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\na.func(x)\r\n",
        ),
        (
            b"# coding: Shift_JIS\n# \x95\x5c\na.func (x)\n",
            b"# coding: Shift_JIS\n# \x95\x5c\na.func(x)\n",
        ),
        (
            b"a.func (x)\n__END__\nraw\xff\n",
            b"a.func(x)\n__END__\nraw\xff\n",
        ),
    ] {
        unchanged(dir.path(), before, &["--only", COP, "lib/a.rb"], Some(1));
        let path = dir.path().join("lib/a.rb");
        let mut held = fs::File::open(&path).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), crate::support::lf::lf(after));
        let mut retained = Vec::new();
        held.read_to_end(&mut retained).unwrap();
        assert_eq!(retained, before);
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_ne!(old.ino(), fs::metadata(&path).unwrap().ino());
        }
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}
#[test]
fn defaults_selectors_strict_toml_and_supplied_paths() {
    let dir = project();
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    for (config, mut args, n) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        ("[cops.Lint]\nenabled=false\n".into(), vec![], 0),
        (
            format!("[cops.Lint]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
            vec![],
            1,
        ),
        (
            format!("[cops.Lint]\nenabled=true\n[cops.\"{COP}\"]\nenabled=false\n"),
            vec![],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled=false\n"),
            vec!["--only", COP],
            1,
        ),
        (
            format!("[cops.\"{COP}\"]\nexclude=['lib/a.rb']\n"),
            vec!["--only", COP],
            0,
        ),
        ("exclude=['lib/a.rb']\n".into(), vec!["--only", COP], 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        args.push("lib/a.rb");
        assert_eq!(
            count(&unchanged(dir.path(), b"a.func (x)\n", &args, None)),
            n
        );
    }
    for key in [
        "max=1",
        "include=['**/*']",
        "Enabled=true",
        "TargetRubyVersion=3.3",
        "SafeAutoCorrect=true",
        "enabled='true'",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            b"a.func (x)\n",
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for path in ["Gemfile", "lib/a.jb", "lib/.hidden.rb", "lib/a_spec.rb"] {
        fs::write(dir.path().join(path), b"a.func (x)\n").unwrap();
        let out = run(dir.path(), &["--only", COP, path]);
        assert_eq!(count(&out), 1);
    }
    assert_eq!(
        count(&run(&dir.path().join("lib"), &["--only", COP, "a.rb"])),
        1
    );
    for config in [
        String::new(),
        format!("[cops.\"{COP}\"]\nexclude=['lib/a.rb']\n"),
    ] {
        let n = usize::from(config.is_empty());
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(dir.path().join("lib/a.rb"), "sentinel\n").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"a.func (x)\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), n);
        if n != 0 {
            assert!(String::from_utf8_lossy(&out.stdout).starts_with("lib/a.rb:1:7:"));
        }
        assert_eq!(
            fs::read(dir.path().join("lib/a.rb")).unwrap(),
            b"sentinel\n"
        );
        assert_eq!(
            run(dir.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"])
                .status
                .code(),
            Some(2)
        );
    }
}
