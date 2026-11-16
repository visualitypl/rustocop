//! real selectors/config/directives, no-write safety and Ruby flow witnesses.
//! Pinned examples: RuboCop MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const MODIFIER: &str = "Style/RescueModifier";
const STANDARD: &str = "Style/RescueStandardError";
const BOTH: &str = "Style/RescueModifier,Style/RescueStandardError";
const SOURCE: &str = "begin\n  work rescue fallback\nrescue\n  handler\nend\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, counts: [usize; 2]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    for (cop, count) in [MODIFIER, STANDARD].into_iter().zip(counts) {
        assert_eq!(
            text.lines()
                .filter(|line| line.ends_with(&format!("({cop})")))
                .count(),
            count,
            "{args:?}: {text}"
        );
    }
    out
}
fn unchanged(dir: &Path, args: &[&str], source: &[u8], status: i32, counts: [usize; 2]) {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir, args, status, counts);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn defaults_config_selectors_and_no_fix_never_write() {
    let dir = project();
    for (config, args, status, counts) in [
        (String::new(), vec!["a.rb"], 1, [1, 1]),
        (String::new(), vec!["--only", MODIFIER, "a.rb"], 1, [1, 0]),
        (String::new(), vec!["--only", STANDARD, "a.rb"], 1, [0, 1]),
        (
            String::new(),
            vec!["--only", BOTH, "--except", MODIFIER, "a.rb"],
            1,
            [0, 1],
        ),
        (
            String::new(),
            vec!["--only", BOTH, "--except", BOTH, "a.rb"],
            0,
            [0, 0],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["a.rb"],
            0,
            [0, 0],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", BOTH, "a.rb"],
            1,
            [1, 1],
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{STANDARD}\"]\nenabled = true\n"),
            vec!["a.rb"],
            1,
            [0, 1],
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(dir.path(), &args, SOURCE.as_bytes(), status, counts);
    }
}
#[test]
fn excludes_and_both_directive_prefixes_suppress_reports_and_edits() {
    let dir = project();
    for cop in [MODIFIER, STANDARD] {
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['**/**']\n"),
            "exclude = ['a.rb']\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                &["--only", cop, "--fix", "a.rb"],
                SOURCE.as_bytes(),
                0,
                [0, 0],
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for source in [
                format!("# {prefix}:disable {cop}\n{SOURCE}"),
                SOURCE.replace(
                    if cop == MODIFIER {
                        "work rescue fallback"
                    } else {
                        "rescue\n"
                    },
                    &format!(
                        "{} # {prefix}:disable {cop}\n",
                        if cop == MODIFIER {
                            "work rescue fallback"
                        } else {
                            "rescue"
                        }
                    ),
                ),
                format!(
                    "# {prefix}:disable-next {cop}\n{}",
                    if cop == MODIFIER {
                        "1 rescue nil\n"
                    } else {
                        "begin; work; rescue; handler; end\n"
                    }
                ),
            ] {
                unchanged(
                    dir.path(),
                    &["--only", cop, "--fix", "a.rb"],
                    source.as_bytes(),
                    0,
                    [0, 0],
                );
            }
        }
    }
}
#[test]
fn stdin_reports_both_cops_without_writes_and_rejects_fix() {
    use std::process::Stdio;
    let dir = project();
    let path = dir.path().join("a.rb");
    fs::write(&path, "untouched\n").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", BOTH, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), SOURCE.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("a.rb:2:3") && text.contains(MODIFIER));
    assert!(text.contains("a.rb:3:1") && text.contains(STANDARD));
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", BOTH, "--fix", "--stdin", "a.rb"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), b"untouched\n");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn fixed_options_are_rejected_and_disabled_fixes_keep_bytes() {
    let dir = project();
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{STANDARD}\"]\nenforced_style = 'implicit'\n"),
    )
    .unwrap();
    unchanged(
        dir.path(),
        &["--only", STANDARD, "--fix", "a.rb"],
        SOURCE.as_bytes(),
        2,
        [0, 0],
    );
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.Style]\nenabled = false\n",
    )
    .unwrap();
    unchanged(dir.path(), &["--fix", "a.rb"], SOURCE.as_bytes(), 0, [0, 0]);
}
#[test]
fn explicit_passes_stabilize_and_incompatible_companion_defers_atomically() {
    let dir = project();
    unchanged(
        dir.path(),
        &[
            "--only",
            "Style/RescueModifier,Style/MethodCallWithArgsParentheses",
            "--fix",
            "--fix-passes",
            "1",
            "a.rb",
        ],
        b"1 rescue nil\n",
        1,
        [1, 0],
    );
    fs::write(dir.path().join("a.rb"), "1 rescue nil\n").unwrap();
    run(
        dir.path(),
        &["--only", BOTH, "--fix", "--fix-passes", "1", "a.rb"],
        1,
        [0, 1],
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        "begin\n  1\nrescue\n  nil\nend\n"
    );
    run(
        dir.path(),
        &["--only", BOTH, "--fix", "--fix-passes", "1", "a.rb"],
        0,
        [0, 0],
    );
    unchanged(
        dir.path(),
        &["--only", BOTH, "--fix", "--fix-passes", "1", "a.rb"],
        b"begin\n  1\nrescue StandardError\n  nil\nend\n",
        0,
        [0, 0],
    );
    fs::write(
        dir.path().join("a.rb"),
        "begin\n  'x' \nrescue\n  'y'\nend\n",
    )
    .unwrap();
    run(
        dir.path(),
        &[
            "--only",
            "Style/RescueStandardError,Style/StringLiterals,Layout/TrailingWhitespace",
            "--fix",
            "--fix-passes",
            "1",
            "a.rb",
        ],
        0,
        [0, 0],
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        "begin\n  \"x\"\nrescue StandardError\n  \"y\"\nend\n"
    );
}
fn runtimes() -> Vec<String> {
    let mut result = Vec::new();
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        result.push("ruby".into());
    }
    if let Ok(native) = crate::support::runtime::ruby()
        && !result.contains(&native)
    {
        result.push(native);
    }
    result
}
#[test]
fn closed_literal_canonical_fixes_preserve_decoded_results() {
    let dir = project();
    for source in [
        "1 rescue nil",
        "false rescue true",
        "\"a\\x82\\n\" rescue :fallback",
        "%q(a\nb) rescue nil",
    ] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        run(
            dir.path(),
            &["--only", MODIFIER, "--fix", "a.rb"],
            0,
            [0, 0],
        );
        let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
        for runtime in runtimes() {
            let eval = |text: &str| {
                Command::new(&runtime)
                    .args([
                        "-e",
                        "p eval(ARGV[0], TOPLEVEL_BINDING, 'literal.rb', 1)",
                        text,
                    ])
                    .output()
                    .unwrap()
            };
            let before = eval(source);
            let fixed = eval(&after);
            assert!(before.status.success() && fixed.status.success());
            assert_eq!((before.stdout, before.stderr), (fixed.stdout, fixed.stderr));
        }
    }
}
