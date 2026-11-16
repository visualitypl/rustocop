//! Ported pinned RuboCop style/{inverse_methods,multiple_comparison}
//! specs (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
use serde::Deserialize;
use std::{fs, path::Path, process::Command};
const COPS: [&str; 2] = ["Style/InverseMethods", "Style/MultipleComparison"];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) -> std::process::Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    o
}
fn unchanged(dir: &Path, s: &[u8], args: &[&str], code: i32) -> std::process::Output {
    let p = dir.join("a.rb");
    fs::write(&p, s).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), s);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
use crate::support::stdin as stdin_support;
#[test]
fn stdin_never_writes_and_fix_is_explicitly_rejected() {
    let d = project();
    let p = d.path().join("a.rb");
    fs::write(&p, "# sentinel\n").unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    for (cop, source) in COPS
        .into_iter()
        .zip(["!foo.none?\n", "a=1; a == 1 || a == 2\n"])
    {
        for fix in [false, true] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            cmd.current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            if fix {
                cmd.arg("--fix");
            }
            let mut child = cmd.spawn().unwrap();
            stdin_support::write_stdin(child.stdin.take().unwrap(), source.as_bytes(), fix)
                .unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }), "{o:?}");
            if fix {
                assert!(String::from_utf8_lossy(&o.stderr).contains("--fix"));
            } else {
                assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
            }
            assert_eq!(fs::read(&p).unwrap(), b"# sentinel\n");
            assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
        }
    }
}
#[derive(Deserialize)]
struct Case {
    cop: String,
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!(
        "fixtures/inverse_multiple_comparison_pinned.json"
    ))
    .unwrap()
}
#[test]
fn every_pinned_call_has_exact_positions_and_messages() {
    let mut counts = [0; 2];
    for c in cases() {
        counts[COPS.iter().position(|&cop| cop == c.cop).unwrap()] += 1;
        let registry = rustocop::cops::Registry::new();
        let config = rustocop::config::ResolvedConfig::default();
        let active =
            rustocop::cops::select_active(&registry, &config, std::slice::from_ref(&c.cop), &[]);
        let result = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(c.source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            result.errors.is_empty() && result.internal_errors.is_empty(),
            "{result:?}"
        );
        let found = result.result.offenses;
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "{}:{} {}",
            c.cop,
            c.spec_line,
            c.source
        );
        assert_eq!(
            found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
            c.messages,
            "{}:{}",
            c.cop,
            c.spec_line
        );
    }
    assert_eq!(counts, [111, 27]);
}
#[test]
fn defaults_selectors_config_excludes_directives_and_no_write() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (i, source) in ["!foo.none?\n", "a=1; a == 1 || a == 2\n"]
        .into_iter()
        .enumerate()
    {
        let cop = COPS[i];
        let rule = registry.cops().iter().find(|r| r.name() == cop).unwrap();
        assert!(rule.default_enabled());
        assert_eq!(rule.safe_fixes(), i == 1);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        for except in [cop, "Style"] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--except", except, "--fix", "a.rb"],
                0,
            );
        }
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(d.path().join("a.rb"), source).unwrap();
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .arg("a.rb")
                .output()
                .unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            assert!(!String::from_utf8_lossy(&o.stdout).contains(cop));
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        assert!(
            String::from_utf8_lossy(&unchanged(d.path(), source.as_bytes(), &["a.rb"], 1).stdout)
                .contains(cop)
        );
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nComparisonsThreshold=3\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            2,
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
        }
        unchanged(d.path(), b"(\n", &["--only", cop, "--fix", "a.rb"], 2);
        unchanged(d.path(), b"a=1\n", &["--only", cop, "--fix", "a.rb"], 0);
    }
}
#[test]
fn mixed_selected_peer_and_default_corrections_stabilize_within_six_explicit_passes() {
    let d = project();
    let source = "a=1; (a == 1 || a == 2)\n!foo.none?\n";
    for selection in [
        Some("Style/MultipleComparison"),
        Some(
            "Style/MultipleComparison,Style/StringLiterals,Style/RedundantReturn,Style/Semicolon,Style/HashSyntax,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(s) = selection {
                args.extend(["--only", s]);
            }
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            let after = fs::read(d.path().join("a.rb")).unwrap();
            assert!(String::from_utf8_lossy(&after).contains("!foo.none?"));
            if before == after {
                stable = true;
                break;
            }
        }
        assert!(stable);
        assert!(
            fs::read_to_string(d.path().join("a.rb"))
                .unwrap()
                .contains(".include?(a)")
        );
    }
}
