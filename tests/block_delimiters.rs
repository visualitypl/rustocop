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
    cli(dir, &["--only", COP, "--fix", "x.rb"])
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
    let source = "# Copyright (c) 2026 Example\n# frozen_string_literal: true\n[1].each do |x| puts(x) end\n";
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
            format!("items.each {{ |x|\n x # {prefix}:disable {COP}\n}}\n"),
            format!("items.each {{ |x|\n x\n}} # {prefix}:disable {COP}\n"),
            format!("# {prefix}:disable-next {COP}\n{BEFORE}"),
        ] {
            let visible = usize::from(source.starts_with("items.each {"));
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "x.rb"],
                visible as i32,
            );
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
fn individually_withheld_clean_and_parse_errors_do_not_write() {
    let dir = project();
    for (source, status) in [
        ("foo bar do 1 end\n", 1),
        ("foo do bar; rescue; baz end\n", 1),
        ("a.+ items.map { |x|\n x\n}\n", 1),
        ("while foo {\n false\n}\n bar\nend\n", 1),
        ("map do 1 end\np DATA.pos\n__END__\nraw\n", 1),
        ("map { 1 }\n", 0),
        (
            "# each do 1 end\n'foo { bar }'\n__END__\neach do 1 end\n",
            0,
        ),
    ] {
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "x.rb"],
            status,
        );
    }
    let path = dir.path().join("x.rb");
    fs::write(&path, "each do\n").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--fix", "x.rb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(fs::read_to_string(&path).unwrap(), "each do\n");
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified);
}

fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("x.rb")
        .output()
        .unwrap()
}
#[test]
fn controlled_runtime_preserves_flow_capture_arity_binding_and_literals() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("x.rb");
    for (source, before, after, expected) in [
        (
            "def f; [1,2].each do |x| return x+8 end; :late; end\np f\n",
            "each do |x| return x+8 end",
            "each { |x| return x+8 }",
            "9\n",
        ),
        // Parenthesized sink ensures the block is map's, not p's.
        (
            "p([1,2].map do |x| next x*2 end)\n",
            "map do |x| next x*2 end",
            "map { |x| next x*2 }",
            "[2, 4]\n",
        ),
        (
            "p([1,2].map do |x| break x+3 end)\n",
            "map do |x| break x+3 end",
            "map { |x| break x+3 }",
            "4\n",
        ),
        (
            "x = 10\np([1].map { |y|\n [2].map { |z| x+y+z }\n})\n",
            "map { |y|\n [2].map { |z| x+y+z }\n}",
            "map do |y|\n [2].map { |z| x+y+z }\nend",
            "[[13]]\n",
        ),
        (
            "f = Proc.new { |x,y|\n [x,y]\n}\np [f.lambda?, f.arity, f.call(1)]\n",
            "new { |x,y|\n [x,y]\n}",
            "new do |x,y|\n [x,y]\nend",
            "[false, 2, [1, nil]]\n",
        ),
        (
            "f = ->(x,y) { [x,y].map do _1+1 end }; p [f.lambda?, f.arity, f.call(1,2)]\nbegin; f.call(1); rescue ArgumentError; p :arity; end\n",
            "map do _1+1 end",
            "map { _1+1 }",
            "[true, 2, [2, 3]]\n:arity\n",
        ),
        (
            "events = []\ndef input(events); events << :receiver; [1,2]; end\nresult = input(events).map { |x|\n events << x; x*2\n}\np [events, result]\n",
            "map { |x|\n events << x; x*2\n}",
            "map do |x|\n events << x; x*2\nend",
            "[[:receiver, 1, 2], [2, 4]]\n",
        ),
        (
            "p([1].map { |x|\n [__LINE__, x, '}end']\n}) # comment\np __LINE__\n",
            "map { |x|\n [__LINE__, x, '}end']\n}",
            "map do |x|\n [__LINE__, x, '}end']\nend",
            "[[2, 1, \"}end\"]]\n4\n",
        ),
        (
            "p([1].map do |x| <<~TEXT end)\n  #{x} } do end\nTEXT\n",
            "map do |x| <<~TEXT end",
            "map { |x| <<~TEXT }",
            "[\"1 } do end\\n\"]\n",
        ),
        (
            "def f; yield; end\ndef g; raise 'boom'; end\np(f do g rescue :rescued end)\n",
            "f do g rescue :rescued end",
            "f { g rescue :rescued }",
            ":rescued\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let observed = ruby(dir.path());
        assert!(observed.status.success(), "{observed:?}");
        assert_eq!(
            String::from_utf8_lossy(&observed.stdout),
            expected,
            "{source}"
        );
        assert!(observed.stderr.is_empty(), "{observed:?}");
        only_fix(dir.path());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source.replace(before, after)
        );
        let fixed = ruby(dir.path());
        assert_eq!(observed.status.code(), fixed.status.code());
        assert_eq!(observed.stdout, fixed.stdout);
        assert_eq!(observed.stderr, fixed.stderr);
    }
}

#[test]
fn explicit_operator_and_command_binding_and_data_have_naive_edit_witnesses() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("x.rb");
    for (source, before, after) in [
        (
            "def inner; block_given? ? yield : :no_inner_block; end\ndef outer(x); block_given? ? yield : :no_outer_block; end\nresult = outer inner do :outer_block end; p(result)\n",
            "outer inner do :outer_block end",
            "outer inner { :outer_block }",
        ),
        (
            "class A; def +(x); [x, block_given?]; end; end\ndef inner; block_given? ? yield : :no_block; end\nresult = A.new.+ inner {\n :inner_block\n}; p(result)\n",
            "inner {\n :inner_block\n}",
            "inner do\n :inner_block\nend",
        ),
        (
            "[1].each do 1 end\np DATA.pos\n__END__\nraw\n",
            "each do 1 end",
            "each { 1 }",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let observed = ruby(dir.path());
        assert!(observed.status.success(), "{observed:?}");
        only_fix(dir.path());
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(ruby(dir.path()).stdout, observed.stdout);
        fs::write(&path, source.replace(before, after)).unwrap();
        let naive = ruby(dir.path());
        assert!(naive.status.success(), "{naive:?}");
        assert_ne!(observed.stdout, naive.stdout);
    }
}

#[test]
fn splatted_command_and_super_arguments_preserve_runtime_block_ownership() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("x.rb");
    for (splat, inner_value, missing_value, arguments, expected, rebound) in [
        (
            "*",
            "[yield]",
            "[:missing_block]",
            "*xs",
            "[[:inner_block], false]\n",
            "[[:missing_block], true]\n",
        ),
        (
            "**",
            "{value: yield}",
            "{value: :missing_block}",
            "**xs",
            "[{:value=>:inner_block}, false]\n",
            "[{:value=>:missing_block}, true]\n",
        ),
    ] {
        for super_call in [false, true] {
            let definitions =
                format!("def inner; block_given? ? {inner_value} : {missing_value}; end\n");
            let source = if super_call {
                format!(
                    "{definitions}class Base\n def f({arguments}); p [xs, block_given?]; end\nend\nclass Child < Base\n def f; super {splat}inner {{\n :inner_block\n}}; end\nend\nChild.new.f\n"
                )
            } else {
                format!(
                    "{definitions}def outer({arguments}); p [xs, block_given?]; end\nouter {splat}inner {{\n :inner_block\n}}\n"
                )
            };
            fs::write(&path, &source).unwrap();
            let observed = ruby(dir.path());
            assert!(observed.status.success(), "{observed:?}: {source}");
            assert_eq!(String::from_utf8_lossy(&observed.stdout), expected);
            assert!(observed.stderr.is_empty(), "{observed:?}");
            // A withheld offense is still reported and must not rewrite even
            // the file's modification time.
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "x.rb"],
                1,
            );
            let report = only_fix(dir.path());
            assert!(String::from_utf8_lossy(&report.stdout).contains(COP));
            let fixed = ruby(dir.path());
            assert_eq!(observed.status.code(), fixed.status.code());
            assert_eq!(observed.stdout, fixed.stdout);
            assert_eq!(observed.stderr, fixed.stderr);

            // Witness that a delimiter-only edit really changes execution,
            // rather than merely failing to parse.
            let naive = source.replace("inner {\n :inner_block\n}", "inner do\n :inner_block\nend");
            fs::write(&path, naive).unwrap();
            let naive = ruby(dir.path());
            assert!(naive.status.success(), "{naive:?}");
            assert_eq!(String::from_utf8_lossy(&naive.stdout), rebound);
            assert!(naive.stderr.is_empty(), "{naive:?}");
        }
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
    cli(dir.path(), &["--only", &only, "--fix", "x.rb"]);
    let first = fs::read_to_string(&path).unwrap();
    assert_eq!(
        first,
        "def f; each do |x|\n return x+'a'\nend; send(1) do |x| x end; end\n"
    );
    // Inner overlapping fixes and the formerly command-argument block defer to
    // the next explicit invocation; the CLI does not run an automatic fix loop.
    cli(dir.path(), &["--only", &only, "--fix", "x.rb"]);
    let second = fs::read_to_string(&path).unwrap();
    assert_eq!(
        second,
        "def f; each do |x|\n return x + \"a\"\nend; send(1) { |x| x }; end\n"
    );
    unchanged(
        dir.path(),
        second.as_bytes(),
        &["--only", &only, "--fix", "x.rb"],
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
    cli(dir.path(), &["--only", &only, "--fix", "x.rb"]);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "each do\n fail({:a => 'x'}); side_effect\nend\n"
    );
    cli(dir.path(), &["--only", &only, "--fix", "x.rb"]);
    let second = "each do\n raise({a: \"x\"})\n side_effect\nend\n";
    assert_eq!(fs::read_to_string(&path).unwrap(), second);
    unchanged(
        dir.path(),
        second.as_bytes(),
        &["--only", &only, "--fix", "x.rb"],
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
        &["--only", COP, "--fix", "x.rb"],
        0,
    );
}
