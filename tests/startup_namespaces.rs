//! default startup namespaces are not fresh, without project preloads.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

// Each direct consumer of core_namespace, including all three accessor cops.
// The Dir case exercises LocalConstructors in the shared source-observer proof.
const CASES: &[(&str, &str, &str)] = &[
    (
        "Style/MixinGrouping",
        "module One; end\nmodule Two; end\nKEYWORD NAME\n  include One, Two\nend\n",
        "module One; end\nmodule Two; end\nKEYWORD NAME\n  include Two\n  include One\nend\n",
    ),
    (
        "Style/AccessorGrouping",
        "KEYWORD NAME\n  attr_reader :one\n  attr_reader :two\nend\n",
        "KEYWORD NAME\n  attr_reader :one, :two\nend\n",
    ),
    (
        "Style/Attr",
        "KEYWORD NAME\n  attr :one\nend\n",
        "KEYWORD NAME\n  attr_reader :one\nend\n",
    ),
    (
        "Style/BisectedAttrAccessor",
        "KEYWORD NAME\n  attr_reader :one\n  attr_writer :one\nend\n",
        "KEYWORD NAME\n  attr_accessor :one\nend\n",
    ),
    (
        "Style/Alias",
        "KEYWORD NAME\n  def one; end\n  alias_method :two, :one\n  def three; end\nend\n",
        "KEYWORD NAME\n  def one; end\n  alias two one\n  def three; end\nend\n",
    ),
    (
        "Style/TrivialAccessors",
        "class NAME\n  def one; @one; end\nend\n",
        "class NAME\n  attr_reader :one\nend\n",
    ),
    (
        "Style/Dir",
        "class NAME\n  def run; File.dirname(File.realpath(__FILE__)); end\nend\np NAME.new.run\n",
        "class NAME\n  def run; __dir__; end\nend\np NAME.new.run\n",
    ),
    (
        "Style/EmptyClassDefinition",
        "NAME = Class.new(StandardError)\n",
        "class NAME < StandardError\nend\n",
    ),
];

fn source(template: &str, name: &str) -> String {
    template.replace("NAME", name).replace(
        "KEYWORD",
        if name == "SyntaxSuggest" {
            "module"
        } else {
            "class"
        },
    )
}

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}

fn cli(dir: &Path, cop: &str, fix: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    command.current_dir(dir).args(["--only", cop]);
    if fix {
        command.arg("--fix");
    }
    let out = command.arg("a.rb").output().unwrap();
    assert!(out.stderr.is_empty(), "{cop}: {out:?}");
    out
}

fn no_write(dir: &Path, cop: &str, source: &str) {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    for fix in [false, true] {
        let out = cli(dir, cop, fix);
        assert_eq!(out.status.code(), Some(1), "{cop}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        assert_eq!(fs::read_to_string(&path).unwrap(), source, "{cop}");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn all_startup_freshness_consumers_keep_diagnostics_bytes_and_mtime() {
    let d = project();
    for &(cop, before, _) in CASES {
        for name in ["SyntaxSuggest", "Monitor"] {
            no_write(d.path(), cop, &source(before, name));
        }
    }
}

#[test]
fn genuinely_fresh_counterparts_keep_their_canonical_corrections() {
    let d = project();
    for &(cop, before, after) in CASES {
        let before = source(before, "FreshStartupAudit");
        let after = source(after, "FreshStartupAudit");
        fs::write(d.path().join("a.rb"), &before).unwrap();
        let out = cli(d.path(), cop, true);
        // Style/Dir's target names `__FILE__`, a listed observer
        // that withholds the edit; the finding stays.
        if before.contains("__FILE__") {
            assert_eq!(out.status.code(), Some(1), "{cop}: {out:?}");
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), before);
            continue;
        }
        assert_eq!(out.status.code(), Some(0), "{cop}: {out:?}");
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
    }
}

fn runtimes() -> Vec<String> {
    let mut runtimes = Vec::new();
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        runtimes.push("ruby".into());
    }
    if let Ok(native) = crate::support::runtime::ruby()
        && !runtimes.contains(&native)
    {
        runtimes.push(native);
    }
    runtimes
}

fn ruby(dir: &Path, runtime: &str, program: &str) -> Output {
    fs::write(dir.join("witness.rb"), program).unwrap();
    let out = Command::new(runtime)
        .current_dir(dir)
        // Use ordinary startup, not --disable-gems or an invented preload.
        .env_remove("RUBYOPT")
        .env_remove("RUBYLIB")
        .env_remove("GEM_HOME")
        .env_remove("GEM_PATH")
        .env_remove("BUNDLE_GEMFILE")
        .args(["-w", "witness.rb"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{runtime}: {out:?}");
    out
}

#[test]
fn controlled_ruby_covers_each_consumer_without_inventing_runtime_differences() {
    let d = project();
    for runtime in runtimes() {
        for name in ["SyntaxSuggest", "Monitor"] {
            let present = ruby(
                d.path(),
                &runtime,
                &format!("p Object.const_defined?(:{name}, false)\n"),
            );
            if present.stdout == b"false\n" {
                continue; // Native miniruby, for example, does not load RubyGems/Monitor.
            }
            for &(cop, before, naive) in CASES {
                let before = source(before, name);
                no_write(d.path(), cop, &before);
                // Keep the observation outside the linted closed declaration file.
                // SyntaxSuggest is a module: class-only consumers already raise;
                // its module consumers and Monitor reopen existing namespaces.
                let observe = format!(
                    "p [{name}.name, {name}.class.name]\nrescue StandardError => e\nputs e.class\nend\n"
                );
                let original = ruby(d.path(), &runtime, &format!("begin\n{before}{observe}"));
                let actual = fs::read_to_string(d.path().join("a.rb")).unwrap();
                let retained = ruby(d.path(), &runtime, &format!("begin\n{actual}{observe}"));
                assert_eq!(original.stdout, retained.stdout, "{runtime}: {cop}");
                assert_eq!(original.stderr, retained.stderr, "{runtime}: {cop}");
                assert!(!original.stdout.is_empty());
                if cop != "Style/EmptyClassDefinition" {
                    // These benign examples witness an inaccurate freshness
                    // proof, not a runtime change. The concrete unsafe mixin
                    // and constant-replacement cases have separate witnesses.
                    let naive = source(naive, name);
                    let changed = ruby(d.path(), &runtime, &format!("begin\n{naive}{observe}"));
                    assert_eq!(original.stdout, changed.stdout, "{runtime}: {cop}");
                    assert_eq!(original.stderr, changed.stderr, "{runtime}: {cop}");
                }
            }
        }
    }
}

#[test]
fn startup_module_nested_constant_witness_preserves_mixin_exception_atomicity() {
    let d = project();
    // Ordinary host startup defines SyntaxSuggest::MiniStringIO (a class).
    // No user callback, project preload or monkey patch is necessary.
    let before = "module MiniStringIO; end\nmodule Other; end\nmodule SyntaxSuggest\n  include MiniStringIO, Other\nend\n";
    let naive = before.replace(
        "include MiniStringIO, Other",
        "include Other\n  include MiniStringIO",
    );
    no_write(d.path(), "Style/MixinGrouping", before);
    for runtime in runtimes() {
        let present = ruby(
            d.path(),
            &runtime,
            "p defined?(SyntaxSuggest::MiniStringIO)\n",
        );
        if present.stdout == b"nil\n" {
            continue;
        }
        let observe = "rescue TypeError\n  p SyntaxSuggest.ancestors.include?(Other)\nend\n";
        let original = ruby(d.path(), &runtime, &format!("begin\n{before}{observe}"));
        let changed = ruby(d.path(), &runtime, &format!("begin\n{naive}{observe}"));
        assert_eq!(original.stdout, b"false\n", "{runtime}: {original:?}");
        assert_eq!(changed.stdout, b"true\n", "{runtime}: {changed:?}");
    }
}

#[test]
fn replacing_startup_constants_and_reopening_them_are_not_equivalent() {
    let d = project();
    for runtime in runtimes() {
        for name in ["SyntaxSuggest", "Monitor"] {
            let present = ruby(d.path(), &runtime, &format!("p defined?({name})\n"));
            if present.stdout == b"nil\n" {
                continue;
            }
            let before = format!("{name} = Class.new(StandardError)\n");
            no_write(d.path(), "Style/EmptyClassDefinition", &before);
            let observe =
                format!("p {name}.superclass\nrescue TypeError\nputs :reopening_failed\nend\n");
            let original = ruby(d.path(), &runtime, &format!("begin\n{before}{observe}"));
            let changed = ruby(
                d.path(),
                &runtime,
                &format!("begin\nclass {name} < StandardError\nend\n{observe}"),
            );
            assert_eq!(original.stdout, b"StandardError\n");
            assert_eq!(changed.stdout, b"reopening_failed\n");
        }
    }
}
