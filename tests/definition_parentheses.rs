//! real CLI controls and isolated, controlled Ruby observations.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const METHOD: &str = "Style/MethodDefParentheses";
const ENDLESS: &str = "Style/AmbiguousEndlessMethodDefinition";
const ONLY: &str = "Style/MethodDefParentheses,Style/AmbiguousEndlessMethodDefinition";
const CASES: [(&str, &str, &str); 2] = [
    (METHOD, "def f arg; arg; end\n", "def f(arg); arg; end\n"),
    (
        ENDLESS,
        "def f = true if condition\n",
        "def f\n  true\nend if condition\n",
    ),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    out
}
#[test]
fn defaults_selectors_config_and_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let registry = rustocop::cops::Registry::new();
    for (cop, before, after) in CASES {
        let registered: Vec<_> = registry.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(registered.len(), 1);
        assert!(registered[0].default_enabled());
        assert!(registered[0].safe_fixes());
        assert!(registered[0].path_policy().is_none());
        assert_eq!(
            registered[0].ruby_versions().minimum,
            if cop == ENDLESS {
                Some(rustocop::cops::policy::RubyVersion(3, 0))
            } else {
                None
            }
        );
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(String::from_utf8_lossy(&cli(dir.path(), &["lib/a.rb"], 1).stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for config in [
            // Isolate the rules from independent single-line fixes and
            // the newly default-enabled boolean-tail Naming diagnostic.
            format!(
                "[cops.\"Naming/PredicateMethod\"]\nenabled = false\n[cops.\"{cop}\"]\nenabled = false\n[cops.\"Style/SingleLineMethods\"]\nenabled = false\n"
            ),
            "[cops.\"Naming/PredicateMethod\"]\nenabled = false\n[cops.Style]\nenabled = false\n"
                .into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            cli(dir.path(), &["--fix", "lib/a.rb"], 0);
            cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
            cli(
                dir.path(),
                &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"Naming/PredicateMethod\"]\nenabled = false\n[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn excludes_directives_and_atomic_parameter_span() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, _) in CASES {
        for config in [
            "exclude = [\"lib/**\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/**\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            cli(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                for directive in ["disable", "disable-next"] {
                    let source = format!("# {prefix}:{directive} {target}\n{before}");
                    fs::write(&path, &source).unwrap();
                    let modified = fs::metadata(&path).unwrap().modified().unwrap();
                    cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                    assert_eq!(fs::read_to_string(&path).unwrap(), source);
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                }
            }
        }
    }
    let source = "def f a,\n  b # rubocop:disable Style/MethodDefParentheses\nend\n";
    fs::write(&path, source).unwrap();
    cli(dir.path(), &["--only", METHOD, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
}
#[test]
fn stdin_is_report_only_and_fix_is_rejected() {
    use std::{io::Write, process::Stdio};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, _) in CASES {
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "lib/a.rb"])
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
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        cli(
            dir.path(),
            &["--only", cop, "--fix", "--stdin", "lib/a.rb"],
            2,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn bytes_bom_crlf_legacy_encoding_data_and_line_observations() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (before, after) in [
        ("\u{feff}# encoding: UTF-8\r\ndef f arg = \"café\"; [arg, __LINE__]; end\r\np __LINE__\r\n__END__\r\nrawÿ\r\n".as_bytes().to_vec(), "\u{feff}# encoding: UTF-8\r\ndef f(arg = \"café\"); [arg, __LINE__]; end\r\np __LINE__\r\n__END__\r\nrawÿ\r\n".as_bytes().to_vec()),
        (b"# encoding: ISO-8859-1\r\ndef f arg; [arg, \"caf\xe9\", __LINE__]; end\r\n__END__\r\nraw\xff\r\n".to_vec(), b"# encoding: ISO-8859-1\r\ndef f(arg); [arg, \"caf\xe9\", __LINE__]; end\r\n__END__\r\nraw\xff\r\n".to_vec()),
        (b"# encoding: ISO-8859-1\r\ndef f = true and effect\r\n__END__\r\nraw\xff\r\n".to_vec(), b"# encoding: ISO-8859-1\r\ndef f\r\n  true\r\nend and effect\r\n__END__\r\nraw\xff\r\n".to_vec()),
    ] {
        fs::write(&path, before).unwrap();
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), after);
    }
}
#[test]
fn uncorrectable_clean_and_error_files_retain_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, status) in [
        (b"def f(arg); end\ndef g = true\n".as_slice(), 0),
        (b"def f = __LINE__ if condition\n", 1),
        (b"def f = true if condition\np __LINE__\n", 1),
        (b"def f = <<~TEXT and effect\n  payload\nTEXT\n", 1),
        (b"def f = true and effect # rustocop:disable Layout\n", 1),
        (b"# encoding: ISO-8859-1\ndef f arg = \"caf\xe9\"; end\n", 1),
        (
            b"# encoding: ISO-8859-1\ndef f = \"caf\xe9\" and effect\n",
            1,
        ),
        (b"def f arg = (\n", 2),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn uncertain_offense_does_not_block_independent_definition_edits() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (before, after) in [
        (
            b"def f arg; arg; end\ndef g = true if condition\np __LINE__\n".as_slice(),
            b"def f(arg); arg; end\ndef g = true if condition\np __LINE__\n".as_slice(),
        ),
        (
            b"# encoding: ISO-8859-1\ndef f arg = \"caf\xe9\"; end\ndef g arg; arg; end\n",
            b"# encoding: ISO-8859-1\ndef f arg = \"caf\xe9\"; end\ndef g(arg); arg; end\n",
        ),
        (
            b"def f arg; arg; end\ndef g = true if condition\np error.backtrace_locations\n",
            b"def f(arg); arg; end\ndef g = true if condition\np error.backtrace_locations\n",
        ),
    ] {
        fs::write(&path, before).unwrap();
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 1);
        assert_eq!(fs::read(&path).unwrap(), after);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 1);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
fn ruby_available() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn runtime_case(source: &str, fixed: &str, harness: &str) {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let run = || {
        Command::new("ruby")
            .current_dir(dir.path())
            .args(["-e", harness, "lib/a.rb"])
            .output()
            .unwrap()
    };
    let before = run();
    assert!(before.status.success(), "{before:?}: {source}");
    cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), fixed);
    let after = run();
    assert!(after.status.success(), "{after:?}: {fixed}");
    assert_eq!(before.stdout, after.stdout, "{source}");
    assert_eq!(before.stderr, after.stderr, "{source}");
}
// The observer must be in the linted file, not only in the external harness.
// Compare real Ruby output and require --fix to retain the finding, bytes and
// mtime on both runs (not just produce parseable replacement source).
fn observer_no_write(source: &str) -> Vec<u8> {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let run = || {
        Command::new("ruby")
            .current_dir(dir.path())
            .args(["lib/a.rb"])
            .output()
            .unwrap()
    };
    let before = run();
    assert!(before.status.success(), "{before:?}: {source}");
    assert!(!before.stdout.is_empty(), "observer must execute: {source}");
    for _ in 0..2 {
        let out = cli(dir.path(), &["--only", ENDLESS, "--fix", "lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(ENDLESS));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let after = run();
        assert!(after.status.success(), "{after:?}: {source}");
        assert_eq!(before.stdout, after.stdout, "{source}");
        assert_eq!(before.stderr, after.stderr, "{source}");
    }
    before.stdout
}
#[test]
fn ruby_exception_metadata_observers_retain_offense_bytes_and_mtime() {
    if !ruby_available() {
        return;
    }
    for observer in [
        "p e.backtrace_locations.map(&:lineno)",
        "p e.backtrace",
        "puts e.full_message(highlight: false)",
        "p [e].flat_map(&:backtrace_locations).map(&:lineno)",
        "p [e].flat_map(&:backtrace)",
        "puts [e].map(&:full_message)",
    ] {
        let source = format!(
            "def f = raise(\"boom\") if true\nbegin\n  f\nrescue => e\n  {observer}\nend\n"
        );
        let output = observer_no_write(&source);
        if observer.contains("backtrace_locations") {
            assert_eq!(output, b"[1, 3]\n");
        } else {
            assert!(String::from_utf8_lossy(&output).contains("a.rb:1"));
        }
    }
}
#[test]
fn ruby_later_method_source_location_retain_offense_bytes_and_mtime() {
    if !ruby_available() {
        return;
    }
    let output = observer_no_write(
        "def f = true if true\ndef later; :later; end\np [method(:later).source_location.last, later]\n",
    );
    assert_eq!(output, b"[2, :later]\n");
}
#[test]
fn ruby_eval_trace_and_source_file_observers_retain_offense_bytes_and_mtime() {
    if !ruby_available() {
        return;
    }
    for source in [
        "def f = true if true\ndef later; :later; end\np eval('method(:later).source_location.last')\n",
        "def f = caller_locations.map(&:lineno) if true\np f\n",
        "def f = caller if true\np f\n",
        "events = []\ntrace = TracePoint.new(:line) { |t| events << t.lineno }\ntrace.enable\ndef f = 42 if true\np f\ntrace.disable\np events\n",
        "events = []\nset_trace_func(proc { |event, file, line, *rest| events << line if event == 'line' })\ndef f = 42 if true\nf\nset_trace_func(nil)\np events\n",
        "def f = true if true\np File.readlines(__FILE__)\n",
    ] {
        observer_no_write(source);
    }
}
#[test]
fn ruby_parameter_binding_defaults_evaluation_visibility_and_literals() {
    if !ruby_available() {
        return;
    }
    let source = r#"$events = []
$object = Object.new
def receiver; $events << :receiver; $object; end
def (receiver).value= value; @value = value; end
def (receiver).[] key; [key, @value]; end
class Fresh
  private def value a = ($events << :default; 4), *rest, key: 2, **kw, &block
    [a, rest, key, kw, block.call, __LINE__]
  end
  def destructure first, (a,b); [first, a,b]; end
  def text arg = <<~TEXT
    café #{__LINE__}
TEXT
    [arg, __LINE__]
  end
end
$object.value = 7
p [$object[:key], Fresh.new.send(:value, key: 3, extra: 5) { :block }, Fresh.new.send(:value, 8, 9) { :explicit }, Fresh.new.destructure(1, [2,3]), Fresh.new.text, Fresh.private_instance_methods(false).sort, $events, __LINE__]
"#;
    let fixed = source
        .replace("value= value;", "value=(value);")
        .replace("[] key;", "[](key);")
        .replace(
            "value a = ($events << :default; 4), *rest, key: 2, **kw, &block",
            "value(a = ($events << :default; 4), *rest, key: 2, **kw, &block)",
        )
        .replace("destructure first, (a,b);", "destructure(first, (a,b));")
        .replace("text arg = <<~TEXT\n", "text(arg = <<~TEXT)\n");
    runtime_case(source, &fixed, "load ARGV[0]");
}
#[test]
fn ruby_endless_outer_operators_definition_results_and_exception_timing() {
    if !ruby_available() {
        return;
    }
    for (operator, condition) in [
        ("and", "($events << :outside; :rhs)"),
        ("or", "($events << :outside; :rhs)"),
        ("if", "($events << :predicate; true)"),
        ("unless", "($events << :predicate; false)"),
        ("while", "($events << :predicate; ($n += 1) < 3)"),
        ("until", "($events << :predicate; ($n += 1) >= 3)"),
    ] {
        let source = format!(
            "$events = []; $n = 0\ndef subject(arg = ($events << :default; 7)) = ($events << [:body, arg]; arg) {operator} {condition}\n"
        );
        let fixed = source.replace(
            " = ($events << [:body, arg]; arg)",
            "\n  ($events << [:body, arg]; arg)\nend",
        );
        // eval result distinguishes `and`/`or` from the method body; events
        // distinguish declaration-time predicates from later default/body calls.
        runtime_case(
            &source,
            &fixed,
            "result = eval(File.read(ARGV[0]), TOPLEVEL_BINDING, ARGV[0]); p [result, $events.dup, $n]; p [subject, subject(9), $events, $n]",
        );
    }
    for receiver in ["", "self."] {
        let source = format!(
            "$events = []\ndef {receiver}subject = ($events << :body; raise 'later') and ($events << :outside; raise 'now') rescue ($events << :caught_now; :recovered)\n"
        );
        let fixed = source.replace(
            " = ($events << :body; raise 'later')",
            "\n  ($events << :body; raise 'later')\nend",
        );
        runtime_case(
            &source,
            &fixed,
            "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING, ARGV[0]); p $events.dup; begin; subject; rescue => e; p [e.message, $events]; end",
        );
    }
    let source = "$events = []\ndef subject = (raise('body') rescue ($events << :inside; :fallback)) and ($events << :outside; :rhs)\n";
    let fixed = source.replace(
        " = (raise('body') rescue ($events << :inside; :fallback))",
        "\n  (raise('body') rescue ($events << :inside; :fallback))\nend",
    );
    runtime_case(
        source,
        &fixed,
        "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING, ARGV[0]); p $events.dup; p [subject, $events]",
    );
}
#[test]
fn ruby_conditional_definition_absence_and_receiver_evaluation_order() {
    if !ruby_available() {
        return;
    }
    for (operator, value) in [
        ("if", "false"),
        ("unless", "true"),
        ("while", "false"),
        ("until", "true"),
        ("if", "true"),
        ("unless", "false"),
    ] {
        let source = format!(
            "$events = []; $object = Object.new\ndef receiver; $events << :receiver; $object; end\ndef (receiver).subject = ($events << :body; 42) {operator} ($events << :predicate; {value})\n"
        );
        let fixed = source.replace(
            " = ($events << :body; 42)",
            "\n  ($events << :body; 42)\nend",
        );
        runtime_case(
            &source,
            &fixed,
            "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING, ARGV[0]); p [$events.dup, $object.respond_to?(:subject)]; p [$object.subject, $events] if $object.respond_to?(:subject)",
        );
    }
}
#[test]
fn style_layout_and_forwarding_peer_stabilize_within_four_explicit_passes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def self::f a = 'x';  \n  return (a);\nend\ndef g = 'y' if condition\n";
    fs::write(&path, source).unwrap();
    let only = format!(
        "{ONLY},Style/ColonMethodDefinition,Style/DefWithParentheses,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout"
    );
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", &only, "--fix", "--fix-passes", "1", "lib/a.rb"])
            .output()
            .unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
    }
    let expected = "def self.f(a = \"x\")\n  a\nend\ndef g\n  \"y\"\nend if condition\n";
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(
        dir.path(),
        &["--only", &only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        0,
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    // The smaller parentheses replacement wins the overlap on pass one.
    // Forwarding's atomic signature+uses replacement follows on pass two;
    // neither pass may leave a named use with an anonymous signature.
    fs::write(
        &path,
        "def f *args, **kwargs, &block\n  g(*args, **kwargs, &block)\nend\n",
    )
    .unwrap();
    let args = [
        "--only",
        "Style/MethodDefParentheses,Style/ArgumentsForwarding",
        "--fix",
        "--fix-passes",
        "1",
        "lib/a.rb",
    ];
    let out = cli(dir.path(), &args, 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains("Style/ArgumentsForwarding"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def f(*args, **kwargs, &block)\n  g(*args, **kwargs, &block)\nend\n"
    );
    cli(dir.path(), &args, 0);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def f(...)\n  g(...)\nend\n"
    );
}
