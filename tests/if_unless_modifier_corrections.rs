//! real single-pass correction pipeline/CLI and controlled Ruby witnesses.
//! Pinned examples in the companion JSON are ported from RuboCop (MIT License,
//! Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const COP: &str = "Style/IfUnlessModifier";
const BEFORE: &str = "if ready\n  work\nend\n";
const AFTER: &str = "work if ready\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], status: i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
}
fn correct(root: &Path, before: &[u8], after: &[u8], selectors: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(["--only", selectors, "--fix", "--fix-passes", "1", "a.rb"])
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(fs::read(root.join("a.rb")).unwrap(), after, "{output:?}");
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) {
    fs::write(root.join("a.rb"), source).unwrap();
    let time = fs::metadata(root.join("a.rb")).unwrap().modified().unwrap();
    cli(root, args, status);
    assert_eq!(fs::read(root.join("a.rb")).unwrap(), source);
    assert_eq!(
        fs::metadata(root.join("a.rb")).unwrap().modified().unwrap(),
        time
    );
}
#[test]
fn defaults_opt_in_config_selectors_and_incompatibilities() {
    let d = project();
    unchanged(d.path(), BEFORE.as_bytes(), &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        BEFORE.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    // Default selections include both declared incompatible peers; no findings
    // are hidden, and no implicit multipass or unsafe correction is attempted.
    unchanged(d.path(), BEFORE.as_bytes(), &["--fix", "a.rb"], 1);
    for peer in ["Style/Next", "Style/SoleNestedConditional"] {
        unchanged(
            d.path(),
            BEFORE.as_bytes(),
            &["--only", &format!("{COP},{peer}"), "--fix", "a.rb"],
            1,
        );
    }
    fs::write(d.path().join("a.rb"), BEFORE).unwrap();
    cli(
        d.path(),
        &[
            "--except",
            "Style/Next,Style/SoleNestedConditional",
            "--fix",
            "a.rb",
        ],
        0,
    );
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), AFTER);
    for config in [
        format!(
            "[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n[cops.\"{COP}\"]\nenabled=false\n"
        ),
        "[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n".into(),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), BEFORE.as_bytes(), &["--fix", "a.rb"], 0);
        correct(d.path(), BEFORE.as_bytes(), AFTER.as_bytes(), COP);
    }
    fs::write(d.path().join("rustocop.toml"), format!("[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n")).unwrap();
    fs::write(d.path().join("a.rb"), BEFORE).unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), AFTER);
}
#[test]
fn excludes_disabled_spans_and_reenabled_directives_apply_to_real_edits() {
    let d = project();
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            BEFORE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        let disabled = format!("# {prefix}:disable {COP}\n{BEFORE}# {prefix}:enable {COP}\n");
        correct(
            d.path(),
            format!("{disabled}{BEFORE}").as_bytes(),
            format!("{disabled}{AFTER}").as_bytes(),
            COP,
        );
        let inline = format!("if ready # {prefix}:disable Style/StringLiterals\n  work\nend\n");
        unchanged(
            d.path(),
            inline.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        let interior = format!("if ready\n  work # {prefix}:disable {COP}\nend\n");
        unchanged(
            d.path(),
            interior.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
}
#[test]
fn source_envelopes_line_values_and_withheld_edits_retain_exact_bytes() {
    let d = project();
    for nl in ["\n", "\r\n"] {
        for header in ["\u{feff}# encoding: UTF-8\n", "# coding: ISO-8859-1\n"] {
            let before =
                format!("{header}if ready # ordinary\n  \"café\\nvalue\"\nend\n__END__\nraw\0\n")
                    .replace('\n', nl);
            let after = format!("{header}\"café\\nvalue\" if ready # ordinary\n__END__\nraw\0\n")
                .replace('\n', nl);
            correct(d.path(), before.as_bytes(), after.as_bytes(), COP);
        }
    }
    correct(
        d.path(),
        b"# coding: ISO-8859-1\r\n# caf\xe9\r\nif ready\r\n work\r\nend\r\n__END__\r\nraw\xff\r\n",
        b"# coding: ISO-8859-1\r\n# caf\xe9\r\nwork if ready\r\n__END__\r\nraw\xff\r\n",
        COP,
    );
    correct(
        d.path(),
        b"p __LINE__\nif ready\n work\nend\n",
        b"p __LINE__\nwork if ready\n",
        COP,
    );
    correct(
        d.path(),
        b"if ready; __LINE__; end\np :done\n",
        b"__LINE__ if ready\np :done\n",
        COP,
    );
    for source in [
        b"if ready\n work\nend\np __LINE__\n".as_slice(),
        b"if ready\n __LINE__\nend\n",
        b"# coding: ISO-8859-1\r\nif ready\r\n 'caf\xe9'\r\nend\r\n",
        b"if ready\n work\nend\np DATA.pos\n__END__\nraw\n",
        b"if ready\n work\nend\np File.binread(__FILE__)\n",
        b"if value\n value = 1\nend\n",
    ] {
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    }
    unchanged(
        d.path(),
        b"if broken\n",
        &["--only", COP, "--fix", "a.rb"],
        2,
    );
    unchanged(
        d.path(),
        AFTER.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
}
#[test]
fn terminal_endless_ranges_do_not_block_unrelated_safe_edits() {
    let d = project();
    for keyword in ["if", "unless"] {
        for operator in ["..", "..."] {
            for body in [
                format!("1{operator}"),
                format!("x = 1{operator}"),
                format!("foo 1{operator}"),
            ] {
                let withheld = format!("{keyword} true\n  {body}\nend\n");
                let before = format!("{withheld}{BEFORE}");
                let after = format!("{withheld}{AFTER}");
                correct(d.path(), before.as_bytes(), after.as_bytes(), COP);
                cli(d.path(), &["--only", COP, "--fix", "a.rb"], 1);
            }
        }
    }
}
#[test]
fn operand_boundaries_preserve_native_ruby34_values_captures_and_effects() {
    let Some(program) = std::env::var_os("RUSTOCOP_RUBY34") else {
        eprintln!("set RUSTOCOP_RUBY34 for native Ruby 3.4 operand regressions");
        return;
    };
    let d = project();
    // prefix, branch value, suffix, inactive/active observations. All runtime
    // programs are controlled fixtures, not application/corpus code.
    for (prefix, body, suffix, inactive, active) in [
        ("p((", "3", " in Integer))\n", "false\n", "true\n"),
        (
            "begin\n",
            "3",
            " => Integer\n p :matched\nrescue NoMatchingPatternError\n p :unmatched\nend\n",
            ":unmatched\n",
            ":matched\n",
        ),
        (
            "p((",
            "3",
            " in Integer => x)); p x\n",
            "false\nnil\n",
            "true\n3\n",
        ),
        (
            "p((",
            "[3]",
            " in [x])); p x\n",
            "false\nnil\n",
            "true\n3\n",
        ),
        (
            "p((",
            "{v: 3}",
            " in {v: x})); p x\n",
            "false\nnil\n",
            "true\n3\n",
        ),
        (
            "begin\n",
            "[3]",
            " => [x]; p x\nrescue NoMatchingPatternError\n p :unmatched; p x\nend\n",
            ":unmatched\nnil\n",
            "3\n",
        ),
        (
            "begin\n",
            "{v: 3}",
            " => {v: x}; p x\nrescue NoMatchingPatternError\n p :unmatched; p x\nend\n",
            ":unmatched\nnil\n",
            "3\n",
        ),
        ("p([*", "3", "])\n", "[]\n", "[3]\n"),
        ("p({**", "{}", "})\n", "{}\n", "{}\n"),
        ("p(*", "3", ")\n", "", "3\n"),
        ("sink(**", "{v: 3}", ")\n", "{}\n", "{v: 3}\n"),
        ("if ", "true", "\n p :yes\nend\n", "", ":yes\n"),
        ("unless ", "true", "\n p :yes\nend\n", ":yes\n", ""),
        ("while ", "false", "\n p :loop; break\nend\n", "", ""),
        ("until ", "true", "\n p :loop; break\nend\n", ":loop\n", ""),
        ("case ", "3", "\nwhen nil then p :nil\nend\n", ":nil\n", ""),
        (
            "case ",
            "3",
            "\nin nil then p :nil\nelse p :other\nend\n",
            ":nil\n",
            ":other\n",
        ),
        ("case nil\nwhen ", "3", " then p :nil\nend\n", ":nil\n", ""),
        (
            "begin\nfor x in ",
            "[3]",
            "\n p x\nend\nrescue NoMethodError\n p :nil\nend\n",
            ":nil\n",
            "3\n",
        ),
        ("p((nil in ^(", "3", ")))\n", "true\n", "false\n"),
        ("p(([nil] in [^(", "3", ")]))\n", "true\n", "false\n"),
        ("p(({v: nil} in {v: ^(", "3", ")}))\n", "true\n", "false\n"),
        (
            "p(/(?<x>a)/ =~ ",
            "'a'",
            "); p x\n",
            "nil\nnil\n",
            "0\n\"a\"\n",
        ),
        (
            "p(defined?(",
            "3",
            "))\n",
            "\"expression\"\n",
            "\"expression\"\n",
        ),
    ] {
        for keyword in ["if", "unless"] {
            for truth in [false, true] {
                let enabled = truth == (keyword == "if");
                let header = format!(
                    "def value; $events << :body; {body}; end\ndef condition; $events << :condition; {truth}; end\ndef sink(**kw); p kw; end\n$events=[]\n"
                );
                let before =
                    format!("{header}{prefix}{keyword} condition\n value\nend{suffix}p $events\n");
                let after =
                    format!("{header}{prefix}(value {keyword} condition){suffix}p $events\n");
                correct(d.path(), before.as_bytes(), after.as_bytes(), COP);
                cli(
                    d.path(),
                    &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
                    0,
                );
                let events = if prefix.contains("defined?") {
                    "[]\n"
                } else if enabled {
                    "[:condition, :body]\n"
                } else {
                    "[:condition]\n"
                };
                let expected = format!("{}{events}", if enabled { active } else { inactive });
                for source in [&before, &after] {
                    let result = ruby(source.as_bytes(), &program);
                    assert!(result.status.success(), "{source}: {result:?}");
                    assert_eq!(result.stdout, expected.as_bytes(), "{source}: {result:?}");
                    assert!(result.stderr.is_empty(), "{source}: {result:?}");
                }
            }
        }
    }
    // The exact probes, including Ruby 3.4's nil double-splat behavior.
    for (before, after, expected) in [
        (
            "p((if false\n 3\nend in Integer))\n",
            "p(((3 if false) in Integer))\n",
            "false\n",
        ),
        ("p([*if false\n 3\nend])\n", "p([*(3 if false)])\n", "[]\n"),
        (
            "p({**if false\n 3\nend})\n",
            "p({**(3 if false)})\n",
            "{}\n",
        ),
    ] {
        correct(d.path(), before.as_bytes(), after.as_bytes(), COP);
        cli(
            d.path(),
            &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
            0,
        );
        for source in [before, after] {
            let result = ruby(source.as_bytes(), &program);
            assert!(result.status.success(), "{source}: {result:?}");
            assert_eq!(result.stdout, expected.as_bytes());
            assert!(result.stderr.is_empty());
        }
    }
}
#[test]
fn uncertain_operand_wrappers_do_not_block_unrelated_cli_fixes() {
    let d = project();
    for keyword in ["if", "unless"] {
        let withheld = format!("p(({keyword} false\n 3\nend rescue 9))\n");
        let before = format!("{withheld}{BEFORE}");
        let after = format!("{withheld}{AFTER}");
        correct(d.path(), before.as_bytes(), after.as_bytes(), COP);
        unchanged(
            d.path(),
            after.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        if let Some(program) = std::env::var_os("RUSTOCOP_RUBY34") {
            // Replace the unrelated undefined calls with controlled literals.
            let before = before.replace("ready", "true").replace("work", "p :safe");
            let after = after.replace("ready", "true").replace("work", "p :safe");
            let old = ruby(before.as_bytes(), &program);
            let new = ruby(after.as_bytes(), &program);
            assert!(
                old.status.success() && new.status.success(),
                "{old:?} {new:?}"
            );
            assert_eq!(old.stdout, new.stdout);
            assert_eq!(old.stderr, new.stderr);
        }
    }
}
#[test]
fn mixed_style_layout_edits_are_atomic_deterministic_single_passes() {
    let d = project();
    let selected = "Style/IfUnlessModifier,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/IndentationWidth,Layout/SpaceAroundOperators";
    let before = "if ready\n  call({ :key => 'value' })\nend\nx=1\n";
    let first = "call({ :key => 'value' }) if ready\nx = 1\n";
    correct(d.path(), before.as_bytes(), first.as_bytes(), selected);
    let reversed = selected.split(',').rev().collect::<Vec<_>>().join(",");
    correct(d.path(), before.as_bytes(), first.as_bytes(), &reversed);
    // The whole conditional owns the overlapping literal/hash edits, so they
    // wait for a later explicitly requested pass rather than a hidden reparse.
    let mut previous = fs::read(d.path().join("a.rb")).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", selected, "--fix", "--fix-passes", "1", "a.rb"])
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        let next = fs::read(d.path().join("a.rb")).unwrap();
        if next == previous {
            stable = true;
            break;
        }
        previous = next;
    }
    assert!(stable);
    assert_eq!(
        String::from_utf8(previous).unwrap(),
        "call({ key: \"value\" }) if ready\nx = 1\n"
    );
    for (source, first) in [
        (
            "def f(c)\n  if c\n    return ('value')\n  end\nend\n",
            "def f(c)\n  return ('value') if c\nend\n",
        ),
        (
            "def f(c)\n  if c\n    fail ('value');\n  end\nend\n",
            "def f(c)\n  fail ('value') if c\nend\n",
        ),
    ] {
        correct(d.path(), source.as_bytes(), first.as_bytes(), selected);
        correct(d.path(), source.as_bytes(), first.as_bytes(), &reversed);
        let mut previous = first.as_bytes().to_vec();
        let mut stable = false;
        for _ in 0..6 {
            let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(["--only", selected, "--fix", "--fix-passes", "1", "a.rb"])
                .output()
                .unwrap();
            assert_ne!(output.status.code(), Some(2), "{output:?}");
            let next = fs::read(d.path().join("a.rb")).unwrap();
            if next == previous {
                stable = true;
                break;
            }
            previous = next;
        }
        assert!(stable, "{source}");
    }
}
fn ruby(source: &[u8], program: &std::ffi::OsStr) -> std::process::Output {
    let mut child = Command::new(program)
        .arg("--disable-gems")
        .arg("-W0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn controlled_values_binding_effect_order_guards_rescue_and_ensure_match_ruby() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    let mut programs = vec![std::ffi::OsString::from("ruby")];
    if let Some(native) = std::env::var_os("RUSTOCOP_RUBY34") {
        programs.push(native);
    }
    for source in [
        "def f(c)\n 1..if c\n  3\n end\nend\np [f(false),f(true)]\n",
        "[false,true].each { |c| log=[]; p [1,2].map { |x| if if (log << :condition; c)\n (log << :body; true).itself\nend..true\n :yes\nelse\n :no\nend }; p log }\n",
        "[false,true].each { |c| log=[]; p [1,2].map { |x| if true..unless (log << :condition; c)\n (log << :body; true).itself\nend\n :yes\nelse\n :no\nend }; p log }\n",
        "def f(c)\n if c\n  1\n end..3\nend\np [f(false),f(true)]\n",
        "def f(c)\n 1...unless c\n  3\n end\nend\np [f(false),f(true)]\n",
        "def f(c)\n unless c\n  1\n end...3\nend\np [f(false),f(true)]\n",
        "def f(c,log)\n if (log << :condition; c)\n  log << :body\n end\nend\nl=[];p [f(false,l),f(true,l),l]\n",
        "def f(c,log)\n x = unless (log << :condition; c)\n  log.push(:body).then { 42 }\n end\n [x,log]\nend\np [f(false,[]),f(true,[])]\n",
        "def f(c,log)\n if c\n  return (log << :return; 42)\n end\n :late\nensure\n log << :ensure\nend\nl=[]; p [f(false,l),f(true,l),l]\n",
        "def f(a,b,log)\n if a\n  if b\n   return (log << :inner; 7)\n  end\n else\n  log << :else\n end\n :late\nend\nl=[]; p [f(false,true,l),f(true,false,l),f(true,true,l),l]\n",
        "log=[]; p [false,true].map { |c|\n if c\n  next (log << :next; 7)\n end\n :late\n}; p log\n",
        "def f(c,log)\n begin\n  if c\n   raise 'controlled'\n  end\n rescue RuntimeError\n  log << :rescue\n ensure\n  log << :ensure\n end\nend\nl=[];p [f(false,l),f(true,l),l]\n",
        "def f(c)\n if c\n  x = ['café', \"a\\nb\", /a\\nb/.source, %q(a b)]\n end\n [x,defined?(x)]\nend\np [f(false),f(true)]\n",
        "def f(c)\n value=:custom\n unless defined?(value)\n  value=:default\n end\n if c\n  value = :changed\n end\n value\nend\np [f(false),f(true)]\n",
        "def sink(value:); yield(value); end\ndef f(c,log)\n value=7\n if c\n  sink value: do |x| log << x; end\n end\nend\nl=[];p [f(false,l),f(true,l),l]\n",
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        let after = fs::read(d.path().join("a.rb")).unwrap();
        assert_ne!(after, source.as_bytes(), "must really correct: {source}");
        for program in &programs {
            let old = ruby(source.as_bytes(), program);
            let new = ruby(&after, program);
            assert!(
                old.status.success() && new.status.success(),
                "{program:?}: {old:?} {new:?}"
            );
            assert_eq!(old.stdout, new.stdout, "{program:?}: {source}");
            assert_eq!(old.stderr, new.stderr, "{program:?}: {source}");
        }
    }
}
#[test]
fn lexical_reordering_withholds_edits_that_would_change_ruby_bindings() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Layout/LineLength\"]\nmax=15\n",
    )
    .unwrap();
    for (source, unsafe_after, max) in [
        (
            "def value; true; end\nif value\n value = 1\nend\np value\n",
            "def value; true; end\nvalue = 1 if value\np value\n",
            80,
        ),
        (
            "def value; true; end\nvalue = 1 if value\np value\n",
            "def value; true; end\nif value\n value = 1\nend\np value\n",
            15,
        ),
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"Layout/LineLength\"]\nmax={max}\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        let original = ruby(source.as_bytes(), std::ffi::OsStr::new("ruby"));
        let unsafe_result = ruby(unsafe_after.as_bytes(), std::ffi::OsStr::new("ruby"));
        assert!(original.status.success() && unsafe_result.status.success());
        assert_ne!(original.stdout, unsafe_result.stdout);
    }
}
#[test]
fn native_34_implicit_it_and_numbered_parameters_keep_their_binding() {
    let Some(program) = std::env::var_os("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = project();
    for parameter in ["it", "_1"] {
        let source = format!("p [false,true].map {{\n if {parameter}\n  42\n end\n}}\n");
        let after = format!("p [false,true].map {{\n 42 if {parameter}\n}}\n");
        correct(d.path(), source.as_bytes(), after.as_bytes(), COP);
        let old = ruby(source.as_bytes(), &program);
        let new = ruby(after.as_bytes(), &program);
        assert!(
            old.status.success() && new.status.success(),
            "{old:?} {new:?}"
        );
        assert_eq!(old.stdout, new.stdout);
        assert_eq!(old.stdout, b"[nil, 42]\n");
    }
}
#[test]
fn long_modifier_expansion_preserves_observations_and_is_stable() {
    let d = project();
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Layout/LineLength\"]\nmax=30\n",
    )
    .unwrap();
    let source = "def f(condition,log)\n log << :body if (log << :condition; condition)\nend\nl=[];p [f(false,l),f(true,l),l]\n";
    let after = "def f(condition,log)\n if (log << :condition; condition)\n   log << :body\n end\nend\nl=[];p [f(false,l),f(true,l),l]\n";
    correct(d.path(), source.as_bytes(), after.as_bytes(), COP);
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    if Command::new("ruby").arg("--version").output().is_ok() {
        let old = ruby(source.as_bytes(), std::ffi::OsStr::new("ruby"));
        let new = ruby(after.as_bytes(), std::ffi::OsStr::new("ruby"));
        assert!(
            old.status.success() && new.status.success(),
            "{old:?} {new:?}"
        );
        assert_eq!(old.stdout, new.stdout);
    }
}
