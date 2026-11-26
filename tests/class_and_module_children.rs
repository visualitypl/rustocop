//! fixed nested-style policy and unsafe-correction no-write contract.
use crate::support::cli::project_with_lib;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use rustocop::{
    config::{RawConfig, ResolvedConfig},
    cops::{Registry, select_active},
    pipeline::lint_one,
};

const COP: &str = "Style/ClassAndModuleChildren";
const SOURCE: &str = "# Namespace.\nmodule Foo\nend\n\nclass Foo::Bar\nend\n";

fn cli(dir: &Path, args: &[&str], status: i32, count: usize) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    if status != 2 {
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        text.lines().filter(|line| line.contains(COP)).count(),
        count,
        "{text}"
    );
    output
}

fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32, count: usize) -> Output {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(dir, args, status, count);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    output
}

#[test]
fn real_pipeline_has_fixed_default_and_no_path_or_version_gate() {
    let registry = Registry::new();
    let cops: Vec<_> = registry.cops().iter().filter(|c| c.name() == COP).collect();
    assert_eq!(cops.len(), 1);
    assert!(cops[0].default_enabled());
    assert!(!cops[0].safe_fixes());
    assert!(cops[0].path_policy().is_none());
    assert_eq!(cops[0].ruby_versions(), Default::default());
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
        assert_eq!((offense.line, offense.col, offense.end_line), (5, 7, 5));
        assert_eq!(
            offense.message,
            "Use nested module/class definitions instead of compact style."
        );
        assert!(offense.correction.is_none());
    }
}

#[test]
fn default_selectors_and_config_precedence_never_rewrite_namespaces() {
    let dir = project_with_lib();
    for (config, args, count) in [
        ("", vec!["lib/a.rb"], 1),
        ("", vec!["--only", COP, "lib/a.rb"], 1),
        ("", vec!["--only", "Style", "lib/a.rb"], 1),
        ("", vec!["--except", COP, "lib/a.rb"], 0),
        ("", vec!["--only", COP, "--except", COP, "lib/a.rb"], 0),
        ("", vec!["--only", COP, "--except", "Style", "lib/a.rb"], 0),
        ("[cops.Style]\nenabled = false\n", vec!["lib/a.rb"], 0),
        (
            "[cops.\"Style/ClassAndModuleChildren\"]\nenabled = false\n",
            vec!["lib/a.rb"],
            0,
        ),
        (
            "[cops.\"Style/ClassAndModuleChildren\"]\nenabled = false\n",
            vec!["--only", COP, "lib/a.rb"],
            1,
        ),
        (
            "[cops.Style]\nenabled = false\n[cops.\"Style/ClassAndModuleChildren\"]\nenabled = true\n",
            vec!["lib/a.rb"],
            1,
        ),
    ] {
        // Isolate this older cop's default/config assertions from the new
        // default-on multiple-top-level-definition and empty-class rules.
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("{config}\n[cops.\"Style/OneClassPerFile\"]\nenabled = false\n[cops.\"Lint/EmptyClass\"]\nenabled = false\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &args,
            i32::from(count != 0),
            count,
        );
        let mut fixing = args;
        fixing.insert(0, "--fix");
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &fixing,
            i32::from(count != 0),
            count,
        );
    }
}

#[test]
fn directives_apply_to_the_identifier_range_not_the_class_body() {
    let dir = project_with_lib();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\nclass Foo::Bar\nend\n"),
                format!("class Foo::Bar # {prefix}:disable {target}\nend\n"),
                format!("class Foo::\n  Bar # {prefix}:disable {target}\nend\n"),
            ] {
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    0,
                    0,
                );
            }
        }
        let source = format!("class Foo::Bar\n  value # {prefix}:disable {COP}\nend\n");
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
            1,
        );
        let source = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        let output = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
            1,
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("lib/a.rb:13:7:"));
    }
}

#[test]
fn config_relative_excludes_and_stdin_respect_selection_without_writes() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, SOURCE).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    for (config, count) in [
        ("", 1),
        ("exclude = ['lib/**']\n", 0),
        (
            "[cops.\"Style/ClassAndModuleChildren\"]\nexclude = ['lib/**']\n",
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        cli(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            count,
            count as usize,
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path().join("lib"))
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
            .write_all(SOURCE.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(count), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| line.contains(COP))
                .count(),
            count as usize
        );
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn unsafe_fix_preserves_bom_crlf_encodings_line_and_data_and_error_files() {
    let dir = project_with_lib();
    for source in [
        "\u{feff}class Foo::Bar\r\n  VALUE = [__LINE__, \"é\"]\r\nend\r\n__END__\r\nmodule X::Y\r\n".as_bytes(),
        b"# encoding: ASCII-8BIT\nclass Foo::Bar\n  VALUE = \"\xff\"\nend\n",
        b"# encoding: Shift_JIS\nmodule Foo::Bar\n  VALUE = \"\x82\xa0\"\nend\n",
    ] {
        for args in [vec!["--only", COP, "lib/a.rb"], vec!["--only", COP, "--fix", "lib/a.rb"]] {
            unchanged(dir.path(), source, &args, 1, 1);
        }
    }
    unchanged(
        dir.path(),
        b"module Foo\n  class Bar\n  end\nend\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        0,
        0,
    );
    unchanged(
        dir.path(),
        b"class Foo::Bar\n def (",
        &["--only", COP, "--fix", "lib/a.rb"],
        2,
        0,
    );
}

#[test]
fn safe_style_and_layout_edits_leave_namespace_and_its_offense_intact() {
    let dir = project_with_lib();
    let source = "# Namespace.\nmodule Foo\nend\n\nclass Foo::Bar\n  def value\n    return { :key => 'value' };\n  end\nend\n";
    let expected = "# Namespace.\nmodule Foo\nend\n\nclass Foo::Bar\n  def value\n    { key: \"value\" }\n  end\nend\n";
    let only = "Style/ClassAndModuleChildren,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    cli(dir.path(), &["--only", only, "lib/a.rb"], 1, 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 1, 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 1, 1);
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("0 corrections applied, 1 offense remaining")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn rejects_style_overrides_instead_of_silently_accepting_them() {
    let dir = project_with_lib();
    for option in [
        "enforced_style = 'compact'",
        "enforced_style_for_classes = 'compact'",
        "max = 3",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            2,
            0,
        );
    }
}

#[test]
fn ruby_constant_lookup_nesting_superclass_visibility_and_order_remain_observable() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("skipping controlled runtime check: Ruby unavailable");
        return;
    }
    let source = r#"EVENTS = []
Base = Class.new
VALUE = :outer
module Foo
  Base = Class.new
  VALUE = :inner
end
module Mix
  def mixed
    :mixed
  end
end
class Foo::Bar < (EVENTS << :super; Base)
  EVENTS << :body
  include Mix
  extend Mix
  private
  def secret
    [VALUE, Module.nesting.map(&:name), __LINE__]
  end
end
class << Foo::Bar
  def snapshot
    [mixed, new.mixed, new.respond_to?(:secret), new.send(:secret)]
  end
end
p [Foo::Bar.superclass == Base, Foo::Bar.snapshot, EVENTS]
"#;
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let ruby = || {
        let output = Command::new("ruby")
            .arg("--disable-gems")
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        output.stdout
    };
    let before = ruby();
    assert_eq!(
        String::from_utf8(before.clone()).unwrap(),
        "[true, [:mixed, :mixed, false, [:outer, [\"Foo::Bar\"], 19]], [:super, :body]]\n"
    );
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 1, 1);
    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
    assert_eq!(ruby(), before);
    // A tempting nested rewrite is parseable but changes actual Ruby binding.
    let nested = source
        .replace("class Foo::Bar <", "module Foo\nclass Bar <")
        .replace("\nclass << Foo::Bar", "\nend\nclass << Foo::Bar");
    fs::write(&path, nested).unwrap();
    let changed = String::from_utf8(ruby()).unwrap();
    assert!(
        changed.starts_with("[false, [:mixed, :mixed, false, [:inner, [\"Foo::Bar\", \"Foo\"]"),
        "{changed}"
    );
}
