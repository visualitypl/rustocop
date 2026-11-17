//! real CLI controls, source envelopes and controlled runtime witnesses.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const MODIFIER: &str = "Style/IfUnlessModifierOfIfUnless";
const BOOLEAN: &str = "Style/IfWithBooleanLiteralBranches";
const BAD: &str = "a ? b : c if ready\n";
const GOOD: &str = "if ready\na ? b : c\nend\n";
const BOOL_BAD: &str = "if ready?\n true\nelse\n false\nend\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(
    root: &Path,
    cop: &str,
    source: &[u8],
    args: &[&str],
    count: usize,
    status: Option<i32>,
) {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(root, args);
    if let Some(status) = status {
        assert_eq!(o.status.code(), Some(status), "{o:?}");
    } else {
        assert_ne!(o.status.code(), Some(2), "{o:?}");
    }
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(cop))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
}
#[test]
fn both_defaults_selectors_and_config_work_without_implicit_writes() {
    let d = project();
    for (cop, source) in [(MODIFIER, BAD), (BOOLEAN, BOOL_BAD)] {
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("a.rb");
            unchanged(d.path(), cop, source.as_bytes(), &args, count, None);
        }
        let registry = rustocop::cops::Registry::new();
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled());
        assert_eq!(rule.safe_fixes(), cop == MODIFIER);
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        assert_eq!(rule.ruby_versions(), Default::default());
    }
}
#[test]
fn excludes_and_both_directive_prefixes_prevent_edits() {
    let d = project();
    for (cop, source) in [(MODIFIER, BAD), (BOOLEAN, BOOL_BAD)] {
        for config in [
            "exclude = [\"a.rb\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
                Some(0),
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                unchanged(
                    d.path(),
                    cop,
                    format!("# {prefix}:disable {target}\n{source}").as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                    Some(0),
                );
            }
            unchanged(
                d.path(),
                cop,
                format!("# {prefix}:disable-next {cop}\n{source}").as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
                Some(0),
            );
        }
    }
    let source = format!("# rubocop:disable {MODIFIER}\n{BAD}# rubocop:enable {MODIFIER}\n{BAD}");
    fs::write(d.path().join("a.rb"), &source).unwrap();
    let o = cli(d.path(), &["--only", MODIFIER, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        format!("# rubocop:disable {MODIFIER}\n{BAD}# rubocop:enable {MODIFIER}\n{GOOD}")
    );
    let source = "if a\n b # rubocop:disable Style/IfUnlessModifierOfIfUnless\nend if ready\n";
    unchanged(
        d.path(),
        MODIFIER,
        source.as_bytes(),
        &["--only", MODIFIER, "--fix", "a.rb"],
        1,
        Some(1),
    );
}
#[test]
fn canonical_fix_is_single_pass_and_incompatible_peer_withholds_only_edits() {
    let d = project();
    fs::write(d.path().join("a.rb"), BAD).unwrap();
    let o = cli(d.path(), &["--only", MODIFIER, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), GOOD);
    unchanged(
        d.path(),
        MODIFIER,
        GOOD.as_bytes(),
        &["--only", MODIFIER, "--fix", "a.rb"],
        0,
        Some(0),
    );
    let selected = format!("{MODIFIER},Style/SoleNestedConditional");
    unchanged(
        d.path(),
        MODIFIER,
        BAD.as_bytes(),
        &["--only", &selected, "--fix", "a.rb"],
        1,
        Some(1),
    );
    unchanged(
        d.path(),
        MODIFIER,
        BAD.as_bytes(),
        &["--fix", "a.rb"],
        1,
        None,
    );
}
#[test]
fn unsafe_pinned_corrections_are_all_byte_and_mtime_holds() {
    // RuboCop spec examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    let d = project();
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/if_with_boolean_literal_branches_pinned.json"
    ))
    .unwrap();
    for c in cases {
        unchanged(
            d.path(),
            BOOLEAN,
            c["source"].as_str().unwrap().as_bytes(),
            &["--only", BOOLEAN, "--fix", "a.rb"],
            c["positions"].as_array().unwrap().len(),
            None,
        );
    }
}
#[test]
fn stdin_reports_exact_positions_and_never_creates_a_file() {
    let d = project();
    for (cop, source, position) in [
        (MODIFIER, BAD, "a.rb:1:11:"),
        (BOOLEAN, BOOL_BAD, "a.rb:1:1:"),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
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
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(1), "{o:?}");
        let text = String::from_utf8_lossy(&o.stdout);
        assert!(text.contains(cop) && text.contains(position), "{text}");
        assert!(!d.path().join("a.rb").exists());
    }
}
#[test]
fn controlled_values_bindings_effect_order_and_exception_flow_survive() {
    // Execute only this controlled fixture, never arbitrary corpus input.
    let source = r#"$trace = []
def observe(x)
  $trace << x
  x
end
def choose(a,b,c)
  observe(a) ? :yes : :no if observe(b) unless observe(c)
end
def flow(a,b)
  begin
    if observe(a)
      return :early
    elsif observe(b)
      raise "failure"
    else
      :late
    end if observe(true)
  rescue RuntimeError
    :rescued
  ensure
    $trace << :ensure
  end
end
def value; true; end
def lexical
  if true
    value = 1
  end if value
end
def ready?; :not_boolean; end
p [false, nil, true].repeated_permutation(3).map { |a,b,c| choose(a,b,c) }
p [flow(true,true), flow(false,true), flow(false,false), lexical]
p 2.times.map { |i|
  if i == 0
    next :skipped
  else
    break :stopped
  end if true
}
p(if ready?; true; else; false; end)
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let selected = format!("{MODIFIER},{BOOLEAN}");
    let o = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "binding and unsafe examples remain: {o:?}"
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    assert!(
        after.contains("unless observe(c)\nif observe(b)\n"),
        "{after}"
    );
    assert!(after.contains("end if value") && after.contains("if ready?; true; else; false; end"));
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby)
            .arg(d.path().join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby)
            .arg(d.path().join("a.rb"))
            .output()
            .unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
        let observed = String::from_utf8_lossy(&after.stdout);
        assert!(
            observed.contains("[:early, :rescued, :late, nil]")
                && observed.contains(":stopped\ntrue\n"),
            "{observed}"
        );
    }
}
#[test]
fn mixed_safe_style_and_layout_edits_stabilize_within_eight_explicit_passes() {
    let d = project();
    let source = "def f\n if a\n  return ({ :key => 'value' });\n else\n  raise ('failure');\n end if ready\nend\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    let selected = format!(
        "{MODIFIER},{BOOLEAN},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/IndentationWidth,Layout/EndAlignment"
    );
    let mut stable = false;
    for _ in 0..8 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        let o = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        if old == fs::read(d.path().join("a.rb")).unwrap() {
            stable = true;
            break;
        }
    }
    assert!(stable);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(
        after.contains("if ready\n") && after.contains("key: \"value\""),
        "{after}"
    );
    assert!(!after.contains("end if ready"));
}
