//! real CLI policy, pinned corrections and controlled Ruby witnesses.
//! Pinned fixtures port RuboCop specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

#[path = "support/stdin.rs"]
mod stdin_support;
const LITERAL: &str = "Style/EmptyLiteral";
const CLASS: &str = "Style/EmptyClassDefinition";
const COPS: [(&str, &str); 2] = [
    (LITERAL, "Array.new\n"),
    (CLASS, "FooError = Class.new(StandardError)\n"),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    assert!(o.stderr.is_empty() || code == 2, "{o:?}");
    o
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
    o
}
fn fixed(dir: &Path, cop: &str, source: &str, after: &str) {
    unchanged(dir, source.as_bytes(), &["--only", cop, "a.rb"], 1);
    cli(dir, &["--only", cop, "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read_to_string(dir.join("a.rb")).unwrap(),
        after,
        "{source}"
    );
    unchanged(dir, after.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
}
#[test]
fn registry_defaults_safe_capabilities_and_fixed_ruby34() {
    use rustocop::cops::{
        Registry,
        policy::{FIXED_RUBY_VERSION, RubyVersion},
    };
    assert_eq!(FIXED_RUBY_VERSION, RubyVersion(3, 4));
    let registry = Registry::new();
    for (name, _) in COPS {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled());
        assert!(cop.safe_fixes());
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
    }
}
#[test]
fn defaults_selectors_config_excludes_and_fixed_setting_rejections_never_write() {
    let d = project();
    for (cop, source) in COPS {
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled = false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "exclude = [\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for setting in [
            "EnforcedStyle = \"class_new\"",
            "AllowedParentClasses = []",
            "TargetRubyVersion = 4.0",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{setting}\n"),
            )
            .unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn both_directive_prefixes_all_scopes_clean_and_error_files_preserve_bytes_and_mtime() {
    let d = project();
    for (cop, source) in COPS {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
            let s = format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            let o = unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (s, code) in [("[]\n", 0), ("(\n", 2)] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
}
#[test]
fn frozen_comment_dependency_uses_configuration_not_selection_and_never_changes_string_mutability()
{
    let d = project();
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Style/FrozenStringLiteralComment\"]\nenabled = true\n",
    )
    .unwrap();
    unchanged(
        d.path(),
        b"String.new\n",
        &["--only", LITERAL, "--fix", "a.rb"],
        0,
    );
    unchanged(
        d.path(),
        b"# frozen_string_literal: false\nString.new\n",
        &["--only", LITERAL, "--fix", "a.rb"],
        1,
    );
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    unchanged(
        d.path(),
        b"String.new\n",
        &["--only", LITERAL, "--fix", "a.rb"],
        1,
    );
    unchanged(
        d.path(),
        b"# frozen_string_literal: true\nString.new\n",
        &["--only", LITERAL, "--fix", "a.rb"],
        0,
    );
    fixed(
        d.path(),
        LITERAL,
        "# encoding: binary\n# frozen_string_literal: false\nString.new\n",
        "# encoding: binary\n# frozen_string_literal: false\n\"\"\n",
    );
}
#[test]
fn all_pinned_detection_and_correction_calls_use_the_real_cli() {
    let d = project();
    let mut totals = [0, 0, 0];
    for (cop, fixture) in [
        (LITERAL, include_str!("fixtures/empty_literal_pinned.json")),
        (
            CLASS,
            include_str!("fixtures/empty_class_definition_pinned.json"),
        ),
    ] {
        let cases: serde_json::Value = serde_json::from_str(fixture).unwrap();
        for e in cases.as_array().unwrap() {
            totals[0] += 1;
            fs::write(
                d.path().join("rustocop.toml"),
                if e["frozen_comment"] == true {
                    "[cops.\"Style/FrozenStringLiteralComment\"]\nenabled = true\n"
                } else {
                    ""
                },
            )
            .unwrap();
            let s = e["source"].as_str().unwrap();
            let has_offense = !e["positions"].as_array().unwrap().is_empty();
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "a.rb"],
                i32::from(has_offense),
            );
            if let Some(after) = e["correction"].as_str() {
                let line = e["spec_line"].as_u64().unwrap();
                let safe = if cop == LITERAL {
                    matches!(line, 7 | 66 | 205 | 217)
                } else {
                    matches!(line, 8 | 38 | 493)
                };
                if safe {
                    fixed(d.path(), cop, s, after);
                    totals[1] += 1;
                } else {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
                    totals[2] += 1;
                }
            }
        }
    }
    assert_eq!(totals, [84, 16, 18]);
}
#[test]
fn stdin_is_diagnostic_only_and_rejects_fix() {
    let d = project();
    for (cop, s) in COPS {
        for fix in [false, true] {
            let mut args = vec!["--only", cop, "--stdin", "a.rb"];
            if fix {
                args.push("--fix");
            }
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            // Only the expected `--stdin --fix` rejection may exit before reading.
            stdin_support::write_stdin(child.stdin.take().unwrap(), s.as_bytes(), fix).unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }), "{o:?}");
            if fix {
                assert!(o.stdout.is_empty(), "{o:?}");
                assert!(
                    String::from_utf8_lossy(&o.stderr)
                        .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                    "{o:?}"
                );
            } else {
                assert!(String::from_utf8_lossy(&o.stdout).contains("a.rb:1:1:"));
            }
            assert!(!d.path().join("a.rb").exists());
        }
    }
}
#[test]
fn legacy_encoded_and_unicode_class_targets_report_without_reencoding_identifier_bytes() {
    let d = project();
    for source in [
        b"# encoding: ISO-8859-1\n\xc9Error = Class.new(StandardError)\n".as_slice(),
        "# encoding: utf-8\nÉError = Class.new(StandardError)\n".as_bytes(),
        b"# encoding: ISO-8859-1\nFoo::\xc9Error = Class.new(StandardError)\n",
    ] {
        let o = unchanged(d.path(), source, &["--only", CLASS, "--fix", "a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains("a.rb:2:1:"));
    }
}

#[test]
fn byte_envelopes_and_mixed_style_layout_corrections_are_stable() {
    let d = project();
    fixed(
        d.path(),
        LITERAL,
        "\u{feff}Array.new\r\nHash.new\r\n__LINE__\r\n__END__\r\npayload\0\r\n",
        "\u{feff}[]\r\n{}\r\n__LINE__\r\n__END__\r\npayload\0\r\n",
    );
    fixed(
        d.path(),
        CLASS,
        "\u{feff}FooError = Class.new(StandardError)\r\n__END__\r\npayload\0\r\n",
        "\u{feff}class FooError < StandardError\r\nend\r\n__END__\r\npayload\0\r\n",
    );
    fs::write(d.path().join("a.rb"), "test = Array.new([]);  \n").unwrap();
    let only = "Style/EmptyLiteral,Style/RedundantArrayConstructor,Style/Semicolon,Layout/TrailingWhitespace";
    for _ in 0..3 {
        let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", only, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert!(matches!(o.status.code(), Some(0 | 1)));
    }
    let after = fs::read(d.path().join("a.rb")).unwrap();
    assert_eq!(after, b"test = []\n");
    unchanged(d.path(), &after, &["--only", only, "--fix", "a.rb"], 0);
}
fn runtimes() -> Vec<String> {
    let mut v = Vec::new();
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        v.push("ruby".into());
    }
    if let Ok(native) = std::env::var("RUSTOCOP_RUBY34")
        && !v.contains(&native)
    {
        v.push(native);
    }
    v
}
fn ruby(dir: &Path, runtime: &str, source: &str) -> Output {
    fs::write(dir.join("witness.rb"), source).unwrap();
    Command::new(runtime)
        .current_dir(dir)
        .args(["-W:deprecated", "witness.rb"])
        .output()
        .unwrap()
}
#[test]
fn safe_literals_preserve_contents_mutability_identity_encoding_and_argument_evaluation() {
    let d = project();
    for (source, prefix, suffix) in [
        (
            "a = Array.new\nb = Array.new([])\nh = Hash.new\nk = Hash([])\n",
            "",
            "a << nil; b << false; h[:a] = a; k[:b] = b; p [a,b,h,k,a.equal?(b),h.default,h.default_proc,a.frozen?,h.frozen?]\n",
        ),
        (
            "consume Hash.new, false, nil\n",
            "def consume(h,a,b); p [h.class,h,a,b]; h[:x] = 1; p h; end\n",
            "",
        ),
        (
            "# encoding: binary\n# frozen_string_literal: false\ns = String.new\n",
            "",
            "p [s.encoding.name,s.frozen?,s.bytes]; s << 'x'; p s\n",
        ),
    ] {
        fixed(
            d.path(),
            LITERAL,
            source,
            if source.starts_with("a =") {
                "a = []\nb = []\nh = {}\nk = {}\n"
            } else if source.starts_with("consume") {
                "consume({}, false, nil)\n"
            } else {
                "# encoding: binary\n# frozen_string_literal: false\ns = \"\"\n"
            },
        );
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        for runtime in runtimes() {
            let before = ruby(d.path(), &runtime, &format!("{prefix}{source}{suffix}"));
            let fixed = ruby(d.path(), &runtime, &format!("{prefix}{after}{suffix}"));
            assert!(before.status.success(), "{before:?}");
            assert_eq!(
                (before.status.code(), before.stdout, before.stderr),
                (fixed.status.code(), fixed.stdout, fixed.stderr),
                "{runtime}: {source}"
            );
        }
    }
}
#[test]
fn repeated_frozen_directives_and_command_separators_preserve_runtime_behavior() {
    let d = project();
    for (source, after, suffix) in [
        (
            "# encoding: binary\n# frozen_string_literal: false\n# frozen_string_literal: true\ns = String.new\n",
            "# encoding: binary\n# frozen_string_literal: false\n# frozen_string_literal: true\ns = String.new\n",
            "s << 'x'; p [s, s.frozen?, s.encoding.name]\n",
        ),
        (
            "# encoding: binary\n# frozen_string_literal: true\n# frozen_string_literal: false\ns = String.new\n",
            "# encoding: binary\n# frozen_string_literal: true\n# frozen_string_literal: false\ns = \"\"\n",
            "s << 'x'; p [s, s.frozen?, s.encoding.name]\n",
        ),
        ("consume  Hash.new, 1\n", "consume({}, 1)\n", ""),
        ("consume\t\tHash.new, 1\n", "consume({}, 1)\n", ""),
        ("consume \t Hash.new, 1\n", "consume({}, 1)\n", ""),
    ] {
        if source == after {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", LITERAL, "--fix", "a.rb"],
                0,
            );
        } else {
            fixed(d.path(), LITERAL, source, after);
        }
        let prefix = if source.starts_with("consume") {
            "def consume(h, n); h[:n] = n; p [h, n]; end\n"
        } else {
            ""
        };
        for runtime in runtimes() {
            let before = ruby(d.path(), &runtime, &format!("{prefix}{source}{suffix}"));
            let corrected = ruby(d.path(), &runtime, &format!("{prefix}{after}{suffix}"));
            assert!(before.status.success(), "{before:?}");
            assert_eq!(
                (before.status.code(), before.stdout, before.stderr),
                (corrected.status.code(), corrected.stdout, corrected.stderr),
                "{runtime}: {source}"
            );
            if source == after {
                let naive = ruby(
                    d.path(),
                    &runtime,
                    &format!("{}{suffix}", source.replace("String.new", "\"\"")),
                );
                assert!(!naive.status.success(), "{runtime}: {naive:?}");
                assert!(String::from_utf8_lossy(&naive.stderr).contains("FrozenError"));
            }
        }
    }
}
#[test]
fn safe_class_definitions_preserve_fresh_names_parents_initialization_and_scope() {
    let d = project();
    for (source, after, suffix) in [
        (
            "FooError = Class.new(StandardError)\n",
            "class FooError < StandardError\nend\n",
            "p [FooError.name,FooError.superclass.name,FooError.new('message').message]\n",
        ),
        (
            "module Foo\n  BarError = Class.new(StandardError)\nend\n",
            "module Foo\n  class BarError < StandardError\n  end\nend\n",
            "p [Foo::BarError.name,Foo::BarError.superclass.name,Foo::BarError.new('message').message]\n",
        ),
    ] {
        fixed(d.path(), CLASS, source, after);
        for runtime in runtimes() {
            let before = ruby(d.path(), &runtime, &format!("{source}{suffix}"));
            let after = ruby(d.path(), &runtime, &format!("{after}{suffix}"));
            assert!(before.status.success(), "{before:?}");
            assert_eq!(
                (before.status.code(), before.stdout, before.stderr),
                (after.status.code(), after.stdout, after.stderr)
            );
        }
    }
}
#[test]
fn unsafe_constructor_and_class_rewrites_have_concrete_runtime_counterexamples() {
    let d = project();
    for (cop, source, naive) in [
        (
            LITERAL,
            "s = String.new\np s.encoding.name\n",
            "s = \"\"\np s.encoding.name\n",
        ),
        (
            LITERAL,
            "class Array; def self.new; puts :allocated; [false]; end; end\np Array.new\n",
            "class Array; def self.new; puts :allocated; [false]; end; end\np []\n",
        ),
        (
            LITERAL,
            "module M; Array = nil; p Array&.new; end\n",
            "module M; Array = nil; p []; end\n",
        ),
        (
            LITERAL,
            "h = Hash.new { |h,k| h[k] = false }; p h[:x]\n",
            "h = {}; p h[:x]\n",
        ),
        (
            CLASS,
            "class Base; def self.inherited(c); p c.name; end; end\nFooError = Class.new(Base)\n",
            "class Base; def self.inherited(c); p c.name; end; end\nclass FooError < Base\nend\n",
        ),
        (
            CLASS,
            "class FooError < StandardError; def custom; true; end; end\nFooError = Class.new(StandardError)\np FooError.new.respond_to?(:custom)\n",
            "class FooError < StandardError; def custom; true; end; end\nclass FooError < StandardError\nend\np FooError.new.respond_to?(:custom)\n",
        ),
        (
            CLASS,
            "module M; Class = Object.new; def Class.new(x); p :factory; nil; end; Foo = Class.new(StandardError); p Foo; end\n",
            "module M; Class = Object.new; def Class.new(x); p :factory; nil; end; class Foo < StandardError; end; p Foo; end\n",
        ),
    ] {
        // No-offense lookalikes are just as important as withheld offenses.
        fs::write(d.path().join("a.rb"), source).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        for runtime in runtimes() {
            let before = ruby(d.path(), &runtime, source);
            let changed = ruby(d.path(), &runtime, naive);
            assert!(before.status.success(), "{before:?}");
            assert!(changed.status.success(), "{changed:?}");
            assert_ne!(before.stdout, changed.stdout, "{runtime}: {source}");
        }
    }
}
#[test]
fn startup_constants_are_not_fresh_class_names_even_without_source_reopening() {
    let d = project();
    let source = "SyntaxSuggest = Class.new(StandardError)\n";
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", CLASS, "--fix", "a.rb"],
        1,
    );
    for runtime in runtimes() {
        let startup = ruby(
            d.path(),
            &runtime,
            "p Object.const_defined?(:SyntaxSuggest, false)\n",
        );
        assert!(startup.status.success(), "{runtime}: {startup:?}");
        assert!(startup.stderr.is_empty(), "{runtime}: {startup:?}");
        let present = match startup.stdout.as_slice() {
            b"true\n" => true,
            b"false\n" => false,
            _ => panic!("unexpected startup fact: {runtime}: {startup:?}"),
        };
        let naive_source = "class SyntaxSuggest < StandardError\nend\n";
        let before = ruby(d.path(), &runtime, source);
        let naive = ruby(d.path(), &runtime, naive_source);
        assert!(before.status.success(), "{runtime}: {before:?}");
        assert_eq!(naive.status.success(), !present, "{runtime}: {naive:?}");
        if present {
            assert!(String::from_utf8_lossy(&naive.stderr).contains("TypeError"));
        }

        // --disable-gems native startup can omit SyntaxSuggest. Still require
        // the real stdlib module and prove the unsafe rewrite, rather than
        // skipping the witness or manufacturing a module with the same name.
        let preload = if present {
            ""
        } else {
            "require 'syntax_suggest/core_ext'\n"
        };
        let premise = format!(
            "{preload}raise 'expected stdlib module' unless SyntaxSuggest.instance_of?(Module)\n"
        );
        let before = ruby(d.path(), &runtime, &format!("{premise}{source}"));
        let naive = ruby(d.path(), &runtime, &format!("{premise}{naive_source}"));
        assert!(before.status.success(), "{runtime}: {before:?}");
        assert!(!naive.status.success(), "{runtime}: {naive:?}");
        assert!(String::from_utf8_lossy(&naive.stderr).contains("TypeError"));
    }
}

#[test]
fn native_ruby34_chilled_literal_mutation_is_not_equivalent_to_string_new() {
    let Ok(runtime) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = project();
    let source = "# encoding: binary\ns = String.new\ns << 'x'\np s\n";
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", LITERAL, "--fix", "a.rb"],
        1,
    );
    let before = ruby(d.path(), &runtime, source);
    let after = ruby(
        d.path(),
        &runtime,
        "# encoding: binary\ns = \"\"\ns << 'x'\np s\n",
    );
    assert!(before.status.success() && after.status.success());
    assert_eq!(before.stdout, after.stdout);
    assert!(before.stderr.is_empty());
    assert!(
        !after.stderr.is_empty(),
        "native Ruby 3.4 must expose chilled mutation: {after:?}"
    );
}
