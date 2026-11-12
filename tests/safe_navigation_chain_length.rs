//! chain-length policy and noncorrectable/no-write CLI contract.
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;

const COP: &str = "Style/SafeNavigationChainLength";
const SOURCE: &str = "x&.a&.b&.c\n";

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}

fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    assert!(out.stderr.is_empty() || code == 2, "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
    out
}

#[test]
fn defaults_selectors_config_enabling_and_excludes_work_without_writes() {
    let d = project();
    for args in [
        vec!["a.rb"],
        vec!["--only", COP, "a.rb"],
        vec!["--only", COP, "--fix", "a.rb"],
    ] {
        let out = unchanged(d.path(), SOURCE.as_bytes(), &args, 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
    }
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled = false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 0);
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 1);
    for config in [
        "exclude = [\"*.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"*.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
}

#[test]
fn max_is_fixed_and_unknown_style_settings_are_rejected() {
    let d = project();
    for option in ["max = 3", "Max = 3", "TargetRubyVersion = 2.2"] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 2);
    }
}

#[test]
fn both_directive_prefixes_suppress_restore_and_source_enable_the_cop() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for kind in ["disable", "disable-next"] {
                let source = format!("# {prefix}:{kind} {target}\n{SOURCE}");
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
            let source = format!("{} # {prefix}:disable {target}\n", SOURCE.trim_end());
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
        let source = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        let out = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout)
                .matches(&format!("({COP})"))
                .count(),
            1
        );
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\n{SOURCE}");
        unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--except", COP, "--fix", "a.rb"],
            0,
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn multiline_suppression_uses_the_send_highlight_not_its_block_body() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        let source = format!("x&.a\n &.b\n &.c # {prefix}:disable {COP}\n");
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        let source = format!("x&.a&.b&.c {{\n body # {prefix}:disable {COP}\n}}\n");
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
}

#[test]
fn noncorrectable_sources_keep_encoding_bom_crlf_line_and_data_bytes_and_mtime() {
    let d = project();
    for source in [
        "\u{feff}# encoding: UTF-8\r\n  x&.a('café', __LINE__)&.b&.c\r\np __LINE__\r\n__END__\r\npayload\0ÿ\r\n".as_bytes(),
        b"# encoding: ISO-8859-1\r\nx&.a('caf\xe9')&.b&.c\r\np [__LINE__, DATA.pos]\r\n__END__\r\nraw\xff\r\n",
    ] {
        unchanged(d.path(), source, &["--only", COP, "a.rb"], 1);
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    }
    unchanged(d.path(), b"x&.a&.b\n", &["--only", COP, "--fix", "a.rb"], 0);
    unchanged(d.path(), b"(\n", &["--only", COP, "--fix", "a.rb"], 2);
}

#[test]
fn stdin_selectors_and_fix_rejection_never_write_the_display_path() {
    let d = project();
    let path = d.path().join("a.rb");
    fs::write(&path, b"sentinel\n").unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    for (args, code) in [
        (vec!["--only", COP, "--stdin", "a.rb"], 1),
        (vec!["--only", COP, "--except", COP, "--stdin", "a.rb"], 0),
        (vec!["--only", COP, "--fix", "--stdin", "a.rb"], 2),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Only the expected `--fix --stdin` rejection may exit before reading.
        stdin_support::write_stdin(child.stdin.take().unwrap(), SOURCE.as_bytes(), code == 2)
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(code), "{out:?}");
        if code == 2 {
            assert!(out.stdout.is_empty(), "{out:?}");
            assert!(
                String::from_utf8_lossy(&out.stderr)
                    .contains("the argument '--fix' cannot be used with '--stdin <PATH>'"),
                "{out:?}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), b"sentinel\n");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

fn runtime(source: &str, expected: &str) {
    let d = project();
    for binary in ["ruby", "/opt/ruby34/miniruby"] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let run = || {
            Command::new(binary)
                .current_dir(d.path())
                .arg("a.rb")
                .output()
        };
        let Ok(before) = run() else { continue };
        assert!(before.status.success(), "{binary}: {before:?}");
        assert_eq!(
            String::from_utf8_lossy(&before.stdout),
            expected,
            "{binary}"
        );
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        let after = run().unwrap();
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        assert_eq!(before.status.code(), after.status.code());
    }
}

#[test]
fn nil_short_circuit_false_dispatch_and_receiver_argument_effects_are_preserved() {
    runtime(
        "$events = []\ndef arg; $events << :arg; 1; end\ndef receiver(x); $events << :receiver; x; end\nclass Box\n def a(x); $events << :a; self; end\n def b(x); $events << :b; nil; end\n def c(x); $events << :c; self; end\nend\np receiver(nil)&.a(arg)&.b(arg)&.c(arg)\np $events\n$events.clear\nbegin; receiver(false)&.a(arg)&.b(arg)&.c(arg); rescue NoMethodError; puts :false_error; end\np $events\n$events.clear\np receiver(Box.new)&.a(arg)&.b(arg)&.c(arg)\np $events\n",
        "nil\n[:receiver]\nfalse_error\n[:receiver, :arg]\nnil\n[:receiver, :arg, :a, :arg, :b]\n",
    );
}

#[test]
fn nonlocal_return_and_block_break_control_flow_are_preserved() {
    runtime(
        "$events = []\nclass Box\n def a; $events << :a; self; end\n def b; $events << :b; self; end\n def c; $events << :c; yield; $events << :after; end\nend\ndef f(x)\n x&.a&.b&.c { return :early }\n :tail\nend\np f(nil)\np f(Box.new)\np Box.new&.a&.b&.c { break :broken }\np $events\n",
        ":tail\n:early\n:broken\n[:a, :b, :c, :a, :b, :c]\n",
    );
}

#[test]
fn initialization_custom_methods_mutable_frozen_identity_and_lexical_scope_are_preserved() {
    runtime(
        "$events = []\nmodule M\n class Box\n  def initialize; $events << :init; end\n  def a; $events << :a; self; end\n  def b; $events << :b; self; end\n  def c; $events << :c; self; end\n  def nil?; true; end\n end\n def self.f\n  x = Box.new\n  p x&.a&.b&.c.equal?(x)\n  x.freeze\n  p x&.a&.b&.c.equal?(x)\n  p x.frozen?\n  p x.nil?\n end\nend\nM.f\np $events\n",
        "true\ntrue\ntrue\ntrue\n[:init, :a, :b, :c, :a, :b, :c]\n",
    );
}
