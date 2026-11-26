//! actual fixed3.4 pipeline, rebuilt CLI, mandatory gates/no writes.
//! RuboCop examples: MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::project_with_lib;
use rustocop::{
    config::ResolvedConfig,
    cops::{
        Registry,
        policy::{RubyVersion, RubyVersions},
        select_active,
    },
    pipeline::lint_one,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/NonDeterministicRequireOrder";
const SOURCE: &[u8] = b"Dir[\"*.rb\"].each do |file|\n  require file\nend\n";

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
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains(&format!("({COP})")),
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
fn registration_is_real_but_fixed_target_selection_is_unforceably_inactive() {
    let registry = Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert!(cop.path_policy().is_none()); // exact pinned Include/Exclude: empty
    assert_eq!(
        cop.ruby_versions(),
        RubyVersions {
            minimum: None,
            maximum: Some(RubyVersion(2, 7))
        }
    );
    for (only, except) in [
        (vec![], vec![]),
        (vec![COP.into()], vec![]),
        (vec!["Lint".into()], vec![]),
        (vec![COP.into()], vec!["Lint".into()]),
    ] {
        assert!(
            !select_active(&registry, &ResolvedConfig::default(), &only, &except)
                .iter()
                .any(|c| c.name() == COP)
        );
    }
}

#[test]
fn all_pinned_and_controlled_sources_run_through_real_pipeline_file_cli_and_stdin_without_edits() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    let active = select_active(&registry, &config, &[COP.into()], &[]);
    assert!(active.is_empty());
    let rows: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/non_deterministic_require_order.json"
    ))
    .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 126);
    for row in rows.as_array().unwrap() {
        let source = row["source"].as_str().unwrap().as_bytes();
        let report = lint_one(&path, Path::new("lib/a.rb"), Some(source), &active, &config);
        assert!(
            report.errors.is_empty()
                && report.internal_errors.is_empty()
                && report.result.offenses.is_empty(),
            "{row}: {report:?}"
        );
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            cli(dir.path(), &args, None, 0);
            unchanged(&path, source, &old);
        }
        cli(
            dir.path(),
            &["--only", COP, "--stdin", "lib/virtual.rb"],
            Some(source),
            0,
        );
    }
}

#[test]
fn overlapping_start_legacy_findings_remain_inactive_and_never_write_in_production() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for require in ["require", "require_relative"] {
        let source = format!("Dir.glob('*.rb', &method(:{require})).each {{ |f| {require} f }}\n");
        fs::write(&path, &source).unwrap();
        let old = fs::metadata(&path).unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            cli(dir.path(), &args, None, 0);
            unchanged(&path, source.as_bytes(), &old);
        }
        cli(
            dir.path(),
            &["--only", COP, "--stdin", "lib/virtual.rb"],
            Some(source.as_bytes()),
            0,
        );
        unchanged(&path, source.as_bytes(), &old);
    }
}

#[test]
fn config_department_selectors_paths_and_all_source_force_scopes_preserve_the_gate() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, SOURCE).unwrap();
    cli(dir.path(), &["lib/a.rb"], None, 0);
    let old = fs::metadata(&path).unwrap();
    for settings in [
        String::new(),
        format!("[cops.\"{COP}\"]\nenabled = true\n"),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
        format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
        format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
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
        assert!(
            !select_active(&registry, &config, &[], &[])
                .iter()
                .any(|c| c.name() == COP)
        );
        for args in [
            vec!["--only", COP, "--fix", "lib/a.rb"],
            vec!["--only", COP, "--except", "Lint", "lib/a.rb"],
        ] {
            cli(dir.path(), &args, None, 0);
            unchanged(&path, SOURCE, &old);
        }
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    )
    .unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!(
                "# {prefix}:enable {COP}\n{}",
                String::from_utf8_lossy(SOURCE)
            ),
            format!(
                "# {prefix}:enable-next {COP}\n{}",
                String::from_utf8_lossy(SOURCE)
            ),
            format!(
                "# {prefix}:next +{COP}\n{}",
                String::from_utf8_lossy(SOURCE)
            ),
            format!(
                "# {prefix}:push +{COP}\n{}# {prefix}:pop\n",
                String::from_utf8_lossy(SOURCE)
            ),
            format!(
                "# {prefix}:disable {COP}\n{}# {prefix}:enable {COP}\n{}",
                String::from_utf8_lossy(SOURCE),
                String::from_utf8_lossy(SOURCE)
            ),
            format!("Dir.glob(\n  '*.rb', # {prefix}:enable {COP}\n  &method(:require)\n)\n"),
        ] {
            fs::write(&path, &source).unwrap();
            let old = fs::metadata(&path).unwrap();
            // No --only: genuinely exercise the configured-off source-opt-in list.
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
            cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], None, 0);
            cli(
                dir.path(),
                &["--only", COP, "--stdin", "lib/a.rb"],
                Some(source.as_bytes()),
                0,
            );
            unchanged(&path, source.as_bytes(), &old);
        }
    }
    for name in ["Rakefile", "spec/a.rb", "a.jb", "a.txt"] {
        cli(
            dir.path(),
            &["--only", COP, "--stdin", name],
            Some(SOURCE),
            0,
        );
    }
}

#[test]
fn source_envelopes_strict_errors_fatal_parser_and_stdin_fix_never_write() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for source in [
        b"\xef\xbb\xbfDir['*.rb'].each { |file| require file }\r\n".as_slice(),
        b"# encoding: ISO-8859-1\n# \xe9\nDir['*.rb'].each { |file| require file }\n",
        b"Dir['*.rb'].each { |file| require file }; __LINE__\n__END__\npayload\0\n",
        b"Dir['*.rb'].each { |file| require file }; text = <<~TEXT\n  Dir.glob('*.rb')\nTEXT\n",
        b"Dir['*.rb'].each { |file| require file }; binding; File.read(__FILE__); RubyVM::InstructionSequence; ObjectSpace\n",
        b"Dir['*.rb'].each do |file|\n require file\n",
    ] {
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let code = if source.ends_with(b"require file\n") { 2 } else { 0 };
        for args in [vec!["--only", COP, "lib/a.rb"], vec!["--only", COP, "--fix", "lib/a.rb"], vec!["--except", "Lint,Style,Layout,Security,Naming", "--fix", "lib/a.rb"]] {
            cli(dir.path(), &args, None, code);
            unchanged(&path, source, &old);
        }
        cli(dir.path(), &["--only", COP, "--stdin", "lib/virtual.rb"], Some(source), code);
    }
    cli(
        dir.path(),
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        None,
        2,
    );
    cli(dir.path(), &["--only", COP, "--fix", "missing.rb"], None, 2);
    for settings in [
        "target_ruby = '2.7'\n".to_owned(),
        format!("[cops.\"{COP}\"]\ninclude = ['**']\n"),
        format!("[cops.\"{COP}\"]\nmax = 1\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), settings).unwrap();
        cli(dir.path(), &["--only", COP, "lib/a.rb"], None, 2);
    }
}

#[cfg(unix)]
#[test]
fn readonly_and_symlink_identity_permissions_are_preserved() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let alias = dir.path().join("alias.rb");
    fs::write(&path, SOURCE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("lib/a.rb", &alias).unwrap();
    let old = fs::metadata(&path).unwrap();
    cli(
        dir.path(),
        &["--only", COP, "--fix", "alias.rb", "lib/a.rb"],
        None,
        0,
    );
    unchanged(&path, SOURCE, &old);
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
}
