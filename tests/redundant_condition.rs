//! RedundantCondition's default-on CLI and safe correction contract.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const COP: &str = "Style/RedundantCondition";
const BAD: &str = "if value\n  value\nelse\n  fallback\nend\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/a.rb"), BAD).unwrap();
    dir
}

fn run(root: &Path, args: &[&str], status: Option<i32>, count: usize) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    if let Some(status) = status {
        assert_eq!(output.status.code(), Some(status), "{output:?}");
    } else {
        assert_ne!(output.status.code(), Some(2), "{output:?}");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.lines().filter(|line| line.contains(COP)).count(),
        count,
        "{output:?}"
    );
    if count > 0 {
        assert!(
            stdout.contains("Use double pipes `||` instead."),
            "{stdout}"
        );
    }
}

#[test]
fn defaults_selectors_and_config_overrides() {
    let dir = project();
    for (config, selectors, count) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Style", "--except", COP], 0),
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
        run(dir.path(), &args, None, count);
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
    let registry = rustocop::cops::Registry::new();
    let cop = registry
        .cops()
        .iter()
        .find(|cop| cop.name() == COP)
        .unwrap();
    assert!(cop.default_enabled());
    assert!(cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
}

#[test]
fn config_relative_exclusions_and_directive_spans_apply_without_writes() {
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
            Some(0),
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
                BAD.replace("  value\n", &format!("  value # {prefix}:disable {COP}\n")),
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
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(i32::from(count > 0)),
                count,
            );
            assert_eq!(fs::read_to_string(&file).unwrap(), source);
        }
    }
}

#[test]
fn withheld_fix_preserves_bytes_and_line_values_even_for_control_flow() {
    let dir = project();
    let file = dir.path().join("lib/a.rb");
    for (source, status, count) in [
        (BAD, 1, 1),
        ("other ? value : fallback\n", 0, 0),
        ("value if value\n", 0, 0),
        ("if value\n  work\n  value\nelse\n  fallback\nend\n", 0, 0),
        ("if value\n", 2, 0),
        ("x = if b then b else next 1 end\n", 2, 0),
        ("x = if b then b else break 1 end\n", 2, 0),
        ("x = if b then b else yield 1 end\n", 2, 0),
        ("x = if b then b else yield(1) end\n", 2, 0),
        (
            "\u{feff}# encoding: UTF-8\r\nif value\r\n  # branch\r\n  value\r\nelse\r\n  __LINE__ # untouched\r\nend\r\n__END__\r\ndata\r\n",
            1,
            1,
        ),
        (
            "begin\n  if value\n    value\n  else\n    fallback\n  end\nrescue\n  recover\nensure\n  cleanup\nend\n",
            1,
            1,
        ),
        (
            "if value\n  value\nelse\n  <<~TEXT\n    literal # untouched\n  TEXT\nend\n",
            1,
            1,
        ),
    ] {
        fs::write(&file, source).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            run(dir.path(), &args, Some(status), count);
            assert_eq!(fs::read(&file).unwrap(), source.as_bytes());
        }
    }
}

#[test]
fn stdin_honors_excludes_and_directives_and_reports_exact_positions() {
    let dir = project();
    for (config, source, count) in [
        (String::new(), "value ? value : fallback\n".to_owned(), 1),
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
            assert!(stdout.contains("a.rb:1:7:"), "{stdout}");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
}

#[test]
fn fixed_methods_are_not_a_public_option() {
    let dir = project();
    for option in ["AllowedMethods = []", "TargetRubyVersion = 3.4", "max = 80"] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        run(
            dir.path(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
            0,
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
}
