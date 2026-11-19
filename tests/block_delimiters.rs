//! exact safe corrections through the real CLI and controlled Ruby.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const COP: &str = "Style/BlockDelimiters";
const BEFORE: &str = "items.each do |x| x end\n";
const AFTER: &str = "items.each { |x| x }\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        matches!(out.status.code(), Some(0 | 1)),
        "{args:?}: {out:?}"
    );
    out
}
fn only_fix(dir: &Path) -> Output {
    cli(dir, &["--only", COP, "--fix", "--fix-passes", "1", "x.rb"])
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) {
    let path = dir.join("x.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified);
}

#[test]
fn default_only_except_config_and_no_write_contract() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for args in [vec!["x.rb"], vec!["--only", COP, "x.rb"]] {
        unchanged(dir.path(), BEFORE.as_bytes(), &args, 1);
        let report = cli(dir.path(), &args);
        assert!(String::from_utf8_lossy(&report.stdout).contains(COP));
    }
    unchanged(
        dir.path(),
        BEFORE.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "x.rb"],
        0,
    );
    for (config, args, fixes) in [
        (
            format!("[cops.\"{COP}\"]\nenabled=false\n"),
            // the independent default-on newline rule may still fix
            // keyword blocks when only BlockDelimiters is disabled.
            vec!["--except", "Style/SingleLineDoEndBlock", "--fix", "x.rb"],
            false,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled=false\n"),
            vec!["--only", COP, "--fix", "x.rb"],
            true,
        ),
        (
            format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
            vec!["--fix", "x.rb"],
            true,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(&path, BEFORE).unwrap();
        let report = cli(dir.path(), &args);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            if fixes { AFTER } else { BEFORE }
        );
        assert!(!String::from_utf8_lossy(&report.stderr).contains("error"));
    }
}

#[test]
fn default_fix_really_corrects_and_stabilizes_without_changing_public_defaults() {
    let dir = project();
    // The blank line keeps Layout/EmptyLineAfterMagicComment quiet.
    let source = "# Copyright (c) 2026 Example\n# frozen_string_literal: true\n\n[1].each do |x| puts(x) end\n";
    let expected = source.replace("do |x| puts(x) end", "{ |x| puts(x) }");
    let path = dir.path().join("x.rb");
    fs::write(&path, source).unwrap();
    cli(dir.path(), &["--fix", "x.rb"]);
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    unchanged(dir.path(), expected.as_bytes(), &["--fix", "x.rb"], 0);
}

#[test]
fn atomic_edits_respect_disabled_interior_lines_and_both_directive_prefixes() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{BEFORE}"),
            format!("items.each do |x| x end # {prefix}:disable {COP}\n"),
            format!("# {prefix}:disable-next {COP}\n{BEFORE}"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "x.rb"],
                0,
            );
        }
        // As in RuboCop, the enabled offense on line 1 is corrected even
        // though its edits reach the disabled lines.
        for (before, after) in [
            (
                format!("items.each {{ |x|\n x # {prefix}:disable {COP}\n}}\n"),
                format!("items.each do |x|\n x # {prefix}:disable {COP}\nend\n"),
            ),
            (
                format!("items.each {{ |x|\n x\n}} # {prefix}:disable {COP}\n"),
                format!("# {prefix}:disable {COP}\nitems.each do |x|\n x\nend\n"),
            ),
        ] {
            fs::write(dir.path().join("x.rb"), before).unwrap();
            only_fix(dir.path());
            assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), after);
        }
        let before = format!("# {prefix}:disable {COP}\n{BEFORE}# {prefix}:enable {COP}\n{BEFORE}");
        let after = format!("# {prefix}:disable {COP}\n{BEFORE}# {prefix}:enable {COP}\n{AFTER}");
        fs::write(dir.path().join("x.rb"), before).unwrap();
        only_fix(dir.path());
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), after);
    }
}

#[test]
fn bytes_bom_crlf_encoding_literals_data_and_legacy_withholding() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (before, after) in [
        (b"\xef\xbb\xbf# encoding: UTF-8\r\nitems.each do |x| next x end\r\np __LINE__\r\n__END__\r\nraw\xff\r\n".as_slice(), b"\xef\xbb\xbf# encoding: UTF-8\r\nitems.each { |x| next x }\r\np __LINE__\r\n__END__\r\nraw\xff\r\n".as_slice()),
        (b"# encoding: ISO-8859-1\r\neach do 1 end # caf\xe9\r\n".as_slice(), b"# encoding: ISO-8859-1\r\neach { 1 } # caf\xe9\r\n".as_slice()),
        // The atomic replacement owns text: no lossy decoding of legacy body bytes.
        (b"# encoding: ISO-8859-1\r\neach do 'caf\xe9' end\r\n".as_slice(), b"# encoding: ISO-8859-1\r\neach do 'caf\xe9' end\r\n".as_slice()),
        (b"map do puts <<~MSG end\n } do end\nMSG\n".as_slice(), b"map { puts <<~MSG }\n } do end\nMSG\n".as_slice()),
    ] {
        fs::write(&path, before).unwrap();
        only_fix(dir.path());
        assert_eq!(fs::read(&path).unwrap(), after);
    }
}

#[test]
fn current_incompatible_and_style_layout_peers_remain_atomic_single_pass() {
    let dir = project();
    let path = dir.path().join("x.rb");
    let mut names = vec![
        COP,
        "Layout/SpaceInsideBlockBraces",
        "Style/MethodCallWithArgsParentheses",
        "Style/StringLiterals",
        "Style/RedundantReturn",
        "Layout/SpaceAroundOperators",
    ];
    // These upstream incompatible peers are not yet registered. Exercise them
    // automatically when implemented; never force a planned selector today.
    let registry = rustocop::cops::Registry::new();
    for peer in ["Layout/SpaceBeforeBlockBraces", "Style/RedundantBegin"] {
        if registry.cops().iter().any(|cop| cop.name() == peer) {
            names.push(peer);
        }
    }
    names.extend(["Style/HashSyntax", "Style/SignalException"]);
    let only = names.join(",");
    let source = "def f; each{ |x|\n return x+'a'\n}; send 1 do |x| x end; end\n";
    fs::write(&path, source).unwrap();
    cli(
        dir.path(),
        &["--only", &only, "--fix", "--fix-passes", "1", "x.rb"],
    );
    let first = fs::read_to_string(&path).unwrap();
    assert_eq!(
        first,
        "def f; each do |x|\n return x+'a'\nend; send(1) do |x| x end; end\n"
    );
    // Inner overlapping fixes and the formerly command-argument block defer to
    // the next explicit invocation; the CLI does not run an automatic fix loop.
    cli(
        dir.path(),
        &["--only", &only, "--fix", "--fix-passes", "1", "x.rb"],
    );
    let second = fs::read_to_string(&path).unwrap();
    assert_eq!(
        second,
        "def f; each do |x|\n return x + \"a\"\nend; send(1) { |x| x }; end\n"
    );
    unchanged(
        dir.path(),
        second.as_bytes(),
        &["--only", &only, "--fix", "--fix-passes", "1", "x.rb"],
        0,
    );
}

#[test]
fn hash_signal_and_semicolon_overlaps_defer_whole_inner_edits() {
    let dir = project();
    let path = dir.path().join("x.rb");
    let only = format!(
        "{COP},Style/HashSyntax,Style/SignalException,Style/Semicolon,Style/StringLiterals"
    );
    let source = "each {\n fail({:a => 'x'}); side_effect\n}\n";
    fs::write(&path, source).unwrap();
    cli(
        dir.path(),
        &["--only", &only, "--fix", "--fix-passes", "1", "x.rb"],
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "each do\n fail({:a => 'x'}); side_effect\nend\n"
    );
    cli(
        dir.path(),
        &["--only", &only, "--fix", "--fix-passes", "1", "x.rb"],
    );
    let second = "each do\n raise({a: \"x\"})\n side_effect\nend\n";
    assert_eq!(fs::read_to_string(&path).unwrap(), second);
    unchanged(
        dir.path(),
        second.as_bytes(),
        &["--only", &only, "--fix", "--fix-passes", "1", "x.rb"],
        0,
    );
}

#[test]
fn nested_suppression_is_unchanged_and_exposes_the_next_explicit_pass() {
    let dir = project();
    let path = dir.path().join("x.rb");
    let source = "(0..3).each { |a| a.times {\n puts a\n}}\n";
    let first = "(0..3).each do |a| a.times {\n puts a\n} end\n";
    let second = "(0..3).each do |a| a.times do\n puts a\nend end\n";
    fs::write(&path, source).unwrap();
    let out = only_fix(dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&path).unwrap(), first);
    assert_eq!(only_fix(dir.path()).status.code(), Some(0));
    assert_eq!(fs::read_to_string(&path).unwrap(), second);
    unchanged(
        dir.path(),
        second.as_bytes(),
        &["--only", COP, "--fix", "--fix-passes", "1", "x.rb"],
        0,
    );
}
