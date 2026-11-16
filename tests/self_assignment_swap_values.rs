//! real CLI policy, pinned corrections and controlled Ruby witnesses.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{fs, path::Path, process::Command};
const SELF: &str = "Style/SelfAssignment";
const SWAP: &str = "Style/SwapValues";
const BOTH: &str = "Style/SelfAssignment,Style/SwapValues";
const CASES: [(&str, &str); 2] = [(SELF, "x = x + 1\n"), (SWAP, "tmp=x\nx=y\ny=tmp\n")];
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
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
}
fn fix(root: &Path, selected: &str, before: &str, after: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let o = cli(root, &["--only", selected, "--fix", "a.rb"]);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        fs::read_to_string(root.join("a.rb")).unwrap(),
        after,
        "{o:?}"
    );
}
#[test]
fn default_selectors_and_config_are_real_and_never_implicitly_write() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (cop, source) in CASES {
        let rule = registry.cops().iter().find(|r| r.name() == cop).unwrap();
        assert!(rule.default_enabled());
        assert_eq!(rule.safe_fixes(), cop == SELF);
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled=false\n"), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled=false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
                vec![],
                1,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("a.rb");
            unchanged(d.path(), cop, source.as_bytes(), &args, count);
        }
    }
}
#[test]
fn excludes_and_both_directive_prefixes_protect_files() {
    let d = project();
    for (cop, source) in CASES {
        for config in [
            "exclude=['a.rb']\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude=['a.rb']\n"),
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
                let before = format!("# {prefix}:disable {target}\n{source}");
                unchanged(
                    d.path(),
                    cop,
                    before.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            let before =
                format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            let after = if cop == SELF {
                format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\nx += 1\n")
            } else {
                before.clone()
            };
            fix(d.path(), cop, &before, &after);
        }
    }
}
#[test]
fn non_utf8_swap_identifiers_are_compared_as_bytes_and_never_rewritten() {
    let d = project();
    for (tmp, x, y) in [
        ("tmp", "café", "thé"),
        ("tèmp", "café", "thé"),
        ("@tèmp", "@café", "@thé"),
        ("@@Tèmp", "@@Café", "@@Thé"),
        ("$tèmp", "$café", "$thé"),
        ("Tèmp", "Café", "Thé"),
        ("::Tèmp", "::Café", "::Thé"),
        ("Café::Tèmp", "Café::X", "Thé::Y"),
        ("::Café::Tèmp", "::Café::X", "::Thé::Y"),
    ] {
        let source = format!("# coding: ISO-8859-1\n{tmp} = {x}\n{x} = {y}\n{y} = {tmp}\n");
        let bytes: Vec<u8> = source
            .chars()
            .map(|c| u8::try_from(c as u32).unwrap())
            .collect();
        unchanged(
            d.path(),
            SWAP,
            &bytes,
            &["--only", SWAP, "--fix", "a.rb"],
            1,
        );
        let o = cli(d.path(), &["--only", SWAP, "a.rb"]);
        let output = String::from_utf8_lossy(&o.stdout);
        assert!(output.contains("a.rb:2:1:"), "{o:?}");
        let message =
            format!("Replace this and assignments at lines 3 and 4 with `{x}, {y} = {y}, {x}`.");
        let message_bytes: Vec<u8> = message
            .chars()
            .map(|c| u8::try_from(c as u32).unwrap())
            .collect();
        assert!(
            output.contains(String::from_utf8_lossy(&message_bytes).as_ref()),
            "{o:?}"
        );
    }
    // These different names have identical lossy UTF-8 renderings.
    unchanged(
        d.path(),
        SWAP,
        b"# coding: ISO-8859-1\ntmp = caf\xe9\ncaf\xe8 = th\xe9\nth\xe9 = tmp\n",
        &["--only", SWAP, "--fix", "a.rb"],
        0,
    );
}
#[test]
fn mixed_style_layout_edits_stabilize_without_reordering_or_removing_swaps() {
    let d = project();
    let before =
        "def f(x)\n  x = x + ({ :key => 'value' });\n  tmp=x\n  x=y\n  y=tmp\n  return x\nend\n";
    // Exclude the explicitly incompatible spacing peer when composing fixes.
    let selected = format!(
        "{BOTH},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/EndAlignment,Layout/IndentationWidth"
    );
    fs::write(d.path().join("a.rb"), before).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        let o = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        if fs::read(d.path().join("a.rb")).unwrap() == old {
            stable = true;
            break;
        }
    }
    assert!(stable);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(
        after.contains("x +=")
            && after.contains("key: \"value\"")
            && after.contains("tmp=x\n  x=y\n  y=tmp"),
        "{after}"
    );
}
