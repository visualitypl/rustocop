//! opt-in method documentation through the real pipeline and CLI.
use rustocop::{
    config::{RawConfig, ResolvedConfig},
    cops::{Registry, select_active},
    pipeline::lint_one,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Style/DocumentationMethod";
const SOURCE: &str = "# A person.\nclass Person\n  def foo; end\nend\n";
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
fn stdin(cwd: &Path, path: &str, source: &[u8], status: i32) {
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
}

#[test]
fn real_pipeline_exact_registration_defaults_positions_and_path_policy() {
    let registry = Registry::new();
    let matches: Vec<_> = registry
        .cops()
        .iter()
        .filter(|cop| cop.name() == COP)
        .collect();
    assert_eq!(matches.len(), 1);
    assert!(!matches[0].default_enabled());
    assert!(!matches[0].safe_fixes());
    assert_eq!(matches[0].ruby_versions(), Default::default());
    let raw: RawConfig = toml::from_str("exclude = [\"lib/global.rb\"]\n[cops.\"Style/DocumentationMethod\"]\nexclude = [\"lib/local.rb\"]\n").unwrap();
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
            assert_eq!((offense.line, offense.col, offense.end_line), (3, 3, 3));
            assert_eq!(offense.message, "Missing method documentation comment.");
            assert!(offense.correction.is_none());
        }
    }
}

#[test]
fn opt_in_selectors_config_precedence_and_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["lib/a.rb"], 0);
    cli(dir.path(), &["--fix", "lib/a.rb"], 0);
    for config in [
        "",
        "[cops.Style]\nenabled = false\n",
        "[cops.\"Style/DocumentationMethod\"]\nenabled = false\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let output = cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&output.stdout).contains(
            "lib/a.rb:3:3: C: Missing method documentation comment. (Style/DocumentationMethod)"
        ));
        for except in [COP, "Style"] {
            cli(
                dir.path(),
                &["--only", COP, "--except", except, "--fix", "lib/a.rb"],
                0,
            );
        }
        let output = cli(dir.path(), &["--only", "Style", "lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&output.stdout).contains(COP));
    }
    for config in [
        "[cops.Style]\nenabled = false\n[cops.\"Style/DocumentationMethod\"]\nenabled = true\n",
        "[cops.Style]\nenabled = true\n[cops.\"Style/FrozenStringLiteralComment\"]\nenabled = false\n[cops.\"Style/Copyright\"]\nenabled = false\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let output = cli(dir.path(), &["--fix", "lib/a.rb"], 1);
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("0 corrections applied, 1 offense remaining")
        );
        cli(dir.path(), &["--except", COP, "lib/a.rb"], 0);
    }
    assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn pinned_and_additive_user_excludes_apply_to_files_and_stdin_from_config_root() {
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
    for config in [
        "exclude = [\"lib/a.rb\"]\n[cops.\"Style/DocumentationMethod\"]\nexclude = [\"other.rb\"]\n",
        "exclude = [\"other.rb\"]\n[cops.\"Style/DocumentationMethod\"]\nexclude = [\"lib/a.rb\"]\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        cli(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        stdin(&dir.path().join("lib"), "a.rb", SOURCE.as_bytes(), 0);
    }
    assert_eq!(
        fs::read(dir.path().join("lib/a.rb")).unwrap(),
        SOURCE.as_bytes()
    );
}

#[test]
fn both_directive_prefixes_suppress_whole_definition_highlights() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rubocop", "rustocop"] {
        for target in [COP, "DocumentationMethod", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\ndef foo; end\n"),
                format!("# {prefix}:disable-next {target}\ndef foo\n work\nend\n"),
                format!("def foo; end # {prefix}:disable {target}\n"),
                format!("def foo\n work # {prefix}:disable {target}\nend\n"),
            ] {
                fs::write(&path, &source).unwrap();
                cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
                stdin(dir.path(), "lib/a.rb", source.as_bytes(), 0);
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            }
        }
        let source = format!(
            "# {prefix}:disable {COP}\ndef foo; end\n# {prefix}:enable {COP}\ndef bar; end\n"
        );
        fs::write(&path, source).unwrap();
        let output = cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&output.stdout).contains("lib/a.rb:4:1:"));
    }
}

#[test]
fn noncorrectable_fix_preserves_bom_crlf_encodings_line_literals_data_and_errors() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, status, count) in [
        (
            "\u{feff}def foo = [__LINE__, \"é\"]\r\n__END__\r\ndef data; end\r\n".as_bytes(),
            1,
            1,
        ),
        (
            b"# encoding: ASCII-8BIT\ndef foo = [__LINE__, \"\xff\"]\n".as_slice(),
            1,
            1,
        ),
        (
            b"# encoding: Shift_JIS\ndef foo = [__LINE__, \"\x82\xa0\"]\n".as_slice(),
            1,
            1,
        ),
        (b"# Documentation\ndef foo; end\n".as_slice(), 0, 0),
        (b"private\ndef foo; end\n".as_slice(), 0, 0),
        (b"def foo(\n".as_slice(), 2, 0),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            let output = cli(dir.path(), &args, status);
            assert_eq!(
                String::from_utf8_lossy(&output.stdout)
                    .matches("Missing method documentation")
                    .count(),
                count
            );
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}

#[test]
fn policy_settings_remain_strictly_fixed() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for key in [
        "allowed_methods = []",
        "require_for_non_public_methods = true",
        "Include = []",
        "max = 2",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 2);
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    }
}
