//! group visibility detection must never move declarations or methods.
use crate::support::cli::project_with_lib;
use rustocop::{
    config::{RawConfig, ResolvedConfig},
    cops::{Registry, select_active},
    pipeline::lint_one,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Style/AccessModifierDeclarations";
const SOURCE: &str = "class Foo\n  # Docs for foo.\n  private def foo; end # trailing\nend\n";
fn cli(dir: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "{args:?}: {output:?}"
    );
    assert!(output.stderr.is_empty(), "{output:?}");
    output
}
fn count(output: &Output, expected: usize) {
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        text.lines().filter(|line| line.contains(COP)).count(),
        expected,
        "{text}"
    );
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str]) -> Output {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(dir, args);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    output
}

#[test]
fn exact_registry_default_pipeline_and_no_version_or_path_gates() {
    let registry = Registry::new();
    let entries: Vec<_> = registry
        .cops()
        .iter()
        .filter(|cop| cop.name() == COP)
        .collect();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].default_enabled());
    assert!(!entries[0].safe_fixes());
    assert_eq!(entries[0].ruby_versions(), Default::default());
    assert!(entries[0].path_policy().is_none());
    let config = ResolvedConfig::resolve(RawConfig::default(), None, &registry).unwrap();
    let active = select_active(&registry, &config, &[COP.into()], &[]);
    for path in ["lib/a.rb", "spec/a.rb", "test/a.rb"] {
        let report = lint_one(
            Path::new(path),
            Path::new(path),
            Some(SOURCE.as_bytes()),
            &active,
            &config,
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        assert_eq!(report.result.offenses.len(), 1);
        let offense = &report.result.offenses[0];
        assert_eq!((offense.line, offense.col, offense.end_line), (3, 3, 3));
        assert_eq!(
            offense.message,
            "`private` should not be inlined in method definitions."
        );
        assert!(offense.correction.is_none());
    }
}

#[test]
fn default_selectors_and_config_enabling_are_real_cli_paths() {
    let dir = project_with_lib();
    let disabled = format!("[cops.\"{COP}\"]\nenabled = false\n");
    let enabled = format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n");
    for (config, args, expected) in [
        ("", vec!["lib/a.rb"], 1),
        ("", vec!["--only", COP, "lib/a.rb"], 1),
        ("", vec!["--only", "Style", "lib/a.rb"], 1),
        ("", vec!["--except", COP, "lib/a.rb"], 0),
        (
            "",
            vec!["--only", COP, "--except", "Style", "--fix", "lib/a.rb"],
            0,
        ),
        (disabled.as_str(), vec!["lib/a.rb"], 0),
        (
            disabled.as_str(),
            vec!["--only", COP, "--fix", "lib/a.rb"],
            1,
        ),
        (enabled.as_str(), vec!["lib/a.rb"], 1),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        count(&unchanged(dir.path(), SOURCE.as_bytes(), &args), expected);
    }
}

#[test]
fn both_directive_prefixes_and_source_opt_ins_preserve_bytes_and_mtime() {
    let dir = project_with_lib();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "AccessModifierDeclarations", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                SOURCE.replace(
                    "  private",
                    &format!("  # {prefix}:disable-next {target}\n  private"),
                ),
                SOURCE.replace("# trailing", &format!("# {prefix}:disable {target}")),
            ] {
                count(
                    &unchanged(
                        dir.path(),
                        source.as_bytes(),
                        &["--only", COP, "--fix", "lib/a.rb"],
                    ),
                    0,
                );
            }
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let enabled = format!("# {prefix}:enable {COP}\n{SOURCE}");
        count(&unchanged(dir.path(), enabled.as_bytes(), &["lib/a.rb"]), 1);
        count(
            &unchanged(
                dir.path(),
                enabled.as_bytes(),
                &["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
            ),
            0,
        );
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        // Only the selector is highlighted, not the complete method body.
        let body_directive =
            format!("class Foo\n  private def foo\n    nil # {prefix}:disable {COP}\n  end\nend\n");
        count(
            &unchanged(
                dir.path(),
                body_directive.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
            ),
            1,
        );
    }
}

#[test]
fn global_and_cop_excludes_win_over_selection_and_source_enables() {
    let dir = project_with_lib();
    for config in [
        "exclude = ['lib/**']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let source = format!("# rustocop:enable {COP}\n{SOURCE}");
        count(
            &unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
            ),
            0,
        );
        count(&cli(dir.path(), &["--only", COP, "lib"]), 0);
    }
}

#[test]
fn fix_never_changes_unsafe_clean_disabled_or_byte_sensitive_files() {
    let dir = project_with_lib();
    for source in [
        SOURCE.as_bytes().to_vec(),
        SOURCE.replace('\n', "\r\n").into_bytes(),
        format!(
            "\u{feff}# encoding: UTF-8\r\n# café\r\n{}p __LINE__\r\n__END__\r\npayload\0\u{ff}\r\n",
            SOURCE.replace('\n', "\r\n")
        )
        .into_bytes(),
        [
            b"# encoding: ISO-8859-1\n# caf\xe9\n".as_slice(),
            SOURCE.as_bytes(),
            b"__END__\n\xff\0\r\n",
        ]
        .concat(),
    ] {
        count(
            &unchanged(dir.path(), &source, &["--only", COP, "lib/a.rb"]),
            1,
        );
        count(
            &unchanged(dir.path(), &source, &["--only", COP, "--fix", "lib/a.rb"]),
            1,
        );
    }
    for source in [
        "class Foo\n  private\n  # Docs for foo.\n  def foo; end\nend\n",
        "class Foo\n  public attr_reader :foo\n  private alias_method :bar, :foo\nend\n",
        "# rustocop:disable Style/AccessModifierDeclarations\nprivate def foo; end\n",
    ] {
        count(
            &unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
            ),
            0,
        );
    }
    let path = dir.path().join("lib/a.rb");
    let source = b"class Foo\n  private def foo; end\n";
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--fix", "lib/a.rb"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn stdin_reports_without_creating_its_display_path() {
    let dir = project_with_lib();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--stdin", "lib/a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(SOURCE.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    count(&output, 1);
    assert!(String::from_utf8_lossy(&output.stdout).contains("lib/a.rb:3:3:"));
    assert!(!dir.path().join("lib/a.rb").exists());
}

fn ruby(source: &str) -> Option<Vec<u8>> {
    let output = match Command::new("ruby")
        .args(["--disable-gems", "-W0", "-e", source])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("{error}"),
    };
    assert!(output.status.success(), "{source}: {output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    Some(output.stdout)
}

#[test]
fn no_movement_preserves_callbacks_visibility_scopes_lookup_and_line_values() {
    let dir = project_with_lib();
    let source = r#"$events = []
class Parent
  def inherited; :parent; end
end
class Example < Parent
  VALUE = :lexical
  def self.method_added(name)
    $events << [name, private_method_defined?(name), public_method_defined?(name)]
  end
  # Docs for first.
  private def first; VALUE; end
  define_method(:dynamic) { [VALUE, __LINE__] }
  public attr_reader :value
  protected attr_writer :value
  private alias_method :aliased, :first
  class << self
    private def secret; :singleton; end
    public
    def exposed; :public; end
  end
  public
  def next_public; :next; end
end
class Example
  public def reopened; :reopened; end
end
module Tools
  module_function def utility; :utility; end
  def later; :later; end
end
"#;
    count(
        &unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
        ),
        4,
    );
    let actual = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
    let observe = r#"p [Example.public_method_defined?(:dynamic), Example.private_method_defined?(:first), Example.private_method_defined?(:aliased), Example.protected_method_defined?(:value=)]
p [Example.new.send(:aliased), Example.new.send(:dynamic), Example.new.inherited, Example.new.reopened, Example.exposed, Example.send(:secret)]
p [Tools.utility, Tools.private_instance_methods(false).include?(:utility), Tools.public_instance_methods(false).include?(:later)]
p $events
"#;
    let Some(original) = ruby(&format!("{source}{observe}")) else {
        return;
    };
    assert!(String::from_utf8_lossy(&original).starts_with("[true, true, true, true]\n"));
    assert_eq!(Some(original.clone()), ruby(&format!("{actual}{observe}")));
    // Even a parseable local grouping changes both callback-time visibility
    // and the dynamically defined method. Detection must not apply that edit.
    let grouped = source.replace("private def first", "private; def first");
    assert_ne!(Some(original), ruby(&format!("{grouped}{observe}")));
}

#[test]
fn no_reordering_preserves_argument_evaluation_return_values_and_control_flow() {
    let dir = project_with_lib();
    let source = r#"$events = []
module Example
  def self.private(*names)
    $events << [:visibility, names]
    super
    :custom_return
  end
  def first; :first; end
  private(begin
    $events << :argument
    :first
  end)
  $events << :after
  define_method(:later) { :later }
end
"#;
    count(
        &unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
        ),
        1,
    );
    let actual = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
    let observe = "p $events; p [Example.private_instance_methods(false).include?(:first), Example.public_instance_methods(false).include?(:later)]; p Example.send(:private, :later)\n";
    let Some(original) = ruby(&format!("{source}{observe}")) else {
        return;
    };
    assert_eq!(
        original,
        b"[:argument, [:visibility, [:first]], :after]\n[true, true]\n:custom_return\n"
    );
    assert_eq!(Some(original), ruby(&format!("{actual}{observe}")));
}
