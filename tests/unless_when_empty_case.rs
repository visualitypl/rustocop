//! real CLI policy, no-write and controlled Ruby semantic witnesses.
use std::{fs, path::Path, process::Command};
const CASES: [(&str, &str, &str); 3] = [
    (
        "Style/UnlessElse",
        "unless ready\n  negative\nelse\n  positive\nend\n",
        "if ready\n  positive\nelse\n  negative\nend\n",
    ),
    (
        "Style/WhenThen",
        "case value\nwhen one; work\nend\n",
        "case value\nwhen one then work\nend\n",
    ),
    (
        "Style/EmptyCaseCondition",
        "case\nwhen ready\n  work\nend\n",
        "if ready\n  work\nend\n",
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
fn unchanged(
    root: &Path,
    cop: &str,
    source: &[u8],
    args: &[&str],
    count: usize,
) -> std::process::Output {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(root, args);
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| l.contains(&format!("({cop})")))
            .count(),
        count,
        "{cop}: {}: {output:?}",
        String::from_utf8_lossy(source)
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{output:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    output
}
#[test]
fn defaults_selectors_enabling_and_config_do_not_write_without_fix() {
    let d = project();
    for (cop, source, _) in CASES {
        for (config, mut args, count) in [
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
            args.push("a.rb");
            unchanged(d.path(), cop, source.as_bytes(), &args, count);
        }
        let registry = rustocop::cops::Registry::new();
        let c = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(c.default_enabled() && c.safe_fixes());
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(c.path_policy().is_none() && c.max_setting().is_none());
    }
}
#[test]
fn both_directive_prefixes_excludes_disabled_spans_and_clean_files_never_write() {
    let d = project();
    for (cop, source, after) in CASES {
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
            let line = if cop == "Style/WhenThen" { 1 } else { 0 };
            let mut lines: Vec<_> = source.lines().map(str::to_owned).collect();
            lines[line].push_str(&format!(" # {prefix}:disable {cop}"));
            unchanged(
                d.path(),
                cop,
                format!("{}\n", lines.join("\n")).as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        unchanged(
            d.path(),
            cop,
            after.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            0,
        );
    }
    unchanged(
        d.path(),
        "Style/UnlessElse",
        b"unless ready\n negative\nelse\n # rubocop:disable Lint/Debugger\n positive\nend\n",
        &["--only", "Style/UnlessElse", "--fix", "a.rb"],
        1,
    );
}
#[test]
fn canonical_and_every_pinned_correction_use_the_real_single_pass_pipeline() {
    // RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let d = project();
    for (cop, source, after) in CASES {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let output = cli(
            d.path(),
            &["--only", cop, "--fix", "--fix-passes", "1", "a.rb"],
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        unchanged(
            d.path(),
            cop,
            after.as_bytes(),
            &["--only", cop, "--fix", "--fix-passes", "1", "a.rb"],
            0,
        );
    }
    let legacy = b"# coding: ISO-8859-1\ncase\nwhen true\n '\xe9'\nend\n";
    fs::write(d.path().join("a.rb"), legacy).unwrap();
    let output = cli(
        d.path(),
        &[
            "--only",
            "Style/EmptyCaseCondition",
            "--fix",
            "--fix-passes",
            "1",
            "a.rb",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        fs::read(d.path().join("a.rb")).unwrap(),
        b"# coding: ISO-8859-1\nif true\n '\xe9'\nend\n"
    );
    for (cop, fixture) in [
        (
            "Style/UnlessElse",
            include_str!("fixtures/unless_else_pinned.json"),
        ),
        (
            "Style/WhenThen",
            include_str!("fixtures/when_then_pinned.json"),
        ),
        (
            "Style/EmptyCaseCondition",
            include_str!("fixtures/empty_case_condition_pinned.json"),
        ),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for c in cases {
            if let Some(after) = c["correction"].as_str() {
                fs::write(d.path().join("a.rb"), c["source"].as_str().unwrap()).unwrap();
                let output = cli(
                    d.path(),
                    &["--only", cop, "--fix", "--fix-passes", "1", "a.rb"],
                );
                assert_ne!(output.status.code(), Some(2), "{output:?}");
                assert_eq!(
                    fs::read_to_string(d.path().join("a.rb")).unwrap(),
                    after,
                    "{c}"
                );
            }
        }
    }
}
#[test]
fn actual_missing_else_uses_configured_unless_else_not_cli_selection() {
    let d = project();
    for (config, selection, missing, unless) in [
        ("", "Style/MissingElse", 0, 0),
        ("", "Style/MissingElse,Style/UnlessElse", 0, 0),
        (
            "[cops.\"Style/UnlessElse\"]\nenabled = false\n",
            "Style/MissingElse",
            1,
            0,
        ),
        (
            "[cops.\"Style/UnlessElse\"]\nenabled = false\n",
            "Style/MissingElse,Style/UnlessElse",
            1,
            0,
        ),
        (
            "[cops.Style]\nenabled = false\n[cops.\"Style/UnlessElse\"]\nenabled = true\n",
            "Style/MissingElse",
            0,
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let output = unchanged(
            d.path(),
            "Style/MissingElse",
            b"unless ready\n work\nend\n",
            &["--only", selection, "--fix", "a.rb"],
            missing,
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .matches("(Style/UnlessElse)")
                .count(),
            unless
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    fs::write(d.path().join("a.rb"), CASES[0].1).unwrap();
    let o = cli(
        d.path(),
        &[
            "--only",
            "Style/MissingElse,Style/UnlessElse",
            "--fix",
            "a.rb",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        CASES[0].2
    );
}
#[test]
fn sensitive_line_data_legacy_encoding_and_reflective_files_retain_bytes() {
    let d = project();
    for (cop, source) in [
        (
            "Style/UnlessElse",
            b"unless true\n p __LINE__\nelse\n :no\nend\n".as_slice(),
        ),
        (
            "Style/EmptyCaseCondition",
            b"case\nwhen true\n p __LINE__\nend\n",
        ),
        ("Style/EmptyCaseCondition", b"case\nwhen /a/\n :yes\nend\n"),
        (
            "Style/UnlessElse",
            b"# coding: ISO-8859-1\nunless true\n '\xe9'\nelse\n :no\nend\n",
        ),
    ] {
        unchanged(d.path(), cop, source, &["--only", cop, "--fix", "a.rb"], 1);
    }
    for (cop, source, _) in CASES {
        for prefix in ["p DATA.pos\n", "p File.read(__FILE__)\n"] {
            unchanged(
                d.path(),
                cop,
                format!("{prefix}{source}__END__\npayload\n").as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                1,
            );
        }
        let path = d.path().join("a.rb");
        let invalid = b"case (\n";
        fs::write(&path, invalid).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(2));
        assert_eq!(fs::read(&path).unwrap(), invalid);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}
fn rubies() -> Vec<String> {
    let mut result = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        result.push(ruby);
    }
    result
}
fn observe(ruby: &str, source: &str) -> std::process::Output {
    Command::new(ruby)
        .args(["-W0", "-e", source])
        .output()
        .unwrap()
}
#[test]
fn controlled_ruby_observations_preserve_values_effects_binding_and_control_flow() {
    let d = project();
    for (cop, source) in [
        (
            "Style/UnlessElse",
            r#"
def probe(flag)
  log = []
  value = unless flag
    log << :negative
    42
  else
    log << :positive
    17
  end
  assigned = unless flag
    a = 1
  else
    a = 2
  end
  [value, assigned, a, log]
end
p [probe(nil), probe(false), probe(0), probe(Object.new)]
"#,
        ),
        (
            "Style/UnlessElse",
            r#"
def probe(flag)
  log = []
  begin
    unless flag
      if flag.nil?
        return :nil
      elsif flag == false
        log << :false
      else
        raise "bad"
      end
    else
      begin
        log << :yes
        raise "caught"
      rescue RuntimeError
        log << :rescued
      ensure
        log << :ensured
      end
    end
  ensure
    log << :outer
  end
  log
end
p [probe(nil), probe(false), probe(true)]
"#,
        ),
        (
            "Style/WhenThen",
            r#"
log = []
values = [1,2,3].map do |n|
 case n
 when (log << :first; 1); log << :a; 10
 when (log << :second; 2); log << :b; 20
 else :other
 end
end
p [values,log]
"#,
        ),
        (
            "Style/EmptyCaseCondition",
            r#"
log = []
values = [false,nil,0,true].map do |n|
 value = case
 when (log << :a; n), (log << :b; false)
   begin
     log << :yes
     :result
   rescue
     :rescued
   ensure
     log << :ensured
   end
 when (x = 7), (log << :never; true)
   x
 else
   :no
 end
 [value, x]
end
p [values,log]
"#,
        ),
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert_ne!(source, after, "{cop} must exercise an actual correction");
        for ruby in rubies() {
            if Command::new(&ruby).arg("--version").output().is_err() {
                continue;
            }
            let before = observe(&ruby, source);
            let after = observe(&ruby, &after);
            assert!(before.status.success(), "{before:?}");
            assert_eq!(
                (before.status.code(), before.stdout, before.stderr),
                (after.status.code(), after.stdout, after.stderr),
                "{cop}: {ruby}"
            );
        }
    }
}
#[test]
fn lead_setter_and_english_not_cli_runtime_regressions() {
    if crate::support::runtime::skips("asserts the Ruby 3.4 grammar, which Ruby 4.0 changed") {
        return;
    }
    let d = project();
    if let Ok(ruby) = crate::support::runtime::ruby() {
        assert!(
            String::from_utf8_lossy(&observe(&ruby, "print RUBY_VERSION").stdout)
                .starts_with(crate::support::runtime::version_prefix())
        );
    } else {
        eprintln!("SKIP native Ruby 3.4 when-not runtime: set RUSTOCOP_RUBY34");
    }
    for (source, expected, observation) in [
        (
            "a=[]; r = case\nwhen a[0] = false, true\n :yes\nelse\n :no\nend; p [r,a]",
            "a=[]; r = if (a[0] = false) || true\n :yes\nelse\n :no\nend; p [r,a]",
            "[:yes, [false]]\n",
        ),
        (
            "r = case\nwhen not false\n :yes\nelse\n :no\nend; p r",
            "r = if (not false)\n :yes\nelse\n :no\nend; p r",
            ":yes\n",
        ),
        (
            "r = case\nwhen not false, true\n :yes\nelse\n :no\nend; p r",
            "r = if (not false) || true\n :yes\nelse\n :no\nend; p r",
            ":yes\n",
        ),
        (
            "r = case\nwhen false, not false\n :yes\nelse\n :no\nend; p r",
            "r = if false || (not false)\n :yes\nelse\n :no\nend; p r",
            ":yes\n",
        ),
        (
            "def probe(*v); v.first; end\nr = case\nwhen false\n :first\nwhen not probe false, false\n :yes\nelse\n :no\nend; p r",
            "def probe(*v); v.first; end\nr = if false\n :first\nelsif (not probe false, false)\n :yes\nelse\n :no\nend; p r",
            ":yes\n",
        ),
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = cli(
            d.path(),
            &["--only", "Style/EmptyCaseCondition", "--fix", "a.rb"],
        );
        assert_eq!(o.status.code(), Some(0), "{source}: {o:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert_eq!(after, expected);
        for ruby in rubies() {
            if Command::new(&ruby).arg("--version").output().is_err() {
                continue;
            }
            if source.contains("not ")
                && !String::from_utf8_lossy(&observe(&ruby, "print RUBY_VERSION").stdout)
                    .starts_with(crate::support::runtime::version_prefix())
            {
                eprintln!("SKIP unparenthesized when-not on non-3.4 runtime {ruby}");
                continue;
            }
            for text in [source, &after] {
                let o = observe(&ruby, text);
                assert!(o.status.success(), "{ruby}: {o:?}");
                assert_eq!(o.stdout, observation.as_bytes(), "{ruby}: {text}");
                assert!(o.stderr.is_empty(), "{ruby}: {o:?}");
            }
        }
    }
}
#[test]
fn assignment_and_call_shape_cli_fixes_preserve_runtime_observations() {
    let d = project();
    let prelude = r#"
class Probe
 attr_reader :value, :log
 def initialize(value); @value=value; @log=[]; end
 def value=(value); @log << [:write, value]; @value=value; :setter_return; end
 def [](index); @log << [:read_index, index]; @value; end
 def []=(index,value); @log << [:write_index,index,value]; @value=value; :setter_return; end
 def probe(*args); @log << [:call,args]; block_given? ? yield : args.first; end
 def call(*args); probe(*args); end
end
class Host
 x=false; @x=false; @@x=false; $x=false; C=false; A=Module.new; A::C=false
 a=[false]; obj=Probe.new(false); log=[]
"#;
    for (setup, expression) in [
        ("", "x = false"),
        ("", "@x = false"),
        ("", "@@x = false"),
        ("", "$x = false"),
        ("", "C = false"),
        ("", "A::C = false"),
        ("", "a[0] = false"),
        ("", "obj.value = false"),
        ("", "obj&.value = false"),
        ("obj=nil", "obj&.value = false"),
        ("x=true", "x &&= false"),
        ("", "x ||= false"),
        ("x=0", "x += 1"),
        ("obj=Probe.new(true)", "obj.value &&= false"),
        ("", "obj.value ||= false"),
        ("obj=Probe.new(0)", "obj.value += 1"),
        ("obj=Probe.new(true)", "obj[0] &&= false"),
        ("", "obj[0] ||= false"),
        ("obj=Probe.new(0)", "obj[0] += 1"),
        ("obj=Probe.new(true)", "obj&.value &&= false"),
        ("", "obj&.value ||= false"),
        ("obj=Probe.new(0)", "obj&.value += 1"),
        ("", "a[(log << :index; 0)] = (log << :rhs; false)"),
        ("", "(log << :receiver; obj).value = (log << :rhs; false)"),
        ("", "obj.value=(false)"),
        ("", "obj.[]=(0,false)"),
        ("", "obj.probe(false)"),
        ("", "obj&.probe(false)"),
        ("", "obj::probe(false)"),
        ("", "obj.(false)"),
        ("", "obj.probe { log << :block; false }"),
        ("", "obj.probe do log << :block; false end"),
        ("", "!obj.probe(false)"),
        ("", "not(obj.probe(false))"),
        ("", "defined? obj.probe"),
    ] {
        for operands in [
            expression.to_owned(),
            format!("{expression}, true"),
            format!("false, {expression}"),
        ] {
            let source = format!(
                "{prelude}{setup}\nr = case\nwhen {operands}\n :yes\nelse\n :no\nend\np [r,x,@x,@@x,$x,C,A::C,a,obj&.value,obj&.log,log]\nend\n"
            );
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let o = cli(
                d.path(),
                &["--only", "Style/EmptyCaseCondition", "--fix", "a.rb"],
            );
            assert_eq!(o.status.code(), Some(0), "{source}: {o:?}");
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            assert_ne!(source, after);
            for ruby in rubies() {
                if Command::new(&ruby).arg("--version").output().is_err() {
                    continue;
                }
                let before = observe(&ruby, &source);
                let after = observe(&ruby, &after);
                assert!(before.status.success(), "{ruby}: {source}: {before:?}");
                assert_eq!(
                    (before.status.code(), before.stdout, before.stderr),
                    (after.status.code(), after.stdout, after.stderr),
                    "{ruby}: {source}"
                );
            }
        }
    }
}
#[test]
fn uncertain_not_comment_edit_does_not_block_unrelated_safe_cli_fixes() {
    if crate::support::runtime::skips("asserts the Ruby 3.4 grammar, which Ruby 4.0 changed") {
        return;
    }
    let d = project();
    let source = "a=[]\nblocked = case\nwhen not # keep operand comment\n false\n :yes\nelse\n :no\nend\nsafe = case\nwhen a[0] = false, true\n :yes\nelse\n :no\nend\ncase :x\nwhen :x; a << :done\nend\np [blocked,safe,a]\n";
    let expected = "a=[]\nblocked = case\nwhen not # keep operand comment\n false\n :yes\nelse\n :no\nend\nsafe = if (a[0] = false) || true\n :yes\nelse\n :no\nend\ncase :x\nwhen :x then a << :done\nend\np [blocked,safe,a]\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(
        d.path(),
        &[
            "--only",
            "Style/EmptyCaseCondition,Style/WhenThen",
            "--fix",
            "a.rb",
        ],
    );
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .matches("(Style/EmptyCaseCondition)")
            .count(),
        1
    );
    unchanged(
        d.path(),
        "Style/EmptyCaseCondition",
        expected.as_bytes(),
        &[
            "--only",
            "Style/EmptyCaseCondition,Style/WhenThen",
            "--fix",
            "a.rb",
        ],
        1,
    );
    if let Ok(ruby) = crate::support::runtime::ruby() {
        assert!(
            String::from_utf8_lossy(&observe(&ruby, "print RUBY_VERSION").stdout)
                .starts_with(crate::support::runtime::version_prefix())
        );
        for text in [source, expected] {
            let o = observe(&ruby, text);
            assert!(o.status.success(), "{o:?}");
            assert_eq!(o.stdout, b"[:yes, :yes, [false, :done]]\n");
            assert!(o.stderr.is_empty(), "{o:?}");
        }
    } else {
        eprintln!("SKIP native Ruby 3.4 comment/command runtime: set RUSTOCOP_RUBY34");
    }
}
#[test]
fn mixed_default_style_layout_and_selected_peer_corrections_stabilize_without_semantic_drift() {
    let d = project();
    let source = "def probe(flag)\n  result = unless flag\n    { :name => 'no' }\n  else\n    { :name => 'yes' }\n  end\n  return result\nend\nvalues = [probe(false), probe(true)]\ncase\nwhen false; values << :bad\nwhen true then values << :ok\nend\np values\n";
    for selectors in [
        vec![],
        vec![
            "--only",
            "Style/UnlessElse,Style/WhenThen,Style/EmptyCaseCondition,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ],
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        let mut args = selectors;
        args.extend(["--fix", "a.rb"]);
        for pass in 0..8 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let output = cli(d.path(), &args);
            assert_ne!(output.status.code(), Some(2), "{output:?}");
            let after = fs::read(d.path().join("a.rb")).unwrap();
            if pass == 0 {
                assert_ne!(before, after);
            }
            if before == after {
                stable = true;
                break;
            }
        }
        assert!(
            stable,
            "curated mixed fixture must stabilize within eight explicit passes"
        );
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        for ruby in rubies() {
            if Command::new(&ruby).arg("--version").output().is_err() {
                continue;
            }
            let before = observe(&ruby, source);
            let after = observe(&ruby, &after);
            assert!(before.status.success());
            assert_eq!(
                (before.status.code(), before.stdout, before.stderr),
                (after.status.code(), after.stdout, after.stderr),
                "{ruby}"
            );
        }
    }
}
#[test]
fn controlled_witnesses_demonstrate_why_report_only_edits_are_withheld() {
    let d = project();
    for (cop, source, naive) in [
        (
            "Style/UnlessElse",
            "def a; :method; end\np(unless false\n a\nelse\n a = :local\nend)\n",
            "def a; :method; end\np(if false\n a = :local\nelse\n a\nend)\n",
        ),
        (
            "Style/UnlessElse",
            "value = unless false\n proc { a = :block }\nelse\n a = :outer\nend\np [value.call, a]\n",
            "value = if false\n a = :outer\nelse\n proc { a = :block }\nend\np [value.call, a]\n",
        ),
        (
            "Style/EmptyCaseCondition",
            "$_ = 'no'\nvalue = case\nwhen /yes/\n :truthy\nelse\n :falsey\nend\np value\n",
            "$_ = 'no'\nvalue = if /yes/\n :truthy\nelse\n :falsey\nend\np value\n",
        ),
        (
            "Style/EmptyCaseCondition",
            "$_ = 'no'\nvalue = case\nwhen false || /yes/\n :truthy\nelse\n :falsey\nend\np value\n",
            "$_ = 'no'\nvalue = if false || /yes/\n :truthy\nelse\n :falsey\nend\np value\n",
        ),
        (
            "Style/UnlessElse",
            "unless false\n a = 1\nelse\n b = 2\nend\np local_variables\n",
            "if false\n b = 2\nelse\n a = 1\nend\np local_variables\n",
        ),
    ] {
        unchanged(
            d.path(),
            cop,
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
        for ruby in rubies() {
            if Command::new(&ruby).arg("--version").output().is_err() {
                continue;
            }
            let before = observe(&ruby, source);
            let bad = observe(&ruby, naive);
            assert!(
                before.status.success() && bad.status.success(),
                "{before:?} {bad:?}"
            );
            assert_ne!(before.stdout, bad.stdout, "{cop}: {ruby}");
        }
    }
}
