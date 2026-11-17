//! accessor policy, atomic corrections and Ruby semantic regressions.
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
const CASES: [(&str, &str, &str, usize); 3] = [
    (
        "Style/AccessorGrouping",
        "class Foo\n  attr_reader :one\n  attr_reader :two\nend\n",
        "class Foo\n  attr_reader :one, :two\nend\n",
        2,
    ),
    (
        "Style/Attr",
        "class Foo\n  attr(:one, :two)\nend\n",
        "class Foo\n  attr_reader(:one, :two)\nend\n",
        1,
    ),
    (
        "Style/BisectedAttrAccessor",
        "class Foo\n  attr_reader :one\n  attr_writer :one\nend\n",
        "class Foo\n  attr_accessor :one\nend\n",
        2,
    ),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
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
fn count(o: &Output, cop: &str, expected: usize) {
    let text = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        text.lines().filter(|l| l.contains(cop)).count(),
        expected,
        "{text}"
    );
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str]) -> Output {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(dir, args);
    assert_eq!(fs::read(&path).unwrap(), source, "{args:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    o
}
#[test]
fn exact_registry_defaults_pipeline_and_canonical_single_pass_fixes() {
    let registry = Registry::new();
    let config = ResolvedConfig::resolve(RawConfig::default(), None, &registry).unwrap();
    let dir = project();
    for (cop, before, after, n) in CASES {
        let entries: Vec<_> = registry.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].default_enabled());
        assert!(entries[0].safe_fixes());
        assert_eq!(entries[0].ruby_versions(), Default::default());
        assert!(entries[0].path_policy().is_none());
        let active = select_active(&registry, &config, &[cop.into()], &[]);
        let report = lint_one(
            Path::new("spec/a.rb"),
            Path::new("spec/a.rb"),
            Some(before.as_bytes()),
            &active,
            &config,
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        assert_eq!(report.result.offenses.len(), n);
        count(
            &unchanged(dir.path(), before.as_bytes(), &["--only", cop, "lib/a.rb"]),
            cop,
            n,
        );
        count(
            &cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"]),
            cop,
            0,
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            after
        );
        count(
            &unchanged(
                dir.path(),
                after.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
            ),
            cop,
            0,
        );
    }
}
#[test]
fn default_only_except_config_disabling_and_explicit_enabling() {
    let d = project();
    for (cop, before, _, n) in CASES {
        for (config, args, expected) in [
            (String::new(), vec!["lib/a.rb"], n),
            (String::new(), vec!["--only", "Style", "lib/a.rb"], n),
            (String::new(), vec!["--except", cop, "lib/a.rb"], 0),
            (
                String::new(),
                vec!["--only", cop, "--except", "Style", "--fix", "lib/a.rb"],
                0,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["lib/a.rb"],
                0,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop, "lib/a.rb"],
                n,
            ),
            (
                format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec!["lib/a.rb"],
                n,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            count(
                &unchanged(d.path(), before.as_bytes(), &args),
                cop,
                expected,
            );
        }
    }
}
#[test]
fn excludes_stdin_and_no_fix_never_write() {
    let d = project();
    for (cop, before, _, n) in CASES {
        for config in [
            "exclude = ['lib/**']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            count(
                &unchanged(
                    d.path(),
                    before.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                ),
                cop,
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        let o = child.wait_with_output().unwrap();
        count(&o, cop, n);
        assert!(o.stderr.is_empty());
        assert_eq!(
            fs::read(d.path().join("lib/a.rb")).unwrap(),
            before.as_bytes()
        );
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
fn controlled_ruby_canonical_values_visibility_and_singleton_binding_match() {
    let d = project();
    for (cop, before, after, _) in CASES {
        // Observe from a separate driver, not by executing arbitrary corpus code.
        let observe = "o=Foo.new; o.instance_variable_set(:@one, 7); o.instance_variable_set(:@two, 9); p [o.one, (o.two if o.respond_to?(:two)), Foo.public_instance_methods(false).sort, Foo.private_instance_methods(false).sort]; o.one=11 if o.respond_to?(:one=); p o.one";
        let Some(a) = ruby(d.path(), before, observe) else {
            return;
        };
        fs::write(d.path().join("lib/a.rb"), before).unwrap();
        cli(d.path(), &["--only", cop, "--fix", "lib/a.rb"]);
        let fixed = fs::read_to_string(d.path().join("lib/a.rb")).unwrap();
        assert_eq!(fixed, after);
        let b = ruby(d.path(), &fixed, observe).unwrap();
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
    }
    let before = "class Foo\n  class << self\n    private\n    attr_reader :one\n    attr_reader :two\n  end\nend\n";
    let observe = "Foo.instance_variable_set(:@one, 7); p [Foo.send(:one), Foo.private_methods.include?(:one), Foo.instance_methods(false)]";
    let Some(a) = ruby(d.path(), before, observe) else {
        return;
    };
    fs::write(d.path().join("lib/a.rb"), before).unwrap();
    cli(
        d.path(),
        &["--only", "Style/AccessorGrouping", "--fix", "lib/a.rb"],
    );
    let fixed = fs::read_to_string(d.path().join("lib/a.rb")).unwrap();
    assert!(fixed.contains("attr_reader :one, :two"));
    let b = ruby(d.path(), &fixed, observe).unwrap();
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
}
#[test]
fn mixed_style_layout_single_passes_converge_without_oscillation() {
    let d = project();
    let source =
        "class Foo\nattr :one;\nattr :two;\nattr_reader :three;\nattr_writer :three;\nend\n";
    fs::write(d.path().join("lib/a.rb"), source).unwrap();
    let selected =
        "Style/AccessorGrouping,Style/Attr,Style/BisectedAttrAccessor,Style/Semicolon,Layout";
    for _ in 0..4 {
        cli(d.path(), &["--only", selected, "--fix", "lib/a.rb"]);
    }
    let fixed = fs::read(d.path().join("lib/a.rb")).unwrap();
    assert_ne!(fixed, source.as_bytes());
    let stable = cli(d.path(), &["--only", selected, "--fix", "lib/a.rb"]);
    assert_eq!(stable.status.code(), Some(0));
    assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), fixed);
}
