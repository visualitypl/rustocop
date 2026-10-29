//! GuardClause's safe-correction contract through the real file/CLI pipeline.
//! Pinned examples: RuboCop guard_clause_spec.rb (MIT, Copyright (c) 2012-26
//! Bozhidar Batsov).
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const COP: &str = "Style/GuardClause";
const BAD: &str = "def example\n  if condition\n    side_effect\n    value\n  end\nend\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/a.rb"), BAD).unwrap();
    dir
}

fn run(root: &Path, args: &[&str], status: i32, offenses: usize) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.lines().filter(|line| line.contains(COP)).count(),
        offenses,
        "{output:?}"
    );
    if offenses > 0 {
        assert!(stdout.contains("Use a guard clause (`"), "{stdout}");
    }
}

#[test]
fn default_selectors_config_and_capability() {
    let dir = project();
    for (config, selectors, offenses) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Style", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
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
        (format!("[cops.\"{COP}\"]\nenabled = false\n"), vec![], 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        // Default-selected Layout can also report; assert the named cop rather
        // than equating total offenses with its own count.
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(&args)
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.contains(COP), offenses == 1, "{output:?}");
        if offenses == 1 {
            assert!(stdout.contains("lib/a.rb:2:3"), "{stdout}");
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
}

#[test]
fn exclusions_and_directives_preserve_files() {
    let dir = project();
    let file = dir.path().join("lib/a.rb");
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("a.rb");
            run(&dir.path().join("lib"), &args, 0, 0);
            assert_eq!(fs::read_to_string(&file).unwrap(), BAD);
        }
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
                BAD.replace("    value", &format!("    value # {prefix}:disable {COP}")),
                1,
            ),
        ] {
            fs::write(&file, &source).unwrap();
            run(
                dir.path(),
                &["--only", COP, "--fix", "lib/a.rb"],
                i32::from(count > 0),
                count,
            );
            assert_eq!(fs::read_to_string(&file).unwrap(), source);
        }
    }
}

#[test]
fn uncertain_clean_and_error_files_are_not_written() {
    let dir = project();
    let file = dir.path().join("lib/a.rb");
    for (source, status, count) in [
        ("def f\n  work if condition\nend\n", 0, 0),
        ("def f\n  if condition\n", 2, 0),
        (
            "if condition\n  work || raise('error')\nelse\n  value\nend\n",
            1,
            1,
        ),
        (
            "if condition\n  raise <<~MESSAGE\n    error\n  MESSAGE\nelse\n  work\nend\n",
            1,
            1,
        ),
        (
            "def f\n  if condition\n    side_effect\n    return __LINE__\n  end\nensure\n  cleanup\nend\n",
            0,
            0,
        ),
    ] {
        fs::write(&file, source).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            run(dir.path(), &args, status, count);
            assert_eq!(fs::read(&file).unwrap(), source.as_bytes());
        }
    }
}

#[test]
fn stdin_uses_display_path_exclusions_and_inline_directives_without_writes() {
    let dir = project();
    for (config, source, expected) in [
        (String::new(), BAD.to_owned(), 1),
        (
            format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
            BAD.to_owned(),
            0,
        ),
        ("exclude = [\"lib/a.rb\"]\n".to_owned(), BAD.to_owned(), 0),
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
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).contains(COP),
            expected == 1,
            "{output:?}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
}

#[test]
fn configured_line_length_applies_even_with_only_guard_clause() {
    let dir = project();
    let source = format!("def f\n  if {}\n    work\n  end\nend\n", "x".repeat(70));
    fs::write(dir.path().join("lib/a.rb"), &source).unwrap();
    for (config, count) in [
        ("", 1),
        ("[cops.\"Layout/LineLength\"]\nmax = 80\n", 0),
        (
            "[cops.\"Layout/LineLength\"]\nmax = 80\nenabled = false\n",
            1,
        ),
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
