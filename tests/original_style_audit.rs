//! preserve the original eight policies while the Style registry grows.
//! Canonical examples adapted from RuboCop's corresponding Style specs
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

struct Case {
    cop: &'static str,
    before: &'static str,
    after: &'static str,
    position: &'static str,
    message: &'static str,
}

const CASES: &[Case] = &[
    Case {
        cop: "Style/FrozenStringLiteralComment",
        before: "puts 1\n",
        after: "puts 1\n", // Unsafe upstream correction is deliberately unavailable.
        position: "1:1",
        message: "Missing frozen string literal comment.",
    },
    Case {
        cop: "Style/HashSyntax",
        before: "x = { :a => 0 }\n",
        after: "x = { a: 0 }\n",
        position: "1:7",
        message: "Use the new Ruby 1.9 hash syntax.",
    },
    Case {
        cop: "Style/RedundantParentheses",
        before: "x = (1)\n",
        after: "x = 1\n",
        position: "1:5",
        message: "Don't use parentheses around a literal.",
    },
    Case {
        cop: "Style/RedundantReturn",
        before: "def f\n  return 1\nend\n",
        after: "def f\n  1\nend\n",
        position: "2:3",
        message: "Redundant `return` detected.",
    },
    Case {
        cop: "Style/Semicolon",
        before: "puts 1;\n",
        after: "puts 1\n",
        position: "1:7",
        message: "Do not use semicolons to terminate expressions.",
    },
    Case {
        cop: "Style/SignalException",
        before: "Kernel.fail\n",
        after: "Kernel.raise\n",
        position: "1:8",
        message: "Always use `raise` to signal exceptions.",
    },
    Case {
        cop: "Style/StringLiterals",
        before: "s = 'abc'\n",
        after: "s = \"abc\"\n",
        position: "1:5",
        message: "Prefer double-quoted strings unless you need single quotes to avoid extra backslashes for escaping.",
    },
    Case {
        cop: "Style/StringLiteralsInInterpolation",
        before: "\"#{'A'}\"\n",
        after: "\"#{\"A\"}\"\n",
        position: "1:4",
        message: "Prefer double-quoted strings inside interpolations.",
    },
];

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn lint(cwd: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "{args:?}: {output:?}"
    );
    assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    output
}

fn cli(cwd: &Path, args: &[&str], status: i32) -> Output {
    let output = lint(cwd, args);
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    output
}

#[test]
fn original_names_keep_their_defaults_and_exactly_seven_safe_capabilities() {
    let registry = rustocop::cops::Registry::new();
    for case in CASES {
        let matches: Vec<_> = registry
            .cops()
            .iter()
            .filter(|cop| cop.name() == case.cop)
            .collect();
        assert_eq!(matches.len(), 1, "{}", case.cop);
        let frozen = case.cop == "Style/FrozenStringLiteralComment";
        assert_eq!(matches[0].default_enabled(), !frozen, "{}", case.cop);
        assert_eq!(matches[0].safe_fixes(), !frozen, "{}", case.cop);
    }
}

#[test]
fn every_original_has_selective_cli_positions_and_config_precedence() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for case in CASES {
        fs::write(&path, case.before).unwrap();
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        let default = lint(dir.path(), &["lib/a.rb"]);
        assert_eq!(
            String::from_utf8_lossy(&default.stdout).contains(&format!("({})", case.cop)),
            case.cop != "Style/FrozenStringLiteralComment",
            "{default:?}"
        );
        for config in [
            format!("[cops.\"{}\"]\nenabled = false\n", case.cop),
            "[cops.Style]\nenabled = false\n".to_owned(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            // Other registered cops may also report the canonical snippet.
            // Assert this cop's absence, not that the whole file is clean.
            let disabled = lint(dir.path(), &["lib/a.rb"]);
            assert!(
                !String::from_utf8_lossy(&disabled.stdout).contains(&format!("({})", case.cop)),
                "{disabled:?}"
            );
            let output = cli(dir.path(), &["--only", case.cop, "lib/a.rb"], 1);
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(
                text.contains(&format!(
                    "lib/a.rb:{}: C: {} ({})",
                    case.position, case.message, case.cop
                )),
                "{text}"
            );
            assert!(text.contains("1 offense detected"), "{text}");
            cli(
                dir.path(),
                &[
                    "--only", case.cop, "--except", case.cop, "--fix", "lib/a.rb",
                ],
                0,
            );
            assert_eq!(fs::read(&path).unwrap(), case.before.as_bytes());
        }
        // Per-cop opt-in overrides a disabled department, including frozen comments.
        fs::write(
            dir.path().join("rustocop.toml"),
            format!(
                "[cops.Style]\nenabled = false\n[cops.\"{}\"]\nenabled = true\n",
                case.cop
            ),
        )
        .unwrap();
        let output = cli(dir.path(), &["lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&output.stdout).contains(case.cop));
        assert_eq!(fs::read(&path).unwrap(), case.before.as_bytes());
    }
}

#[test]
fn canonical_fixes_preserve_bom_crlf_line_observations_and_end_data() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for case in CASES {
        let wrap = |code: &str| {
            format!(
                "\u{feff}# encoding: UTF-8\r\n{}p __LINE__\r\n__END__\r\nraw 'data'; :a => 1\r\n",
                code.replace('\n', "\r\n")
            )
        };
        let before = wrap(case.before);
        let after = wrap(case.after);
        fs::write(&path, &before).unwrap();
        cli(dir.path(), &["--only", case.cop, "lib/a.rb"], 1);
        assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        let frozen = case.cop == "Style/FrozenStringLiteralComment";
        let output = cli(
            dir.path(),
            &["--only", case.cop, "--fix", "lib/a.rb"],
            i32::from(frozen),
        );
        let expected = if frozen {
            "0 corrections applied, 1 offense remaining"
        } else {
            "1 correction applied, 0 offenses remaining"
        };
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(expected),
            "{output:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes(), "{}", case.cop);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &["--only", case.cop, "--fix", "lib/a.rb"],
            i32::from(frozen),
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn both_directive_prefixes_protect_every_original_even_when_force_selected() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for case in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [case.cop, "Style"] {
                let source = format!("# {prefix}:disable {target}\n{}", case.before);
                fs::write(&path, &source).unwrap();
                cli(dir.path(), &["--only", case.cop, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            }
        }
    }
}

#[test]
fn every_original_obeys_additive_config_relative_exclusions_before_fixing() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for case in CASES {
        fs::write(&path, case.before).unwrap();
        // One exclusion misses while the other matches: neither replaces the other.
        for (global, per_cop) in [("lib/a.rb", "other.rb"), ("other.rb", "lib/a.rb")] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!(
                    "exclude = [\"{global}\"]\n[cops.\"{}\"]\nexclude = [\"{per_cop}\"]\n",
                    case.cop
                ),
            )
            .unwrap();
            cli(
                &dir.path().join("lib"),
                &["--only", case.cop, "--fix", "../lib/a.rb"],
                0,
            );
            assert_eq!(fs::read(&path).unwrap(), case.before.as_bytes());
        }
    }
}

#[test]
fn opting_into_frozen_comments_does_not_change_the_other_seven_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let before: String = CASES.iter().map(|case| case.before).collect();
    let after: String = CASES.iter().map(|case| case.after).collect();
    let only = CASES
        .iter()
        .map(|case| case.cop)
        .collect::<Vec<_>>()
        .join(",");
    fs::write(&path, &before).unwrap();
    let output = cli(dir.path(), &["--only", &only, "lib/a.rb"], 1);
    let text = String::from_utf8_lossy(&output.stdout);
    for case in CASES {
        assert!(text.contains(case.cop), "{text}");
    }
    assert!(text.contains("8 offenses detected"), "{text}");
    assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
    let output = cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 1);
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("7 corrections applied, 1 offense remaining"),
        "{output:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
    cli(
        dir.path(),
        &[
            "--only",
            &only,
            "--except",
            "Style/FrozenStringLiteralComment",
            "--fix",
            "lib/a.rb",
        ],
        0,
    );
    assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
}
