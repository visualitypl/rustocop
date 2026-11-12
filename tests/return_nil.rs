//! real policy controls, canonical/no-write fixes and Ruby flow witnesses.
//! Pinned fixtures port RuboCop specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const RETURN: &str = "Style/ReturnNil";
const PREDICATE: &str = "Style/ReturnNilInPredicateMethodDefinition";
const BOTH: &str = "Style/ReturnNil,Style/ReturnNilInPredicateMethodDefinition";
const SOURCE: &str = "def f?\n  return nil if condition\n  true\nend\n";
/// The pinned source's guard clause is also a default-on
/// Layout/EmptyLineAfterGuardClause finding; it is not the subject here.
const GUARD_OFF: &str = "[cops.\"Layout/EmptyLineAfterGuardClause\"]\nenabled = false\n";
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
    for (cop, count) in [RETURN, PREDICATE].into_iter().zip(counts) {
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
fn default_opt_in_config_and_selectors_never_write_without_fix() {
    let dir = project();
    for (config, args, status, counts) in [
        (String::new(), vec!["a.rb"], 1, [0, 1]),
        (String::new(), vec!["--only", RETURN, "a.rb"], 1, [1, 0]),
        (String::new(), vec!["--only", BOTH, "a.rb"], 1, [1, 1]),
        (
            String::new(),
            vec!["--only", BOTH, "--except", RETURN, "a.rb"],
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
            format!("[cops.\"{RETURN}\"]\nenabled = true\n"),
            vec!["a.rb"],
            1,
            [1, 1],
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{RETURN}\"]\nenabled = true\n"),
            vec!["a.rb"],
            1,
            [1, 0],
        ),
        (
            format!("[cops.\"{PREDICATE}\"]\nenabled = false\n"),
            vec!["a.rb"],
            0,
            [0, 0],
        ),
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("{config}{GUARD_OFF}"),
        )
        .unwrap();
        unchanged(dir.path(), &args, SOURCE.as_bytes(), status, counts);
    }
}
#[test]
fn fixed_options_are_strict_and_config_disabled_fixes_do_not_write() {
    let dir = project();
    for (cop, option) in [
        (RETURN, "enforced_style = 'return_nil'"),
        (PREDICATE, "allowed_methods = ['f?']"),
        (PREDICATE, "allowed_patterns = ['f']"),
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            &["--only", cop, "--fix", "a.rb"],
            SOURCE.as_bytes(),
            2,
            [0, 0],
        );
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{PREDICATE}\"]\nenabled = false\n{GUARD_OFF}"),
    )
    .unwrap();
    unchanged(dir.path(), &["--fix", "a.rb"], SOURCE.as_bytes(), 0, [0, 0]);
}
#[test]
fn config_excludes_and_both_directive_prefixes_suppress_fixes() {
    let dir = project();
    for cop in [RETURN, PREDICATE] {
        for prefix in ["rustocop", "rubocop"] {
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            for source in [
                format!("# {prefix}:disable {cop}\n{SOURCE}"),
                SOURCE.replace(
                    "return nil if condition",
                    &format!("return nil if condition # {prefix}:disable {cop}"),
                ),
                SOURCE.replace(
                    "  return",
                    &format!("  # {prefix}:disable-next {cop}\n  return"),
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
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['a.rb']\n"),
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
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        let source = format!("# {prefix}:enable {RETURN}\n{SOURCE}");
        unchanged(dir.path(), &["a.rb"], source.as_bytes(), 1, [1, 1]);
    }
}
#[test]
fn every_pinned_correction_runs_through_real_cli() {
    let dir = project();
    for (cop, fixture, expected) in [
        (RETURN, include_str!("fixtures/return_nil_pinned.json"), 1),
        (
            PREDICATE,
            include_str!("fixtures/return_nil_in_predicate_method_definition_pinned.json"),
            17,
        ),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        let mut calls = 0;
        for case in cases {
            let Some(after) = case["correction"].as_str() else {
                continue;
            };
            let before = case["source"].as_str().unwrap();
            if cop == RETURN {
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
            } else {
                unchanged(
                    dir.path(),
                    &["--only", cop, "--fix", "a.rb"],
                    before.as_bytes(),
                    1,
                    [0, 1],
                );
            }
            calls += 1;
        }
        assert_eq!(calls, expected);
    }
}
#[test]
fn shared_offense_safe_fix_does_not_apply_unsafe_false_rewrite() {
    let dir = project();
    fs::write(dir.path().join("a.rb"), SOURCE).unwrap();
    run(dir.path(), &["--only", BOTH, "--fix", "a.rb"], 1, [0, 1]);
    let after = SOURCE.replace("return nil", "return");
    assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), after);
    unchanged(
        dir.path(),
        &["--only", BOTH, "--fix", "a.rb"],
        after.as_bytes(),
        1,
        [0, 1],
    );
}
#[test]
fn source_envelopes_literals_lines_data_payload_and_legacy_bytes_are_preserved() {
    let dir = project();
    for (before, after) in [
        (b"\xef\xbb\xbfdef f\r\n  return nil # keep\r\nend\r\np __LINE__\r\n__END__\r\nreturn nil\x82\r\n".as_slice(), b"\xef\xbb\xbfdef f\r\n  return # keep\r\nend\r\np __LINE__\r\n__END__\r\nreturn nil\x82\r\n".as_slice()),
        (b"# coding: Shift_JIS\n# \x82\xa0\ndef f\n  return nil\nend\n", b"# coding: Shift_JIS\n# \x82\xa0\ndef f\n  return\nend\n"),
        (b"def f\n  s = %q(return nil)\n  return nil if s\nend\n", b"def f\n  s = %q(return nil)\n  return if s\nend\n"),
    ] {
        fs::write(dir.path().join("a.rb"), before).unwrap();
        run(dir.path(), &["--only", RETURN, "--fix", "a.rb"], 0, [0,0]);
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), after);
    }
}
#[test]
fn source_observation_and_multiline_holds_keep_findings_and_bytes() {
    let dir = project();
    for source in [
        "def f\nreturn nil\nend\np DATA.pos\n__END__\npayload\n",
        "def f\nreturn nil\nend\np File.read(__FILE__)\n",
        "def f\nreturn \\\n nil\nend\np __LINE__\n",
    ] {
        unchanged(
            dir.path(),
            &["--only", RETURN, "--fix", "a.rb"],
            source.as_bytes(),
            1,
            [1, 0],
        );
    }
}
#[test]
fn clean_unsafe_and_syntax_error_inputs_are_not_rewritten() {
    let dir = project();
    unchanged(
        dir.path(),
        &["--only", BOTH, "--fix", "a.rb"],
        b"def f?\n  true\nend\n",
        0,
        [0, 0],
    );
    unchanged(
        dir.path(),
        &["--only", PREDICATE, "--fix", "a.rb"],
        SOURCE.as_bytes(),
        1,
        [0, 1],
    );
    unchanged(
        dir.path(),
        &["--only", BOTH, "--fix", "a.rb"],
        b"def f?\nreturn nil\n",
        2,
        [0, 0],
    );
}
#[test]
fn stdin_selection_and_fix_rejection_never_write_display_path() {
    use std::{io::Write, process::Stdio};
    let dir = project();
    fs::write(dir.path().join("a.rb"), "untouched\n").unwrap();
    for cop in [RETURN, PREDICATE] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(SOURCE.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--fix", "--stdin", "a.rb"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&out.stderr).contains("--fix"));
        assert_eq!(
            fs::read_to_string(dir.path().join("a.rb")).unwrap(),
            "untouched\n"
        );
    }
}
fn rubies() -> Vec<String> {
    let mut result = Vec::new();
    if Command::new("ruby").arg("--version").output().is_ok() {
        result.push("ruby".into());
    }
    if let Ok(native) = crate::support::runtime::ruby() {
        let out = Command::new(&native).arg("--version").output().unwrap();
        assert!(out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .starts_with(crate::support::runtime::banner_prefix())
        );
        result.push(native);
    }
    result
}
fn ruby(runtime: &str, dir: &Path) -> Output {
    Command::new(runtime)
        .current_dir(dir)
        .args(["-r", "./observer.rb", "a.rb"])
        .output()
        .unwrap()
}
#[test]
fn safe_corrections_preserve_method_proc_lambda_rescue_ensure_and_exception_observations() {
    let dir = project();
    fs::write(
        dir.path().join("observer.rb"),
        "def observed_line(e); e.backtrace.first.split(':')[1]; end\n",
    )
    .unwrap();
    let source = r#"$events = []
class ParentError < StandardError; end
class ChildError < ParentError; end
def method_case(flag)
  begin
    raise ChildError, 'original'
  rescue ParentError => error
    $events << [error.class.name, error.is_a?(StandardError), error.message, observed_line(error)]
    return nil if flag
    :value
  ensure
    $events << :method_ensure
  end
end
def proc_case
  proc { return nil }.call
  :unreachable
ensure
  $events << :proc_ensure
end
def ensure_case
  :discarded
ensure
  $events << :override
  return nil
end
lambda_case = -> { begin; return nil; ensure; $events << :lambda_ensure; end }
def control_case(flag)
  ($events << :guard; flag) || (return nil)
  flag ? (return nil) : :unreachable
ensure
  $events << :control_ensure
end
def escaped_proc
  proc { return nil }
end
begin
  escaped_proc.call
rescue LocalJumpError => error
  $events << [error.class.name, observed_line(error)]
ensure
  $events << :escaped_ensure
end
values = [method_case(true), method_case(false), proc_case, ensure_case, lambda_case.call, control_case(true), control_case(false)]
p [values, $events, __LINE__]
"#;
    for runtime in rubies() {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(&runtime, dir.path());
        assert!(before.status.success(), "{runtime}: {before:?}");
        assert!(
            String::from_utf8_lossy(&before.stdout)
                .contains("[nil, :value, nil, nil, nil, nil, nil]")
        );
        run(dir.path(), &["--only", RETURN, "--fix", "a.rb"], 0, [0, 0]);
        assert_ne!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        let after = ruby(&runtime, dir.path());
        assert_eq!(
            (after.status.code(), after.stdout, after.stderr),
            (before.status.code(), before.stdout, before.stderr),
            "{runtime}"
        );
    }
}
#[test]
fn nil_to_false_is_observably_unsafe_and_is_never_applied() {
    let dir = project();
    fs::write(dir.path().join("observer.rb"), "").unwrap();
    let source = "def f?(flag)\n  return nil if flag\n  nil\nend\np [f?(true).nil?, f?(false).equal?(false)]\n";
    for runtime in rubies() {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(&runtime, dir.path());
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, b"[true, false]\n");
        unchanged(
            dir.path(),
            &["--only", PREDICATE, "--fix", "a.rb"],
            source.as_bytes(),
            1,
            [0, 2],
        );
        let naive = source
            .replace("return nil", "return false")
            .replace("  nil\n", "  false\n");
        fs::write(dir.path().join("a.rb"), naive).unwrap();
        let changed = ruby(&runtime, dir.path());
        assert!(changed.status.success());
        assert_ne!(changed.stdout, before.stdout);
    }
}
#[test]
fn existing_return_string_parentheses_semicolon_and_layout_fixes_compose() {
    let dir = project();
    fs::write(dir.path().join("observer.rb"), "").unwrap();
    let source = "def f(flag)\n  return nil if flag\n  return 'ok'\nend\np [f(true), f(false)]\n";
    let selected = "Style/ReturnNil,Style/RedundantReturn,Style/StringLiterals,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    for runtime in rubies() {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(&runtime, dir.path());
        assert!(before.status.success());
        let out = run(
            dir.path(),
            &["--only", selected, "--fix", "a.rb"],
            0,
            [0, 0],
        );
        assert!(!String::from_utf8_lossy(&out.stdout).contains(PREDICATE));
        let after = fs::read(dir.path().join("a.rb")).unwrap();
        assert_ne!(after, source.as_bytes());
        let observed = ruby(&runtime, dir.path());
        assert_eq!(observed.stdout, before.stdout);
        assert!(observed.status.success());
        unchanged(
            dir.path(),
            &["--only", selected, "--fix", "a.rb"],
            &after,
            0,
            [0, 0],
        );
    }
}
