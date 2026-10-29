//! fixed defaults, selectors and byte/runtime preservation through CLI.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const CASES: &[(&str, &str, &str, bool)] = &[
    (
        "Style/CommentedKeyword",
        "class X # comment\nend\n",
        "1:9",
        true,
    ),
    ("Style/Copyright", "# header\n", "1:1", false),
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
fn exact_registration_fixed_defaults_and_correction_classification() {
    let registry = rustocop::cops::Registry::new();
    for (name, _, _, default) in CASES {
        let cops: Vec<_> = registry
            .cops()
            .iter()
            .filter(|cop| cop.name() == *name)
            .collect();
        assert_eq!(cops.len(), 1);
        assert_eq!(cops[0].default_enabled(), *default);
        assert_eq!(cops[0].safe_fixes(), *name == "Style/Copyright");
    }
}

#[test]
fn defaults_only_except_config_enabling_and_never_write_without_an_edit() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, source, position, default) in CASES {
        fs::write(&path, source).unwrap();
        // Isolate the historical Style default assertions from EmptyClass.
        fs::write(
            dir.path().join("rustocop.toml"),
            "[cops.\"Lint/EmptyClass\"]\nenabled = false\n",
        )
        .unwrap();
        let out = cli(dir.path(), &["lib/a.rb"], i32::from(*default));
        assert_eq!(String::from_utf8_lossy(&out.stdout).contains(cop), *default);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("{config}\n[cops.\"Lint/EmptyClass\"]\nenabled = false\n"),
            )
            .unwrap();
            cli(dir.path(), &["lib/a.rb"], 0);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            for fix in [false, true] {
                let mut args = vec!["--only", cop, "lib/a.rb"];
                if fix {
                    args.push("--fix");
                }
                let out = cli(dir.path(), &args, 1);
                assert!(
                    String::from_utf8_lossy(&out.stdout)
                        .contains(&format!("lib/a.rb:{position}: C:")),
                    "{out:?}"
                );
                if fix {
                    assert!(
                        String::from_utf8_lossy(&out.stdout).contains("0 corrections applied"),
                        "{out:?}"
                    );
                }
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
            cli(
                dir.path(),
                &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 1);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
    }
}

#[test]
fn directives_and_additive_config_relative_excludes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, body, _, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [*cop, "Style", "all"] {
                let source = format!("# {prefix}:disable {target}\n{body}");
                fs::write(&path, &source).unwrap();
                cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            }
        }
        fs::write(&path, body).unwrap();
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
            assert_eq!(fs::read(&path).unwrap(), body.as_bytes());
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    // A first-line Copyright offense is not suppressed by a later directive.
    fs::write(&path, "# header\n# rubocop:disable Style/Copyright\n").unwrap();
    cli(dir.path(), &["--only", "Style/Copyright", "lib/a.rb"], 1);
    // Keyword findings resume after enable, unlike globally missing notices.
    fs::write(&path, "# rubocop:disable Style/CommentedKeyword\nclass X # ignored\nend\n# rubocop:enable Style/CommentedKeyword\nclass Y # flagged\nend\n").unwrap();
    let out = cli(
        dir.path(),
        &["--only", "Style/CommentedKeyword", "lib/a.rb"],
        1,
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:5:9:"));
}

#[test]
fn headers_crlf_bom_legacy_encodings_literals_data_and_errors_never_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        b"#!/usr/bin/env ruby\n# encoding: UTF-8\n# frozen_string_literal: true\nclass X # comment\nend\np __LINE__\n".as_slice(),
        b"\xef\xbb\xbf# encoding: UTF-8\r\nclass X # comment\r\nend\r\np __LINE__\r\n__END__\r\n# Copyright 2026 X\r\nend # data\r\n",
        b"# encoding: Windows-1252\nclass X # \xe9\nend\np __LINE__\n",
        b"# encoding: Shift_JIS\nclass X # \x82\xa0\nend\np __LINE__\n",
        b"class X # comment\nend\nx = <<~H\ndef x # literal\n# Copyright 2026 X\nH\n",
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        for cop in ["Style/CommentedKeyword", "Style/Copyright"] {
            cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 1);
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
    let clean = "# Copyright 2026 X\nx = \"end # literal\"\nx = <<~H\ndef x # literal\nH\n__END__\nend # data\n";
    fs::write(&path, clean).unwrap();
    cli(
        dir.path(),
        &[
            "--only",
            "Style/CommentedKeyword,Style/Copyright",
            "--fix",
            "lib/a.rb",
        ],
        0,
    );
    assert_eq!(fs::read(&path).unwrap(), clean.as_bytes());
    let error = b"class X # comment\ndef broken(\n";
    fs::write(&path, error).unwrap();
    cli(
        dir.path(),
        &[
            "--only",
            "Style/CommentedKeyword,Style/Copyright",
            "--fix",
            "lib/a.rb",
        ],
        2,
    );
    assert_eq!(fs::read(&path).unwrap(), error);
}

#[test]
fn controlled_ruby_observations_and_directive_scope_remain_identical() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "#!/usr/bin/env ruby\n# encoding: UTF-8\n# frozen_string_literal: true\nmodule M # description\n  class X # details\n    def self.value # keep\n      [__LINE__, \"é\".encoding.name, \"a\".frozen?]\n    end # close method\n  end # close class\nend # close module\nx = 0\nbegin # meaningful\n  x += 1\nensure\n  x += 10\nend # keep\np [M::X.value, x, __LINE__, DATA.read.bytes]\n__END__\nend # raw data\n",
        "# coding: ASCII-8BIT\ndef value # rubocop:disable Lint/Debugger\n  [__LINE__, \"é\".bytes, \"é\".encoding.name]\nend # rubocop:enable Lint/Debugger\np value\n",
    ] {
        fs::write(&path, source).unwrap();
        let run = || {
            let out = Command::new("ruby").arg(&path).output().unwrap();
            assert!(out.status.success(), "{out:?}");
            out.stdout
        };
        let before = run();
        cli(
            dir.path(),
            &[
                "--only",
                "Style/CommentedKeyword,Style/Copyright",
                "--fix",
                "lib/a.rb",
            ],
            1,
        );
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(run(), before);
    }
}

#[test]
fn stdin_obeys_selectors_excludes_and_directives_without_writing_display_file() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "untouched\n").unwrap();
    for (cop, source, position, _) in CASES {
        for excluded in [false, true] {
            fs::write(
                dir.path().join("rustocop.toml"),
                if excluded {
                    format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n")
                } else {
                    String::new()
                },
            )
            .unwrap();
            for suppressed in [false, true] {
                let source = if suppressed {
                    format!("# rustocop:disable {cop}\n{source}")
                } else {
                    source.to_string()
                };
                let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                    .current_dir(dir.path())
                    .args(["--only", cop, "--stdin", "lib/a.rb"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(source.as_bytes())
                    .unwrap();
                let out = child.wait_with_output().unwrap();
                assert_eq!(
                    out.status.code(),
                    Some(i32::from(!excluded && !suppressed)),
                    "{out:?}"
                );
                assert!(out.stderr.is_empty(), "{out:?}");
                if !excluded && !suppressed {
                    assert!(
                        String::from_utf8_lossy(&out.stdout)
                            .contains(&format!("lib/a.rb:{position}: C:")),
                        "{out:?}"
                    );
                }
                assert_eq!(fs::read(&path).unwrap(), b"untouched\n");
            }
        }
    }
}

#[test]
fn notice_and_annotation_configuration_cannot_expand_the_fixed_policy() {
    let dir = project();
    for (cop, option) in [
        ("Style/Copyright", "notice"),
        ("Style/Copyright", "autocorrect_notice"),
        ("Style/CommentedKeyword", "allowed_comments"),
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\n{option} = \"custom\"\n"),
        )
        .unwrap();
        let out = cli(dir.path(), &["--only", cop, "lib/a.rb"], 2);
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("unknown field"),
            "{out:?}"
        );
    }
}

#[test]
fn unrelated_safe_style_and_layout_edits_do_not_hide_uncorrectable_findings() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "class X # keep\nend\nx = 'text';  \nx\n").unwrap();
    let only = "Style/CommentedKeyword,Style/Copyright,Style/StringLiterals,Style/Semicolon,Layout/TrailingWhitespace";
    cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 1);
    assert_eq!(
        fs::read(&path).unwrap(),
        b"class X # keep\nend\nx = \"text\"\nx\n"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 1);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
