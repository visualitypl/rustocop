//! real pipeline controls, exact bytes and controlled Ruby semantics.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const CASES: [(&str, &str, &str); 3] = [
    (
        "Style/SuperArguments",
        "def f(a)\n  super(a)\nend\n",
        "def f(a)\n  super\nend\n",
    ),
    (
        "Style/SuperWithArgsParentheses",
        "def f(a)\n  super 1\nend\n",
        "def f(a)\n  super(1)\nend\n",
    ),
    (
        "Style/KeywordParametersOrder",
        "def f(a: 1, b:)\n  [a, b]\nend\n",
        "def f(b:, a: 1)\n  [a, b]\nend\n",
    ),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
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
fn defaults_selectors_config_and_no_write() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, after) in CASES {
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let output = cli(dir.path(), &["x.rb"], 1);
        assert!(String::from_utf8_lossy(&output.stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "x.rb"], 1);
        cli(
            dir.path(),
            &["--only", cop, "--except", cop, "--fix", "x.rb"],
            0,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let output = cli(dir.path(), &["x.rb"], 1);
            assert!(!String::from_utf8_lossy(&output.stdout).contains(cop));
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 0);
            assert_eq!(fs::read_to_string(&path).unwrap(), after);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 0);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            fs::write(&path, before).unwrap();
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&cli(dir.path(), &["x.rb"], 1).stdout).contains(cop));
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn byte_boundaries_encoding_bom_crlf_line_and_payload() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, after) in CASES {
        for header in [
            b"\xef\xbb\xbf# encoding: UTF-8\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\n# caf\xe9\r\n",
        ] {
            let suffix = b"p __LINE__\r\n__END__\r\nraw\xff\r\n";
            let before = [header, before.replace('\n', "\r\n").as_bytes(), suffix].concat();
            let after = [header, after.replace('\n', "\r\n").as_bytes(), suffix].concat();
            fs::write(&path, before).unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 0);
            assert_eq!(fs::read(&path).unwrap(), after);
        }
    }
    let before =
        b"# encoding: ISO-8859-1\r\ndef f; super \"caf\xe9\"; end\r\ndef g; super 1; end\r\n";
    let after =
        b"# encoding: ISO-8859-1\r\ndef f; super \"caf\xe9\"; end\r\ndef g; super(1); end\r\n";
    fs::write(&path, before).unwrap();
    cli(dir.path(), &["--only", CASES[1].0, "--fix", "x.rb"], 1);
    assert_eq!(fs::read(&path).unwrap(), after);
}
fn ruby_available() -> bool {
    Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
fn ruby(path: &Path) -> Output {
    Command::new("ruby").arg("-W0").arg(path).output().unwrap()
}
fn observed_fix(cop: &str, source: &str, expected: &str, status: i32) -> Vec<u8> {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(&path);
    assert!(before.status.success(), "{before:?}\n{source}");
    cli(dir.path(), &["--only", cop, "--fix", "x.rb"], status);
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    let after = ruby(&path);
    assert_eq!(before.status.code(), after.status.code());
    assert_eq!(before.stdout, after.stdout, "{cop}\n{source}");
    assert_eq!(before.stderr, after.stderr, "{cop}\n{source}");
    before.stdout
}
#[test]
fn runtime_parentheses_preserve_evaluation_count_control_and_literal_block_ownership() {
    if !ruby_available() {
        eprintln!("Ruby unavailable: runtime test skipped");
        return;
    }
    let source = r#"$events = []
class P
  def f(*args, **kwargs)
    $events << [args, kwargs.to_a, block_given? && yield(:parent)]
    :result
  end
end
class C < P
  def value(x)
    $events << x
    x
  end
  def f
    super value(1), value(2), key: value(3) do |x|
      $events << x
      break :broken
    end
  end
end
p C.new.f
p $events
"#;
    observed_fix(
        CASES[1].0,
        source,
        &source.replace(
            "super value(1), value(2), key: value(3)",
            "super(value(1), value(2), key: value(3))",
        ),
        0,
    );
    let source = "class P; def f(*a); p [a, block_given?]; end; end\nclass C < P; def inner; [yield, __LINE__]; end; def f; super inner { :inner }; end; end\nC.new.f\n";
    observed_fix(
        CASES[1].0,
        source,
        &source.replace("super inner { :inner }", "super(inner { :inner })"),
        0,
    );
    let source = "class P; def f(x); p [x, __LINE__]; end; end\nclass C < P; def f; super <<~TEXT\n  café #{__LINE__}\nTEXT\nend; end\nC.new.f\n__END__\npayload\n";
    observed_fix(
        CASES[1].0,
        source,
        &source.replace("super <<~TEXT", "super(<<~TEXT)"),
        0,
    );
}
#[test]
fn runtime_keyword_binding_local_metadata_and_implicit_super_hash_order() {
    if !ruby_available() {
        eprintln!("Ruby unavailable: runtime test skipped");
        return;
    }
    let source = "def f(a: 1, b:, c: 2); [a,b,c,__LINE__]; end\np f(b: 3)\np f(a: 4,b: 5,c: 6)\n";
    observed_fix(
        CASES[2].0,
        source,
        &source.replace("a: 1, b:", "b:, a: 1"),
        0,
    );
    for source in [
        "def b; :method; end\ndef f(a: b, b:); [a,b]; end\np f(b: :local)\n",
        "def f(a: 1, b:); local_variables; end\np f(b: 2)\n",
        "class P; def f(**k); k.keys; end; end\nclass C < P; def f(a: 1, b:); super; end; end\np C.new.f(b: 2)\n",
        "def f(a: 1, b:); end\np method(:f).parameters\n",
        "$events = []\ndef f(a: ($events << :default; 1), b:); [a,b]; end\np f(b: 2)\np $events\n",
    ] {
        observed_fix(CASES[2].0, source, source, 1);
    }
    // Naive reordering changes lexical binding. Ruby 3.2 normalizes required
    // and optional slots for local reflection/zsuper regardless of declaration
    // order; we conservatively withhold those source-level hazards at 3.4 too.
    for (source, naive) in [
        (
            "def b; :method; end; def f(a: b, b:); [a,b]; end; p f(b: :local)",
            "def b; :method; end; def f(b:, a: b); [a,b]; end; p f(b: :local)",
        ),
        (
            "def f(a: (b rescue :missing), b:); [a,b]; end; p f(b: :local)",
            "def f(b:, a: (b rescue :missing)); [a,b]; end; p f(b: :local)",
        ),
    ] {
        let dir = project();
        let path = dir.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let before = ruby(&path);
        fs::write(&path, naive).unwrap();
        let after = ruby(&path);
        assert!(before.status.success() && after.status.success());
        assert_ne!(before.stdout, after.stdout);
    }
}
#[test]
fn mixed_single_pass_fixes_stabilize_within_six_explicit_passes() {
    let dir = project();
    let path = dir.path().join("x.rb");
    let source = "class Child < Parent\n  def f(*args, **kwargs, &block)\n    super *args, **kwargs, &block\n  end\n\n  def g(optional: 1, required:)\n    super(required)\n  end\nend\n";
    for only in [
        "Style/SuperArguments,Style/SuperWithArgsParentheses,Style/KeywordParametersOrder,Style/ArgumentsForwarding,Style/EndlessMethod,Style/RedundantParentheses,Style/MethodDefParentheses,Layout",
        "Style,Layout",
    ] {
        fs::write(&path, source).unwrap();
        let mut stabilized = false;
        for _ in 0..6 {
            let before = fs::read(&path).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", only, "--fix", "x.rb"])
                .output()
                .unwrap();
            assert_ne!(output.status.code(), Some(2), "{output:?}");
            let after = fs::read(&path).unwrap();
            if before == after {
                stabilized = true;
                break;
            }
        }
        assert!(stabilized, "{}", fs::read_to_string(&path).unwrap());
        let after = fs::read_to_string(&path).unwrap();
        assert!(!after.contains("super *"));
        assert!(after.contains("required:, optional: 1"));
    }
}
