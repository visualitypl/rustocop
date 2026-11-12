//! Source-only pinned parity, rebuilt CLI/no-write and authored native witnesses.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = ["Lint/ArrayLiteralInRegexp", "Lint/BooleanSymbol"];
const SOURCES: [&str; 2] = ["/#{[1]}/\n", ":true\n"];
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/literal_lookalikes.json")).unwrap()
}
fn source(row: &serde_json::Value) -> Vec<u8> {
    row["source"]
        .as_str()
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_else(|| serde_json::from_value(row["bytes"].clone()).unwrap())
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
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], code: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, bytes).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(code) = code {
        assert_eq!(
            out.status.code(),
            Some(code),
            "{args:?}\n{out:?}\n{}",
            String::from_utf8_lossy(bytes)
        );
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes, "{out:?}");
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
fn all_pinned_main_source_canonical_and_control_rows_pipeline() {
    assert_eq!(rows().len(), 416);
    for row in rows() {
        let registry = rustocop::cops::Registry::new();
        let active: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| COPS.contains(&c.name()))
            .map(|c| c.as_ref())
            .collect();
        let result = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source(&row)),
            &active,
            &rustocop::config::ResolvedConfig::default(),
        );
        assert!(
            result.errors.is_empty() && result.internal_errors.is_empty(),
            "{row}\n{:?}",
            result.errors
        );
        for cop in COPS {
            let found: Vec<_> = result
                .result
                .offenses
                .iter()
                .filter(|o| o.cop == cop)
                .map(|o| {
                    assert!(o.correction.is_none());
                    serde_json::json!([o.line, o.col, o.end_line, o.message])
                })
                .collect();
            assert_eq!(
                serde_json::json!(found),
                row[cop]["findings"],
                "{cop}\n{row}"
            );
        }
    }
}
#[test]
fn all_rows_exact_rebuilt_cli_with_and_without_fix_preserve_bytes_mtime_inode_mode() {
    let dir = project();
    for row in rows() {
        for cop in COPS {
            let findings = row[cop]["findings"].as_array().unwrap();
            for fix in [false, true] {
                let mut args = vec!["--only", cop, "lib/a.rb"];
                if fix {
                    args.push("--fix");
                }
                let out = unchanged(
                    dir.path(),
                    &source(&row),
                    &args,
                    Some(i32::from(!findings.is_empty())),
                );
                assert_eq!(count(&out, cop), findings.len(), "{row}\n{out:?}");
                for f in findings {
                    assert!(
                        String::from_utf8_lossy(&out.stdout).contains(&format!(
                            "lib/a.rb:{}:{}: E: {} ({cop})",
                            f[0],
                            f[1],
                            f[3].as_str().unwrap()
                        )),
                        "{row}\n{out:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn source_filename_value_counts_unicode_characters_with_real_stdin_provenance() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry
        .cops()
        .iter()
        .find(|c| c.name() == COPS[0])
        .unwrap()
        .as_ref();
    let dir = project();
    for path in ["a", "é", "ab", "e\u{0301}"] {
        let category = if path.chars().count() == 1 {
            "a character class"
        } else {
            "alternation"
        };
        let message = format!("Use {category} instead of interpolating an array in a regexp.");
        let result = rustocop::pipeline::lint_one(
            Path::new(path),
            Path::new(path),
            Some(b"/#{[__FILE__]}/"),
            &[cop],
            &rustocop::config::ResolvedConfig::default(),
        );
        assert_eq!(result.result.offenses.len(), 1);
        assert_eq!(result.result.offenses[0].message, message, "{path}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COPS[0], "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"/#{[__FILE__]}/")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(&message),
            "{path}\n{out:?}"
        );
    }
}
#[test]
fn metadata_defaults_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let c = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(c.default_enabled() && !c.safe_fixes());
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(
            c.path_policy().is_none()
                && c.max_setting().is_none()
                && c.autocorrect_incompatible_with().is_empty()
        );
        let dir = project();
        for (config, selectors, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint"], 1),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
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
            assert_eq!(
                count(&unchanged(dir.path(), source.as_bytes(), &args, None), cop),
                n
            );
        }
        for key in [
            "max = 1",
            "include = ['**/*']",
            "Enabled = true",
            "enabled = 'true'",
            "target_ruby = 3.3",
            "SafeAutoCorrect = false",
            "NewCops = 'enable'",
            "Preview = true",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(2),
            );
        }
        for config in [
            "exclude = ['lib/**']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
}
#[test]
fn directive_prefixes_range_multiline_next_signed_push_and_source_activation() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(["/#{\n[1]\n}/\n", ":\"tr\\\nue\"\n"]) {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Lint", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!(
                        "# {prefix}:push\n# {prefix}:disable {target}\n{source}# {prefix}:pop\n"
                    ),
                ] {
                    unchanged(
                        dir.path(),
                        s.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        Some(0),
                    );
                }
            }
            for mode in [format!("disable-next {cop}"), format!("next -{cop}")] {
                unchanged(
                    dir.path(),
                    format!("# {prefix}:{mode}\n{source}").as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(0),
                );
            }
            for opener in [
                format!("# {prefix}:push +{cop}"),
                format!("# {prefix}:enable {cop}"),
            ] {
                unchanged(
                    dir.path(),
                    format!("# {prefix}:disable {cop}\n{opener}\n{source}# {prefix}:pop\n")
                        .as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(1),
                );
            }
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            let s = format!("# {prefix}:enable {cop}\n{source}");
            assert_eq!(
                count(
                    &unchanged(dir.path(), s.as_bytes(), &["lib/a.rb"], None),
                    cop
                ),
                1
            );
            assert_eq!(
                count(
                    &unchanged(
                        dir.path(),
                        s.as_bytes(),
                        &["--except", cop, "lib/a.rb"],
                        None
                    ),
                    cop
                ),
                0
            );
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        }
    }
    for prefix in ["rubocop", "rustocop"] {
        unchanged(
            dir.path(),
            format!("/#{{\n [1] # {prefix}:disable {}\n}}/\n", COPS[0]).as_bytes(),
            &["--only", COPS[0], "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn file_and_virtual_stdin_paths_builtins_and_stdin_fix_rejection() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for path in [
            "lib/a.rb",
            "a.jb",
            "view.erb",
            "db/schema.rb",
            "vendor/a.rb",
            ".hidden.rb",
        ] {
            let p = dir.path().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, source).unwrap();
            let n = usize::from(!path.starts_with("vendor/") && path != "db/schema.rb");
            assert_eq!(count(&run(dir.path(), &["--only", cop, path]), cop), n);
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
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
                .write_all(source.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(count(&out, cop), n, "{out:?}");
            if n == 1 {
                let col = if cop == COPS[0] { 2 } else { 1 };
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:1:{col}:")),
                    "{out:?}"
                );
            }
        }
        assert_eq!(
            run(dir.path(), &["--only", cop, "--stdin", "lib/a.rb", "--fix"])
                .status
                .code(),
            Some(2)
        );
    }
}
#[cfg(unix)]
#[test]
fn readonly_symlink_open_fd_invalid_source_atomic_failure_and_actual_peer_edit() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        fs::write(&path, source).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let mut fd = fs::File::open(&path).unwrap();
        for alias in ["lib/a.rb", "lib/link.rb"] {
            assert_eq!(
                run(dir.path(), &["--only", cop, "--fix", alias])
                    .status
                    .code(),
                Some(1)
            );
        }
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap(), old.permissions()),
            (new.ino(), new.modified().unwrap(), new.permissions())
        );
        let mut bytes = vec![];
        fd.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, source.as_bytes());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        unchanged(
            dir.path(),
            format!("{source}(\n").as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(2),
        );
        let before = source.replace('\n', "  \n");
        let only = format!("{cop},Layout/TrailingWhitespace");
        fs::write(&path, &before).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", &only, "--fix", "lib/link.rb"])
                .status
                .code(),
            Some(1)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert!(
            fs::symlink_metadata(dir.path().join("lib/link.rb"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::write(&path, &before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", &only, "--fix", "lib/a.rb"])
                .status
                .code(),
            Some(2)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap()),
            (new.ino(), new.modified().unwrap())
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    }
}
#[test]
fn authored_native_pattern_and_symbol_value_counterexamples_remain_unedited() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let dir = project();
    // Separately authored bounded programs, never original spec/doc/corpus code.
    for (cop, before, after) in [
        (
            COPS[0],
            "r = /#{['q', 'z']}/; p [r.source, r.match?(' '), r.match?('q')]",
            "r = /[qz]/; p [r.source, r.match?(' '), r.match?('q')]",
        ),
        (
            COPS[0],
            "r = /#{['amber', 'blue']}/; p [r.source, r.match?('m'), r.match?('amber')]",
            "r = /(?:amber|blue)/; p [r.source, r.match?('m'), r.match?('amber')]",
        ),
        (COPS[0], "p /#{[]}/.source", "p /[]/.source"),
        (
            COPS[1],
            "v = :true; p [v.class, v, {true => 7, :true => 9}[v]]",
            "v = true; p [v.class, v, {true => 7, :true => 9}[v]]",
        ),
        (
            COPS[1],
            "v = :false; p [v.class, v, !!v]",
            "v = false; p [v.class, v, !!v]",
        ),
    ] {
        let execute = |s| {
            Command::new(&ruby)
                .args(["--parser=prism", "-w", "-e", s])
                .output()
                .unwrap()
        };
        let a = execute(before);
        let b = execute(after);
        assert!(
            a.stdout != b.stdout || a.stderr != b.stderr || a.status.code() != b.status.code(),
            "{a:?}\n{b:?}"
        );
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    for suffix in [
        "p __LINE__\n",
        "p File.binread(__FILE__).bytesize\n",
        "p DATA.read\n__END__\npayload\n",
        "p RubyVM::InstructionSequence.of(method(:p)).inspect\n",
        "p ObjectSpace.count_objects[:T_STRING]\n",
        "x = <<~TEXT\n auth #{__LINE__}\nTEXT\n",
    ] {
        for s in [
            format!("\u{feff}/#{{[1]}}/ # retained\r\n{suffix}"),
            format!(":false # retained\r\n{suffix}"),
        ] {
            unchanged(
                dir.path(),
                s.as_bytes(),
                &["--only", &COPS.join(","), "--fix", "lib/a.rb"],
                Some(1),
            );
        }
    }
}
#[test]
fn all_reference_inputs_and_canonical_outputs_are_native_syntax_only() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let mut invalid = vec![];
    for row in rows() {
        let mut sources = vec![(None, source(&row))];
        for cop in COPS {
            sources.push((
                Some(cop),
                serde_json::from_value::<Vec<u8>>(row[cop]["upstream_after_bytes"].clone())
                    .unwrap(),
            ));
        }
        for (cop, bytes) in sources {
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-c", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
            let out = child.wait_with_output().unwrap();
            if !out.status.success() {
                // These are exact upstream invalid outputs, never detector
                // exclusions. Reference-buffer UTF8 output retains declared
                // legacy headers, which can produce invalid native bytes.
                let cop = cop.expect("original fixture must be native-valid");
                let error = String::from_utf8_lossy(&out.stderr);
                assert!(
                    error.contains("empty char-class")
                        || error.contains("invalid multibyte character")
                        || error.contains("unexpected backslash"),
                    "{row}\n{out:?}"
                );
                invalid.push((
                    row["origin"].as_str().unwrap().to_owned(),
                    row["line"].as_u64().unwrap(),
                    cop,
                ));
            }
        }
    }
    let mut expected = vec![
        ("control".into(), 1, COPS[0]),
        ("control".into(), 2, COPS[0]),
        ("control".into(), 3, COPS[0]),
        ("bytes".into(), 316, COPS[1]),
        ("bytes".into(), 317, COPS[1]),
        ("bytes".into(), 320, COPS[1]),
        ("bytes".into(), 321, COPS[0]),
        ("bytes".into(), 323, COPS[1]),
        ("bytes".into(), 324, COPS[1]),
        ("bytes".into(), 327, COPS[1]),
        ("bytes".into(), 328, COPS[0]),
    ];
    // Upstream replaces the interpolation but leaves heredoc bodies behind as
    // code: these exact 27 outputs expose unexpected backslashes. All 72 valid
    // originals are still detected and preserved, not parser-skipped.
    expected.extend(
        [
            2, 3, 4, 10, 11, 12, 18, 19, 20, 26, 27, 28, 34, 35, 36, 42, 43, 44, 50, 51, 52, 58,
            59, 60, 66, 67, 68,
        ]
        .into_iter()
        .map(|line| ("heredoc".into(), line, COPS[0])),
    );
    assert_eq!(invalid, expected);
}
