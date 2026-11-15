//! Pinned RuboCop redundant_regexp_{argument,character_class,constructor}
//! specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov); fixed-policy adaptations.
use serde::Deserialize;
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
fn lint(cop: &str, s: &str) -> Vec<rustocop::offense::Offense> {
    let r = rustocop::cops::Registry::new();
    let cfg = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&r, &cfg, &[cop.into()], &[]);
    let result = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(s.as_bytes()),
        &active,
        &cfg,
    );
    assert!(
        result.errors.is_empty() && result.internal_errors.is_empty(),
        "{s}: {result:?}"
    );
    result.result.offenses
}
#[derive(Deserialize)]
struct Case {
    cop: String,
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
}
fn fixed(s: &str) -> String {
    s.replace("'f'", "\"f\"")
        .replace("'.'", "\".\"")
        .replace("'  '", "\"  \"")
}
#[test]
fn every_pinned_call_message_and_correction_is_accounted_for() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/redundant_regexps_pinned.json")).unwrap();
    assert_eq!(cases.len(), 117);
    let d = project();
    let mut canonical = [0; 3];
    let mut withheld = [0; 3];
    let mut calls = [0; 3];
    let mut discarded_classes = 0;
    for c in cases {
        let i = COPS.iter().position(|&cop| cop == c.cop).unwrap();
        calls[i] += 1;
        let found = lint(&c.cop, &c.source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "{}:{}",
            c.cop,
            c.spec_line
        );
        assert_eq!(
            found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
            c.messages
                .iter()
                .map(|m| if i == 0 { fixed(m) } else { m.clone() })
                .collect::<Vec<_>>(),
            "{}:{}",
            c.cop,
            c.spec_line
        );
        if let Some(after) = c.correction {
            // Per-offense safety exceptions, not hidden detection deviations:
            // argument: unknown receiver/match-state changes (all but scan /f/);
            // class: all original examples assign/export the Regexp value;
            // constructor: unknown interpolation effects or observable copy identity.
            let can_fix = match i {
                0 => c.spec_line == 6 || c.spec_line == 17,
                1 => false,
                2 => [5, 16, 27, 49, 60].contains(&c.spec_line),
                _ => unreachable!(),
            };
            let can_fix =
                can_fix && (i != 0 || c.source.contains(".scan(") || c.source.contains("&.scan("));
            if can_fix {
                fs::write(d.path().join("a.rb"), &c.source).unwrap();
                cli(d.path(), &["--only", &c.cop, "--fix", "a.rb"], 0);
                let after = if i == 0 { fixed(&after) } else { after.clone() };
                assert_eq!(
                    fs::read_to_string(d.path().join("a.rb")).unwrap(),
                    after,
                    "{}:{}",
                    c.cop,
                    c.spec_line
                );
                unchanged(
                    d.path(),
                    after.as_bytes(),
                    &["--only", &c.cop, "--fix", "a.rb"],
                    0,
                );
                canonical[i] += 1;
            } else {
                unchanged(
                    d.path(),
                    c.source.as_bytes(),
                    &["--only", &c.cop, "--fix", "a.rb"],
                    1,
                );
                withheld[i] += 1;
            }
            // Preserve safe exact discarded-literal counterparts independently.
            // 371 exports an inner Regexp through interpolation; 432 is dynamic.
            if i == 1 && ![371, 432].contains(&c.spec_line) {
                let before = c.source.strip_prefix("foo = ").unwrap();
                let after = after.strip_prefix("foo = ").unwrap();
                fs::write(d.path().join("a.rb"), before).unwrap();
                cli(d.path(), &["--only", &c.cop, "--fix", "a.rb"], 0);
                assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
                discarded_classes += 1;
            }
        }
    }
    assert_eq!(calls, [51, 49, 17]);
    assert_eq!(canonical, [2, 0, 5]);
    assert_eq!(withheld, [41, 26, 4]);
    assert_eq!(discarded_classes, 24);
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
fn lexical_fusion_interpolation_and_frozen_copy_witnesses_do_not_write() {
    let d = project();
    for (cop, s) in [
        (COPS[0], "/old/ =~ 'old'; 'foo'.split(/f/); p $~\n"),
        (
            COPS[0],
            "class String; def scan(x); x.class; end; end; p 'foo'.scan(/f/)\n",
        ),
        (COPS[1], r"p 'aa2'.match(/(a)\1[2]/)&.to_a"),
        (COPS[1], r#"p /[\0]1/.match?("\0\x31")"#),
        (COPS[1], r"x = '(?x)'; p /#{x}[ ]/.match?('')"),
        (
            COPS[2],
            "r = Regexp.new(/a/); r.instance_variable_set(:@x, 1); p r.frozen?\n",
        ),
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
        if Command::new("ruby").arg("--version").output().is_ok() {
            let result = Command::new("ruby")
                .arg(d.path().join("a.rb"))
                .output()
                .unwrap();
            assert!(result.status.success(), "{s}: {result:?}");
        }
    }
}
#[test]
fn singleton_escape_comment_and_nested_set_runtime_regressions() {
    let d = project();
    for (s, expected, code) in [
        (r#"p /\x1[a]/.match?("\x01a")"#, "true\n", 1),
        (r#"p /(?#\) (?-x:(?-x:\) )[ ]/x.match?("")"#, "false\n", 0),
        (r#"p /[a[-]z]/.match?("m")"#, "false\n", 1),
        (r#"p /[a[&]&b]/.match?("a")"#, "true\n", 1),
        (r#"p /[[a]-z]/.match?("m")"#, "false\n", 1),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COPS[1], "--fix", "a.rb"],
            code,
        );
        let mut runtimes = vec![];
        if Command::new("ruby").arg("--version").output().is_ok() {
            runtimes.push(std::ffi::OsString::from("ruby"));
        }
        if let Some(native) = crate::support::runtime::ruby_os() {
            runtimes.push(native);
        }
        for runtime in runtimes {
            assert_eq!(
                observe(&runtime, s.as_bytes()).stdout,
                expected.as_bytes(),
                "{s}"
            );
            assert_eq!(
                observe(&runtime, &fs::read(d.path().join("a.rb")).unwrap()).stdout,
                expected.as_bytes()
            );
        }
    }
}
#[test]
fn mixed_style_layout_stabilizes_in_two_explicit_passes() {
    let d = project();
    let source = "'foo'.scan(/[f]/); Regexp.new(/[a]/)\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    let selected = "Style/RedundantRegexpArgument,Style/RedundantRegexpCharacterClass,Style/RedundantRegexpConstructor,Style/StringLiterals,Style/Semicolon,Layout";
    cli(d.path(), &["--only", selected, "--fix", "a.rb"], 1);
    cli(d.path(), &["--only", selected, "--fix", "a.rb"], 1);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    // scan exposes MatchData#regexp; the unknown preceding call also prevents
    // proving constructor dispatch. Stabilize with those diagnostics intact.
    // Layout/IndentationConsistency aligns the split statement.
    assert_eq!(after, "\"foo\".scan(/[f]/)\nRegexp.new(/[a]/)\n");
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", selected, "--fix", "a.rb"],
        1,
    );
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
fn observe(runtime: &std::ffi::OsStr, source: &[u8]) -> Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(runtime)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(
        o.status.success(),
        "{}: {o:?}",
        String::from_utf8_lossy(source)
    );
    o
}
#[test]
fn runtime_match_results_captures_options_encodings_and_match_state_survive() {
    let d = project();
    let mut runtimes = vec![];
    if Command::new("ruby").arg("--version").output().is_ok() {
        runtimes.push(std::ffi::OsString::from("ruby"));
    }
    if let Some(native) = crate::support::runtime::ruby_os() {
        runtimes.push(native);
    }
    for method in ["scan", "sub", "gsub", "sub!", "gsub!"] {
        let tail = if method == "scan" { "" } else { ", \"x\"" };
        let s = format!("result = \"foo\".{method}(/f/{tail})\n");
        fs::write(d.path().join("a.rb"), &s).unwrap();
        cli(d.path(), &["--only", COPS[0], "--fix", "a.rb"], 0);
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert_ne!(s, after);
        let suffix = "p [result, $~.to_a, $~.begin(0), $~.end(0), $~.pre_match, $~.post_match, $~.regexp.options, $~.regexp.encoding.name, $1]\n";
        for runtime in &runtimes {
            assert_eq!(
                observe(runtime, format!("{s}{suffix}").as_bytes()).stdout,
                observe(runtime, format!("{after}{suffix}").as_bytes()).stdout
            );
        }
    }
    let s = r#"patterns = [/([a])\1/, /(?<x>[a])[\s]/iu, /[\032]/n, /[\p{Digit}]/u, %r{[/]([a])}im, /[\#]{0}/, /[\u{06F2}]/]
patterns.each do |r|
  ["aa", "a ", "2", "\x1a", "/a", "", "۲"].each do |text|
    m = r.match(text)
    p [r.options, r.encoding.name, text, m&.to_a, m && (0...m.size).map { |i| [m.begin(i), m.end(i)] }, $1]
  end
end
"#;
    fs::write(d.path().join("a.rb"), s).unwrap();
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", COPS[1], "--fix", "a.rb"],
        1,
    );
    let after = fs::read(d.path().join("a.rb")).unwrap();
    assert_eq!(s.as_bytes(), after);
    for runtime in &runtimes {
        assert_eq!(
            observe(runtime, s.as_bytes()).stdout,
            observe(runtime, &after).stdout
        );
    }
    let s = "x = \"a\"\nRegexp.new(/#{x}/io)\n";
    fs::write(d.path().join("a.rb"), s).unwrap();
    cli(d.path(), &["--only", COPS[2], "--fix", "a.rb"], 0);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_eq!(after, "x = \"a\"\n/#{x}/io\n");
    for runtime in &runtimes {
        assert_eq!(
            observe(runtime, format!("{s}p x\n").as_bytes()).stdout,
            observe(runtime, format!("{after}p x\n").as_bytes()).stdout
        );
    }
}
#[test]
fn bom_crlf_data_line_values_and_source_observers_preserve_bytes() {
    let d = project();
    for (cop, s, after) in [
        (
            COPS[0],
            "\u{feff}# coding: utf-8\r\n\"foo\".scan(/f/)\r\n__LINE__\r\n__END__\r\npayload\r\n",
            "\u{feff}# coding: utf-8\r\n\"foo\".scan(\"f\")\r\n__LINE__\r\n__END__\r\npayload\r\n",
        ),
        (
            COPS[1],
            "\u{feff}# coding: utf-8\r\n/[a]/\r\n__LINE__\r\n__END__\r\npayload\r\n",
            "\u{feff}# coding: utf-8\r\n/a/\r\n__LINE__\r\n__END__\r\npayload\r\n",
        ),
        (
            COPS[2],
            "\u{feff}# coding: utf-8\r\nRegexp.new(/a/)\r\n__LINE__\r\n__END__\r\npayload\r\n",
            "\u{feff}# coding: utf-8\r\n/a/\r\n__LINE__\r\n__END__\r\npayload\r\n",
        ),
    ] {
        fs::write(d.path().join("a.rb"), s).unwrap();
        cli(d.path(), &["--only", cop, "--fix", "a.rb"], 0);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after.as_bytes());
        for observer in ["File.binread(__FILE__)", "DATA.pos"] {
            let s = format!(
                "{}\n{observer}\n__END__\nx\n",
                s.split("__END__").next().unwrap()
            );
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
        }
    }
}
