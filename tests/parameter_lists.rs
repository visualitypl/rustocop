//! Source-only pinned fixtures, actual CLI/native syntax and no-write proofs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, select_active},
    offense::Severity,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Metrics/ParameterLists";
const SOURCE: &[u8] = b"def foo(a,b,c,d,e,f); end\n";
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
        .filter(|l| l.contains(&format!("({COP})")))
        .count()
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, bytes).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}\n{bytes:?}");
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/parameter_lists.json")).unwrap()
}
#[test]
fn complete_fixed_reference_bytes_match_pipeline_and_actual_cli_no_write() {
    let d = project();
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    let active = select_active(&registry, &config, &[COP.into()], &[]);
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let report = rustocop::pipeline::lint_one(
            &d.path().join("lib/a.rb"),
            Path::new("lib/a.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        let rejected = row.get("parser_rejection").is_some();
        assert_eq!(!report.errors.is_empty(), rejected, "{row}: {report:?}");
        let mut found: Vec<_> = report
            .result
            .offenses
            .into_iter()
            .map(|o| {
                assert_eq!(o.cop, COP);
                assert_eq!(o.severity, Severity::Convention);
                assert!(o.correction.is_none());
                (o.line, o.col, o.end_line, o.message)
            })
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> = if rejected {
            vec![]
        } else {
            serde_json::from_value(
                row.get("project_findings")
                    .unwrap_or(&row["findings"])
                    .clone(),
            )
            .unwrap()
        };
        assert_eq!(found, expected, "{row}");
        for fix in [false, true] {
            let mut args = vec!["--only", COP, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = unchanged(
                d.path(),
                &bytes,
                &args,
                if rejected {
                    2
                } else {
                    i32::from(!found.is_empty())
                },
            );
            assert_eq!(count(&out), found.len(), "{row}: {out:?}");
            let stdout = String::from_utf8_lossy(&out.stdout);
            for (line, col, _, message) in &found {
                assert!(
                    stdout.contains(&format!("lib/a.rb:{line}:{col}: C: {message} ({COP})")),
                    "{row}: {stdout}"
                );
            }
        }
    }
}
#[test]
fn actual_metadata_and_only_landed_metrics_names_are_admitted() {
    let registry = Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(cop.severity(), Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
    assert_eq!(
        registry
            .cops()
            .iter()
            .filter(|c| c.name().starts_with("Metrics/"))
            .map(|c| c.name())
            .collect::<Vec<_>>(),
        [
            COP,
            "Metrics/BlockLength",
            "Metrics/ClassLength",
            "Metrics/MethodLength",
            "Metrics/ModuleLength",
            "Metrics/BlockNesting",
            "Metrics/CollectionLiteralLength",
            "Metrics/CyclomaticComplexity",
            "Metrics/PerceivedComplexity",
            "Metrics/AbcSize"
        ]
    );
    for name in ["Metrics/Future", "Gemspec/Future"] {
        assert!(!registry.has_cop(name));
    }
}
#[test]
fn defaults_config_department_only_except_exclusions_and_strict_errors() {
    let d = project();
    for (config, flags, expected) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", "Metrics"], 1),
        (String::new(), vec!["--only", COP, "--except", "Metrics"], 0),
        (String::new(), vec!["--only", "Metrics", "--except", COP], 0),
        ("[cops.Metrics]\nenabled=false\n".into(), vec![], 0),
        (
            format!("[cops.Metrics]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
            vec![],
            1,
        ),
        (
            format!("[cops.Metrics]\nenabled=true\n[cops.\"{COP}\"]\nenabled=false\n"),
            vec![],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled=false\n"),
            vec!["--only", "Metrics"],
            1,
        ),
        ("exclude=['lib/a.rb']\n".into(), vec!["--only", COP], 0),
        (
            format!("[cops.\"{COP}\"]\nexclude=['lib/a.rb']\n"),
            vec!["--only", COP],
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = flags;
        args.push("lib/a.rb");
        let out = unchanged(
            d.path(),
            SOURCE,
            &args,
            if args.contains(&"--only") {
                expected
            } else {
                1
            },
        );
        assert_eq!(count(&out), expected as usize, "{out:?}");
    }
    for body in [
        "max=6",
        "Max=6",
        "MaxOptionalParameters=4",
        "CountKeywordArgs=false",
        "include=['**/*']",
        "enabled='true'",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{body}\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCE, &["--only", COP, "--fix", "lib/a.rb"], 2);
    }
    for name in ["Migration/Nonexistent", "Metrics/Future"] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{name}\"]\nenabled=true\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCE, &["--only", COP, "--fix", "lib/a.rb"], 2);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        unchanged(d.path(), SOURCE, &["--only", name, "--fix", "lib/a.rb"], 2);
    }
}
#[test]
fn supplied_nonstandard_paths_relative_exclusions_and_invalid_encoding_do_not_write() {
    let d = project();
    fs::write(d.path().join("lib/input.arb"), SOURCE).unwrap();
    let out = run(d.path(), &["--only", "Metrics", "lib/input.arb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out), 1);
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nexclude=['lib/input.arb']\n"),
    )
    .unwrap();
    let out = run(
        &d.path().join("lib"),
        &["--only", "Metrics", "--fix", "input.arb"],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(d.path().join("lib/input.arb")).unwrap(), SOURCE);
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for bytes in [
        b"# coding: UTF-8\ndef foo(a,b,c,d,e,f); '\xff'; end\n".as_slice(),
        b"# coding: NotAnEncoding\ndef foo(a,b,c,d,e,f); end\n",
    ] {
        let out = unchanged(
            d.path(),
            bytes,
            &["--only", "Metrics", "--fix", "lib/a.rb"],
            2,
        );
        assert_eq!(count(&out), 0);
    }
}

#[test]
fn both_directive_prefixes_spans_next_push_enable_and_unknown_names() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Metrics\ndef foo(a,b,c,d,e,f); end\n"),
            format!("# {prefix}:disable-next {COP}\ndef foo(a,b,c,d,e,f)\nend\n"),
            format!("def foo(a,b,c,\n d,e,f) # {prefix}:todo {COP}\nend\n"),
            format!("# {prefix}:push -Metrics\ndef foo(a,b,c,d,e,f); end\n# {prefix}:pop\n"),
        ] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "# {prefix}:disable Metrics\ndef foo(a,b,c,d,e,f); end\n# {prefix}:enable {COP}\ndef bar(a,b,c,d,e,f); end\n"
        );
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
    let source = b"# rubocop:disable Metrics/AbcSize\ndef foo(a,b,c,d,e,f); end\n";
    assert_eq!(
        count(&unchanged(
            d.path(),
            source,
            &["--only", "Metrics,Lint/CopDirectiveSyntax", "lib/a.rb"],
            1
        )),
        1
    );
}
#[test]
fn stdin_paths_exclusions_encoded_buffers_and_early_fix_rejection_never_write() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, b"sentinel\n").unwrap();
    let old = fs::metadata(&path).unwrap();
    for (config, expected) in [
        (String::new(), 1),
        (format!("[cops.\"{COP}\"]\nexclude=['lib/a.rb']\n"), 0),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", "Metrics", "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"# coding: Windows-1252\r\ndef \xe9(a,b,c,d,e,f); end\r\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(expected));
        assert_eq!(count(&out), expected as usize);
    }
    let out = run(
        d.path(),
        &["--only", "Metrics", "--stdin", "lib/a.rb", "--fix"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"sentinel\n");
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
}
#[test]
fn parse_failure_missing_readonly_alias_and_atomic_peer_failure_preserve_signature() {
    let d = project();
    let out = unchanged(
        d.path(),
        b"def foo(a,b,c,d,e,f); end\ndef broken(\n",
        &["--only", "Metrics", "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!(count(&out), 0);
    assert_eq!(
        run(d.path(), &["--only", "Metrics", "--fix", "missing.rb"])
            .status
            .code(),
        Some(2)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = d.path().join("lib/a.rb");
        let source = b"def foo(a,b,c,d,e,f);\t'value'; end\n";
        fs::write(&path, source).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        symlink("a.rb", d.path().join("lib/link.rb")).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(
            d.path(),
            &[
                "--only",
                "Metrics,Style/StringLiterals,Layout/IndentationStyle",
                "--fix",
                "lib/link.rb",
            ],
        );
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), 1);
        assert_eq!(fs::read(&path).unwrap(), source);
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert!(
            fs::symlink_metadata(d.path().join("lib/link.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
#[test]
fn style_layout_composition_fixes_peers_not_parameter_lists_and_stabilizes() {
    let d = project();
    let source = b"def foo(a,b,c,d,e,f)\n\t'value'\nend\n";
    fs::write(d.path().join("lib/a.rb"), source).unwrap();
    let flags = [
        "--only",
        "Metrics,Style/StringLiterals,Layout/IndentationStyle",
        "--fix",
        "lib/a.rb",
    ];
    let out = run(d.path(), &flags);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out), 1);
    let after = fs::read(d.path().join("lib/a.rb")).unwrap();
    assert_eq!(after, b"def foo(a,b,c,d,e,f)\n  \"value\"\nend\n");
    let old = fs::metadata(d.path().join("lib/a.rb")).unwrap();
    let out = run(d.path(), &flags);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), after);
    assert_eq!(
        fs::metadata(d.path().join("lib/a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        old.modified().unwrap()
    );
}
#[test]
fn native_ruby34_syntax_only_all_original_reference_bytes() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-W0", "-c"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            out.status.success(),
            row.get("parser_rejection").is_none(),
            "{row}: {out:?}"
        );
    }
}
