//! real selection/config/directive/fix paths and controlled Ruby semantics.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const CASES: &[(&str, &str, &str, &str)] = &[
    ("Style/Encoding", "# encoding: utf-8\nx\n", "x\n", "1:1"),
    (
        "Style/MagicCommentFormat",
        "# frozen-string-literal: true\nx\n",
        "# frozen_string_literal: true\nx\n",
        "1:3",
    ),
    (
        "Style/InlineComment",
        "x # comment\n",
        "x # comment\n",
        "1:3",
    ),
];
/// The magic-comment fixtures are also default-on
/// Layout/EmptyLineAfterMagicComment findings; it is not the subject here.
const MAGIC_OFF: &str = "[cops.\"Layout/EmptyLineAfterMagicComment\"]\nenabled = false\n";
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
fn defaults_opt_in_exact_names_selectors_config_and_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let registry = rustocop::cops::Registry::new();
    for (cop, before, after, position) in CASES {
        let inline = *cop == "Style/InlineComment";
        let matched: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == *cop)
            .collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].default_enabled(), !inline);
        assert_eq!(matched[0].safe_fixes(), !inline);
        fs::write(&path, before).unwrap();
        fs::write(dir.path().join("rustocop.toml"), MAGIC_OFF).unwrap();
        let out = cli(dir.path(), &["lib/a.rb"], i32::from(!inline));
        assert_eq!(String::from_utf8_lossy(&out.stdout).contains(cop), !inline);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("{config}{MAGIC_OFF}"),
            )
            .unwrap();
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
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n{MAGIC_OFF}"),
        )
        .unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], i32::from(inline));
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &["--only", cop, "--fix", "lib/a.rb"],
            i32::from(inline),
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
                let source = if *cop == "Style/Encoding" {
                    format!("# encoding: utf-8 # {prefix}:disable {target}\nx\n")
                } else {
                    format!("# {prefix}:disable {target}\n{before}")
                };
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
fn stdin_uses_display_path_excludes_and_directives_without_writing() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "sentinel\n").unwrap();
    for (cop, before, _, position) in CASES {
        let run = |source: &str, status| {
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
            assert_eq!(out.status.code(), Some(status), "{out:?}");
            assert!(out.stderr.is_empty(), "{out:?}");
            out
        };
        let out = run(before, 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("lib/a.rb:{position}:")));
        for prefix in ["rubocop", "rustocop"] {
            let source = if *cop == "Style/Encoding" {
                format!("# encoding: utf-8 # {prefix}:disable {cop}\nx\n")
            } else {
                format!("# {prefix}:disable {cop}\n{before}")
            };
            run(&source, 0);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
        )
        .unwrap();
        run(before, 0);
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"sentinel\n");
    }
}

#[test]
fn bom_crlf_headers_literals_line_and_data_bytes_are_preserved() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "\u{feff}#!/usr/bin/env ruby\r\n# -*- encoding: UTF-8; frozen-string-literal: true -*-\r\np [__LINE__, \"é\".encoding, \"a\".frozen?]\r\ns = <<~H\r\n# encoding: utf-8\r\n# FROZEN-STRING-LITERAL: true\r\nH\r\n__END__\r\n# encoding: utf-8\r\n# frozen-string-literal: true\r\n";
    let after = source.replace(
        "# -*- encoding: UTF-8; frozen-string-literal: true -*-",
        "# -*- frozen_string_literal: true -*-",
    );
    fs::write(&path, source).unwrap();
    // Overlapping Encoding/Format edits resolve deterministically, one pass.
    let only = "Style/Encoding,Style/MagicCommentFormat";
    cli(
        dir.path(),
        &["--only", only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        1,
    );
    cli(
        dir.path(),
        &["--only", only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        0,
    );
    assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(
        dir.path(),
        &["--only", only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        0,
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn legacy_encoding_header_normalization_does_not_transcode_literal_bytes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for encoding in ["ASCII-8BIT", "Windows-1252", "Shift_JIS"] {
        let mut source =
            format!("# EnCoDiNg: {encoding}\r\n# frozen-STRING-literal: true\r\nx = \"")
                .into_bytes();
        source.extend_from_slice(b"\x82\xa0\"\r\n__END__\r\n\xff\r\n");
        let mut after =
            format!("# encoding: {encoding}\r\n# frozen_string_literal: true\r\nx = \"")
                .into_bytes();
        after.extend_from_slice(b"\x82\xa0\"\r\n__END__\r\n\xff\r\n");
        fs::write(&path, &source).unwrap();
        cli(
            dir.path(),
            &["--only", "Style/MagicCommentFormat", "--fix", "lib/a.rb"],
            0,
        );
        assert_eq!(fs::read(&path).unwrap(), after);
    }
}

#[test]
fn invalid_utf8_editor_replacements_are_withheld_without_lossy_transcoding() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = b"# -*- encoding: utf-8; mode: \xff -*-\nx\n";
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(
        dir.path(),
        &["--only", "Style/Encoding", "--fix", "lib/a.rb"],
        // Syntax now owns the real global buffer exception. Invalid source
        // never reaches ordinary Style hooks, even with Syntax unselected.
        2,
    );
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn encoding_fix_does_not_activate_frozen_directive_after_interior_emacs_delimiter() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "# -*- encoding: utf-8; encoding: utf-8 -*-; frozen_string_literal: true -*-\np \"a\".frozen?\n";
    fs::write(&path, source).unwrap();
    let run = || {
        let out = Command::new("ruby").arg(&path).output().unwrap();
        assert!(out.status.success(), "{out:?}");
        out.stdout
    };
    assert_eq!(run(), b"false\n");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", "Style/Encoding", "--fix", "lib/a.rb"])
        .output()
        .unwrap();
    assert_eq!(run(), b"false\n", "{out:?}");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Style/Encoding"));
    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
