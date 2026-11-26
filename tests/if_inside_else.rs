//! real CLI, safety/composition and controlled Ruby observations.
use crate::support::cli::project;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const COP: &str = "Style/IfInsideElse";
const BAD: &str = "if a\n  first\nelse\n  if b\n    second\n  end\nend\n";
const GOOD: &str = "if a\n  first\nelsif b\n  second\nend\n";
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: Option<i32>, count: usize) {
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
            .filter(|l| l.contains(COP))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), time);
}
fn fix(root: &Path, before: &str, after: &str, selected: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let o = cli(root, &["--only", selected, "--fix", "a.rb"]);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        fs::read_to_string(root.join("a.rb")).unwrap(),
        after,
        "{o:?}"
    );
}
#[test]
fn default_enablement_selectors_and_config_have_no_implicit_writes() {
    let d = project();
    for (config, selectors, count) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Style", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
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
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("a.rb");
        unchanged(d.path(), BAD.as_bytes(), &args, None, count);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    fix(d.path(), BAD, GOOD, COP);
    unchanged(
        d.path(),
        GOOD.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        Some(0),
        0,
    );
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert_eq!(cop.ruby_versions(), Default::default());
}
#[test]
fn exclusions_and_both_directive_prefixes() {
    let d = project();
    for config in [
        "exclude = [\"a.rb\"]\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            BAD.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            Some(0),
            0,
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for (source, count) in [
            (format!("# {prefix}:disable {COP}\n{BAD}"), 0),
            (format!("# {prefix}:disable Style\n{BAD}"), 0),
            (format!("# {prefix}:disable all\n{BAD}"), 0),
            (
                BAD.replace("if b", &format!("if b # {prefix}:disable {COP}")),
                0,
            ),
            (
                BAD.replace("if b", &format!("# {prefix}:disable-next {COP}\n  if b")),
                0,
            ),
        ] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                Some(i32::from(count > 0)),
                count,
            );
        }
        // A directive inside the corrected span does not hold the edit.
        fix(
            d.path(),
            &BAD.replace("second", &format!("second # {prefix}:disable {COP}")),
            &GOOD.replace("second", &format!("second # {prefix}:disable {COP}")),
            COP,
        );
        let source = format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n{BAD}");
        let after = format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n{GOOD}");
        fix(d.path(), &source, &after, COP);
    }
}
#[test]
fn stdin_reports_exact_keyword_positions_without_writing() {
    let d = project();
    for (config, source, count) in [
        (String::new(), BAD.to_owned(), 1),
        (
            format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
            BAD.to_owned(),
            0,
        ),
        (String::new(), format!("# rubocop:disable {COP}\n{BAD}"), 0),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
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
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(count), "{o:?}");
        let text = String::from_utf8_lossy(&o.stdout);
        assert_eq!(text.contains(COP), count == 1, "{text}");
        if count == 1 {
            assert!(
                text.contains("a.rb:4:3:")
                    && text.contains("Convert `if` nested inside `else` to `elsif`."),
                "{text}"
            );
        }
        assert!(!d.path().join("a.rb").exists());
    }
}
#[test]
fn controlled_branch_values_side_effects_bindings_and_exception_flow_survive_fixes() {
    // Controlled fixtures only; never execute arbitrary input/corpus source.
    let source = r#"$trace = []
def guard(x)
  $trace << x
  x
end
def choose(a, b, c)
  if guard(a)
    :a
  else
    if guard(b)
      value = "b\nvalue"
      value
    elsif guard(c)
      :c
    else
      :none
    end
  end
end
def then_form(a, b)
  if a
    :a
  else
    if b then "b\nvalue" else :none end
  end
end
def modifier(a, b)
  if a
    :a
  else
    :b if guard(b)
  end
end
def flow(a, b)
  begin
    if a
      return :early
    else
      if b
        raise "failure"
      else
        return :late
      end
    end
  rescue RuntimeError
    :rescued
  ensure
    $trace << :ensure
  end
end
def binding_case(a)
  if a
    nil
  else
    value = 1 if value
  end
end
results = [false, nil, true].repeated_permutation(3).map { |a,b,c| choose(a,b,c) }
p [:none, :c, "b\nvalue", :a].map { |value| results.count(value) }
p [modifier(false, false), modifier(false, true), flow(true, true), flow(false, true), flow(false, false), binding_case(false)]
p [then_form(true, false), then_form(false, true), then_form(false, false)]
p 2.times.map { |i|
  if false
    :never
  else
    if i == 0
      next :skipped
    else
      break :stopped
    end
  end
}
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", COP, "--fix", "a.rb"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "binding case stays reported: {o:?}"
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    assert!(after.contains("elsif guard(b)") && after.contains("value = 1 if value"));
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
            observed.contains("[8, 4, 6, 9]")
                && observed.contains("[nil, :b, :early, :rescued, :late, nil]")
                && observed.contains(":stopped"),
            "{observed}"
        );
    }
}
#[test]
fn mixed_style_layout_fixes_stabilize_and_preserve_atomic_else_association() {
    let d = project();
    let before = "if a\n  return ('a');\nelse\n  if b\n    { :key => 'b' }\n  else\n    { :key => 'c' }\n  end\nend\n";
    let selected = "Style/IfInsideElse,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/EndAlignment,Layout/IndentationWidth";
    fs::write(d.path().join("a.rb"), before).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        let o = cli(d.path(), &["--only", selected, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        let new = fs::read(d.path().join("a.rb")).unwrap();
        assert_eq!(ruby_prism::parse(&new).errors().count(), 0);
        if old == new {
            stable = true;
            break;
        }
    }
    assert!(
        stable,
        "curated mixed fixture stabilizes within six explicit passes"
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(
        after.contains("elsif b") && after.contains("key: \"b\"") && after.contains("key: \"c\""),
        "{after}"
    );
    // The new cop's own canonical cases are stable after ONE explicit pass.
    fix(d.path(), BAD, GOOD, COP);
}
