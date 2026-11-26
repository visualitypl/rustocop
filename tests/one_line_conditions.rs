//! CLI policy, safe fixes and controlled Ruby semantic observations.
use crate::support::cli::project;
use std::{fs, path::Path, process::Command};
const ONE: &str = "Style/OneLineConditional";
const PARENS: &str = "Style/ParenthesesAroundCondition";
const BOTH: &str = "Style/OneLineConditional,Style/ParenthesesAroundCondition";
const CASES: [(&str, &str, &str); 2] = [
    (ONE, "if a then b else c end\n", "a ? b : c\n"),
    (PARENS, "if (a); b; end\n", "if a; b; end\n"),
];
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, cop: &str, source: &[u8], args: &[&str], count: usize) {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(root, args);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(cop))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn default_only_except_and_config_no_write_policy() {
    let d = project();
    for (cop, source, _) in CASES {
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
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
            unchanged(d.path(), cop, source.as_bytes(), &args, count);
        }
        let registry = rustocop::cops::Registry::new();
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    }
}
#[test]
fn excludes_and_all_directive_scopes_do_not_write() {
    let d = project();
    for (cop, source, _) in CASES {
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
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(
                        d.path(),
                        cop,
                        source.as_bytes(),
                        &["--only", cop, "--fix", "a.rb"],
                        0,
                    );
                }
            }
        }
    }
}
#[test]
fn all_pinned_correction_calls_run_through_cli() {
    // Ported RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let d = project();
    let mut fixes = 0;
    for (cop, fixture) in [
        (
            ONE,
            include_str!("fixtures/one_line_conditional_pinned.json"),
        ),
        (
            PARENS,
            include_str!("fixtures/parentheses_around_condition_pinned.json"),
        ),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for c in cases {
            let Some(after) = c["correction"].as_str() else {
                continue;
            };
            fixes += 1;
            let source = c["source"].as_str().unwrap();
            let (source, after) = if source.contains("yield") {
                (
                    format!("def fixture\n{source}end\n"),
                    format!("def fixture\n{after}end\n"),
                )
            } else {
                (source.to_owned(), after.to_owned())
            };
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let o = cli(
                d.path(),
                &["--only", cop, "--fix", "--fix-passes", "1", "a.rb"],
            );
            assert_ne!(o.status.code(), Some(2), "{c}: {o:?}");
            assert_eq!(
                fs::read_to_string(d.path().join("a.rb")).unwrap(),
                after,
                "{c}: {o:?}"
            );
            // Nested candidates deliberately take one explicit pass at a time.
            if !["if cond2; bar else baz end"]
                .iter()
                .any(|s| after.contains(s))
            {
                unchanged(
                    d.path(),
                    cop,
                    after.as_bytes(),
                    &["--only", cop, "--fix", "--fix-passes", "1", "a.rb"],
                    0,
                );
            }
        }
    }
    assert_eq!(fixes, 63);
}
fn observations_equal(root: &Path) {
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby)
            .arg(root.join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby).arg(root.join("a.rb")).output().unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
    }
}
#[test]
fn runtime_index_assignment_condition_preserves_stored_value() {
    let d = project();
    for value in ["false", "nil", "true"] {
        let source =
            format!("a = []; result = if a[0] = {value} then :yes else :no end; p [result, a]\n");
        let expected = format!("a = []; result = (a[0] = {value}) ? :yes : :no; p [result, a]\n");
        fs::write(d.path().join("before.rb"), &source).unwrap();
        fs::write(d.path().join("a.rb"), &source).unwrap();
        let o = cli(d.path(), &["--only", ONE, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(0), "{o:?}");
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
        observations_equal(d.path());
    }
}
#[test]
fn runtime_values_scope_order_count_nested_guards_and_rescue_ensure() {
    let source = r#"$trace=[]
def see(x); $trace << x; x; end
def x; :method; end
def branches(a,b)
  first = if see(a) then see(x) else x=see(:bound) end
  second = unless see(b) then see(:no) else see(:yes) end
  third = if see(a) then see(:first) elsif see(b) then see(:second) else see(:third) end
  fourth = (see(:body) if (see(a) && see(b)))
  [first,second,third,fourth,x]
end
def scopes(a)
  one=unless a then x=see(1) else see(x) end
  two=if a then see(x) else x=see(2) end
  [one,two,x]
end
def transfers(a,b)
  begin
    if a then return see(:early) else see(:later) end
    if b then raise 'failure' else see(:finish) end
  rescue RuntimeError
    see(:rescued)
  ensure
    see(:ensured)
  end
end
def protected_branch(a)
  if a then begin see(:work); raise 'failure'; rescue; see(:rescue); ensure; see(:ensure); end else see(:other) end
end
def iteration(a)
  result=[]
  [1,2,3].each do |i|
    if i==1 then next else result << see(i) end
    if a then break result else see(:continue) end
  end
  j=0
  j+=1 while (j<2)
  result << j
end
p [nil,false,true].product([nil,false,true]).map { |a,b| [branches(a,b),scopes(a),transfers(a,b),protected_branch(a),iteration(a)] }
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", BOTH, "--fix", "a.rb"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "binding finding must remain: {o:?}"
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    assert!(after.contains("unless a then x=see(1) else see(x) end"));
    observations_equal(d.path());
}
#[test]
fn runtime_block_binding_operator_dispatch_and_parent_precedence() {
    let source = r#"$trace=[]
def see(x); $trace << x; x; end
def with_block; see(:call); yield; end
class Box
  def +(other); see([:plus,other]); :answer; end
end
[true,false].each do |c|
  p(if c then with_block() do see(:block) end else see(:none) end)
  p(Box.new + if c then see(1) else see(2) end)
  p(if c then false else true end ? see(:yes) : see(:no))
  p((if c then 1 else 2 end..3).to_a)
  if (with_block do see(c) end); see(:body); end
  if (see(:first)
      see(c)); see(:last); end
end
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", BOTH, "--fix", "a.rb"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "multi-expression condition must remain: {o:?}"
    );
    assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
    observations_equal(d.path());
}
#[test]
fn selected_default_and_style_layout_composition_stabilizes_within_eight_passes() {
    let d = project();
    let source = "def f(a)\n return (if (a) then { :key => 'value' } else nil end);\nend\n";
    for selection in [
        Some(BOTH),
        Some(
            "Style/OneLineConditional,Style/ParenthesesAroundCondition,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..8 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec![];
            if let Some(s) = selection {
                args.extend(["--only", s]);
            }
            args.extend(["--fix", "a.rb"]);
            let o = cli(d.path(), &args);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if before == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selection:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(!after.contains("then"), "{after}");
        if selection != Some(BOTH) {
            assert!(!after.contains("(a) ?"), "{after}");
        }
    }
}
#[test]
fn strict_config_and_stdin_never_write() {
    use std::{io::Write, process::Stdio};
    let d = project();
    for (cop, source, _) in CASES {
        for key in ["max = 1", "EnforcedStyle = \"other\""] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            fs::write(d.path().join("a.rb"), source).unwrap();
            assert_eq!(
                cli(d.path(), &["--only", cop, "--fix", "a.rb"])
                    .status
                    .code(),
                Some(2)
            );
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        fs::write(d.path().join("a.rb"), "sentinel\n").unwrap();
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
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "sentinel\n"
        );
    }
}
