//! fixed TernaryParentheses policy through the real CLI.
//! Pinned fixtures port RuboCop (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{fs, path::Path, process::Command};
const COP: &str = "Style/TernaryParentheses";
const BAD: &str = "answer = (ready) ? yes : no\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/a.rb"), BAD).unwrap();
    dir
}
fn run(root: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(2), "{out:?}");
    out
}
fn fix(root: &Path, before: &str, after: &str) {
    let path = root.join("lib/a.rb");
    fs::write(&path, before).unwrap();
    let old_time = fs::metadata(&path).unwrap().modified().unwrap();
    run(root, &["--only", COP, "--fix", "lib/a.rb"]);
    assert_eq!(
        fs::read(&path).unwrap(),
        crate::support::lf::lf(after),
        "{before}"
    );
    if before == after {
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), old_time);
    }
}

#[test]
fn default_on_selectors_and_config_no_write() {
    let dir = project();
    for (config, selectors, count) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--only", "Style", "--except", COP], 0),
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
        let out = run(dir.path(), &args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            stdout.lines().filter(|line| line.contains(COP)).count(),
            count,
            "{out:?}"
        );
        if count > 0 {
            assert!(
                stdout.contains("lib/a.rb:1:10: C: Omit parentheses for ternary conditions."),
                "{stdout}"
            );
        }
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
fn config_relative_excludes_and_directives_prevent_writes() {
    let dir = project();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let out = run(&dir.path().join("lib"), &["--only", COP, "--fix", "a.rb"]);
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{BAD}"),
            format!("# {prefix}:disable-next {COP}\n{BAD}"),
            format!("answer = (ready) ? yes : no # {prefix}:disable {COP}\n"),
            format!("# {prefix}:disable Style\n{BAD}"),
        ] {
            fix(dir.path(), &source, &source);
        }
    }
    for source in [
        "ready ? yes : no\n",
        "(n += 1) ? yes : no\n",
        "(foo # keep\n  ) ? yes : no\n",
        "(x = work) ? x : no\n",
    ] {
        fix(dir.path(), source, source);
    }
    let invalid = "(ready) ? yes :\n";
    fs::write(dir.path().join("lib/a.rb"), invalid).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--fix", "lib/a.rb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
        invalid
    );
}

#[test]
fn every_pinned_call_and_correction_runs_through_cli() {
    let dir = project();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ternary_parentheses_pinned.json")).unwrap();
    let mut canonical = 0;
    let mut invalid = 0;
    for case in cases.as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let is_invalid = ruby_prism::parse(source.as_bytes()).errors().count() > 0;
        invalid += usize::from(is_invalid);
        let wrapped = format!("def f\n{source}end\n");
        let before = if is_invalid { &wrapped } else { source };
        let expected = case["correction"].as_str().unwrap_or(source);
        canonical += usize::from(case["correction"].is_string());
        let wrapped_after = format!("def f\n{expected}end\n");
        fix(
            dir.path(),
            before,
            if is_invalid { &wrapped_after } else { expected },
        );
    }
    assert_eq!((canonical, invalid), (27, 2));
}

fn observe(ruby: &Path, source: &str) -> Vec<u8> {
    let out = Command::new(ruby).arg("-e").arg(source).output().unwrap();
    assert!(out.status.success(), "{out:?}\n{source}");
    out.stdout
}

#[test]
fn runtime_observations_preserve_binding_branch_returns_and_effect_order() {
    let dir = project();
    let before = "$events = []\ndef check(v); $events << [:check, v]; v; end\ndef result(v); $events << [:result, v]; v; end\ndef choose(v)\n  (check v) ? (return result(:yes)) : (return result(:no))\nensure\n  $events << :ensure\nend\np [choose(true), choose(false)]\np $events\nx = nil\np ((x = check(false)) ? :wrong : x)\nbegin\n  (check(false)) ? :wrong : (raise 'boom')\nrescue\n  puts 'rescued'\nensure\n  puts 'ensured'\nend\nif false\n  puts 'wrong'\nelsif true\n  p ((true && true) ? __LINE__ : :wrong)\nelse\n  puts 'wrong'\nend\n";
    fs::write(dir.path().join("lib/a.rb"), before).unwrap();
    run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
    let after = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
    assert!(after.contains("check(v) ? (return result(:yes))"));
    assert!(after.contains("(x = check(false))"));
    assert!(!after.contains("(true && true)"));
    let expected = b"[:yes, :no]\n[[:check, true], [:result, :yes], :ensure, [:check, false], [:result, :no], :ensure]\nfalse\nrescued\nensured\n23\n";
    for ruby in [Path::new("ruby"), Path::new("/opt/ruby34/miniruby")] {
        if Command::new(ruby).arg("-v").output().is_err() {
            continue;
        }
        assert_eq!(observe(ruby, before), expected, "{ruby:?}");
        assert_eq!(observe(ruby, &after), expected, "{ruby:?}");
    }
}

#[test]
fn keyword_adjacent_predicates_preserve_runtime_control_flow() {
    let dir = project();
    let before = "def f; return(true) ? :yes : :no; end; p f\np [1].map { next(true) ? :yes : :no }\np [1].each { break(false) ? :yes : :no }\n";
    let after = "def f; return true ? :yes : :no; end; p f\np [1].map { next true ? :yes : :no }\np [1].each { break false ? :yes : :no }\n";
    // Check execution before spelling so a token merge reproduces the actual
    // behavior change, not just a difference in expected formatting.
    fs::write(dir.path().join("lib/a.rb"), before).unwrap();
    let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let corrected = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
    for ruby in [Path::new("ruby"), Path::new("/opt/ruby34/miniruby")] {
        if Command::new(ruby).arg("-v").output().is_err() {
            continue;
        }
        let expected = b":yes\n[:yes]\n:no\n";
        assert_eq!(observe(ruby, before), expected, "{ruby:?}");
        assert_eq!(observe(ruby, &corrected), expected, "{ruby:?}");
    }
    assert_eq!(corrected, after);
    fix(dir.path(), after, after);
}

#[test]
fn source_bytes_and_mixed_correction_passes_stabilize() {
    let dir = project();
    fix(
        dir.path(),
        "\u{feff}# encoding: utf-8\r\n(\"é\r\ntext\") ? __LINE__ : nil\r\n__END__\r\nraw\x00\r\n",
        "\u{feff}# encoding: utf-8\r\n\"é\r\ntext\" ? __LINE__ : nil\r\n__END__\r\nraw\x00\r\n",
    );
    let before = "def choose(value)\n  return (value) ? {'key' => 'yes'} : {'key' => 'no'};\nend\n";
    for selected in [
        Some(
            "Style/TernaryParentheses,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout",
        ),
        Some("Style,Layout"),
        None,
    ] {
        fs::write(dir.path().join("lib/a.rb"), before).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(dir.path().join("lib/a.rb")).unwrap();
            let mut args = vec!["--fix", "lib/a.rb"];
            if let Some(selected) = selected {
                args.extend(["--only", selected]);
            }
            run(dir.path(), &args);
            let new = fs::read(dir.path().join("lib/a.rb")).unwrap();
            if new == old {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        let after = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
        assert!(!after.contains("(value) ?"), "{after}");
        for ruby in [Path::new("ruby"), Path::new("/opt/ruby34/miniruby")] {
            if Command::new(ruby).arg("-v").output().is_err() {
                continue;
            }
            let invocation = "\np [choose(true), choose(false)]\n";
            assert_eq!(
                observe(ruby, &(before.to_owned() + invocation)),
                observe(ruby, &(after.to_owned() + invocation))
            );
        }
    }
}
