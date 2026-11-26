//! real pipeline/CLI contracts and controlled Ruby semantic witnesses.
//! Pinned fixtures ported from RuboCop (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use crate::support::cli::project;
use std::{fs, path::Path, process::Command};
const ASSIGN: &str = "Style/RedundantAssignment";
const SELF: &str = "Style/RedundantSelfAssignment";
const BRANCH: &str = "Style/RedundantSelfAssignmentBranch";
const ALL: &str =
    "Style/RedundantAssignment,Style/RedundantSelfAssignment,Style/RedundantSelfAssignmentBranch";
const CASES: [(&str, &str, &str); 3] = [
    (
        ASSIGN,
        "def f\n  x = work\n  x\nend\n",
        "def f\n  work\n  \nend\n",
    ),
    (SELF, "foo = foo.concat(ary)\n", "foo = foo.concat(ary)\n"),
    (
        BRANCH,
        "foo = condition ? bar : foo\n",
        "foo = bar if condition\n",
    ),
];
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
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
}
fn fix(root: &Path, cop: &str, before: &str, after: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let output = cli(root, &["--only", cop, "--fix", "a.rb"]);
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        fs::read_to_string(root.join("a.rb")).unwrap(),
        after,
        "{output:?}"
    );
}
#[test]
fn defaults_selectors_config_and_no_implicit_writes() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (cop, before, _) in CASES {
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled());
        assert_eq!(rule.safe_fixes(), cop != SELF);
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
fn excludes_directives_and_explicit_fix_respect_policy() {
    let d = project();
    for (cop, before, after) in CASES {
        for config in [
            "exclude = ['a.rb']\n".to_string(),
            format!("[cops.\"{cop}\"]\nexclude = ['a.rb']\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                before.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                let source = format!("# {prefix}:disable {target}\n{before}");
                unchanged(
                    d.path(),
                    cop,
                    source.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            let source =
                format!("# {prefix}:disable {cop}\n{before}# {prefix}:enable {cop}\n{before}");
            let expected =
                format!("# {prefix}:disable {cop}\n{before}# {prefix}:enable {cop}\n{after}");
            fix(d.path(), cop, &source, &expected);
        }
    }
}
#[test]
fn stdin_has_exact_positions_selectors_and_no_disk_writes() {
    use std::{io::Write, process::Stdio};
    let d = project();
    for (cop, before, _) in CASES {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let text = String::from_utf8_lossy(&output.stdout);
        let pos = if cop == ASSIGN {
            "a.rb:2:3:"
        } else if cop == SELF {
            "a.rb:1:5:"
        } else {
            "a.rb:1:25:"
        };
        assert!(text.contains(pos) && text.contains(cop), "{text}");
        assert!(!d.path().join("a.rb").exists());
    }
}
#[test]
fn terminal_rescue_modifier_corrections_preserve_runtime_results() {
    let d = project();
    let before = "def f(fail); (raise if fail; :ok) rescue (x = 1; x); end\ndef g(fail); (x = (raise if fail; :ok); x) rescue :rescued; end\np [f(false), f(true), g(false), g(true)]\n";
    let after = "def f(fail); (raise if fail; :ok) rescue (1; ); end\ndef g(fail); ((raise if fail; :ok); ) rescue :rescued; end\np [f(false), f(true), g(false), g(true)]\n";
    fix(d.path(), ASSIGN, before, after);
    fs::write(d.path().join("before.rb"), before).unwrap();
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        for path in ["before.rb", "a.rb"] {
            let output = Command::new(&ruby)
                .arg(d.path().join(path))
                .output()
                .unwrap();
            assert!(output.status.success(), "{ruby}: {output:?}");
            assert_eq!(output.stdout, b"[:ok, 1, :ok, :rescued]\n", "{ruby}");
            assert!(output.stderr.is_empty(), "{ruby}: {output:?}");
        }
    }
}
#[test]
fn mixed_style_layout_composition_stabilizes_within_six_explicit_passes() {
    let d = project();
    let source = "def f\n  x = ({ :key => 'value' });\n  x\nend\nfoo = condition ? foo : 'other'\n";
    let selected = format!(
        "{ALL},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/EndAlignment,Layout/IndentationWidth,Layout/TrailingWhitespace"
    );
    fs::write(d.path().join("a.rb"), source).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        let output = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        if old == fs::read(d.path().join("a.rb")).unwrap() {
            stable = true;
            break;
        }
    }
    assert!(stable);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(
        after.contains("key: \"value\"")
            && !after.contains("x =")
            && after.contains("foo = \"other\" unless condition"),
        "{after}"
    );
}
