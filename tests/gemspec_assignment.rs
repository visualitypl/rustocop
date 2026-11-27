//! Source-only fixtures from RuboCop (MIT, (c) 2012-26 Bozhidar Batsov).
use crate::support::cli::project;
use rustocop::{config::ResolvedConfig, cops::Registry, pipeline};
use std::{fs, path::Path, process::Command};
const NAMES: [&str; 2] = [
    "Gemspec/AttributeAssignment",
    "Gemspec/DuplicatedAssignment",
];
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/gemspec_assignment.json")).unwrap()
}
#[test]
fn all_main_specs_and_structural_controls_match_exact_reference() {
    let d = project();
    let r = Registry::new();
    for row in rows() {
        let cop = row["cop"].as_str().unwrap();
        assert!(r.has_cop(cop), "{cop} is not implemented");
        let config = ResolvedConfig::default();
        let selected = rustocop::cops::select_active(&r, &config, &[cop.into()], &[]);
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let report = pipeline::lint_one(
            &d.path().join("a.gemspec"),
            Path::new("a.gemspec"),
            Some(&bytes),
            &selected,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        let rejected = row.get("parser_errors").is_some();
        assert_eq!(!report.errors.is_empty(), rejected, "{row}: {report:?}");
        let mut found: Vec<_> = report
            .result
            .offenses
            .into_iter()
            .map(|o| {
                assert_eq!(o.cop, cop);
                assert!(o.correction.is_none());
                (o.line, o.col, o.end_line, o.message)
            })
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> = serde_json::from_value(
            row.get("findings")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .unwrap();
        assert_eq!(found, expected, "{row}");
    }
}
#[test]
fn fixture_cli_lint_and_fix_match_exact_reference_and_preserve_open_file_identity() {
    use std::io::Read;
    let position =
        regex::Regex::new(r"^a\.gemspec:(\d+):(\d+): [CW]: (.*?) \((Gemspec/\w+)\)$").unwrap();
    parallel_rows!(for row in rows() => {
        let d = project();
        let path = d.path().join("a.gemspec");
        let cop = row["cop"].as_str().unwrap();
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        fs::write(&path, &bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
        }
        let expected: Vec<(u32, u32, u32, String)> = serde_json::from_value(
            row.get("findings")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .unwrap();
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(line, col, _, message)| (line, col, message))
            .collect();
        let rejected = row.get("parser_errors").is_some();
        for fix in [false, true] {
            let old = fs::metadata(&path).unwrap();
            let mut held = fs::File::open(&path).unwrap();
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command.current_dir(d.path()).args(["--only", cop]);
            if fix {
                command.arg("--fix");
            }
            let out = command.arg("a.gemspec").output().unwrap();
            assert_eq!(
                out.status.code(),
                Some(if rejected {
                    2
                } else {
                    i32::from(!expected.is_empty())
                }),
                "{row}: {out:?}"
            );
            if !rejected {
                assert!(out.stderr.is_empty(), "{row}: {out:?}");
            }
            let mut actual: Vec<_> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| {
                    let m = position.captures(line)?;
                    assert_eq!(&m[4], cop, "{row}: {out:?}");
                    Some((
                        m[1].parse::<u32>().unwrap(),
                        m[2].parse::<u32>().unwrap(),
                        m[3].to_owned(),
                    ))
                })
                .collect();
            actual.sort();
            assert_eq!(actual, expected, "fix={fix}: {row}: {out:?}");
            let mut held_bytes = Vec::new();
            held.read_to_end(&mut held_bytes).unwrap();
            assert_eq!(held_bytes, bytes);
            assert_eq!(fs::read(&path).unwrap(), bytes);
            let new = fs::metadata(&path).unwrap();
            assert_eq!(old.modified().unwrap(), new.modified().unwrap());
            assert_eq!(old.permissions(), new.permissions());
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(
                    (old.ino(), old.dev(), old.mtime_nsec()),
                    (new.ino(), new.dev(), new.mtime_nsec())
                );
                assert_eq!(held.metadata().unwrap().ino(), old.ino());
            }
        }
    });
}
#[test]
fn metadata_admits_exactly_two_landed_gemspec_cops() {
    let r = Registry::new();
    assert_eq!(
        r.cops()
            .iter()
            .filter(|c| c.name().starts_with("Gemspec/"))
            .map(|c| c.name())
            .collect::<Vec<_>>(),
        // Plus OrderedDependencies, the gemspec-dependencies
        // family and the gemspec-requirements family
        //, in registry order.
        [
            "Gemspec/AddRuntimeDependency",
            NAMES[0],
            "Gemspec/DependencyVersion",
            "Gemspec/DeprecatedAttributeAssignment",
            "Gemspec/DevelopmentDependencies",
            NAMES[1],
            "Gemspec/OrderedDependencies",
            "Gemspec/RequireMFA",
            "Gemspec/RequiredRubyVersion",
            "Gemspec/RubyVersionGlobalsUsage"
        ]
    );
    for name in NAMES {
        let c = r.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.severity(), rustocop::offense::Severity::for_cop(name));
        assert!(!c.safe_fixes());
        assert!(c.max_setting().is_none());
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(c.path_policy().is_some());
    }
    assert!(!r.has_cop("Gemspec/Future"));
}
const BOTH: &str = "Gemspec/AttributeAssignment,Gemspec/DuplicatedAssignment";
const SOURCE: &str = "Gem::Specification.new do |spec|\n  spec.metadata = {}\n  spec.metadata[:x] = 1\n  spec.metadata[:x] = 2\nend\n";
fn cli(root: &Path, args: &[&str], input: Option<&[u8]>) -> std::process::Output {
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        stdin::write_stdin(child.stdin.take().unwrap(), input, args.contains(&"--fix")).unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}
use crate::support::stdin;
fn count(out: &std::process::Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| NAMES.iter().any(|name| l.contains(&format!("({name})"))))
        .count()
}
#[test]
fn selectors_strict_config_and_real_path_gates_cannot_be_bypassed() {
    let d = project();
    for path in [
        "a.gemspec",
        "nested/.hidden.gemspec",
        "a.rb",
        "a.gemspec.rb",
        "Gemfile",
        "gems.locked",
    ] {
        for selector in ["Gemspec", BOTH] {
            let out = cli(
                d.path(),
                &["--only", selector, "--stdin", path],
                Some(SOURCE.as_bytes()),
            );
            let expected = if path.ends_with(".gemspec") { 3 } else { 0 };
            assert_eq!(count(&out), expected, "{path}: {out:?}");
            assert_eq!(out.status.code(), Some(i32::from(expected > 0)), "{out:?}");
        }
    }
    for (config, flags, expected) in [
        (
            "",
            vec![
                "--except",
                "Style,Layout,Lint,Naming,Security,Metrics,Bundler",
            ],
            3,
        ),
        ("[cops.Gemspec]\nenabled=false\n", vec![], 0),
        (
            "[cops.Gemspec]\nenabled=false\n",
            vec!["--only", "Gemspec"],
            3,
        ),
        (
            "[cops.Gemspec]\nenabled=false\n[cops.\"Gemspec/AttributeAssignment\"]\nenabled=true\n",
            vec![],
            2,
        ),
        (
            "",
            vec![
                "--only",
                "Gemspec",
                "--except",
                "Gemspec/DuplicatedAssignment",
            ],
            2,
        ),
        ("", vec!["--only", "Gemspec", "--except", "Gemspec"], 0),
        ("exclude=['a.gemspec']\n", vec!["--only", "Gemspec"], 0),
        (
            "[cops.\"Gemspec/AttributeAssignment\"]\nexclude=['a.gemspec']\n",
            vec!["--only", "Gemspec"],
            1,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = flags;
        args.extend(["--stdin", "a.gemspec"]);
        let out = cli(d.path(), &args, Some(SOURCE.as_bytes()));
        assert_eq!(count(&out), expected, "{args:?}: {config}: {out:?}");
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
    }
    for key in [
        "max=1",
        "enforced_style='indexed'",
        "Include=['**/*']",
        "target_ruby_version='3.4'",
        "use_project_index=true",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{}\"]\n{key}\n", NAMES[0]),
        )
        .unwrap();
        assert_eq!(
            cli(d.path(), &["--stdin", "a.gemspec"], Some(b""))
                .status
                .code(),
            Some(2)
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for unknown in ["Gemspec/Futures", "Gemspec/Future"] {
        for selector in ["--only", "--except"] {
            assert_eq!(
                cli(
                    d.path(),
                    &[selector, unknown, "--stdin", "a.gemspec"],
                    Some(b"")
                )
                .status
                .code(),
                Some(2)
            );
        }
    }
}
#[test]
fn both_directives_and_syntax_encoding_resource_io_failures_compose() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for target in ["all", "Gemspec", NAMES[0], NAMES[1]] {
            let bytes = format!("# {prefix}:disable {target}\n{SOURCE}").replace('\n', "\r\n");
            let out = cli(
                d.path(),
                &["--only", "Gemspec", "--stdin", "a.gemspec"],
                Some(bytes.as_bytes()),
            );
            assert_eq!(
                count(&out),
                match target {
                    "Gemspec/AttributeAssignment" => 1,
                    "Gemspec/DuplicatedAssignment" => 2,
                    _ => 0,
                },
                "{out:?}"
            );
        }
    }
    for bytes in [
        format!("{SOURCE}\ndef (").into_bytes(),
        b"# coding: no-such-encoding\nx=1\n".to_vec(),
        format!("{}{}{}", SOURCE, "[".repeat(4100), "]".repeat(4100)).into_bytes(),
    ] {
        for path in ["a.gemspec", "a.rb"] {
            let out = cli(
                d.path(),
                &["--only", "Gemspec", "--stdin", path],
                Some(&bytes),
            );
            assert_eq!(out.status.code(), Some(2), "{out:?}");
            assert_eq!(count(&out), 0, "{out:?}");
        }
    }
    for bytes in [
        format!("\u{feff}{SOURCE}"),
        format!("{SOURCE}__END__\ninvalid ruby {{["),
        format!("# coding: Windows-1252\n{SOURCE}"),
    ] {
        let out = cli(
            d.path(),
            &["--only", BOTH, "--stdin", "a.gemspec"],
            Some(bytes.as_bytes()),
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out), 3, "{out:?}");
    }
    assert_eq!(
        cli(d.path(), &["--only", "Gemspec", "missing.gemspec"], None)
            .status
            .code(),
        Some(2)
    );
    fs::write(d.path().join("a.gemspec"), b"sentinel").unwrap();
    let old = fs::metadata(d.path().join("a.gemspec")).unwrap();
    let out = cli(
        d.path(),
        &["--only", "Gemspec", "--stdin", "a.gemspec", "--fix"],
        Some(SOURCE.as_bytes()),
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
        "{out:?}"
    );
    assert_eq!(fs::read(d.path().join("a.gemspec")).unwrap(), b"sentinel");
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(d.path().join("a.gemspec"))
            .unwrap()
            .modified()
            .unwrap()
    );
}
#[cfg(unix)]
#[test]
fn opaque_alias_and_atomic_readonly_peer_failure_retain_all_detection() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("a.gemspec");
    let bytes = SOURCE.replace("{}", "{ 'x' => 'y' }");
    fs::write(&path, &bytes).unwrap();
    symlink("a.gemspec", d.path().join("gems.locked")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for paths in [["a.gemspec", "gems.locked"], ["gems.locked", "a.gemspec"]] {
        let out = cli(
            d.path(),
            &[
                "--only",
                "Gemspec,Style/StringLiterals",
                "--fix",
                paths[0],
                paths[1],
            ],
            None,
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out), 3);
        assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap(), old.permissions()),
            (new.ino(), new.modified().unwrap(), new.permissions())
        );
    }
    fs::remove_file(d.path().join("gems.locked")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = cli(
        d.path(),
        &[
            "--only",
            "Gemspec,Style/StringLiterals",
            "--fix",
            "a.gemspec",
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(count(&out), 3);
    assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
}
#[test]
fn native_ruby34_checks_every_reference_source_without_execution() {
    use std::{io::Write, process::Stdio};
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
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
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
            row.get("parser_errors").is_none(),
            "{row}: {out:?}"
        );
    }
}
