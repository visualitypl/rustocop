//! Default no-index CLI/pipeline controls. Indexed parity is private.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;
const COP: &str = "Lint/NameTypo";
const SOURCE: &str = "module Services\n  class UserCreator; VALUE = 1; end\nend\nclass Report\n  def self.generate_summary; end\nend\n::Services::UserCraetor.new\n::Report.generate_sumary\n";
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
    let mut held = fs::File::open(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}");
    assert_eq!(count(&out), 0, "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    let mut held_bytes = Vec::new();
    held.read_to_end(&mut held_bytes).unwrap();
    assert_eq!(held_bytes, source);
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!((old.ino(), old.mode()), (new.ino(), new.mode()));
    }
    out
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/name_typo.json")).unwrap()
}
fn bytes(row: &serde_json::Value) -> Vec<u8> {
    row["source"]
        .as_str()
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_else(|| crate::support::fixture::value(&row["source_bytes"]))
}
#[test]
fn registered_metadata_and_default_selection_do_not_supply_index() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry
        .cops()
        .iter()
        .find(|c| c.name() == COP)
        .expect("real matcher registered");
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
    assert_eq!(
        cop.ruby_versions(),
        rustocop::cops::policy::RubyVersions::default()
    );
    let dir = project_with_lib();
    for (text, args, selected) in [
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
        fs::write(dir.path().join("rustocop.toml"), text).unwrap();
        let mut cli = args.clone();
        if cli.is_empty() {
            cli.extend(["--except", "Style,Naming,Layout,Security"]);
        }
        cli.extend(["--fix", "lib/a.rb"]);
        unchanged(dir.path(), SOURCE.as_bytes(), &cli, 0);
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
}
#[test]
fn every_reference_source_without_index_in_pipeline_and_rebuilt_cli() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let dir = project_with_lib();
    let rows = rows();
    assert_eq!(rows.iter().filter(|r| r["origin"] == "spec").count(), 35);
    for row in rows {
        let source = bytes(&row);
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
fn strict_toml_defaults_and_existing_cops_are_preserved() {
    let dir = project_with_lib();
    unchanged(dir.path(), SOURCE.as_bytes(), &["lib/a.rb"], 1);
    let out = unchanged(
        dir.path(),
        format!("{SOURCE}binding.pry\n").as_bytes(),
        &["--only", "Lint/NameTypo,Lint/Debugger", "lib/a.rb"],
        1,
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Lint/Debugger"));
    for key in [
        "max = 1",
        "UseProjectIndex = true",
        "CheckMethods = true",
        "AllowedNames = []",
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
fn supplied_paths_stdin_exclusions_and_no_hidden_discovery() {
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
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&sentinel).unwrap().modified().unwrap()
    );
    assert_eq!(
        old.permissions(),
        fs::metadata(&sentinel).unwrap().permissions()
    );
    fs::write(
        dir.path().join("lib/definitions.rb"),
        "class Report; def self.generate_summary; end; end\n",
    )
    .unwrap();
    unchanged(
        dir.path(),
        b"Report.generate_sumary\n",
        &["--only", COP, "--fix", "lib/a.rb", "lib/definitions.rb"],
        0,
    );
}
#[test]
fn both_prefixes_multiline_next_push_and_explicit_enable_never_create_an_index() {
    let dir = project_with_lib();
    for prefix in ["rubocop", "rustocop"] {
        for directive in [
            format!("# {prefix}:disable Lint\n"),
            format!("# {prefix}:disable-next {COP}\n"),
            format!("# {prefix}:push -{COP}\n"),
        ] {
            let source = format!(
                "{directive}Report.\n  generate_sumary\nServices::\n  UserCraetor\n# {prefix}:pop\n"
            );
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
                format!("# {prefix}:enable {COP}\n"),
                format!("# {prefix}:enable-next {COP}\n"),
                format!("# {prefix}:push +{COP}\n"),
            ] {
                let source = format!("{directive}Report.\n  generate_sumary\n");
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
fn observers_heredocs_and_fatal_parser_gates_preserve_bytes() {
    let dir = project_with_lib();
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
        format!("x = <<~TEXT\nReport.generate_sumary\nTEXT\n{SOURCE}").into_bytes(),
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
}
#[test]
#[cfg(unix)]
fn readonly_symlinks_and_inodes_are_untouched() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project_with_lib();
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
    assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.mode(), old.modified().unwrap()),
        (new.ino(), new.mode(), new.modified().unwrap())
    );
    assert!(
        fs::symlink_metadata(dir.path().join("lib/alias.rb"))
            .unwrap()
            .is_symlink()
    );
}
#[test]
fn composition_preserves_existing_atomic_layout_and_style_edits() {
    let dir = project_with_lib();
    let source = format!("{SOURCE}consume(1,2)\nthing.kind_of?(Thing)\n");
    let both = "Lint/NameTypo,Layout/SpaceAfterComma,Style/ClassCheck";
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", both, "lib/a.rb"],
        1,
    );
    let out = run(dir.path(), &["--only", both, "--fix", "lib/a.rb"]);
    assert!(out.status.success(), "{out:?}");
    let expected = source.replace("1,2", "1, 2").replace("kind_of?", "is_a?");
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
fn peer_atomic_failure_preserves_readonly_symlink_target() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        let dir = project_with_lib();
        let path = dir.path().join("lib/a.rb");
        let source = format!("{SOURCE}consume(1,2)\n");
        fs::write(&path, &source).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(
            dir.path(),
            &[
                "--only",
                "Lint/NameTypo,Layout/SpaceAfterComma",
                "--fix",
                "lib/alias.rb",
            ],
        );
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), 0);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.mode(), old.modified().unwrap()),
            (new.ino(), new.mode(), new.modified().unwrap())
        );
        assert!(
            fs::symlink_metadata(dir.path().join("lib/alias.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
#[test]
fn native_ruby34_syntax_only_for_every_inspected_source() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-W0", "-c", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes(&row)).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
    }
}
