//! Source-only RuboCop MIT examples. Never execute fixture inputs.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/TopLevelReturnWithArgument";
const SAFE: &[u8] = b"return 1, 2, 3\n";
const EFFECT: &[u8] = b"return effect()\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/top_level_return_with_argument.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    serde_json::from_value(row[key].clone()).unwrap()
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
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let mut held = fs::File::open(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}\n{source:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    let mut old_bytes = Vec::new();
    held.read_to_end(&mut old_bytes).unwrap();
    assert_eq!(old_bytes, source);
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
fn all_original_bytes_match_pipeline_cli_and_withhold_warning_changing_edits() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let d = project();
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
            assert!(
                !report.errors.is_empty() && report.result.offenses.is_empty(),
                "{row} {report:?}"
            );
            unchanged(d.path(), &source, &["--only", COP, "lib/a.rb"], 2);
            unchanged(d.path(), &source, &["--only", COP, "--fix", "lib/a.rb"], 2);
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
        let expected: Vec<(u32, u32, u32, String)> =
            serde_json::from_value(row["findings"].clone()).unwrap();
        assert_eq!(got, expected, "{row}");
        let status = i32::from(!expected.is_empty());
        let out = unchanged(d.path(), &source, &["--only", COP, "lib/a.rb"], status);
        let mut lines: Vec<_> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.ends_with(&format!("({COP})")))
            .map(str::to_owned)
            .collect();
        lines.sort();
        let mut want: Vec<_> = expected
            .iter()
            .map(|(l, c, _, m)| format!("lib/a.rb:{l}:{c}: E: {m} ({COP})"))
            .collect();
        want.sort();
        assert_eq!(lines, want, "{row}");
        // The historical reviewed_after witnesses remain syntax-checked below,
        // but no longer stand in for safe CLI applicability (default warnings).
        unchanged(
            d.path(),
            &source,
            &["--only", COP, "--fix", "lib/a.rb"],
            status,
        );
    }
}
#[test]
fn defaults_selectors_strict_toml_department_precedence_and_stdin_provenance() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(
        !cop.safe_fixes(),
        "warning safety disposition is unresolved"
    );
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(
        cop.path_policy().is_some()
            && cop.max_setting().is_none()
            && cop.autocorrect_incompatible_with().is_empty()
    );
    let d = project();
    for (config, select, want) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint"], 1),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        (String::new(), vec!["--only", "Lint", "--except", COP], 0),
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
        fs::write(d.path().join("lib/a.rb"), EFFECT).unwrap();
        let mut args = select.clone();
        args.push("lib/a.rb");
        let out = run(d.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), want);
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
        child.stdin.take().unwrap().write_all(EFFECT).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), want);
        if want > 0 {
            assert!(String::from_utf8_lossy(&out.stdout).starts_with("lib/a.rb:"));
        }
        assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), EFFECT);
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
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(d.path(), SAFE, &["--only", COP, "--fix", "lib/a.rb"], 2);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    assert_eq!(
        run(d.path(), &["--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
}
#[test]
fn mandatory_jb_exclusion_uses_actual_supplied_file_and_stdin_path() {
    let d = project();
    for (path, n) in [
        ("a.jb", 0),
        ("lib/a.jb", 0),
        ("lib/.hidden.jb", 0),
        ("lib/a.jb.rb", 1),
        ("Gemfile", 1),
        ("lib/.hidden.rb", 1),
    ] {
        fs::write(d.path().join(path), EFFECT).unwrap();
        assert_eq!(count(&run(d.path(), &["--only", COP, "--fix", path])), n);
        assert_eq!(fs::read(d.path().join(path)).unwrap(), EFFECT);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", COP, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(EFFECT).unwrap();
        assert_eq!(count(&child.wait_with_output().unwrap()), n);
    }
    // Disabling/reenabling cannot bypass pinned path eligibility.
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    fs::write(
        d.path().join("lib/a.jb"),
        format!("# rustocop:enable {COP}\nreturn 1, 2, 3\n"),
    )
    .unwrap();
    assert_eq!(
        count(&run(d.path(), &["--only", COP, "--fix", "lib/a.jb"])),
        0
    );
}
#[test]
fn directives_both_prefixes_multiline_next_and_push_pop_scopes() {
    let d = project();
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
            let s = format!("# {prefix}:{directive}\nreturn effect()\n");
            assert_eq!(
                count(&unchanged(d.path(), s.as_bytes(), &["lib/a.rb"], 1)),
                1
            );
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", "Lint/Debugger", "lib/a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for s in [
            format!("# {prefix}:disable Lint\nreturn 1, 2, 3\n"),
            format!("# {prefix}:disable-next {COP}\nreturn 1\n"),
            format!("# {prefix}:push -{COP}\nreturn 1\n# {prefix}:pop\n"),
            format!("return (\n  1 # {prefix}:disable {COP}\n)\n"),
        ] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        let s = format!(
            "# {prefix}:disable {COP}\nreturn effect\n# {prefix}:enable {COP}\nreturn effect\n# {prefix}:push -{COP}\nreturn effect\n# {prefix}:pop\nreturn effect\n"
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                1
            )),
            2
        );
    }
}
#[test]
fn effectful_observer_and_source_runtime_holds_retain_findings() {
    let d = project();
    for source in [
        "return effect()",
        "return x = 1",
        "return *[1]",
        "return [1,2]",
        "return \"#{effect}\"",
        "return 1.0",
        "return (1)",
        "return <<~DOC\ntext\nDOC\n",
        "File.binread(__FILE__)\nreturn 1",
        "RubyVM::InstructionSequence.compile('1')\nreturn 1",
        "ObjectSpace.each_object {}\nreturn 1",
        "GC.stat\nreturn 1",
        "binding.eval('local_variables')\nreturn 1",
        "return 1\nDATA.read\n__END__\ndata\n",
    ] {
        assert_eq!(
            count(&unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                1
            )),
            1
        );
    }
    unchanged(
        d.path(),
        b"return 1\ndef broken(\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        2,
    );
    for source in [
        "def f; return 1; end",
        "def self.f; return 1; end",
        "each { return 1 }",
        "-> { return 1 }",
        "each { return it }",
        "each { return _1 }",
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            0,
        );
    }
}
#[test]
fn withheld_fix_composes_with_style_layout_and_is_idempotent() {
    let d = project();
    let p = d.path().join("lib/a.rb");
    fs::write(&p, b"return 1, 2, 3;  \n").unwrap();
    let selection = "Lint/TopLevelReturnWithArgument,Style/Semicolon,Layout/TrailingWhitespace";
    let out = run(d.path(), &["--only", selection, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&p).unwrap(), SAFE);
    unchanged(
        d.path(),
        SAFE,
        &["--only", selection, "--fix", "lib/a.rb"],
        1,
    );
    fs::write(&p, b"return effect;  \n").unwrap();
    let out = run(d.path(), &["--only", selection, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&p).unwrap(), b"return effect\n");
}
#[cfg(unix)]
#[test]
fn withheld_fix_preserves_permissions_held_inode_symlink_and_readonly_file() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&path, d.path().join("link.rb")).unwrap();
    let mut held = fs::File::open(&path).unwrap();
    let out = run(d.path(), &["--only", COP, "--fix", "link.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    let mut previous = Vec::new();
    held.read_to_end(&mut previous).unwrap();
    assert_eq!(previous, SAFE);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(d.path().join("link.rb"))
            .unwrap()
            .is_symlink()
    );
    // Still exercise actual shared atomic writes through the symlink, using a
    // safe peer while this cop's finding remains withheld.
    fs::write(&path, b"return 1, 2, 3;\n").unwrap();
    let mut old_handle = fs::File::open(&path).unwrap();
    let peers = "Lint/TopLevelReturnWithArgument,Style/Semicolon";
    let out = run(d.path(), &["--only", peers, "--fix", "link.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    let mut previous = Vec::new();
    old_handle.read_to_end(&mut previous).unwrap();
    assert_eq!(previous, b"return 1, 2, 3;\n");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(d.path().join("link.rb"))
            .unwrap()
            .is_symlink()
    );
    fs::write(&path, b"return 1, 2, 3;\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(d.path(), &["--only", COP, "--fix", "link.rb"])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(fs::read(&path).unwrap(), b"return 1, 2, 3;\n");
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    assert_eq!(
        old.permissions(),
        fs::metadata(&path).unwrap().permissions()
    );
    assert_eq!(
        run(d.path(), &["--only", peers, "--fix", "link.rb"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(&path).unwrap(), b"return 1, 2, 3;\n");
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
fn native_syntax_only_for_every_original_input_and_expected_output() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        let mut sources = vec![("bytes", bytes(&row, "bytes"))];
        for key in ["reviewed_after", "upstream_after"] {
            if row.get(key).is_some() {
                sources.push((key, bytes(&row, key)));
            }
        }
        for (key, source) in sources {
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
                row.get("parser_rejection").is_none()
                    && !(key == "upstream_after" && row.get("upstream_native_rejection").is_some()),
                "{row}\n{out:?}"
            );
        }
    }
}
