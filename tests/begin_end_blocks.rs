//! CLI policy, no-write checks and controlled Ruby exit-hook witnesses.
//! Pinned examples ported from RuboCop (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

const BEGIN: &str = "Style/BeginBlock";
const END: &str = "Style/EndBlock";
const BOTH: &str = "Style/BeginBlock,Style/EndBlock";
const SOURCE: &str = "BEGIN { work }\nEND { work }\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, counts: [usize; 2]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    for (cop, count) in [BEGIN, END].into_iter().zip(counts) {
        assert_eq!(
            text.lines()
                .filter(|l| l.ends_with(&format!("({cop})")))
                .count(),
            count,
            "{args:?}: {text}"
        );
    }
    out
}
fn unchanged(dir: &Path, args: &[&str], source: &[u8], status: i32, counts: [usize; 2]) {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir, args, status, counts);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn defaults_config_selectors_and_detection_mode_never_write() {
    let dir = project();
    for (config, args, status, counts) in [
        (String::new(), vec!["a.rb"], 1, [1, 1]),
        (String::new(), vec!["--only", BEGIN, "a.rb"], 1, [1, 0]),
        (String::new(), vec!["--only", END, "a.rb"], 1, [0, 1]),
        (
            String::new(),
            vec!["--only", BOTH, "--except", BEGIN, "a.rb"],
            1,
            [0, 1],
        ),
        (
            String::new(),
            vec!["--only", BOTH, "--except", BOTH, "a.rb"],
            0,
            [0, 0],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["a.rb"],
            0,
            [0, 0],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", BOTH, "a.rb"],
            1,
            [1, 1],
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{END}\"]\nenabled = true\n"),
            vec!["a.rb"],
            1,
            [0, 1],
        ),
        (
            format!("[cops.\"{BEGIN}\"]\nenabled = false\n[cops.\"{END}\"]\nenabled = false\n"),
            vec!["a.rb"],
            0,
            [0, 0],
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(dir.path(), &args, SOURCE.as_bytes(), status, counts);
    }
}
#[test]
fn excludes_and_both_directive_prefixes_suppress_reports_and_edits() {
    let dir = project();
    for (cop, keyword) in [(BEGIN, "BEGIN"), (END, "END")] {
        // END must be genuinely correction-eligible before suppression.
        let eligible = format!("{keyword} {{ :work }}\n");
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['a.rb']\n"),
            "exclude = ['a.rb']\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                &["--only", cop, "--fix", "a.rb"],
                eligible.as_bytes(),
                0,
                [0, 0],
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for source in [
                format!("# {prefix}:disable {cop}\n{keyword} {{ :work }}\n"),
                format!("# {prefix}:disable-next {cop}\n{keyword} {{ :work }}\n"),
                format!("{keyword} {{ :work }} # {prefix}:disable {cop}\n"),
                format!("# {prefix}:disable Style\n{keyword} {{ :work }}\n"),
            ] {
                unchanged(
                    dir.path(),
                    &["--only", cop, "--fix", "a.rb"],
                    source.as_bytes(),
                    0,
                    [0, 0],
                );
            }
            let mut counts = [0, 0];
            counts[usize::from(cop == END)] = 1;
            // Only the keyword is highlighted; a closing-line suppression is unrelated.
            let source = format!("{keyword} {{\n work\n}} # {prefix}:disable {cop}\n");
            unchanged(
                dir.path(),
                &["--only", cop, "a.rb"],
                source.as_bytes(),
                1,
                counts,
            );
        }
    }
}
#[test]
fn stdin_reports_exact_keyword_positions_without_writes_and_rejects_fix() {
    let dir = project();
    let path = dir.path().join("a.rb");
    fs::write(&path, "untouched\n").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", BOTH, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), SOURCE.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("a.rb:1:1") && text.contains(BEGIN));
    assert!(text.contains("a.rb:2:1") && text.contains(END));
    run(
        dir.path(),
        &["--only", BOTH, "--fix", "--stdin", "a.rb"],
        2,
        [0, 0],
    );
    assert_eq!(fs::read(&path).unwrap(), b"untouched\n");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn all_three_pinned_examples_withheld_upstream_edit_and_canonical_adaptation_use_real_cli() {
    let dir = project();
    // begin_block_spec:5; end_block_spec:5 and :16. No config/version skips.
    unchanged(
        dir.path(),
        &["--only", BEGIN, "--fix", "a.rb"],
        b"BEGIN { test }\n",
        1,
        [1, 0],
    );
    unchanged(
        dir.path(),
        &["--only", END, "a.rb"],
        b"END { test }\n",
        1,
        [0, 1],
    );
    unchanged(
        dir.path(),
        &["--only", END, "--fix", "a.rb"],
        b"END { test }\n",
        1,
        [0, 1],
    );
    fs::write(dir.path().join("a.rb"), "END { :test }\n").unwrap();
    run(dir.path(), &["--only", END, "--fix", "a.rb"], 0, [0, 0]);
    assert_eq!(
        fs::read(dir.path().join("a.rb")).unwrap(),
        b"at_exit { :test }\n"
    );
    unchanged(
        dir.path(),
        &["--only", END, "--fix", "a.rb"],
        b"at_exit { :test }\n",
        0,
        [0, 0],
    );
    unchanged(
        dir.path(),
        &["--only", END, "--fix", "a.rb"],
        b"end_block { test }\n",
        0,
        [0, 0],
    );
}
#[test]
fn disabled_uncorrectable_clean_invalid_and_fixed_setting_errors_do_not_write() {
    let dir = project();
    unchanged(
        dir.path(),
        &["--only", BOTH, "--fix", "a.rb"],
        SOURCE.as_bytes(),
        1,
        [1, 1],
    );
    unchanged(
        dir.path(),
        &["--only", BOTH, "--fix", "a.rb"],
        b"at_exit { work }\n",
        0,
        [0, 0],
    );
    unchanged(
        dir.path(),
        &["--only", BOTH, "--fix", "a.rb"],
        b"END {\n",
        2,
        [0, 0],
    );
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.Style]\nenabled = false\n",
    )
    .unwrap();
    unchanged(dir.path(), &["--fix", "a.rb"], SOURCE.as_bytes(), 0, [0, 0]);
    for cop in [BEGIN, END] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenforced_style = 'other'\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            &["--only", cop, "--fix", "a.rb"],
            SOURCE.as_bytes(),
            2,
            [0, 0],
        );
    }
}
#[test]
fn token_edit_preserves_bom_crlf_encoding_literal_line_and_data_bytes() {
    let dir = project();
    for source in [
        "\u{feff}# coding: UTF-8\r\nEND { ['é',__LINE__,__ENCODING__] } # kept\r\n__END__\r\nEND {}\r\n".as_bytes(),
        b"# coding: ISO-8859-1\r\nEND { '\xe9' }\r\np __LINE__\r\n__END__\r\n\xff\x00\r\n",
        b"END { <<~TEXT }\n  END {}\nTEXT\n",
    ] {
        unchanged(dir.path(), &["--only",END,"a.rb"],source,1,[0,1]);
        let start = source.windows(3).position(|w| w == b"END").unwrap();
        let mut expected = source[..start].to_vec();
        expected.extend_from_slice(b"at_exit");
        expected.extend_from_slice(&source[start+3..]);
        run(dir.path(), &["--only",END,"--fix","a.rb"],0,[0,0]);
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(),expected);
    }
}
#[test]
fn mixed_style_layout_passes_are_deterministic_and_stabilize() {
    let dir = project();
    let selected = "Style/BeginBlock,Style/EndBlock,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    fs::write(dir.path().join("a.rb"), "END { 'x' }\n").unwrap();
    run(
        dir.path(),
        &["--only", selected, "--fix", "a.rb"],
        0,
        [0, 0],
    );
    let bytes = fs::read(dir.path().join("a.rb")).unwrap();
    assert_eq!(bytes, b"at_exit { \"x\" }\n");
    unchanged(
        dir.path(),
        &["--only", selected, "--fix", "a.rb"],
        &bytes,
        0,
        [0, 0],
    );
}
fn runtimes() -> Vec<String> {
    let mut result = vec![];
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        result.push("ruby".into());
    }
    if let Ok(native) = crate::support::runtime::ruby()
        && !result.contains(&native)
    {
        result.push(native);
    }
    result
}
fn ruby(dir: &Path, runtime: &str, source: &str) -> Output {
    fs::write(dir.join("witness.rb"), source).unwrap();
    Command::new(runtime)
        .current_dir(dir)
        .arg("witness.rb")
        .output()
        .unwrap()
}
#[test]
fn corrected_closed_hooks_preserve_literals_surrounding_rescue_ensure_and_exit_status() {
    let dir = project();
    for (source, expected, status) in [
        ("END { :first }\nEND { :second }\nputs :body\n", "body\n", 0),
        (
            "x = 1\nEND { [x,__LINE__,\"é\\n雪\"] }\nx = 2\np [x,__LINE__,\"é\\n雪\"]\n",
            "[2, 4, \"é\\n雪\"]\n",
            0,
        ),
        (
            "END { begin; :body; ensure; :ensured; end }\nputs :body\n",
            "body\n",
            0,
        ),
        (
            "END { :exit }\nbegin; raise Exception, 'outer'; rescue StandardError; puts :wrong; rescue Exception; puts :outer; ensure; puts :ensure; end\n",
            "outer\nensure\n",
            0,
        ),
        (
            "END { :exit }\nbegin; exit 7; ensure; puts :ensure; end\n",
            "ensure\n",
            7,
        ),
    ] {
        for runtime in runtimes() {
            let before = ruby(dir.path(), &runtime, source);
            assert_eq!(before.status.code(), Some(status), "{runtime}: {before:?}");
            assert_eq!(before.stdout, expected.as_bytes(), "{runtime}: {before:?}");
            fs::write(dir.path().join("a.rb"), source).unwrap();
            run(dir.path(), &["--only", END, "--fix", "a.rb"], 0, [0, 0]);
            let fixed = fs::read_to_string(dir.path().join("a.rb")).unwrap();
            assert_ne!(
                fixed, source,
                "positive runtime witness must actually correct"
            );
            let after = ruby(dir.path(), &runtime, &fixed);
            assert_eq!(
                (before.status.code(), before.stdout, before.stderr),
                (after.status.code(), after.stdout, after.stderr),
                "{runtime}: {source}"
            );
        }
    }
}
#[test]
fn naive_rewrites_change_registration_locals_returns_or_dispatch_and_are_withheld() {
    let dir = project();
    for source in [
        "def f; END { puts :exit }; end\nf; f\n",
        "f = proc { END { puts :exit } }; f.call; f.call\n",
        "f = -> { END { puts :exit } }; f.call; f.call\n",
        "p proc { END { puts :exit } }.call.class\n",
        "p -> { END { puts :exit } }.call.class\n",
        "END { x = 1 }; p defined?(x)\n",
        "END { 1 => x }; p defined?(x)\n",
        "END { /(?<x>x)/ =~ 'x' }; p defined?(x)\n",
        "def at_exit; yield; end\nEND { puts :exit }; puts :body\n",
    ] {
        for runtime in runtimes() {
            let before = ruby(dir.path(), &runtime, source);
            let naive = ruby(dir.path(), &runtime, &source.replace("END {", "at_exit {"));
            assert!(
                before.status.success() && naive.status.success(),
                "{runtime}: {before:?} {naive:?}"
            );
            assert_ne!(
                before.stdout, naive.stdout,
                "{runtime}: counterexample must be observable: {source}"
            );
        }
        unchanged(
            dir.path(),
            &["--only", END, "--fix", "a.rb"],
            source.as_bytes(),
            1,
            [0, 1],
        );
    }
}
#[test]
fn begin_timing_and_end_backtrace_rescue_scope_witnesses_stay_unchanged() {
    let dir = project();
    for (source, counts) in [
        ("puts :body\nBEGIN { puts :begin }\n", [1, 0]),
        (
            "END { puts :first }\nEND { puts :second }\nputs :body\n",
            [0, 2],
        ),
        (
            "END { begin; raise 'bad'; rescue RuntimeError; puts :rescued; ensure; puts :ensure; end }\nputs :body\n",
            [0, 1],
        ),
        (
            "END { begin; raise Exception, 'outer'; rescue StandardError; puts :wrong; rescue Exception => e; puts e.class; puts e.backtrace.first; ensure; puts :ensure; end }\nputs :body\n",
            [0, 1],
        ),
        (
            "def f\n  END { begin; raise 'deferred'; rescue => e; p [e.class,e.backtrace.first,$!.equal?(e)]; ensure; puts :ensure; end }\nrescue\n  puts :wrong_scope\nensure\n  puts :method_ensure\nend\np f\n",
            [0, 1],
        ),
        (
            "proc { END { puts :exit }; :proc_value }.tap { |f| p f.call; p f.call }\n",
            [0, 1],
        ),
        (
            "f = -> { END { puts :exit }; :lambda_value }; p f.call; p f.call\n",
            [0, 1],
        ),
    ] {
        for runtime in runtimes() {
            let before = ruby(dir.path(), &runtime, source);
            assert!(before.status.success(), "{runtime}: {before:?}");
            unchanged(
                dir.path(),
                &["--only", BOTH, "--fix", "a.rb"],
                source.as_bytes(),
                1,
                counts,
            );
            let after = ruby(
                dir.path(),
                &runtime,
                &fs::read_to_string(dir.path().join("a.rb")).unwrap(),
            );
            assert_eq!(
                (before.status.code(), before.stdout, before.stderr),
                (after.status.code(), after.stdout, after.stderr),
                "{runtime}: {source}"
            );
        }
    }
}
#[test]
fn native_ruby34_unhandled_errors_need_no_explicit_backtrace_observer_to_differ() {
    let Ok(runtime) = crate::support::runtime::ruby() else {
        return;
    };
    let dir = project();
    for source in [
        "END { raise 'bad' }\n",
        "END { work }\ndef work; raise Exception, 'bad'; end\n",
    ] {
        let before = ruby(dir.path(), &runtime, source);
        let naive = ruby(dir.path(), &runtime, &source.replace("END {", "at_exit {"));
        assert_eq!(before.status.code(), Some(1));
        assert_eq!(naive.status.code(), Some(1));
        assert_ne!(before.stderr, naive.stderr, "{source}");
        unchanged(
            dir.path(),
            &["--only", END, "--fix", "a.rb"],
            source.as_bytes(),
            1,
            [0, 1],
        );
    }
}

#[test]
fn native_ruby34_naive_backtrace_and_implicit_it_changes_are_observable() {
    let Ok(runtime) = crate::support::runtime::ruby() else {
        return;
    };
    let dir = project();
    for source in [
        "END { begin; raise 'x'; rescue => e; p e.backtrace.first; end }\n",
        "def it; :method_value; end\nEND { p it }\n",
        "define_singleton_method(:_1) { :method_value }\nEND { p _1 }\n",
    ] {
        let before = ruby(dir.path(), &runtime, source);
        let naive = ruby(dir.path(), &runtime, &source.replace("END {", "at_exit {"));
        assert!(
            before.status.success() && naive.status.success(),
            "{before:?} {naive:?}"
        );
        assert_ne!(before.stdout, naive.stdout, "{source}");
        unchanged(
            dir.path(),
            &["--only", END, "--fix", "a.rb"],
            source.as_bytes(),
            1,
            [0, 1],
        );
    }
}
