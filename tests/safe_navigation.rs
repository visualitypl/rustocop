//! unsafe safe-navigation suggestions never rewrite Ruby evaluation.
//! Pinned specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/SafeNavigation";
const SOURCE: &str = "foo && foo.bar\n";
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
fn default_selectors_config_excludes_and_detection_only_fix_are_real_cli_behavior() {
    let d = project();
    let out = unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
    unchanged(d.path(), SOURCE.as_bytes(), &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
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
        let out = unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 0);
        assert!(!String::from_utf8_lossy(&out.stdout).contains(COP));
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
    let out = unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
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
fn real_next_incompatibility_preserves_its_offense_without_applying_edits() {
    let d = project();
    let source = "[].each do\n  if ready?\n    a\n    b\n    c\n  end\nend\n";
    for only in [
        "Style/Next,Style/SafeNavigation",
        "Style/SafeNavigation,Style/Next",
    ] {
        let out = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", only, "--fix", "a.rb"],
            1,
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("Style/Next"));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", "Style/Next", "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "[].each do\n  next unless ready?\n  a\n  b\n  c\nend\n"
    );
}
#[test]
fn fixed_options_are_not_exposed_as_new_configuration() {
    let d = project();
    for option in [
        "MaxChainLength = 3",
        "ConvertCodeThatCanStartToReturnNil = true",
        "AllowedMethods = []",
        "TargetRubyVersion = 2.2",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 2);
    }
}
#[test]
fn both_directive_prefixes_and_all_targets_suppress_and_restore_findings() {
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
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    )
    .unwrap();
    let out = unchanged(
        d.path(),
        format!("# rustocop:enable {COP}\n{SOURCE}").as_bytes(),
        &["a.rb"],
        1,
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
}
#[test]
fn all_707_pinned_calls_and_493_unsafe_corrections_preserve_bytes_and_mtime() {
    let d = project();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/safe_navigation_pinned.json")).unwrap();
    let mut corrections = 0;
    let mut rejections = 0;
    for case in cases.as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let invalid = case["spec_line"] == 222 && source.contains("yield ");
        let code = if invalid {
            2
        } else {
            i32::from(!case["positions"].as_array().unwrap().is_empty())
        };
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
        corrections += usize::from(!case["correction"].is_null());
        if invalid {
            rejections += 1;
            unchanged(
                d.path(),
                format!("def f\n{source}end\n").as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
    }
    assert_eq!(
        (cases.as_array().unwrap().len(), corrections, rejections),
        (707, 493, 6)
    );
}
#[test]
fn bom_crlf_encodings_line_data_clean_and_parse_error_sources_are_never_written() {
    let d = project();
    for source in [
        "\u{feff}# encoding: UTF-8\r\n  x && x.foo('café', __LINE__) # keep\r\np __LINE__\r\n__END__\r\npayload\0ÿ\r\n".as_bytes(),
        b"# encoding: ISO-8859-1\r\nx.foo('caf\xe9') if x\r\np [__LINE__, DATA.pos]\r\n__END__\r\nraw\xff\r\n",
    ] {
        unchanged(d.path(), source, &["--only", COP, "a.rb"], 1);
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    }
    unchanged(d.path(), b"x&.foo\n", &["--only", COP, "--fix", "a.rb"], 0);
    unchanged(d.path(), b"(\n", &["--only", COP, "--fix", "a.rb"], 2);
}
fn runtime(source: &str, expected: &str) {
    let d = project();
    fs::write(d.path().join("a.rb"), source).unwrap();
    for binary in ["ruby", "/opt/ruby34/miniruby"] {
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
fn false_nil_argument_evaluation_and_intermediate_errors_prove_unsafe_rewrites() {
    runtime(
        "$events = []\ndef arg; $events << :arg; 1; end\nx = false\np(x && x.foo(arg))\nbegin; x&.foo(arg); rescue NoMethodError; puts :false_error; end\np $events\nx = nil\np(x && x.foo(arg))\np(x&.foo(arg))\np $events\nx = Struct.new(:foo).new(nil)\nbegin; x && x.foo.bar; rescue NoMethodError; puts :chain_error; end\np(x&.foo&.bar)\n",
        "false\nfalse_error\n[:arg]\nnil\nnil\n[:arg]\nchain_error\nnil\n",
    );
}
#[test]
fn repeated_receiver_initialization_and_mutable_frozen_identity_are_preserved() {
    runtime(
        "$events = []\nclass Box\n def initialize; $events << :init; end\n def foo; $events << :foo; self; end\nend\ndef make; Box.new; end\nx = make && make.foo\np $events\np x.frozen?\nx.freeze\np((x && x.foo).equal?(x))\np x.frozen?\np $events\n",
        "[:init, :init, :foo]\nfalse\ntrue\ntrue\n[:init, :init, :foo, :foo]\n",
    );
}
#[test]
fn overridable_nil_and_negation_refinements_and_control_flow_are_preserved() {
    runtime(
        "class Box\n def nil?; true; end\n def !; true; end\n def foo; :called; end\nend\nx = Box.new\np((x.foo unless x.nil?))\np((x.foo if !x.nil?))\np((x.foo unless !x))\np(x&.foo)\nmodule R\n refine Box do\n  def nil?; false; end\n end\nend\nusing R\np((x.foo unless x.nil?))\ndef f(x)\n return :early unless x\n x && x.foo\nend\np f(false)\np f(x)\n",
        "nil\nnil\nnil\n:called\n:called\n:early\n:called\n",
    );
}
