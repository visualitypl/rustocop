//! real CLI, conservative corrections, and controlled Ruby observations.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/TrivialAccessors";
const BEFORE: &str = "class Foo\n  def foo; @foo; end\n  def foo=(val); @foo=val; end\nend\n";
const AFTER: &str = "class Foo\n  attr_reader :foo\n  attr_writer :foo\nend\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(matches!(o.status.code(), Some(0 | 1)), "{args:?}: {o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    o
}
fn count(o: &Output, n: usize) {
    let text = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        text.lines().filter(|l| l.contains(COP)).count(),
        n,
        "{text}"
    );
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str]) -> Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(dir, args);
    assert_eq!(fs::read(&path).unwrap(), source, "{args:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    o
}
#[test]
fn default_selectors_config_and_no_write() {
    let d = project();
    for (config, args, n) in [
        (String::new(), vec!["a.rb"], 2),
        (String::new(), vec!["--only", COP, "a.rb"], 2),
        (String::new(), vec!["--except", COP, "a.rb"], 0),
        (
            String::new(),
            vec!["--only", COP, "--except", "Style", "--fix", "a.rb"],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["a.rb"],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP, "a.rb"],
            2,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec!["a.rb"],
            2,
        ),
        (
            "exclude = ['a.rb']\n".into(),
            vec!["--only", COP, "--fix", "a.rb"],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nexclude = ['a.rb']\n"),
            vec!["--only", COP, "--fix", "a.rb"],
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        count(&unchanged(d.path(), BEFORE.as_bytes(), &args), n);
    }
}
#[test]
fn stdin_clean_and_error_files_do_not_write() {
    use std::io::Write;
    use std::process::Stdio;
    let d = project();
    fs::write(d.path().join("a.rb"), b"untouched\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(BEFORE.as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stderr.is_empty());
    count(&o, 2);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"untouched\n");
    unchanged(
        d.path(),
        AFTER.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
    );
    let invalid = "class Foo\n  def foo; @foo; end\n";
    fs::write(d.path().join("a.rb"), invalid).unwrap();
    let mtime = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), invalid);
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
}

#[test]
fn directives_optins_and_multiline_atomic_suppression() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "TrivialAccessors", "Style", "all"] {
            for action in ["disable", "disable-next"] {
                let source = format!("# {prefix}:{action} {target}\n{BEFORE}");
                count(
                    &unchanged(
                        d.path(),
                        source.as_bytes(),
                        &["--only", COP, "--fix", "a.rb"],
                    ),
                    0,
                );
            }
        }
        // Offense highlight is only `def`; suppression inside its subtree must
        // still block the whole-definition replacement.
        let source =
            format!("class Foo\n  def foo\n    @foo # {prefix}:disable {COP}\n  end\nend\n");
        count(
            &unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
            ),
            1,
        );
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled=false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\n{BEFORE}");
        count(&unchanged(d.path(), source.as_bytes(), &["a.rb"]), 2);
        count(&cli(d.path(), &["--only", COP, "--fix", "a.rb"]), 0);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            format!("# {prefix}:enable {COP}\n{AFTER}")
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn canonical_single_pass_bytes_and_stability() {
    let d = project();
    for (before, after) in [
        (BEFORE, AFTER),
        (
            "class Foo\n  def self.foo; @foo; end\nend\n",
            "class Foo\n  class << self\n    attr_reader :foo\n  end\nend\n",
        ),
    ] {
        for (header, newline) in [
            (b"\xef\xbb\xbf# docs\r\n".as_slice(), "\r\n"),
            (b"# coding: Windows-1252\n# caf\xe9\n".as_slice(), "\n"),
        ] {
            let bytes = |s: &str| {
                let mut b = header.to_vec();
                b.extend_from_slice(s.replace('\n', newline).as_bytes());
                b.extend_from_slice(format!("__END__{newline}").as_bytes());
                b.extend_from_slice(b"\xff\0 payload\r\n");
                b
            };
            let original = bytes(before);
            count(
                &unchanged(d.path(), &original, &["--only", COP, "a.rb"]),
                if before == BEFORE { 2 } else { 1 },
            );
            count(&cli(d.path(), &["--only", COP, "--fix", "a.rb"]), 0);
            let expected = bytes(after);
            assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), expected);
            count(
                &unchanged(d.path(), &expected, &["--only", COP, "--fix", "a.rb"]),
                0,
            );
        }
    }
}
#[test]
fn uncertainty_keeps_offenses_and_bytes() {
    let d = project();
    for source in [
        "class Foo < Parent\n  def foo; @foo; end\nend\n",
        "class Foo\n  def café; @café; end\nend\n",
        "TracePoint.new(:call) { |t| p [t.method_id, t.parameters] }.enable\nclass Foo\n  def foo; @foo; end\nend\n",
        "class Foo\n  def self.attr_reader; @attr_reader; end\n  def foo; @foo; end\nend\n",
        "class Foo\n  def self.method_added(n); p n; end\n  def foo; @foo; end\nend\n",
        "class Foo\n  def self.singleton_method_added(n); p n; end\n  def self.foo; @foo; end\nend\n",
        "class Foo\n  # Reader docs\n  def foo; @foo; end\nend\n",
        "class Foo\n  def foo # docs\n    @foo\n  end\nend\n",
        "class Foo\n  def foo\n    # implementation docs\n    @foo\n  end\nend\n",
        "class Foo\n  private def foo; @foo; end\nend\n",
        "class Foo\n  def foo; @foo; end; attr_writer :foo\nend\n",
        "class Foo\n  def foo; @foo; end\n  def foo; @foo; end\nend\n",
        "class Foo\n  attr_reader :foo\n  def foo; @foo; end\nend\n",
        "class Foo\n  def self.foo; @foo; end\n  class << self\n    def foo; @foo; end\n  end\nend\n",
        "class Foo\n  def foo; @foo; end\n  attr_reader :\"bad?\"\nend\n",
        "class Foo\n  def foo; @foo; end\nend\nclass Foo; end\n",
        "class Object\n  def foo; @foo; end\nend\n",
        "class Foo\n  def foo; @foo; end\nend\np Foo.instance_method(:foo).parameters\n",
        "class Foo\n  def foo; @foo; end\nend\np Foo.instance_method(:foo).source_location\n",
        "class Foo\n  def foo; @foo; end\nend\neval('Foo.new.foo')\n",
        "class Foo\n  def foo; @foo; end\nend\np __LINE__\n",
        "result = class Foo\n  def foo; @foo; end\nend\np result\n",
        "class Foo\n  def receiver; @receiver; end\n  def receiver.foo; @foo; end\nend\n",
        "class Foo\n  if flag\n    def foo; @foo; end\n  end\nend\n",
        "class << self\n  def foo; @foo; end\nend\n",
    ] {
        let o = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
        );
        assert!(String::from_utf8_lossy(&o.stdout).contains(COP), "{source}");
    }
}
fn ruby(dir: &Path, source: &str, observe: &str) -> Option<Output> {
    if Command::new("ruby").arg("--version").output().is_err() {
        return None;
    }
    fs::write(dir.join("fixture.rb"), source).unwrap();
    Some(
        Command::new("ruby")
            .current_dir(dir)
            .args([
                "--disable-gems",
                "-w",
                "-e",
                &format!("require_relative './fixture'; {observe}"),
            ])
            .output()
            .unwrap(),
    )
}
#[test]
fn setter_shaped_readers_keep_identity_arity_and_bytes() {
    let d = project();
    for (method, receiver) in [
        ("def foo=; @foo; end", "Foo.new"),
        ("def foo=(); @foo; end", "Foo.new"),
        ("def self.foo=; @foo; end", "Foo"),
        ("def self.foo=(); @foo; end", "Foo"),
    ] {
        let source = format!("class Foo\n  {method}\nend\n");
        let observe = format!(
            "o={receiver}; o.instance_variable_set(:@foo, 7); p [o.respond_to?(:foo=), o.respond_to?(:foo), o.method(:foo=).arity, o.send(:foo=)]"
        );
        let before = ruby(d.path(), &source, &observe);
        let o = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
        );
        count(&o, 1);
        assert_eq!(o.status.code(), Some(1));
        if let Some(before) = before {
            let after = ruby(
                d.path(),
                &fs::read_to_string(d.path().join("a.rb")).unwrap(),
                &observe,
            )
            .unwrap();
            assert!(before.status.success() && after.status.success());
            assert_eq!(before.stdout, b"[true, false, 0, 7]\n");
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
        }
    }
}
#[test]
fn controlled_ruby_values_visibility_return_values_and_eigenclass_binding_match() {
    let d = project();
    for (source, observe) in [
        (
            BEFORE,
            "o=Foo.new; p [o.foo, o.send(:foo=, 7), o.foo, Foo.instance_methods(false).sort]; p [o.method(:foo).arity, o.method(:foo=).arity]",
        ),
        (
            "class Foo\n  private\n  def foo; @foo; end\n  def foo=(val); @foo=val; end\n  protected\n  def bar; @bar; end\nend\n",
            "o=Foo.new; p [o.send(:foo=, 7), o.send(:foo), Foo.private_instance_methods(false).sort, Foo.protected_instance_methods(false).sort]",
        ),
        (
            "class Foo\n  private\n  def self.foo; @foo; end\n  def self.foo=(val); @foo=val; end\n  class << self\n    protected\n    def bar; @bar; end\n  end\nend\n",
            "p [Foo.foo, Foo.send(:foo=, 9), Foo.foo, Foo.singleton_methods(false).sort, Foo.protected_methods(false).sort, Foo.instance_methods(false)]",
        ),
        (
            "class Foo\n  class << self\n    private\n    def foo; @foo; end\n    def foo=(val); @foo=val; end\n  end\nend\n",
            "p [Foo.send(:foo=, 9), Foo.send(:foo), Foo.private_methods(false).sort, Foo.instance_methods(false)]",
        ),
    ] {
        let Some(a) = ruby(d.path(), source, observe) else {
            return;
        };
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"]);
        let fixed = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert_ne!(source, fixed);
        let b = ruby(d.path(), &fixed, observe).unwrap();
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
    }
}
#[test]
fn controlled_ruby_withheld_observers_callbacks_warnings_and_definition_values_match() {
    let d = project();
    for source in [
        "class Foo\n  def self.attr_reader(*n); p [:dispatch, n]; end\n  def foo; @foo; end\nend\n",
        "class Foo\n  def self.method_added(n); p [n, instance_method(n).parameters, instance_method(n).source_location]; end\n  def foo=(val); @foo=val; end\nend\n",
        "class Foo\n  def self.singleton_method_added(n); p [n, method(n).parameters]; end\n  def self.foo=(val); @foo=val; end\nend\n",
        "class Foo\n  def foo; @foo; end\n  def foo; @foo; end\nend\n",
        "class Foo\n  def foo; @foo; end\n  attr_reader :foo, :\"bad?\"\nend\n",
        "result = class Foo\n  def foo; @foo; end\nend\np [result, __LINE__, Foo.instance_method(:foo).source_location]\n",
        "class Foo\n  def foo=(val); @foo=val; end\nend\np Foo.instance_method(:foo=).parameters\n",
        "trace = TracePoint.new(:call) { |t| p [t.method_id, t.parameters] }; trace.enable\nclass Foo\n  def foo=(val); @foo=val; end\nend\nFoo.new.foo=7; trace.disable\n",
    ] {
        let Some(a) = ruby(d.path(), source, "") else {
            return;
        };
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
        );
        let b = ruby(
            d.path(),
            &fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "",
        )
        .unwrap();
        assert_eq!(a.status.code(), b.status.code());
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
    }
}
#[test]
fn style_layout_composition_stabilizes_within_four_explicit_passes() {
    let d = project();
    let source = "class Foo\nprivate\ndef foo; @foo; end;\ndef foo=(val); @foo=val; end;\nend\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    let selected = "Style/TrivialAccessors,Style/AccessorGrouping,Style/BisectedAttrAccessor,Style/Attr,Style/RedundantReturn,Style/RedundantParentheses,Style/StringLiterals,Style/HashSyntax,Style/SignalException,Style/Semicolon,Layout";
    for _ in 0..4 {
        cli(d.path(), &["--only", selected, "--fix", "a.rb"]);
    }
    let fixed = fs::read(d.path().join("a.rb")).unwrap();
    assert_ne!(fixed, source.as_bytes());
    let stable = cli(d.path(), &["--only", selected, "--fix", "a.rb"]);
    assert_eq!(stable.status.code(), Some(0), "{stable:?}");
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), fixed);
}
