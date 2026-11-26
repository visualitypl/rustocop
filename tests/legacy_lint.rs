//! real fixed-target pipeline/CLI gates, not a legacy target knob.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::project_with_lib;
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, select_active},
    pipeline::lint_one,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const IT: &str = "Lint/ItWithoutArgumentsInBlock";
const ELSE: &str = "Lint/UselessElseWithoutRescue";
const BOTH: &str = "Lint/ItWithoutArgumentsInBlock,Lint/UselessElseWithoutRescue";

fn cli(root: &Path, args: &[&str], input: Option<&[u8]>, code: i32) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(&format!("({IT})")) && !stdout.contains(&format!("({ELSE})")),
        "{out:?}"
    );
    out
}
fn unchanged(path: &Path, bytes: &[u8], old: &fs::Metadata) {
    let new = fs::metadata(path).unwrap();
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}

#[test]
fn review_implicit_else_recovery_never_reaches_production_dispatch() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "def foo; work; else; other; end",
        "def self.foo; work; else; other; end",
        "class Foo; work; else; other; end",
        "class << self; work; else; other; end",
        "module Foo; work; else; other; end",
        "foo do; work; else; other; end",
    ] {
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        for args in [
            vec!["--only", ELSE, "lib/a.rb"],
            vec!["--only", ELSE, "--fix", "lib/a.rb"],
            vec!["--except", "Lint,Style,Layout,Security,Naming", "lib/a.rb"],
        ] {
            cli(dir.path(), &args, None, 2);
            unchanged(&path, source.as_bytes(), &old);
        }
        cli(
            dir.path(),
            &["--only", ELSE, "--stdin", "lib/a.rb"],
            Some(source.as_bytes()),
            2,
        );
    }
}

#[test]
fn all_pinned_examples_use_fixed_parser_lifecycle_and_never_write() {
    let dir = project_with_lib();
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    let active = select_active(&registry, &config, &[IT.into(), ELSE.into()], &[]);
    assert!(active.is_empty());
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/legacy_lint.json")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 26);
    let path = dir.path().join("lib/a.rb");
    for row in rows.as_array().unwrap() {
        let source = row["source"].as_str().unwrap().as_bytes();
        let report = lint_one(&path, Path::new("lib/a.rb"), Some(source), &active, &config);
        assert_eq!(
            !report.errors.is_empty(),
            row["fixed_parser_error"].as_bool().unwrap(),
            "{row}: {report:?}"
        );
        assert!(report.internal_errors.is_empty() && report.result.offenses.is_empty());
        if row["cop"] == ELSE && row["fixed_parser_error"] == true {
            let expected_line = if row["origin"] == "spec" { 3 } else { 4 };
            assert_eq!(report.errors[0].position, Some((expected_line, 1)));
        }
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let code = if report.errors.is_empty() { 0 } else { 2 };
        for fix in [false, true] {
            let mut args = vec!["--only", BOTH];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            cli(dir.path(), &args, None, code);
            unchanged(&path, source, &old);
        }
        cli(
            dir.path(),
            &["--only", BOTH, "--stdin", "lib/virtual.rb"],
            Some(source),
            code,
        );
    }
}

#[test]
fn real_selection_config_precedence_and_source_force_scopes_cannot_bypass_versions() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let source = b"items.each { it }\n";
    fs::write(&path, source).unwrap();
    cli(dir.path(), &["lib/a.rb"], None, 0);
    for name in [IT, ELSE] {
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        for settings in [
            String::new(),
            format!("[cops.\"{name}\"]\nenabled = true\n"),
            format!("[cops.\"{name}\"]\nenabled = false\n"),
            format!("[cops.Lint]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
            format!("[cops.Lint]\nenabled = true\n[cops.\"{name}\"]\nenabled = false\n"),
            format!("[cops.\"{name}\"]\nexclude = ['lib/**']\n"),
            "exclude = ['lib/**']\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), &settings).unwrap();
            let registry = Registry::new();
            let config = ResolvedConfig::resolve(
                toml::from_str(&settings).unwrap(),
                Some(&dir.path().join("rustocop.toml")),
                &registry,
            )
            .unwrap();
            for (only, except) in [
                (vec![name.into()], vec![]),
                (vec![name.into()], vec![name.into()]),
                (vec!["Lint".into()], vec!["Lint".into()]),
            ] {
                assert!(
                    !select_active(&registry, &config, &only, &except)
                        .iter()
                        .any(|c| c.name() == name)
                );
            }
            cli(dir.path(), &["--only", name, "--fix", "lib/a.rb"], None, 0);
            cli(
                dir.path(),
                &["--only", name, "--except", "Lint", "lib/a.rb"],
                None,
                0,
            );
            unchanged(&path, source, &old);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{name}\"]\nenabled = false\n"),
        )
        .unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for directives in [
                format!("# {prefix}:enable {name}\nitems.each {{ it }}\n"),
                format!("# {prefix}:enable-next {name}\nitems.each do\n it\nend\n"),
                format!("# {prefix}:next +{name}\nitems.each {{ it }}\n"),
                format!("# {prefix}:push +{name}\nitems.each {{ it }}\n# {prefix}:pop\n"),
                format!(
                    "# {prefix}:disable {name}\nitems.each {{ it }}\n# {prefix}:enable {name}\nitems.each {{ it }}\n"
                ),
            ] {
                fs::write(&path, &directives).unwrap();
                let old = fs::metadata(&path).unwrap();
                cli(dir.path(), &["--only", BOTH, "--fix", "lib/a.rb"], None, 0);
                // No --only: genuinely exercise configured-off source opt-ins,
                // rather than only the force-selected enabled candidate list.
                cli(
                    dir.path(),
                    &[
                        "--except",
                        "Style,Layout,Security,Naming",
                        "--fix",
                        "lib/a.rb",
                    ],
                    None,
                    0,
                );
                cli(
                    dir.path(),
                    &["--only", BOTH, "--stdin", "lib/a.rb"],
                    Some(directives.as_bytes()),
                    0,
                );
                unchanged(&path, directives.as_bytes(), &old);
            }
        }
    }
}

#[test]
fn no_write_byte_controls_parser_failures_strict_keys_stdin_and_supplied_paths() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for source in [
        b"\xef\xbb\xbfitems.each { it }\r\n".as_slice(),
        b"# encoding: ISO-8859-1\n# \xe9\nitems.each { it }\n",
        b"items.each { it }\n__LINE__\n__END__\npayload\n",
        b"items.each { it }\ntext = <<~TEXT\n  it\n  else\nTEXT\n",
        b"items.each { it }; binding; File.read(__FILE__)\n",
        b"begin\n work\nelse\n other\nend\n",
    ] {
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let invalid = source.starts_with(b"begin");
        for args in [
            vec!["--only", BOTH, "lib/a.rb"],
            vec!["--only", BOTH, "--fix", "lib/a.rb"],
            vec![
                "--except",
                "Lint,Style,Layout,Security,Naming",
                "--fix",
                "lib/a.rb",
            ],
        ] {
            let out = cli(dir.path(), &args, None, if invalid { 2 } else { 0 });
            if invalid {
                assert!(
                    String::from_utf8_lossy(&out.stdout)
                        .contains("lib/a.rb: error: syntax error on line 3:"),
                    "{out:?}"
                );
            }
            unchanged(&path, source, &old);
        }
    }
    cli(
        dir.path(),
        &["--only", BOTH, "--stdin", "lib/a.rb", "--fix"],
        None,
        2,
    );
    cli(
        dir.path(),
        &["--only", BOTH, "missing.rb", "--fix"],
        None,
        2,
    );
    for setting in [
        "target_ruby = '2.5'\n".to_owned(),
        format!("[cops.\"{IT}\"]\ninclude = ['**']\n"),
        format!("[cops.\"{ELSE}\"]\nmax = 1\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), setting).unwrap();
        cli(dir.path(), &["--only", BOTH, "lib/a.rb"], None, 2);
    }
}

#[cfg(unix)]
#[test]
fn symlink_and_readonly_detection_preserve_actual_write_identity_and_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let alias = dir.path().join("alias.rb");
    let source = b"items.each { it }\n";
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("lib/a.rb", &alias).unwrap();
    let old = fs::metadata(&path).unwrap();
    cli(
        dir.path(),
        &["--only", BOTH, "--fix", "alias.rb", "lib/a.rb"],
        None,
        0,
    );
    unchanged(&path, source, &old);
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
}
