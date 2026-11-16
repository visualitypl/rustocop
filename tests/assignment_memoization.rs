//! real CLI policy, pinned corrections and controlled Ruby witnesses.
//! RuboCop spec fixtures: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const MEMO: &str = "Style/MultilineMemoization";
const OR: &str = "Style/OrAssignment";
const BOTH: &str = "Style/MultilineMemoization,Style/OrAssignment";
const CASES: [(&str, &str, &str); 2] = [
    (
        MEMO,
        "foo ||= (\n  work\n)\n",
        "foo ||= begin\n  work\nend\n",
    ),
    (OR, "foo = foo ? foo : 'default'\n", "foo ||= 'default'\n"),
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
    let time = fs::metadata(&path).unwrap().modified().unwrap();
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
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), time);
}
#[test]
fn default_enablement_selectors_and_config_never_implicitly_write() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (cop, before, _) in CASES {
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
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
            unchanged(d.path(), cop, before.as_bytes(), &args, count);
        }
    }
}
#[test]
fn stdin_obeys_selectors_excludes_and_reports_exact_positions() {
    let d = project();
    for (cop, source, _) in CASES {
        for (config, count) in [
            (String::new(), 1),
            (format!("[cops.\"{cop}\"]\nexclude = ['a.rb']\n"), 0),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
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
            assert_eq!(o.status.code(), Some(count), "{o:?}");
            let text = String::from_utf8_lossy(&o.stdout);
            assert_eq!(text.contains(cop), count == 1, "{text}");
            if count == 1 {
                assert!(text.contains("a.rb:1:1:"), "{text}");
            }
            assert!(!d.path().join("a.rb").exists());
        }
    }
}
#[test]
fn mixed_assignment_and_existing_style_layout_fixes_stabilize_within_six_passes() {
    let d = project();
    let before = "def f\n  foo = if foo\n    foo\n  else\n    ({ :key => 'value' });\n  end\nend\n";
    let selected = format!(
        "{BOTH},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/EndAlignment,Layout/IndentationWidth"
    );
    fs::write(d.path().join("a.rb"), before).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        let o = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        let new = fs::read(d.path().join("a.rb")).unwrap();
        if old == new {
            stable = true;
            break;
        }
    }
    assert!(stable);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(
        after.contains("foo ||=") && after.contains("key: \"value\""),
        "{after}"
    );
}
