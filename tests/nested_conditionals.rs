//! real CLI controls, canonical corrections and controlled Ruby observations.
use crate::support::source_getter;
use std::{fs, path::Path, process::Command};
const MODIFIER: &str = "Style/NestedModifier";
const TERNARY: &str = "Style/NestedTernaryOperator";
const ALL: &str = "Style/NestedModifier,Style/NestedTernaryOperator";
const CASES: [(&str, &str, &str); 2] = [
    (MODIFIER, "work if a if b\n", "work if b && a\n"),
    (
        TERNARY,
        "a ? (b ? 1 : 2) : 3\n",
        "if a\nb ? 1 : 2\nelse\n3\nend\n",
    ),
];
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
fn defaults_selectors_config_and_plain_lint_do_not_write() {
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
fn excludes_and_both_directive_prefixes_hold_bytes() {
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
            unchanged(
                d.path(),
                cop,
                format!("# {prefix}:disable-next {cop}\n{source}").as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            unchanged(
                d.path(),
                cop,
                format!("{} # {prefix}:disable {cop}\n", source.trim_end()).as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
    unchanged(
        d.path(),
        TERNARY,
        b"a ? (b ? 1 : 2) : (3 # rubocop:disable Lint/Debugger\n)\n",
        &["--only", TERNARY, "--fix", "a.rb"],
        1,
    );
}
#[test]
fn every_pinned_correction_call_runs_and_safe_edits_stabilize() {
    // RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let d = project();
    let mut counts = (0, 0, 0);
    for (cop, fixture) in [
        (
            MODIFIER,
            include_str!("fixtures/nested_modifier_pinned.json"),
        ),
        (
            TERNARY,
            include_str!("fixtures/nested_ternary_operator_pinned.json"),
        ),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let withheld = cop == MODIFIER
                && [47, 58, 69, 80, 102, 113, 124, 135, 157, 239]
                    .contains(&c["spec_line"].as_u64().unwrap());
            let expected = c["correction"]
                .as_str()
                .filter(|_| !withheld)
                .unwrap_or(source);
            if c["correction"].is_string() {
                counts.0 += 1;
                if withheld {
                    counts.1 += 1;
                } else {
                    counts.2 += 1;
                }
            }
            if source == expected {
                unchanged(
                    d.path(),
                    cop,
                    source.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    c["positions"].as_array().unwrap().len(),
                );
            } else {
                fs::write(d.path().join("a.rb"), source).unwrap();
                let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
                assert_eq!(o.status.code(), Some(0), "{c}: {o:?}");
                assert_eq!(
                    fs::read_to_string(d.path().join("a.rb")).unwrap(),
                    expected,
                    "{c}"
                );
                unchanged(
                    d.path(),
                    cop,
                    expected.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
        }
    }
    assert_eq!(counts, (32, 10, 22));
}
#[test]
fn unsafe_clean_invalid_encoded_and_source_sensitive_files_do_not_write() {
    let d = project();
    for (cop, source) in [
        (MODIFIER, b"work if a while b\n".as_slice()),
        (MODIFIER, b"work if a if a = true\n"),
        (MODIFIER, b"work unless a if b\n"),
        (MODIFIER, b"DATA.pos unless a unless b\n__END__\npayload\n"),
        (MODIFIER, b"# coding: ISO-8859-1\nwork if '\xe9' if true\n"),
        (TERNARY, b"a ? (b ? 1 : 2) : 3\np __LINE__\n"),
        (TERNARY, b"a ? (b ? DATA.pos : 2) : 3\n__END__\npayload\n"),
        (TERNARY, b"# coding: ISO-8859-1\na ? (b ? '\xe9' : 2) : 3\n"),
        (TERNARY, b"a ? <<~H : (b ? 1 : 2)\nliteral\nH\n"),
    ] {
        unchanged(d.path(), cop, source, &["--only", cop, "--fix", "a.rb"], 1);
    }
    for (cop, _, _) in CASES {
        unchanged(
            d.path(),
            cop,
            b"work\n",
            &["--only", cop, "--fix", "a.rb"],
            0,
        );
        let path = d.path().join("a.rb");
        fs::write(&path, b"if broken\n").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(2), "{o:?}");
        assert_eq!(fs::read(&path).unwrap(), b"if broken\n");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
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
fn runtime_binding_values_evaluation_order_branch_effects_guards_and_protection() {
    let source = r#"$trace=[]
def observe(x); $trace << x; x; end
def x; :method; end
def modifiers(a,b)
  v = (observe(:body) if observe(a) if observe(b))
  w = (observe(:unless) unless observe(a) unless observe(b))
  [v,w]
end
def scope(a,b)
  v=(observe(x) if x if (x = a))
  [v,x,b]
end
def ternaries(a,b)
  result = observe(a) ? (observe(b) ? observe(x) : (x=observe(:bound))) : observe(:last)
  [result,x]
end
def associations(a,b)
  if a
    return :early if a if b
  elsif b
    :second
  else
    :last
  end
end
def guards(a,b)
  begin
    return observe(a) ? (observe(b) ? :first : :second) : raise('failure')
  rescue RuntimeError
    observe(:rescued)
  ensure
    observe(:ensure)
  end
end
def inside_protection(a,b)
  a ? (begin
    b ? observe(:inside_true) : raise('failure')
  rescue RuntimeError
    observe(:inside_rescue)
  ensure
    observe(:inside_ensure)
  end) : (observe(:inside_false) rescue :fallback)
end
def transfers(a,b)
  result=[]
  [0,1].each do |i|
    next a ? (b ? :next : :other) : :false if i==0
    result << (a ? (b ? 1 : 2) : 3)
    break a ? (b ? :break : :other) : :false
  end
  result
end
p [nil,false,true].product([nil,false,true]).map { |a,b| [modifiers(a,b),scope(a,b),ternaries(a,b),associations(a,b),guards(a,b),inside_protection(a,b),transfers(a,b)] }
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", ALL, "--fix", "a.rb"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "binding finding must remain: {o:?}"
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(source, after);
    assert!(after.contains("if x if (x = a)"));
    assert!(after.contains("unless observe(b) || observe(a)"));
    observations_equal(d.path());
}
#[test]
fn runtime_custom_negation_and_predicate_bindings_remain_unmodified() {
    let source = r#"$trace=[]
class Odd
  def !; $trace << :negated; false; end
end
def x; :method; end
def f
  v=(x if x if (x=true))
  [v,x]
end
value=Odd.new
p (:body unless value if true)
p (:body if value unless false)
p f
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    unchanged(
        d.path(),
        MODIFIER,
        source.as_bytes(),
        &["--only", MODIFIER, "--fix", "a.rb"],
        3,
    );
    observations_equal(d.path());
}
#[test]
fn runtime_closed_negation_and_command_operand_boundaries() {
    let d = project();
    for source in [
        "one = (1 unless false if true)\ntwo = (2 if true unless false)\np [one,two]\n",
        r#"$trace=[]
def café(x); $trace << [:café,x]; false; end
def observe(x); $trace << x; x; end
class Operand
  def +(x); $trace << [:plus,x]; false; end
  def [](x); $trace << [:index,x]; false; end
end
obj=Operand.new
observe(:body) if observe(:inner) if café 1
observe(:body) if observe(:inner) if obj.+ 1
observe(:body) if observe(:inner) if obj.[] 1
p $trace
"#,
    ] {
        fs::write(d.path().join("before.rb"), source).unwrap();
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = cli(d.path(), &["--only", MODIFIER, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(0), "{o:?}");
        assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        observations_equal(d.path());
    }
}
#[test]
fn runtime_modifier_setters_keep_assignment_values_and_precedence() {
    let d = project();
    let source = r#"$trace=[]
class Setter
  attr_reader :value
  def foo=(value); $trace << [:set,value]; @value=value; false; end
end
[false,true].each do |a|
  obj=[]
  work=:body
  p((work if a if obj[1] = 2))
  p obj
  obj=Setter.new
  p((work if a if obj.foo = 1))
  p obj.value
  obj=[]
  p((work if obj[1] = 2 if a))
  p obj
  obj=Setter.new
  p((work if obj.foo = 1 if a))
  p obj.value
  obj=[]
  p((work unless a unless obj[1] = false))
  p obj
  obj=Setter.new
  p((work unless a unless obj.foo = false))
  p obj.value
end
p $trace
"#;
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", MODIFIER, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(after.contains("if (obj[1] = 2) && a"), "{after}");
    assert!(after.contains("if (obj.foo = 1) && a"), "{after}");
    observations_equal(d.path());
}
#[test]
fn runtime_ternary_hash_keys_in_bare_commands_keep_grouping() {
    let d = project();
    for source in [
        "p true ? (false ? 1 : 2) : 3 => 4\n",
        "p :first, true ? (false ? 1 : 2) : 3 => 4\n",
        "p true ? (false ? (true ? 1 : 2) : 3) : 4 => 5\n",
        "p({true ? (false ? 1 : 2) : 3 => 4})\n",
        "p true ? (p false ? (true ? 1 : 2) : 3 => 4) : 5\n",
    ] {
        fs::write(d.path().join("before.rb"), source).unwrap();
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = cli(d.path(), &["--only", TERNARY, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(0), "{source}: {o:?}");
        assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        observations_equal(d.path());
    }
}
#[test]
fn runtime_literals_encoding_bom_crlf_frozen_state_and_line_values() {
    let body = r#"def f(a,b)
  first=a ? (b ? "é\nline
  b" : %q(c
 d)) : /e
 f/x
  second=("value" if a if b)
  [first,second].map { |v| [v.class.to_s,v.is_a?(Regexp) ? v.source.bytes : v.to_s.bytes,v.to_s.encoding.name,v.frozen?] }
end
p [true,false].product([true,false]).map { |a,b| f(a,b) }
"#;
    let d = project();
    for frozen in ["true", "false"] {
        for nl in ["\n", "\r\n"] {
            let source =
                format!("\u{feff}# coding: utf-8\n# frozen_string_literal: {frozen}\n{body}")
                    .replace('\n', nl);
            let source = source_getter::pattern_sibling(d.path(), "a.rb", &source, ALL);
            fs::write(d.path().join("before.rb"), &source).unwrap();
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let o = cli(d.path(), &["--only", ALL, "--fix", "a.rb"]);
            assert_eq!(o.status.code(), Some(0), "{o:?}");
            assert_ne!(fs::read(d.path().join("a.rb")).unwrap(), source.as_bytes());
            observations_equal(d.path());
        }
    }
    for source in [
        "p((__LINE__ if true if true))\np(true ? (false ? __LINE__ : 2) : 3)\np __LINE__\n",
        "p((:body if (__LINE__ == 1 &&\n true) if (__LINE__ == 2 &&\n true)))\n",
    ] {
        fs::write(d.path().join("before.rb"), source).unwrap();
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = cli(d.path(), &["--only", ALL, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(1), "{o:?}");
        observations_equal(d.path());
    }
}
#[test]
fn selected_default_and_style_layout_composition_stabilizes_within_eight_passes() {
    let source = "def f(a,b)\n return a ? (b ? { :key => 'value' } : nil) : false;\nend\ndef g(a,b)\n raise Exception, 'boom' if a if b\nend\n";
    let d = project();
    for selection in [
        Some(ALL),
        Some(
            "Style/NestedModifier,Style/NestedTernaryOperator,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..8 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec![];
            if let Some(s) = selection {
                args.extend(["--only", s]);
            }
            args.extend(["--fix", "a.rb"]);
            let o = cli(d.path(), &args);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if old == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selection:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(!after.contains("if a if b"), "{after}");
        assert!(!after.contains("a ? (b ?"), "{after}");
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
            let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(o.status.code(), Some(2), "{o:?}");
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
