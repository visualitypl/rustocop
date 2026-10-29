//! fixed class policies through the real pipeline and CLI.
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

const CASES: [(&str, &str, bool); 3] = [
    (
        "Style/OneClassPerFile",
        "# A.\nclass A\nend\n\n# B.\nclass B\nend\n",
        true,
    ),
    ("Style/ClassVars", "# A.\nclass A\n  @@a = 1\nend\n", true),
    (
        "Style/StaticClass",
        "# A.\nclass A\n  def self.foo; end\nend\n",
        false,
    ),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for sub in [".git", "lib", "spec", "test"] {
        fs::create_dir(dir.path().join(sub)).unwrap();
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, cop: &str, args: &[&str], status: i32, count: usize) -> Output {
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
        text.lines().filter(|line| line.contains(cop)).count(),
        count,
        "{args:?}: {text}"
    );
    output
}
fn unchanged(
    dir: &Path,
    cop: &str,
    source: &[u8],
    args: &[&str],
    status: i32,
    count: usize,
) -> Output {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(dir, cop, args, status, count);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    output
}

#[test]
fn registered_fixed_defaults_versions_and_path_policy_through_pipeline() {
    let registry = Registry::new();
    let config = ResolvedConfig::resolve(RawConfig::default(), None, &registry).unwrap();
    for (cop, source, default) in CASES {
        let registered: Vec<_> = registry.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].default_enabled(), default);
        assert!(!registered[0].safe_fixes());
        assert_eq!(registered[0].ruby_versions(), Default::default());
        assert_eq!(
            registered[0].path_policy().is_some(),
            cop == "Style/OneClassPerFile"
        );
        let active = select_active(&registry, &config, &[cop.into()], &[]);
        for path in ["lib/a.rb", "spec/a.rb", "test/unit/a.rb", "lib/spec/a.rb"] {
            let report = lint_one(
                Path::new(path),
                Path::new(path),
                Some(source.as_bytes()),
                &active,
                &config,
            );
            assert!(report.errors.is_empty() && report.internal_errors.is_empty());
            let expected = usize::from(cop != "Style/OneClassPerFile" || path.starts_with("lib/"));
            assert_eq!(report.result.offenses.len(), expected, "{cop} {path}");
            assert!(
                report
                    .result
                    .offenses
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
    }
}

#[test]
fn cli_defaults_only_except_and_config_precedence_never_write() {
    let dir = project();
    for (cop, source, default) in CASES {
        let enable = format!("[cops.\"{cop}\"]\nenabled = true\n");
        let disable = format!("[cops.\"{cop}\"]\nenabled = false\n");
        let override_department = format!("[cops.Style]\nenabled = false\n{enable}");
        for (config, args, count) in [
            ("", vec!["lib/a.rb"], usize::from(default)),
            ("", vec!["--only", cop, "lib/a.rb"], 1),
            ("", vec!["--only", "Style", "lib/a.rb"], 1),
            ("", vec!["--except", cop, "lib/a.rb"], 0),
            ("", vec!["--only", cop, "--except", cop, "lib/a.rb"], 0),
            ("", vec!["--only", cop, "--except", "Style", "lib/a.rb"], 0),
            (disable.as_str(), vec!["lib/a.rb"], 0),
            (disable.as_str(), vec!["--only", cop, "lib/a.rb"], 1),
            (enable.as_str(), vec!["lib/a.rb"], 1),
            (override_department.as_str(), vec!["lib/a.rb"], 1),
        ] {
            // Isolate the historical Style precedence assertions from EmptyClass.
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("{config}\n[cops.\"Lint/EmptyClass\"]\nenabled = false\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                cop,
                source.as_bytes(),
                &args,
                i32::from(count != 0),
                count,
            );
            let mut fixing = args;
            fixing.insert(0, "--fix");
            unchanged(
                dir.path(),
                cop,
                source.as_bytes(),
                &fixing,
                i32::from(count != 0),
                count,
            );
        }
    }
}

#[test]
fn source_directives_and_static_class_source_opt_in_respect_highlight_ranges() {
    let dir = project();
    for (cop, source, default) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                let source = format!("# {prefix}:disable {target}\n{source}");
                unchanged(
                    dir.path(),
                    cop,
                    source.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    0,
                    0,
                );
            }
            let source = match cop {
                "Style/OneClassPerFile" => {
                    format!("class A; end\n# {prefix}:disable-next {cop}\nclass B\nend\n")
                }
                "Style/ClassVars" => {
                    format!("class A\n# {prefix}:disable-next {cop}\n@@a = 1\nend\n")
                }
                _ => format!("# {prefix}:disable-next {cop}\nclass A\ndef self.foo; end\nend\n"),
            };
            unchanged(
                dir.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                0,
                0,
            );
            // OneClassPerFile's header highlight does not include its body;
            // StaticClass's full-class highlight does. ClassVars is name-only.
            let source = match cop {
                "Style/OneClassPerFile" => {
                    format!("class A; end\nclass B\n  value # {prefix}:disable {cop}\nend\n")
                }
                "Style/ClassVars" => {
                    format!("class A\n  @@a = (\n    value # {prefix}:disable {cop}\n  )\nend\n")
                }
                _ => format!("class A\n  def self.foo; end # {prefix}:disable {cop}\nend\n"),
            };
            let count = usize::from(default);
            unchanged(
                dir.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                count as i32,
                count,
            );
        }
    }
    for prefix in ["rustocop", "rubocop"] {
        let cop = "Style/StaticClass";
        let source = format!("# {prefix}:enable {cop}\n{}", CASES[2].1);
        unchanged(
            dir.path(),
            cop,
            source.as_bytes(),
            &["--fix", "lib/a.rb"],
            1,
            1,
        );
        unchanged(
            dir.path(),
            cop,
            source.as_bytes(),
            &["--except", cop, "--fix", "lib/a.rb"],
            0,
            0,
        );
        unchanged(
            dir.path(),
            cop,
            source.as_bytes(),
            &["--only", "Style/ClassVars", "--fix", "lib/a.rb"],
            0,
            0,
        );
    }
    // Suppression cannot change the count of earlier declarations.
    unchanged(
        dir.path(),
        CASES[0].0,
        b"class A; end # rubocop:disable Style/OneClassPerFile\nclass B; end\n",
        &["--only", CASES[0].0, "--fix", "lib/a.rb"],
        1,
        1,
    );
}

#[test]
fn excludes_and_stdin_are_config_relative_and_never_bypass_pinned_spec_test_policy() {
    let dir = project();
    for (cop, source, _) in CASES {
        fs::write(dir.path().join("lib/a.rb"), source).unwrap();
        for (config, excluded) in [
            (String::new(), false),
            ("exclude = ['lib/**']\n".into(), true),
            (format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"), true),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let count = usize::from(!excluded);
            cli(
                &dir.path().join("lib"),
                cop,
                &["--only", cop, "--fix", "a.rb"],
                count as i32,
                count,
            );
            for (path, expected) in [
                ("a.rb", count),
                ("../spec/a.rb", usize::from(cop != CASES[0].0)),
            ] {
                let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                    .current_dir(dir.path().join("lib"))
                    .args(["--only", cop, "--stdin", path])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(source.as_bytes())
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(expected as i32),
                    "{cop} {path}: {output:?}"
                );
                assert!(output.stderr.is_empty(), "{output:?}");
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .filter(|line| line.contains(cop))
                        .count(),
                    expected
                );
            }
            assert_eq!(
                fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
                source
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for path in ["spec/a.rb", "test/a.rb"] {
            fs::write(dir.path().join(path), source).unwrap();
            let count = usize::from(cop != CASES[0].0);
            cli(
                dir.path(),
                cop,
                &["--only", cop, "--fix", path],
                count as i32,
                count,
            );
            assert_eq!(fs::read_to_string(dir.path().join(path)).unwrap(), source);
        }
    }
}

#[test]
fn detection_only_preserves_bom_crlf_encodings_line_data_and_error_files() {
    let dir = project();
    for (cop, source, _) in CASES {
        for bytes in [
            format!(
                "\u{feff}{}__END__\r\nclass Extra; end\r\n@@payload = 1\r\n",
                source.replace('\n', "\r\n")
            )
            .into_bytes(),
            [
                b"# encoding: ASCII-8BIT\n# \xff\n".as_slice(),
                source.as_bytes(),
                b"p __LINE__\n",
            ]
            .concat(),
            [
                b"# encoding: Shift_JIS\n# \x82\xa0\n".as_slice(),
                source.as_bytes(),
                b"p __LINE__\n",
            ]
            .concat(),
        ] {
            unchanged(dir.path(), cop, &bytes, &["--only", cop, "lib/a.rb"], 1, 1);
            unchanged(
                dir.path(),
                cop,
                &bytes,
                &["--only", cop, "--fix", "lib/a.rb"],
                1,
                1,
            );
        }
        unchanged(
            dir.path(),
            cop,
            b"class A; def (",
            &["--only", cop, "--fix", "lib/a.rb"],
            2,
            0,
        );
        unchanged(
            dir.path(),
            cop,
            b"module A\nend\n",
            &["--only", cop, "--fix", "lib/a.rb"],
            0,
            0,
        );
    }
}

#[test]
fn rejects_public_style_threshold_version_and_include_knobs() {
    let dir = project();
    for (cop, source, _) in CASES {
        for option in [
            "allowed_classes = ['A']",
            "max = 2",
            "target_ruby_version = '3.0'",
            "include = ['lib/**']",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{option}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                2,
                0,
            );
        }
    }
}

#[test]
fn safe_style_and_layout_corrections_do_not_rewrite_class_semantics() {
    let dir = project();
    let cop = "Style/StaticClass";
    let source = "# A.\nclass A\n  @@a = 1\n\n  def self.foo\n    return { :key => 'value' };\n  end\nend\n\n# B.\nclass B\nend\n";
    let expected = source.replace("return { :key => 'value' };", "{ key: \"value\" }");
    let only = "Style/OneClassPerFile,Style/ClassVars,Style/StaticClass,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    cli(dir.path(), cop, &["--only", only, "lib/a.rb"], 1, 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    cli(
        dir.path(),
        cop,
        &["--only", only, "--fix", "lib/a.rb"],
        1,
        1,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(
        dir.path(),
        cop,
        &["--only", only, "--fix", "lib/a.rb"],
        1,
        1,
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("0 corrections applied, 3 offenses remaining")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn isolated_ruby_observes_class_identity_lexical_lookup_inheritance_and_mixin_order() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let source = r#"EVENTS = []
VALUE = :outer
module Namespace
  VALUE = :inner
end
module Mix
  def mixed
    :mixed
  end
end
class Namespace::Tools
  @@count = 1
  EVENTS << :body
  extend Mix
  EVENTS << :extended
  def self.snapshot
    [VALUE, Module.nesting.map(&:name), @@count, mixed, __LINE__]
  end
end
class Child < Namespace::Tools
  @@count += 1
end
class Namespace::Tools
  private
  def secret
    :secret
  end
end
class << Namespace::Tools
  EVENTS << :singleton
end
p [Namespace::Tools.class, Child.superclass == Namespace::Tools, Namespace::Tools.snapshot, Namespace::Tools.new.respond_to?(:secret), EVENTS]
"#;
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
        String::from_utf8_lossy(&before),
        "[Class, true, [:outer, [\"Namespace::Tools\"], 2, :mixed, 17], false, [:body, :extended, :singleton]]\n"
    );
    cli(
        dir.path(),
        "Style/OneClassPerFile",
        &[
            "--only",
            "Style/OneClassPerFile,Style/ClassVars,Style/StaticClass",
            "--fix",
            "lib/a.rb",
        ],
        1,
        4,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(ruby(), before);
    // A separate actually flagged static class is instantiated/subclassed and
    // re-opened dynamically: converting it to a module would break execution.
    let source = "class Tools\n  def self.foo; :ok; end\nend\nclass Child < Tools; end\nTools.class_eval { def bar; :instance; end }\np [Tools.class, Tools.foo, Child.new.bar]\n";
    fs::write(&path, source).unwrap();
    let before = ruby();
    assert_eq!(
        String::from_utf8_lossy(&before),
        "[Class, :ok, :instance]\n"
    );
    cli(
        dir.path(),
        "Style/StaticClass",
        &["--only", "Style/StaticClass", "--fix", "lib/a.rb"],
        1,
        1,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(ruby(), before);
}
