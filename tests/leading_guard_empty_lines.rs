//! Actual CLI lifecycle for
//! Layout/LeadingEmptyLines, Layout/EndOfLine, Layout/EmptyLineAfterMagicComment,
//! Layout/EmptyLineAfterGuardClause and Layout/EmptyLineAfterMultilineCondition.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const LEADING: &str = "Layout/LeadingEmptyLines";
const EOL: &str = "Layout/EndOfLine";
const MAGIC: &str = "Layout/EmptyLineAfterMagicComment";
const GUARD: &str = "Layout/EmptyLineAfterGuardClause";

/// `(cop, before, after, first finding)`: correction witnesses (`after` is
/// `before` for the detection-only cop).
const WITNESSES: &[(&str, &str, &str, &str)] = &[
    (
        LEADING,
        "\nputs 1\n",
        "puts 1\n",
        "example.rb:2:1: C: Unnecessary blank line at the beginning of the source.",
    ),
    (
        EOL,
        "x = 1\r\n",
        "x = 1\r\n",
        "example.rb:1:1: C: Carriage return character detected.",
    ),
    (
        MAGIC,
        "# frozen_string_literal: true\nx = 1\n",
        "# frozen_string_literal: true\n\nx = 1\n",
        "example.rb:2:1: C: Expected at least 1 empty line after magic comments; found 0.",
    ),
    (
        GUARD,
        "def f(a)\n  return if a\n  a\nend\n",
        "def f(a)\n  return if a\n\n  a\nend\n",
        "example.rb:2:3: C: Add empty line after guard clause.",
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

fn fix(root: &Path, source: &[u8], args: &[&str]) -> (Output, Vec<u8>) {
    let path = root.join("example.rb");
    fs::write(&path, source).unwrap();
    let mut args = args.to_vec();
    args.extend(["--fix", "example.rb"]);
    let out = run(root, &args);
    (out, fs::read(&path).unwrap())
}

/// The native Ruby 3.4 result of `example.rb`, when `RUSTOCOP_RUBY34` is
/// configured.
fn ruby(root: &Path) -> Option<String> {
    let ruby = crate::support::runtime::ruby().ok()?;
    let run = Command::new(ruby)
        .current_dir(root)
        .args(["--parser=prism", "-W0", "example.rb"])
        .output()
        .unwrap();
    assert!(run.status.success(), "{run:?}");
    Some(String::from_utf8_lossy(&run.stdout).into_owned())
}

#[test]
fn metadata_defaults_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    for &(cop, before, _, finding) in WITNESSES {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(found.default_enabled(), "{cop}");
        assert_eq!(found.safe_fixes(), cop != EOL, "{cop}");
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
        // No public style or count knob exists.
        for key in [
            "EnforcedStyle = \"crlf\"",
            "EnforcedStyle = \"native\"",
            "NumberOfEmptyLines = 2",
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

/// EndOfLine is detection-only: `--fix` never normalizes line endings, alone
/// or while other cops correct the same file.
#[test]
fn end_of_line_never_normalizes_bytes() {
    let dir = project();
    for source in [
        b"x = 1\r\n".as_slice(),
        b"x = 1\r\ny = 2\n",
        b"\xef\xbb\xbfx = 1\r\n",
        b"# coding: ISO-8859-15\r\n# Euro symbol: \xa4\r\n",
        b"x = <<~X\r\n  a\r\nX\r\np x\r\n",
    ] {
        let out = unchanged(dir.path(), source, &["--only", EOL, "--fix"], Some(1));
        assert_eq!(count(&out, EOL), 1, "{out:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("0 corrections applied"),
            "{out:?}"
        );
    }
    // Another cop's edit keeps every CR LF; the finding remains.
    let (out, fixed) = fix(
        dir.path(),
        b"x = 1  \r\ny = 2\r\n",
        &["--only", "Layout/TrailingWhitespace,Layout/EndOfLine"],
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(fixed, b"x = 1\r\ny = 2\r\n");
    assert_eq!(count(&out, EOL), 1, "{out:?}");
    // Lines after the last token and the data section are not checked.
    for source in [
        b"x = 1\n\r\n".as_slice(),
        b"x = 1\n# done\r\n",
        b"x = 1\n__END__\r\ndata\r\n",
    ] {
        let out = unchanged(dir.path(), source, &["--only", EOL, "--fix"], Some(0));
        assert_eq!(count(&out, EOL), 0, "{out:?}");
    }
}

/// Moving a `#!` or `coding` comment into lines 1-2 changes what Ruby reads
/// there; the removal is held, alone and composed with EmptyComment's
/// whole-line removals, and the native result stays the original one.
#[test]
fn leading_removal_keeps_header_meaning() {
    let dir = project();
    for (source, selection, result) in [
        (
            "\n# coding: binary\np \"\u{e9}\".encoding\n",
            LEADING,
            "#<Encoding:UTF-8>\n",
        ),
        // On line 1 the shebang's `-w` would set `$VERBOSE` (nil under -W0).
        ("\n#!ruby -w\np $VERBOSE\n", LEADING, "nil\n"),
        (
            "\n#\n  # coding: binary\np \"\u{e9}\".encoding\n",
            "Layout/LeadingEmptyLines,Layout/EmptyComment",
            "#<Encoding:UTF-8>\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        if let Some(before) = &before {
            assert_eq!(before, result, "{source:?}");
        }
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", selection, "--fix"],
            Some(1),
        );
        assert_eq!(count(&out, LEADING), 1, "{out:?}");
        assert_eq!(ruby(dir.path()), before);
    }
    // The default selection converges without changing the result.
    let source = "\n#\n  # coding: binary\np \"\u{e9}\".encoding\n";
    fs::write(dir.path().join("example.rb"), source).unwrap();
    let before = ruby(dir.path());
    for _ in 0..3 {
        let path = dir.path().join("example.rb");
        let current = fs::read(&path).unwrap();
        let (_, fixed) = fix(dir.path(), &current, &[]);
        assert_eq!(
            ruby(dir.path()),
            before,
            "{:?}",
            String::from_utf8_lossy(&fixed)
        );
    }
    // After the first code token a coding comment can never reach line 1.
    let (out, fixed) = fix(
        dir.path(),
        b"\np 1\n# coding: binary\n",
        &["--only", LEADING],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fixed, b"p 1\n# coding: binary\n");
}

/// The guard and magic-comment insertions keep the program's result, hold
/// where a heredoc body continues on the next line, and an `enable`
/// directive in either spelling may sit between the guard and the blank line.
#[test]
fn guard_and_magic_insertions_keep_meaning() {
    let dir = project();
    for (source, fixed, result) in [
        (
            "# frozen_string_literal: true\np \"a\".frozen?\n",
            Some("# frozen_string_literal: true\n\np \"a\".frozen?\n"),
            "true\n",
        ),
        (
            "def f(a)\n  return :early if a\n  :late\nend\np f(true), f(false)\n",
            Some("def f(a)\n  return :early if a\n\n  :late\nend\np f(true), f(false)\n"),
            ":early\n:late\n",
        ),
        (
            "def f(a)\n  # rubocop:disable Style/Foo\n  return :early if a\n  # rustocop:enable Style/Foo\n  :late\nend\np f(true)\n",
            Some(
                "def f(a)\n  # rubocop:disable Style/Foo\n  return :early if a\n  # rustocop:enable Style/Foo\n\n  :late\nend\np f(true)\n",
            ),
            ":early\n",
        ),
        // The body of `<<~A` starts on the line after the guard: held.
        (
            "def f(a)\n  return [<<~A].first if a\n    body\n  A\n\n  :late\nend\np f(true)\n",
            None,
            "\"body\\n\"\n",
        ),
        // The second heredoc's body starts after the first terminator: held.
        (
            "def f(a)\n  raise format(<<~A, <<~B) if a\n    one\n  A\n    two\n  B\n  :late\nend\nbegin\n  f(true)\nrescue => e\n  p e.message\nend\n",
            None,
            "\"one\\n\"\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        if let Some(before) = ruby(dir.path()) {
            assert_eq!(before, result, "{source:?}");
        }
        let selection = "Layout/EmptyLineAfterMagicComment,Layout/EmptyLineAfterGuardClause";
        match fixed {
            Some(fixed) => {
                let (out, after) = fix(dir.path(), source.as_bytes(), &["--only", selection]);
                assert_eq!(out.status.code(), Some(0), "{out:?}");
                assert_eq!(String::from_utf8_lossy(&after), fixed);
            }
            None => {
                let out = unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", selection, "--fix"],
                    Some(1),
                );
                assert_eq!(count(&out, GUARD), 1, "{out:?}");
            }
        }
        if let Some(after) = ruby(dir.path()) {
            assert_eq!(after, result, "{source:?}");
        }
    }
}

/// LeadingEmptyLines' removal and the magic-comment insertion apply together
/// in one pass; the default selection converges.
#[test]
fn leading_and_magic_compose() {
    let dir = project();
    let (out, fixed) = fix(
        dir.path(),
        b"\n# frozen_string_literal: true\nx = 1\np x\n",
        &[
            "--only",
            "Layout/LeadingEmptyLines,Layout/EmptyLineAfterMagicComment",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fixed, b"# frozen_string_literal: true\n\nx = 1\np x\n");
    let source =
        b"\n# frozen_string_literal: true\ndef f(a)\n  return 1 if a\n  2\nend\np f(false)\n";
    let path = dir.path().join("example.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    let mut current = source.to_vec();
    for _ in 0..3 {
        let (_, fixed) = fix(dir.path(), &current, &[]);
        assert_eq!(ruby(dir.path()), before);
        if fixed == current {
            break;
        }
        current = fixed;
    }
    assert_eq!(
        String::from_utf8_lossy(&current),
        "# frozen_string_literal: true\n\ndef f(a)\n  return 1 if a\n\n  2\nend\np f(false)\n"
    );
}

/// The guard-clause and multi-line-condition insertions at the same line end
/// are one line break, and the program's result is unchanged.
#[test]
fn guard_and_condition_insertions_compose() {
    let dir = project();
    let source = "def f(a, b)\n  return :early if a &&\n    b\n  :late\nend\np f(true, true), f(true, false)\n";
    fs::write(dir.path().join("example.rb"), source).unwrap();
    let before = ruby(dir.path());
    let (out, after) = fix(
        dir.path(),
        source.as_bytes(),
        &[
            "--only",
            "Layout/EmptyLineAfterGuardClause,Layout/EmptyLineAfterMultilineCondition",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&after),
        "def f(a, b)\n  return :early if a &&\n    b\n\n  :late\nend\np f(true, true), f(true, false)\n"
    );
    assert_eq!(ruby(dir.path()), before);
    if let Some(before) = before {
        assert_eq!(before, ":early\n:late\n");
    }
}
