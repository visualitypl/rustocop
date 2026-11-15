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
fn every_pinned_example_and_correction_runs_through_real_cli() {
    let dir = project();
    let mut totals = [0, 0];
    for (index, cop, fixture) in [
        (
            0,
            MODIFIER,
            include_str!("fixtures/rescue_modifier_pinned.json"),
        ),
        (
            1,
            STANDARD,
            include_str!("fixtures/rescue_standard_error_pinned.json"),
        ),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for case in cases {
            let before = case["source"].as_str().unwrap();
            let positions: std::collections::BTreeSet<String> = case["positions"]
                .as_array()
                .unwrap()
                .iter()
                .map(ToString::to_string)
                .collect();
            let n = positions.len();
            let mut counts = [0, 0];
            counts[index] = n;
            let config = if index == 0 && case["spec_line"] == 404 {
                format!("[cops.\"{cop}\"]\nexclude = ['**/**']\n")
            } else {
                String::new()
            };
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                &["--only", cop, "a.rb"],
                before.as_bytes(),
                i32::from(n > 0),
                counts,
            );
            if let Some(after) = case["correction"].as_str() {
                totals[index] += 1;
                if index == 0 {
                    unchanged(
                        dir.path(),
                        &["--only", cop, "--fix", "a.rb"],
                        before.as_bytes(),
                        1,
                        counts,
                    );
                } else {
                    fs::write(dir.path().join("a.rb"), before).unwrap();
                    run(dir.path(), &["--only", cop, "--fix", "a.rb"], 0, [0, 0]);
                    assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), after);
                    unchanged(
                        dir.path(),
                        &["--only", cop, "--fix", "a.rb"],
                        after.as_bytes(),
                        0,
                        [0, 0],
                    );
                }
            }
        }
    }
    assert_eq!(totals, [15, 4]);
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
fn controlled_runtime_exception_ancestry_scope_ensure_results_and_backtraces() {
    let dir = project();
    for source in [
        "def f\n  raise 'bad'\nrescue => e\n  [e.class.name, e.message, e.backtrace.first.split(':')[1]]\nensure\n  puts :ensured\nend\np f\n",
        "def f\n  begin\n    raise Exception, 'outer'\n  rescue\n    :wrong\n  end\nrescue Exception => e\n  [e.class.name, e.message]\nend\np f\n",
        "p -> do\n  raise 'lambda'\nrescue => e\n  [e.message, $!.equal?(e)]\nensure\n  puts :lambda_ensure\nend.call\np(proc do\n  raise 'proc'\nrescue => e\n  e.message\nend.call)\n",
        "p(begin\n  :normal\nrescue\n  :wrong\nelse\n  :else_value\nensure\n  puts :ensure\nend)\n",
        "def f\n  ((raise 'inner') rescue $!)\nrescue\n  :outer\nensure\n  puts :ensure\nend\ne = f\np [e.class.name, e.backtrace.first.split(':')[1]]\n",
        "module Shadow\n  StandardError = Exception\n  def self.f\n    raise Exception, 'not caught'\n  rescue\n    :wrong\n  end\nend\nbegin; Shadow.f; rescue Exception => e; puts e.message; end\n",
    ] {
        for runtime in runtimes() {
            fs::write(dir.path().join("a.rb"), source).unwrap();
            let original = Command::new(&runtime)
                .current_dir(dir.path())
                .arg("a.rb")
                .output()
                .unwrap();
            assert!(
                original.status.success(),
                "{runtime}: {original:?}\n{source}"
            );
            let lint = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", BOTH, "--fix", "a.rb"])
                .output()
                .unwrap();
            assert!(matches!(lint.status.code(), Some(0 | 1)), "{lint:?}");
            let corrected = Command::new(&runtime)
                .current_dir(dir.path())
                .arg("a.rb")
                .output()
                .unwrap();
            assert_eq!(
                (corrected.status.code(), corrected.stdout, corrected.stderr),
                (original.status.code(), original.stdout, original.stderr),
                "{runtime}: {source}"
            );
        }
    }
}
#[test]
fn naive_multiline_and_shadowed_constant_rewrites_have_observable_counterexamples() {
    let dir = project();
    for (before, naive, cop, expected_before, expected_naive) in [
        (
            "e = ((raise 'bad') rescue $!)\np e.backtrace.first.split(':')[1]\n",
            "e = begin\n  raise 'bad'\nrescue\n  $!\nend\np e.backtrace.first.split(':')[1]\n",
            MODIFIER,
            "\"1\"\n",
            "\"2\"\n",
        ),
        (
            "module Shadow\n  StandardError = Exception\n  def self.f\n    raise Exception\n  rescue\n    :caught\n  end\nend\nbegin; p Shadow.f; rescue Exception; puts :escaped; end\n",
            "module Shadow\n  StandardError = Exception\n  def self.f\n    raise Exception\n  rescue StandardError\n    :caught\n  end\nend\nbegin; p Shadow.f; rescue Exception; puts :escaped; end\n",
            STANDARD,
            "escaped\n",
            ":caught\n",
        ),
    ] {
        for runtime in runtimes() {
            let observe = |source: &str| {
                fs::write(dir.path().join("witness.rb"), source).unwrap();
                let out = Command::new(&runtime)
                    .current_dir(dir.path())
                    .arg("witness.rb")
                    .output()
                    .unwrap();
                assert!(out.status.success(), "{runtime}: {out:?}");
                String::from_utf8(out.stdout).unwrap()
            };
            assert_eq!(observe(before), expected_before);
            assert_eq!(observe(naive), expected_naive);
        }
        unchanged(
            dir.path(),
            &["--only", cop, "--fix", "a.rb"],
            before.as_bytes(),
            1,
            if cop == MODIFIER { [1, 0] } else { [0, 1] },
        );
    }
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
