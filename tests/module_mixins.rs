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
const FIXED: &str =
    "module Bar; end\nmodule Qux; end\nclass Foo\n  include Qux\n  include Bar\nend\n";
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
fn default_only_except_config_and_detection_only_no_write() {
    let dir = project();
    for (cop, source) in CASES {
        let disable = format!("[cops.\"{cop}\"]\nenabled = false\n");
        let enable = format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n");
        for (config, args, n) in [
            ("", vec!["lib/a.rb"], 1),
            ("", vec!["--only", cop, "lib/a.rb"], 1),
            ("", vec!["--only", "Style", "lib/a.rb"], 1),
            ("", vec!["--except", cop, "lib/a.rb"], 0),
            ("", vec!["--only", cop, "--except", "Style", "lib/a.rb"], 0),
            (disable.as_str(), vec!["lib/a.rb"], 0),
            (disable.as_str(), vec!["--only", cop, "lib/a.rb"], 1),
            (enable.as_str(), vec!["lib/a.rb"], 1),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            count(&unchanged(dir.path(), source.as_bytes(), &args), cop, n);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        count(
            &unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
            ),
            cop,
            1,
        );
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                let s = format!("# {prefix}:disable {target}\n{source}");
                count(
                    &unchanged(
                        dir.path(),
                        s.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                    ),
                    cop,
                    0,
                );
            }
        }
        for config in [
            "exclude = ['lib/**']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            count(
                &unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                ),
                cop,
                0,
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn safe_fixes_are_opt_in_atomic_single_pass_and_preserve_bytes() {
    let dir = project();
    let cop = "Style/MixinGrouping";
    for (before, after) in [
        (SAFE.to_string(), FIXED.to_string()),
        (SAFE.replace('\n', "\r\n"), FIXED.replace('\n', "\r\n")),
        (
            format!("\u{feff}# encoding: UTF-8\n{SAFE}__END__\nraw\0\r\n"),
            format!("\u{feff}# encoding: UTF-8\n{FIXED}__END__\nraw\0\r\n"),
        ),
    ] {
        count(
            &unchanged(dir.path(), before.as_bytes(), &["--only", cop, "lib/a.rb"]),
            cop,
            1,
        );
        let o = cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"]);
        assert_eq!(o.status.code(), Some(0));
        assert_eq!(
            fs::read(dir.path().join("lib/a.rb")).unwrap(),
            after.as_bytes()
        );
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
        );
    }
    for prefix in ["rustocop", "rubocop"] {
        for comment in [
            format!("# {prefix}:disable-next {cop}\n"),
            format!("# {prefix}:disable {cop}\n"),
        ] {
            let before = SAFE.replace("  include", &format!("{comment}  include"));
            count(
                &unchanged(
                    dir.path(),
                    before.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                ),
                cop,
                0,
            );
        }
        let before = SAFE.replace(
            "Bar, Qux",
            &format!("Bar, Qux # {prefix}:disable Style/ModuleFunction"),
        );
        count(
            &unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
            ),
            cop,
            1,
        );
    }
    for before in [
        format!("{SAFE}p __LINE__\n"),
        SAFE.replace("Bar, Qux", "Bar, Qux # retained"),
        SAFE.replace("Bar, Qux", "Bar, 42"),
        format!("{SAFE}module Bar; end\n"),
    ] {
        count(
            &unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
            ),
            cop,
            1,
        );
    }
    // Config disabling/exclusion and explicit except cannot generate edits.
    for config in [
        format!("[cops.\"{cop}\"]\nenabled = false\n"),
        format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.Layout]\nenabled = false\n{config}"),
        )
        .unwrap();
        unchanged(dir.path(), SAFE.as_bytes(), &["--fix", "lib/a.rb"]);
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    unchanged(
        dir.path(),
        SAFE.as_bytes(),
        &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
    );
}
#[test]
fn detection_only_preserves_encoding_bom_crlf_data_and_line_values() {
    let dir = project();
    for (cop, source) in CASES {
        let mut bytes = format!(
            "\u{feff}# encoding: UTF-8\r\n# café\r\n{}p __LINE__\r\n__END__\r\n",
            source.replace('\n', "\r\n")
        )
        .into_bytes();
        bytes.extend_from_slice(b"payload\0\xff\r\n");
        count(
            &unchanged(dir.path(), &bytes, &["--only", cop, "--fix", "lib/a.rb"]),
            cop,
            1,
        );
        let mut legacy = b"# encoding: ISO-8859-1\n# caf\xe9\n".to_vec();
        legacy.extend_from_slice(source.as_bytes());
        legacy.extend_from_slice(b"__END__\n\xff\0\r\n");
        count(
            &unchanged(dir.path(), &legacy, &["--only", cop, "--fix", "lib/a.rb"]),
            cop,
            1,
        );
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

#[test]
fn core_class_constant_shadowing_preserves_exception_atomicity() {
    let dir = project();
    for method in ["include", "extend", "prepend"] {
        // Encoding::Converter is a class, shadowing the earlier root module.
        let before = format!(
            "module Converter; end\nmodule Bar; end\nclass Encoding\n  {method} Converter, Bar\nend\n"
        );
        count(
            &unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", "Style/MixinGrouping", "--fix", "lib/a.rb"],
            ),
            "Style/MixinGrouping",
            1,
        );
        let actual = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
        let receiver = if method == "extend" {
            "Encoding.singleton_class"
        } else {
            "Encoding"
        };
        let observe = format!("p {receiver}.ancestors.include?(Bar)\n");
        let program = |body: &str| {
            format!("begin\n{body}rescue TypeError => e\nputs e.class\nend\n{observe}")
        };
        let Some(original) = run_ruby(&program(&before)) else {
            return;
        };
        assert_eq!(original, b"TypeError\nfalse\n");
        assert_eq!(Some(original), run_ruby(&program(&actual)));
        let split = before.replace(
            &format!("{method} Converter, Bar"),
            &format!("{method} Bar\n  {method} Converter"),
        );
        assert_eq!(
            run_ruby(&program(&split)),
            Some(b"TypeError\ntrue\n".to_vec())
        );
    }
}

#[test]
fn callbacks_overrides_evaluation_and_invalid_arguments_are_not_unsafely_split() {
    let dir = project();
    let cases = [
        // include validates *all* arguments before any callback/ancestor change.
        (
            "module Bar; end\nclass Foo\n  include 1, Bar\nend\n",
            "p Foo.ancestors.include?(Bar)\n",
        ),
        // Callback observes how many arguments have been evaluated before it.
        (
            "$events = []\nmodule Bar; def self.included(base); $events << :callback; end; end\ndef make_module; $events << :argument; Bar; end\nclass Foo\n  include make_module, Bar\nend\n",
            "p $events\n",
        ),
        // Receiver overrides see arity and can return a non-self value.
        (
            "module Bar; end\nmodule Qux; end\nclass Foo\n  def self.include(*args); $count = args.length; :custom; end\n  include Bar, Qux\nend\n",
            "p $count\n",
        ),
        // Later textual argument lookup can change during an earlier callback.
        (
            "module Bar; end\nmodule Qux; def self.included(base); base.const_set(:Bar, 1); end; end\nclass Foo\n  include Bar, Qux\nend\n",
            "p Foo.ancestors.include?(::Bar)\n",
        ),
    ];
    for (before, observe) in cases {
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, before).unwrap();
        count(
            &unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", "Style/MixinGrouping", "--fix", "lib/a.rb"],
            ),
            "Style/MixinGrouping",
            1,
        );
        let actual = fs::read_to_string(path).unwrap();
        let original =
            format!("begin\n{before}rescue StandardError => e\nputs e.class\nend\n{observe}");
        let fixed =
            format!("begin\n{actual}rescue StandardError => e\nputs e.class\nend\n{observe}");
        let Some(a) = run_ruby(&original) else { return };
        assert_eq!(Some(a), run_ruby(&fixed));
        // Independently show the upstream reverse-text rewrite is unsafe here.
        let split = before
            .replace("include 1, Bar", "include Bar\n  include 1")
            .replace(
                "include make_module, Bar",
                "include Bar\n  include make_module",
            )
            .replace("include Bar, Qux", "include Qux\n  include Bar");
        let unsafe_program =
            format!("begin\n{split}rescue StandardError => e\nputs e.class\nend\n{observe}");
        assert_ne!(run_ruby(&original), run_ruby(&unsafe_program), "{original}");
    }
}
