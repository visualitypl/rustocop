//! detection-only dig policies through the real CLI.
//! Pinned fixtures port RuboCop specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const SINGLE: &str = "Style/SingleArgumentDig";
const CHAIN: &str = "Style/DigChain";
const BOTH: &str = "Style/SingleArgumentDig,Style/DigChain";
const CASES: [(&str, &str); 2] = [
    (SINGLE, "data.dig(:key)\n"),
    (CHAIN, "data.dig(:a).dig(:b)\n"),
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
#[test]
fn registry_exact_names_defaults_and_no_safe_fixes() {
    let registry = rustocop::cops::Registry::new();
    for (name, _) in CASES {
        let cops: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == name)
            .collect();
        assert_eq!(cops.len(), 1);
        assert!(cops[0].default_enabled());
        assert!(!cops[0].safe_fixes());
        assert!(cops[0].path_policy().is_none());
        assert_eq!(cops[0].ruby_versions(), Default::default());
    }
}
#[test]
fn defaults_selectors_config_excludes_and_no_write() {
    let d = project();
    for (cop, source) in CASES {
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for (index, config) in [
            "[cops.Style]\nenabled = false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ]
        .into_iter()
        .enumerate()
        {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            // Disabling DigChain alone restores SingleArgumentDig's findings.
            let code = i32::from(cop == CHAIN && index == 1);
            let o = unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], code);
            assert!(!String::from_utf8_lossy(&o.stdout).contains(&format!("({cop})")));
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                1,
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        let o = unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        for config in [
            "exclude = [\"*.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"*.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for setting in ["EnforcedStyle = \"index\"", "TargetRubyVersion = 4.0"] {
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
fn directives_config_disabled_source_enabling_clean_and_parse_errors() {
    let d = project();
    for (cop, source) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                for kind in ["disable", "disable-next"] {
                    let s = format!("# {prefix}:{kind} {target}\n{source}");
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
                let s = format!("{} # {prefix}:disable {target}\n", source.trim_end());
                unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
            }
            let s = format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            let o = unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            format!("# rustocop:enable {cop}\n{source}").as_bytes(),
            &["a.rb"],
            1,
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        unchanged(d.path(), b"nil\n", &["--only", cop, "--fix", "a.rb"], 0);
        unchanged(d.path(), b"(\n", &["--only", cop, "--fix", "a.rb"], 2);
    }
}
#[test]
fn delegation_depends_on_configuration_not_cli_selection_or_inline_suppression() {
    let d = project();
    let source = CASES[1].1.as_bytes();
    // --only/--except do not change configured companion policy.
    unchanged(d.path(), source, &["--only", SINGLE, "--fix", "a.rb"], 0);
    unchanged(
        d.path(),
        source,
        &["--only", BOTH, "--except", CHAIN, "--fix", "a.rb"],
        0,
    );
    unchanged(
        d.path(),
        format!("# rustocop:disable {CHAIN}\n{}", CASES[1].1).as_bytes(),
        &["--only", BOTH, "--fix", "a.rb"],
        0,
    );
    for config in [
        format!("[cops.\"{CHAIN}\"]\nenabled = false\n"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{SINGLE}\"]\nenabled = true\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let o = unchanged(d.path(), source, &["--only", SINGLE, "--fix", "a.rb"], 1);
        assert_eq!(
            String::from_utf8_lossy(&o.stdout)
                .matches(&format!("({SINGLE})"))
                .count(),
            2
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{CHAIN}\"]\nenabled = true\n"),
    )
    .unwrap();
    unchanged(d.path(), source, &["--only", SINGLE, "--fix", "a.rb"], 0);
}
#[test]
fn every_pinned_call_and_all_16_unsafe_corrections_are_no_write() {
    let d = project();
    let mut calls = 0;
    let mut corrections = 0;
    for fixture in [
        include_str!("fixtures/single_argument_dig_pinned.json"),
        include_str!("fixtures/dig_chain_pinned.json"),
    ] {
        let cases: serde_json::Value = serde_json::from_str(fixture).unwrap();
        for case in cases.as_array().unwrap() {
            let mut config = String::new();
            for (section, settings) in case["toml"].as_object().unwrap() {
                config.push_str(&format!("[cops.\"{section}\"]\n"));
                for (key, value) in settings.as_object().unwrap() {
                    config.push_str(&format!("{key} = {value}\n"));
                }
            }
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let cop = case["cop"].as_str().unwrap();
            let source = case["source"].as_str().unwrap();
            let code = i32::from(!case["positions"].as_array().unwrap().is_empty());
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], code);
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
            calls += 1;
            corrections += usize::from(case["correction"].is_string());
        }
    }
    assert_eq!((calls, corrections), (39, 16));
}
#[test]
fn byte_exact_bom_crlf_encoding_line_and_data_envelopes() {
    let d = project();
    for source in [
        "\u{feff}# encoding: UTF-8\r\n  data.dig(\"café\").dig(__LINE__) # keep\r\np __LINE__\r\n__END__\r\nraw\0ÿ\r\n".as_bytes(),
        b"# encoding: ISO-8859-1\r\ndata.dig('caf\xe9')\r\np [__LINE__, DATA.pos]\r\n__END__\r\nraw\xff\r\n",
    ] {
        unchanged(d.path(), source, &["--only", BOTH, "a.rb"], 1);
        unchanged(d.path(), source, &["--only", BOTH, "--fix", "a.rb"], 1);
    }
}
fn ruby(binary: &str, dir: &Path) -> Option<Output> {
    Command::new(binary)
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .ok()
}
fn runtime(source: &str, expected: &str) {
    let d = project();
    fs::write(d.path().join("a.rb"), source).unwrap();
    for binary in ["ruby", "/opt/ruby34/miniruby"] {
        let Some(before) = ruby(binary, d.path()) else {
            continue;
        };
        assert!(before.status.success(), "{binary}: {before:?}");
        assert_eq!(
            String::from_utf8_lossy(&before.stdout),
            expected,
            "{binary}"
        );
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", BOTH, "--fix", "a.rb"],
            1,
        );
        let after = ruby(binary, d.path()).unwrap();
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        assert_eq!(before.status.code(), after.status.code());
    }
}
#[test]
fn custom_dispatch_lexical_refinements_initialization_mutability_and_false_nil_are_preserved() {
    runtime(
        "$events = []\nclass Box\n def initialize; $events << :init; end\n def dig(*keys); $events << keys; self; end\n def [](*keys); raise 'not dig'; end\nend\ndef key; $events << :key; :a; end\nx = Box.new\np x.dig(key).equal?(x)\np x.dig(:a).dig(:b).equal?(x)\np $events\np [{a: false}.dig(:a), {a: nil}.dig(:a), [false].dig(0)]\n",
        "true\ntrue\n[:init, :key, [:a], [:a], [:b]]\n[false, nil, false]\n",
    );
    runtime(
        "module Refined\n refine Hash do\n  def dig(*keys); [:custom, keys]; end\n end\nend\nusing Refined\np({}.dig(:a))\na = [{a: false}]\np a.dig(0).dig(:a)\np a.frozen?\np({a: nil}.freeze.dig(:a))\np ['x'.freeze].freeze.dig(0).frozen?\n",
        "[:custom, [:a]]\n[:custom, [:a]]\nfalse\n[:custom, [:a]]\ntrue\n",
    );
}
#[test]
fn nil_short_circuit_argument_effects_and_error_control_flow_are_preserved() {
    runtime(
        "$events = []\ndef key; $events << :key; :b; end\np({a: nil}.dig(:a)&.dig(key))\np $events\nbegin\n {a: nil}.dig(:a).dig(key)\nrescue NoMethodError\n puts :missing\nend\np $events\nbegin\n {a: false}.dig(:a).dig(key)\nrescue NoMethodError\n puts :false_receiver\nend\np $events\n",
        "nil\n[]\nmissing\n[:key]\nfalse_receiver\n[:key, :key]\n",
    );
}
