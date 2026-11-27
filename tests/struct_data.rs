//! fixed Struct/Data policies through the real pipeline and CLI.
use crate::support::cli::project_with_lib;
use rustocop::{
    config::{RawConfig, ResolvedConfig},
    cops::{
        Registry,
        policy::{RubyVersion, RubyVersions},
        select_active,
    },
    pipeline::lint_one,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const CASES: [(&str, &str, bool); 3] = [
    (
        "Style/DataInheritance",
        "# Person.\nclass Person < Data.define(:a)\nend\n",
        true,
    ),
    (
        "Style/StructInheritance",
        "# Person.\nclass Person < Struct.new(:a)\nend\n",
        true,
    ),
    (
        "Style/RedundantStructKeywordInit",
        "Struct.new(:a, keyword_init: true)\n",
        false,
    ),
];
fn run(dir: &Path, cop: &str, args: &[&str], count: usize, status: i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    if status != 2 {
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.contains(&format!("({cop})")))
            .count(),
        count,
        "{output:?}"
    );
}
fn unchanged(dir: &Path, cop: &str, source: &[u8], args: &[&str], count: usize, status: i32) {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir, cop, args, count, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn pipeline_registration_defaults_gates_and_detection_only_contract() {
    let registry = Registry::new();
    let config = ResolvedConfig::resolve(RawConfig::default(), None, &registry).unwrap();
    for (cop, source, default) in CASES {
        let found: Vec<_> = registry.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].default_enabled(), default);
        assert!(!found[0].safe_fixes());
        assert_eq!(
            found[0].ruby_versions(),
            RubyVersions {
                minimum: (cop != "Style/StructInheritance").then_some(RubyVersion(3, 2)),
                maximum: None
            }
        );
        assert!(found[0].path_policy().is_none());
        for only in [vec![], vec![cop.into()]] {
            let selected = select_active(&registry, &config, &only, &[]);
            let report = lint_one(
                Path::new("lib/a.rb"),
                Path::new("lib/a.rb"),
                Some(source.as_bytes()),
                &selected,
                &config,
            );
            assert!(report.errors.is_empty() && report.internal_errors.is_empty());
            let offenses: Vec<_> = report
                .result
                .offenses
                .iter()
                .filter(|o| o.cop == cop)
                .collect();
            assert_eq!(offenses.len(), usize::from(default || !only.is_empty()));
            assert!(offenses.iter().all(|o| o.correction.is_none()));
        }
    }
}

#[test]
fn cli_selection_config_directives_and_exclusions_never_write() {
    let dir = project_with_lib();
    for (cop, source, default) in CASES {
        let enable = format!("[cops.\"{cop}\"]\nenabled = true\n");
        let disable = format!("[cops.\"{cop}\"]\nenabled = false\n");
        let department = format!("[cops.Style]\nenabled = false\n{enable}");
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
            (department.as_str(), vec!["lib/a.rb"], 1),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                cop,
                source.as_bytes(),
                &args,
                count,
                count as i32,
            );
            let mut fixing = args;
            fixing.insert(0, "--fix");
            unchanged(
                dir.path(),
                cop,
                source.as_bytes(),
                &fixing,
                count,
                count as i32,
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rubocop", "rustocop"] {
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
            let opted = format!("# {prefix}:enable {cop}\n{source}");
            unchanged(dir.path(), cop, opted.as_bytes(), &["lib/a.rb"], 1, 1);
        }
        for config in [
            "exclude = ['lib/**']\n".to_string(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                0,
                0,
            );
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path().join("lib"))
                .args(["--only", cop, "--stdin", "a.rb"])
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
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty());
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn rejects_public_version_and_style_knobs() {
    let dir = project_with_lib();
    for (cop, source, _) in CASES {
        for option in [
            "target_ruby_version = '3.1'",
            "enforced_style = 'block'",
            "max = 2",
            "Include = ['lib/**']",
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
                0,
                2,
            );
        }
    }
}

#[test]
fn detection_only_preserves_bytes_line_data_encodings_and_error_files() {
    let dir = project_with_lib();
    for (cop, source, _) in CASES {
        for bytes in [
            format!(
                "\u{feff}{}__END__\r\nStruct.new(:payload, keyword_init: true)\r\n",
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
            for args in [
                vec!["--only", cop, "lib/a.rb"],
                vec!["--only", cop, "--fix", "lib/a.rb"],
            ] {
                unchanged(dir.path(), cop, &bytes, &args, 1, 1);
            }
        }
        unchanged(
            dir.path(),
            cop,
            b"class A; def (",
            &["--only", cop, "--fix", "lib/a.rb"],
            0,
            2,
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
fn mixed_safe_fixes_do_not_rewrite_superclasses_or_keyword_initialization() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let source = "class Person < Struct.new(:a, keyword_init: true)\n  def foo\n    return { :key => 'value' };\n  end\nend\nclass Other < Data.define(:a)\nend\n";
    let expected = source
        .replace("return { :key => 'value' };", "{ key: \"value\" }")
        .replace("end\nclass Other", "end\n\nclass Other");
    let only = "Style/StructInheritance,Style/DataInheritance,Style/RedundantStructKeywordInit,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    fs::write(&path, source).unwrap();
    run(
        dir.path(),
        "Style/StructInheritance",
        &["--only", only, "--fix", "lib/a.rb"],
        1,
        1,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    unchanged(
        dir.path(),
        "Style/DataInheritance",
        expected.as_bytes(),
        &["--only", only, "--fix", "lib/a.rb"],
        1,
        1,
    );
}

#[test]
fn isolated_ruby_preserves_nesting_ancestors_callbacks_visibility_and_keyword_semantics() {
    let available = Command::new("ruby")
        .args(["--disable-gems", "-e", "exit defined?(Data) ? 0 : 1"])
        .output();
    if !available.is_ok_and(|o| o.status.success()) {
        return;
    }
    let dir = project_with_lib();
    let source = r#"EVENTS = []
VALUE = :outer
module Mix
  VALUE = :inherited
  def self.included(base); EVENTS << :included; end
  def self.extended(base); EVENTS << :extended; end
  def mixed; :mixed; end
end
module N
  VALUE = :nested
  class Person < Struct.new(:a, keyword_init: true) { include Mix }
    EVENTS << :struct_body
    extend Mix
    private
    def secret; :secret; end
    public
    def snapshot; [VALUE, Module.nesting.map(&:name), mixed]; end
  end
  class Record < Data.define(:a) { include Mix }
    EVENTS << :data_body
    def snapshot; [VALUE, Module.nesting.map(&:name), mixed]; end
  end
end
class N::Qualified < Struct.new(:a) { include Mix }
  def snapshot; [VALUE, Module.nesting.map(&:name), mixed]; end
end
class N::Person
  def reopened; :reopened; end
end
class << N::Person
  def extra; :singleton; end
end
Strict = Struct.new(:a, keyword_init: true)
Flexible = Struct.new(:a, keyword_init: nil)
x = N::Person.new(a: 1)
y = N::Record.new(a: 2)
p [x.snapshot, y.snapshot, N::Qualified.new.snapshot, x.respond_to?(:secret), x.reopened, N::Person.extra]
p [N::Person.superclass.superclass == Struct, N::Record.superclass.superclass == Data, N::Person.ancestors.include?(Mix), N::Record.ancestors.include?(Mix), EVENTS]
p [Strict.keyword_init?, Flexible.keyword_init?, Strict.new({a: 3}).a, Flexible.new({a: 3}).a, Flexible.new(a: 3).a]
begin
  Strict.new(3)
rescue ArgumentError
  p :positional_rejected
end
p __LINE__
"#;
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let ruby = || {
        let output = Command::new("ruby")
            .args(["--disable-gems"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        output.stdout
    };
    let before = ruby();
    let observations = String::from_utf8_lossy(&before);
    assert!(
        observations.contains("[:nested, [\"N::Person\", \"N\"], :mixed]"),
        "{observations}"
    );
    assert!(
        observations.contains("[:inherited, [\"N::Qualified\"], :mixed]"),
        "{observations}"
    );
    assert!(
        observations.contains("[true, true, true, true, [:included, :struct_body, :extended, :included, :data_body, :included]]"),
        "{observations}"
    );
    assert!(
        observations.contains("[true, nil, 3, {:a=>3}, 3]"),
        "{observations}"
    );
    assert!(observations.contains(":positional_rejected"));
    let only = "Style/StructInheritance,Style/DataInheritance,Style/RedundantStructKeywordInit";
    run(
        dir.path(),
        "Style/StructInheritance",
        &["--only", only, "--fix", "lib/a.rb"],
        2,
        1,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(ruby(), before);
}
