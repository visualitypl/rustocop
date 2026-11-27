//! Real rebuilt CLI/pipeline, opt-in and absent assigned-index gate.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;
const COP: &str = "Lint/UnusedPrivateMethod";
const SOURCE: &str = "class Service\n  private\n\n  def helper\n  end\nend\n";
/// `expect_early_error` only for runs whose asserted exit is the early
/// `--stdin --fix` rejection, which may exit before reading stdin.
fn stdin(root: &Path, source: &[u8], args: &[&str], expect_early_error: bool) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    stdin_support::write_stdin(child.stdin.take().unwrap(), source, expect_early_error).unwrap();
    child.wait_with_output().unwrap()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let before = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains(COP),
        "{out:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source);
    let after = fs::metadata(path).unwrap();
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(before.permissions(), after.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(before.ino(), after.ino());
    }
    out
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/unused_private_method.json")).unwrap()
}
fn source_bytes(row: &serde_json::Value) -> Vec<u8> {
    row["source"]
        .as_str()
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_else(|| crate::support::fixture::value(&row["source_bytes"]))
}
#[test]
fn every_reference_source_has_no_production_index_even_when_forced_or_fixing() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let dir = project_with_lib();
    let rows = rows();
    assert_eq!(rows.len(), 803);
    for row in rows {
        let source = source_bytes(&row);
        let report = rustocop::pipeline::lint_one(
            Path::new("/lib/current.rb"),
            Path::new("/lib/current.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}\n{report:?}"
        );
        assert!(report.result.offenses.is_empty(), "{row}");
        for fix in [false, true] {
            let mut args = vec!["--only", COP, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            unchanged(dir.path(), &source, &args, 0);
        }
    }
}
#[test]
fn actual_opt_in_metadata_and_selection_are_separate_from_missing_index() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(!cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
    assert_eq!(
        cop.ruby_versions(),
        rustocop::cops::policy::RubyVersions::default()
    );
    let dir = project_with_lib();
    for (text, only, except, selected) in [
        (String::new(), vec![], vec![], false),
        (String::new(), vec![COP], vec![], true),
        (String::new(), vec!["Lint"], vec![], true),
        (String::new(), vec![COP], vec![COP], false),
        (String::new(), vec!["Lint"], vec![COP], false),
        ("[cops.Lint]\nenabled = true\n".into(), vec![], vec![], true),
        (
            "[cops.Lint]\nenabled = false\n".into(),
            vec![],
            vec![],
            false,
        ),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            vec![],
            true,
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            vec![],
            false,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec![COP],
            vec![],
            true,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), text).unwrap();
        let (path, raw) = rustocop::config::find_config_from(dir.path())
            .unwrap()
            .unwrap();
        let config =
            rustocop::config::ResolvedConfig::resolve(raw, Some(&path), &registry).unwrap();
        let active = rustocop::cops::select_active(
            &registry,
            &config,
            &only.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &except.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        );
        assert_eq!(active.iter().any(|c| c.name() == COP), selected);
        let mut args = vec![
            "--except",
            "Style,Naming,Layout,Security",
            "--fix",
            "lib/a.rb",
        ];
        if let Some(only) = only.first() {
            args.extend(["--only", only]);
        }
        if let Some(except) = except.first() {
            args.extend(["--except", except]);
        }
        unchanged(dir.path(), SOURCE.as_bytes(), &args, 0);
    }
}
#[test]
fn strict_toml_rejects_index_style_and_unsupported_settings_without_writes() {
    let dir = project_with_lib();
    for key in [
        "max = 1",
        "UseProjectIndex = true",
        "AllowedNames = ['helper']",
        "AllowedPatterns = ['.*']",
        "Include = ['**/*']",
        "enabled = 'true'",
        "exclude = ['[']",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            2,
        );
    }
}
#[test]
fn exclusions_supplied_paths_and_stdin_provenance_never_provide_an_index() {
    let dir = project_with_lib();
    let sentinel = dir.path().join("lib/display.rb");
    fs::write(&sentinel, "sentinel\n").unwrap();
    let old = fs::metadata(&sentinel).unwrap();
    for text in [
        String::new(),
        "exclude = ['lib/*.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/*.rb']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), text).unwrap();
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            0,
        );
        assert!(
            run(&dir.path().join("lib"), &["--only", COP, "a.rb"])
                .status
                .success()
        );
        let out = stdin(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--stdin", "lib/display.rb"],
            false,
        );
        assert!(out.status.success(), "{out:?}");
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/source.arb"), SOURCE).unwrap();
    assert!(
        run(dir.path(), &["--only", COP, "lib/source.arb"])
            .status
            .success()
    );
    let out = stdin(
        dir.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--stdin", "lib/display.rb", "--fix"],
        true,
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
        "{out:?}"
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel\n");
    let new = fs::metadata(sentinel).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}
#[test]
fn both_directive_prefixes_multiline_next_signed_and_push_scopes_keep_gate() {
    let dir = project_with_lib();
    for prefix in ["rubocop", "rustocop"] {
        for directive in [
            format!("# {prefix}:disable Lint\n"),
            format!("# {prefix}:disable-next {COP}\n"),
            format!("# {prefix}:push -{COP}\n"),
        ] {
            let source = SOURCE.replace(
                "  def helper\n",
                &format!("{directive}  def helper(\n    value = nil\n  )\n"),
            );
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        for target in ["Lint", COP] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{target}\"]\nenabled = false\n"),
            )
            .unwrap();
            for directive in [
                format!("# {prefix}:enable {COP}\n"),
                format!("# {prefix}:enable-next {COP}\n"),
                format!("# {prefix}:push +{COP}\n"),
            ] {
                let source =
                    SOURCE.replace("  def helper\n", &format!("{directive}  def helper\n"));
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &[
                        "--except",
                        "Style,Naming,Layout,Security",
                        "--fix",
                        "lib/a.rb",
                    ],
                    0,
                );
            }
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn byte_observers_encoding_and_parser_errors_are_not_fix_opportunities() {
    let dir = project_with_lib();
    for source in [format!("\u{feff}# coding: utf-8\r\n{}", SOURCE.replace('\n', "\r\n")).into_bytes(),
        [b"# coding: Windows-1252\np '\xe9'\n".as_slice(), SOURCE.as_bytes()].concat(),
        format!("{SOURCE}x = <<~TEXT\nhelper\nTEXT\n").into_bytes(),
        format!("{SOURCE}File.binread(__FILE__); __LINE__; DATA.pos; RubyVM; ObjectSpace\n__END__\nraw\0\n").into_bytes()] {
        unchanged(dir.path(), &source, &["--only", COP, "--fix", "lib/a.rb"], 0);
    }
    unchanged(
        dir.path(),
        format!("{SOURCE}def broken(\n").as_bytes(),
        &["--only", COP, "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "missing.rb"])
            .status
            .code(),
        Some(2)
    );
    fs::write(dir.path().join("lib/other.rb"), "obj.helper\n").unwrap();
    unchanged(
        dir.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--fix", "lib/a.rb", "lib/other.rb"],
        0,
    );
}
#[test]
#[cfg(unix)]
fn readonly_symlink_permissions_and_inode_are_unchanged() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, SOURCE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
    let before = fs::metadata(&path).unwrap();
    assert!(
        run(
            dir.path(),
            &["--only", COP, "--fix", "lib/a.rb", "lib/alias.rb"]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    let after = fs::metadata(&path).unwrap();
    assert_eq!(before.ino(), after.ino());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(before.permissions(), after.permissions());
    assert!(
        fs::symlink_metadata(dir.path().join("lib/alias.rb"))
            .unwrap()
            .is_symlink()
    );
}
#[test]
fn no_fix_never_writes_but_existing_style_layout_atomic_corrections_still_compose() {
    let dir = project_with_lib();
    let source = format!("{SOURCE}consume(1,2);\n");
    let both = "Lint/UnusedPrivateMethod,Layout/SpaceAfterComma,Style/Semicolon";
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", both, "lib/a.rb"],
        1,
    );
    let out = run(dir.path(), &["--only", both, "--fix", "lib/a.rb"]);
    assert!(out.status.success(), "{out:?}");
    let expected = source.replace("1,2", "1, 2").replace(");\n", ")\n");
    assert_eq!(
        fs::read(dir.path().join("lib/a.rb")).unwrap(),
        expected.as_bytes()
    );
    unchanged(
        dir.path(),
        expected.as_bytes(),
        &["--only", both, "--fix", "lib/a.rb"],
        0,
    );
}
#[test]
fn native_ruby34_syntax_only_all_original_reference_inputs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-c", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&source_bytes(&row))
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
    }
}
