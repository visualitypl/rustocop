//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::project_with_lib;
use crate::support::stdin;
use rustocop::{
    config::ResolvedConfig,
    cops::{Cop, Registry, lint::MissingCopEnableDirective},
    pipeline::lint_one,
};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/MissingCopEnableDirective";
const TARGET: &str = "Layout/LineLength";
const BEFORE: &str = "# rubocop:disable Layout/LineLength\nx\n";
fn cli(root: &Path, args: &[&str], code: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        // This suite isolates open-range diagnostics from redundant disables.
        .args(["--except", "Lint/RedundantCopDisableDirective"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    out
}
fn config(text: &str) -> ResolvedConfig {
    ResolvedConfig::resolve(toml::from_str(text).unwrap(), None, &Registry::new()).unwrap()
}
fn findings(source: &[u8], config: &ResolvedConfig) -> Vec<(u32, u32, String)> {
    let report = lint_one(
        Path::new("example.rb"),
        Path::new("example.rb"),
        Some(source),
        &[&MissingCopEnableDirective],
        config,
    );
    assert!(
        report.errors.is_empty() && report.internal_errors.is_empty(),
        "{report:?}"
    );
    assert!(
        report
            .result
            .offenses
            .iter()
            .all(|o| o.correction.is_none())
    );
    report
        .result
        .offenses
        .into_iter()
        .map(|o| (o.line, o.col, o.message))
        .collect()
}
fn unchanged(path: &Path, source: &[u8], old: &fs::Metadata) {
    let new = fs::metadata(path).unwrap();
    assert_eq!(fs::read(path).unwrap(), source);
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}

#[test]
fn all_pinned_main_source_and_focused_lifecycle_cases_pipeline_cli_no_write() {
    let d = project_with_lib();
    let path = d.path().join("lib/a.rb");
    for fixture in [
        include_str!("fixtures/missing_cop_enable_directive_pinned.json"),
        include_str!("fixtures/missing_cop_enable_directive_source.json"),
        include_str!("fixtures/missing_cop_enable_directive_controls.json"),
    ] {
        let rows: serde_json::Value = serde_json::from_str(fixture).unwrap();
        for row in rows.as_array().unwrap() {
            let toml = if row["disabled"] == true {
                "[cops.\"Layout/LineLength\"]\nenabled = false\n"
            } else {
                ""
            };
            fs::write(d.path().join("rustocop.toml"), toml).unwrap();
            let config = config(toml);
            let expected: Vec<_> = row["positions"]
                .as_array()
                .unwrap()
                .iter()
                .zip(row["messages"].as_array().unwrap())
                .map(|(p, m)| {
                    (
                        p[0].as_u64().unwrap() as u32,
                        p[1].as_u64().unwrap() as u32,
                        m.as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            for prefix in ["rubocop", "rustocop"] {
                let source = row["source"]
                    .as_str()
                    .unwrap()
                    .replace("rubocop:", &format!("{prefix}:"));
                assert_eq!(
                    findings(source.as_bytes(), &config),
                    expected,
                    "{row:?} ({prefix})"
                );
                fs::write(&path, &source).unwrap();
                let old = fs::metadata(&path).unwrap();
                for fix in [false, true] {
                    let args = if fix {
                        vec!["--only", COP, "--fix", "lib/a.rb"]
                    } else {
                        vec!["--only", COP, "lib/a.rb"]
                    };
                    let out = cli(d.path(), &args, if expected.is_empty() { 0 } else { 1 });
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    for (line, col, message) in &expected {
                        assert!(
                            stdout
                                .contains(&format!("lib/a.rb:{line}:{col}: W: {message} ({COP})")),
                            "{stdout}"
                        );
                    }
                    assert_eq!(
                        stdout
                            .lines()
                            .filter(|l| l.ends_with(&format!("({COP})")))
                            .count(),
                        expected.len(),
                        "{stdout}"
                    );
                    unchanged(&path, source.as_bytes(), &old);
                }
            }
        }
    }
}

#[test]
fn actual_metadata_defaults_selectors_toml_precedence_excludes_and_strict_keys() {
    assert!(MissingCopEnableDirective.default_enabled());
    assert!(!MissingCopEnableDirective.safe_fixes());
    assert!(MissingCopEnableDirective.path_policy().is_none());
    assert!(MissingCopEnableDirective.max_setting().is_none());
    assert_eq!(
        MissingCopEnableDirective.ruby_versions(),
        Default::default()
    );
    let d = project_with_lib();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, BEFORE).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = cli(d.path(), &["lib/a.rb"], 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
    for (toml, args, code, expected) in [
        (
            "",
            vec!["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
            0,
            false,
        ),
        (
            "",
            vec!["--only", "Lint", "--except", "Layout", "--fix", "lib/a.rb"],
            1,
            true,
        ),
        (
            "",
            vec!["--only", COP, "--except", "Lint", "lib/a.rb"],
            0,
            false,
        ),
        (
            "[cops.\"Lint/MissingCopEnableDirective\"]\nenabled = false\n",
            vec!["--only", COP, "lib/a.rb"],
            1,
            true,
        ),
        (
            "[cops.Lint]\nenabled = false\n",
            vec!["--only", COP, "lib/a.rb"],
            1,
            true,
        ),
        (
            "[cops.Lint]\nenabled = false\n[cops.\"Lint/MissingCopEnableDirective\"]\nenabled = true\n",
            vec!["lib/a.rb"],
            1,
            true,
        ),
        (
            "[cops.Lint]\nenabled = true\n[cops.\"Lint/MissingCopEnableDirective\"]\nenabled = false\n",
            vec!["lib/a.rb"],
            0,
            false,
        ),
        (
            "[cops.\"Lint/MissingCopEnableDirective\"]\nexclude = ['lib/**']\n",
            vec!["--only", COP, "--fix", "lib/a.rb"],
            0,
            false,
        ),
        (
            "exclude = ['lib/a.rb']\n",
            vec!["--only", COP, "--fix", "lib/a.rb"],
            0,
            false,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), toml).unwrap();
        let out = cli(d.path(), &args, code);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).contains(&format!("({COP})")),
            expected,
            "{out:?}"
        );
        unchanged(&path, BEFORE.as_bytes(), &old);
    }
    for key in [
        "max = 2",
        "MaxRangeSize = 2",
        "Include = ['**/*']",
        "enabled = 'true'",
        "EnforcedStyle = 'other'",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 2);
        unchanged(&path, BEFORE.as_bytes(), &old);
    }
}

#[test]
fn configured_disabled_exemption_is_independent_of_selection_path_and_version() {
    let d = project_with_lib();
    let path = d.path().join("lib/a.rb");
    for (target, default) in [
        (TARGET, true),
        ("Style/ReverseFind", true),
        ("Security/YAMLLoad", true),
        ("Style/FrozenStringLiteralComment", false),
    ] {
        let department = target.split('/').next().unwrap();
        for (toml, enabled) in [
            (String::new(), default),
            (format!("[cops.\"{target}\"]\nenabled = false\n"), false),
            (
                format!(
                    "[cops.{department}]\nenabled = false\n[cops.\"{target}\"]\nenabled = true\n"
                ),
                true,
            ),
            (
                format!(
                    "[cops.{department}]\nenabled = true\n[cops.\"{target}\"]\nenabled = false\n"
                ),
                false,
            ),
            (
                format!("[cops.\"{target}\"]\nexclude = ['lib/**']\n"),
                default,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), &toml).unwrap();
            for prefix in ["rubocop", "rustocop"] {
                let source =
                    format!("# {prefix}:enable {target}\n# {prefix}:disable {target}\nx\n");
                assert_eq!(
                    findings(source.as_bytes(), &config(&toml)).len(),
                    usize::from(enabled),
                    "{toml}: {source}"
                );
                fs::write(&path, &source).unwrap();
                let old = fs::metadata(&path).unwrap();
                let forced = format!("{COP},{target}");
                for args in [
                    vec!["--only", COP, "--fix", "lib/a.rb"],
                    vec!["--only", &forced, "--except", target, "lib/a.rb"],
                ] {
                    cli(d.path(), &args, i32::from(enabled));
                    unchanged(&path, source.as_bytes(), &old);
                }
            }
        }
    }
}

#[test]
fn both_prefixes_and_ordinary_next_multiline_push_suppression_apply_to_origin_span() {
    let d = project_with_lib();
    let path = d.path().join("lib/a.rb");
    for prefix in ["rubocop", "rustocop"] {
        for (source, expected) in [
            (
                format!("# {prefix}:disable {COP}\n# {prefix}:disable {TARGET}\nx\n"),
                0,
            ),
            (
                format!("# {prefix}:push -{COP}\n# {prefix}:disable {TARGET}\nx\n"),
                0,
            ),
            (
                format!(
                    "# {prefix}:disable-next {COP}\nclass A\n# {prefix}:disable {TARGET}\nx\nend\n"
                ),
                0,
            ),
            (
                format!("# {prefix}:next -{COP}\nclass A\n# {prefix}:disable {TARGET}\nx\nend\n"),
                0,
            ),
            (
                format!("# {prefix}:disable-next {COP}\n\n# {prefix}:disable {TARGET}\nx\n"),
                1,
            ),
            (
                format!(
                    "# {prefix}:disable {COP}\n# {prefix}:enable-next {COP}\nclass A\n# {prefix}:disable {TARGET}\nx\nend\n"
                ),
                1,
            ),
            (
                format!(
                    "# {prefix}:disable {COP}\n# {prefix}:next +{COP}\nclass A\n# {prefix}:disable {TARGET}\nx\nend\n"
                ),
                1,
            ),
        ] {
            assert_eq!(
                findings(source.as_bytes(), &ResolvedConfig::default()).len(),
                expected,
                "{source}"
            );
            fs::write(&path, &source).unwrap();
            let old = fs::metadata(&path).unwrap();
            cli(
                d.path(),
                &["--only", COP, "--fix", "lib/a.rb"],
                expected as i32,
            );
            unchanged(&path, source.as_bytes(), &old);
        }
    }
}

#[test]
fn source_envelopes_remain_detection_only_with_unchanged_bytes_mtime_permissions() {
    let d = project_with_lib();
    let path = d.path().join("lib/a.rb");
    for source in [
        b"\xef\xbb\xbf# rubocop:disable Layout/LineLength\r\nx\r\n".as_slice(),
        b"# encoding: ISO-8859-1\n# rubocop:disable Layout/LineLength -- \xe9\nx\n",
        b"# rubocop:disable Layout/LineLength\nx = <<~TEXT\n  hello\nTEXT\n__LINE__\n",
        b"# rubocop:disable Layout/LineLength\n__END__\nDATA bytes\n",
        b"# rubocop:disable Layout/LineLength\nFile.read(__FILE__)\n",
        b"# rubocop:disable Layout/LineLength\nRubyVM::InstructionSequence.of(method(:foo))\n",
        b"# rubocop:disable Layout/LineLength\nObjectSpace.each_object(String)\n",
        b"# rubocop:disable Layout/LineLength",
    ] {
        assert_eq!(findings(source, &ResolvedConfig::default()).len(), 1);
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            cli(d.path(), &args, 1);
            unchanged(&path, source, &old);
        }
    }
    fs::write(&path, b"# rubocop:disable Layout/LineLength\ndef broken(\n").unwrap();
    let source = fs::read(&path).unwrap();
    let old = fs::metadata(&path).unwrap();
    cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 2);
    unchanged(&path, &source, &old);
}

#[test]
fn supplied_file_stdin_paths_and_stdin_fix_rejection() {
    let d = project_with_lib();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, BEFORE).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = cli(&d.path().join("lib"), &["--only", COP, "a.rb"], 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:1:1: W:"));
    for (args, code) in [
        (vec!["--only", COP, "--stdin", "virtual/custom.rb"], 1),
        (vec!["--only", COP, "--stdin", "lib/a.rb", "--fix"], 2),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // `--stdin --fix` is rejected (exit 2) before stdin is read.
        stdin::write_stdin(child.stdin.take().unwrap(), BEFORE.as_bytes(), code == 2).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(code), "{out:?}");
        if code == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/custom.rb:1:1: W:"));
        } else {
            assert!(out.stdout.is_empty(), "{out:?}");
            assert!(
                String::from_utf8_lossy(&out.stderr)
                    .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                "{out:?}"
            );
        }
        unchanged(&path, BEFORE.as_bytes(), &old);
    }
}

#[cfg(unix)]
#[test]
fn detection_only_fix_preserves_read_only_mode_symlinks_and_targets() {
    use std::os::unix::{
        fs::MetadataExt,
        fs::{PermissionsExt, symlink},
    };
    let d = project_with_lib();
    let path = d.path().join("lib/a.rb");
    let link = d.path().join("lib/link.rb");
    fs::write(&path, BEFORE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", &link).unwrap();
    let old = fs::metadata(&path).unwrap();
    let link_old = fs::symlink_metadata(&link).unwrap();
    cli(
        d.path(),
        &["--only", COP, "--fix", "lib/link.rb", "lib/a.rb"],
        1,
    );
    unchanged(&path, BEFORE.as_bytes(), &old);
    assert_eq!(fs::symlink_metadata(&link).unwrap().ino(), link_old.ino());
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("a.rb"));
}
