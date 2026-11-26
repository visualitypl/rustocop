//! real CLI, conservative corrections, and controlled Ruby observations.
use crate::support::cli::project;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/TrivialAccessors";
const BEFORE: &str = "class Foo\n  def foo; @foo; end\n  def foo=(val); @foo=val; end\nend\n";
const AFTER: &str = "class Foo\n  attr_reader :foo\n  attr_writer :foo\nend\n";
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
        // Offense highlight is only `def`, so, as in RuboCop, a disable
        // inside its subtree does not stop the whole-definition replacement.
        fs::write(
            d.path().join("a.rb"),
            format!("class Foo\n  def foo\n    @foo # {prefix}:disable {COP}\n  end\nend\n"),
        )
        .unwrap();
        count(&cli(d.path(), &["--only", COP, "--fix", "a.rb"]), 0);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "class Foo\n  attr_reader :foo\nend\n"
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
