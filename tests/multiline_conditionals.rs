//! CLI controls and controlled Ruby semantic witnesses.
use crate::support::cli::project;
use std::{fs, path::Path, process::Command};
const IF: &str = "Style/MultilineIfThen";
const TERNARY: &str = "Style/MultilineTernaryOperator";
const WHEN: &str = "Style/MultilineWhenThen";
const ALL: &str = "Style/MultilineIfThen,Style/MultilineTernaryOperator,Style/MultilineWhenThen";
const CASES: [(&str, &str, &str); 3] = [
    (IF, "if cond then\n work\nend\n", "if cond\n work\nend\n"),
    (
        TERNARY,
        "a = cond ?\n yes : no\n",
        "a = if cond\n  yes\nelse\n  no\nend\n",
    ),
    (
        WHEN,
        "case x\nwhen one then\n work\nend\n",
        "case x\nwhen one\n work\nend\n",
    ),
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
fn default_on_selectors_config_and_plain_lint_do_not_write() {
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
fn exclusions_and_both_directive_prefixes_hold_bytes() {
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
                unchanged(
                    d.path(),
                    cop,
                    format!("# {prefix}:disable {target}\n{source}").as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            let next = if cop == WHEN {
                source.replacen('\n', &format!("\n# {prefix}:disable-next {cop}\n"), 1)
            } else {
                format!("# {prefix}:disable-next {cop}\n{source}")
            };
            unchanged(
                d.path(),
                cop,
                next.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            let inline = if cop == TERNARY {
                format!("{} # {prefix}:disable {cop}\n", source.trim_end())
            } else {
                source.replacen(" then\n", &format!(" then # {prefix}:disable {cop}\n"), 1)
            };
            unchanged(
                d.path(),
                cop,
                inline.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
    // As in RuboCop, an interior comment moves above the rewrite.
    fs::write(
        d.path().join("a.rb"),
        "a = cond ?\n # rubocop:disable Lint/Debugger\n yes : no\n",
    )
    .unwrap();
    let o = cli(d.path(), &["--only", TERNARY, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "# rubocop:disable Lint/Debugger\na = if cond\n  yes\nelse\n  no\nend\n"
    );
}
#[test]
fn all_pinned_correction_calls_are_canonical_and_single_pass_stable() {
    // RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let d = project();
    let mut count = 0;
    for (cop, fixture) in [
        (IF, include_str!("fixtures/multiline_if_then_pinned.json")),
        (
            TERNARY,
            include_str!("fixtures/multiline_ternary_operator_pinned.json"),
        ),
        (
            WHEN,
            include_str!("fixtures/multiline_when_then_pinned.json"),
        ),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for c in cases {
            let Some(after) = c["correction"].as_str() else {
                continue;
            };
            count += 1;
            fs::write(d.path().join("a.rb"), c["source"].as_str().unwrap()).unwrap();
            let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(o.status.code(), Some(0), "{cop}: {c}: {o:?}");
            assert_eq!(
                fs::read_to_string(d.path().join("a.rb")).unwrap(),
                after,
                "{c}"
            );
            unchanged(
                d.path(),
                cop,
                after.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
    assert_eq!(count, 25);
}
fn rubies() -> Vec<String> {
    let mut v = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        v.push(ruby);
    }
    v
}
fn observations_equal(root: &Path) {
    for ruby in rubies() {
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
fn runtime_explicit_continuation_keeps_body_out_of_predicate() {
    let d = project();
    for keyword in ["if true", "unless false", "if false\n  :unused\nelsif true"] {
        for nl in ["\n", "\r\n"] {
            let source = format!("p({keyword} \\\nthen\n -1\nend)\n").replace('\n', nl);
            fs::write(d.path().join("before.rb"), &source).unwrap();
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let o = cli(d.path(), &["--only", IF, "--fix", "a.rb"]);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            observations_equal(d.path());
            assert_eq!(o.status.code(), Some(1), "{o:?}");
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        }
    }
}
#[test]
fn when_escaped_newlines_retain_exact_diagnostics_bytes_and_mtime() {
    let d = project();
    let path = d.path().join("a.rb");
    for nl in ["\n", "\r\n"] {
        for conditions in ["true", "false, true"] {
            let source =
                format!("case true\nwhen {conditions} \\\n then -1\nend\n").replace('\n', nl);
            fs::write(&path, &source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            // Plain lint and repeated fix passes must all preserve the finding.
            for args in [
                vec!["--only", WHEN, "a.rb"],
                vec!["--only", WHEN, "--fix", "a.rb"],
                vec!["--only", WHEN, "--fix", "a.rb"],
            ] {
                let o = cli(d.path(), &args);
                assert_eq!(o.status.code(), Some(1), "{o:?}");
                assert!(String::from_utf8_lossy(&o.stdout).contains(
                    "a.rb:3:2: C: Do not use `then` for multiline `when` statement. (Style/MultilineWhenThen)"
                ), "{o:?}");
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
    }
}
#[test]
fn runtime_when_escaped_newlines_keep_unary_bodies_and_condition_effects() {
    let d = project();
    for (source, expected) in [
        ("p(case true\nwhen true \\\n then -1\nend)\n", "-1\n"),
        ("p(case\nwhen true \\\n then +1\nend)\n", "1\n"),
        (
            "trace = []\nresult = case true\nwhen (trace << :condition; true) \\\n then -(trace << :body; 1)\nend\np [result, trace]\n",
            "[-1, [:condition, :body]]\n",
        ),
    ] {
        for nl in ["\n", "\r\n"] {
            let source = source.replace('\n', nl);
            let naive = source.replace(" then ", " ");
            // Parse validation alone cannot reject this binding-changing edit.
            assert_eq!(
                ruby_prism::parse_ruby_3_4(naive.as_bytes())
                    .errors()
                    .count(),
                0
            );
            fs::write(d.path().join("before.rb"), &source).unwrap();
            fs::write(d.path().join("naive.rb"), &naive).unwrap();
            unchanged(
                d.path(),
                WHEN,
                source.as_bytes(),
                &["--only", WHEN, "--fix", "a.rb"],
                1,
            );
            for ruby in rubies() {
                if Command::new(&ruby).arg("--version").output().is_err() {
                    continue;
                }
                let run = |name| {
                    Command::new(&ruby)
                        .arg(d.path().join(name))
                        .output()
                        .unwrap()
                };
                let before = run("before.rb");
                let after = run("a.rb");
                let changed = run("naive.rb");
                assert!(
                    before.status.success() && after.status.success(),
                    "{ruby}: {before:?} {after:?}"
                );
                assert_eq!(before.stdout, expected.as_bytes(), "{ruby}");
                assert_eq!(before.stdout, after.stdout, "{ruby}");
                assert_eq!(before.stderr, after.stderr, "{ruby}");
                assert!(!changed.status.success(), "{ruby}: {changed:?}");
                assert!(
                    String::from_utf8_lossy(&changed.stderr).contains("NoMethodError"),
                    "{ruby}: {changed:?}"
                );
            }
        }
    }
}
#[test]
fn when_escaped_boundary_does_not_block_a_safe_sibling_correction() {
    let d = project();
    for nl in ["\n", "\r\n"] {
        let source =
            "p(case true\nwhen true \\\n then -1\nend)\np(case true\nwhen true\n then +1\nend)\n"
                .replace('\n', nl);
        let fixed = source.replace(" then +1", " +1");
        fs::write(d.path().join("before.rb"), &source).unwrap();
        fs::write(d.path().join("a.rb"), &source).unwrap();
        let o = cli(d.path(), &["--only", WHEN, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(1), "{o:?}");
        assert_eq!(
            fs::read(d.path().join("a.rb")).unwrap(),
            crate::support::lf::lf(&fixed)
        );
        observations_equal(d.path());
        unchanged(
            d.path(),
            WHEN,
            fixed.as_bytes(),
            &["--only", WHEN, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn runtime_open_range_ternaries_keep_branch_values() {
    let d = project();
    for expression in [
        "true.. ?\n false : 2",
        "true... ?\n false : 2",
        "true ?\n true.. ?\n false : 2 : 3",
        "false ?\n 3 : true.. ?\n false : 2",
    ] {
        for source in [
            format!("x = {expression}\np x\n"),
            format!("p({expression})\n"),
        ] {
            fs::write(d.path().join("before.rb"), &source).unwrap();
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let o = cli(d.path(), &["--only", TERNARY, "--fix", "a.rb"]);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            observations_equal(d.path());
            // A direct call argument can safely join the ternary separators;
            // nested branch expansion must still withhold the entire edit.
            if source.starts_with("x =")
                || expression.contains("true ?")
                || expression.contains("false ?")
            {
                assert_eq!(o.status.code(), Some(1), "{o:?}");
                assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
            } else {
                assert_eq!(o.status.code(), Some(0), "{o:?}");
            }
        }
    }
}
#[test]
fn runtime_binding_values_branch_effects_nested_guards_and_rescue_ensure() {
    let source = r#"$trace = []
def observe(x); $trace << x; x; end
def x; :method; end
def branches(a,b)
  if observe(a) then # keep association
    observe(:first)
  elsif observe(b) then
    observe(:second)
  else
    observe(:last)
  end
end
def unless_branch(a)
  unless observe(a) then
    observe(:unless)
  end
end
def ternary(a)
  result = observe(a) ? # ordinary separator comment
    observe(x) : (x = observe(:bound))
  [result,x]
end
def lexical(a)
  result = a ?
    (x = 1) : x
  [result,x]
end
def modifier_scope(a)
  result = a ?
    (x = :body if (x = a)) : :else
  [result,x]
end
def nested(a,b)
  a ? observe(:outer) :
    b ? observe(:inner) :
      observe(:neither)
end
def choose(a)
  case observe(a)
  when true then
    observe(:truth)
  when false
    then observe(:falsehood)
  else
    observe(:nil)
  end
end
def protected_flow(a)
  begin
    return observe(a) ?
      :early : raise('failure')
  rescue RuntimeError
    observe(:rescued)
  ensure
    observe(:ensure)
  end
end
def inner_protection(a)
  a ?
    (begin
       observe(:inside_true)
       raise 'failure'
     rescue RuntimeError
       observe(:inside_rescue)
     ensure
       observe(:inside_ensure)
     end) : (observe(:inside_false) rescue :fallback)
end
def guarded(a,b)
  if a then
    return b ?
      :guarded : :other
  end if b
end
def open_range
  $_ = '1'
  if 1.. then
    observe(:open_range)
  end
end
p [nil,false,true].product([nil,false,true]).map { |a,b| [branches(a,b),nested(a,b),guarded(a,b)] }
p [nil,false,true].map { |a| [unless_branch(a),ternary(a),lexical(a),modifier_scope(a),choose(a),protected_flow(a),inner_protection(a)] }
p open_range
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", ALL, "--fix", "a.rb"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "open-range finding must remain: {o:?}"
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    assert!(after.contains("if 1.. then"));
    assert!(after.contains("# ordinary separator comment\n"));
    observations_equal(d.path());
}
#[test]
fn selected_and_default_style_layout_interactions_stabilize_within_eight_passes() {
    let d = project();
    let source = "def f(ready)\n if ready then\n  return ready ?\n   { :key => 'value' } : nil;\n else\n  case ready\n  when false then\n   raise Exception, 'boom'\n  end\n end\nend\n";
    for selected in [
        Some(ALL),
        Some(
            "Style/MultilineIfThen,Style/MultilineTernaryOperator,Style/MultilineWhenThen,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..8 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec![];
            if let Some(selected) = selected {
                args.extend(["--only", selected]);
            }
            args.extend(["--fix", "a.rb"]);
            let o = cli(d.path(), &args);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if old == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(!after.contains(" then"), "{after}");
    }
}
#[test]
fn strict_config_and_stdin_never_write() {
    use std::{io::Write, process::Stdio};
    let d = project();
    for (cop, source, _) in CASES {
        for (config, key) in [
            (format!("[cops.\"{cop}\"]\nmax = 1\n"), "max"),
            (
                format!("[cops.\"{cop}\"]\nEnforcedStyle = \"other\"\n"),
                "EnforcedStyle",
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(d.path().join("a.rb"), source).unwrap();
            let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(o.status.code(), Some(2), "{o:?}");
            assert!(String::from_utf8_lossy(&o.stderr).contains(key), "{o:?}");
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
