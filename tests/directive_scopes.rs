//! end-to-end source opt-ins and fixed CLI precedence.
use std::{fs, process::Command};

fn debugger_lines(source: &str, flags: &[&str]) -> Vec<u32> {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.\"Lint/Debugger\"]\nenabled = false\n",
    )
    .unwrap();
    fs::write(dir.path().join("x.rb"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(flags)
        .arg("x.rb")
        .output()
        .unwrap();
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_ne!(out.status.code(), Some(2), "{out:?}");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|line| line.contains("(Lint/Debugger)"))
        .map(|line| line.split(':').nth(1).unwrap().parse().unwrap())
        .collect()
}

#[test]
fn safe_fixes_compose_with_signed_and_restored_scopes() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for (mode, close) in [("next", ""), ("push", "pop")] {
            let source = format!(
                "# {prefix}:disable StringLiterals\n# {prefix}:{mode} +StringLiterals -SignalException\nfail 'one'\n# {prefix}:{close}\nfail 'two'\n"
            );
            let path = dir.path().join("x.rb");
            fs::write(&path, &source).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args([
                    "--only",
                    "Style/StringLiterals,Style/SignalException",
                    "--fix",
                    "x.rb",
                ])
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            assert!(out.stderr.is_empty(), "{out:?}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                source
                    .replace("'one'", "\"one\"")
                    .replace("fail 'two'", "raise 'two'")
            );
        }
        for mode in ["enable-next StringLiterals", "next +StringLiterals"] {
            let source = format!("# {prefix}:{mode}\n'one'\n'other'\n");
            fs::write(
                dir.path().join("rustocop.toml"),
                "[cops.\"Style/StringLiterals\"]\nenabled = false\n",
            )
            .unwrap();
            let path = dir.path().join("x.rb");
            fs::write(&path, &source).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--except", "Lint/Void", "--fix", "x.rb"])
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                source.replace("'one'", "\"one\"")
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn explicit_ban_remains_immune_to_all_new_suppression_forms() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    let ban = "Style/DisableCopsWithinSourceCodeDirective";
    for prefix in ["rubocop", "rustocop"] {
        for mode in [
            format!("todo-next {ban}"),
            format!("next -{ban}"),
            format!("push -{ban}"),
            format!("disable {ban}"),
        ] {
            let source = format!(
                "# {prefix}:{mode}\n# {prefix}:enable-next Lint/Debugger\nbinding.pry\n# {prefix}:pop\n"
            );
            fs::write(dir.path().join("x.rb"), &source).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", ban, "x.rb"])
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(1), "{source} {out:?}");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout)
                    .matches("directives are not permitted")
                    .count(),
                2,
                "{source} {out:?}"
            );
        }
    }
}

#[test]
fn source_only_ban_activation_preserves_scope_and_self_disable_immunity() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let ban = "Style/DisableCopsWithinSourceCodeDirective";
    for prefix in ["rubocop", "rustocop"] {
        for mode in [format!("enable-next {ban}"), format!("next +{ban}")] {
            for (body, expected) in [
                ("foo\n".to_owned(), vec![]),
                ("\nfoo\n".to_owned(), vec![]),
                (
                    format!("foo( # {prefix}:disable {ban}\n  bar\n)\n"),
                    vec![4],
                ),
            ] {
                let source = format!(
                    "# {prefix}:disable Lint/Debugger\nbinding.pry\n# {prefix}:{mode}\n{body}# {prefix}:disable Style/StringLiterals\n"
                );
                fs::write(dir.path().join("x.rb"), &source).unwrap();
                let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                    .current_dir(dir.path())
                    .arg("x.rb")
                    .output()
                    .unwrap();
                assert!(out.stderr.is_empty(), "{out:?}");
                assert_ne!(out.status.code(), Some(2), "{out:?}");
                let lines: Vec<u32> = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .filter(|line| line.contains(&format!("({ban})")))
                    .map(|line| line.split(':').nth(1).unwrap().parse().unwrap())
                    .collect();
                assert_eq!(lines, expected, "{source} {out:?}");
            }
        }
    }
}

#[test]
fn source_opt_ins_still_respect_file_and_stdin_path_exclusions() {
    use std::io::Write;
    use std::process::Stdio;
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.\"Lint/Debugger\"]\nenabled = false\nexclude = [\"lib/x.rb\"]\n",
    )
    .unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for mode in ["enable-next Lint", "next +Debugger", "push +Lint/Debugger"] {
            let source = format!("# {prefix}:{mode}\nbinding.pry\n");
            fs::write(dir.path().join("lib/x.rb"), &source).unwrap();
            for stdin in [false, true] {
                let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
                command
                    .current_dir(dir.path().join("lib"))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                if stdin {
                    command.arg("--stdin").stdin(Stdio::piped());
                }
                let mut child = command.arg("x.rb").spawn().unwrap();
                if stdin {
                    child
                        .stdin
                        .take()
                        .unwrap()
                        .write_all(source.as_bytes())
                        .unwrap();
                }
                let out = child.wait_with_output().unwrap();
                assert_eq!(out.status.code(), Some(0), "{source} {out:?}");
                assert!(out.stderr.is_empty(), "{out:?}");
                assert!(!String::from_utf8_lossy(&out.stdout).contains("(Lint/Debugger)"));
            }
        }
    }
}

#[test]
fn disabled_cops_opt_in_only_for_the_enabling_scope() {
    for prefix in ["rubocop", "rustocop"] {
        for (open, close) in [
            ("enable-next Lint/Debugger", ""),
            ("next +Debugger", ""),
            ("push +Lint", "pop"),
        ] {
            let source = format!(
                "binding.pry\n# {prefix}:{open}\nbinding.pry\n# {prefix}:{close}\nbinding.pry\n"
            );
            assert_eq!(debugger_lines(&source, &[]), [3], "{source}");
            assert!(debugger_lines(&source, &["--except", "Lint/Debugger"]).is_empty());
            assert!(debugger_lines(&source, &["--only", "Style/StringLiterals"]).is_empty());
            assert_eq!(
                debugger_lines(&source, &["--only", "Lint/Debugger"]),
                [1, 3, 5]
            );
        }
        for mode in ["enable-next all", "next +all", "push +all"] {
            let source = format!("# {prefix}:{mode}\nbinding.pry\n");
            assert!(debugger_lines(&source, &[]).is_empty(), "{source}");
        }
        let source = format!("binding.pry\n# {prefix}:enable Debugger\nbinding.pry\n");
        assert_eq!(debugger_lines(&source, &[]), [3]);
        for target in ["Lint", "all"] {
            let source = format!("# {prefix}:enable {target}\nbinding.pry\n");
            assert!(debugger_lines(&source, &[]).is_empty());
        }
    }
}
