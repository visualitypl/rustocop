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
fn directives_exclusions_errors_and_withheld_fixes_do_not_write() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, _) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                let source = format!("# {prefix}:disable {target}\n{before}");
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 0);
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
        for config in [
            "exclude = [\"x.rb\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 0);
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        fs::write(&path, "def f(\n").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 2);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    for (cop, source) in [
        (CASES[0].0, "def f(a); super(\n a\n); end"),
        (CASES[2].0, "def f(a: b, b:); [a,b]; end"),
        (CASES[2].0, "def f(a: 1, b:); super; end"),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
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
fn runtime_super_channels_rebinding_blocks_defaults_post_and_exceptions() {
    if !ruby_available() {
        eprintln!("Ruby unavailable: runtime test skipped");
        return;
    }
    let source = r#"$events = []
class Parent
  def f(*args, **kwargs, &block)
    $events << [args, kwargs.to_a, block && block.call(:parent)]
    raise "expected" if args == [:raise]
    [args, kwargs]
  end
end
class Child < Parent
  def f(first = ($events << :default; 10), *items, last, a: 1, b:, **settings, &callback)
    first += 1
    items = items.reverse
    a += 2
    $events << :before
    super(first, *items, last, a: a, b: b, **settings, &callback)
  rescue RuntimeError => e
    $events << e.message
  ensure
    $events << :ensure
  end
end
p Child.new.f(2, 3, 4, 5, b: 6, extra: 7) { |x| [:block, x] }
p Child.new.f(5, b: 6)
p $events
p Child.private_instance_methods(false)
"#;
    // Implicit super inserts rest keywords first: preserve the offense only.
    observed_fix(CASES[0].0, source, source, 1);
    let positional = source
        .replace(", a: 1, b:, **settings", "")
        .replace("    a += 2\n", "")
        .replace(", a: a, b: b, **settings", "")
        .replace(", b: 6, extra: 7", "")
        .replace(", b: 6", "");
    let after = positional.replace("super(first, *items, last, &callback)", "super");
    observed_fix(CASES[0].0, &positional, &after, 0);
    let source = "class P; def f(*a, **k); [a, k.to_a, yield]; end; end\nclass C < P; def f(a, &blk); super(a) { :replacement }; end; end\np C.new.f({}) { :original }\n";
    observed_fix(CASES[0].0, source, &source.replace("super(a)", "super"), 0);
    // A reassigned named block must stay explicit, while a shadowed local is harmless.
    let source = "class P; def f; yield; end; end\nclass C < P; def f(&blk); blk = proc { :replacement }; super(&blk); end; end\np C.new.f { :original }\n";
    observed_fix(CASES[0].0, source, source, 0);
    let source = "class P; def f; yield; end; end\nclass C < P; def f(&blk); [1].each { |blk| blk = nil }; ->(blk) { blk = nil }; super(&blk); end; end\np C.new.f { :original }\n";
    observed_fix(
        CASES[0].0,
        source,
        &source.replace("super(&blk)", "super"),
        0,
    );
    let source = "class P; def f(a); a; end; end\nclass C < P; def build; define_singleton_method(:f) { |a| super(a) }; end; end\nc = C.new; c.build; p c.f(3)\n";
    observed_fix(CASES[0].0, source, source, 0);
    let source = "class P; def f(*a); raise ArgumentError, a.inspect; end; end\nclass C < P; def f(a); super(a); rescue ArgumentError => e; p [e.class, e.message, __LINE__]; ensure; p :ensure; end; end\nC.new.f(:raise)\n";
    observed_fix(CASES[0].0, source, &source.replace("super(a)", "super"), 0);
}
#[test]
fn runtime_super_expression_continuations_keep_their_parse() {
    if !ruby_available() {
        eprintln!("Ruby unavailable: runtime test skipped");
        return;
    }
    for (expression, input, expected, naive_expected) in [
        ("super(a) +1", "10", "11\n", "1\n"),
        ("super(a) -1", "10", "9\n", "-1\n"),
        ("super(a) [0]", "[10]", "10\n", "[0]\n"),
    ] {
        let source = format!(
            "class P; def f(a); a; end; end\nclass C < P; def f(a); {expression}; end; end\np C.new.f({input})\n"
        );
        let dir = project();
        let path = dir.path().join("naive.rb");
        fs::write(&path, source.replace("super(a)", "super")).unwrap();
        let naive = ruby(&path);
        assert!(naive.status.success(), "{naive:?}");
        assert_eq!(naive.stdout, naive_expected.as_bytes());
        assert_eq!(
            observed_fix(CASES[0].0, &source, &source, 1),
            expected.as_bytes()
        );
    }
}
// Controlled sources only: never execute external corpus code.
const SUPER_SURROUNDINGS: [(&str, &str); 8] = [
    ("super(a)and true", "true\n"),
    ("super(a)if true", "10\n"),
    ("super(a)unless false", "10\n"),
    ("super(a)rescue 1", "10\n"),
    ("super(a)or 1", "10\n"),
    ("super(a) \\\n +1", "11\n"),
    ("super(a) <<value", "20\n"),
    ("super(a)do;end", "10\n"),
];
fn surrounding_source(receiver: &str, expression: &str, newline: &str, sibling: bool) -> String {
    let target = if receiver.is_empty() { "C.new" } else { "C" };
    let sibling = if sibling {
        format!(" def {receiver}safe(a); super(a); end\n")
    } else {
        String::new()
    };
    format!(
        "class P\n def {receiver}f(a); a; end\n def {receiver}safe(a); a; end\nend\nclass C < P\n def {receiver}f(a)\n  value=1\n  {expression}\n end\n{sibling}end\np {target}.f(10)\n"
    ).replace('\n', newline)
}
#[test]
fn unsafe_super_surroundings_keep_bytes_and_do_not_block_safe_siblings() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for receiver in ["", "self."] {
        for newline in ["\n", "\r\n"] {
            for (expression, _) in SUPER_SURROUNDINGS {
                for sibling in [false, true] {
                    let source = surrounding_source(receiver, expression, newline, sibling);
                    fs::write(&path, &source).unwrap();
                    let modified = fs::metadata(&path).unwrap().modified().unwrap();
                    let output = cli(dir.path(), &["--only", CASES[0].0, "x.rb"], 1);
                    assert!(String::from_utf8_lossy(&output.stdout).contains(CASES[0].0));
                    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                    cli(dir.path(), &["--only", CASES[0].0, "--fix", "x.rb"], 1);
                    let expected = source.replace("safe(a); super(a)", "safe(a); super");
                    assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
                    if !sibling {
                        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                    }
                    let modified = fs::metadata(&path).unwrap().modified().unwrap();
                    cli(dir.path(), &["--only", CASES[0].0, "--fix", "x.rb"], 1);
                    assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                }
            }
        }
    }
}
#[test]
fn runtime_super_surroundings_preserve_control_continuations_and_inline_blocks() {
    if !ruby_available() {
        eprintln!("Ruby unavailable: runtime test skipped");
        return;
    }
    for receiver in ["", "self."] {
        for newline in ["\n", "\r\n"] {
            for (expression, output) in SUPER_SURROUNDINGS {
                let source = surrounding_source(receiver, expression, newline, true);
                let expected = source.replace("safe(a); super(a)", "safe(a); super");
                assert_eq!(
                    observed_fix(CASES[0].0, &source, &expected, 1),
                    output.as_bytes()
                );
                // Witness why successful parsing alone was insufficient: tight
                // keywords become method names, continuations change the value,
                // and `<<value` can instead demand a heredoc terminator.
                let dir = project();
                let path = dir.path().join("naive.rb");
                fs::write(&path, source.replace("super(a)", "super")).unwrap();
                let naive = ruby(&path);
                if expression.contains('\\') {
                    assert!(naive.status.success(), "{naive:?}");
                    assert_eq!(naive.stdout, b"1\n");
                } else {
                    assert!(!naive.status.success(), "{expression}: {naive:?}");
                }
            }
            // A real, delimited inline block ends argument parsing, unlike a
            // tight `do` token at the edit boundary. Do not withhold safe edits
            // merely because the main AST has a modifier/operator above it.
            for (suffix, output) in [
                ("and true", "true\n"),
                ("if true", "10\n"),
                ("unless false", "10\n"),
                ("rescue 1", "10\n"),
                ("or 1", "10\n"),
                (" \\\n +1", "11\n"),
                (" <<value", "20\n"),
            ] {
                let expression = format!("super(a){{ :inline }}{suffix}");
                let source = surrounding_source(receiver, &expression, newline, true);
                assert_eq!(
                    observed_fix(CASES[0].0, &source, &source.replace("super(a)", "super"), 0),
                    output.as_bytes()
                );
            }
        }
    }
}
#[test]
fn runtime_super_binding_mutation_keeps_the_explicit_block() {
    if !ruby_available() {
        eprintln!("Ruby unavailable: runtime test skipped");
        return;
    }
    for mutation in [
        "binding.local_variable_set(:blk, proc { :replacement })",
        "eval('blk = proc { :replacement }')",
        "b = binding; [1].each { b.local_variable_set(:blk, proc { :replacement }) }",
        "send(:binding).local_variable_set(:blk, proc { :replacement })",
    ] {
        let source = format!(
            "class P; def f; yield; end; end\nclass C < P; def f(&blk); {mutation}; super(&blk); end; end\np C.new.f {{ :original }}\n"
        );
        let dir = project();
        let path = dir.path().join("naive.rb");
        fs::write(&path, source.replace("super(&blk)", "super")).unwrap();
        let naive = ruby(&path);
        assert!(naive.status.success(), "{naive:?}");
        assert_eq!(naive.stdout, b":original\n");
        assert_eq!(
            observed_fix(CASES[0].0, &source, &source, 1),
            b":replacement\n"
        );
    }
}
#[test]
fn runtime_super_keyword_order_forwarding_and_empty_hash_channels() {
    if !ruby_available() {
        eprintln!("Ruby unavailable: runtime test skipped");
        return;
    }
    for (params, args, input, safe) in [
        ("a:, b:", "a: a, b: b", "a: 1, b: 2", true),
        ("a: 1, b: 2", "a: a, b: b", "", true),
        ("b:, a: 1", "b: b, a: a", "b: 2", true),
        ("a: 1, b:", "a: a, b: b", "b: 2", false),
        ("a:, **settings", "a: a, **settings", "a: 1, b: 2", false),
        ("**settings", "**settings", "a: 1, b: 2", true),
        ("**settings", "**settings", "**{}", true),
        ("a = {}", "a", "", true),
        ("a", "a", "{}", true),
        ("*items", "*items", "{}, 1, 2", true),
        ("&callback", "&callback", "", true),
        ("...", "...", "{}, key: 1", true),
    ] {
        let source = format!(
            "class P; def f(*a, **k, &b); [a, k.to_a, b && b.call]; end; end\nclass C < P; def f({params}); super({args}); end; end\np C.new.f({input}) {{ :block }}\n"
        );
        let naive = source.replace(&format!("super({args})"), "super");
        observed_fix(
            CASES[0].0,
            &source,
            if safe { &naive } else { &source },
            i32::from(!safe),
        );
        if !safe {
            let dir = project();
            let path = dir.path().join("x.rb");
            fs::write(&path, &source).unwrap();
            let before = ruby(&path);
            fs::write(&path, naive).unwrap();
            let after = ruby(&path);
            assert!(before.status.success() && after.status.success());
            assert_ne!(before.stdout, after.stdout, "{params}");
        }
    }
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
fn runtime_data_position_guards_and_naive_edit_witnesses() {
    if !ruby_available() {
        eprintln!("Ruby unavailable: runtime test skipped");
        return;
    }
    for (cop, declaration, naive) in [
        (
            CASES[0].0,
            "def f(a); super(a); end",
            "def f(a); super; end",
        ),
        (
            CASES[1].0,
            "def f(a); super a; end",
            "def f(a); super(a); end",
        ),
        (CASES[2].0, "def f(a: 1,   b:); end", "def f(b:, a: 1); end"),
    ] {
        let source =
            format!("{declaration}\np [DATA.pos, DATA.lineno, DATA.read]\n__END__\npayload\n");
        let before = observed_fix(cop, &source, &source, 1);
        let dir = project();
        let path = dir.path().join("x.rb");
        fs::write(&path, source.replace(declaration, naive)).unwrap();
        let after = ruby(&path);
        assert!(after.status.success());
        assert_ne!(before, after.stdout);
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
