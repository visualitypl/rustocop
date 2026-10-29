//! ConditionalAssignment's unchanged fixed-default detection pipeline contract.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const COP: &str = "Style/ConditionalAssignment";
const BAD: &str = "if condition\n  value = 1\nelse\n  value = 2\nend\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/a.rb"), BAD).unwrap();
    dir
}

fn run(root: &Path, args: &[&str], status: i32, count: usize) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.lines().filter(|line| line.contains(COP)).count(),
        count,
        "{output:?}"
    );
    if count > 0 {
        assert!(
            stdout.contains(
                "Use the return of the conditional for variable assignment and comparison."
            ),
            "{stdout}"
        );
    }
}

#[test]
fn default_enablement_selectors_and_config_overrides() {
    let dir = project();
    for (config, selectors, count) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (
            String::new(),
            vec![
                "--only",
                "Style",
                "--except",
                "Style/ConditionalAssignment,Style/FrozenStringLiteralComment",
            ],
            0,
        ),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (format!("[cops.\"{COP}\"]\nenabled = false\n"), vec![], 0),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            1,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        // Other default cops may report unused variables; isolate this cop's
        // findings while still exercising the actual default selection path.
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(&args)
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.lines().filter(|line| line.contains(COP)).count(),
            count,
            "{stdout}"
        );
        if count > 0 {
            assert!(stdout.contains("lib/a.rb:1:1:"), "{stdout}");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
    let registry = rustocop::cops::Registry::new();
    let rule = registry
        .cops()
        .iter()
        .find(|cop| cop.name() == COP)
        .unwrap();
    assert!(rule.default_enabled());
    assert!(rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(rule.path_policy().is_none());
    assert!(rule.max_setting().is_none());
}

#[test]
fn exclusions_and_both_directive_prefixes_suppress_detection_without_writes() {
    let dir = project();
    let file = dir.path().join("lib/a.rb");
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        run(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            0,
            0,
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), BAD);
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for (source, count) in [
            (format!("# {prefix}:disable {COP}\n{BAD}"), 0),
            (format!("# {prefix}:disable-next {COP}\n{BAD}"), 0),
            (format!("# {prefix}:disable Style\n{BAD}"), 0),
            (
                BAD.replace(
                    "if condition",
                    &format!("if condition # {prefix}:disable {COP}"),
                ),
                0,
            ),
            (
                BAD.replace("value = 1", &format!("value = 1 # {prefix}:disable {COP}")),
                0,
            ),
            (
                format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n{BAD}"),
                1,
            ),
        ] {
            fs::write(&file, &source).unwrap();
            run(
                dir.path(),
                &["--only", COP, "lib/a.rb"],
                i32::from(count > 0),
                count,
            );
            assert_eq!(fs::read_to_string(&file).unwrap(), source);
        }
    }
}

#[test]
fn no_fix_mode_preserves_comments_scopes_control_flow_and_line_literals() {
    let dir = project();
    let file = dir.path().join("lib/a.rb");
    for (source, status, count) in [
        (BAD, 1, 1),
        ("value = if condition\n  1\nelse\n  2\nend\n", 0, 0),
        ("value = 1 if value.nil?\n", 0, 0),
        (
            "if condition\n  work\n  value = 1\nelse\n  value = 2\nend\n",
            0,
            0,
        ),
        ("if condition\n", 2, 0),
        (
            "\u{feff}# encoding: UTF-8\r\nif condition\r\n  value = __LINE__ # keep\r\nelse\r\n  # else comment\r\n  value = __LINE__\r\nend\r\n__END__\r\ndata\r\n",
            1,
            1,
        ),
        (
            "begin\n  if condition\n    value = 1\n  else\n    value = 2\n  end\nrescue\n  recover\nensure\n  cleanup\nend\n",
            1,
            1,
        ),
        (
            "if condition\n  value = begin\n    work\n  rescue\n    recover\n  ensure\n    cleanup\n  end\nelse\n  value = 2\nend\n",
            1,
            1,
        ),
        (
            "if condition\n  value = <<~TEXT\n    literal # untouched\n  TEXT\nelse\n  value = \"other\"\nend\n",
            1,
            1,
        ),
        (
            "if condition\n  return value = 1\nelse\n  return value = 2\nend\n",
            0,
            0,
        ),
    ] {
        fs::write(&file, source).unwrap();
        run(dir.path(), &["--only", COP, "lib/a.rb"], status, count);
        assert_eq!(fs::read(&file).unwrap(), source.as_bytes());
    }
}

#[test]
fn configured_line_length_is_independent_of_cli_selection() {
    let dir = project();
    let source = format!(
        "if {}\n  value = 1\nelse\n  value = 2\nend\n",
        "x".repeat(75)
    );
    fs::write(dir.path().join("lib/a.rb"), &source).unwrap();
    for (config, count) in [
        ("", 1),
        ("[cops.\"Layout/LineLength\"]\nmax = 80\n", 0),
        (
            "[cops.\"Layout/LineLength\"]\nmax = 80\nenabled = false\n",
            1,
        ),
        ("[cops.Layout]\nenabled = false\n", 1),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        run(
            dir.path(),
            &["--only", COP, "lib/a.rb"],
            i32::from(count > 0),
            count,
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            source
        );
    }
}

#[test]
fn stdin_obeys_config_and_directives_without_touching_display_file() {
    let dir = project();
    for (config, source, count) in [
        (String::new(), BAD.to_owned(), 1),
        (
            format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
            BAD.to_owned(),
            0,
        ),
        (String::new(), format!("# rubocop:disable {COP}\n{BAD}"), 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path().join("lib"))
            .args(["--only", COP, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(count), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.contains(COP), count == 1, "{stdout}");
        if count == 1 {
            assert!(stdout.contains("a.rb:1:1:"), "{stdout}");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
}

#[test]
fn arbitrary_style_options_remain_strict_errors() {
    let dir = project();
    for option in [
        "EnforcedStyle = \"assign_inside_condition\"",
        "SingleLineConditionsOnly = false",
        "IncludeTernaryExpressions = false",
        "max = 80",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 2, 0);
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
}
