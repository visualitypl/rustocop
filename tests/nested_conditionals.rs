//! real CLI controls, canonical corrections and controlled Ruby observations.
use std::{fs, path::Path, process::Command};
const MODIFIER: &str = "Style/NestedModifier";
const TERNARY: &str = "Style/NestedTernaryOperator";
const ALL: &str = "Style/NestedModifier,Style/NestedTernaryOperator";
const CASES: [(&str, &str, &str); 2] = [
    (MODIFIER, "work if a if b\n", "work if b && a\n"),
    (
        TERNARY,
        "a ? (b ? 1 : 2) : 3\n",
        "if a\nb ? 1 : 2\nelse\n3\nend\n",
    ),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, cop: &str, source: &[u8], args: &[&str], count: usize) {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(root, args);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(cop))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn defaults_selectors_config_and_plain_lint_do_not_write() {
    let d = project();
    for (cop, source, _) in CASES {
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("a.rb");
            unchanged(d.path(), cop, source.as_bytes(), &args, count);
        }
        let registry = rustocop::cops::Registry::new();
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    }
}
#[test]
fn excludes_and_both_directive_prefixes_hold_bytes() {
    let d = project();
    for (cop, source, _) in CASES {
        for config in [
            "exclude = [\"a.rb\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                unchanged(
                    d.path(),
                    cop,
                    format!("# {prefix}:disable {target}\n{source}").as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            unchanged(
                d.path(),
                cop,
                format!("# {prefix}:disable-next {cop}\n{source}").as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            unchanged(
                d.path(),
                cop,
                format!("{} # {prefix}:disable {cop}\n", source.trim_end()).as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
    unchanged(
        d.path(),
        TERNARY,
        b"a ? (b ? 1 : 2) : (3 # rubocop:disable Lint/Debugger\n)\n",
        &["--only", TERNARY, "--fix", "a.rb"],
        1,
    );
}
fn observations_equal(root: &Path) {
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby)
            .arg(root.join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby).arg(root.join("a.rb")).output().unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
    }
}
#[test]
fn runtime_ternary_hash_keys_in_bare_commands_keep_grouping() {
    let d = project();
    for source in [
        "p true ? (false ? 1 : 2) : 3 => 4\n",
        "p :first, true ? (false ? 1 : 2) : 3 => 4\n",
        "p true ? (false ? (true ? 1 : 2) : 3) : 4 => 5\n",
        "p({true ? (false ? 1 : 2) : 3 => 4})\n",
        "p true ? (p false ? (true ? 1 : 2) : 3 => 4) : 5\n",
    ] {
        fs::write(d.path().join("before.rb"), source).unwrap();
        fs::write(d.path().join("a.rb"), source).unwrap();
        let o = cli(d.path(), &["--only", TERNARY, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(0), "{source}: {o:?}");
        assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        observations_equal(d.path());
    }
}
#[test]
fn selected_default_and_style_layout_composition_stabilizes_within_eight_passes() {
    let source = "def f(a,b)\n return a ? (b ? { :key => 'value' } : nil) : false;\nend\ndef g(a,b)\n raise Exception, 'boom' if a if b\nend\n";
    let d = project();
    for selection in [
        Some(ALL),
        Some(
            "Style/NestedModifier,Style/NestedTernaryOperator,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..8 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec![];
            if let Some(s) = selection {
                args.extend(["--only", s]);
            }
            args.extend(["--fix", "a.rb"]);
            let o = cli(d.path(), &args);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if old == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selection:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(!after.contains("if a if b"), "{after}");
        assert!(!after.contains("a ? (b ?"), "{after}");
    }
}
#[test]
fn strict_config_and_stdin_never_write() {
    use std::{io::Write, process::Stdio};
    let d = project();
    for (cop, source, _) in CASES {
        for key in ["max = 1", "EnforcedStyle = \"other\""] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            fs::write(d.path().join("a.rb"), source).unwrap();
            let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(o.status.code(), Some(2), "{o:?}");
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        fs::write(d.path().join("a.rb"), "sentinel\n").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
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
        assert_eq!(o.status.code(), Some(1), "{o:?}");
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "sentinel\n"
        );
    }
}
