//! real CLI policy, byte/no-write contracts and controlled Ruby observations.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const CASES: [(&str, &str, &str); 3] = [
    (
        "Style/MethodCallWithoutArgsParentheses",
        "object.test()\n",
        "object.test\n",
    ),
    ("Style/OperatorMethodCall", "foo.+ bar\n", "foo + bar\n"),
    (
        "Style/NestedParenthesizedCalls",
        "puts(compute something)\n",
        "puts(compute(something))\n",
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
fn default_on_selectors_config_and_no_write_contracts() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after) in CASES {
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let out = cli(dir.path(), &["lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
        cli(
            dir.path(),
            &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
            0,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            cli(dir.path(), &["--fix", "lib/a.rb"], 0);
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
            assert_eq!(fs::read_to_string(&path).unwrap(), after);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            fs::write(&path, before).unwrap();
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn directives_excludes_and_stdin_do_not_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, _) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{before}"),
                    format!("{} # {prefix}:disable {target}\n", before.trim()),
                    format!("# {prefix}:disable-next {target}\n{before}"),
                ] {
                    fs::write(&path, &source).unwrap();
                    let modified = fs::metadata(&path).unwrap().modified().unwrap();
                    cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                    assert_eq!(fs::read_to_string(&path).unwrap(), source);
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                }
            }
        }
        for config in [
            "exclude = [\"lib/a.rb\"]\n".to_string(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
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
    }
}

fn ruby(source: &str) -> Option<Output> {
    let mut child = match Command::new("ruby")
        .args(["--disable-gems", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => panic!("ruby: {e}"),
    };
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    Some(child.wait_with_output().unwrap())
}
fn runtime_equivalent(cop: &str, source: &str, status: i32) {
    let Some(before) = ruby(source) else { return };
    assert!(before.status.success(), "{before:?}");
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    cli(
        dir.path(),
        &["--only", cop, "--fix", "--fix-passes", "1", "lib/a.rb"],
        status,
    );
    let corrected = fs::read_to_string(&path).unwrap();
    assert_ne!(corrected, source);
    let after = ruby(&corrected).unwrap();
    assert!(after.status.success(), "{after:?}\n{corrected}");
    assert_eq!(after.stdout, before.stdout, "{corrected}");
    assert_eq!(after.stderr, before.stderr, "{corrected}");
}

#[test]
fn empty_calls_preserve_local_method_binding_scope_safe_navigation_and_evaluation() {
    runtime_equivalent(
        CASES[0].0,
        r#"$events = []
def event; $events << :method; 10; end
event = 99
def receiver; $events << :receiver; self; end
def self.value; $events << :value; 3; end
p [receiver().value(), event(), event, receiver()&.value(), __LINE__]
p $events
"#,
        0,
    );
}

#[test]
fn adjacent_keywords_preserve_method_names_control_flow_and_block_binding() {
    runtime_equivalent(
        CASES[0].0,
        r#"def foo; puts :foo; true; end
foo()and true
foo()or false
foo()if true
foo()unless false
foo()while false
foo()until true
foo()rescue false
foo()do; puts :unused; end
begin; foo()end
if foo()then; puts :then; end
p foo()
"#,
        0,
    );
}

#[test]
fn unicode_uppercase_calls_preserve_method_versus_constant_binding() {
    for constant in ["", "É = :constant\n"] {
        runtime_equivalent(
            CASES[0].0,
            &format!("{constant}def É; :method; end\np É()\ndef plain; :plain; end\np plain()\n"),
            // RuboCop's reparse check drops `É()`: bare, it reads as a constant.
            0,
        );
    }
}

#[test]
fn nested_calls_preserve_argument_block_heredoc_line_and_throw_binding() {
    runtime_equivalent(
        CASES[2].0,
        r#"$events = []
def value; $events << :value; block_given? ? yield : 2; end
def inner(*args); $events << [:inner, args]; block_given? ? yield : args; end
def outer(arg); $events << [:outer, arg]; arg; end
p outer(inner value)
def with_inner_block(&block); outer(inner value, &block); end
p with_inner_block { $events << :inner_block; 7 }
p outer(inner value) { $events << :outer_block; 6 }
p outer(inner value { $events << :value_block; 8 })
p outer(inner <<~TEXT)
  hello #{__LINE__}
TEXT
p catch(:done) { outer(inner throw :done, 9) }
p $events
"#,
        1,
    );
}

#[test]
fn heredoc_scope_lead_counterexample_keeps_two_and_file_identity() {
    let source = "class C; def self.foo;2;end; foo=1; X=<<~TXT\n#{foo()}\nTXT\nend; p C::X\n";
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir.path(), &["--only", CASES[0].0, "lib/a.rb"], 0);
    assert!(!String::from_utf8_lossy(&out.stdout).contains(CASES[0].0));
    cli(dir.path(), &["--only", CASES[0].0, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    if let Some(before) = ruby(source) {
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, b"\"2\\n\"\n");
        let after = ruby(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after.status.code(), before.status.code());
        assert_eq!(after.stdout, before.stdout);
        assert_eq!(after.stderr, before.stderr);
    }
}

#[test]
fn heredoc_scope_headers_and_def_receivers_keep_outer_binding_and_evaluation_order() {
    for (header, target) in [
        ("class C < choose(<<~TXT)", "Object"),
        ("class choose(<<~TXT)::C", "Module.new"),
        ("module choose(<<~TXT)::M", "Module.new"),
        ("class << choose(<<~TXT)", "Object.new"),
        ("def (choose(<<~TXT)).method", "Object.new"),
        ("def (choose(<<~TXT)).method(foo)", "Object.new"),
    ] {
        for outer_local in [true, false] {
            for inline_end in [true, false] {
                let local = if outer_local { "foo=:local" } else { "" };
                let end = if inline_end { "; foo=:inner; end" } else { "" };
                let later_end = if inline_end { "" } else { "foo=:inner; end\n" };
                let source = format!(
                    r#"$events = []
$values = []
$target = {target}
def foo; $events << :foo; "method"; end
def choose(value); $events << :choose; $values << value; $target; end
def safe; $events << :safe; end
{local}
{header}{end}
#{{foo()}} #{{__LINE__}}
TXT
{later_end}safe()
puts $values
p $events
"#
                );
                let dir = project();
                let path = dir.path().join("lib/a.rb");
                fs::write(&path, &source).unwrap();
                cli(dir.path(), &["--only", CASES[0].0, "--fix", "lib/a.rb"], 0);
                let corrected = fs::read_to_string(&path).unwrap();
                let expected = source.replace("safe()", "safe");
                let expected = if outer_local {
                    expected
                } else {
                    expected.replace("foo()", "foo")
                };
                assert_eq!(corrected, expected, "{source}");
                if let Some(before) = ruby(&source) {
                    assert!(before.status.success(), "{before:?}\n{source}");
                    assert_eq!(before.stdout, b"method 9\n[:foo, :choose, :safe]\n");
                    let after = ruby(&corrected).unwrap();
                    assert_eq!(after.status.code(), before.status.code());
                    assert_eq!(after.stdout, before.stdout);
                    assert_eq!(after.stderr, before.stderr);
                }
            }
        }
    }
}

#[test]
fn heredoc_scope_corrections_preserve_headers_directives_excludes_and_raw_bytes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let body = "class C; foo=1; X=<<~TXT\n#{foo()} #{safe()} #{__LINE__}\nTXT\nend\n";
    for prefix in [
        "\u{feff}# encoding: UTF-8\r\n# café\r\n".as_bytes(),
        b"#!/usr/bin/env ruby\r\n# encoding: ISO-8859-1\r\n# caf\xe9\r\n".as_slice(),
    ] {
        let suffix = b"p __LINE__\r\n__END__\r\nraw\xff\r\n";
        let body = body.replace('\n', "\r\n");
        let source = [prefix, body.as_bytes(), suffix].concat();
        let expected = [prefix, body.replace("safe()", "safe").as_bytes(), suffix].concat();
        fs::write(&path, &source).unwrap();
        cli(dir.path(), &["--only", CASES[0].0, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), crate::support::lf::lf(&expected));
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", CASES[0].0, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for directive in ["rubocop", "rustocop"] {
            for (before, after) in [
                (
                    format!("# {directive}:disable-next {}\r\n", CASES[0].0),
                    String::new(),
                ),
                (
                    format!("# {directive}:disable {}\r\n", CASES[0].0),
                    format!("# {directive}:enable {}\r\n", CASES[0].0),
                ),
            ] {
                let source = [
                    prefix,
                    before.as_bytes(),
                    body.as_bytes(),
                    after.as_bytes(),
                    suffix,
                ]
                .concat();
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                cli(dir.path(), &["--only", CASES[0].0, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
        for config in [
            "exclude = [\"lib/a.rb\"]\n".to_string(),
            format!("[cops.\"{}\"]\nexclude = [\"lib/a.rb\"]\n", CASES[0].0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, &source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", CASES[0].0, "--fix", "lib/a.rb"], 0);
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn all_three_and_style_layout_composition_stabilize_without_cycles() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def f\n  return sink(inner 1); object::value()\nend\nfoo.+(bar).to_s\n";
    fs::write(&path, source).unwrap();
    let only = "Style/MethodCallWithoutArgsParentheses,Style/OperatorMethodCall,Style/NestedParenthesizedCalls,Style/MethodCallWithArgsParentheses,Style/ColonMethodCall,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/SignalException,Style/RedundantParentheses,Layout";
    let mut seen = vec![source.to_string()];
    let mut stable = false;
    for _ in 0..4 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert!(matches!(output.status.code(), Some(0 | 1)), "{output:?}");
        let next = fs::read_to_string(&path).unwrap();
        if next == *seen.last().unwrap() {
            stable = true;
            break;
        }
        assert!(!seen.contains(&next), "cycle: {seen:?}");
        seen.push(next);
    }
    assert!(stable, "{seen:?}");
    let after = seen.last().unwrap();
    assert!(after.contains("sink(inner(1))"), "{after}");
    assert!(after.contains("object.value"), "{after}");
    assert!(after.contains("(foo + bar).to_s"), "{after}");
}
