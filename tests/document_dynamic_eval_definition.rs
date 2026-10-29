//! dynamic eval documentation through the actual pipeline and CLI.
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

const COP: &str = "Style/DocumentDynamicEvalDefinition";
// Keep source metadata valid so default-enabled EvalWithLocation does not
// obscure this cop's independent config/path-policy assertions. The
// Security/Eval also reports this dynamic string; disable it in isolation checks.
const SOURCE: &str = "eval(\"def #{name}; end\", binding, __FILE__, __LINE__)\n";
const MESSAGE: &str = "Add a comment block showing its appearance if interpolated.";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.\"Security/Eval\"]\nenabled = false\n",
    )
    .unwrap();
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
fn real_pipeline_default_policy_exact_diagnostics_and_no_documentation_path_exemptions() {
    let registry = Registry::new();
    let cops: Vec<_> = registry.cops().iter().filter(|c| c.name() == COP).collect();
    assert_eq!(cops.len(), 1);
    assert!(cops[0].default_enabled());
    assert!(!cops[0].safe_fixes());
    assert_eq!(cops[0].ruby_versions(), Default::default());
    assert!(cops[0].path_policy().is_none());
    let raw: RawConfig = toml::from_str("exclude = [\"lib/global.rb\"]\n[cops.\"Style/DocumentDynamicEvalDefinition\"]\nexclude = [\"lib/local.rb\"]\n").unwrap();
    let config =
        ResolvedConfig::resolve(raw, Some(Path::new("/project/rustocop.toml")), &registry).unwrap();
    let active = select_active(&registry, &config, &[], &["Security/Eval".into()]);
    for (path, count) in [
        ("lib/a.rb", 1),
        ("spec/a.rb", 1),
        ("test/deep/a.rb", 1),
        ("/project/spec/a.rb", 1),
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
            assert_eq!(offense.cop, COP);
            assert_eq!((offense.line, offense.col, offense.end_line), (1, 1, 1));
            assert_eq!(offense.message, MESSAGE);
            assert!(offense.correction.is_none());
        }
    }
}

#[test]
fn default_selectors_enabling_config_precedence_and_noncorrectable_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(dir.path(), &["lib/a.rb"], 1);
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("lib/a.rb:1:1: C: {MESSAGE} ({COP})"))
    );
    for selector in [COP, "Style"] {
        cli(dir.path(), &["--only", selector, "lib/a.rb"], 1);
        for except in [COP, "Style"] {
            let status = i32::from(selector == "Style" && except == COP);
            // --only Style also force-enables unrelated opt-ins. Do not fix
            // those here; assert only the selected cop's absence.
            let output = cli(
                dir.path(),
                &["--only", selector, "--except", except, "lib/a.rb"],
                status,
            );
            assert!(!String::from_utf8_lossy(&output.stdout).contains(COP));
        }
    }
    for config in [
        "[cops.Style]\nenabled = false\n",
        "[cops.\"Style/DocumentDynamicEvalDefinition\"]\nenabled = false\n",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("{config}[cops.\"Security/Eval\"]\nenabled = false\n"),
        )
        .unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
    }
    fs::write(dir.path().join("rustocop.toml"), "[cops.Style]\nenabled = false\n[cops.\"Style/DocumentDynamicEvalDefinition\"]\nenabled = true\n[cops.\"Security/Eval\"]\nenabled = false\n").unwrap();
    let output = cli(dir.path(), &["--fix", "lib/a.rb"], 1);
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("0 corrections applied, 1 offense remaining")
    );
    cli(dir.path(), &["--except", COP, "lib/a.rb"], 0);
    assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn additive_excludes_resolve_from_config_root_for_files_and_stdin() {
    let dir = project();
    for name in ["spec/a.rb", "test/deep/a.rb"] {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, SOURCE).unwrap();
        cli(dir.path(), &["--only", COP, "--fix", name], 1);
        stdin(dir.path(), name, SOURCE.as_bytes(), 1);
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    }
    for config in [
        "exclude = [\"lib/a.rb\"]\n[cops.\"Style/DocumentDynamicEvalDefinition\"]\nexclude = [\"other.rb\"]\n",
        "exclude = [\"other.rb\"]\n[cops.\"Style/DocumentDynamicEvalDefinition\"]\nexclude = [\"lib/a.rb\"]\n",
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
fn directives_suppress_the_selector_not_unrelated_heredoc_body_lines() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rubocop", "rustocop"] {
        for target in [COP, "DocumentDynamicEvalDefinition", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                format!("eval(\"#{{name}}\") # {prefix}:disable {target}\n"),
            ] {
                fs::write(&path, &source).unwrap();
                cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
                stdin(dir.path(), "lib/a.rb", source.as_bytes(), 0);
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            }
        }
        let source = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        fs::write(&path, &source).unwrap();
        let output = cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&output.stdout).contains("lib/a.rb:4:1:"));
        let source = format!("eval <<~EOT\n# {prefix}:disable {COP}\ndef #{{name}}; end\nEOT\n");
        fs::write(&path, &source).unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 1);
    }
}

#[test]
fn no_writes_preserve_bom_crlf_legacy_encodings_line_literals_data_clean_and_error_files() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, status, count) in [
        ("\u{feff}eval(\"#{name} é\", binding, __FILE__, __LINE__)\r\n__END__\r\neval(\"#{data}\")\r\n".as_bytes(), 1, 1),
        (b"# encoding: ASCII-8BIT\neval(\"#{name} \xff\", binding, __FILE__, __LINE__)\n".as_slice(), 1, 1),
        (b"# encoding: Shift_JIS\neval(\"#{name} \x82\xa0\")\n".as_slice(), 1, 1),
        (b"eval(\"#{name}\") # example\r\n".as_slice(), 0, 0),
        (b"eval <<~EOT\r\n# def foo; end\r\ndef #{name}; end\r\nEOT\r\n".as_slice(), 0, 0),
        (b"eval(\"literal\")\n".as_slice(), 0, 0),
        (b"eval(\"#{name}\"\n".as_slice(), 2, 0),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        for args in [vec!["--only", COP, "lib/a.rb"], vec!["--only", COP, "--fix", "lib/a.rb"]] {
            let output = cli(dir.path(), &args, status);
            assert_eq!(String::from_utf8_lossy(&output.stdout).matches(MESSAGE).count(), count);
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}

#[test]
fn unsupported_policy_settings_are_errors_before_writes() {
    let dir = project();
    for key in ["include = []", "max = 3", "allowed_methods = []"] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 2);
        assert_eq!(
            fs::read(dir.path().join("lib/a.rb")).unwrap(),
            SOURCE.as_bytes()
        );
    }
}
