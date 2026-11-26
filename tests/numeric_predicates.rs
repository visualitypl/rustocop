//! pinned Ruby numeric idioms, CLI contracts and controlled observations.
use crate::support::cli::project;
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/EvenOdd";
const CASES: [(&str, &str); 3] = [
    ("Style/BitwisePredicate", "(variable & flags).positive?\n"),
    (COP, "x % 2 == 0\n"),
    ("Style/ExponentialNotation", "10e6\n"),
];
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
fn unchanged(dir: &Path, s: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, s).unwrap();
    let mtime = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), s);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), mtime);
    o
}
#[test]
fn defaults_selectors_config_excludes_and_fixed_version() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (cop, s) in CASES {
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let c = r.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), cop == COP);
        if cop == "Style/BitwisePredicate" {
            assert_eq!(
                c.ruby_versions().minimum,
                Some(rustocop::cops::policy::RubyVersion(2, 5))
            );
        }
        assert!(
            String::from_utf8_lossy(&unchanged(d.path(), s.as_bytes(), &["a.rb"], 1).stdout)
                .contains(cop)
        );
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), &config).unwrap();
            // NumericPredicate independently reports the zero comparison when
            // EvenOdd alone is disabled; neither rule rewrites unknown x.
            let numeric_remains = cop == COP && !config.contains("[cops.Style]");
            let o = unchanged(
                d.path(),
                s.as_bytes(),
                &["--fix", "a.rb"],
                i32::from(numeric_remains),
            );
            if numeric_remains {
                assert!(String::from_utf8_lossy(&o.stdout).contains("Style/NumericPredicate"));
            }
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
        }
        for key in [
            "EnforcedStyle=\"engineering\"",
            "TargetRubyVersion=2.4",
            "SafeAutoCorrect=true",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 2);
        }
    }
}
#[test]
fn directives_clean_and_error_files_retain_bytes_and_mtime() {
    let d = project();
    for (cop, s) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{s}"),
                    format!("# {prefix}:disable-next {target}\n{s}"),
                    format!("{} # {prefix}:disable {target}\n", s.trim_end()),
                ] {
                    unchanged(
                        d.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "a.rb"],
                        0,
                    );
                }
            }
            let s = format!("# {prefix}:disable {cop}\n{s}# {prefix}:enable {cop}\n{s}");
            let o = unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (s, code) in [("p :clean\n", 0), ("(\n", 2), ("01e2\n", 2)] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
}
fn rubies() -> Vec<String> {
    let mut r = vec!["ruby".into()];
    if let Ok(p) = crate::support::runtime::ruby() {
        r.push(p);
    }
    r
}
fn observe(ruby: &str, dir: &Path) -> Vec<u8> {
    let o = Command::new(ruby)
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap();
    assert!(o.status.success(), "{ruby}: {o:?}");
    o.stdout
}
#[test]
fn runtime_preserves_negative_zero_parity_values_and_line_numbers() {
    let d = project();
    let s = "result = [(-3 % 2) == 1, -2 % 2 != 1, 0 % 2 == 0, +2 % 2 != 0, 0x11 % 2 == 1, __LINE__]\np result\np __LINE__\n";
    for ruby in rubies() {
        fs::write(d.path().join("a.rb"), s).unwrap();
        let before = observe(&ruby, d.path());
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), s);
        assert_eq!(before, observe(&ruby, d.path()));
        assert_eq!(before, b"[true, true, true, false, true, 1]\n3\n");
    }
}
#[test]
fn peer_and_default_corrections_stabilize_without_oscillation() {
    let d = project();
    let s = "values = [2 % 2 == 0, 3 % 2 != 0];\np values\n";
    for only in [
        Some("Style/EvenOdd,Style/RedundantParentheses,Style/Semicolon,Layout"),
        None,
    ] {
        fs::write(d.path().join("a.rb"), s).unwrap();
        let mut stable = false;
        for _ in 0..4 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let args = if let Some(c) = only {
                vec!["--only", c, "--fix", "a.rb"]
            } else {
                vec!["--fix", "a.rb"]
            };
            cli(d.path(), &args, 0);
            let after = fs::read(d.path().join("a.rb")).unwrap();
            stable |= before == after;
        }
        assert!(stable);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "values = [2.even?, 3.odd?]\np values\n"
        );
    }
}
#[test]
fn stdin_does_not_write_even_when_fix_is_rejected() {
    let d = project();
    for (cop, s) in CASES {
        for fix in [false, true] {
            fs::write(d.path().join("a.rb"), "disk bytes\n").unwrap();
            let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            c.current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"]);
            if fix {
                c.arg("--fix");
            }
            let mut child = c
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            stdin::write_stdin(child.stdin.take().unwrap(), s.as_bytes(), fix).unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }));
            if fix {
                assert!(o.stdout.is_empty(), "{o:?}");
                assert!(
                    String::from_utf8_lossy(&o.stderr)
                        .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                    "{o:?}"
                );
            }
            assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"disk bytes\n");
        }
    }
}
