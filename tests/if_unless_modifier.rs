//! IfUnlessModifier's detection/file/CLI contract.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const COP: &str = "Style/IfUnlessModifier";
const BAD: &str = "if condition\n  side_effect\nend\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/a.rb"), BAD).unwrap();
    dir
}
fn run(root: &Path, args: &[&str], expected: usize) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.lines().filter(|line| line.contains(COP)).count(),
        expected,
        "{output:?}"
    );
}
#[test]
fn defaults_selectors_config_and_safe_correction_capability() {
    let dir = project();
    for (config, mut args, count) in [
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
        args.push("lib/a.rb");
        run(dir.path(), &args, count);
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
    assert_eq!(cop.max_setting(), None);
}
#[test]
fn global_per_cop_excludes_and_both_directive_spellings() {
    let dir = project();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        run(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for (source, count) in [
            (format!("# {prefix}:disable {COP}\n{BAD}"), 0),
            (format!("# {prefix}:disable-next {COP}\n{BAD}"), 0),
            (
                BAD.replace(
                    "if condition",
                    &format!("if condition # {prefix}:disable Style"),
                ),
                0,
            ),
            (
                format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n{BAD}"),
                1,
            ),
        ] {
            fs::write(dir.path().join("lib/a.rb"), &source).unwrap();
            run(dir.path(), &["--only", COP, "lib/a.rb"], count);
            assert_eq!(
                fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
                source
            );
        }
    }
}
#[test]
fn length_dependency_ignores_selectors_but_honors_config_and_directives() {
    let dir = project();
    let normal = format!("if {}\n work\nend\n", "x".repeat(90));
    let modifier = format!("work if {}\n", "x".repeat(90));
    for (source, config, expected) in [
        (normal.clone(), "", 1),
        (
            normal.clone(),
            "[cops.\"Layout/LineLength\"]\nmax = 80\n",
            0,
        ),
        (
            normal,
            "[cops.\"Layout/LineLength\"]\nmax = 80\nenabled = false\n",
            1,
        ),
        (
            modifier.clone(),
            "[cops.\"Layout/LineLength\"]\nmax = 80\n",
            1,
        ),
        (
            modifier.clone(),
            "[cops.\"Layout/LineLength\"]\nmax = 80\nenabled = false\n",
            0,
        ),
        (
            format!("# rubocop:disable Layout/LineLength\n{modifier}"),
            "[cops.\"Layout/LineLength\"]\nmax = 80\n",
            0,
        ),
        (
            modifier.trim_end().to_owned() + " # rustocop:disable Layout/LineLength\n",
            "[cops.\"Layout/LineLength\"]\nmax = 80\n",
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(dir.path().join("lib/a.rb"), &source).unwrap();
        run(
            dir.path(),
            &["--only", COP, "--except", "Layout", "lib/a.rb"],
            expected,
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            source
        );
    }
}
#[test]
fn stdin_uses_display_path_and_never_writes() {
    let dir = project();
    for (config, source, status) in [
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
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
}
