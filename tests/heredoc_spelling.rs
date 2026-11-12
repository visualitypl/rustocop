//! CLI, main-parse ownership and controlled string observations.
use rustocop::cops::{
    Cop,
    style::{EmptyHeredoc, RedundantHeredocDelimiterQuotes, RedundantStringEscape},
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const E: &str = "Style/RedundantStringEscape";
const Q: &str = "Style/RedundantHeredocDelimiterQuotes";
const H: &str = "Style/EmptyHeredoc";
const ALL: &str =
    "Style/RedundantStringEscape,Style/RedundantHeredocDelimiterQuotes,Style/EmptyHeredoc";
const CASES: [(&str, &str, &str); 3] = [
    (E, "p \"\\#hi\"\n", "p \"#hi\"\n"),
    (Q, "p <<~'EOS'\n  hi\nEOS\n", "p <<~EOS\n  hi\nEOS\n"),
    (H, "p <<~EOS\nEOS\n", "p \"\"\n"),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    out
}
fn unchanged(dir: &Path, name: &str, source: &[u8], status: i32) {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir, &["--only", name, "--fix", "a.rb"], status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}
fn fixed(cop: &dyn Cop, source: &str) -> Vec<u8> {
    let dir = project();
    fs::write(dir.path().join("a.rb"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", cop.name(), "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(2), "{out:?}");
    fs::read(dir.path().join("a.rb")).unwrap()
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap()
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}

#[test]
fn registry_default_policy_lint_only_and_stdin_never_write() {
    let dir = project();
    let registry = rustocop::cops::Registry::new();
    for (name, before, _) in CASES {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled() && cop.safe_fixes());
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
        fs::write(dir.path().join("a.rb"), before).unwrap();
        let time = fs::metadata(dir.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        let out = cli(dir.path(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(name));
        cli(dir.path(), &["--only", name, "a.rb"], 1);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", name, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), before);
        assert_eq!(
            fs::metadata(dir.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            time
        );
    }
}
#[test]
fn selectors_config_and_single_pass_canonicals() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (name, before, after) in CASES {
        for config in [
            "[cops.Style]\nenabled = false\n".to_string(),
            CASES
                .iter()
                .map(|(c, _, _)| format!("[cops.\"{c}\"]\nenabled = false\n"))
                .collect(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            cli(
                dir.path(),
                &["--only", ALL, "--except", name, "--fix", "a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            cli(dir.path(), &["--only", name, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(&path).unwrap(), after);
            let time = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", name, "--fix", "a.rb"], 0);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            // This test isolates per-Style enabling, not the heredoc delimiter cops' new default
            // meaningful-label finding for its historical EOS fixture.
            format!("[cops.Style]\nenabled = false\n[cops.Naming]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
        )
        .unwrap();
        fs::write(&path, before).unwrap();
        cli(dir.path(), &["--fix", "a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }
}
#[test]
fn both_directive_prefixes_and_excludes_preserve_bytes_and_mtime() {
    let dir = project();
    for (name, before, _) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [name, name.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{before}"),
                    format!("# {prefix}:disable-next {target}\n{before}"),
                    before.replacen('\n', &format!(" # {prefix}:disable {target}\n"), 1),
                ] {
                    unchanged(dir.path(), name, source.as_bytes(), 0);
                }
            }
        }
        for config in [
            "exclude = [\"a.rb\"]\n".to_string(),
            format!("[cops.\"{name}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(dir.path(), name, before.as_bytes(), 0);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn atomic_envelopes_keep_header_tails_comments_queued_and_nested_owners() {
    for (source, expected) in [
        ("p <<~EOS, arg # tail\nEOS\n", "p \"\", arg # tail\n"),
        ("p <<~EOS\n  EOS\n", "p \"\"\n"),
        ("p <<~EOS\nEOS", "p \"\"\n"),
        ("p <<~A, <<~B\nA\ntext\nB\n", "p \"\", <<~B\ntext\nB\n"),
        ("p <<~A, <<~B\ntext\nA\nB\n", "p <<~A, \"\"\ntext\nA\n"),
        ("p <<~OUT\n#{<<~IN}\nIN\nOUT\n", "p <<~OUT\n#{\"\"}\nOUT\n"),
        (
            "p <<~OUT\n#{\n<<~IN\nIN\n}\nOUT\n",
            "p <<~OUT\n#{\n\"\"\n}\nOUT\n",
        ),
    ] {
        assert_eq!(
            String::from_utf8(fixed(&EmptyHeredoc, source)).unwrap(),
            expected,
            "{source}"
        );
    }
    for source in [
        "p <<~'EOS'\n  hi\nEOS\n",
        "p <<~\"EOS\"\n#{<<~IN}\ntext\nIN\nEOS\n",
        "p <<~OUT\n#{<<~'IN'}\ntext\nIN\nOUT\n",
    ] {
        let result = fixed(&RedundantHeredocDelimiterQuotes, source);
        assert_ne!(result, source.as_bytes(), "{source}");
        assert_eq!(ruby_prism::parse_ruby_3_4(&result).errors().count(), 0);
    }
}
#[test]
fn source_only_commands_are_never_converted_or_executed() {
    let dir = project();
    unchanged(dir.path(), ALL, b"p <<~`SH`\nSH\n[%x(\\#), `\\#`]\n", 0);
    // Real string children in command interpolation still participate.
    let source = "p <<~`SH`\n#{\"\\#\"}\nSH\n";
    let after = fixed(&RedundantStringEscape, source);
    assert_eq!(after, b"p <<~`SH`\n#{\"#\"}\nSH\n");
    for bytes in [source.as_bytes(), after.as_slice()] {
        let parse = ruby_prism::parse_ruby_3_4(bytes);
        assert_eq!(parse.errors().count(), 0);
        let arg = parse
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap()
            .as_call_node()
            .unwrap()
            .arguments()
            .unwrap()
            .arguments()
            .iter()
            .next()
            .unwrap();
        assert!(arg.as_interpolated_x_string_node().is_some());
    }
}
#[test]
fn stronger_safety_keeps_individual_offenses_clean_and_error_files_unwritten() {
    let dir = project();
    for (cop, source, status) in [
        (Q, "p <<~'123'\ntext\n123\n", 1),
        (Q, "p <<~' EOS '\ntext\n EOS \n", 1),
        (H, "p <<~EOS\nEOS\np __LINE__\n", 1),
        (
            H,
            "p <<~EOS\nEOS\n# frozen_string_literal: true\np 'x'\n",
            1,
        ),
        (H, "p <<~EOS # rubocop:disable Layout\nEOS\n", 1),
        (E, "p \"\\\\#\\{not_run}\"\n", 1),
        (ALL, "p \"hi\"\n", 0),
        (ALL, "p <<~EOS\n", 2),
    ] {
        unchanged(dir.path(), cop, source.as_bytes(), status);
    }
    for (cop, before, _) in CASES {
        for observer in [
            "p __FILE__",
            "p method(:p).source_location",
            "p errors.map(&:backtrace_locations)",
            "p File.binread($0)",
            "p RubyVM::AbstractSyntaxTree",
            "p ObjectSpace.each_object(String).count",
        ] {
            unchanged(
                dir.path(),
                cop,
                format!("{before}{observer}\n").as_bytes(),
                1,
            );
        }
        unchanged(
            dir.path(),
            cop,
            format!("{before}p DATA.pos; p DATA.read.bytes\n__END__\nraw\n").as_bytes(),
            1,
        );
    }
}
#[test]
fn bom_crlf_data_and_line_sensitive_siblings_remain_exact() {
    let dir = project();
    let before = "\u{feff}# encoding: UTF-8\r\np <<~'EOS'\r\n  café\r\nEOS\r\np \"\\#\"\r\np <<~EMPTY\r\nEMPTY\r\np __LINE__\r\n__END__\r\nraw\\payload\r\n";
    let expected = before.replace("<<~'EOS'", "<<~EOS").replace("\\#", "#");
    fs::write(dir.path().join("a.rb"), before).unwrap();
    cli(dir.path(), &["--only", ALL, "--fix", "a.rb"], 1);
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        expected
    );
    let before = "\u{feff}# encoding: UTF-8\r\np <<~EMPTY # tail\r\nEMPTY\r\n__END__\r\nraw\r\n";
    fs::write(dir.path().join("a.rb"), before).unwrap();
    cli(dir.path(), &["--only", H, "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        "\u{feff}# encoding: UTF-8\r\np \"\" # tail\r\n__END__\r\nraw\r\n"
    );
}
#[test]
fn legacy_character_boundaries_and_ascii_only_edits() {
    let dir = project();
    for (cop, before, expected) in [
        (
            E,
            b"# coding: Windows-31J\np \"\x83\x5c\\#\"\n".as_slice(),
            b"# coding: Windows-31J\np \"\x83\x5c#\"\n".as_slice(),
        ),
        (
            Q,
            b"# coding: Windows-31J\np <<~'EOS'\n\x83\x5c\nEOS\n".as_slice(),
            b"# coding: Windows-31J\np <<~EOS\n\x83\x5c\nEOS\n".as_slice(),
        ),
        (
            E,
            b"# coding: ISO-8859-1\np \"caf\xe9\\#\"\n".as_slice(),
            b"# coding: ISO-8859-1\np \"caf\xe9#\"\n".as_slice(),
        ),
    ] {
        fs::write(dir.path().join("a.rb"), before).unwrap();
        let original = has_ruby().then(|| ruby(dir.path()));
        cli(dir.path(), &["--only", cop, "--fix", "a.rb"], 0);
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), expected);
        if let Some(original) = original {
            let after = ruby(dir.path());
            assert!(original.status.success());
            assert_eq!(original.stdout, after.stdout);
            assert_eq!(original.stderr, after.stderr);
        }
    }
    unchanged(
        dir.path(),
        Q,
        b"# coding: Windows-31J\np <<~'EOS'\n\x83\x5c\\n\nEOS\n",
        0,
    );
    unchanged(dir.path(), E, b"# coding: ISO-8859-1\np \"\\\xe9\"\n", 0);
    unchanged(dir.path(), E, b"# coding: ISO-8859-1\np \"\\\xd7\"\n", 1);
}
// Encoded source bytes are deliberate: a UTF-8 String cannot reproduce trail
// bytes 0x5c/0x7d inside CP932 identifiers.
type EncodedCharacterCases = (
    &'static str,
    &'static [u8],
    &'static [u8],
    &'static [u8],
    &'static [u8],
);
const ESCAPE_ENCODINGS: [EncodedCharacterCases; 10] = [
    (
        "Windows-31J",
        b"\x83\x5c",
        b"\x81\x41",
        b"\x81\x99",
        b"\x82\x50",
    ),
    ("EUC-JP", b"\xa4\xa2", b"\xa1\xa2", b"\xa1\xf9", b"\xa3\xb1"),
    ("Big5", b"\xa4\xa4", b"\xa1\x42", b"\xa1\xb8", b"\xa2\xb0"),
    ("GBK", b"\xd6\xd0", b"\xa1\xa2", b"\xa1\xee", b"\xa3\xb1"),
    ("GB2312", b"\xd6\xd0", b"\xa1\xa2", b"\xa1\xee", b"\xa3\xb1"),
    (
        "Big5-HKSCS",
        b"\xa4\xa4",
        b"\xa1\x42",
        b"\xa1\xb8",
        b"\xa2\xb0",
    ),
    ("EUC-KR", b"\xb0\xa1", b"\xa1\xa2", b"\xa1\xd9", b"\xa3\xb1"),
    ("CP949", b"\xb0\xa1", b"\xa1\xa2", b"\xa1\xd9", b"\xa3\xb1"),
    (
        "GB18030",
        b"\xd6\xd0",
        b"\xa1\xa2",
        b"\xa1\xee",
        b"\xa3\xb1",
    ),
    (
        "UTF-8",
        "ソ".as_bytes(),
        "、".as_bytes(),
        "☆".as_bytes(),
        "１".as_bytes(),
    ),
];

#[test]
fn escaped_legacy_posix_letters_digits_and_punctuation_match_the_pin() {
    let dir = project();
    for (encoding, letter, comma, star, digit) in ESCAPE_ENCODINGS {
        for (character, status) in [(letter, 0), (digit, 0), (comma, 1), (star, 1)] {
            let mut source = format!("# encoding: {encoding}\np \"\\").into_bytes();
            source.extend_from_slice(character);
            source.extend_from_slice(b"\"\n");
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let out = cli(dir.path(), &["--only", E, "a.rb"], status);
            if status == 1 {
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(text.contains("a.rb:2:4:"), "{encoding}: {text}");
                assert!(
                    text.contains(&format!(
                        "Redundant escape of {} inside string literal.",
                        String::from_utf8_lossy(character)
                    )),
                    "{text}"
                );
            }
            // Punctuation is still detected, but non-ASCII edits are withheld.
            unchanged(dir.path(), E, &source, status);
        }
    }
}

#[test]
fn recovery_four_byte_gb18030_and_cp949_extension_keep_punctuation_visible() {
    let dir = project();
    for (encoding, character, status) in [
        ("CP949", b"\x81\x41".as_slice(), 0), // 갂, outside EUC-KR
        ("GB18030", b"\x82\x37\xcf\x35".as_slice(), 0), // 가
        ("GB18030", b"\x90\x30\xeb\x34".as_slice(), 0), // 𐐨
        ("GB18030", b"\x95\x32\x82\x36".as_slice(), 0), // 𠀀
        ("GB18030", b"\x94\x33\xec\x37".as_slice(), 0), // 𝟙 (decimal)
        ("GB18030", b"\x81\x30\x85\x35".as_slice(), 1), // ² (not decimal)
        ("GB18030", b"\x94\x39\xfc\x36".as_slice(), 1), // 😀
    ] {
        // Static, interpolated fragments, word arrays and physical heredoc bodies.
        for (left, right, line, col) in [
            ("p \"", "\"\n", 2, 4),
            ("p \"#{1}", "\"\n", 2, 8),
            ("p %W[", "]\n", 2, 6),
            ("p <<~EOS\n  ", "\nEOS\n", 3, 3),
        ] {
            let mut source = format!("# encoding: {encoding}\n{left}\\").into_bytes();
            source.extend_from_slice(character);
            source.extend_from_slice(right.as_bytes());
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let out = cli(dir.path(), &["--only", E, "a.rb"], status);
            if status == 1 {
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(text.contains(&format!("a.rb:{line}:{col}:")), "{text}");
                assert!(
                    text.contains(&format!(
                        "Redundant escape of {} inside string literal.",
                        String::from_utf8_lossy(character)
                    )),
                    "{text}"
                );
            }
            unchanged(dir.path(), E, &source, status);
            // Retain that exact character while fixing independent ASCII punctuation.
            let mut expected = source.clone();
            expected.extend_from_slice(b"p \"!\"\n");
            let mut mixed = source;
            mixed.extend_from_slice(b"p \"\\!\"\n");
            fs::write(dir.path().join("a.rb"), &mixed).unwrap();
            let before = has_ruby().then(|| ruby(dir.path()));
            cli(dir.path(), &["--only", E, "--fix", "a.rb"], status);
            let actual = fs::read(dir.path().join("a.rb")).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(escape_projection(&mixed), escape_projection(&actual));
            if let Some(before) = before {
                assert!(before.status.success(), "{before:?}");
                let after = ruby(dir.path());
                assert_eq!(before.status, after.status);
                assert_eq!(before.stdout, after.stdout);
                assert_eq!(before.stderr, after.stderr);
            }
            unchanged(dir.path(), E, &actual, status);
        }
    }
}

#[test]
fn recovery_review_utf8_unicode_version_offenses_and_neighbor_controls() {
    if crate::support::runtime::skips("pins the Unicode version of Ruby 3.4") {
        return;
    }
    let dir = project();
    let mut runtimes = Vec::new();
    if has_ruby() {
        runtimes.push("ruby".to_string());
    }
    if let Ok(exe) = crate::support::runtime::ruby() {
        let version = Command::new(&exe)
            .args([
                "-e",
                crate::support::runtime::script("exit(RUBY_VERSION.start_with?('3.4.') ? 0 : 1)")
                    .as_str(),
            ])
            .output()
            .unwrap();
        assert!(version.status.success(), "{version:?}");
        runtimes.push(exe);
    }
    for (character, status) in [
        ('\u{1c89}', 1),
        ('\u{a7cb}', 1),
        ('\u{16100}', 1),
        ('\u{1c88}', 0),
        ('\u{a7ca}', 0),
        ('\u{160ff}', 1),
        ('\u{16a40}', 0),
        ('Ⅰ', 0),
        ('１', 0),
        ('²', 1),
        ('、', 1),
    ] {
        for exe in &runtimes {
            let class = Command::new(exe)
                .args([
                    "-e",
                    "exit(/[[:alnum:]]/.match?(ARGV[0]) ? 0 : 1)",
                    &character.to_string(),
                ])
                .output()
                .unwrap();
            assert_eq!(class.status.code(), Some(status), "{exe}: {character}");
        }
        for (left, right, line, col) in [
            ("p \"", "\"\n", 1, 4),
            ("p \"#{1}", "\"\n", 1, 8),
            ("p %W[", "]\n", 1, 6),
            ("p <<~EOS\n  ", "\nEOS\n", 2, 3),
        ] {
            let source = format!("{left}\\{character}{right}");
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let out = cli(dir.path(), &["--only", E, "a.rb"], status);
            if status == 1 {
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(text.contains(&format!("a.rb:{line}:{col}:")), "{text}");
                assert!(
                    text.contains(&format!(
                        "Redundant escape of {character} inside string literal."
                    )),
                    "{text}"
                );
            }
            unchanged(dir.path(), E, source.as_bytes(), status);
            let mixed = format!("{source}p \"\\!\"\n");
            fs::write(dir.path().join("a.rb"), &mixed).unwrap();
            let run = |exe: &String| {
                Command::new(exe)
                    .current_dir(dir.path())
                    .args(["-W:deprecated", "a.rb"])
                    .output()
                    .unwrap()
            };
            let before: Vec<_> = runtimes.iter().map(run).collect();
            cli(dir.path(), &["--only", E, "--fix", "a.rb"], status);
            let actual = fs::read(dir.path().join("a.rb")).unwrap();
            assert_eq!(actual, format!("{source}p \"!\"\n").as_bytes());
            for (exe, before) in runtimes.iter().zip(before) {
                let after = run(exe);
                assert!(before.status.success(), "{exe}: {before:?}");
                assert_eq!(before.status, after.status);
                assert_eq!(before.stdout, after.stdout, "{exe}: {character}");
                assert_eq!(before.stderr, after.stderr, "{exe}: {character}");
            }
            assert_eq!(
                escape_projection(mixed.as_bytes()),
                escape_projection(&actual)
            );
        }
    }
}

#[test]
fn continuation_utf8_family_letters_and_unicode_boundaries_keep_exact_no_write_contract() {
    let dir = project();
    // Includes all six rejected fixtures, every sibling profile and the aliases
    // resolved by Prism (identity is deliberately NOT ordinary UTF-8).
    for encoding in [
        "UTF-8",
        "CP65001",
        "UTF8-MAC",
        "UTF-8-HFS",
        "UTF8-DoCoMo",
        "UTF8-KDDI",
        "UTF8-SoftBank",
        "CESU-8",
        "utf8-docomo",
        "UTF-8-UNIX",
    ] {
        for (character, status) in [
            ('中', 0),
            ('\u{a7ca}', 0),
            ('１', 0),
            ('Ⅰ', 0),
            ('、', 1),
            ('☆', 1),
            ('²', 1),
            ('\u{1c89}', 1),
            ('\u{a7cb}', 1),
        ] {
            for (left, right, line, col) in [
                ("p \"", "\"\n", 2, 4),
                ("p \"#{1}", "\"\n", 2, 8),
                ("p %W[", "]\n", 2, 6),
                ("p <<~EOS\n  ", "\nEOS\n", 3, 3),
            ] {
                let source = format!("# encoding: {encoding}\n{left}\\{character}{right}");
                fs::write(dir.path().join("a.rb"), &source).unwrap();
                let out = cli(dir.path(), &["--only", E, "a.rb"], status);
                if status == 1 {
                    let text = String::from_utf8_lossy(&out.stdout);
                    assert!(text.contains(&format!("a.rb:{line}:{col}:")), "{text}");
                    assert!(
                        text.contains(&format!(
                            "Redundant escape of {character} inside string literal."
                        )),
                        "{text}"
                    );
                }
                unchanged(dir.path(), E, source.as_bytes(), status);
                let mixed = format!("{source}p \"\\!\"\n");
                fs::write(dir.path().join("a.rb"), &mixed).unwrap();
                cli(dir.path(), &["--only", E, "--fix", "a.rb"], status);
                let actual = fs::read(dir.path().join("a.rb")).unwrap();
                assert_eq!(actual, format!("{source}p \"!\"\n").as_bytes());
                assert_eq!(
                    escape_projection(mixed.as_bytes()),
                    escape_projection(&actual)
                );
            }
        }
    }
}

#[test]
fn continuation_cesu8_supplementary_classes_use_bounded_pair_projection() {
    let dir = project();
    for (bytes, status) in [
        (b"\xed\xa0\x81\xed\xb0\xa8".as_slice(), 0), // Deseret letter
        (b"\xed\xa0\xb5\xed\xbf\x99".as_slice(), 0), // decimal
        (b"\xed\xa0\x98\xed\xb4\x80".as_slice(), 1), // post-Unicode15
        (b"\xed\xa0\xbd\xed\xb8\x80".as_slice(), 1), // emoji
    ] {
        let mut source = b"# coding: CESU-8\np \"\\".to_vec();
        source.extend_from_slice(bytes);
        source.extend_from_slice(b"\"\n");
        unchanged(dir.path(), E, &source, status);
    }
}

#[test]
fn continuation_unicode_profiles_preserve_host_and_native_runtime_observations() {
    let dir = project();
    let mut runtimes = Vec::new();
    if has_ruby() {
        runtimes.push("ruby".to_string());
    }
    if let Ok(exe) = crate::support::runtime::ruby() {
        runtimes.push(exe);
    }
    for exe in runtimes {
        for encoding in [
            "UTF-8",
            "UTF8-MAC",
            "UTF-8-HFS",
            "UTF8-DoCoMo",
            "UTF8-KDDI",
            "UTF8-SoftBank",
            "CESU-8",
        ] {
            // A miniruby can list a dummy profile yet fail to load its codec
            // and silently fall back to ASCII-8BIT. Test actual source identity,
            // not the registry list, before claiming native encoding evidence.
            let expected_encoding = if matches!(encoding, "UTF8-MAC" | "UTF-8-HFS") {
                "UTF8-MAC"
            } else {
                encoding
            };
            let probe =
                format!("# coding: {encoding}\nexit(\"中\".encoding.name == ARGV[0] ? 0 : 1)");
            let available = Command::new(&exe)
                .args(["-e", &probe, expected_encoding])
                .output()
                .unwrap();
            if !available.status.success() {
                eprintln!(
                    "EXCLUDED runtime codec {exe}: {encoding}: {}; detection is tested separately",
                    String::from_utf8_lossy(&available.stderr).trim()
                );
                continue;
            }
            for policy in [
                "",
                "# frozen_string_literal: true\n",
                "# frozen_string_literal: false\n",
            ] {
                let source = format!(
                    r#"# coding: {encoding}
{policy}def observe(s)
  p [s.bytes, s.encoding.name, s.frozen?]
  begin
    s << "x"
    p s.bytes
  rescue FrozenError
    puts "frozen"
  end
end
values = ["\中", "\ꟊ", "\１", "\、", "\Ᲊ", "\!"]
values.each {{ |s| observe(s) }}
"#
                );
                fs::write(dir.path().join("a.rb"), &source).unwrap();
                let run = || {
                    Command::new(&exe)
                        .current_dir(dir.path())
                        .args(["-W:deprecated", "a.rb"])
                        .output()
                        .unwrap()
                };
                let before = run();
                assert!(before.status.success(), "{exe} {encoding}: {before:?}");
                cli(dir.path(), &["--only", E, "--fix", "a.rb"], 1);
                assert_eq!(
                    fs::read_to_string(dir.path().join("a.rb")).unwrap(),
                    source.replace("\\!", "!")
                );
                let after = run();
                assert_eq!(before.status, after.status);
                assert_eq!(before.stdout, after.stdout, "{exe} {encoding}");
                assert_eq!(before.stderr, after.stderr, "{exe} {encoding}");
            }
        }
    }
}

fn escape_projection(source: &[u8]) -> Vec<String> {
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Projection(Vec<String>);
    impl<'pr> NodeVisitor<'pr> for Projection {
        fn enter(&mut self, n: ruby_prism::Node<'pr>) {
            self.0.push(format!("{:?}", std::mem::discriminant(&n)));
            if let Some(s) = n.as_string_node() {
                self.0.push(format!(
                    "{:?}:{}:{}:{}:{}",
                    s.unescaped(),
                    s.is_forced_utf8_encoding(),
                    s.is_forced_binary_encoding(),
                    s.is_frozen(),
                    s.is_mutable()
                ));
            }
            let name = if let Some(v) = n.as_instance_variable_read_node() {
                Some(v.name())
            } else if let Some(v) = n.as_class_variable_read_node() {
                Some(v.name())
            } else {
                n.as_global_variable_read_node().map(|v| v.name())
            };
            if let Some(name) = name {
                self.0.push(format!("{:?}", name.as_slice()));
            }
        }
        fn leave(&mut self) {
            self.0.push("end".into());
        }
    }
    let parsed = ruby_prism::parse_ruby_3_4(source);
    assert_eq!(parsed.errors().count(), 0);
    let mut p = Projection::default();
    walk(&mut p, &parsed.node());
    p.0
}

#[test]
fn encoded_unbraced_variable_ownership_preserves_binding_and_safe_braced_controls() {
    let dir = project();
    let explicit_34 = has_ruby()
        && Command::new("ruby")
            .args(["-rprism", "-e", "Prism.parse('nil', version: '3.4')"])
            .output()
            .is_ok_and(|o| o.status.success());
    let prism_shape = r#"
r = Prism.parse(File.binread('a.rb'), version: '3.4'); abort r.errors.inspect unless r.success?
w = ->(n) do
  p n.class.name
  p [n.unescaped.bytes, n.unescaped.encoding.name, n.forced_utf8_encoding?, n.forced_binary_encoding?, n.frozen?, n.mutable?] if n.is_a?(Prism::StringNode)
  p n.name.to_s.bytes if [Prism::InstanceVariableReadNode, Prism::ClassVariableReadNode, Prism::GlobalVariableReadNode].any? { |k| n.is_a?(k) }
  n.compact_child_nodes.each { |c| w.call(c) }; p :end
end
w.call(r.value)
"#;
    let mut encodings: Vec<_> = ESCAPE_ENCODINGS
        .iter()
        .map(|(e, c, _, _, _)| (*e, *c))
        .collect();
    encodings.extend([
        ("CP932", b"\x83\x7d".as_slice()),
        ("Shift_JIS", b"\x83\x5c".as_slice()),
        ("CP949", b"\x81\x41".as_slice()),
        ("GB18030", b"\x90\x30\xeb\x34".as_slice()),
    ]);
    for (encoding, name) in encodings {
        for prefix in ["@", "@@", "$"] {
            let template = format!(
                "# encoding: {encoding}\nclass EscapeFixture\n{prefix}NAME = \"A\"; {prefix}NAME_ = \"B\"\np \"#{prefix}NAME\\_\"\nend\n"
            );
            let source = template.as_bytes();
            let substitute = |s: &[u8]| {
                let mut result = Vec::new();
                let mut start = 0;
                for at in memchr::memmem::find_iter(s, b"NAME") {
                    result.extend_from_slice(&s[start..at]);
                    result.extend_from_slice(name);
                    start = at + 4;
                }
                result.extend_from_slice(&s[start..]);
                result
            };
            let source = substitute(source);
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let runtime = has_ruby().then(|| ruby(dir.path()));
            let shape = || {
                Command::new("ruby")
                    .current_dir(dir.path())
                    .args(["-rprism", "-e", prism_shape])
                    .output()
                    .unwrap()
            };
            let before_shape = explicit_34.then(shape);
            let out = cli(dir.path(), &["--only", E, "a.rb"], 1);
            assert!(
                String::from_utf8_lossy(&out.stdout)
                    .contains(&format!("a.rb:4:{}:", 6 + prefix.len())),
                "{out:?}"
            );
            unchanged(dir.path(), E, &source, 1);
            // Braced interpolation and a punctuation-separated suffix are safe.
            // Keep the unsafe finding visible while applying independent edits.
            let controls =
                format!("p [\"#{{{prefix}NAME}}\\_\", \"#{prefix}NAME!\\_\", \"\\!\"]\n");
            let mut mixed = source[..source.len() - 4].to_vec();
            mixed.extend(substitute(controls.as_bytes()));
            mixed.extend_from_slice(b"end\n");
            fs::write(dir.path().join("a.rb"), &mixed).unwrap();
            let original = has_ruby().then(|| ruby(dir.path()));
            let original_shape = explicit_34.then(shape);
            cli(dir.path(), &["--only", E, "--fix", "a.rb"], 1);
            let actual = fs::read(dir.path().join("a.rb")).unwrap();
            let mut expected = source[..source.len() - 4].to_vec();
            expected.extend(substitute(
                controls.replace("\\_", "_").replace("\\!", "!").as_bytes(),
            ));
            expected.extend_from_slice(b"end\n");
            assert_eq!(actual, expected, "{encoding} {prefix}");
            assert_eq!(escape_projection(&mixed), escape_projection(&actual));
            if let Some(original) = original {
                assert!(original.status.success(), "{original:?}");
                let after = ruby(dir.path());
                assert_eq!(original.stdout, after.stdout);
                assert_eq!(original.stderr, after.stderr);
                assert_eq!(runtime.unwrap().stdout, b"\"A_\"\n");
            }
            if let Some(before) = original_shape {
                let after = shape();
                assert!(before.status.success() && after.status.success());
                assert_eq!(before.stdout, after.stdout);
                assert_eq!(before.stderr, after.stderr);
                assert!(before_shape.unwrap().status.success());
            }
            unchanged(dir.path(), E, &actual, 1);
        }
    }
}

#[test]
fn controlled_runtime_preserves_bytes_encoding_identity_flags_and_effect_order() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for frozen in ["true", "false"] {
        let source = format!(
            r#"# frozen_string_literal: {frozen}
$events = []
def mark(x)
  $events << x
  x
end
def literal
  <<~EMPTY
EMPTY
end
values = ["\#foo", "\'", %Q(\"), %Q(\#\{{text}}), %W[\#one \#two], <<~'A', <<~"B", <<~C]
  café
A
  \#a #{{mark(1)}} #{{mark(2)}}
B
C
values << <<~OUT
  prefix #{{<<~INNER}} suffix
INNER
  #{{
    <<~'OTHER'
OTHER
  }}
OUT
values.each {{ |v| p(v.is_a?(String) ? [v.bytes, v.encoding.name, v.frozen?] : v.map {{ |s| [s.bytes, s.encoding.name, s.frozen?] }}) }}
p [literal.equal?(literal), literal.frozen?, $events]
"#
        );
        for cop in [E, Q, H, ALL] {
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let before = ruby(dir.path());
            assert!(before.status.success(), "{before:?}");
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", cop, "--fix", "a.rb"])
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_ne!(
                fs::read_to_string(dir.path().join("a.rb")).unwrap(),
                source,
                "{cop}"
            );
            let after = ruby(dir.path());
            assert_eq!(before.status, after.status);
            assert_eq!(before.stdout, after.stdout, "{cop}");
            assert_eq!(before.stderr, after.stderr, "{cop}");
        }
    }
}
#[test]
fn escape_run_parity_and_composed_variable_escapes_have_runtime_witnesses() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let source = r##"@foo = "ACTIVE"
$foo = "ACTIVE"
p ["\#\@foo", "\#\$foo", "\#\@@foo", "\#\{foo}", "\\#\{foo}", "\\\#foo", "\\\\\#foo", "\²", "\Ⅰ", "\😀"]
"##;
    fs::write(dir.path().join("a.rb"), source).unwrap();
    let before = ruby(dir.path());
    assert!(before.status.success());
    cli(dir.path(), &["--only", E, "--fix", "a.rb"], 1);
    let actual = fs::read_to_string(dir.path().join("a.rb")).unwrap();
    assert_ne!(actual, source);
    let after = ruby(dir.path());
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
    fs::write(dir.path().join("a.rb"), source.replace(r"\#\@foo", "#@foo")).unwrap();
    let naive = ruby(dir.path());
    assert!(naive.status.success());
    assert_ne!(before.stdout, naive.stdout);
}

#[test]
fn escape_fixes_preserve_variable_binding_control_bytes_and_vertical_tabs() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let source = concat!(
        "@name = 'A'; @name_ = 'B'\n",
        "@@name = 'C'; @@name_ = 'D'\n",
        "$name = 'E'; $name_ = 'F'\n",
        "p [\"#@name\\_\", \"#@@name\\_\", \"#$name\\_\"]\n",
        "p [\"\\c\\?\", \"\\C-\\?\", \"\\M-\\C-\\?\", \"\\c\\M-\\?\"].map(&:bytes)\n",
        "p %W[a\\\u{b}b].map(&:bytes)\n",
        "p((<<~EOS).bytes)\n  \\\u{b}text\nEOS\n",
        "p \"\\!\"\n",
    );
    // Class variables need a class scope on the host runtime.
    let source = format!("class EscapeFixture\n{source}end\n");
    fs::write(dir.path().join("a.rb"), &source).unwrap();
    let before = ruby(dir.path());
    assert!(before.status.success(), "{before:?}");
    assert_eq!(
        String::from_utf8_lossy(&before.stdout),
        "[\"A_\", \"C_\", \"E_\"]\n[[31], [31], [159], [159]]\n[[97, 11, 98]]\n[11, 116, 101, 120, 116, 10]\n\"!\"\n"
    );
    cli(dir.path(), &["--only", E, "--fix", "a.rb"], 1);
    let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
    assert_eq!(after, source.replace(r"\!", "!"));
    let actual = ruby(dir.path());
    assert_eq!(before.status, actual.status);
    assert_eq!(before.stdout, actual.stdout);
    assert_eq!(before.stderr, actual.stderr);
}

#[test]
fn unicode_columns_and_strict_no_options_contract() {
    let dir = project();
    for (cop, before, _) in CASES {
        fs::write(
            dir.path().join("a.rb"),
            format!("# heading\np \"é\"; {before}"),
        )
        .unwrap();
        let out = cli(dir.path(), &["--only", cop, "a.rb"], 1);
        let col = if cop == E { 11 } else { 10 };
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(&format!("a.rb:2:{col}:")),
            "{out:?}"
        );
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nEnforcedStyle = \"other\"\n"),
        )
        .unwrap();
        let before = fs::read(dir.path().join("a.rb")).unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "a.rb"], 2);
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), before);
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn fixed_34_main_ast_payload_encoding_frozen_mutable_and_bindings_match() {
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Shape(Vec<String>);
    impl<'pr> NodeVisitor<'pr> for Shape {
        fn enter(&mut self, n: ruby_prism::Node<'pr>) {
            self.0.push(format!("{:?}", std::mem::discriminant(&n)));
            if let Some(s) = n.as_string_node() {
                self.0.push(format!(
                    "{:?}:{}:{}:{}:{}",
                    s.unescaped(),
                    s.is_forced_utf8_encoding(),
                    s.is_forced_binary_encoding(),
                    s.is_frozen(),
                    s.is_mutable()
                ));
            }
            if let Some(s) = n.as_local_variable_read_node() {
                self.0
                    .push(format!("{:?}:{}", s.name().as_slice(), s.depth()));
            }
            if let Some(s) = n.as_call_node() {
                self.0.push(format!("{:?}", s.name().as_slice()));
            }
            if let Some(s) = n.as_instance_variable_read_node() {
                self.0.push(format!("{:?}", s.name().as_slice()));
            }
            if let Some(s) = n.as_class_variable_read_node() {
                self.0.push(format!("{:?}", s.name().as_slice()));
            }
            if let Some(s) = n.as_global_variable_read_node() {
                self.0.push(format!("{:?}", s.name().as_slice()));
            }
        }
        fn leave(&mut self) {
            self.0.push("end".into());
        }
    }
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
        "# shareable_constant_value: literal\n",
    ] {
        for (cop, source) in [
            (
                &EmptyHeredoc as &dyn Cop,
                "x = 1\nxs.map { [<<~EOS, x, it] }\nEOS\n",
            ),
            (
                &RedundantHeredocDelimiterQuotes,
                "x = 1\nxs.map { [<<~\"EOS\", x, it] }\n#{it} #{x}\nEOS\n",
            ),
            (
                &RedundantStringEscape,
                "x = 1\nxs.map { [\"\\#hi#{it}#{x}\", %W[\\#a], \"\\xff\\#\", \"\\u{41}\\#\"] }\n",
            ),
            (
                &RedundantStringEscape,
                concat!(
                    "class EscapeFixture\n",
                    "p [\"#@name\\_\", \"#@@name\\_\", \"#$name\\_\", \"#@é\\_\"]\n",
                    "p [\"\\c\\?\", \"\\M-\\C-\\?\", \"\\c\\M-\\?\", \"\\!\"]\n",
                    "p %W[a\\\u{b}b]\np <<~EOS\n  \\\u{b}text\nEOS\nend\n",
                ),
            ),
        ] {
            let before = format!("{header}{source}");
            let after = fixed(cop, &before);
            assert_ne!(after, before.as_bytes());
            let a = ruby_prism::parse_ruby_3_4(before.as_bytes());
            let b = ruby_prism::parse_ruby_3_4(&after);
            assert_eq!(a.errors().count(), 0);
            assert_eq!(b.errors().count(), 0);
            let mut sa = Shape::default();
            let mut sb = Shape::default();
            walk(&mut sa, &a.node());
            walk(&mut sb, &b.node());
            assert_eq!(
                sa.0,
                sb.0,
                "{}: {}",
                cop.name(),
                String::from_utf8_lossy(&after)
            );
        }
    }
}
#[test]
fn explicit_prism_34_shape_and_literal_flags_are_equal() {
    if !Command::new("ruby")
        .args(["-rprism", "-e", "Prism.parse('nil', version: '3.4')"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("SKIP explicit Prism 3.4 check (gem unavailable)");
        return;
    }
    let dir = project();
    let shape = r#"
r = Prism.parse(File.binread('a.rb'), version: '3.4')
abort r.errors.inspect unless r.success?
w = ->(n) do
  p n.class.name
  p [n.unescaped.bytes, n.unescaped.encoding.name, n.forced_utf8_encoding?, n.forced_binary_encoding?, n.frozen?, n.mutable?] if n.is_a?(Prism::StringNode)
  p [n.frozen?, n.mutable?] if n.is_a?(Prism::InterpolatedStringNode)
  p [n.name, n.depth] if n.is_a?(Prism::LocalVariableReadNode)
  p n.name if [Prism::CallNode, Prism::InstanceVariableReadNode, Prism::ClassVariableReadNode, Prism::GlobalVariableReadNode].any? { |k| n.is_a?(k) }
  n.compact_child_nodes.each { |c| w.call(c) }
  p :end
end
w.call(r.value)
"#;
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
        "# shareable_constant_value: literal\n",
    ] {
        let escapes = concat!(
            "class EscapeFixture\n",
            "p [\"#@name\\_\", \"#@@name\\_\", \"#$name\\_\", \"#@é\\_\"]\n",
            "p [\"\\c\\?\", \"\\M-\\C-\\?\", \"\\c\\M-\\?\", \"\\!\"]\n",
            "p %W[a\\\u{b}b]\np <<~EOS\n  \\\u{b}text\nEOS\nend\n",
        );
        let source = format!(
            "{header}x = 1\nxs.map {{ [\"\\#hi#{{it}}#{{x}}\", <<~\"A\", <<~B, \"\\xff\\#\", \"\\u{{41}}\\#\"] }}\n#{{it}} #{{x}}\nA\nB\n{escapes}"
        );
        fs::write(dir.path().join("a.rb"), &source).unwrap();
        let before = Command::new("ruby")
            .current_dir(dir.path())
            .args(["-rprism", "-e", shape])
            .output()
            .unwrap();
        assert!(before.status.success(), "{before:?}");
        // Atomic empty-doc envelopes retain later header siblings; overlapping
        // escape edits deliberately wait for the next explicit pass.
        cli(dir.path(), &["--only", ALL, "--fix", "a.rb"], 1);
        cli(dir.path(), &["--only", ALL, "--fix", "a.rb"], 1);
        assert_ne!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        let after = Command::new("ruby")
            .current_dir(dir.path())
            .args(["-rprism", "-e", shape])
            .output()
            .unwrap();
        assert!(after.status.success(), "{after:?}");
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}

#[test]
fn recovery_native_34_keeps_original_escape_holds_and_heredoc_effects() {
    let exe = crate::support::runtime::ruby().unwrap_or_else(|_| "ruby".into());
    if !Command::new(&exe)
        .args([
            "-e",
            crate::support::runtime::script("exit(RUBY_VERSION.start_with?('3.4.') ? 0 : 1)")
                .as_str(),
        ])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("SKIP native Ruby 3.4 recovery witness (set RUSTOCOP_RUBY34)");
        return;
    }
    let dir = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        // Mutations/warnings occur in the unchanged helper before any removed
        // heredoc lines, so diagnostic positions do not obscure chilled flags.
        let source = format!(
            r##"{header}def observe(s)
  p [s.bytes, s.encoding.name, s.frozen?]
  begin
    s << "x"
    p s.bytes
  rescue FrozenError
    puts "frozen"
  end
end
class EscapeRecoveryFixture
@name = "A"; @name_ = "B"
@@name = "C"; @@name_ = "D"
$name = "E"; $name_ = "F"
events = []
values = ["#@name\_", "#@@name\_", "#$name\_", "#{{@name}}\_", "\c\?", "\M-\C-\?", "\c\M-\?", "\#\@name", "\#\{{text}}", "\!", "\²", "\Ⅰ", <<~'A', <<~B, <<~EMPTY]
  café
A
  \# #{{events << 1; "one"}} #{{events << 2; "two"}}
B
EMPTY
values.each {{ |s| observe(s) }}
p events
end
"##
        );
        let source =
            source + "p %W[a\\\u{b}b].map(&:bytes)\np((<<~EOS).bytes)\n  \\\u{b}text\nEOS\n";
        for cop in [E, Q, H, ALL] {
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let run = || {
                Command::new(&exe)
                    .current_dir(dir.path())
                    .args(["-W:deprecated", "a.rb"])
                    .output()
                    .unwrap()
            };
            let before = run();
            assert!(before.status.success(), "{before:?}");
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", cop, "--fix", "a.rb"])
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            let actual = fs::read(dir.path().join("a.rb")).unwrap();
            assert_ne!(actual, source.as_bytes(), "{cop}");
            let after = run();
            assert_eq!(before.status, after.status);
            assert_eq!(before.stdout, after.stdout, "{cop}: {header}");
            assert_eq!(before.stderr, after.stderr, "{cop}: {header}");
            if cop == E || cop == ALL {
                for held in [
                    "#@name\\_",
                    "#@@name\\_",
                    "#$name\\_",
                    "\\c\\?",
                    "\\M-\\C-\\?",
                ] {
                    assert!(String::from_utf8_lossy(&actual).contains(held), "{held}");
                }
                assert!(String::from_utf8_lossy(&actual).contains("\"!\""));
            }
        }
    }
}

#[test]
fn ruby_34_runtime_chilled_and_explicit_flags_gate() {
    let exe = crate::support::runtime::ruby().unwrap_or_else(|_| "ruby".into());
    if !Command::new(&exe)
        .args([
            "-e",
            crate::support::runtime::script("exit(RUBY_VERSION.start_with?('3.4.') ? 0 : 1)")
                .as_str(),
        ])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!(
            "SKIP Ruby 3.4 runtime (set RUSTOCOP_RUBY34); host Ruby 3.2 and explicit Prism 3.4 checks are separate"
        );
        return;
    }
    let dir = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        let source = format!(
            "{header}def literal\n  <<~EMPTY\nEMPTY\nend\nvalues = [\"\\#hi\", <<~'A', literal]\nhi\nA\nvalues.each {{ |s| p [s.bytes, s.encoding.name, s.frozen?]; begin; s << 'x'; p s.bytes; rescue FrozenError; puts 'frozen'; end }}\np literal.equal?(literal)\n"
        );
        fs::write(dir.path().join("a.rb"), &source).unwrap();
        let before = Command::new(&exe)
            .current_dir(dir.path())
            .arg("a.rb")
            .output()
            .unwrap();
        assert!(before.status.success());
        cli(dir.path(), &["--only", ALL, "--fix", "a.rb"], 0);
        let after = Command::new(&exe)
            .current_dir(dir.path())
            .arg("a.rb")
            .output()
            .unwrap();
        assert_eq!(before.status, after.status);
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}

#[test]
fn actual_default_and_selected_peer_composition_stabilizes_within_six_passes() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let source = "def f\n  return ({ :a => \"\\#hi\" });\nend\np [f, <<~'EOS', <<~'EMPTY', <<~OTHER, \"\\#\\@foo\"]\n  text\nEOS\nEMPTY\nOTHER\n";
    let peers = format!(
        "{ALL},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout"
    );
    for selected in [Some(peers.as_str()), None] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success());
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(dir.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(names) = selected {
                args.extend(["--only", names]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            let after = ruby(dir.path());
            assert_eq!(before.status, after.status);
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
            if fs::read(dir.path().join("a.rb")).unwrap() == old {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        assert_ne!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
    }
}
