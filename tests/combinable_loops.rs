//! real CLI controls and unsafe loop-fusion semantic witnesses.
//! Pinned fixture examples: RuboCop style/combinable_loops_spec.rb
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
use crate::support::cli::project;
use std::{fs, path::Path, process::Command};
const COP: &str = "Style/CombinableLoops";
const SOURCE: &str = "items.each { |item| foo(item) }\nitems.each { |item| bar(item) }\n";

fn cli(dir: &Path, args: &[&str], code: i32) -> std::process::Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    out
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> std::process::Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args, code);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    out
}

#[test]
fn default_registration_selectors_config_and_excludes_are_real() {
    let registry = rustocop::cops::Registry::new();
    let cops: Vec<_> = registry.cops().iter().filter(|c| c.name() == COP).collect();
    assert_eq!(cops.len(), 1);
    assert!(cops[0].default_enabled());
    assert!(!cops[0].safe_fixes());
    let d = project();
    let out = unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains(
        "a.rb:2:1: C: Combine this loop with the previous loop. (Style/CombinableLoops)"
    ));
    for only in [COP, "Style"] {
        let out = unchanged(d.path(), SOURCE.as_bytes(), &["--only", only, "a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
    }
    for except in [COP, "Style"] {
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--except", except, "--fix", "a.rb"],
            0,
        );
    }
    for config in [
        "[cops.Style]\nenabled=false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled=false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let out = cli(d.path(), &["a.rb"], 0);
        assert!(!String::from_utf8_lossy(&out.stdout).contains(COP));
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    assert!(String::from_utf8_lossy(&cli(d.path(), &["a.rb"], 1).stdout).contains(COP));
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
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle=\"other\"\n"),
    )
    .unwrap();
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        2,
    );
}

#[test]
fn directives_disable_whole_loop_ranges_and_reenable_without_losing_siblings() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("{} # {prefix}:disable {target}\n", SOURCE.trim_end()),
                format!(
                    "items.each {{ foo }}\n# {prefix}:disable-next {target}\nitems.each {{ bar }}\n"
                ),
                format!(
                    "items.each {{ foo }}\nitems.each do\n bar\nend # {prefix}:disable {target}\n"
                ),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
            let source = format!(
                "# {prefix}:disable {target}\n{SOURCE}# {prefix}:enable {target}\nitems.each {{ baz }}\n"
            );
            let out = unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                1,
            );
            assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:5:1:"));
        }
    }
}

#[derive(serde::Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    correction: Option<String>,
}
#[test]
fn all_26_unsafe_pinned_correction_calls_remain_reported_but_unedited() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/combinable_loops_pinned.json")).unwrap();
    let d = project();
    let mut count = 0;
    for case in cases.into_iter().filter(|c| c.correction.is_some()) {
        count += 1;
        let out = unchanged(
            d.path(),
            case.source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(COP),
            "spec {}",
            case.spec_line
        );
    }
    assert_eq!(count, 26);
}

#[test]
fn source_envelopes_data_line_literals_clean_and_errors_are_never_written() {
    let d = project();
    for source in [
        format!("\u{feff}{}", SOURCE.replace('\n', "\r\n")).into_bytes(),
        format!("# coding: us-ascii\r\n{}p __LINE__\r\n", SOURCE.replace('\n', "\r\n")).into_bytes(),
        [b"# coding: windows-1252\n# caf\xe9\n".as_slice(), SOURCE.as_bytes(), b"__END__\nraw\0payload\r\n"].concat(),
        b"items.each { puts <<~TEXT }\n  first\nTEXT\nitems.each { puts <<~TEXT }\n  second\nTEXT\n".to_vec(),
    ] {
        let out = unchanged(d.path(), &source, &["--only", COP, "--fix", "a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
    }
    unchanged(
        d.path(),
        b"items.each {}; items.each {}\n",
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    unchanged(
        d.path(),
        b"items.each {\n",
        &["--only", COP, "--fix", "a.rb"],
        2,
    );
}

use crate::support::stdin as stdin_support;
#[test]
fn stdin_reports_without_writing_and_rejects_fix() {
    let d = project();
    fs::write(d.path().join("a.rb"), b"# sentinel\n").unwrap();
    for fix in [false, true] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
        cmd.current_dir(d.path())
            .args(["--only", COP, "--stdin", "a.rb"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if fix {
            cmd.arg("--fix");
        }
        let mut child = cmd.spawn().unwrap();
        stdin_support::write_stdin(child.stdin.take().unwrap(), SOURCE.as_bytes(), fix).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(if fix { 2 } else { 1 }), "{out:?}");
        if fix {
            assert!(out.stdout.is_empty(), "{out:?}");
            assert!(
                String::from_utf8_lossy(&out.stderr)
                    .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                "{out:?}"
            );
        }
        if !fix {
            assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:2:1:"));
        }
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"# sentinel\n");
    }
}

#[test]
fn controlled_ruby_witnesses_explain_why_fusion_and_for_to_each_are_unsafe() {
    let d = project();
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let run = |source: &str| {
            let out = Command::new(&ruby)
                .args(["--disable-gems", "-e", source])
                .output()
                .unwrap();
            assert!(out.status.success(), "{source}: {out:?}");
            out.stdout
        };
        for (before, unsafe_after) in [
            (
                "log=[]; items=[1,2]; items.each { |i| log << [:a,i] }; items.each { |i| log << [:b,i] }; p log",
                "log=[]; items=[1,2]; items.each { |i| log << [:a,i]; log << [:b,i] }; p log",
            ),
            (
                "def f; items=[1,2]; items.each { break :first }; items.each { break :second }; end; p f",
                "def f; items=[1,2]; items.each { break :first; break :second }; end; p f",
            ),
            (
                "log=[]; items=[1,2]; items.each { |i| next if i==1; log << :a }; items.each { log << :b }; p log",
                "log=[]; items=[1,2]; items.each { |i| next if i==1; log << :a; log << :b }; p log",
            ),
            (
                "log=[]; again=true; items=[1]; items.each { log << :a }; items.each { log << :b; if again; again=false; redo; end }; p log",
                "log=[]; again=true; items=[1]; items.each { log << :a; log << :b; if again; again=false; redo; end }; p log",
            ),
            (
                "log=[]; items=[1,2]; begin; items.each { |i| log << [:a,i] }; items.each { |i| log << [:b,i]; raise 'stop' }; rescue; log << :rescued; ensure; log << :ensured; end; p log",
                "log=[]; items=[1,2]; begin; items.each { |i| log << [:a,i]; log << [:b,i]; raise 'stop' }; rescue; log << :rescued; ensure; log << :ensured; end; p log",
            ),
            (
                "x=:outer; y=:outer; log=[]; items=[1,2]; for x in items; log << x; end; for y in items; log << y; end; p [x,y,log]",
                "x=:outer; y=:outer; log=[]; items=[1,2]; items.each { |x| log << x }; items.each { |y| log << y }; p [x,y,log]",
            ),
            (
                "$n=0; def items; $n+=1; [$n]; end; log=[]; items.each { |i| log << i }; items.each { |i| log << i }; p [log,$n]",
                "$n=0; def items; $n+=1; [$n]; end; log=[]; items.each { |i| log << i; log << i }; p [log,$n]",
            ),
            (
                "log=[]; items=[1,2]; n=0; while n<1; n+=1; items.each { |i| log << [:a,i] }; items.each { |i| log << [:b,i] }; end; begin; items.each { |i| log << [:c,i] }; items.each { |i| log << [:d,i] }; end until true; p log",
                "log=[]; items=[1,2]; n=0; while n<1; n+=1; items.each { |i| log << [:a,i]; log << [:b,i] }; end; begin; items.each { |i| log << [:c,i]; log << [:d,i] }; end until true; p log",
            ),
        ] {
            let expected = run(before);
            assert_ne!(
                expected,
                run(unsafe_after),
                "witness must distinguish unsafe rewrite"
            );
            unchanged(
                d.path(),
                before.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                1,
            );
            let retained = fs::read_to_string(d.path().join("a.rb")).unwrap();
            assert_eq!(run(&retained), expected);
        }
    }
}
