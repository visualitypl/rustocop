//! real CLI, exact bytes and controlled Ruby observations.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Style/MethodCallWithArgsParentheses";
const BEFORE: &str = "object.test 1\n";
const AFTER: &str = "object.test(1)\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    output
}

#[test]
fn opt_in_fix_selectors_config_and_no_write_contract() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, BEFORE).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--fix", "lib/a.rb"], 0);
    cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
    cli(
        dir.path(),
        &["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
        0,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    for config in [
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
        "[cops.Style]\nenabled = false\n".into(),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), AFTER);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        fs::write(&path, BEFORE).unwrap();
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    cli(dir.path(), &["--fix", "lib/a.rb"], 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), AFTER);
}

#[test]
fn directives_exclusions_and_stdin_preserve_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "MethodCallWithArgsParentheses", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{BEFORE}"),
                format!("object.test 1 # {prefix}:disable {target}\n"),
                format!("# {prefix}:disable-next {target}\nobject.test 1,\n  2\n"),
                format!("object.test 1,\n  2 # {prefix}:disable {target}\n"),
            ] {
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
        // The highlight and correction stop before the literal block body.
        let before = format!("object.test 1 do\n  body # {prefix}:disable {COP}\nend\n");
        fs::write(&path, &before).unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before.replacen("test 1", "test(1)", 1)
        );
        // A directive within a physical argument span withholds the entire
        // atomic edit, even if the offense's send location ends earlier.
        let before =
            format!("object.test <<~TEXT,\n  body # literal\nTEXT\n  1 # {prefix}:disable {COP}\n");
        fs::write(&path, &before).unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_string(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(&path, BEFORE).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--stdin", "lib/a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(BEFORE.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("lib/a.rb:1:1:"));
    cli(
        dir.path(),
        &["--only", COP, "--fix", "--stdin", "lib/a.rb"],
        2,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
}

#[test]
fn clean_macro_operator_setter_forwarding_and_error_files_are_not_written() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, status) in [
        (AFTER, 0),
        (
            "macro :value\nobject.foo = 1\nobject[1] = 2\nobject + 1\n",
            0,
        ),
        ("def f(...); object.test(...); end\n", 0),
        ("object.test(\n", 2),
        ("yield\n", 2),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn statement_only_arguments_and_separators_do_not_abort_independent_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for argument in [
        "alias copied original",
        "alias $copied $original",
        "undef original",
        "1;",
        ";1",
        ";1;",
        "1; # trailing separator\n",
        "# leading separator\n;1",
    ] {
        let before = format!("self.sink ({argument})\nobject.test 2\n");
        let after = format!("self.sink(({argument}))\nobject.test(2)\n");
        fs::write(&path, &before).unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn post_execution_arguments_do_not_abort_independent_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for argument in [
        "END {}",
        "END { 1 }",
        "# before END\nEND { 1 } # after END\n",
        "\n  END { # deferred body\n    __LINE__\n  }\n",
        "\r\n  END { # deferred body\r\n    __LINE__\r\n  } # after END\r\n",
    ] {
        let before = format!("self.sink ({argument})\nself.other 1\n");
        let after = format!("self.sink(({argument}))\nself.other(1)\n");
        fs::write(&path, &before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let lint = cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
        let report = String::from_utf8(lint.stdout).unwrap();
        assert_eq!(report.matches(COP).count(), 2, "{report}");
        assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("lib/a.rb")
        .output()
        .unwrap()
}

#[test]
fn controlled_ruby_observes_post_execution_arguments_and_at_exit_order() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let prelude = r#"$events = []
at_exit { $events << :last; p $events }
at_exit { $events << :earlier }
def sink(*values); $events << [:sink, values]; end
def other(value); $events << [:other, value]; end
"#;
    let tail = "at_exit { $events << :later }\n$events << :body\n";
    for (argument, deferred) in [
        ("END {}", ""),
        ("END { 1 }", ""),
        (
            "# before END\nEND { $events << [:end, __LINE__] } # after END\n",
            "[:end, 7], ",
        ),
        (
            "\n  END { # deferred body\n    $events << [:end, __LINE__]\n  }\n",
            "[:end, 8], ",
        ),
    ] {
        let source = format!("{prelude}self.sink ({argument})\nself.other 1\n{tail}");
        let fixed = format!("{prelude}self.sink(({argument}))\nself.other(1)\n{tail}");
        fs::write(&path, &source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success(), "{before:?}");
        assert!(before.stderr.is_empty(), "{before:?}");
        // END evaluates to nil, regardless of its deferred body's value.
        // Both independent calls run once before LIFO at-exit callbacks, and
        // the nonempty END bodies run exactly once with unchanged __LINE__.
        let expected =
            format!("[[:sink, [nil]], [:other, 1], :body, :later, {deferred}:earlier, :last]\n");
        assert_eq!(before.stdout, expected.as_bytes(), "{source}");
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), fixed.as_bytes());
        let after = ruby(dir.path());
        assert_eq!(after.status.code(), before.status.code(), "{after:?}");
        assert_eq!(after.stdout, before.stdout);
        assert_eq!(after.stderr, before.stderr);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), fixed.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn controlled_ruby_observes_identical_arguments_binding_control_flow_and_lines() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = r#"$events = []
class Trace
  def take(*args, **kwargs, &block)
    $events << [:take, args, kwargs, block ? block.call : :no_block]
    args
  end
  def []=(key, value); $events << [:index, key, value]; end
  def value; @value ||= 0; end
  def value=(value); @value = value; end
end
$trace = Trace.new
def receiver; $events << :receiver; $trace; end
def argument(value, &block)
  $events << [:argument, value, block ? block.call : :no_block]
  value
end
def outer(value, &block)
  $events << [:outer, value, block ? block.call : :no_block]
  value
end
def run
  outer receiver.take argument(:do_end) do :outer_block end
  outer receiver.take argument(:braces) { :argument_block }
  receiver.take argument(1), argument(2), key: argument(3)
  receiver.take *[argument(:splat)], **{key: argument(:keyword)}, &-> { :passed_block }
  receiver.take (argument(true) and argument(:and))
  receiver.take (argument(false) or argument(:or))
  receiver.take (not argument(false))
  receiver.take (argument(:if) if argument(true))
  receiver.take (argument(:ternary) ? :yes : :no)
  receiver.take (argument(1) + 2)
  receiver.take (a, b = 1, 2)
  receiver.take (raise('expected') rescue :rescued)
  receiver.take argument(:modifier) if true
  receiver.take raise('expected') rescue $events << :outer_rescue
  receiver.take <<~TEXT, argument(:after_header) # retain comment
    café #{argument(:interpolation)}
  TEXT
  receiver.take <<~A, <<~B
    first
  A
    second
  B
  receiver.take <<~TEXT,
    multiline header
  TEXT
    __LINE__
  receiver.take \
    __LINE__
  receiver.take 1, # keep comment
    __LINE__
  receiver.take yield argument(:yield)
  receiver[argument(:index)] = argument(:value)
  receiver.value += argument(4)
  nil&.take argument(:never_evaluated)
  receiver&.take argument(:safe_navigation)
  receiver::take argument(:colon_dispatch)
  [1, 2].map { |v| next receiver.take v }
  receiver.take(__LINE__)
  $events << [__LINE__, DATA.read]
end
run { |value| [:yielded, value] }
p $events
__END__
raw payload
"#;
    // DATA acquisition now withholds byte-changing call edits, even
    // though this fixture previously observed only the unchanged payload.
    fs::write(&path, source).unwrap();
    let observed = ruby(dir.path());
    assert!(observed.status.success(), "{observed:?}");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 1);
    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert_eq!(ruby(dir.path()).stdout, observed.stdout);
    // Retain the full correcting/binding/LINE witness with an inert payload
    // counterpart, not by weakening the new conservative DATA boundary.
    let source = source.replace("DATA.read", "\"raw payload\\n\"");
    fs::write(&path, &source).unwrap();
    let before = ruby(dir.path());
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.stdout, observed.stdout);
    // Explicit bounded passes: atomic outer commands win over nested edits.
    for _ in 0..4 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        let after = ruby(dir.path());
        assert!(after.status.success(), "{after:?}");
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
    cli(dir.path(), &["--only", COP, "lib/a.rb"], 0);
    let fixed = fs::read_to_string(&path).unwrap();
    assert_eq!(source.lines().count(), fixed.lines().count());
    assert!(fixed.contains("outer(receiver.take(argument(:do_end))) do :outer_block end"));
    assert!(fixed.contains("outer(receiver.take(argument(:braces) { :argument_block }))"));
    assert!(fixed.ends_with("raw payload\n"));
    let output = String::from_utf8_lossy(&before.stdout);
    assert!(output.contains("outer_block"));
    assert!(output.contains("argument_block"));
    assert!(output.contains("yielded"));
    assert!(!output.contains("never_evaluated"));
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn incompatible_peers_defer_atomically_without_silently_disabling_either_cop() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let registry = rustocop::cops::Registry::new();
    for (peer, source) in [
        ("Style/RedundantParentheses", "object.test ((1))\n"),
        ("Style/NestedParenthesizedCalls", "object.test(inner 1)\n"),
        ("Style/RescueModifier", "object.test (a rescue b)\n"),
        ("Style/TernaryParentheses", "object.test ((a) ? b : c)\n"),
        ("Style/TrailingCommaInArguments", "object.test inner(1,)\n"),
    ] {
        if !registry.has_cop(peer) {
            eprintln!("{peer} overlap awaits its implementation; {COP} remains enabled");
            continue;
        }
        fs::write(&path, source).unwrap();
        let only = format!("{COP},{peer}");
        let output = cli(dir.path(), &["--only", &only, "lib/a.rb"], 1);
        let report = String::from_utf8_lossy(&output.stdout);
        assert!(report.contains(COP));
        assert!(report.contains(peer));
        for _ in 0..4 {
            let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", &only, "--fix", "lib/a.rb"])
                .output()
                .unwrap();
            assert_ne!(output.status.code(), Some(2), "{output:?}");
        }
        let fixed = fs::read_to_string(&path).unwrap();
        if peer == "Style/RedundantParentheses" {
            assert_eq!(fixed, "object.test(1)\n");
        }
        // RescueModifier retains dynamic/backtrace-sensitive
        // offenses rather than expanding them onto different physical lines.
        // The call-parentheses fix can settle while that offense remains.
        let remaining = i32::from(peer == "Style/RescueModifier");
        let output = cli(
            dir.path(),
            &["--only", &only, "--fix", "lib/a.rb"],
            remaining,
        );
        if remaining == 1 {
            let report = String::from_utf8_lossy(&output.stdout);
            assert!(report.contains(peer));
            assert!(!report.contains(&format!("({COP})")));
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), fixed);
    }
}

#[test]
fn mixed_style_layout_is_deterministic_and_stabilizes_within_a_finite_bound() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def f\n  return (self::take 'x',{:key => 1});  \nend\n";
    let only = "Style/MethodCallWithArgsParentheses,Style/ColonMethodCall,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout";
    let mut first_pass = None;
    for _ in 0..2 {
        fs::write(&path, source).unwrap();
        for pass in 0..4 {
            let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", only, "--fix", "lib/a.rb"])
                .output()
                .unwrap();
            assert_ne!(output.status.code(), Some(2), "{output:?}");
            if pass == 0 {
                let bytes = fs::read(&path).unwrap();
                if let Some(expected) = &first_pass {
                    assert_eq!(&bytes, expected);
                } else {
                    first_pass = Some(bytes);
                }
            }
        }
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "def f\n  self.take(\"x\", { key: 1 })\nend\n"
        );
        cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 0);
    }
}
