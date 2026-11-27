//! real documentation policy, selectors, directives and no-write paths.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use rustocop::{
    config::{RawConfig, ResolvedConfig},
    cops::{Registry, select_active},
    pipeline::lint_one,
};

const COP: &str = "Style/Documentation";
const SOURCE: &str = "class Person\n  def foo; end\nend\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/a.rb"), SOURCE).unwrap();
    dir
}

fn cli(cwd: &Path, args: &[&str], status: i32) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    if status != 2 {
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    output
}

fn stdin(cwd: &Path, path: &str, source: &[u8], status: i32) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(cwd)
        .args(["--only", COP, "--stdin", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    output
}

#[test]
fn real_pipeline_has_exact_default_policy_and_additive_path_exclusions() {
    let registry = Registry::new();
    let matches: Vec<_> = registry.cops().iter().filter(|c| c.name() == COP).collect();
    assert_eq!(matches.len(), 1);
    let cop = matches[0];
    assert!(!cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert!(cop.path_policy().is_some());
    assert_eq!(cop.ruby_versions(), Default::default());
    let raw: RawConfig = toml::from_str("exclude = [\"lib/global.rb\"]\n[cops.\"Style/Documentation\"]\nexclude = [\"lib/local.rb\"]\n").unwrap();
    let config =
        ResolvedConfig::resolve(raw, Some(Path::new("/project/rustocop.toml")), &registry).unwrap();
    let active = select_active(&registry, &config, &[COP.into()], &[]);
    for (path, count) in [
        ("lib/a.rb", 1),
        ("/project/lib/a.rb", 1),
        ("spec/a.rb", 0),
        ("./spec/deep/a.rb", 0),
        ("/project/test/deep/a.rb", 0),
        ("test/a.rb", 0),
        ("specification/a.rb", 1),
        ("tests/a.rb", 1),
        ("lib/global.rb", 0),
        ("lib/local.rb", 0),
    ] {
        let report = lint_one(
            Path::new(path),
            Path::new(path),
            Some(SOURCE.as_bytes()),
            &active,
            &config,
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        assert_eq!(report.result.offenses.len(), count, "{path}");
        for offense in report.result.offenses {
            assert_eq!((offense.line, offense.col, offense.end_line), (1, 1, 1));
            assert_eq!(
                offense.message,
                "Missing top-level documentation comment for `class Person`."
            );
            assert!(offense.correction.is_none());
        }
    }
}

#[test]
fn opt_in_selectors_and_explicit_config_enablement_work_without_writes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    // The cop is opt-in: a run without a selector or config does not report.
    cli(dir.path(), &["lib/a.rb"], 0);
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.\"Style/Documentation\"]\nenabled = true\n",
    )
    .unwrap();
    let output = cli(dir.path(), &["lib/a.rb"], 1);
    assert!(String::from_utf8_lossy(&output.stdout).contains("lib/a.rb:1:1: C: Missing top-level documentation comment for `class Person`. (Style/Documentation)"));
    for config in [
        "[cops.Style]\nenabled = false\n",
        "[cops.\"Style/Documentation\"]\nenabled = false\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
        cli(
            dir.path(),
            &["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
            0,
        );
        cli(
            dir.path(),
            &["--only", COP, "--except", "Style", "lib/a.rb"],
            0,
        );
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.Style]\nenabled = false\n[cops.\"Style/Documentation\"]\nenabled = true\n",
    )
    .unwrap();
    let output = cli(dir.path(), &["--fix", "lib/a.rb"], 1);
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("0 corrections applied, 1 offense remaining")
    );
    assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn pinned_paths_apply_to_explicit_files_and_stdin_relative_to_config_not_cwd() {
    let dir = project();
    for name in ["spec/a.rb", "spec/deep/a.rb", "test/a.rb", "test/deep/a.rb"] {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, SOURCE).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", name], 0);
        cli(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", &format!("../{name}")],
            0,
        );
        stdin(dir.path(), name, SOURCE.as_bytes(), 0);
        stdin(
            &dir.path().join("lib"),
            &format!("../{name}"),
            SOURCE.as_bytes(),
            0,
        );
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    stdin(dir.path(), "lib/a.rb", SOURCE.as_bytes(), 1);
    assert_eq!(
        fs::read(dir.path().join("lib/a.rb")).unwrap(),
        SOURCE.as_bytes()
    );
}

#[test]
fn user_excludes_are_additive_and_cannot_be_bypassed_by_only() {
    let dir = project();
    for config in [
        "exclude = [\"lib/a.rb\"]\n[cops.\"Style/Documentation\"]\nexclude = [\"other.rb\"]\n",
        "exclude = [\"other.rb\"]\n[cops.\"Style/Documentation\"]\nexclude = [\"lib/a.rb\"]\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        cli(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        stdin(&dir.path().join("lib"), "a.rb", SOURCE.as_bytes(), 0);
        assert_eq!(
            fs::read(dir.path().join("lib/a.rb")).unwrap(),
            SOURCE.as_bytes()
        );
    }
}

#[test]
fn directives_suppress_the_heading_not_the_body_and_support_both_prefixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rubocop", "rustocop"] {
        for target in [COP, "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                format!("class Person # {prefix}:disable {target}\n  def foo; end\nend\n"),
            ] {
                fs::write(&path, &source).unwrap();
                cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
                stdin(dir.path(), "lib/a.rb", source.as_bytes(), 0);
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            }
        }
        let source = format!("class Person\n  def foo; end # {prefix}:disable {COP}\nend\n");
        fs::write(&path, &source).unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 1);
        let source = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        fs::write(&path, &source).unwrap();
        let output = cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&output.stdout).contains("lib/a.rb:6:1:"));
    }
}

#[test]
fn noncorrectable_fix_preserves_encoding_bom_crlf_line_literals_and_data() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "\u{feff}class Person\r\n  def foo = [__LINE__, \"é\"]\r\nend\r\n__END__\r\nclass Undocumented\r\n".as_bytes(),
        b"# encoding: ASCII-8BIT\nclass Person\n  VALUE = \"\xff\"\n  def foo = __LINE__\nend\n",
        b"# encoding: Shift_JIS\nclass Person\n  VALUE = \"\x82\xa0\"\n  def foo = __LINE__\nend\n",
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        for args in [vec!["--only", COP, "lib/a.rb"], vec!["--only", COP, "--fix", "lib/a.rb"]] {
            let output = cli(dir.path(), &args, 1);
            assert_eq!(String::from_utf8_lossy(&output.stdout).matches("Missing top-level").count(), 1);
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
    for (source, status) in [
        ("# Person docs.\nclass Person\n  def foo; end\nend\n", 0),
        ("class Person\n  def (", 2),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn empty_semicolon_attachment_is_detection_only_through_cli_and_stdin() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, status, position) in [
        (";# Documentation.\nclass C; def f; end; end\n", 0, None),
        ("# Documentation.\n\x0b;class C; def f; end; end\n", 0, None),
        ("# Documentation.\n;class C; def f; end; end\n", 0, None),
        (
            ";# coding: utf-8\nclass C; def f; end; end\n",
            1,
            Some("2:1"),
        ),
        (
            ";#! /usr/bin/ruby\nclass C; def f; end; end\n",
            1,
            Some("2:1"),
        ),
        (
            ";# warn_indent: true\nclass C; def f; end; end\n",
            1,
            Some("2:1"),
        ),
        (
            ";# coding: utf-8\n;# Documentation.\n;class C; def f; end; end\n",
            0,
            None,
        ),
        (
            "\u{feff}\x0b;# Documentation.\r\n\x0b;class C; def f = __LINE__; end\r\n__END__\r\n;# data\r\n",
            0,
            None,
        ),
        (
            "work; ;# Documentation.\nclass C; def f; end; end\n",
            1,
            Some("2:1"),
        ),
        (
            "# Documentation.\n;work;class C; def f; end; end\n",
            1,
            Some("2:7"),
        ),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            let output = cli(dir.path(), &args, status);
            if let Some(position) = position {
                assert!(String::from_utf8_lossy(&output.stdout).contains(&format!(
                    "lib/a.rb:{position}: C: Missing top-level documentation comment for `class C`. (Style/Documentation)"
                )));
            }
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        stdin(dir.path(), "lib/a.rb", source.as_bytes(), status);
        // The fixed upstream path exemption still wins over explicit --only.
        stdin(dir.path(), "spec/a.rb", source.as_bytes(), 0);
    }
}

#[test]
fn fixed_allowed_constants_and_paths_do_not_expand_public_configuration() {
    let dir = project();
    for key in [
        "allowed_constants = [\"ClassMethods\"]",
        "Include = [\"lib/**\"]",
        "max = 1",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"Style/Documentation\"]\n{key}\n"),
        )
        .unwrap();
        cli(dir.path(), &["--only", COP, "lib/a.rb"], 2);
    }
}
