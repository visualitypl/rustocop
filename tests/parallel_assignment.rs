//! CLI controls, pinned corrections and controlled Ruby witnesses.
//! RuboCop fixtures: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::project;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const COP: &str = "Style/ParallelAssignment";
const BEFORE: &str = "a,b=1,2\n";
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], count: usize) {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(root, args);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(COP))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), time);
}
#[test]
fn defaults_selectors_and_config_never_implicitly_write() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    let rule = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(rule.default_enabled() && rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    for (config, selectors, count) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Style", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (format!("[cops.\"{COP}\"]\nenabled=false\n"), vec![], 0),
        (
            format!("[cops.\"{COP}\"]\nenabled=false\n"),
            vec!["--only", COP],
            1,
        ),
        (
            format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
            vec![],
            1,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("a.rb");
        unchanged(d.path(), BEFORE.as_bytes(), &args, count);
    }
}
#[test]
fn stdin_has_exact_positions_and_never_writes() {
    let d = project();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"  a,b=1,2\n")
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        text.contains("a.rb:1:3:")
            && text.contains(COP)
            && text.contains("Do not use parallel assignment."),
        "{text}"
    );
    assert!(!d.path().join("a.rb").exists());
}
#[test]
fn mixed_style_layout_and_default_fixes_stabilize_within_eight_explicit_passes() {
    let d = project();
    let before = "a,b=[1,2],{ :key => 'value' };\np [a,b]\n";
    let selected = format!(
        "{COP},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    for selector in [Some(selected.as_str()), Some("Style,Layout"), None] {
        fs::write(d.path().join("a.rb"), before).unwrap();
        let mut stable = false;
        for _ in 0..8 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(s) = selector {
                args.extend(["--only", s]);
            }
            let o = cli(d.path(), &args);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if fs::read(d.path().join("a.rb")).unwrap() == old {
                stable = true;
                break;
            }
        }
        assert!(stable);
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(
            after.contains("a = [1, 2]") && after.contains("key: \"value\""),
            "{after}"
        );
    }
}
