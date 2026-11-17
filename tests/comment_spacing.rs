//! Actual CLI lifecycle for Layout/SpaceBeforeComment,
//! Layout/LeadingCommentSpace and Layout/EmptyComment.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const SPACE: &str = "Layout/SpaceBeforeComment";
const LEADING: &str = "Layout/LeadingCommentSpace";
const EMPTY: &str = "Layout/EmptyComment";

/// `(cop, before, after, first finding)`: the inventory's correction witnesses.
const WITNESSES: [(&str, &str, &str, &str); 3] = [
    (
        SPACE,
        "a += 1# increment\n",
        "a += 1 # increment\n",
        "example.rb:1:7: C: Put a space before an end-of-line comment.",
    ),
    (
        LEADING,
        "#missing space\n",
        "# missing space\n",
        "example.rb:1:1: C: Missing space after `#`.",
    ),
    (
        EMPTY,
        "#\n",
        "",
        "example.rb:1:1: C: Source code comment is empty.",
    ),
];

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({cop})"))
        .count()
}

/// Run `args` on `example.rb` holding `source`; the file must keep its bytes,
/// mtime, permissions and inode.
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: Option<i32>) -> Output {
    let path = root.join("example.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let mut args = args.to_vec();
    args.push("example.rb");
    let out = run(root, &args);
    if code.is_some() {
        assert_eq!(out.status.code(), code, "{args:?} {out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
    out
}

fn fix(root: &Path, source: &str, args: &[&str]) -> (Output, String) {
    let path = root.join("example.rb");
    fs::write(&path, source).unwrap();
    let mut args = args.to_vec();
    args.extend(["--fix", "--fix-passes", "1", "example.rb"]);
    let out = run(root, &args);
    (out, fs::read_to_string(&path).unwrap())
}

#[test]
fn metadata_defaults_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    for (cop, before, _, finding) in WITNESSES {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(found.default_enabled() && found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        assert!(found.autocorrect_incompatible_with().is_empty());
        assert!(found.department_selection_enables());
        for (config, selectors, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Layout"], 1),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (String::new(), vec!["--except", "Layout"], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(dir.path(), before.as_bytes(), &selectors, None);
            assert_eq!(count(&out, cop), n, "{cop} {selectors:?} {out:?}");
            if n == 1 {
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(text.contains(finding), "{text}");
            }
        }
        // No public allowance or style knob exists.
        for key in [
            "AllowBorderComment = false",
            "AllowMarginComment = false",
            "AllowDoxygenCommentStyle = true",
            "AllowRBSInlineAnnotation = true",
            "max = 1",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", cop, "--fix"],
                Some(2),
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn magic_comments_shebangs_and_directives_keep_their_meaning() {
    let dir = project();
    let only = ["--only", LEADING];
    for (before, after) in [
        (
            "#frozen_string_literal: true\nx = 'a'\n",
            "# frozen_string_literal: true\nx = 'a'\n",
        ),
        (
            "#!/usr/bin/env ruby\n#encoding: binary\nx = 'a'\n",
            "#!/usr/bin/env ruby\n# encoding: binary\nx = 'a'\n",
        ),
        (
            "#rubocop:disable Lint/Debugger\nbinding.pry\n",
            "# rubocop:disable Lint/Debugger\nbinding.pry\n",
        ),
    ] {
        let (out, fixed) = fix(dir.path(), before, &only);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, after);
    }
    for source in [
        "#!/usr/bin/env nix-shell\n#! nix-shell -i ruby\nx = 1\n",
        "#!ruby -w\n#\n#\n",
    ] {
        let out = unchanged(dir.path(), source.as_bytes(), &only, Some(0));
        assert_eq!(count(&out, LEADING), 0, "{out:?}");
    }
    // Removing line 1 would turn `#!ruby -w` into the shebang.
    let out = unchanged(
        dir.path(),
        b"  #\n#!ruby -w\nx = 1\n",
        &["--only", EMPTY, "--fix"],
        Some(1),
    );
    assert_eq!(count(&out, EMPTY), 1, "{out:?}");
    for prefix in ["rubocop", "rustocop"] {
        for (cop, before, _, _) in WITNESSES {
            for source in [
                format!("# {prefix}:disable {cop}\n{before}"),
                format!("# {prefix}:disable Layout\n{before}"),
                format!("# {prefix}:push -{cop}\n{before}# {prefix}:pop\n"),
            ] {
                let out = unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix"],
                    Some(0),
                );
                assert_eq!(count(&out, cop), 0, "{source}");
            }
        }
    }
}

#[test]
fn default_selection_converges() {
    let dir = project();
    // SpaceBeforeComment's insertion wins the first pass; the next pass
    // removes the empty comment.
    let (out, after) = fix(dir.path(), "def foo#\n  1\nend\n", &[]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(after, "def foo #\n  1\nend\n");
    let (out, after) = fix(dir.path(), &after, &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(after, "def foo\n  1\nend\n");
    unchanged(
        dir.path(),
        after.as_bytes(),
        &["--fix", "--fix-passes", "1"],
        Some(0),
    );
    let (out, after) = fix(dir.path(), "p 1#note\n#\n#text\np 2\n", &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(after, "p 1 # note\n#\n# text\np 2\n");
    unchanged(
        dir.path(),
        after.as_bytes(),
        &["--fix", "--fix-passes", "1"],
        Some(0),
    );
}

#[test]
fn stdin_paths_and_parse_errors() {
    let dir = project();
    let stdin = |path: &str, source: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", LEADING, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    // The rackup line is accepted on line 1 of a file named config.ru only.
    let out = stdin("config.ru", "#\\ -p 8765\nrun App\n");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let out = stdin("app/config.ru", "#\\ -p 8765\nrun App\n");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let out = stdin("virtual.rb", "#\\ -p 8765\nrun App\n");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("virtual.rb:1:1: C: "), "{text}");
    // An invalid file is an error, never a finding or a write.
    for (cop, before, _, _) in WITNESSES {
        let invalid = format!("{before}def\n");
        let out = unchanged(
            dir.path(),
            invalid.as_bytes(),
            &["--only", cop, "--fix"],
            Some(2),
        );
        assert_eq!(count(&out, cop), 0, "{out:?}");
    }
}

/// Whole-line removals that together move a coding
/// comment into line 1, or that join a backslash continuation with the next
/// line, change the program's result. Both are held: findings kept, no write,
/// and the native Ruby 3.4 result is the original one.
#[test]
fn composed_header_moves_and_continuations_are_held() {
    let dir = project();
    for (source, findings, result) in [
        (
            "#\n #\n  #\n# coding: binary\np \"\u{e9}\".encoding\n",
            3,
            "#<Encoding:UTF-8>\n",
        ),
        ("x = 1 \\\n  #\n+ 2\np x\n", 1, "1\n"),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", EMPTY, "--fix"],
            Some(1),
        );
        assert_eq!(count(&out, EMPTY), findings, "{out:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("0 corrections"),
            "{out:?}"
        );
        if let Ok(ruby) = crate::support::runtime::ruby() {
            let run = Command::new(ruby)
                .current_dir(dir.path())
                .args(["--parser=prism", "-W0", "example.rb"])
                .output()
                .unwrap();
            assert!(run.status.success(), "{run:?}");
            assert_eq!(String::from_utf8_lossy(&run.stdout), result, "{source:?}");
        }
    }
}
