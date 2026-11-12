//! genuine no-index prerequisite, rebuilt CLI and no-write controls.
//! Indexed detection is tested directly in the cop, not claimed as default parity.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

#[path = "support/stdin.rs"]
mod stdin_support;
const COP: &str = "Lint/DeprecatedReference";
const SOURCE: &str = "class Base\n  # @deprecated Use modern instead.\n  def old(first); end\nend\nclass Child < Base\n  def run(first)\n    self.old(first)\n  end\nend\n";
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
        .lines()
        .filter(|l| l.contains(COP))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}");
    assert_eq!(count(&out), 0, "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}
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
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/deprecated_reference.json")).unwrap()
}
#[test]
fn all_reference_sources_without_assigned_index_have_no_pipeline_or_cli_findings() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let dir = project();
    let rows = rows();
    assert_eq!(rows.len(), 601);
    for row in rows {
        let source = row["source"].as_str().unwrap().as_bytes();
        let report = rustocop::pipeline::lint_one(
            Path::new("child.rb"),
            Path::new("child.rb"),
            Some(source),
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
            unchanged(dir.path(), source, &args, 0);
        }
    }
}
#[test]
fn metadata_and_selection_remain_separate_from_input_prerequisite() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(
        cop.ruby_versions(),
        rustocop::cops::policy::RubyVersions::default()
    );
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
    let dir = project();
    for (config_text, args, selected) in [
        (String::new(), vec![], true),
        (String::new(), vec!["--only", COP], true),
        (
            String::new(),
            vec!["--only", "Lint", "--except", COP],
            false,
        ),
        (String::new(), vec!["--only", COP, "--except", COP], false),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], false),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            true,
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            false,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            true,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config_text).unwrap();
        let mut cli = args.clone();
        if cli.is_empty() {
            cli.extend(["--except", "Style,Naming,Layout,Security"]);
        }
        cli.extend(["--fix", "lib/a.rb"]);
        unchanged(dir.path(), SOURCE.as_bytes(), &cli, 0);
        // Confirm selection itself, independently of conditional inactivity.
        let (path, raw) = rustocop::config::find_config_from(dir.path())
            .unwrap()
            .unwrap();
        let config =
            rustocop::config::ResolvedConfig::resolve(raw, Some(&path), &registry).unwrap();
        let only: Vec<String> = args
            .windows(2)
            .find(|a| a[0] == "--only")
            .map(|a| vec![a[1].into()])
            .unwrap_or_default();
        let except: Vec<String> = args
            .windows(2)
            .find(|a| a[0] == "--except")
            .map(|a| vec![a[1].into()])
            .unwrap_or_default();
        let active = rustocop::cops::select_active(&registry, &config, &only, &except);
        assert_eq!(active.iter().any(|c| c.name() == COP), selected);
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let out = unchanged(dir.path(), SOURCE.as_bytes(), &["lib/a.rb"], 1);
    assert!(!out.stdout.is_empty()); // Other departments still run normally.
    let out = unchanged(
        dir.path(),
        format!("{SOURCE}binding.pry\n").as_bytes(),
        &[
            "--only",
            "Lint/DeprecatedReference,Lint/Debugger",
            "lib/a.rb",
        ],
        1,
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Lint/Debugger"));
    for key in [
        "max = 1",
        "UseProjectIndex = true",
        "include = ['**/*']",
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
fn excludes_and_supplied_file_stdin_path_provenance() {
    let dir = project();
    let sentinel = dir.path().join("lib/display.rb");
    fs::write(&sentinel, "sentinel\n").unwrap();
    let old = fs::metadata(&sentinel).unwrap();
    for config in [
        String::new(),
        "exclude = ['lib/*.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/*.rb']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
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
        assert_eq!(count(&out), 0);
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
fn both_directive_prefixes_multiline_next_push_and_forced_enable_do_not_supply_index() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for directive in [
            format!("# {prefix}:disable Lint\n"),
            format!("# {prefix}:disable-next {COP}\n"),
            format!("# {prefix}:push -{COP}\n"),
        ] {
            let source = SOURCE.replace(
                "    self.old(first)\n",
                &format!("{directive}    self.old(\n      first\n    )\n"),
            );
            let source = format!("{source}# {prefix}:pop\n");
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        for disabled in ["Lint", COP] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{disabled}\"]\nenabled = false\n"),
            )
            .unwrap();
            for directive in [
                format!("# {prefix}:enable-next {COP}\n"),
                format!("# {prefix}:push +{COP}\n"),
            ] {
                let source = SOURCE.replace(
                    "    self.old(first)\n",
                    &format!("{directive}    self.old(\n      first\n    )\n"),
                );
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
fn encoding_observers_and_parser_failures_preserve_source() {
    let dir = project();
    for source in [
        format!(
            "\u{feff}# encoding: utf-8\r\n{}",
            SOURCE.replace('\n', "\r\n")
        )
        .into_bytes(),
        [
            b"# encoding: Windows-1252\np '\xe9'\n".as_slice(),
            SOURCE.as_bytes(),
        ]
        .concat(),
        format!("x = <<~TEXT\nself.old(first)\nTEXT\n{SOURCE}").into_bytes(),
        format!("{SOURCE}File.binread(__FILE__); DATA.pos; __LINE__\n__END__\nraw\0\n")
            .into_bytes(),
        format!(
            "{SOURCE}RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {{}}\n"
        )
        .into_bytes(),
    ] {
        unchanged(
            dir.path(),
            &source,
            &["--only", COP, "--fix", "lib/a.rb"],
            0,
        );
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
    fs::write(
        dir.path().join("lib/base.rb"),
        "class Base\n # @deprecated\n def old(a); end\nend\n",
    )
    .unwrap();
    unchanged(
        dir.path(),
        b"class Child < Base; def run(a); old(a); end; end\n",
        &["--only", COP, "--fix", "lib/a.rb", "lib/base.rb"],
        0,
    );
}
#[test]
#[cfg(unix)]
fn readonly_symlink_target_and_permissions_are_unchanged() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, SOURCE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(
        dir.path(),
        &["--only", COP, "--fix", "lib/alias.rb", "lib/a.rb"],
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(count(&out), 0);
    assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    assert!(
        fs::symlink_metadata(dir.path().join("lib/alias.rb"))
            .unwrap()
            .is_symlink()
    );
}
#[test]
fn detection_only_consumer_composes_with_existing_atomic_source_corrections() {
    let dir = project();
    let source = format!("{SOURCE}consume(1,2)\n");
    let both = "Lint/DeprecatedReference,Layout/SpaceAfterComma";
    let out = unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", both, "lib/a.rb"],
        1,
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Layout/SpaceAfterComma"));
    let out = run(dir.path(), &["--only", both, "--fix", "lib/a.rb"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(count(&out), 0);
    let expected = source.replace("1,2", "1, 2");
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
fn native_ruby34_syntax_only_reference_controls() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
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
            .write_all(row["source"].as_str().unwrap().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
    }
}
