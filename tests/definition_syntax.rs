//! real definition-syntax controls, bytes and isolated Ruby observations.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const CASES: [(&str, &str, &str); 3] = [
    ("Style/Alias", "alias :ala :bala\n", "alias ala bala\n"),
    (
        "Style/ColonMethodDefinition",
        "def self::foo; end\n",
        "def self.foo; end\n",
    ),
    (
        "Style/DefWithParentheses",
        "def foo()\n  42\nend\n",
        "def foo\n  42\nend\n",
    ),
];
const ONLY: &str = "Style/Alias,Style/ColonMethodDefinition,Style/DefWithParentheses";
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
fn defaults_selectors_config_and_no_write_contract() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let registry = rustocop::cops::Registry::new();
    for (cop, before, after) in CASES {
        let registered: Vec<_> = registry.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(registered.len(), 1);
        assert!(registered[0].default_enabled());
        assert!(registered[0].safe_fixes());
        assert!(registered[0].path_policy().is_none());
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let out = cli(dir.path(), &["lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
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
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
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
fn config_exclusions_and_directives_suppress_atomic_edits() {
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
    // An alias highlight is just the selector, but a whole-call replacement
    // crossing a disabled argument line must still be withheld atomically.
    let source = "class Fresh\n def bala; 42; end\n alias_method(\n :ala, # rubocop:disable Style/Alias\n :bala)\nend\n";
    fs::write(&path, source).unwrap();
    cli(
        dir.path(),
        &["--only", "Style/Alias", "--fix", "lib/a.rb"],
        1,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
}
#[test]
fn stdin_reports_each_cop_without_writing_display_paths() {
    use std::io::Write;
    use std::process::Stdio;
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
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains(cop));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn errors_clean_and_uncertain_offenses_do_not_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, status) in [
        ("def foo()=1\ndef bar()end\n", 0),
        ("alias_method :ala, :bala\n", 1),
        ("included { alias :ala :bala }\n", 1),
        ("def foo( # retain\n)\nend\n", 1),
        ("def foo() bar\nend\n", 1),
        ("def self::foo(\n", 2),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn token_edits_preserve_bom_crlf_encoding_data_and_explicit_lines() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "\u{feff}# encoding: UTF-8\r\ndef self::foo()\r\n  [\"café\", __LINE__]\r\nend\r\nalias :ala :bala\r\np __LINE__\r\n__END__\r\nraw::() :ala :bala\r\n".as_bytes(),
        b"# encoding: ISO-8859-1\r\ndef self::foo()\r\n  [\"caf\xe9\", __LINE__]\r\nend\r\nalias :ala :bala\r\np __LINE__\r\n__END__\r\nraw\xff::() :ala :bala\r\n",
    ] {
        fs::write(&path, source).unwrap();
        let mut expected = source.to_vec();
        for (from, to) in [(b"self::foo()".as_slice(), b"self.foo".as_slice()), (b"alias :ala :bala", b"alias ala bala")] {
            let start = expected.windows(from.len()).position(|w| w == from).unwrap();
            expected.splice(start..start + from.len(), to.iter().copied());
        }
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), expected);
    }
}
fn ruby(dir: &Path, harness: &str) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .args(["-w", "-e", harness, "lib/a.rb"])
        .output()
        .unwrap()
}
fn runtime_case(source: &str, harness: &str, expected_fixed: &str, status: i32) {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path(), harness);
    assert!(before.status.success(), "{before:?}");
    cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], status);
    assert_eq!(fs::read_to_string(&path).unwrap(), expected_fixed);
    let after = ruby(dir.path(), harness);
    assert!(after.status.success(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn controlled_ruby_alias_target_visibility_order_and_values() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let source = "class Fresh\n  def original(x = 4, key: 2); [x + key, __LINE__]; end\n  private\n  alias_method :renamed, :original\n  alias :other :original\nend\nmodule FreshModule\n  def original; 7; end\n  alias_method :renamed, :original\n  public\nend\n";
    let fixed = source
        .replace("alias_method :renamed, :original", "alias renamed original")
        .replace("alias :other :original", "alias other original");
    runtime_case(
        source,
        "load ARGV[0]; p [Fresh.new.send(:renamed), Fresh.new.send(:other), Fresh.private_instance_methods(false).sort, Fresh.instance_method(:renamed).source_location, Object.new.extend(FreshModule).renamed]",
        &fixed,
        0,
    );
}
#[test]
fn controlled_ruby_alias_preserves_enclosing_declaration_eval_result() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    for scope in ["class", "module"] {
        for aliases in [
            "alias_method :renamed, :original",
            "alias_method :earlier, :original; alias_method :renamed, :earlier",
        ] {
            let dir = project();
            let path = dir.path().join("lib/a.rb");
            let source = format!("{scope} Fresh; def original; 42; end; {aliases}; end\n");
            fs::write(&path, &source).unwrap();
            let harness = "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING, ARGV[0])";
            let before = ruby(dir.path(), harness);
            assert!(before.status.success(), "{before:?}");
            assert_eq!(before.stdout, b":renamed\n");
            let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", "Style/Alias", "--fix", "lib/a.rb"])
                .output()
                .unwrap();
            let after = ruby(dir.path(), harness);
            assert!(after.status.success(), "{after:?}");
            assert_eq!(before.stdout, after.stdout, "{source}");
            assert_eq!(before.stderr, after.stderr);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                source.replace("alias_method :earlier, :original", "alias earlier original")
            );
        }
    }
}
#[test]
fn controlled_ruby_quoted_operator_and_keyword_alias_identities() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let source = "class Fresh\n  def original; [42, __LINE__]; end\n  alias_method :\"@type\", :original\n  alias_method :\"foo bar\", :original\n  alias_method :[], :original\n  alias_method :if, :original\n  alias :\"escaped\\x5fname\" :original\n  alias :+@ :original\nend\n";
    let fixed = source
        .replace(
            "alias_method :\"@type\", :original",
            "alias :\"@type\" original",
        )
        .replace(
            "alias_method :\"foo bar\", :original",
            "alias :\"foo bar\" original",
        )
        .replace("alias_method :[], :original", "alias [] original")
        .replace("alias_method :if, :original", "alias if original")
        .replace(
            "alias :\"escaped\\x5fname\" :original",
            "alias escaped_name original",
        )
        .replace("alias :+@ :original", "alias +@ original");
    runtime_case(
        source,
        "load ARGV[0]; p [:original, :\"@type\", :\"foo bar\", :[], :if, :escaped_name, :+@].map { |name| [name, Fresh.new.send(name), Fresh.instance_method(name).original_name] }",
        &fixed,
        0,
    );
}

#[test]
fn controlled_ruby_definition_binding_evaluation_flow_and_literals() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let source = r#"$events = []
$object = Object.new
def receiver(); $events << :receiver; $object; end
def (receiver)::value() = [42, __LINE__]
def (receiver)::value=(value); @value = value; end
def (receiver)::[](key); [$events << key, @value]; end
def (receiver)::+(value); value + 1; end
def (receiver)::text()
  <<~TEXT
    café #{__LINE__}
TEXT
end
class Fresh
  private def value()
    return [42, __LINE__]
  end
  def bang()end
  def tight()=1
end
p [$object.value, $object.text, $events, Fresh.new.send(:value), __LINE__]
$object.value = 7
p [$object[:key], $object + 3]
"#;
    let fixed = source
        .replace("receiver()", "receiver")
        .replace("::", ".")
        .replace("value()", "value")
        .replace("text()", "text");
    runtime_case(source, "load ARGV[0]", &fixed, 0);
}
#[test]
fn controlled_ruby_uncertain_alias_dispatch_eval_and_return_values_are_preserved() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let source = r#"$events = []
class Fresh
  def original; :original; end
  def self.method_added(name); $events << name; end
  def self.alias_method(*args); $events << [:custom, args]; super; end
  private
  NAME = alias_method :assigned, :original
  alias_method :renamed, :original
end
class Base
  def original; :base; end
end
class Derived < Base
  def original; :derived; end
  def self.install
    class_eval { alias renamed original }
  end
end
Derived.install
p [Fresh::NAME, Fresh.new.send(:renamed), Fresh.private_instance_methods(false), Derived.new.renamed, $events]
"#;
    runtime_case(source, "load ARGV[0]", source, 1);
}
#[test]
fn mixed_style_layout_composition_stabilizes_within_four_explicit_passes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def self::foo();\n  return ('x');  \nend\nalias :ala :bala\n";
    fs::write(&path, source).unwrap();
    let only = format!(
        "{ONLY},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout"
    );
    for _ in 0..4 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", &only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def self.foo\n  \"x\"\nend\nalias ala bala\n"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
