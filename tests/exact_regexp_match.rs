//! Pinned RuboCop exact_regexp_match_spec.rb examples (MIT,
//! Copyright (c) 2012-26 Bozhidar Batsov), CLI policy and semantic witnesses.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/ExactRegexpMatch";
const SOURCE: &str = "\"x\".match?(/\\Ax\\z/)\n";
const AFTER: &str = "\"x\" == 'x'\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    o
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
}
fn lint(source: &str) -> Vec<rustocop::offense::Offense> {
    let r = rustocop::cops::Registry::new();
    let cfg = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&r, &cfg, &[COP.into()], &[]);
    let result = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(source.as_bytes()),
        &active,
        &cfg,
    );
    assert!(
        result.errors.is_empty() && result.internal_errors.is_empty(),
        "{source}: {result:?}"
    );
    result.result.offenses
}
#[test]
fn defaults_selectors_config_and_atomic_canonical_fix() {
    let registry = rustocop::cops::Registry::new();
    let c = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled() && c.safe_fixes());
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(c.path_policy().is_none());
    let d = project();
    unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
    unchanged(d.path(), SOURCE.as_bytes(), &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled=false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled=false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 0);
        unchanged(d.path(), SOURCE.as_bytes(), &["--only", COP, "a.rb"], 1);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
    cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), AFTER);
    unchanged(
        d.path(),
        AFTER.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
}
#[test]
fn excludes_directives_clean_errors_and_strict_settings_preserve_bytes() {
    let d = project();
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    for knob in [
        "EnforcedStyle=\"other\"",
        "SafeAutoCorrect=true",
        "TargetRubyVersion=3.4",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{knob}\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 2);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "ExactRegexpMatch", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                format!("{} # {prefix}:disable {target}\n", SOURCE.trim_end()),
            ] {
                unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 0);
            }
        }
        let s = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        let found = lint(&s);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 4);
    }
    for (s, code) in [("p :clean\n", 0), ("(\n", 2)] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
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
    if let Ok(ruby) = crate::support::runtime::ruby() {
        r.push(ruby);
    }
    r
}
fn observe(ruby: &str, s: &str) -> Vec<u8> {
    let o = Command::new(ruby).args(["-e", s]).output().unwrap();
    assert!(o.status.success() && o.stderr.is_empty(), "{s}: {o:?}");
    o.stdout
}
#[test]
fn canonical_fixes_preserve_runtime_and_peer_fixes_stabilize() {
    for only in [
        Some(COP.to_string()),
        Some(format!(
            "{COP},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
        )),
        None,
    ] {
        let d = project();
        let path = d.path().join("a.rb");
        let source = "a = 'x'.match?(/\\Ax\\z/); b = 'y'.match?(/\\Ax\\z/); p [a,b,$~,__LINE__]\n";
        fs::write(&path, source).unwrap();
        let mut seen = std::collections::HashSet::new();
        let mut stable = false;
        for _ in 0..5 {
            let before = fs::read(&path).unwrap();
            assert!(seen.insert(before.clone()));
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command.current_dir(d.path());
            if let Some(only) = &only {
                command.args(["--only", only]);
            }
            let o = command.args(["--fix", "a.rb"]).output().unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            let after = fs::read_to_string(&path).unwrap();

            for ruby in rubies() {
                assert_eq!(observe(&ruby, source), observe(&ruby, &after));
            }
            if before == after.as_bytes() {
                assert!(!after.contains("match?"), "{after}");
                stable = true;
                break;
            }
        }
        assert!(stable);
    }
}
