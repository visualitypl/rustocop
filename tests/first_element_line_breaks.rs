//! Actual CLI lifecycle for the opt-in
//! Layout/FirstArrayElementLineBreak, Layout/FirstHashElementLineBreak,
//! Layout/FirstMethodArgumentLineBreak and Layout/FirstMethodParameterLineBreak
//! (fixed `AllowMultilineFinalElement: false`, `AllowImplicitArrayLiterals:
//! false`, `AllowedMethods: []`).
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const ARRAY: &str = "Layout/FirstArrayElementLineBreak";
const HASH: &str = "Layout/FirstHashElementLineBreak";
const ARGUMENT: &str = "Layout/FirstMethodArgumentLineBreak";
const PARAMETER: &str = "Layout/FirstMethodParameterLineBreak";
const ALL: &str = "Layout/FirstArrayElementLineBreak,Layout/FirstHashElementLineBreak,\
                   Layout/FirstMethodArgumentLineBreak,Layout/FirstMethodParameterLineBreak";

/// (cop, offending source, canonical correction, offense line:col)
const CASES: [(&str, &str, &str, &str); 4] = [
    (
        ARRAY,
        "a = [:a,\n     :b]\n",
        "a = [\n:a,\n     :b]\n",
        "1:6",
    ),
    (
        HASH,
        "a = { a: 1,\n      b: 2 }\n",
        "a = { \na: 1,\n      b: 2 }\n",
        "1:7",
    ),
    (
        ARGUMENT,
        "foo(bar,\n  baz)\n",
        "foo(\nbar,\n  baz)\n",
        "1:5",
    ),
    (
        PARAMETER,
        "def foo(bar,\n  baz)\nend\n",
        "def foo(\nbar,\n  baz)\nend\n",
        "1:9",
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

/// `example.rb`'s output under native Ruby 3.4, when available.
fn ruby(root: &Path) -> Option<String> {
    let ruby = crate::support::runtime::ruby().ok()?;
    let run = Command::new(ruby)
        .current_dir(root)
        .args(["--parser=prism", "-w", "example.rb"])
        .output()
        .unwrap();
    assert!(run.status.success(), "{run:?}");
    Some(format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    ))
}

#[test]
fn metadata_defaults_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    for (cop, before, _, position) in CASES {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(!found.default_enabled(), "{cop}");
        assert!(!found.department_selection_enables(), "{cop}");
        assert!(found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        // Upstream's MethodCallWithArgsParentheses incompatibility is kept
        // (that cop is opt-in); MethodDefParentheses' is not declared.
        let incompatible: &[&str] = if cop == ARGUMENT {
            &["Style/MethodCallWithArgsParentheses"]
        } else {
            &[]
        };
        assert_eq!(found.autocorrect_incompatible_with(), incompatible);
        let dir = project();
        for (config, selectors, n) in [
            (String::new(), vec![], 0),
            (String::new(), vec!["--only", cop], 1),
            // A department selection runs an opt-in cop only when the
            // configuration enables it, as upstream.
            (String::new(), vec!["--only", "Layout"], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled = true\n"),
                vec!["--only", "Layout"],
                1,
            ),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", "Layout"], 0),
            (format!("[cops.\"{cop}\"]\nenabled = true\n"), vec![], 1),
            (
                format!("[cops.\"{cop}\"]\nenabled = true\n"),
                vec!["--except", cop],
                0,
            ),
            ("[cops.Layout]\nenabled = true\n".into(), vec![], 0),
            (
                format!("[cops.Layout]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(dir.path(), before.as_bytes(), &selectors, None);
            assert_eq!(count(&out, cop), n, "{cop} {selectors:?} {out:?}");
            if n == 1 {
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(
                    text.contains(&format!("example.rb:{position}: C: ")),
                    "{text}"
                );
            }
        }
        // No public option knobs exist.
        for key in [
            "AllowMultilineFinalElement = true",
            "AllowImplicitArrayLiterals = true",
            "AllowedMethods = [\"foo\"]",
            "EnforcedStyle = \"compact\"",
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
    }
    // Unknown neighbouring names are rejected, never admitted.
    let dir = project();
    for name in [
        "Layout/FirstArrayElementLineBreaks",
        "Layout/FirstElementLineBreak",
        "Layout/FirstMethodArgumentLineBreaks",
        "Layout/FirstMethodParametersLineBreak",
    ] {
        unchanged(dir.path(), b"x = 1\n", &["--only", name], Some(2));
    }
}

/// Programs whose runtime results (stdout and `-w` warnings) the
/// corrections must keep, with their single-run corrections.
const RUNTIME: &[(&str, &str)] = &[
    ("x = [1,\n  2]\np x\n", "x = [\n1,\n  2]\np x\n"),
    ("x = 1,\n  2\np x\n", "x = \n1,\n  2\np x\n"),
    ("a, b = 1,\n  2\np a, b\n", "a, b = \n1,\n  2\np a, b\n"),
    (
        "o = Struct.new(:c).new\no.c = 1,\n  2\np o.c\n",
        "o = Struct.new(:c).new\no.c = \n1,\n  2\np o.c\n",
    ),
    ("x = *[1],\n  2\np x\n", "x = \n*[1],\n  2\np x\n"),
    ("p %w(a b\n  c)\n", "p %w(\na b\n  c)\n"),
    (
        "p %W[#{[1,\n  2].join} b\n  c]\n",
        "p %W[\n#{[\n1,\n  2].join} b\n  c]\n",
    ),
    ("p [?a,\n  -1, :\"b c\"]\n", "p [\n?a,\n  -1, :\"b c\"]\n"),
    ("p [1, \\\n  2]\n", "p [\n1, \\\n  2]\n"),
    (
        "x = <<~A\n  #{[1,\n  2].sum}\n    b\nA\np x\n",
        "x = <<~A\n  #{[\n1,\n  2].sum}\n    b\nA\np x\n",
    ),
    (
        "p({ a: 1,\n  b: 2 }, [3,\n  4])\n",
        "p(\n{ \na: 1,\n  b: 2 }, [\n3,\n  4])\n",
    ),
    (
        "h = { a: [1,\n  2], **{ c: 1,\n  d: 2 } }\np h\n",
        "h = { \na: [\n1,\n  2], **{ \nc: 1,\n  d: 2 } }\np h\n",
    ),
    (
        "x = { \"a\nb\": 1,\n  c: 2 }\np x\n",
        "x = { \n\"a\nb\": 1,\n  c: 2 }\np x\n",
    ),
    (
        "p({a: <<~A, b: 1,\n  t\nA\n  c: 2})\n",
        "p(\n{\na: <<~A, b: 1,\n  t\nA\n  c: 2})\n",
    ),
    ("p [[1,\n  2]]\n", "p [\n[\n1,\n  2]]\n"),
    ("p(/#{[1,\n  2].join}/)\n", "p(\n/#{[\n1,\n  2].join}/)\n"),
    (
        "f = ->(x) { [x,\n  1] }\np f.(2)\n",
        "f = ->(x) { [\nx,\n  1] }\np f.(2)\n",
    ),
    ("def f = [1,\n  2]\np f\n", "def f = [\n1,\n  2]\np f\n"),
    (
        "def f(a, b = [1,\n  2], *c, d:, e: 3, **f, &g) = [a, b, c, d, e, f]\np f(1,\n  d: 2, z: 4)\n",
        "def f(\na, b = [\n1,\n  2], *c, d:, e: 3, **f, &g) = [a, b, c, d, e, f]\np f(\n1,\n  d: 2, z: 4)\n",
    ),
    (
        "def f((a, b),\n  c)\n  [a, b, c]\nend\np(f([1, 2],\n  3))\n",
        "def f(\n(a, b),\n  c)\n  [a, b, c]\nend\np(\nf(\n[1, 2],\n  3))\n",
    ),
    (
        "def f(a = <<~A, b = 1,\n  body\nA\n  c: 2) = [a, b, c]\np f\n",
        "def f(\na = <<~A, b = 1,\n  body\nA\n  c: 2) = [a, b, c]\np f\n",
    ),
    (
        "class A\n  def f(*a) = a\nend\nclass B < A\n  def f(a,\n    b) = super(a,\n    b)\nend\np B.new&.f(1,\n  2)\n",
        "class A\n  def f(*a) = a\nend\nclass B < A\n  def f(\na,\n    b) = super(\na,\n    b)\nend\np B.new&.f(\n1,\n  2)\n",
    ),
];

#[test]
fn combined_corrections_keep_syntax_values_and_warnings() {
    let dir = project();
    let both = ALL.replace(' ', "");
    for (source, after) in RUNTIME {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", &both]);
        assert_eq!(out.status.code(), Some(0), "{source:?} {out:?}");
        assert_eq!(String::from_utf8_lossy(&fixed), *after, "{source:?}");
        unchanged(dir.path(), &fixed, &["--only", &both, "--fix"], Some(0));
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &fixed).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{after:?}");
        }
    }
}

#[test]
fn layout_and_default_selections_converge_without_oscillation() {
    let dir = project();
    // Department selections do not run the opt-in cops.
    for (cop, before, _, _) in CASES {
        let out = unchanged(dir.path(), before.as_bytes(), &["--only", "Layout"], None);
        assert_eq!(count(&out, cop), 0, "{out:?}");
    }
    // With them and every Layout cop selected, the inserted break is
    // followed by the first-element indentation and alignment of the rest,
    // and the default-on brace layout cops move the
    // closing brackets and parentheses of the now open-line lists to their
    // own lines.
    let selection = format!("{},Layout", ALL.replace(' ', ""));
    for (source, result) in [
        ("a = [:a,\n     :b]\np a\n", "a = [\n  :a,\n  :b\n]\np a\n"),
        (
            "a = { a: 1,\n      b: 2 }\np a\n",
            "a = {\n  a: 1,\n  b: 2\n}\np a\n",
        ),
        (
            "x = [[1,\n  2],\n 3]\np x\n",
            "x = [\n  [\n    1,\n    2\n  ],\n  3\n]\np x\n",
        ),
        (
            "def foo(a,\n  b)\n  a + b\nend\np foo(1,\n  2)\n",
            "def foo(\n  a,\n  b\n)\n  a + b\nend\np foo(\n  1,\n  2\n)\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let mut after = source.as_bytes().to_vec();
        // Six runs: the nested array's closing bracket moves one
        // run before it is indented.
        for _ in 0..6 {
            let (out, next) = fix(dir.path(), &after, &["--only", &selection]);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            after = next;
        }
        assert_eq!(String::from_utf8_lossy(&after), result);
        unchanged(
            dir.path(),
            &after,
            &["--only", &selection, "--fix"],
            Some(0),
        );
        assert_eq!(ruby(dir.path()), before);
    }
    // Enabled in the configuration, with the whole default selection.
    fs::write(
        dir.path().join("rustocop.toml"),
        [ARRAY, HASH, ARGUMENT, PARAMETER]
            .map(|cop| format!("[cops.\"{cop}\"]\nenabled = true\n"))
            .concat(),
    )
    .unwrap();
    for (source, result) in [
        ("x = 1,\n  2\np x\n", "x =\n  1,\n  2\np x\n"),
        (
            "h = { a: 1,\n  b: [2,\n    3] }\np h\n",
            "h = {\n  a: 1,\n  b: [\n    2,\n    3\n  ]\n}\np h\n",
        ),
        (
            "def foo(abc,\n  bcd)\n  abc + bcd\nend\np foo(1,\n  2)\n",
            "def foo(\n  abc,\n  bcd\n)\n  abc + bcd\nend\np foo(\n  1,\n  2\n)\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let mut after = source.as_bytes().to_vec();
        // The default-on brace layout cops then move and indent
        // the closing brackets, which takes two more runs.
        for _ in 0..8 {
            let (out, next) = fix(dir.path(), &after, &[]);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            after = next;
        }
        assert_eq!(String::from_utf8_lossy(&after), result);
        unchanged(dir.path(), &after, &["--fix"], Some(0));
        assert_eq!(ruby(dir.path()), before);
    }
}

#[test]
fn compositions_with_the_parentheses_style_cops() {
    let dir = project();
    // The opt-in Style/MethodCallWithArgsParentheses holds the argument
    // edit (upstream's declared incompatibility); the finding stays.
    let selection = format!("{ARGUMENT},Style/MethodCallWithArgsParentheses");
    let source = "def foo(*a) = a\np foo(1,\n  2)\n";
    let out = unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", &selection, "--fix"],
        Some(1),
    );
    assert_eq!(count(&out, ARGUMENT), 1, "{out:?}");
    // The default-on Style/MethodDefParentheses adds the parentheses that
    // the parameter cop then breaks after; both converge.
    let selection = format!("{PARAMETER},Style/MethodDefParentheses");
    let source = "def foo a,\n  b\n  a + b\nend\np foo(1, 2)\n";
    fs::write(dir.path().join("example.rb"), source).unwrap();
    let before = ruby(dir.path());
    let mut after = source.as_bytes().to_vec();
    for _ in 0..3 {
        let (out, next) = fix(dir.path(), &after, &["--only", &selection]);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        after = next;
    }
    assert_eq!(
        String::from_utf8_lossy(&after),
        "def foo(\na,\n  b)\n  a + b\nend\np foo(1, 2)\n"
    );
    unchanged(
        dir.path(),
        &after,
        &["--only", &selection, "--fix"],
        Some(0),
    );
    assert_eq!(ruby(dir.path()), before);
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for (cop, before, _, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for source in [
                format!("# {prefix}:disable {cop}\n{before}"),
                format!("# {prefix}:disable Layout\n{before}"),
                format!("# {prefix}:disable all\n{before}"),
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
fn stdin_reports_positions_and_parse_errors_report_nothing() {
    let dir = project();
    for (cop, before, after, position) in CASES {
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
            text.contains(&format!("virtual.rb:{position}: C: ")),
            "{text}"
        );
        assert!(!dir.path().join("virtual.rb").exists());
        // Supplied paths outside the project are linted and fixed as given.
        let other = tempfile::tempdir().unwrap();
        let path = other.path().join("outside.rb");
        fs::write(&path, before).unwrap();
        let out = run(
            dir.path(),
            &["--only", cop, "--fix", path.to_str().unwrap()],
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        // An invalid file is an error, never a finding or a write.
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
