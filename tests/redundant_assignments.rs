//! real pipeline/CLI contracts and controlled Ruby semantic witnesses.
//! Pinned fixtures ported from RuboCop (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{fs, path::Path, process::Command};
const ASSIGN: &str = "Style/RedundantAssignment";
const SELF: &str = "Style/RedundantSelfAssignment";
const BRANCH: &str = "Style/RedundantSelfAssignmentBranch";
const ALL: &str =
    "Style/RedundantAssignment,Style/RedundantSelfAssignment,Style/RedundantSelfAssignmentBranch";
const CASES: [(&str, &str, &str); 3] = [
    (
        ASSIGN,
        "def f\n  x = work\n  x\nend\n",
        "def f\n  work\n  \nend\n",
    ),
    (SELF, "foo = foo.concat(ary)\n", "foo = foo.concat(ary)\n"),
    (
        BRANCH,
        "foo = condition ? bar : foo\n",
        "foo = bar if condition\n",
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
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(root, args);
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| l.contains(cop))
            .count(),
        count,
        "{output:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{output:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
}
fn fix(root: &Path, cop: &str, before: &str, after: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let output = cli(root, &["--only", cop, "--fix", "a.rb"]);
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        fs::read_to_string(root.join("a.rb")).unwrap(),
        after,
        "{output:?}"
    );
}
#[test]
fn defaults_selectors_config_and_no_implicit_writes() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (cop, before, _) in CASES {
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled());
        assert_eq!(rule.safe_fixes(), cop != SELF);
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
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
            unchanged(d.path(), cop, before.as_bytes(), &args, count);
        }
    }
}
#[test]
fn excludes_directives_and_explicit_fix_respect_policy() {
    let d = project();
    for (cop, before, after) in CASES {
        for config in [
            "exclude = ['a.rb']\n".to_string(),
            format!("[cops.\"{cop}\"]\nexclude = ['a.rb']\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                before.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                let source = format!("# {prefix}:disable {target}\n{before}");
                unchanged(
                    d.path(),
                    cop,
                    source.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            let source =
                format!("# {prefix}:disable {cop}\n{before}# {prefix}:enable {cop}\n{before}");
            let expected =
                format!("# {prefix}:disable {cop}\n{before}# {prefix}:enable {cop}\n{after}");
            fix(d.path(), cop, &source, &expected);
        }
    }
}
#[test]
fn all_pinned_safe_corrections_and_unsafe_no_write_holds() {
    let d = project();
    let mut canonical = 0;
    let mut unsafe_holds = 0;
    let mut result_holds = Vec::new();
    for (cop, fixture) in [
        (
            ASSIGN,
            include_str!("fixtures/redundant_assignment_pinned.json"),
        ),
        (
            SELF,
            include_str!("fixtures/redundant_self_assignment_pinned.json"),
        ),
        (
            BRANCH,
            include_str!("fixtures/redundant_self_assignment_branch_pinned.json"),
        ),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for c in cases {
            let Some(after) = c["correction"].as_str() else {
                continue;
            };
            let before = c["source"].as_str().unwrap();
            if cop == SELF || (cop == BRANCH && before.starts_with("def ")) {
                unchanged(
                    d.path(),
                    cop,
                    before.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    1,
                );
                if cop == SELF {
                    unsafe_holds += 1
                } else {
                    result_holds.push(c["spec_line"].as_u64().unwrap());
                }
            } else {
                fix(d.path(), cop, before, after);
                unchanged(
                    d.path(),
                    cop,
                    after.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
                canonical += 1;
            }
        }
    }
    assert_eq!(canonical, 21);
    assert_eq!(unsafe_holds, 9);
    assert_eq!(result_holds, [166, 268]);
}
#[test]
fn stdin_has_exact_positions_selectors_and_no_disk_writes() {
    use std::{io::Write, process::Stdio};
    let d = project();
    for (cop, before, _) in CASES {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let text = String::from_utf8_lossy(&output.stdout);
        let pos = if cop == ASSIGN {
            "a.rb:2:3:"
        } else if cop == SELF {
            "a.rb:1:5:"
        } else {
            "a.rb:1:25:"
        };
        assert!(text.contains(pos) && text.contains(cop), "{text}");
        assert!(!d.path().join("a.rb").exists());
    }
}
#[test]
fn controlled_truthiness_binding_evaluation_setter_result_and_control_flow() {
    let source = r#"$trace = []
def work(value)
  $trace << value
  value
end
def direct(value)
  x = work(value)
  x
end
def arms(c)
  if c
    x = work(:yes)
    x
  else
    x = work(:no)
    x
  end
end
def returned(foo, c)
  foo = c ? foo : work(:changed)
end
def discarded(foo, c)
  foo = c ? foo : work(:changed)
  foo
end
def captured
  x = nil
  f = -> { x }
  x = f
  x
end
p [nil,false,true].map { |v| [direct(v),returned(v,true),discarded(v,true)] }
p [arms(true),arms(false),discarded(:old,false)]
p captured.call.equal?(captured.call)
foo = nil
p [1].map { |foo| foo = false ? foo : work(:block); foo }
p [1].map { |;foo| foo = false ? foo : work(:local); foo }
2.times { foo = false ? foo : work(:captured) }
p foo
foo = (foo.nil? || false) ? foo : work(:unused)
p foo
foo = if false or false
  foo
else
  not false
end
p foo
text = if false
  text
else
  <<~TEXT
    value é
  TEXT
end
p text
class Receiver
  def value
    $trace << :get
    @value ||= []
  end
  def value=(value)
    $trace << [:set, value]
    @value = value
    :not_the_assignment_result
  end
end
obj = Receiver.new
p(obj.value = obj.value.concat([:element]))
$watched = []
trace_var(:$watched) { |v| $trace << [:hook, v.dup] }
$watched = $watched.concat([:global])
untrace_var(:$watched)
class FrozenReceiver
  def initialize
    @value = []
    freeze
  end
  def run
    @value = @value.concat([:mutated])
  end
end
begin
  FrozenReceiver.new.run
rescue FrozenError
  $trace << :frozen
end
def flow(flag)
  if flag
    x = work(:normal)
    x
  else
    x = begin
      return :early if !flag
      :unreachable
    end
    x
  end
ensure
  $trace << :ensure
end
p [flow(true),flow(false)]
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let output = cli(d.path(), &["--only", ALL, "--fix", "a.rb"]);
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(source, after);
    assert!(after.contains("foo = work(:changed) unless c"));
    assert!(
        after.contains("obj.value = obj.value.concat")
            && after.contains("$watched = $watched.concat")
    );
    assert!(after.contains("x = f") && !after.contains("x = work(value)"));
    let locals = "foo = proc { local_variables }.call.include?(:foo) ? foo : 1\np foo\ndef table; x = local_variables; x; end\np table\n";
    fs::write(d.path().join("locals.rb"), locals).unwrap();
    let held = cli(d.path(), &["--only", ALL, "--fix", "locals.rb"]);
    assert_eq!(held.status.code(), Some(1), "{held:?}");
    assert_eq!(
        fs::read_to_string(d.path().join("locals.rb")).unwrap(),
        locals
    );
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let table = Command::new(&ruby)
            .arg(d.path().join("locals.rb"))
            .output()
            .unwrap();
        assert!(table.status.success(), "{ruby}: {table:?}");
        assert_eq!(table.stdout, b"nil\n[:x]\n", "{ruby}");
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
        assert_eq!(before.stderr, after.stderr, "{ruby}");
        let observed = String::from_utf8_lossy(&after.stdout);
        assert!(
            observed.contains("value é")
                && observed.contains("[:hook, [:global]]")
                && observed.contains(":frozen")
                && observed.contains(":early"),
            "{observed}"
        );
    }
}
#[test]
fn compound_local_table_observations_survive_fix() {
    let d = project();
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for operator in ["||=", "+="] {
        let source = format!(
            "def local_variables=(value); end\ndef f; x = (self.local_variables {operator} []); x; end\np f\nfoo = (self.local_variables {operator} []).include?(:foo) ? foo : 1\np foo\n"
        );
        fs::write(d.path().join("before.rb"), &source).unwrap();
        fs::write(d.path().join("a.rb"), &source).unwrap();
        let output = cli(d.path(), &["--only", ALL, "--fix", "a.rb"]);
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        for ruby in &rubies {
            if Command::new(ruby).arg("--version").output().is_err() {
                continue;
            }
            let before = Command::new(ruby)
                .arg(d.path().join("before.rb"))
                .output()
                .unwrap();
            let after = Command::new(ruby)
                .arg(d.path().join("a.rb"))
                .output()
                .unwrap();
            assert!(before.status.success() && after.status.success());
            assert_eq!(before.stdout, b"[:x]\nnil\n", "{ruby}: {operator}");
            assert_eq!(before.stdout, after.stdout, "{ruby}: {operator}");
            assert_eq!(before.stderr, after.stderr, "{ruby}: {operator}");
        }
        for cop in [ASSIGN, BRANCH] {
            unchanged(
                d.path(),
                cop,
                source.as_bytes(),
                &["--only", ALL, "--fix", "a.rb"],
                1,
            );
        }
    }
}
#[test]
fn terminal_rescue_modifier_corrections_preserve_runtime_results() {
    let d = project();
    let before = "def f(fail); (raise if fail; :ok) rescue (x = 1; x); end\ndef g(fail); (x = (raise if fail; :ok); x) rescue :rescued; end\np [f(false), f(true), g(false), g(true)]\n";
    let after = "def f(fail); (raise if fail; :ok) rescue (1; ); end\ndef g(fail); ((raise if fail; :ok); ) rescue :rescued; end\np [f(false), f(true), g(false), g(true)]\n";
    fix(d.path(), ASSIGN, before, after);
    fs::write(d.path().join("before.rb"), before).unwrap();
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        for path in ["before.rb", "a.rb"] {
            let output = Command::new(&ruby)
                .arg(d.path().join(path))
                .output()
                .unwrap();
            assert!(output.status.success(), "{ruby}: {output:?}");
            assert_eq!(output.stdout, b"[:ok, 1, :ok, :rescued]\n", "{ruby}");
            assert!(output.stderr.is_empty(), "{ruby}: {output:?}");
        }
    }
}
#[test]
fn mixed_style_layout_composition_stabilizes_within_six_explicit_passes() {
    let d = project();
    let source = "def f\n  x = ({ :key => 'value' });\n  x\nend\nfoo = condition ? foo : 'other'\n";
    let selected = format!(
        "{ALL},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/EndAlignment,Layout/IndentationWidth,Layout/TrailingWhitespace"
    );
    fs::write(d.path().join("a.rb"), source).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        let output = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        if old == fs::read(d.path().join("a.rb")).unwrap() {
            stable = true;
            break;
        }
    }
    assert!(stable);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(
        after.contains("key: \"value\"")
            && !after.contains("x =")
            && after.contains("foo = \"other\" unless condition"),
        "{after}"
    );
}
