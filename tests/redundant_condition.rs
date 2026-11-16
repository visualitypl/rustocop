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
