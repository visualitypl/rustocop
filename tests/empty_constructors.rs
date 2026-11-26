//! real CLI policy, pinned corrections and controlled Ruby witnesses.
//! Pinned fixtures port RuboCop specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use crate::support::cli::project;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;
const LITERAL: &str = "Style/EmptyLiteral";
const CLASS: &str = "Style/EmptyClassDefinition";
const COPS: [(&str, &str); 2] = [
    (LITERAL, "Array.new\n"),
    (CLASS, "FooError = Class.new(StandardError)\n"),
];
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
        policy::{DEFAULT_RUBY_VERSION, RubyVersion},
    };
    assert_eq!(DEFAULT_RUBY_VERSION, RubyVersion(3, 4));
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

fn runtimes() -> Vec<String> {
    let mut v = Vec::new();
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        v.push("ruby".into());
    }
    if let Ok(native) = crate::support::runtime::ruby()
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
