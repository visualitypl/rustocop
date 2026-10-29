//! source-visible indirect dispatch in newline-changing consumers.
use std::{fs, path::Path, process::Command};

const CASES: [(&str, &str, &str, &str); 3] = [
    (
        "Style/EndlessMethod",
        "def f = begin\n  raise(\"boom\")\nend\n",
        "def f\n  begin\n  raise(\"boom\")\nend\nend\n",
        "f",
    ),
    (
        "Style/AmbiguousEndlessMethodDefinition",
        "def f = raise(\"boom\") if true\n",
        "def f\n  raise(\"boom\")\nend if true\n",
        "f",
    ),
    (
        "Style/ClassMethodsDefinitions",
        "class A\n  class << self\n    def one; :one; end\n  end\nend\n",
        "class A\n  def self.one; :one; end\nend\n",
        "A.one(123)",
    ),
];
fn ruby(path: &Path, harness: &str) -> Vec<u8> {
    let out = Command::new("ruby")
        .args(["--disable-gems", "-e", harness])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    assert!(!out.stdout.is_empty());
    out.stdout
}
fn withheld_runtime(cop: &str, source: &str, naive: &str, harness: &str) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(&path, harness);
    // Every controlled counterexample must actually distinguish the naive edit.
    fs::write(&path, naive).unwrap();
    assert_ne!(ruby(&path, harness), before, "{source}");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    for _ in 0..2 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--fix", "case.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}: {source}");
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(ruby(&path, harness), before, "{source}");
    }
}
#[test]
fn indirect_exception_observers_retain_runtime_output_offenses_bytes_and_mtime() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    for (cop, declaration, naive, call) in CASES {
        for selector in ["backtrace", "backtrace_locations", "full_message"] {
            // Isolate unknown/anonymous/ellipsis block forwarding: the source
            // contains no observer selector or conversion to mask these guards.
            for dispatch in [
                "def observe(items, &block); items.flat_map(&block); end\n",
                "def observe(items, &); items.flat_map(&); end\n",
                "def observe(items, ...); items.flat_map(...); end\n",
            ] {
                let suffix = format!(
                    "{dispatch}begin\n  {call}\nrescue => e\n  puts $invoke.call(e)\nend\n"
                );
                let harness =
                    format!("$invoke = ->(e) {{ observe([e], &:{selector}) }}; load ARGV[0]");
                withheld_runtime(
                    cop,
                    &format!("{declaration}{suffix}"),
                    &format!("{naive}{suffix}"),
                    &harness,
                );
            }
            for expression in [
                format!(":{selector}"),
                format!("(:{selector})"),
                format!("((:\"{selector}\"))"),
                format!("(:'{selector}')"),
                format!("(%s({selector}))"),
                format!(":{selector}.to_proc"),
                format!("((:{selector})).to_proc"),
                format!("selector = :{selector}; selector"),
                format!("selector = :{selector}.to_proc; selector"),
            ] {
                let expression = if expression.contains(';') {
                    format!("({expression})")
                } else {
                    expression
                };
                let observer = if selector == "backtrace_locations" {
                    format!("p [e].flat_map(&{expression}).map(&:lineno)")
                } else {
                    format!("puts [e].flat_map(&{expression})")
                };
                let suffix = format!("begin\n  {call}\nrescue => e\n  {observer}\nend\n");
                withheld_runtime(
                    cop,
                    &format!("{declaration}{suffix}"),
                    &format!("{naive}{suffix}"),
                    "load ARGV[0]",
                );
            }
            for (setup, observer) in [
                (
                    format!("class Exception; alias observed {selector}; end\n"),
                    "puts [e].flat_map(&:observed)".to_owned(),
                ),
                (
                    format!("Exception.alias_method(:observed, :{selector})\n"),
                    "puts [e].flat_map(&:observed)".to_owned(),
                ),
                (
                    String::new(),
                    format!("puts e.enum_for(:{selector}).each {{}}"),
                ),
                (
                    String::new(),
                    format!("puts [:{selector}].map(&:to_proc).first.call(e)"),
                ),
                (
                    "def observe(items, &block); items.flat_map(&block); end\n".to_owned(),
                    format!("puts observe([e], &(:{selector}))"),
                ),
                (
                    String::new(),
                    format!("puts e.to_enum(:{selector}).each {{}}"),
                ),
            ] {
                // Backtrace locations' default #to_s contains the filename/line.
                let suffix = format!("{setup}begin\n  {call}\nrescue => e\n  {observer}\nend\n");
                withheld_runtime(
                    cop,
                    &format!("{declaration}{suffix}"),
                    &format!("{naive}{suffix}"),
                    "load ARGV[0]",
                );
            }
        }
    }
}
#[test]
fn compound_assignment_proc_getters_preserve_observed_runtime_locations() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    for (cop, declaration, naive, call) in CASES {
        for selector in ["backtrace", "backtrace_locations", "full_message"] {
            let suffix = format!(
                "begin\n  {call}\nrescue => e\n  puts (:{selector}.to_proc ||= nil).call(e)\nend\n"
            );
            withheld_runtime(
                cop,
                &format!("{declaration}{suffix}"),
                &format!("{naive}{suffix}"),
                "load ARGV[0]",
            );
        }
        let suffix = "def later; :later; end\np (:source_location.to_proc ||= nil).call($lookup.call(:later))\n";
        withheld_runtime(
            cop,
            &format!("{declaration}{suffix}"),
            &format!("{naive}{suffix}"),
            "$lookup = ->(name) { method(name) }; load ARGV[0]",
        );
    }
}

#[test]
fn harmless_static_symbol_dispatch_keeps_canonical_runtime_fixes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for (cop, declaration, expected, _) in CASES {
        let suffix =
            "p [:backtrace, :full_message, :source_location].map(&((:\"to_s\").to_proc))\n";
        fs::write(&path, format!("{declaration}{suffix}")).unwrap();
        let before = ruby(&path, "load ARGV[0]");
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--fix", "case.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("{expected}{suffix}")
        );
        assert_eq!(ruby(&path, "load ARGV[0]"), before);
    }
}
#[test]
fn unsafe_newline_edits_do_not_block_independent_style_layout_corrections() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    let source = "class A\n  def A.two arg; arg; end  \n  class << self\n    def one; :one; end\n  end\nend\ndef f = true if true\ndef h = [\n  1]\np errors.flat_map(&((:backtrace_locations)))\n";
    let expected = source.replace("def A.two arg; arg; end  ", "def self.two(arg); arg; end");
    fs::write(&path, source).unwrap();
    let only = "Style/ClassMethods,Style/ClassMethodsDefinitions,Style/AmbiguousEndlessMethodDefinition,Style/EndlessMethod,Style/MethodDefParentheses,Layout/TrailingWhitespace";
    for _ in 0..2 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "case.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let report = String::from_utf8_lossy(&out.stdout);
        assert!(report.contains("Style/ClassMethodsDefinitions"));
        assert!(report.contains("Style/AmbiguousEndlessMethodDefinition"));
        assert!(report.contains("Style/EndlessMethod"));
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
}
#[test]
fn later_source_location_observed_through_indirect_dispatch_is_preserved() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    for (cop, declaration, naive, _) in CASES {
        for expression in [
            "(:source_location)",
            "((:\"source_location\"))",
            ":source_location.to_proc",
            "selector = :source_location; selector",
        ] {
            // Acquisition alone is external; the actual source_location dispatch
            // is in the linted file and must independently prevent the edit.
            let suffix =
                format!("def later; :later; end\np [$lookup.call(:later)].map(&({expression}))\n");
            withheld_runtime(
                cop,
                &format!("{declaration}{suffix}"),
                &format!("{naive}{suffix}"),
                "$lookup = ->(name) { method(name) }; load ARGV[0]",
            );
        }
    }
}
