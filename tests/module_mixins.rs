//! fixed policies, conservative fixes and controlled Ruby semantics.
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

const CASES: [(&str, &str); 3] = [
    ("Style/ModuleFunction", "module Test\n  extend self\nend\n"),
    ("Style/MixinUsage", "include M\n"),
    ("Style/MixinGrouping", "class Test\n  include M, N\nend\n"),
];
const SAFE: &str = "module Bar; end\nmodule Qux; end\nclass Foo\n  include Bar, Qux\nend\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
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
        text.lines().filter(|line| line.contains(cop)).count(),
        expected,
        "{text}"
    );
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str]) -> Output {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(dir, args);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    o
}
#[test]
fn registry_pipeline_fixed_defaults_and_no_version_or_path_gates() {
    let registry = Registry::new();
    let config = ResolvedConfig::resolve(RawConfig::default(), None, &registry).unwrap();
    for (cop, source) in CASES {
        let entries: Vec<_> = registry.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].default_enabled());
        assert_eq!(entries[0].safe_fixes(), cop == "Style/MixinGrouping");
        assert_eq!(entries[0].ruby_versions(), Default::default());
        assert!(entries[0].path_policy().is_none());
        let active = select_active(&registry, &config, &[cop.into()], &[]);
        for path in ["lib/a.rb", "spec/a.rb", "test/a.rb"] {
            let report = lint_one(
                Path::new(path),
                Path::new(path),
                Some(source.as_bytes()),
                &active,
                &config,
            );
            assert!(report.errors.is_empty() && report.internal_errors.is_empty());
            assert_eq!(report.result.offenses.len(), 1);
            assert!(report.result.offenses[0].correction.is_none());
        }
    }
}

#[test]
fn canonical_fixes_compose_with_semicolon_and_layout_without_oscillation() {
    let dir = project();
    fs::write(dir.path().join("lib/a.rb"), SAFE).unwrap();
    let args = [
        "--only",
        "Style/MixinGrouping,Style/Semicolon,Layout",
        "--fix",
        "lib/a.rb",
    ];
    cli(dir.path(), &args);
    let first = fs::read(dir.path().join("lib/a.rb")).unwrap();
    assert!(String::from_utf8_lossy(&first).contains("include Qux\n  include Bar"));
    // At most two explicit passes for this curated composition, no auto loop.
    cli(dir.path(), &args);
    let second = fs::read(dir.path().join("lib/a.rb")).unwrap();
    cli(dir.path(), &args);
    assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), second);
}

#[test]
fn stdin_is_no_write_and_accepts_all_assigned_selectors() {
    let dir = project();
    for (cop, source) in CASES {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
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
            .write_all(source.as_bytes())
            .unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(1));
        count(&o, cop, 1);
        assert!(!dir.path().join("lib/a.rb").exists());
    }
}
fn ruby(source: &str) -> Option<Output> {
    let child = match Command::new("ruby")
        .args(["--disable-gems", "-W0", "-e", source])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("{error}"),
    };
    Some(child.wait_with_output().unwrap())
}
fn run_ruby(source: &str) -> Option<Vec<u8>> {
    let o = ruby(source)?;
    assert!(o.status.success(), "{source}: {o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    Some(o.stdout)
}
#[test]
fn corrected_core_mixins_preserve_observable_ancestors_nesting_return_and_lookup() {
    let dir = project();
    for method in ["include", "extend", "prepend"] {
        for kind in ["class", "module"] {
            let before = SAFE
                .replace("include", method)
                .replace("class Foo", &format!("{kind} Foo"));
            fs::write(dir.path().join("lib/a.rb"), &before).unwrap();
            cli(
                dir.path(),
                &["--only", "Style/MixinGrouping", "--fix", "lib/a.rb"],
            );
            let after = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
            assert_ne!(after, before);
            let observation = "p Foo.ancestors.map(&:name); p Foo.singleton_class.ancestors.map(&:name); p Module.nesting; p [Bar.name, Qux.name]; p value.equal?(Foo)\n";
            let original = format!("value = begin\n{before}end\n{observation}");
            let fixed = format!("value = begin\n{after}end\n{observation}");
            let Some(a) = run_ruby(&original) else { return };
            assert_eq!(Some(a), run_ruby(&fixed));
        }
    }
}
#[test]
fn unsafe_module_function_and_top_level_usage_keep_binding_visibility_and_object_effects() {
    let dir = project();
    let module_source = "module Outer\n  VALUE = :lexical\n  module Inner\n    extend self\n    def read; VALUE; end\n  end\nend\nmodule Outer::Inner\n  def read; :redefined; end\nend\n";
    let observe = "p Outer::Inner.read; p Outer::Inner.private_instance_methods(false).include?(:read); p Outer::Inner.instance_methods(false).include?(:read); p Outer::Inner.singleton_class.ancestors.map(&:name)\n";
    unchanged(
        dir.path(),
        module_source.as_bytes(),
        &["--only", "Style/ModuleFunction", "--fix", "lib/a.rb"],
    );
    let Some(original) = run_ruby(&format!("{module_source}{observe}")) else {
        return;
    };
    assert_ne!(
        Some(original),
        run_ruby(&format!(
            "{}{observe}",
            module_source.replace("extend self", "module_function")
        ))
    );
    let usage = "module M; def marker; :mixed; end; end\ninclude M\n";
    let observe = "p Object.ancestors.include?(M); p Object.new.marker\n";
    unchanged(
        dir.path(),
        usage.as_bytes(),
        &["--only", "Style/MixinUsage", "--fix", "lib/a.rb"],
    );
    assert_eq!(
        run_ruby(&format!("{usage}{observe}")),
        Some(b"true\n:mixed\n".to_vec())
    );
}
