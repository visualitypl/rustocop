//! pinned RuboCop loop specs (MIT, Copyright (c) 2012-26
//! Bozhidar Batsov), real pipeline/CLI controls and controlled runtime witnesses.
use std::{fs, path::Path, process::Command};
const NEG: &str = "Style/NegatedWhile";
const DO: &str = "Style/WhileUntilDo";
const MOD: &str = "Style/WhileUntilModifier";
const ALL: &str = "Style/NegatedWhile,Style/WhileUntilDo,Style/WhileUntilModifier";
const CASES: [(&str, &str, &str); 3] = [
    (NEG, "while !true\n work\nend\n", "until true\n work\nend\n"),
    (
        DO,
        "while cond do\n work\nend\n",
        "while cond\n work\nend\n",
    ),
    (MOD, "while cond\n work\nend\n", "work while cond\n"),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_ne!(o.status.code(), Some(2), "{args:?}: {o:?}");
    o
}
fn unchanged(root: &Path, source: &[u8], args: &[&str]) -> std::process::Output {
    fs::write(root.join("a.rb"), source).unwrap();
    let time = fs::metadata(root.join("a.rb")).unwrap().modified().unwrap();
    let o = cli(root, args);
    assert_eq!(fs::read(root.join("a.rb")).unwrap(), source, "{o:?}");
    assert_eq!(
        fs::metadata(root.join("a.rb")).unwrap().modified().unwrap(),
        time
    );
    o
}
fn correct(root: &Path, cop: &str, before: &str) -> String {
    fs::write(root.join("a.rb"), before).unwrap();
    cli(root, &["--only", cop, "--fix", "--fix-passes", "1", "a.rb"]);
    fs::read_to_string(root.join("a.rb")).unwrap()
}
#[test]
fn default_on_selectors_config_and_no_write_without_fix() {
    let d = project();
    for (cop, before, after) in CASES {
        let registry = rustocop::cops::Registry::new();
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        for (cfg, args, expected) in [
            (String::new(), vec!["a.rb"], true),
            (String::new(), vec!["--only", cop, "a.rb"], true),
            (
                String::new(),
                vec!["--only", cop, "--except", cop, "--fix", "a.rb"],
                false,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled=false\n"),
                vec!["a.rb"],
                false,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled=false\n"),
                vec!["--only", cop, "a.rb"],
                true,
            ),
            (
                format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
                vec!["a.rb"],
                true,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), cfg).unwrap();
            let o = unchanged(d.path(), before.as_bytes(), &args);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout).contains(&format!("({cop})")),
                expected,
                "{o:?}"
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        assert_eq!(correct(d.path(), cop, before), after);
        unchanged(
            d.path(),
            after.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
        );
    }
}
#[test]
fn legacy_bytes_do_not_hide_modifier_offenses_or_get_lossily_corrected() {
    let d = project();
    let source = b"# coding: ISO-8859-1\nwhile cond\n \"caf\xe9\"\nend\n";
    let o = unchanged(d.path(), source, &["--only", MOD, "--fix", "a.rb"]);
    assert!(
        String::from_utf8_lossy(&o.stdout).contains(&format!("({MOD})")),
        "{o:?}"
    );
}
fn ruby_observation(ruby: &str, source: &str) -> Vec<u8> {
    let o = Command::new(ruby).args(["-e", source]).output().unwrap();
    assert!(o.status.success(), "{ruby}: {source}: {o:?}");
    o.stdout
}
#[test]
fn lead_open_flip_flop_do_regression_preserves_bytes_diagnostic_and_native_result() {
    let d = project();
    let source = "events=[]; r=until false.. do\n events << :body\n break :done\nend; p [r,events]";
    if let Ok(ruby) = crate::support::runtime::ruby() {
        assert_eq!(ruby_observation(&ruby, source), b"[:done, [:body]]\n");
    }
    let o = unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", DO, "--fix", "a.rb"],
    );
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    assert!(
        String::from_utf8_lossy(&o.stdout).contains(
            "a.rb:1:28: C: Do not use `do` with multi-line `until`. (Style/WhileUntilDo)"
        ),
        "{o:?}"
    );
    if let Ok(ruby) = crate::support::runtime::ruby() {
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert_eq!(ruby_observation(&ruby, &after), b"[:done, [:body]]\n");
    }
}
#[test]
fn do_range_wrappers_and_continuations_preserve_runtime_per_individual_edit() {
    let d = project();
    let rubies: Vec<_> = std::iter::once("ruby".to_owned())
        .chain(crate::support::runtime::ruby())
        .filter(|ruby| Command::new(ruby).arg("--version").output().is_ok())
        .collect();
    for word in ["while", "until"] {
        let (truth, opposite) = if word == "while" {
            ("true", "false")
        } else {
            ("false", "true")
        };
        for dots in ["..", "..."] {
            for (predicate, changes) in [
                (format!("{truth}{dots}"), false),
                (format!("true && {truth}{dots}"), false),
                (format!("false || {truth}{dots}"), false),
                (format!("not {opposite}{dots}"), false),
                (format!("consume {truth}{dots}"), false),
                (format!("flag = {truth}{dots}"), false),
                (format!("({truth}{dots})"), true),
                (format!("!({opposite}{dots})"), true),
                (format!("consume({truth}{dots})"), true),
                (format!("(flag = {truth}{dots})"), true),
                (format!("{truth} \\\n  "), false),
                (format!("({truth} && \\\n {truth})"), true),
            ] {
                for nl in ["\n", "\r\n"] {
                    let source = format!("def consume(value); value; end\nevents=[]\nr={word} {predicate} do\n -1\n events << :body\n break :done\nend\np [r,events]\n").replace('\n', nl);
                    let after = if changes {
                        let after = correct(d.path(), DO, &source);
                        assert_eq!(
                            after,
                            crate::support::lf::lf_str(&source.replacen(
                                &format!(" do{nl}"),
                                nl,
                                1
                            ))
                        );
                        after
                    } else {
                        let o = unchanged(
                            d.path(),
                            source.as_bytes(),
                            &["--only", DO, "--fix", "a.rb"],
                        );
                        assert_eq!(o.status.code(), Some(1), "{source}: {o:?}");
                        assert!(
                            String::from_utf8_lossy(&o.stdout).contains(&format!("({DO})")),
                            "{o:?}"
                        );
                        fs::read_to_string(d.path().join("a.rb")).unwrap()
                    };
                    for ruby in &rubies {
                        assert_eq!(
                            ruby_observation(ruby, &source),
                            ruby_observation(ruby, &after),
                            "{ruby}: {source} -> {after}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn withheld_open_range_does_not_block_unrelated_safe_do_fixes() {
    let d = project();
    let before = "events=[]; r=until false.. do\n events << :body\n break :done\nend; p [r,events]\ns=while true do\n events << :safe\n break :safe_done\nend; p [s,events]\n";
    let expected = before.replacen("while true do", "while true", 1);
    fs::write(d.path().join("a.rb"), before).unwrap();
    let o = cli(d.path(), &["--only", DO, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    let output = String::from_utf8_lossy(&o.stdout);
    assert!(
        output.contains("1 correction applied, 1 offense remaining"),
        "{o:?}"
    );
    assert!(output.contains("a.rb:1:28:"), "{o:?}");
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_eq!(after, expected);
    unchanged(d.path(), after.as_bytes(), &["--only", DO, "--fix", "a.rb"]);
    for ruby in std::iter::once("ruby".to_owned()).chain(crate::support::runtime::ruby()) {
        if Command::new(&ruby).arg("--version").output().is_ok() {
            assert_eq!(
                ruby_observation(&ruby, before),
                b"[:done, [:body]]\n[:safe_done, [:body, :safe]]\n"
            );
            assert_eq!(
                ruby_observation(&ruby, &after),
                ruby_observation(&ruby, before)
            );
        }
    }
}
#[test]
fn stdin_clean_and_parser_error_paths_never_write() {
    use std::io::Write;
    use std::process::Stdio;
    let d = project();
    for (cop, before, _) in CASES {
        fs::write(d.path().join("a.rb"), b"sentinel\n").unwrap();
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
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&o.stdout).contains(&format!("({cop})")));
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"sentinel\n");
        unchanged(d.path(), b"work\n", &["--only", cop, "--fix", "a.rb"]);
        let bad = b"while (\n";
        fs::write(d.path().join("a.rb"), bad).unwrap();
        let time = fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "{o:?}");
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), bad);
        assert_eq!(
            fs::metadata(d.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            time
        );
    }
}
#[test]
fn original_style_peers_preserve_control_flow_and_literal_values() {
    let d = project();
    let before = "def run\n while true do\n  raise 'boom'\n end\nend\nbegin\n run\nrescue => e\n p e.message\nend\n";
    let peers = format!(
        "{ALL},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    let mut current = before.to_owned();
    for _ in 0..4 {
        current = correct(d.path(), &peers, &current);
    }
    assert_eq!(correct(d.path(), &peers, &current), current);
    assert!(current.contains("raise \"boom\" while true"), "{current}");
    if Command::new("ruby").arg("--version").output().is_ok() {
        assert_eq!(
            ruby_observation("ruby", before),
            ruby_observation("ruby", &current)
        );
    }
}
#[test]
fn all_three_cops_and_layout_converge_in_explicit_bounded_passes() {
    let d = project();
    let before = "while !true do\n  work\nend\n";
    let one = correct(d.path(), ALL, before);
    assert_eq!(one, "until true\n  work\nend\n");
    let two = correct(d.path(), ALL, &one);
    let three = correct(d.path(), ALL, &two);
    let four = correct(d.path(), ALL, &three);
    assert_eq!(four, three);
    assert_eq!(three, "work until true\n");
    let mut current = before.to_owned();
    for _ in 0..4 {
        current = correct(d.path(), &format!("{ALL},Layout"), &current);
    }
    assert_eq!(
        correct(d.path(), &format!("{ALL},Layout"), &current),
        current
    );
}
