//! class-method policies through real selectors, writes and Ruby.
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
const CASES: [(&str, &str, bool, bool); 3] = [
    (
        "Style/OpenStructUse",
        "value = OpenStruct.new(a: 42)\n",
        true,
        false,
    ),
    (
        "Style/ClassMethods",
        "# A.\nclass A\n  def A.one; :one; end\nend\n",
        true,
        true,
    ),
    (
        "Style/ClassMethodsDefinitions",
        "# A.\nclass A\n  class << self\n    def one; :one; end\n  end\nend\n",
        false,
        true,
    ),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for path in [".git", "lib", "spec"] {
        fs::create_dir(dir.path().join(path)).unwrap();
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    assert_ne!(output.status.code(), Some(2), "{args:?}: {output:?}");
    output
}
fn count(output: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|s| s.contains(cop))
        .count()
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str]) -> Output {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(dir, args);
    assert_eq!(fs::read(&path).unwrap(), source, "{args:?}");
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        modified,
        "{args:?}"
    );
    output
}
#[test]
fn pipeline_defaults_opt_in_safe_capabilities_and_unbounded_ruby_paths() {
    let registry = Registry::new();
    let config = ResolvedConfig::resolve(RawConfig::default(), None, &registry).unwrap();
    for (cop, source, default, safe) in CASES {
        let implementation = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert_eq!(implementation.default_enabled(), default);
        assert_eq!(implementation.safe_fixes(), safe);
        assert_eq!(implementation.ruby_versions(), Default::default());
        assert!(implementation.path_policy().is_none());
        for path in ["lib/a.rb", "spec/a.rb", "test/a.rb"] {
            for (only, except, expected) in [
                (vec![], vec![], usize::from(default)),
                (vec![cop.into()], vec![], 1),
                (vec![cop.into()], vec![cop.into()], 0),
            ] {
                let active = select_active(&registry, &config, &only, &except);
                let report = lint_one(
                    Path::new(path),
                    Path::new(path),
                    Some(source.as_bytes()),
                    &active,
                    &config,
                );
                assert!(report.errors.is_empty() && report.internal_errors.is_empty());
                assert_eq!(
                    report
                        .result
                        .offenses
                        .iter()
                        .filter(|o| o.cop == cop)
                        .count(),
                    expected
                );
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
}
#[test]
fn cli_defaults_selectors_config_and_no_write_without_fix() {
    let dir = project();
    for (cop, source, default, _) in CASES {
        let enabled = format!("[cops.\"{cop}\"]\nenabled = true\n");
        let disabled = format!("[cops.\"{cop}\"]\nenabled = false\n");
        let override_department = format!("[cops.Style]\nenabled = false\n{enabled}");
        for (config, args, expected) in [
            ("", vec!["lib/a.rb"], usize::from(default)),
            ("", vec!["--only", cop, "lib/a.rb"], 1),
            ("", vec!["--only", "Style", "lib/a.rb"], 1),
            ("", vec!["--except", cop, "lib/a.rb"], 0),
            ("", vec!["--only", cop, "--except", cop, "lib/a.rb"], 0),
            ("", vec!["--only", cop, "--except", "Style", "lib/a.rb"], 0),
            (enabled.as_str(), vec!["lib/a.rb"], 1),
            (disabled.as_str(), vec!["lib/a.rb"], 0),
            (disabled.as_str(), vec!["--only", cop, "lib/a.rb"], 1),
            (override_department.as_str(), vec!["lib/a.rb"], 1),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let output = unchanged(dir.path(), source.as_bytes(), &args);
            assert_eq!(count(&output, cop), expected, "{cop} {args:?}: {output:?}");
        }
    }
}
#[test]
fn directives_excludes_and_source_opt_in_prevent_or_enable_safe_writes() {
    let dir = project();
    for (cop, source, _, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                let disabled = format!("# {prefix}:disable {target}\n{source}");
                let output = unchanged(
                    dir.path(),
                    disabled.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                );
                assert_eq!(output.status.code(), Some(0));
                assert_eq!(count(&output, cop), 0);
            }
        }
        for config in [
            "exclude = ['lib/**']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let output = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
            );
            assert_eq!(output.status.code(), Some(0));
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    let (cop, source, _, _) = CASES[2];
    for prefix in ["rubocop", "rustocop"] {
        let source = format!("# {prefix}:enable {cop}\n{source}");
        let output = unchanged(dir.path(), source.as_bytes(), &["lib/a.rb"]);
        assert_eq!(count(&output, cop), 1);
        // Isolate the directive-enabled cop: the default-on
        // SingleLineMethods can independently rewrite this fixture's body.
        let except = format!("{cop},Style/SingleLineMethods");
        let output = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--except", &except, "--fix", "lib/a.rb"],
        );
        assert_eq!(count(&output, cop), 0);
    }
    // A disabled interior line suppresses a whole singleton-class offense.
    let source = "class A\nclass << self\ndef one; end # rubocop:disable Style/ClassMethodsDefinitions\nend\nend\n";
    assert_eq!(
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"]
        )
        .status
        .code(),
        Some(0)
    );
}
#[test]
fn canonical_fixes_are_atomic_single_pass_and_stable_with_bytes_preserved() {
    let dir = project();
    for (cop, source, _, safe) in CASES {
        let expected = match cop {
            "Style/ClassMethods" => source.replace("def A.", "def self."),
            "Style/ClassMethodsDefinitions" => {
                "# A.\nclass A\n  def self.one; :one; end\nend\n".into()
            }
            _ => source.into(),
        };
        for (before, after) in [
            (source.as_bytes().to_vec(), expected.as_bytes().to_vec()),
            (
                format!(
                    "\u{feff}{}__END__\r\nraw payload\r\n",
                    source.replace('\n', "\r\n")
                )
                .into_bytes(),
                format!(
                    "\u{feff}{}__END__\r\nraw payload\r\n",
                    expected.replace('\n', "\r\n")
                )
                .into_bytes(),
            ),
            (
                [
                    b"# encoding: ASCII-8BIT\n# \xff\n".as_slice(),
                    source.as_bytes(),
                ]
                .concat(),
                [
                    b"# encoding: ASCII-8BIT\n# \xff\n".as_slice(),
                    expected.as_bytes(),
                ]
                .concat(),
            ),
            (
                [
                    b"# encoding: Shift_JIS\n# \x82\xa0\n".as_slice(),
                    source.as_bytes(),
                ]
                .concat(),
                [
                    b"# encoding: Shift_JIS\n# \x82\xa0\n".as_slice(),
                    expected.as_bytes(),
                ]
                .concat(),
            ),
        ] {
            unchanged(dir.path(), &before, &["--only", cop, "lib/a.rb"]);
            let output = cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"]);
            assert_eq!(output.status.code(), Some(i32::from(!safe)), "{output:?}");
            let after = crate::support::lf::fixed(&before, &after);
            assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), after);
            unchanged(dir.path(), &after, &["--only", cop, "--fix", "lib/a.rb"]);
        }
    }
}
#[test]
fn stdin_uses_config_relative_paths_and_never_writes() {
    let dir = project();
    for (cop, source, _, _) in CASES {
        for excluded in [false, true] {
            fs::write(
                dir.path().join("rustocop.toml"),
                if excluded {
                    format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n")
                } else {
                    String::new()
                },
            )
            .unwrap();
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
            assert_eq!(
                output.status.code(),
                Some(i32::from(!excluded)),
                "{output:?}"
            );
            assert!(output.stderr.is_empty());
            assert_eq!(count(&output, cop), usize::from(!excluded));
            assert!(!dir.path().join("lib/a.rb").exists());
        }
    }
}
fn ruby(path: &Path) -> Vec<u8> {
    let output = Command::new("ruby")
        .arg("--disable-gems")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    output.stdout
}
#[test]
fn controlled_ruby_confirms_canonical_values_binding_visibility_and_return_values() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    for (cop, source, expected) in [
        (
            CASES[1].0,
            "class A\n  def A.one(x = 2); [x, self, Module.nesting, __LINE__]; end\nend\np [A.one, A.singleton_class.public_instance_methods(false)]\n",
            "[[2, A, [A], 2], [:one]]\n",
        ),
        (
            CASES[2].0,
            "class A\n  class << self\n    def one(x = 2); [x, :one]; end\n  end\nend\np [A.one, A.singleton_class.public_instance_methods(false)]\n",
            "[[2, :one], [:one]]\n",
        ),
        (
            CASES[2].0,
            "class A\n  class << self\n    def one; :one; end\n  end\nend\n",
            "",
        ),
        (
            CASES[2].0,
            "class A\n  class << self\n    def one(x = 2); return [x, :one]; :unreachable; end\n  end\nend\np A.one\n",
            "[2, :one]\n",
        ),
    ] {
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, source).unwrap();
        let before = ruby(&path);
        assert_eq!(before, expected.as_bytes());
        let output = cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_ne!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(ruby(&path), before);
        if expected.is_empty() {
            // Observe the class expression's value externally, without adding
            // executable statements to the proven declaration prefix.
            let evaluate = |text: &str| {
                let output = Command::new("ruby")
                    .args([
                        "--disable-gems",
                        "-e",
                        "p eval(ARGV.fetch(0), TOPLEVEL_BINDING, 'fixture.rb')",
                        text,
                    ])
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{output:?}");
                output.stdout
            };
            assert_eq!(evaluate(source), b":one\n");
            assert_eq!(
                evaluate(&fs::read_to_string(&path).unwrap()),
                evaluate(source)
            );
        }
    }
}
#[test]
fn mixed_style_layout_fixes_preserve_and_stabilize_class_method_rules() {
    let dir = project();
    let source = "class A\n  class << self\n    def one\n      return { :key => 'value' };\n    end\n  end\nend\n";
    let expected = "class A\n  def self.one\n    { key: \"value\" }\n  end\nend\n";
    let only = "Style/ClassMethodsDefinitions,Style/ClassMethods,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    // Overlapping fixes are deferred atomically. At most three explicit passes
    // on this fixture: singleton extraction, return/hash/string, then layout.
    for _ in 0..3 {
        cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"]);
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    unchanged(
        dir.path(),
        expected.as_bytes(),
        &["--only", only, "--fix", "lib/a.rb"],
    );
}
