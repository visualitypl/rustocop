//! Actual CLI lifecycle for Layout/ExtraSpacing,
//! Layout/SpaceBeforeFirstArg and Layout/SpaceBeforeSemicolon.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const EXTRA: &str = "Layout/ExtraSpacing";

const FIRST: &str = "Layout/SpaceBeforeFirstArg";
const SEMI: &str = "Layout/SpaceBeforeSemicolon";

/// `(cop, before, after, findings)`: correction witnesses with two findings.
const WITNESSES: &[(&str, &str, &str, [&str; 2])] = &[
    (
        EXTRA,
        "p(1,  2)\nx = 3  # c\n",
        "p(1, 2)\nx = 3 # c\n",
        [
            "example.rb:1:5: C: Unnecessary spacing detected.",
            "example.rb:2:6: C: Unnecessary spacing detected.",
        ],
    ),
    (
        FIRST,
        "[1].push  2\nputs'a'\n",
        "[1].push 2\nputs 'a'\n",
        [
            "example.rb:1:9: C: Put one space between the method name and the first argument.",
            "example.rb:2:5: C: Put one space between the method name and the first argument.",
        ],
    ),
    (
        SEMI,
        "x = 1 ; p x\np x\t;\n",
        "x = 1; p x\np x;\n",
        [
            "example.rb:1:6: C: Space found before semicolon.",
            "example.rb:2:4: C: Space found before semicolon.",
        ],
    ),
];

/// Each cop's upstream `autocorrect_incompatible_with`.
fn incompatible(cop: &str) -> &'static [&'static str] {
    match cop {
        FIRST => &["Style/MethodCallWithArgsParentheses"],
        _ => &[],
    }
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
    for &(cop, before, _, findings) in WITNESSES {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(found.default_enabled() && found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        assert_eq!(found.autocorrect_incompatible_with(), incompatible(cop));
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
                for finding in findings {
                    assert!(text.contains(finding), "{text}");
                }
            }
        }
        // No public style, alignment or threshold knob exists.
        for key in [
            "AllowForAlignment = false",
            "AllowBeforeTrailingComments = true",
            "ForceEqualSignAlignment = true",
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
    // Unknown lookalike names are rejected, never admitted.
    for name in [
        "Layout/ExtraSpaces",
        "Layout/ExtraSpacings",
        "Layout/SpaceBeforeFirstArgument",
        "Layout/SpaceBeforeSemicolons",
    ] {
        let out = unchanged(
            dir.path(),
            WITNESSES[0].1.as_bytes(),
            &["--only", name],
            Some(2),
        );
        assert_eq!(count(&out, EXTRA), 0);
    }
}

#[test]
fn directives_stdin_and_parse_errors() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for &(cop, before, _, _) in WITNESSES {
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
            // A same-line disable silences only its own line.
            let mut lines: Vec<String> = before.lines().map(String::from).collect();
            lines[0].push_str(&format!(" # {prefix}:disable {cop}"));
            let source = lines.join("\n") + "\n";
            let out = unchanged(dir.path(), source.as_bytes(), &["--only", cop], Some(1));
            assert_eq!(count(&out, cop), 1, "{source:?}");
        }
    }
    // stdin reports at the supplied path and never writes.
    for &(cop, before, _, findings) in WITNESSES {
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
        for finding in findings {
            assert!(
                text.contains(&finding.replace("example.rb", "virtual.rb")),
                "{text}"
            );
        }
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

/// The spacing edits keep the program's result; spaces that decide how Ruby
/// lexes a token (unary operators, heredoc openers, percent delimiters,
/// comments) keep one blank.
#[test]
fn spacing_edits_keep_meaning() {
    let dir = project();
    for (source, fixed, result) in [
        (
            "a = [1,  2]\nb = -  1\np a,  b  # sum\np(a.sum  +  b)\n",
            "a = [1, 2]\nb = - 1\np a, b # sum\np(a.sum + b)\n",
            "[1, 2]\n-1\n2\n",
        ),
        (
            "def m(*a) = a\np m  -1\np m  *[2]\np m  :x\np(m  [3])\n",
            "def m(*a) = a\np m -1\np m *[2]\np m :x\np(m [3])\n",
            "[-1]\n[2]\n[:x]\n[[3]]\n",
        ),
        (
            "x = 4\np x  /2\np(x+1  ? 1 : 2)\np  <<~H\n  a  b\nH\np  %w[c  d]\n",
            "x = 4\np x /2\np(x+1 ? 1 : 2)\np <<~H\n  a  b\nH\np %w[c  d]\n",
            "2\n1\n\"a  b\\n\"\n[\"c\", \"d\"]\n",
        ),
        (
            "p \"#{1  + 1}  x\"\np(  1)\n",
            "p \"#{1 + 1}  x\"\np( 1)\n",
            "\"2  x\"\n1\n",
        ),
        // First arguments: the blank deciding unary/splat/bracket/regexp
        // readings stays; a quoted string gets its missing blank.
        (
            "def m(*a) = a\np  m  -1\nx = m  [2]\nyy = m  /3/\nzzz = m  ?c\np [x, yy, zzz]\nputs'q' ; puts\"#{4}\"  ;\n",
            "def m(*a) = a\np m -1\nx = m [2]\nyy = m /3/\nzzz = m ?c\np [x, yy, zzz]\nputs 'q'; puts \"#{4}\";\n",
            "[-1]\n[[[2]], [/3/], [\"c\"]]\nq\n4\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        if let Some(before) = ruby(dir.path()) {
            assert_eq!(before, result, "{source:?}");
        }
        // One `--fix` run applies one round of non-overlapping edits; an
        // ExtraSpacing gap before `;` overlaps the semicolon removal and
        // converges on the next run.
        let selection = [EXTRA, FIRST, SEMI].join(",");
        let mut after = source.as_bytes().to_vec();
        for _ in 0..3 {
            let (out, next) = fix(dir.path(), &after, &["--only", &selection]);
            after = next;
            if out.status.code() == Some(0) {
                break;
            }
        }
        unchanged(
            dir.path(),
            &after,
            &["--only", &selection, "--fix"],
            Some(0),
        );
        assert_eq!(String::from_utf8_lossy(&after), fixed);
        if let Some(after) = ruby(dir.path()) {
            assert_eq!(after, result, "{fixed:?}");
        }
    }
}

/// Joint corrections with the default set converge, and the incompatible
/// companion holds the spacing edit.
#[test]
fn composition_with_companions() {
    let dir = project();
    for (source, fixed) in [
        ("x  =  1\np x\n", "x = 1\np x\n"),
        (
            "h = {\n  a:    1,\n  bb: 2\n}\np h\n",
            "h = {\n  a: 1,\n  bb: 2\n}\np h\n",
        ),
    ] {
        let (out, after) = fix(dir.path(), source.as_bytes(), &[]);
        assert!(out.status.code().is_some(), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&after), fixed, "{out:?}");
    }
    // Upstream's autocorrect_incompatible_with of ExtraSpacing only orders
    // RuboCop's rounds and is not declared: with the default-on
    // Style/SelfAssignment selected both edits apply, and the result keeps
    // its value.
    let selection = format!("{EXTRA},Style/SelfAssignment");
    let source = "x = 1\nx  = x + 1\nyy  = 2\np x, yy\n";
    fs::write(dir.path().join("example.rb"), source).unwrap();
    let before = ruby(dir.path());
    let (out, after) = fix(dir.path(), source.as_bytes(), &["--only", &selection]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&after),
        "x = 1\nx += 1\nyy = 2\np x, yy\n",
        "{out:?}"
    );
    assert_eq!(ruby(dir.path()), before);
    // SpaceBeforeFirstArg holds its edits while the opt-in
    // Style/MethodCallWithArgsParentheses is selected (upstream's declared
    // incompatibility); that cop's own rewrite still applies and converges.
    let selection = format!("{FIRST},Style/MethodCallWithArgsParentheses");
    let source = "def foo(*a) = a\np foo  1\nputs'x'\n";
    fs::write(dir.path().join("example.rb"), source).unwrap();
    let before = ruby(dir.path());
    let mut after = source.as_bytes().to_vec();
    for _ in 0..4 {
        let (out, next) = fix(dir.path(), &after, &["--only", &selection]);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        if next == after {
            break;
        }
        after = next;
    }
    assert_eq!(
        String::from_utf8_lossy(&after),
        // `p`/`puts` are macros to the fixed IgnoreMacros, so `puts'x'`
        // keeps SpaceBeforeFirstArg's held finding.
        "def foo(*a) = a\np foo(1)\nputs'x'\n"
    );
    assert_eq!(ruby(dir.path()), before);
}
