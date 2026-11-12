//! RuboCop MIT examples are SOURCE ONLY; authored native controls separate.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/UnusedBlockArgument";
const SOURCE: &[u8] = b"hash.each do |key, value|\n  puts key\nend\n";
const SAFE: &[u8] = b"if false\nhash.each do |key, value|\n  puts key\nend\nend\n";
const AFTER: &[u8] = b"if false\nhash.each do |key, _value|\n  puts key\nend\nend\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/unused_block_argument.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    serde_json::from_value(row[key].clone()).unwrap()
}
#[test]
fn pinned_main_source_and_all_original_bytes_match_exact_pipeline_findings() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let rows = rows();
    assert_eq!(rows.len(), 4415);
    let mut failures = Vec::new();
    for row in rows {
        let source = bytes(&row, "bytes");
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        if row.get("parser_rejection").is_some() {
            assert!(
                !report.errors.is_empty() && report.result.offenses.is_empty(),
                "{row}: {report:?}"
            );
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
                String::from_utf8_lossy(&source)
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
fn all_original_bytes_match_rebuilt_cli_lint_and_actual_guarded_fix() {
    parallel_rows!(for row in rows() => {
        let d = project();
        let source = bytes(&row, "bytes");
        let rejected = row.get("parser_rejection").is_some();
        let expected: Vec<Finding> = if rejected {
            vec![]
        } else {
            serde_json::from_value(row["findings"].clone()).unwrap()
        };
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            if fix && row.get("project_after").is_some() {
                let p = d.path().join("lib/a.rb");
                fs::write(&p, &source).unwrap();
                let out = run(d.path(), &args);
                assert_eq!(out.status.code(), Some(0), "{row}: {out:?}");
                assert_eq!(fs::read(&p).unwrap(), bytes(&row, "project_after"));
                assert_eq!(count(&out), 0);
            } else {
                let out = unchanged(
                    d.path(),
                    &source,
                    &args,
                    Some(if rejected {
                        2
                    } else {
                        i32::from(!expected.is_empty())
                    }),
                );
                assert_eq!(count(&out), expected.len(), "{row}: {out:?}");
                if !rejected {
                    for (line, col, _, message) in &expected {
                        assert!(
                            String::from_utf8_lossy(&out.stdout).contains(&format!(
                                "lib/a.rb:{line}:{col}: E: {message} ({COP})\n"
                            )),
                            "{row}: {out:?}"
                        );
                    }
                }
            }
        }
    });
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
fn count(o: &Output) -> usize {
    String::from_utf8_lossy(&o.stdout)
        .matches(&format!("({COP})"))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let p = root.join("lib/a.rb");
    fs::write(&p, source).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o751)).unwrap();
    }
    let old = fs::metadata(&p).unwrap();
    let mut held = fs::File::open(&p).unwrap();
    let o = run(root, args);
    if let Some(s) = status {
        assert_eq!(o.status.code(), Some(s), "{o:?} {source:?}");
    } else {
        assert_ne!(o.status.code(), Some(2), "{o:?}");
    }
    assert_eq!(fs::read(&p).unwrap(), source);
    let mut buf = Vec::new();
    held.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, source);
    let new = fs::metadata(&p).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
    o
}
#[test]
fn defaults_selectors_minimal_toml_paths_stdin_and_directive_scopes() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    let detector = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(detector.default_enabled() && detector.safe_fixes());
    assert!(detector.max_setting().is_none() && detector.path_policy().is_none());
    assert!(detector.autocorrect_incompatible_with().is_empty());
    assert_eq!(
        detector.ruby_versions(),
        rustocop::cops::policy::RubyVersions {
            minimum: None,
            maximum: None
        }
    );
    for (conf, mut args, n) in [
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
            format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
            vec!["--only", COP],
            0,
        ),
        ("exclude = ['lib/a.rb']\n".into(), vec!["--only", COP], 0),
    ] {
        fs::write(d.path().join("rustocop.toml"), conf).unwrap();
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(d.path(), SOURCE, &args, None)), n);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "TargetRubyVersion = 3.3",
        "SafeAutoCorrect = true",
        "AllowUnusedKeywordArguments = true",
        "IgnoreEmptyBlocks = false",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.Lint]\nexclude = ['lib/a.rb']\n",
    )
    .unwrap();
    unchanged(
        d.path(),
        SOURCE,
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        let text = std::str::from_utf8(SOURCE).unwrap();
        for source in [
            format!("# {prefix}:disable Lint\n{text}"),
            format!("# {prefix}:disable-next {COP}\n{text}"),
            format!("# {prefix}:next -{COP}\n{text}"),
            format!("# {prefix}:push -{COP}\n{text}# {prefix}:pop\n"),
            format!("hash.each do |key, value| # {prefix}:disable {COP}\n puts key\nend\n"),
        ] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        let source = format!(
            "# {prefix}:disable {COP}\n{text}# {prefix}:push +{COP}\n{text}# {prefix}:pop\n{text}"
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
        let source =
            format!("hash.each do |key, value|\n puts key # {prefix}:disable {COP}\nend\n");
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\n{text}");
        assert_eq!(
            count(&unchanged(d.path(), source.as_bytes(), &["lib/a.rb"], None)),
            1
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                source.as_bytes(),
                &["--except", COP, "lib/a.rb"],
                None
            )),
            0
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
    for conf in [
        String::new(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
        "exclude = ['lib/a.rb']\n".into(),
    ] {
        let n = usize::from(conf.is_empty());
        fs::write(d.path().join("rustocop.toml"), conf).unwrap();
        let supplied = d.path().join("lib/a.rb");
        fs::write(&supplied, b"sentinel\n").unwrap();
        let old = fs::metadata(&supplied).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(fs::read(&supplied).unwrap(), b"sentinel\n");
        let new = fs::metadata(&supplied).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert_eq!(count(&out), n);
        if n > 0 {
            assert!(String::from_utf8_lossy(&out.stdout).starts_with("lib/a.rb:"));
        }
        assert_eq!(
            count(&unchanged(
                d.path(),
                SOURCE,
                &["--only", COP, "lib/a.rb"],
                Some(n as i32)
            )),
            n
        );
        assert_eq!(
            count(&run(&d.path().join("lib"), &["--only", COP, "a.rb"])),
            n
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    assert_eq!(
        run(d.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
    for (path, n) in [
        ("Gemfile", 1),
        ("lib/a.jb", 1),
        ("lib/.hidden.rb", 1),
        ("lib/a_spec.rb", 1),
        ("vendor/a.rb", 0),
        ("db/schema.rb", 0),
    ] {
        let p = d.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, SOURCE).unwrap();
        let old = fs::metadata(&p).unwrap();
        assert_eq!(count(&run(d.path(), &["--only", COP, "--fix", path])), n);
        assert_eq!(fs::read(&p).unwrap(), SOURCE);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&p).unwrap().modified().unwrap()
        );
    }
}
#[test]
fn withheld_fixes_keep_findings_and_bytes_observers_binding_defaults_and_collision_controls() {
    let d = project();
    for source in [
        "xs.each { |foo:| nil }\n",
        "xs.each { |foo: 1| nil }\n",
        "xs.each { |&foo| nil }\n",
        "->(foo) { nil }\n",
        "define_method(:f) { |foo| nil }\n",
        "xs.each { |foo,_foo| nil }\n",
        "xs.each { |foo=effect| nil }\n",
        "xs.each { |foo| foo=1 }\n",
        "xs.each { |foo; local| nil }\n",
        "xs.each { |foo| eval('foo') }\n",
        "if false\nxs.each { |foo| nil }\nend\n",
    ] {
        assert!(
            count(&unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )) > 0
        );
    }
    for tail in [
        "File.binread(__FILE__)",
        "RubyVM::InstructionSequence.compile('1')",
        "ObjectSpace.count_objects",
        "GC.stat",
        "p __LINE__",
        "p __ENCODING__",
        "binding",
        "DATA.read\n__END__\nraw\0",
    ] {
        let source = [SAFE, tail.as_bytes(), b"\n"].concat();
        assert_eq!(
            count(&unchanged(
                d.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
    }
    for wrapper in ["def deferred; %s; end", "-> { %s }", "if true; %s; end"] {
        let source = wrapper.replace("%s", std::str::from_utf8(SAFE).unwrap());
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    let invalid = [SAFE, b"def broken(\n"].concat();
    unchanged(
        d.path(),
        &invalid,
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    for prefix in ["rubocop", "rustocop"] {
        let source = format!(
            "# {prefix}:disable-next {COP}\n{}",
            std::str::from_utf8(SAFE).unwrap()
        );
        // disable-next covers the complete following conditional's range.
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
        let source = format!(
            "if false\n# {prefix}:disable-next {COP}\nhash.each do |key, value|\n  puts key\nend\nend\n"
        );
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn symlink_failure_and_style_layout_atomic_composition() {
    let d = project();
    let p = d.path().join("lib/a.rb");
    let peers = format!("{COP},Style/StringLiterals,Layout/SpaceAfterComma");
    let source = [SOURCE, b"a = ['a',2]\n"].concat();
    let after = [SOURCE, b"a = [\"a\", 2]\n"].concat();
    unchanged(d.path(), &source, &["--only", &peers, "lib/a.rb"], Some(1));
    let out = run(d.path(), &["--only", &peers, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&p).unwrap(), after);
    unchanged(
        d.path(),
        &after,
        &["--only", &peers, "--fix", "lib/a.rb"],
        Some(1),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        symlink(&p, d.path().join("link.rb")).unwrap();
        fs::write(&p, SAFE).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&p).unwrap();
        let out = run(d.path(), &["--only", COP, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(fs::read(&p).unwrap(), SAFE);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&p).unwrap().modified().unwrap()
        );
        assert_eq!(old.permissions(), fs::metadata(&p).unwrap().permissions());
        assert!(
            fs::symlink_metadata(d.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
        fs::set_permissions(&p, fs::Permissions::from_mode(0o640)).unwrap();
        let out = run(d.path(), &["--only", COP, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(&p).unwrap(), AFTER);
        assert!(
            fs::symlink_metadata(d.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
#[test]
fn native_ruby34_syntax_only_for_every_reference_input_and_canonical_output() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    assert!(
        String::from_utf8_lossy(
            &Command::new(&ruby)
                .arg("--version")
                .output()
                .unwrap()
                .stdout
        )
        .contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        for key in ["bytes", "project_after", "upstream_after"] {
            if row.get(key).is_none() {
                continue;
            }
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-W0", "-c"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&bytes(&row, key))
                .unwrap();
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
fn canonical_rename_is_real_atomic_and_idempotent_but_ordinary_binding_is_held() {
    let d = project();
    unchanged(d.path(), SAFE, &["--only", COP, "lib/a.rb"], Some(1));
    let p = d.path().join("lib/a.rb");
    let mut held = fs::File::open(&p).unwrap();
    let old = fs::metadata(&p).unwrap();
    let out = run(d.path(), &["--only", COP, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(&p).unwrap(), AFTER);
    let mut old_bytes = Vec::new();
    held.read_to_end(&mut old_bytes).unwrap();
    assert_eq!(old_bytes, SAFE);
    assert_eq!(old.permissions(), fs::metadata(&p).unwrap().permissions());
    unchanged(
        d.path(),
        AFTER,
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(0),
    );
    unchanged(
        d.path(),
        SOURCE,
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(1),
    );
}
