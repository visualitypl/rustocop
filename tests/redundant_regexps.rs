//! Pinned RuboCop redundant_regexp_{argument,character_class,constructor}
//! specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov); fixed-policy adaptations.
use crate::support::cli::project;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COPS: [&str; 3] = [
    "Style/RedundantRegexpArgument",
    "Style/RedundantRegexpCharacterClass",
    "Style/RedundantRegexpConstructor",
];
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
#[test]
fn default_selectors_config_excludes_directives_and_no_write() {
    let d = project();
    for ((cop, source), after) in COPS
        .into_iter()
        .zip(["\"foo\".scan(/f/)\n", "/[a]/\n", "Regexp.new(/a/)\n"])
        .zip(["\"foo\".scan(\"f\")\n", "/a/\n", "/a/\n"])
    {
        let registry = rustocop::cops::Registry::new();
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            // Style/StringLiterals stays on: disabled, it means single quotes.
            "[cops.Style]\nenabled=false\n[cops.\"Style/StringLiterals\"]\nenabled = true\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 0);
            cli(d.path(), &["--only", cop, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"Style/StringLiterals\"]\nenabled = true\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--fix", "a.rb"], 0);
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nEnforcedStyle=\"other\"\n"),
        )
        .unwrap();
        unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 2);
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
        }
        unchanged(
            d.path(),
            after.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            0,
        );
        unchanged(d.path(), b"(\n", &["--only", cop, "--fix", "a.rb"], 2);
    }
}
#[test]
fn legacy_encoding_character_heads_and_unrepresentable_edits() {
    let d = project();
    let mut source = b"# coding: Windows-31J\np /[".to_vec();
    // Windows-31J ソ has a backslash trail byte, never regex syntax.
    source.extend_from_slice(&[0x83, 0x5c]);
    source.extend_from_slice(b"]/\n");
    unchanged(d.path(), &source, &["--only", COPS[1], "--fix", "a.rb"], 1);
    let mut source = b"# coding: Windows-31J\n/".to_vec();
    source.extend_from_slice(&[0x83, 0x5c]);
    source.extend_from_slice(b"[a]/\n");
    fs::write(d.path().join("a.rb"), &source).unwrap();
    cli(d.path(), &["--only", COPS[1], "--fix", "a.rb"], 0);
    let mut after = b"# coding: Windows-31J\n/".to_vec();
    after.extend_from_slice(&[0x83, 0x5c]);
    after.extend_from_slice(b"a/\n");
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
}
#[test]
fn stdin_is_detection_only_and_never_touches_named_file() {
    use std::io::Write;
    use std::process::Stdio;
    let d = project();
    fs::write(d.path().join("a.rb"), "sentinel\n").unwrap();
    for (cop, s) in COPS
        .into_iter()
        .zip(["\"foo\".scan(/f/)\n", "/[a]/\n", "Regexp.new(/a/)\n"])
    {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        let result = child.wait_with_output().unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&result.stdout).contains(cop));
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "sentinel\n"
        );
    }
}
