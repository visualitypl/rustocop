//! Actual CLI lifecycle for
//! Layout/EmptyLinesAroundBeginBody, Layout/EmptyLinesAroundBlockBody,
//! Layout/EmptyLinesAroundArguments and
//! Layout/EmptyLinesAroundExceptionHandlingKeywords.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const BEGIN: &str = "Layout/EmptyLinesAroundBeginBody";
const BLOCK: &str = "Layout/EmptyLinesAroundBlockBody";
const ARGS: &str = "Layout/EmptyLinesAroundArguments";
const KEYWORDS: &str = "Layout/EmptyLinesAroundExceptionHandlingKeywords";
const ALL: [&str; 4] = [BEGIN, BLOCK, ARGS, KEYWORDS];

/// `(cop, before, after, first finding)`: correction witnesses.
const WITNESSES: &[(&str, &str, &str, &str)] = &[
    (
        BEGIN,
        "x = begin\n\n  1\n\nend\np x\n",
        "x = begin\n  1\nend\np x\n",
        "example.rb:2:1: C: Extra empty line detected at `begin` body beginning.",
    ),
    (
        BLOCK,
        "[1].each do |x|\n\n  p x\n\nend\n",
        "[1].each do |x|\n  p x\nend\n",
        "example.rb:2:1: C: Extra empty line detected at block body beginning.",
    ),
    (
        ARGS,
        "p(\n\n  1,\n\n  2\n)\n",
        "p(\n  1,\n  2\n)\n",
        "example.rb:2:1: C: Empty line detected around arguments.",
    ),
    (
        KEYWORDS,
        "begin\n  raise \"x\"\n\nrescue\n\n  p 1\nend\n",
        "begin\n  raise \"x\"\nrescue\n  p 1\nend\n",
        "example.rb:3:1: C: Extra empty line detected before the `rescue`.",
    ),
];

fn count_all(out: &Output) -> usize {
    ALL.iter().map(|cop| count(out, cop)).sum()
}

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
    let ruby = std::env::var("RUSTOCOP_RUBY34").ok()?;
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
        assert!(found.default_enabled() && found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        assert!(found.autocorrect_incompatible_with().is_empty());
        assert!(found.department_selection_enables());
        assert_eq!(found.ruby_versions(), Default::default());
        for (config, selectors, n) in [
            (String::new(), vec![], 2),
            (String::new(), vec!["--only", cop], 2),
            (String::new(), vec!["--only", "Layout"], 2),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (String::new(), vec!["--except", "Layout"], 0),
            (String::new(), vec!["--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
            (
                format!("[cops.Layout]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                2,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(dir.path(), before.as_bytes(), &selectors, None);
            assert_eq!(count(&out, cop), n, "{cop} {selectors:?} {out:?}");
            if n > 0 {
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(text.contains(finding), "{text}");
            }
        }
        // No public style knob exists.
        for key in [
            "EnforcedStyle = \"empty_lines\"",
            "EnforcedStyle = \"no_empty_lines\"",
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
    // An unknown lookalike name is rejected, never admitted.
    for name in [
        "Layout/EmptyLinesAroundBeginBodies",
        "Layout/EmptyLinesAroundBlocksBody",
    ] {
        let out = unchanged(
            dir.path(),
            WITNESSES[0].1.as_bytes(),
            &["--only", name],
            Some(2),
        );
        assert_eq!(count(&out, BEGIN), 0);
    }
}

#[test]
fn lint_never_writes_and_fix_writes_the_canonical_edit_once() {
    let dir = project();
    for &(cop, before, after, _) in WITNESSES {
        let out = unchanged(dir.path(), before.as_bytes(), &["--only", cop], Some(1));
        assert_eq!(count(&out, cop), 2, "{out:?}");
        let (out, fixed) = fix(dir.path(), before.as_bytes(), &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, after.as_bytes(), "{cop}");
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", cop, "--fix"],
            Some(0),
        );
        // CRLF line endings and a UTF-8 BOM are preserved around the edit.
        let (_, fixed) = fix(
            dir.path(),
            before.replace('\n', "\r\n").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(fixed, after.replace('\n', "\r\n").as_bytes(), "{cop}");
        let (_, fixed) = fix(
            dir.path(),
            format!("\u{feff}{before}").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(fixed, format!("\u{feff}{after}").as_bytes(), "{cop}");
        // A legacy-encoded comment keeps its bytes.
        let mut legacy = b"# encoding: ISO-8859-15\n# \xa4\n".to_vec();
        legacy.extend_from_slice(before.as_bytes());
        let mut legacy_after = b"# encoding: ISO-8859-15\n# \xa4\n".to_vec();
        legacy_after.extend_from_slice(after.as_bytes());
        let (out, fixed) = fix(dir.path(), &legacy, &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, legacy_after, "{cop}");
        // Each listed source observer and a later `__LINE__` keep the
        // findings and the file.
        for observer in [
            "p __FILE__",
            "p __dir__",
            "p $0",
            "p $PROGRAM_NAME",
            "p caller",
            "p caller_locations",
            "p method(:p).source_location",
            "p SCRIPT_LINES__",
            "p __LINE__",
        ] {
            let held = format!("{before}{observer}\n");
            let out = unchanged(
                dir.path(),
                held.as_bytes(),
                &["--only", cop, "--fix"],
                Some(1),
            );
            assert_eq!(count(&out, cop), 2, "{observer} {out:?}");
        }
        let held = format!("{before}p DATA.read\n__END__\nx\n");
        let out = unchanged(
            dir.path(),
            held.as_bytes(),
            &["--only", cop, "--fix"],
            Some(1),
        );
        assert_eq!(count(&out, cop), 2, "{out:?}");
        // An earlier `__LINE__` is not moved by the removal.
        let (out, fixed) = fix(
            dir.path(),
            format!("p __LINE__\n{before}").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, format!("p __LINE__\n{after}").as_bytes());
    }
}

#[test]
fn directives_stdin_and_parse_errors() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for &(cop, before, _, finding) in WITNESSES {
            for disable in [
                format!("# {prefix}:disable {cop}\n"),
                format!("# {prefix}:disable Layout\n"),
                format!("# {prefix}:disable all\n"),
            ] {
                let source = format!("{disable}{before}");
                let out = unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix"],
                    Some(0),
                );
                assert_eq!(count(&out, cop), 0, "{source:?}");
            }
            // A trailing disable on the first blank line's neighbour keeps
            // both findings but holds the adjacent edit.
            let first: usize = finding.split(':').nth(1).unwrap().parse().unwrap();
            let mut lines: Vec<String> = before.lines().map(String::from).collect();
            lines[first - 2].push_str(&format!(" # {prefix}:disable Lint/Debugger"));
            let source = lines.join("\n") + "\n";
            let out = unchanged(dir.path(), source.as_bytes(), &["--only", cop], Some(1));
            assert_eq!(count(&out, cop), 2, "{source:?}");
            let (_, fixed) = fix(dir.path(), source.as_bytes(), &["--only", cop]);
            let fixed = String::from_utf8(fixed).unwrap();
            assert!(fixed.contains("Lint/Debugger\n\n"), "{fixed}");
            assert_eq!(fixed.matches("\n\n").count(), 1, "{fixed}");
        }
    }
    // stdin reports at the supplied path and never writes.
    for &(cop, before, _, finding) in WITNESSES {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "virtual.rb"])
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
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains(&finding.replace("example.rb", "virtual.rb")),
            "{text}"
        );
        assert!(!dir.path().join("virtual.rb").exists());
    }
    // An invalid file is an error, never a finding or a write.
    for &(cop, before, _, _) in WITNESSES {
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

/// The removals keep the program's result; held edits keep the file.
#[test]
fn removals_keep_meaning() {
    let dir = project();
    let selection = ALL.join(",");
    for (source, fixed, result) in [
        (
            "x = begin\n\n  [1, 2].map do |a,\n    b = 2|\n\n    a * b\n\n  end\n\nrescue\n  0\nend\np x\n",
            Some(
                "x = begin\n  [1, 2].map do |a,\n    b = 2|\n    a * b\n  end\nrescue\n  0\nend\np x\n",
            ),
            "[2, 4]\n",
        ),
        // Numbered, `it` and lambda bodies.
        (
            "p [1].map {\n\n  _1 + 1\n}, [2].map {\n  it * 2\n\n}, -> do\n\n  3\nend.call\n",
            Some("p [1].map {\n  _1 + 1\n}, [2].map {\n  it * 2\n}, -> do\n  3\nend.call\n"),
            "[2]\n[4]\n3\n",
        ),
        // A magic comment that the removal moves up is still after code.
        (
            "begin\n\n# encoding: binary\n  p __ENCODING__\nend\n",
            Some("begin\n# encoding: binary\n  p __ENCODING__\nend\n"),
            "#<Encoding:UTF-8>\n",
        ),
        // A heredoc body opened on the header line: held, value unchanged.
        (
            "x = [<<~H].map do |a|\n\nH\n  a\nend\np x\n",
            None,
            "[\"\\n\"]\n",
        ),
        ("p(<<~H + begin\n\nH\n  \"1\"\nend)\n", None, "\"\\n1\"\n"),
        // A backslash continuation before the blank line: held.
        ("p [1].map { |a| \\\n\n  a\n}\n", None, "[1]\n"),
        // Arguments and keywords with blocks and `begin` bodies; one blank
        // line found by two cops is removed once.
        (
            "p(\n\n  [1].map do |x|\n\n    x\n  end,\n\n  2\n)\n",
            Some("p(\n  [1].map do |x|\n    x\n  end,\n  2\n)\n"),
            "[1]\n2\n",
        ),
        (
            "x = [1].map do |a|\n  raise \"e\" if a == 1\n\nrescue\n\n  :r\nensure\n\n  :ignored\nend\np x\n",
            Some(
                "x = [1].map do |a|\n  raise \"e\" if a == 1\nrescue\n  :r\nensure\n  :ignored\nend\np x\n",
            ),
            "[:r]\n",
        ),
        (
            "x = begin\n\nrescue\n\nend\np x\n",
            Some("x = begin\nrescue\nend\np x\n"),
            "nil\n",
        ),
        // Heredoc text before an argument is never touched.
        (
            "p(<<~H,\n\n  a\nH\n\n  1)\n",
            Some("p(<<~H,\n\n  a\nH\n  1)\n"),
            "\"\\na\\n\"\n1\n",
        ),
        // A heredoc body opened on a keyword's line: held.
        (
            "x = begin\n  raise \"e\"\nrescue then <<~H\n\nH\nend\np x\n",
            None,
            "\"\\n\"\n",
        ),
        // String text and `__END__` data are not padding.
        (
            "x = begin\n  \"a\n\nb\"\nend\np x\n__END__\nbegin\n\nend\n",
            None,
            "\"a\\n\\nb\"\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        if let Some(before) = ruby(dir.path()) {
            assert_eq!(before, result, "{source:?}");
        }
        match fixed {
            Some(fixed) => {
                let (out, after) = fix(dir.path(), source.as_bytes(), &["--only", &selection]);
                assert_eq!(out.status.code(), Some(0), "{out:?}");
                assert_eq!(String::from_utf8_lossy(&after), fixed);
            }
            None => {
                let out = unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", &selection, "--fix"],
                    None,
                );
                assert_eq!(
                    out.status.code(),
                    Some((count_all(&out) > 0) as i32),
                    "{out:?}"
                );
            }
        }
        if let Some(after) = ruby(dir.path()) {
            assert_eq!(after, result, "{source:?}");
        }
    }
}

/// The body removals compose with the class/module/method body cops,
/// `EmptyLines` and the access modifier cop under the default selection,
/// converge, and keep the program's result.
#[test]
fn body_removals_converge_with_the_default_selection() {
    let dir = project();
    let source = "module M\n\n  def self.run\n\n    begin\n\n\n      [1, 2].each_slice(2).map do |a,\n        b|\n\n        Class.new do\n\n          private\n\n          def v = 1\n\n        end\n\n        a + b\n\n      end\n\n    end\n\n  end\n\nend\np M.run\n";
    let path = dir.path().join("example.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    if let Some(before) = &before {
        assert_eq!(before, "[3]\n");
    }
    let mut current = source.as_bytes().to_vec();
    for _ in 0..6 {
        let (_, fixed) = fix(dir.path(), &current, &[]);
        assert_eq!(ruby(dir.path()), before);
        if fixed == current {
            break;
        }
        current = fixed;
    }
    let out = unchanged(dir.path(), &current, &["--fix"], None);
    assert_eq!(count_all(&out), 0, "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&current),
        // Default-on Layout/MultilineBlockLayout joins the split
        // block parameters, as upstream.
        "module M\n  def self.run\n    begin\n      [1, 2].each_slice(2).map do |a, b|\n        Class.new do\n          private\n\n          def v = 1\n        end\n\n        a + b\n      end\n    end\n  end\nend\np M.run\n"
    );
}

/// Argument and exception-keyword removals apply together with the body
/// cops in one default-selection pass, converge and keep the result.
#[test]
fn argument_and_keyword_removals_converge_with_the_default_selection() {
    let dir = project();
    let source = "def run(a)\n\n  [a].map do |x|\n    Integer(\n\n      x\n\n    )\n\n  rescue ArgumentError\n\n    0\n\n  end\n\nrescue\n\n  nil\nend\np run(\"1\"), run(\"z\")\n";
    fs::write(dir.path().join("example.rb"), source).unwrap();
    let before = ruby(dir.path());
    if let Some(before) = &before {
        assert_eq!(before, "[1]\n[0]\n");
    }
    let (out, fixed) = fix(dir.path(), source.as_bytes(), &[]);
    assert_eq!(ruby(dir.path()), before);
    assert_eq!(count_all(&out), 0, "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&fixed),
        "def run(a)\n  [a].map do |x|\n    Integer(\n      x\n    )\n  rescue ArgumentError\n    0\n  end\nrescue StandardError\n  nil\nend\np run(\"1\"), run(\"z\")\n"
    );
    let out = unchanged(dir.path(), &fixed, &["--fix"], None);
    assert_eq!(count_all(&out), 0, "{out:?}");
}
