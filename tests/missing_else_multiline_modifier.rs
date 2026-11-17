//! real selectors/config/directives/no-write and controlled Ruby witnesses.
use std::{fs, path::Path, process::Command};
const MISSING: &str = "Style/MissingElse";
const MULTI: &str = "Style/MultilineIfModifier";
const BAD: &str = "[\n  work\n] if ready\n";
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
fn unchanged(
    root: &Path,
    cop: &str,
    source: &[u8],
    args: &[&str],
    count: usize,
) -> std::process::Output {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(root, args);
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| l.contains(cop))
            .count(),
        count,
        "{output:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{output:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    output
}
#[test]
fn default_opt_in_selectors_config_and_no_write_contract() {
    let d = project();
    for (cop, source, default) in [(MISSING, "if ready; work; end\n", 0), (MULTI, BAD, 1)] {
        for (config, selectors, count) in [
            (String::new(), vec![], default),
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
        assert_eq!(rule.default_enabled(), default == 1);
        assert!(rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    }
}
#[test]
fn actual_guard_clause_companion_incompatibility_and_configured_exemptions() {
    let d = project();
    let source = b"def f\n  if ready\n    work\n  end\nend\n";
    for selected in [
        "Style/MissingElse,Style/GuardClause",
        "Style/GuardClause,Style/MissingElse",
    ] {
        let o = unchanged(
            d.path(),
            MISSING,
            source,
            &["--only", selected, "--fix", "a.rb"],
            1,
        );
        assert!(
            String::from_utf8_lossy(&o.stdout).contains("(Style/GuardClause)"),
            "{o:?}"
        );
    }
    for (config, source, count) in [
        ("", "if a; raise 'boom'; end\n", 0),
        (
            "[cops.\"Style/GuardClause\"]\nenabled = false\n",
            "if a; raise 'boom'; end\n",
            1,
        ),
        (
            "[cops.Style]\nenabled = false\n",
            "unless a; work; end\n",
            1,
        ),
        ("", "unless a; work; end\n", 0),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            MISSING,
            source.as_bytes(),
            &[
                "--only",
                MISSING,
                "--except",
                "Style/GuardClause",
                "--fix",
                "a.rb",
            ],
            count,
        );
    }
}
#[test]
fn stdin_and_strict_config_errors_never_write() {
    use std::{io::Write, process::Stdio};
    let d = project();
    for cop in [MISSING, MULTI] {
        for (config, detail) in [
            (format!("[cops.\"{cop}\"]\nmax = 1\n"), "max"),
            (
                format!("[cops.\"{cop}\"]\nEnforcedStyle = \"other\"\n"),
                "EnforcedStyle",
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(d.path().join("a.rb"), BAD).unwrap();
            let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(o.status.code(), Some(2), "{o:?}");
            assert!(String::from_utf8_lossy(&o.stderr).contains(detail), "{o:?}");
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), BAD);
        }
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    fs::write(d.path().join("a.rb"), "sentinel\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", MULTI, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(BAD.as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    assert!(String::from_utf8_lossy(&o.stdout).contains(MULTI));
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "sentinel\n"
    );
}
#[test]
fn selected_and_default_style_layout_composition_stabilizes_within_eight_explicit_passes() {
    let d = project();
    let source = "def f(ready)\n [\n  { :key => 'value' }\n ] if ready\nend\n";
    for selected in [
        Some("Style/MissingElse,Style/MultilineIfModifier"),
        Some(
            "Style/MultilineIfModifier,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..8 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec![];
            if let Some(selected) = selected {
                args.extend(["--only", selected]);
            }
            args.extend(["--fix", "a.rb"]);
            let o = cli(d.path(), &args);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if old == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(!after.contains("] if ready"), "{after}");
    }
}
