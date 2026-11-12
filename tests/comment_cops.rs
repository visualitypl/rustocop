//! real CLI policy, byte preservation, and controlled Ruby observations.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const CASES: &[(&str, &str, &str, &str)] = &[
    ("Style/AsciiComments", "# 日本語\n", "# 日本語\n", "1:3"),
    (
        "Style/BlockComments",
        "=begin\ncomment\n=end\n",
        "# comment\n",
        "1:1",
    ),
    (
        "Style/CommentAnnotation",
        "# todo improve this\n",
        "# TODO: improve this\n",
        "1:3",
    ),
];

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn cli(cwd: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    if status != 2 {
        assert!(out.stderr.is_empty(), "{out:?}");
    }
    out
}

#[test]
fn exact_names_defaults_and_safe_capabilities() {
    let registry = rustocop::cops::Registry::new();
    for (name, _, _, _) in CASES {
        let cops: Vec<_> = registry
            .cops()
            .iter()
            .filter(|cop| cop.name() == *name)
            .collect();
        assert_eq!(cops.len(), 1);
        let ascii = *name == "Style/AsciiComments";
        assert_eq!(cops[0].default_enabled(), !ascii);
        assert_eq!(cops[0].safe_fixes(), !ascii);
    }
}

#[test]
fn defaults_opt_in_selectors_config_and_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after, position) in CASES {
        fs::write(&path, before).unwrap();
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        let ascii = *cop == "Style/AsciiComments";
        let out = cli(dir.path(), &["lib/a.rb"], i32::from(!ascii));
        assert_eq!(String::from_utf8_lossy(&out.stdout).contains(cop), !ascii);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            cli(dir.path(), &["lib/a.rb"], 0);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            let out = cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!("lib/a.rb:{position}: C:")),
                "{out:?}"
            );
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            cli(
                dir.path(),
                &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        let out = cli(dir.path(), &["--fix", "lib/a.rb"], i32::from(ascii));
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(if ascii {
                "0 corrections applied"
            } else {
                "1 correction applied"
            }),
            "{out:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &["--only", cop, "--fix", "lib/a.rb"],
            i32::from(ascii),
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn directives_and_additive_config_relative_excludes_protect_all_three() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, _, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [*cop, "Style", "all"] {
                let source = format!("# {prefix}:disable {target}\n{before}");
                fs::write(&path, &source).unwrap();
                cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            }
        }
        fs::write(&path, before).unwrap();
        for (global, local) in [("lib/a.rb", "other.rb"), ("other.rb", "lib/a.rb")] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("exclude = [\"{global}\"]\n[cops.\"{cop}\"]\nexclude = [\"{local}\"]\n"),
            )
            .unwrap();
            cli(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "../lib/a.rb"],
                0,
            );
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn bom_crlf_literals_data_and_line_observations_keep_exact_bytes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let before = "\u{feff}# encoding: UTF-8\r\np __LINE__\r\n=begin\r\ncomment\r\n=end\r\nx = \"# TODO text\" # todo improve this\r\nx = <<~H\r\n=begin\r\n# todo text\r\n日本語\r\n=end\r\nH\r\n__END__\r\n# todo text\r\n=begin\r\n日本語\r\n=end\r\n";
    let after = before
        .replace("=begin\r\ncomment\r\n=end\r\n", "# comment\r\n")
        .replace("# todo improve this", "# TODO: improve this");
    fs::write(&path, before).unwrap();
    cli(
        dir.path(),
        &[
            "--only",
            "Style/BlockComments,Style/CommentAnnotation",
            "--fix",
            "lib/a.rb",
        ],
        0,
    );
    assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
    // Non-ASCII inside the heredoc and data section is never a comment offense.
    cli(
        dir.path(),
        &["--only", "Style/AsciiComments", "lib/a.rb"],
        0,
    );
}

#[test]
fn legacy_notes_and_non_utf8_document_bytes_are_not_transcoded() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for encoding in ["ASCII-8BIT", "Windows-1252", "Shift_JIS"] {
        let mut before = format!("# coding: {encoding}\nx = 1 # todo ").into_bytes();
        before.extend_from_slice(b"\x82\xa0\r\n=begin\r\n\x82\xa0\r\n=end\r\n");
        let mut after = format!("# coding: {encoding}\nx = 1 # TODO: ").into_bytes();
        after.extend_from_slice(b"\x82\xa0\r\n=begin\r\n\x82\xa0\r\n=end\r\n");
        fs::write(&path, &before).unwrap();
        let out = cli(
            dir.path(),
            &[
                "--only",
                "Style/BlockComments,Style/CommentAnnotation",
                "--fix",
                "lib/a.rb",
            ],
            1,
        );
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .contains("1 correction applied, 1 offense remaining"),
            "{out:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), after);
        let out = cli(
            dir.path(),
            &["--only", "Style/AsciiComments", "--fix", "lib/a.rb"],
            1,
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("a.rb:2:15:"), "{encoding}: {text}");
        assert!(text.contains("a.rb:4:1:"), "{encoding}: {text}");
        assert_eq!(fs::read(&path).unwrap(), after);
    }
}

#[test]
fn uncertain_edits_and_parse_errors_never_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "=begin\ntext\n=end\np __LINE__\n",
        "=begin\ncoding: ASCII-8BIT\n=end\np \"é\".encoding\n",
        "=begin\nfrozen_string_literal: true\n=end\np \"a\".frozen?\n",
        "=begin\n=end\n# coding: ASCII-8BIT\np \"é\".encoding\n",
        "=begin\n# rubocop:disable Lint/Debugger\n=end\nbinding.pry\n",
        "# TODO coding: ASCII-8BIT\n",
        "# HACK:\n",
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &[
                "--only",
                "Style/BlockComments,Style/CommentAnnotation",
                "--fix",
                "lib/a.rb",
            ],
            1,
        );
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    let source = "# todo something\ndef broken(\n";
    fs::write(&path, source).unwrap();
    cli(
        dir.path(),
        &["--only", "Style/CommentAnnotation", "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
}

#[test]
fn controlled_ruby_observations_preserve_values_encodings_frozen_state_and_data() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "# encoding: UTF-8\np __LINE__\n=begin\ncomment\n=end\nx = 0 # todo improve this\nx += 1\np [x, \"é\".encoding.name, \"a\".frozen?, DATA.read.bytes]\n__END__\n# TODO data\n",
        "=begin\nfrozen_string_literal: true\n=end\np [__LINE__, \"a\".frozen?]\n",
        "=begin\nFrozen-String-Literal: true\n=end\np \"a\".frozen?\n",
        "=begin\n=end\n# coding: ASCII-8BIT\np [\"é\".encoding.name, \"é\".bytes]\n",
        "#!/usr/bin/ruby\n=begin\n=end\n# coding: ASCII-8BIT\np [\"é\".encoding.name, \"é\".bytes]\n",
        "# TODO coding: ASCII-8BIT\np [__LINE__, \"é\".encoding.name, \"é\".bytes]\n",
    ] {
        fs::write(&path, source).unwrap();
        let run = || {
            let out = Command::new("ruby").arg(&path).output().unwrap();
            assert!(out.status.success(), "{out:?}");
            out.stdout
        };
        let before = run();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args([
                "--only",
                "Style/BlockComments,Style/CommentAnnotation",
                "--fix",
                "lib/a.rb",
            ])
            .output()
            .unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
        assert_eq!(run(), before, "{source}");
    }
}

#[test]
fn mixed_style_layout_and_newly_exposed_annotations_stabilize_in_two_passes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let before = "=begin\nTODO improve this\n=end\nx = 'text';  \nx\n";
    fs::write(&path, before).unwrap();
    let only = "Style/BlockComments,Style/CommentAnnotation,Style/StringLiterals,Style/Semicolon,Layout/TrailingWhitespace";
    cli(
        dir.path(),
        &["--only", only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        1,
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"# TODO improve this\nx = \"text\"\nx\n"
    );
    cli(
        dir.path(),
        &["--only", only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        0,
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"# TODO: improve this\nx = \"text\"\nx\n"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(
        dir.path(),
        &["--only", only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        0,
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
