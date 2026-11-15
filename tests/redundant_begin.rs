//! real CLI controls and controlled Ruby binding/backtrace witnesses.
//! Pinned fixture examples: RuboCop MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/RedundantBegin";
const SOURCE: &str = "begin\n  work\nend\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, count: usize) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        text.lines()
            .filter(|l| l.ends_with(&format!("({COP})")))
            .count(),
        count,
        "{args:?}: {text}"
    );
    out
}
fn unchanged(dir: &Path, args: &[&str], source: &[u8], status: i32, count: usize) {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir, args, status, count);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn defaults_selectors_config_and_no_fix_preserve_bytes() {
    let dir = project();
    for (config, args, status, count) in [
        (String::new(), vec!["a.rb"], 1, 1),
        (String::new(), vec!["--only", COP, "a.rb"], 1, 1),
        (
            String::new(),
            vec!["--only", COP, "--except", COP, "--fix", "a.rb"],
            0,
            0,
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--fix", "a.rb"],
            0,
            0,
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", COP, "a.rb"],
            1,
            1,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--fix", "a.rb"],
            0,
            0,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec!["a.rb"],
            1,
            1,
        ),
        (
            format!("[cops.\"{COP}\"]\nmax = 40\n"),
            vec!["--only", COP, "--fix", "a.rb"],
            2,
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(dir.path(), &args, SOURCE.as_bytes(), status, count);
    }
}
#[test]
fn excludes_and_both_directive_prefixes_suppress_reports_and_edits() {
    let dir = project();
    for config in [
        format!("[cops.\"{COP}\"]\nexclude = ['a.rb']\n"),
        "exclude = ['a.rb']\n".into(),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            &["--only", COP, "--fix", "a.rb"],
            SOURCE.as_bytes(),
            0,
            0,
        );
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{SOURCE}"),
            format!("# {prefix}:disable-next {COP}\n{SOURCE}"),
            format!("begin # {prefix}:disable {COP}\n  work\nend\n"),
        ] {
            unchanged(
                dir.path(),
                &["--only", COP, "--fix", "a.rb"],
                source.as_bytes(),
                0,
                0,
            );
        }
        // The highlight stays enabled, but the atomic edit crosses a disabled span.
        let source =
            format!("begin\n# {prefix}:disable {COP}\n  work\n# {prefix}:enable {COP}\nend\n");
        unchanged(
            dir.path(),
            &["--only", COP, "--fix", "a.rb"],
            source.as_bytes(),
            1,
            1,
        );
    }
}
#[test]
fn every_pinned_example_runs_with_real_config_and_canonical_or_held_fix() {
    let dir = project();
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/redundant_begin_pinned.json")).unwrap();
    let mut totals = [0; 3];
    for case in cases {
        let before = case["source"].as_str().unwrap();
        let line = case["spec_line"].as_u64().unwrap();
        let n = case["positions"].as_array().unwrap().len();
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"Layout/LineLength\"]\nmax = {}\n", case["max"]),
        )
        .unwrap();
        unchanged(
            dir.path(),
            &["--only", COP, "a.rb"],
            before.as_bytes(),
            i32::from(n > 0),
            n,
        );
        if let Some(after) = case["correction"].as_str() {
            if matches!(
                line,
                211 | 232
                    | 250
                    | 283
                    | 298
                    | 845
                    | 869
                    | 999
                    | 1037
                    | 1051
                    | 1065
                    | 1079
                    | 1097
                    | 1116
            ) {
                unchanged(
                    dir.path(),
                    &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
                    before.as_bytes(),
                    1,
                    n,
                );
                totals[0] += 1;
            } else {
                fs::write(dir.path().join("a.rb"), before).unwrap();
                if matches!(line, 886 | 923 | 961) {
                    run(
                        dir.path(),
                        &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
                        1,
                        1,
                    );
                    let first = before.replacen("  begin\n", "  \n", 1).replacen(
                        "  end\nend\n",
                        "  \nend\n",
                        1,
                    );
                    assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), first);
                    totals[1] += 1;
                } else {
                    totals[2] += 1;
                }
                run(
                    dir.path(),
                    &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
                    0,
                    0,
                );
                assert_eq!(
                    fs::read_to_string(dir.path().join("a.rb")).unwrap(),
                    after,
                    "spec {line}"
                );
                unchanged(
                    dir.path(),
                    &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
                    after.as_bytes(),
                    0,
                    0,
                );
            }
        }
    }
    assert_eq!(totals, [15, 3, 20]);
}
#[test]
fn stdin_reports_positions_without_writes_and_rejects_fix() {
    use std::process::Stdio;
    let dir = project();
    fs::write(dir.path().join("a.rb"), "untouched\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), SOURCE.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("a.rb:1:1") && text.contains("Redundant `begin` block detected."));
    run(
        dir.path(),
        &["--only", COP, "--fix", "--stdin", "a.rb"],
        2,
        0,
    );
    assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), b"untouched\n");
}

#[test]
fn line_length_dependency_is_configured_not_selected() {
    let dir = project();
    let source = "x = begin\n  very_long_name if condition\nend\n";
    for (config, count) in [
        ("[cops.\"Layout/LineLength\"]\nmax = 15\n", 0),
        (
            "[cops.\"Layout/LineLength\"]\nmax = 15\nenabled = false\n",
            1,
        ),
        ("", 1),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            &["--only", COP, "a.rb"],
            source.as_bytes(),
            i32::from(count > 0),
            count,
        );
    }
}
#[test]
fn incompatible_peer_and_mixed_style_layout_passes_are_deterministic() {
    let dir = project();
    unchanged(
        dir.path(),
        &[
            "--only",
            "Style/RedundantBegin,Style/BlockDelimiters",
            "--fix",
            "a.rb",
        ],
        b"proc do\n  begin\n    work\n  end\nend\n",
        1,
        1,
    );
    let selected = "Style/RedundantBegin,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    fs::write(
        dir.path().join("a.rb"),
        "def f\n  begin\n    return ('x')\n  end\nend\n",
    )
    .unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let old = fs::read(dir.path().join("a.rb")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", selected, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
        if old == fs::read(dir.path().join("a.rb")).unwrap() {
            stable = true;
            break;
        }
    }
    assert!(stable);
    assert!(
        !fs::read_to_string(dir.path().join("a.rb"))
            .unwrap()
            .contains("begin")
    );
}
fn runtimes() -> Vec<String> {
    let mut result = vec![];
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
fn controlled_ruby_rescue_ancestry_retry_ensure_returns_and_backtraces() {
    let dir = project();
    // Keep the controlled observer outside the linted source: the shared
    // backtrace/source-observation barrier intentionally withholds all edits
    // when a backtrace selector appears in the main AST.
    fs::write(dir.path().join("observer.rb"), "def observe_error(e); [e.class.name,e.message,e.backtrace.first.split(':')[1],$!.equal?(e)]; end\n").unwrap();
    for source in [
        "def f\n  begin\n    raise 'bad'\n  rescue => e\n    observe_error(e)\n  ensure\n    puts :ensure\n  end\nend\np f\n",
        "def self.f\n  begin\n    raise Exception, 'outer'\n  rescue StandardError\n    :wrong\n  ensure\n    puts :ensure\n  end\nend\nbegin; f; rescue Exception => e; p observe_error(e); end\n",
        "def f\n  begin\n    return :returned\n  ensure\n    puts :ensure\n  end\nend\np f\n",
        "def f(x=raise('parameter'))\n  begin\n    :body\n  rescue\n    :wrong\n  ensure\n    puts :ensure\n  end\nend\nbegin; p f; rescue => e; p observe_error(e); end\n",
        "p(proc do\n  begin\n    raise 'proc'\n  rescue => e\n    e.message\n  ensure\n    puts :ensure\n  end\nend.call)\n",
        "p -> do\n  begin\n    raise 'lambda'\n  rescue => e\n    e.message\n  end\nend.call\n",
        "attempt = 0\np(proc do\n  begin\n    attempt += 1\n    raise 'retry' if attempt < 2\n    :done\n  rescue\n    retry\n  else\n    :else\n  ensure\n    puts attempt\n  end\nend.call)\n",
        "def f\n  begin\n    :normal\n  rescue\n    :wrong\n  else\n    :else_value\n  ensure\n    puts :ensure\n  end\nend\np f\n",
        "p(proc do\n  begin\n    next :next_value\n  ensure\n    puts :ensure\n  end\nend.call)\np(1.times do\n  begin\n    break :break_value\n  ensure\n    puts :break_ensure\n  end\nend)\n",
        "p(proc { x = begin 1 and false end; [x,__LINE__] }.call)\n",
        "begin\n  p [__LINE__,\"é\\n雪\",%q(a\nb),{value: :ok}]\nend\np __LINE__\n__END__\nraw payload\n",
        "def f\n  begin\n    raise 'scope'\n  rescue => e\n    observe_error(e)\n  end\n  :after\nend\np f\n",
    ] {
        for runtime in runtimes() {
            fs::write(dir.path().join("a.rb"), source).unwrap();
            let observe = || {
                Command::new(&runtime)
                    .current_dir(dir.path())
                    .args(["-r", "./observer.rb", "a.rb"])
                    .output()
                    .unwrap()
            };
            let before = observe();
            assert!(before.status.success(), "{runtime}: {before:?}\n{source}");
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", COP, "--fix", "a.rb"])
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            let after = observe();
            if !source.starts_with("p ->") && !source.contains(":after") {
                assert_ne!(
                    fs::read_to_string(dir.path().join("a.rb")).unwrap(),
                    source,
                    "positive runtime witness must actually correct: {source}"
                );
            }
            assert_eq!(
                (before.status.code(), before.stdout, before.stderr),
                (after.status.code(), after.stdout, after.stderr),
                "{runtime}: {source}"
            );
        }
    }
}
#[test]
fn naive_assignment_hoisting_changes_backtrace_lines_but_our_fix_is_withheld() {
    let dir = project();
    let source = "def f\n x = begin\n  raise 'bad'\n end\nrescue => e\n p e.backtrace.first.split(':')[1]\nend\nf\n";
    let naive = source
        .replace("begin\n  raise", "raise")
        .replace(" end\nrescue", " \nrescue");
    for runtime in runtimes() {
        let observe = |text| {
            fs::write(dir.path().join("witness.rb"), text).unwrap();
            let out = Command::new(&runtime)
                .current_dir(dir.path())
                .arg("witness.rb")
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
            out.stdout
        };
        assert_eq!(observe(source), b"\"3\"\n");
        assert_eq!(observe(&naive), b"\"2\"\n");
    }
    unchanged(
        dir.path(),
        &["--only", COP, "--fix", "a.rb"],
        source.as_bytes(),
        1,
        1,
    );
}
