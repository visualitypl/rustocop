//! Only controlled Ruby programs are executed, never corpus code.
use crate::support::cli::project;
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 3] = [
    "Style/MinMax",
    "Style/MinMaxComparison",
    "Style/RedundantMinMaxBy",
];
const SOURCES: [&str; 3] = [
    "[[3, 1, 2].min, [3, 1, 2].max]\n",
    "a > b ? a : b\n",
    "[3, 1, 2].min_by { |x| x }\n",
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
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
#[test]
fn fixed_defaults_capabilities_and_default_cli_detection() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let c = r.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), i != 1);
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(c.path_policy().is_none());
        let o = unchanged(d.path(), SOURCES[i].as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
    }
}
#[test]
fn selectors_config_exclusions_and_rejected_style_options() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let s = SOURCES[i].as_bytes();
        unchanged(d.path(), s, &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            s,
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 0);
            unchanged(d.path(), s, &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        assert!(
            String::from_utf8_lossy(&unchanged(d.path(), s, &["a.rb"], 1).stdout).contains(cop)
        );
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s, &["--only", cop, "--fix", "a.rb"], 0);
        }
        for knob in [
            "EnforcedStyle=\"other\"",
            "SafeAutoCorrect=true",
            "TargetRubyVersion=3.4",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{knob}\n"),
            )
            .unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_and_clean_error_files_never_write() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for prefix in ["rubocop", "rustocop"] {
            for target in [*cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{}", SOURCES[i]),
                    format!("# {prefix}:disable-next {target}\n{}", SOURCES[i]),
                    format!("{} # {prefix}:disable {target}\n", SOURCES[i].trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
            let s = format!(
                "# {prefix}:disable {cop}\n{}# {prefix}:enable {cop}\n{}",
                SOURCES[i], SOURCES[i]
            );
            let o = unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (s, code) in [("p :clean\n", 0), ("(\n", 2)] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
    unchanged(
        d.path(),
        b"[1].min_by do |x|\n x # rustocop:disable Style/RedundantMinMaxBy\nend\n",
        &["--only", COPS[2], "--fix", "a.rb"],
        0,
    );
}
#[test]
fn stdin_is_read_only_and_fix_is_rejected() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
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
            stdin::write_stdin(child.stdin.take().unwrap(), SOURCES[i].as_bytes(), fix).unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }));
            if fix {
                assert!(String::from_utf8_lossy(&o.stderr).contains("cannot be used with '--fix'"));
            } else {
                assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
            }
            assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"disk bytes\n");
        }
    }
}
fn rubies() -> Vec<String> {
    let mut r = vec![];
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        r.push("ruby".into());
    }
    if let Ok(p) = crate::support::runtime::ruby() {
        let o = Command::new(&p)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(
            o.status.success()
                && o.stdout
                    .starts_with(crate::support::runtime::version_prefix().as_bytes())
        );
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
    assert!(o.stderr.is_empty(), "{o:?}");
    o.stdout
}
#[test]
fn canonical_fixes_preserve_numeric_values_identity_binding_and_control_flow() {
    let d = project();
    let s = "a = [3,1,2].min, [3,1,2].max\nb = [3,1,2].min_by { |x| x }\nc = [3,1,2].max_by { _1 }\nd = [3,1,2].minmax_by do |item|\n item\nend\np [a, b, c, d, b.equal?(1), c.equal?(3)]\n";
    fs::write(d.path().join("a.rb"), s).unwrap();
    let expected = b"[[1, 3], 1, 3, [1, 3], true, true]\n";
    for ruby in rubies() {
        assert_eq!(observe(&ruby, d.path()), expected);
    }
    cli(d.path(), &["--only", &COPS.join(","), "--fix", "a.rb"], 0);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(!after.contains("_by"));
    assert!(after.contains("a = [3,1,2].minmax"));
    for ruby in rubies() {
        assert_eq!(observe(&ruby, d.path()), expected);
    }
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", &COPS.join(","), "--fix", "a.rb"],
        0,
    );
    // Keep the return keyword and script exit, not merely a parseable expression.
    let source = "return [3,1].min, [3,1].max\nraise 'unreachable'\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    let return_output = |ruby: &str| {
        let o = Command::new(ruby)
            .current_dir(d.path())
            .arg("a.rb")
            .output()
            .unwrap();
        assert!(o.status.success() && o.stdout.is_empty(), "{o:?}");
        o.stderr // Ruby 3.4 warns that a top-level return value is ignored.
    };
    let original: Vec<_> = rubies().iter().map(|r| return_output(r)).collect();
    cli(d.path(), &["--only", COPS[0], "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "return [3,1].minmax\nraise 'unreachable'\n"
    );
    assert_eq!(
        rubies()
            .iter()
            .map(|r| return_output(r))
            .collect::<Vec<_>>(),
        original
    );
}
