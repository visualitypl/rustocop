//! selectors, directive immunity, conservative fixes and Ruby observations.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const SCOPE: &str = "Style/DirectiveScope";
const BAN: &str = "Style/DisableCopsWithinSourceCodeDirective";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(cwd: &Path, args: &[&str], code: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(cwd)
        .args(args)
        // This suite isolates the two Style directive policies; the new Lint
        // policies have independent diagnostic and correction tests.
        .args([
            "--except",
            "Lint/RedundantCopEnableDirective,Lint/MissingCopEnableDirective,Lint/RedundantCopDisableDirective",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    if code != 2 {
        assert!(out.stderr.is_empty(), "{out:?}");
    }
    out
}
#[test]
fn registry_defaults_and_safety() {
    let registry = rustocop::cops::Registry::new();
    for (name, default, safe) in [(SCOPE, true, false), (BAN, false, true)] {
        let found: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == name)
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].default_enabled(), default);
        assert_eq!(found[0].safe_fixes(), safe);
    }
}
#[test]
fn default_opt_in_selectors_config_and_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let scope =
        "# rubocop:disable Migration/Nonexistent\nwork\n# rubocop:enable Migration/Nonexistent\n";
    fs::write(&path, scope).unwrap();
    // Isolate Style rules: Migration/Nonexistent is deliberately not a landed
    // cop name (all standard departments have cops).
    let out = cli(
        dir.path(),
        &["--except", "Lint/CopDirectiveSyntax", "lib/a.rb"],
        1,
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("lib/a.rb:1:1: C:") && text.contains("(Style/DirectiveScope)"),
        "{text}"
    );
    assert!(!text.contains(BAN));
    fs::write(&path, "# rubocop:enable Migration/Nonexistent\n").unwrap();
    cli(
        dir.path(),
        &["--except", "Lint/CopDirectiveSyntax", "lib/a.rb"],
        0,
    );
    for (cop, before, after, fix_code) in [
        (SCOPE, scope, scope, 1),
        (BAN, "# rubocop:enable Migration/Nonexistent\n", "\n", 0),
    ] {
        fs::write(&path, before).unwrap();
        for cfg in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), cfg).unwrap();
            cli(
                dir.path(),
                &["--except", "Lint/CopDirectiveSyntax", "lib/a.rb"],
                0,
            );
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            cli(
                dir.path(),
                &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], fix_code);
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], fix_code);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn explicit_ban_ignores_self_disable_but_scope_honors_it() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rubocop", "rustocop"] {
        for target in [SCOPE, "Style", "all"] {
            let source = format!(
                "# {prefix}:disable {target}\n# {prefix}:disable Migration/Nonexistent\nwork\n# {prefix}:enable Migration/Nonexistent\n"
            );
            fs::write(&path, &source).unwrap();
            cli(dir.path(), &["--only", SCOPE, "--fix", "lib/a.rb"], 0);
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        }
        for target in [BAN, "Style", "all"] {
            let source = format!(
                "# {prefix}:disable {target}\n# {prefix}:disable Migration/Nonexistent\nwork\n# {prefix}:enable Migration/Nonexistent\n"
            );
            fs::write(&path, &source).unwrap();
            let out = cli(dir.path(), &["--only", BAN, "lib/a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&out.stdout)
                    .matches("directives are not permitted")
                    .count(),
                3,
                "{out:?}"
            );
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        }
    }
}
#[test]
fn excludes_are_additive_config_relative_and_not_bypassed() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source =
        "# rubocop:disable Migration/Nonexistent\nwork\n# rubocop:enable Migration/Nonexistent\n";
    fs::write(&path, source).unwrap();
    for cop in [SCOPE, BAN] {
        for (global, local) in [("lib/a.rb", "other.rb"), ("other.rb", "lib/a.rb")] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("exclude = [\"{global}\"]\n[cops.\"{cop}\"]\nexclude = [\"{local}\"]\n"),
            )
            .unwrap();
            cli(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "../lib/a.rb"],
                0,
            );
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        }
    }
}
#[test]
fn removal_never_unsuppresses_other_selected_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "# rubocop:disable Style, Layout\ndef foo\n\treturn ('value');\nend\n# rubocop:enable Style, Layout\n# rubocop:disable Lint/Debugger\nbinding.pry\n# rubocop:enable Lint/Debugger\n# rustocop:disable Migration/Nonexistent\nwork\n";
    fs::write(&path, source).unwrap();
    let only = format!(
        "{BAN},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout,Lint/Debugger"
    );
    cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 1);
    let after = source.replace("# rustocop:disable Migration/Nonexistent", "");
    assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
    let out = cli(
        dir.path(),
        &[
            "--only",
            "Lint/Debugger,Style/StringLiterals,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Layout",
            "lib/a.rb",
        ],
        0,
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("no offenses detected"),
        "{out:?}"
    );
}
#[test]
fn stacked_next_attachments_survive_standalone_and_paired_pop_removals() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!(
                "\u{feff}# {prefix}:disable-next Lint/Debugger\n# {prefix}:disable Migration/Nonexistent\nbinding.pry\n"
            ),
            format!(
                "# {prefix}:disable-next Lint/Debugger\r\n# {prefix}:disable Migration/Nonexistent\r\nbinding.pry\r\n"
            ),
            format!(
                "# {prefix}:push -Migration/Nonexistent\nwork\n# {prefix}:disable-next Lint/Debugger\n# ordinary comment\n# {prefix}:pop\nbinding.pry\n"
            ),
            format!(
                "# {prefix}:disable-next Lint/Debugger\n# {prefix}:enable-next Migration/Nonexistent\n# {prefix}:next -Migration/Nonexistent\nbinding.pry\n"
            ),
            format!("# {prefix}:disable-next Lint/Debugger\n# {prefix}:next +all\nbinding.pry\n"),
            format!("# {prefix}:disable-next Lint/Debugger\n# {prefix}:next -all\nbinding.pry\n"),
        ] {
            fs::write(&path, &source).unwrap();
            cli(dir.path(), &["--only", "Lint/Debugger", "lib/a.rb"], 0);
            let only = format!("{BAN},Lint/Debugger");
            let out = cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 1);
            assert!(
                !String::from_utf8_lossy(&out.stdout).contains("(Lint/Debugger)"),
                "{out:?}"
            );
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            cli(dir.path(), &["--only", "Lint/Debugger", "lib/a.rb"], 0);
        }
    }
}
#[test]
fn signed_all_removals_preserve_other_cop_diagnostics() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let only = format!("{BAN},Lint/Debugger");
    for prefix in ["rubocop", "rustocop"] {
        for sign in ["+", "-"] {
            for mode in ["push", "next"] {
                let pop = if mode == "push" {
                    format!("# {prefix}:pop\n")
                } else {
                    String::new()
                };
                let source = format!("# {prefix}:{mode} {sign}all\nbinding.pry\n{pop}");
                let expected = if mode == "push" {
                    "\nbinding.pry\n\n"
                } else {
                    "\nbinding.pry\n"
                };
                fs::write(&path, &source).unwrap();
                let original = cli(dir.path(), &["--only", &only, "lib/a.rb"], 1);
                let fixed = cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 1);
                for out in [&original, &fixed] {
                    let text = String::from_utf8_lossy(&out.stdout);
                    assert!(
                        text.contains("lib/a.rb:2:1: E:") && text.contains("(Lint/Debugger)"),
                        "{source}: {text}"
                    );
                }
                assert_eq!(fs::read_to_string(&path).unwrap(), expected, "{source}");
                let after = cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 1);
                assert!(!String::from_utf8_lossy(&after.stdout).contains(BAN));
                assert_eq!(fs::read_to_string(&path).unwrap(), expected);
            }
        }
    }
}
#[test]
fn unknown_push_removal_preserves_unrelated_restoration() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let only = format!("{BAN},Lint/Debugger");
    for prefix in ["rubocop", "rustocop"] {
        for (before, inside, expected) in [
            ("disable", "enable", vec![4]),
            ("enable", "disable", vec![6]),
        ] {
            for (open, close) in [
                (
                    format!("# {prefix}:push -Unknown/Cop\n"),
                    format!("# {prefix}:pop\n"),
                ),
                (
                    format!("# {prefix}:push -Unknown/Cop\n# {prefix}:push\n"),
                    format!("# {prefix}:pop\n# {prefix}:pop\n"),
                ),
                (format!("# {prefix}:push -Unknown/Cop\n"), String::new()),
                (
                    format!("# {prefix}:push +all\n"),
                    format!("# {prefix}:pop\n"),
                ),
                (
                    format!("# {prefix}:push -all\n"),
                    format!("# {prefix}:pop\n"),
                ),
                (format!("# {prefix}:push +all\n"), String::new()),
                (format!("# {prefix}:push -all\n"), String::new()),
            ] {
                let source = format!(
                    "# {prefix}:{before} Lint/Debugger\n{open}# {prefix}:{inside} Lint/Debugger\nbinding.pry\n{close}binding.pry\n"
                );
                fs::write(&path, &source).unwrap();
                let lines = |out: &Output| -> Vec<u32> {
                    String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .filter(|line| line.contains("(Lint/Debugger)"))
                        .map(|line| line.split(':').nth(1).unwrap().parse().unwrap())
                        .collect()
                };
                let original = cli(dir.path(), &["--only", &only, "lib/a.rb"], 1);
                if open.lines().count() == 1 && !close.is_empty() {
                    assert_eq!(lines(&original), expected, "{source}");
                }
                let fixed = cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 1);
                assert_eq!(lines(&fixed), lines(&original), "{source}");
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                let after = cli(dir.path(), &["--only", &only, "lib/a.rb"], 1);
                assert_eq!(lines(&after), lines(&original), "{source}");
            }
        }
    }
}

#[test]
fn bom_crlf_encoding_line_literals_and_data_bytes_survive() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for encoding in ["UTF-8", "ASCII-8BIT", "Windows-1252"] {
        let bom = if encoding == "UTF-8" { "\u{feff}" } else { "" };
        let source = format!(
            "{bom}# coding: {encoding}\r\n# rubocop:disable Migration/Nonexistent\r\np __LINE__\r\nx = \"# rubocop:disable all\"\r\nx = <<~H\r\n# rustocop:disable all\r\nH\r\n# rubocop:enable Migration/Nonexistent\r\n__END__\r\n# rubocop:disable all\r\n"
        );
        let mut before = source.as_bytes().to_vec();
        before.extend_from_slice(b"\xff\x82\xa0\r\n");
        fs::write(&path, &before).unwrap();
        cli(dir.path(), &["--only", BAN, "--fix", "lib/a.rb"], 0);
        let after = source
            .replace("# rubocop:disable Migration/Nonexistent", "")
            .replace("# rubocop:enable Migration/Nonexistent", "");
        let mut expected = after.into_bytes();
        expected.extend_from_slice(b"\xff\x82\xa0\r\n");
        assert_eq!(fs::read(&path).unwrap(), expected);
    }
}
#[test]
fn stdin_uses_display_path_excludes_and_never_writes() {
    use std::{io::Write, process::Stdio};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "original bytes\n").unwrap();
    let source =
        "# rustocop:disable Migration/Nonexistent\nwork\n# rustocop:enable Migration/Nonexistent\n";
    let only = format!("{SCOPE},{BAN}");
    for (config, code) in [("", 1), ("exclude = [\"lib/a.rb\"]\n", 0)] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", &only, "--stdin", "lib/a.rb"])
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
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(code), "{out:?}");
        assert!(out.stderr.is_empty(), "{out:?}");
        if code == 1 {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains(SCOPE) && text.contains(BAN), "{text}");
        }
        assert_eq!(fs::read(&path).unwrap(), b"original bytes\n");
    }
    cli(
        dir.path(),
        &["--only", BAN, "--stdin", "lib/a.rb", "--fix"],
        2,
    );
    assert_eq!(fs::read(&path).unwrap(), b"original bytes\n");
}

#[test]
fn withheld_shebang_headers_active_ranges_and_errors_are_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "#!/usr/bin/env ruby # rubocop:disable Migration/Nonexistent\np __LINE__\n",
        "# rubocop:disable Migration/Nonexistent -- frozen_string_literal: true\np \"a\".frozen?\n",
        "# rubocop:disable Lint/Debugger\nbinding.pry\n# rubocop:enable Lint/Debugger\n",
        "=begin\n# rubocop:disable Migration/Nonexistent\n=end\np __LINE__\n",
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", BAN, "--fix", "lib/a.rb"], 1);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    // A paired edit would have to copy non-UTF-8 interior bytes through the
    // UTF-8 correction representation. Withhold the entire pair, not half.
    let raw = b"# coding: ASCII-8BIT\r\n# rubocop:push -Migration/Nonexistent\r\np \"\xff\"\r\n# rubocop:pop\r\n";
    fs::write(&path, raw).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", BAN, "--fix", "lib/a.rb"], 1);
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    let source = "# rubocop:disable Migration/Nonexistent\ndef broken(\n";
    fs::write(&path, source).unwrap();
    cli(dir.path(), &["--only", BAN, "--fix", "lib/a.rb"], 2);
    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
}

/// Once Migration/DepartmentName landed its name is an active
/// selector for both Style directive policies: reported, never removed.
#[test]
fn landed_department_name_is_an_active_selector_and_never_removed() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:enable Migration/DepartmentName\n"),
            format!("work # {prefix}:todo Migration/DepartmentName\n"),
            format!("# {prefix}:push -Migration/DepartmentName\nwork\n# {prefix}:pop\n"),
            format!(
                "# {prefix}:disable Migration/DepartmentName\nwork\n# {prefix}:enable Migration/DepartmentName\n"
            ),
        ] {
            fs::write(&path, &source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            let out = cli(dir.path(), &["--only", BAN, "--fix", "lib/a.rb"], 1);
            assert!(
                String::from_utf8_lossy(&out.stdout).contains("directives are not permitted"),
                "{out:?}"
            );
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source}");
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}
