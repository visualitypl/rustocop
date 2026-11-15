//! Actual CLI lifecycle for the opt-in
//! Layout/MultilineArrayLineBreaks, Layout/MultilineHashKeyLineBreaks,
//! Layout/MultilineMethodArgumentLineBreaks and
//! Layout/MultilineMethodParameterLineBreaks (fixed
//! `AllowMultilineFinalElement: false`), alone, jointly with the
//! first-element line breaks, and with the Layout and default selections.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const ARRAY: &str = "Layout/MultilineArrayLineBreaks";
const HASH: &str = "Layout/MultilineHashKeyLineBreaks";
const ARGUMENT: &str = "Layout/MultilineMethodArgumentLineBreaks";
const PARAMETER: &str = "Layout/MultilineMethodParameterLineBreaks";
const ALL: &str = "Layout/MultilineArrayLineBreaks,Layout/MultilineHashKeyLineBreaks,\
                   Layout/MultilineMethodArgumentLineBreaks,\
                   Layout/MultilineMethodParameterLineBreaks";
const FIRST: &str = "Layout/FirstArrayElementLineBreak,Layout/FirstHashElementLineBreak,\
                     Layout/FirstMethodArgumentLineBreak,Layout/FirstMethodParameterLineBreak";

/// (cop, offending source, canonical correction, offense line:col)
const CASES: [(&str, &str, &str, &str); 4] = [
    (ARRAY, "a = [1,\n  2, 3]\n", "a = [1,\n  2, \n3]\n", "2:6"),
    (
        HASH,
        "a = { a: 1,\n  b: 2, c: 3 }\n",
        "a = { a: 1,\n  b: 2, \nc: 3 }\n",
        "2:9",
    ),
    (ARGUMENT, "foo(1,\n  2, 3)\n", "foo(1,\n  2, \n3)\n", "2:6"),
    (
        PARAMETER,
        "def foo(a,\n  b, c)\nend\n",
        "def foo(a,\n  b, \nc)\nend\n",
        "2:6",
    ),
];

fn all() -> String {
    ALL.replace(' ', "")
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

/// Native Ruby 3.4's Prism token stream without layout tokens (newlines,
/// and the blank runs of percent-array separators) and its AST without
/// locations must be equal before and after: an inserted line break changes
/// nothing else. Never executes either side.
fn same_tokens_and_ast(before: &[u8], after: &[u8]) {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("before.rb"), before).unwrap();
    fs::write(dir.path().join("after.rb"), after).unwrap();
    let script = r##"
        require "prism"
        shape = ARGV.map do |path|
          source = File.binread(path)
          parsed = Prism.parse(source, version: "3.4")
          abort("#{path}: #{parsed.errors.map(&:message)}") unless parsed.errors.empty?
          tokens = Prism.lex(source, version: "3.4").value.filter_map do |token, _state|
            next if %i[NEWLINE IGNORED_NEWLINE EOF].include?(token.type)
            # A percent-array separator is any run of blanks and breaks.
            next [token.type] if token.type == :WORDS_SEP
            [token.type, token.value]
          end
          tree = parsed.value.inspect.gsub(/\(\d+,\d+\)-\(\d+,\d+\)/, "")
          [tokens, tree]
        end
        abort("token or AST change") unless shape[0] == shape[1]
    "##;
    let out = Command::new(ruby)
        .current_dir(dir.path())
        .args(["-e", script, "before.rb", "after.rb"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{:?} -> {:?}: {out:?}",
        String::from_utf8_lossy(before),
        String::from_utf8_lossy(after)
    );
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
        assert!(found.autocorrect_incompatible_with().is_empty(), "{cop}");
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
            "AllowMultilineFinalElement = false",
            "EnforcedStyle = \"x\"",
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
        "Layout/MultilineArrayLineBreak",
        "Layout/MultilineElementLineBreaks",
        "Layout/MultilineHashKeysLineBreaks",
        "Layout/MultilineMethodArgumentsLineBreaks",
        "Layout/MultilineBlockParameterLineBreaks",
    ] {
        unchanged(dir.path(), b"x = 1\n", &["--only", name], Some(2));
    }
}

/// Programs whose runtime results (stdout and `-w` warnings), tokens and AST
/// the corrections of all four cops must keep, with their single-run
/// corrections.
const RUNTIME: &[(&str, &str)] = &[
    ("x = [1,\n  2, 3]\np x\n", "x = [1,\n  2, \n3]\np x\n"),
    ("x = 1, 2,\n  3\np x\n", "x = 1, \n2,\n  3\np x\n"),
    (
        "a, b = 1, 2,\n  3\np a, b\n",
        "a, b = 1, \n2,\n  3\np a, b\n",
    ),
    // Percent-array words keep their values across the inserted break.
    ("p %w(a b\n  c d)\n", "p %w(a \nb\n  c \nd)\n"),
    ("p %i[a b\n  c]\n", "p %i[a \nb\n  c]\n"),
    ("p %W[#{1} a\n  b#{2} c]\n", "p %W[#{1} \na\n  b#{2} \nc]\n"),
    (
        "p [?a, -1,\n  :\"b c\", 'd']\n",
        "p [?a, \n-1,\n  :\"b c\", \n'd']\n",
    ),
    ("p [1, 2, \\\n  3]\n", "p [1, \n2, \\\n  3]\n"),
    (
        "h = { a: 1, b: <<~A,\n  t\nA\n  c: 2 }\np h\n",
        "h = { a: 1, \nb: <<~A,\n  t\nA\n  c: 2 }\np h\n",
    ),
    (
        "h = { \"a\nb\": 1,\n  c: 2, **{ d: 3 } }\np h\n",
        "h = { \"a\nb\": 1,\n  c: 2, \n**{ d: 3 } }\np h\n",
    ),
    (
        "x = <<~A\n  #{[1,\n  2, 3].sum}\n    b\nA\np x\n",
        "x = <<~A\n  #{[1,\n  2, \n3].sum}\n    b\nA\np x\n",
    ),
    (
        "def f(*a, **k) = [a, k]\np f(1, 2, a: 1,\n  b: 2)\n",
        "def f(*a, **k) = [a, k]\np f(1, \n2, \na: 1,\n  b: 2)\n",
    ),
    (
        "def f(a, b = 1,\n  c: 2) = [a, b, c]\np f(0)\n",
        "def f(a, \nb = 1,\n  c: 2) = [a, b, c]\np f(0)\n",
    ),
    (
        "def f((a, b), c, *d,\n  e:, **f, &g) = [a, b, c, d, e, f]\np f([1, 2], 3, e: 4)\n",
        "def f((a, b), \nc, \n*d,\n  e:, \n**f, \n&g) = [a, b, c, d, e, f]\np f([1, 2], 3, e: 4)\n",
    ),
    (
        "class C\n  def [](*a) = (@v ||= {})[a] || 0\n  def []=(*a, v)\n    (@v ||= {})[a] = v\n  end\nend\nc = C.new\nc[1,\n  2, 3] += 4\np c[1, 2, 3]\n",
        "class C\n  def [](*a) = (@v ||= {})[a] || 0\n  def []=(*a, v)\n    (@v ||= {})[a] = v\n  end\nend\nc = C.new\nc[1,\n  2, \n3] += 4\np c[1, 2, 3]\n",
    ),
    ("p [1]&.push(2,\n  3, 4)\n", "p [1]&.push(2,\n  3, \n4)\n"),
    (
        "p [3, 1].sort_by(&:-@).push(2,\n  4, &nil)\n",
        "p [3, 1].sort_by(&:-@).push(2,\n  4, \n&nil)\n",
    ),
    ("p [1, [2,\n  3, 4]]\n", "p [1, \n[2,\n  3, \n4]]\n"),
    (
        "f = ->(x) { [x,\n  1, 2] }\np f.(0)\n",
        "f = ->(x) { [x,\n  1, \n2] }\np f.(0)\n",
    ),
    (
        "class A\n  def f(*a) = a\nend\nclass B < A\n  def f(a,\n    b, c) = super(a,\n    b, c)\nend\np B.new.f(1, 2, 3)\n",
        "class A\n  def f(*a) = a\nend\nclass B < A\n  def f(a,\n    b, \nc) = super(a,\n    b, c)\nend\np B.new.f(1, 2, 3)\n",
    ),
];

#[test]
fn combined_corrections_keep_tokens_ast_values_and_warnings() {
    let dir = project();
    let all = all();
    for (source, after) in RUNTIME {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", &all]);
        assert_eq!(out.status.code(), Some(0), "{source:?} {out:?}");
        assert_eq!(String::from_utf8_lossy(&fixed), *after, "{source:?}");
        same_tokens_and_ast(source.as_bytes(), &fixed);
        unchanged(dir.path(), &fixed, &["--only", &all, "--fix"], Some(0));
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
    // followed by trailing-whitespace removal and alignment of the rest.
    let selection = format!("{},Layout", all());
    for (source, result) in [
        (
            "a = [1,\n     2, 3]\np a\n",
            "a = [1,\n     2,\n     3]\np a\n",
        ),
        (
            "a = { a: 1,\n      b: 2, c: 3 }\np a\n",
            "a = { a: 1,\n      b: 2,\n      c: 3 }\np a\n",
        ),
        (
            "def foo(a,\n        b, c)\n  a + b + c\nend\np foo(1,\n      2, 3)\n",
            "def foo(a,\n        b,\n        c)\n  a + b + c\nend\np foo(1,\n      2,\n      3)\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let mut after = source.as_bytes().to_vec();
        for _ in 0..5 {
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
    // Enabled in the configuration with the first-element family and the
    // whole default selection (whose brace layout cops
    // then move the closing brackets and parentheses to their own
    // lines).
    fs::write(
        dir.path().join("rustocop.toml"),
        all()
            .split(',')
            .chain(FIRST.replace(' ', "").split(','))
            .map(|cop| format!("[cops.\"{cop}\"]\nenabled = true\n"))
            .collect::<String>(),
    )
    .unwrap();
    for (source, result) in [
        (
            "x = [1, 2,\n  3]\np x\n",
            "x = [\n  1,\n  2,\n  3\n]\np x\n",
        ),
        (
            "h = { a: 1, b: [2,\n    3, 4] }\np h\n",
            "h = {\n  a: 1,\n  b: [\n    2,\n    3,\n    4\n  ]\n}\np h\n",
        ),
        (
            "def foo(abc, bcd,\n  cde)\n  abc + bcd + cde\nend\np foo(1, 2,\n  3)\n",
            "def foo(\n  abc,\n  bcd,\n  cde\n)\n  abc + bcd + cde\nend\np foo(\n  1,\n  2,\n  3\n)\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let mut after = source.as_bytes().to_vec();
        for _ in 0..6 {
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
fn upstream_send_with_literal_method_name_edge_is_vacuous() {
    // Upstream's Style/SendWithLiteralMethodName declares this cop
    // autocorrect-incompatible to defer its own unsafe rewrite; here that
    // cop is detection-only, so both report and only the line break is
    // written, in either selection order.
    let dir = project();
    let source = "obj = []\nobj.public_send(:push, 1,\n  2, 3)\np obj\n";
    for selection in [
        format!("{ARGUMENT},Style/SendWithLiteralMethodName"),
        format!("Style/SendWithLiteralMethodName,{ARGUMENT}"),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", &selection],
            Some(1),
        );
        assert_eq!(count(&out, ARGUMENT), 2, "{out:?}");
        assert_eq!(count(&out, "Style/SendWithLiteralMethodName"), 1, "{out:?}");
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", &selection]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(
            String::from_utf8_lossy(&fixed),
            "obj = []\nobj.public_send(:push, \n1,\n  2, \n3)\np obj\n"
        );
        assert_eq!(count(&out, "Style/SendWithLiteralMethodName"), 1, "{out:?}");
    }
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
