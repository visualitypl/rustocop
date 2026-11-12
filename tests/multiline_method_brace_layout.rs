//! Actual CLI lifecycle for the default-on
//! Layout/MultilineMethodCallBraceLayout and
//! Layout/MultilineMethodDefinitionBraceLayout (fixed `EnforcedStyle:
//! symmetrical`), alone, together, and with the Layout and default
//! selections, the array/hash brace layout cops and their upstream-
//! incompatible Style peers.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const CALL: &str = "Layout/MultilineMethodCallBraceLayout";
const DEF: &str = "Layout/MultilineMethodDefinitionBraceLayout";
const BOTH: &str =
    "Layout/MultilineMethodCallBraceLayout,Layout/MultilineMethodDefinitionBraceLayout";
const ALL: &str = "Layout/MultilineMethodCallBraceLayout,Layout/MultilineMethodDefinitionBraceLayout,Layout/MultilineArrayBraceLayout,Layout/MultilineHashBraceLayout";

/// (cop, offending source, canonical correction, offense line:col)
const CASES: [(&str, &str, &str, &str); 4] = [
    (CALL, "x = foo(1,\n  2\n)\n", "x = foo(1,\n  2)\n", "3:1"),
    (
        CALL,
        "x = foo(\n  1,\n  2)\n",
        "x = foo(\n  1,\n  2\n)\n",
        "3:4",
    ),
    (
        DEF,
        "def f(a,\n  b\n)\nend\n",
        "def f(a,\n  b)\nend\n",
        "3:1",
    ),
    (
        DEF,
        "def f(\n  a,\n  b)\nend\n",
        "def f(\n  a,\n  b\n)\nend\n",
        "3:4",
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
    args.extend(["--fix", "--fix-passes", "1", "example.rb"]);
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

/// Native Ruby 3.4's Prism token stream without layout and comment tokens,
/// its AST without locations and its comments (as a multiset of their exact
/// bytes) must be equal before and after: moving a closing bracket changes
/// nothing else. Never executes either side.
fn same_tokens_ast_and_comments(before: &[u8], after: &[u8]) {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("before.rb"), before).unwrap();
    fs::write(dir.path().join("after.rb"), after).unwrap();
    let script = r##"
        require "prism"
        layout = %i[NEWLINE IGNORED_NEWLINE EOF COMMENT EMBDOC_BEGIN EMBDOC_LINE EMBDOC_END]
        shape = ARGV.map do |path|
          source = File.binread(path)
          parsed = Prism.parse(source, version: "3.4")
          abort("#{path}: #{parsed.errors.map(&:message)}") unless parsed.errors.empty?
          tokens = Prism.lex(source, version: "3.4").value.filter_map do |token, _state|
            next if layout.include?(token.type)
            # Percent-array separators are layout; the words stay tokens.
            next if token.type == :WORDS_SEP
            [token.type, token.value]
          end
          tree = parsed.value.inspect.gsub(/\(\d+,\d+\)-\(\d+,\d+\)/, "")
          [tokens, tree, parsed.comments.map { |c| c.location.slice }.sort]
        end
        abort("token, AST or comment change") unless shape[0] == shape[1]
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
        assert!(found.default_enabled(), "{cop}");
        assert!(found.department_selection_enables(), "{cop}");
        assert!(found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        let peer = if cop == CALL {
            "Style/MethodCallWithArgsParentheses"
        } else {
            "Style/MultilineMethodSignature"
        };
        assert_eq!(found.autocorrect_incompatible_with(), [peer], "{cop}");
        let dir = project();
        for (config, selectors, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Layout"], 1),
            (String::new(), vec!["--except", cop], 0),
            (String::new(), vec!["--except", "Layout"], 0),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", "Layout"], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            // `--only` force-enables a configured-off cop.
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
            (
                format!("[cops.Layout]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (
                format!("[cops.\"{cop}\"]\nexclude = [\"example.rb\"]\n"),
                vec![],
                0,
            ),
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
        // No public style knobs exist.
        for key in [
            "EnforcedStyle = \"symmetrical\"",
            "EnforcedStyle = \"same_line\"",
            "SupportedStyles = []",
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
    // Neighbouring names are rejected, never admitted.
    let dir = project();
    for name in [
        "Layout/MultilineMethodCallBracesLayout",
        "Layout/MultilineMethodDefinitionBracketLayout",
        "Layout/MultilineMethodBraceLayout",
        "Layout/MultilineDefinitionBraceLayout",
    ] {
        unchanged(dir.path(), b"x = 1\n", &["--only", name], Some(2));
    }
}

#[test]
fn lint_never_writes_and_fix_writes_the_canonical_edit_once() {
    let dir = project();
    for (cop, before, after, _) in CASES {
        let out = unchanged(dir.path(), before.as_bytes(), &["--only", cop], Some(1));
        assert_eq!(count(&out, cop), 1, "{out:?}");
        let (out, fixed) = fix(dir.path(), before.as_bytes(), &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{cop}: {out:?}");
        assert_eq!(fixed, after.as_bytes(), "{cop}");
        same_tokens_ast_and_comments(before.as_bytes(), &fixed);
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", cop, "--fix"],
            Some(0),
        );
        // A UTF-8 BOM is preserved around the edit.
        let (_, fixed) = fix(
            dir.path(),
            format!("\u{feff}{before}").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(fixed, format!("\u{feff}{after}").as_bytes(), "{cop}");
        // CRLF: a moved parenthesis keeps every `\r\n`; an inserted break is
        // upstream's bare `\n`.
        let crlf = before.replace('\n', "\r\n");
        let (_, fixed) = fix(dir.path(), crlf.as_bytes(), &["--only", cop]);
        let fixed = String::from_utf8(fixed).unwrap();
        if after.matches('\n').count() < before.matches('\n').count() {
            assert_eq!(fixed, after.replace('\n', "\r\n"), "{cop}");
        } else {
            let mut expected = before.replace('\n', "\r\n");
            let parenthesis = expected.rfind(')').unwrap();
            expected.insert(parenthesis, '\n');
            assert_eq!(fixed, expected, "{cop}");
        }
        // Every listed source observer keeps the finding and the file.
        for observer in [
            "p __FILE__\n",
            "p __dir__\n",
            "p $0\n",
            "p $PROGRAM_NAME\n",
            "p caller\n",
            "p caller_locations\n",
            "p method(:p).source_location\n",
            "p SCRIPT_LINES__\n",
        ] {
            let observed = format!("{observer}{before}");
            let out = unchanged(
                dir.path(),
                observed.as_bytes(),
                &["--only", cop, "--fix"],
                Some(1),
            );
            assert_eq!(count(&out, cop), 1, "{out:?}");
        }
        let data = format!("{before}p DATA.read\n__END__\nz\n");
        unchanged(
            dir.path(),
            data.as_bytes(),
            &["--only", cop, "--fix"],
            Some(1),
        );
        // A line-number observer after the edit keeps the file; one before
        // it does not observe the move.
        let lines = format!("{before}p __LINE__\n");
        unchanged(
            dir.path(),
            lines.as_bytes(),
            &["--only", cop, "--fix"],
            Some(1),
        );
        let (out, fixed) = fix(
            dir.path(),
            format!("p __LINE__\n{before}").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, format!("p __LINE__\n{after}").as_bytes());
    }
    // Upstream holds, unsafe moves and upstream's broken heredoc chain
    // moves keep the finding without a write.
    for (cop, source) in [
        (CALL, "x = foo(1,\n  2 # c\n).size\n"),
        (CALL, "p(foo(1,\n  2 # c\n))\n"),
        (CALL, "x = foo(1,\n  2 \\\n)\n"),
        (CALL, "x = foo(1,\n  2\n  # c\n).size\n"),
        (CALL, "x = foo(1, # a\n  2 # b\n) # c\n"),
        (CALL, "x = foo(<<~A, 1\n  t\n  u\nA\n).size\n"),
        (CALL, "x = foo(<<~A, 1\n  t\nA\n) + 1\n"),
        (CALL, "x = foo(<<~A, 1\n  t\nA\n).bar.baz\n"),
        (DEF, "def f(a, b = ->(x # c\n) { x }\n)\nend\n"),
        (DEF, "def f(a, # a\n  b # b\n) # c\nend\n"),
        (DEF, "def f(a = <<~A, b = 1\n  t\nA\n) = a\n"),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix"],
            None,
        );
        assert!(count(&out, cop) >= 1, "{source:?} {out:?}");
    }
    // A legacy-encoded argument keeps its bytes.
    let latin = b"# encoding: iso-8859-1\nx = foo(1,\n  \"\xe9\"\n)\n";
    let (out, fixed) = fix(dir.path(), latin, &["--only", CALL]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fixed, b"# encoding: iso-8859-1\nx = foo(1,\n  \"\xe9\")\n");
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for (cop, before, _, position) in CASES {
            let line: usize = position.split(':').next().unwrap().parse().unwrap();
            let disabled: String = before
                .lines()
                .enumerate()
                .map(|(index, text)| {
                    if index + 1 == line {
                        format!("{text} # {prefix}:disable {cop}\n")
                    } else {
                        format!("{text}\n")
                    }
                })
                .collect();
            let out = unchanged(
                dir.path(),
                disabled.as_bytes(),
                &["--only", cop, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out, cop), 0, "{out:?}");
            let block = format!("# {prefix}:disable {cop}\n{before}# {prefix}:enable {cop}\n");
            let out = unchanged(
                dir.path(),
                block.as_bytes(),
                &["--only", cop, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out, cop), 0, "{out:?}");
        }
    }
}

#[test]
fn stdin_reports_positions_and_parse_errors_report_nothing() {
    let dir = project();
    for (cop, before, _, position) in CASES {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "x.rb"])
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
        assert!(text.contains(&format!("x.rb:{position}: C: ")), "{text}");
    }
    for broken in [&b"x = foo(1,\n  2\n"[..], b"def f(a,\n  b\n)\n"] {
        let out = unchanged(dir.path(), broken, &["--only", BOTH, "--fix"], Some(2));
        assert_eq!(count(&out, CALL) + count(&out, DEF), 0, "{out:?}");
    }
}

/// Programs whose runtime results (stdout and `-w` warnings), tokens, AST and
/// comments both cops' corrections must keep, with their single-run results.
const RUNTIME: &[(&str, &str)] = &[
    (
        "def f(a,\n  b\n)\n  a + b\nend\np f(1,\n  2\n)\n",
        "def f(a,\n  b)\n  a + b\nend\np f(1,\n  2)\n",
    ),
    (
        "def f(\n  a,\n  b) = a * b\np(f(\n  2,\n  3))\n",
        "def f(\n  a,\n  b\n) = a * b\np(f(\n  2,\n  3\n))\n",
    ),
    (
        "def f(a:,\n  b: 2\n) = a + b\np f(a: 1,\n  b: 3\n)\n",
        "def f(a:,\n  b: 2) = a + b\np f(a: 1,\n  b: 3)\n",
    ),
    ("p(1, # a\n  2 # b\n)\n", "p(1, # a\n  2) # b\n"),
    (
        "x = [3, 1].sort_by(\n  &:itself)\np x\n",
        "x = [3, 1].sort_by(\n  &:itself\n)\np x\n",
    ),
    (
        "def f(*a, **k, &b) = [a, k, b&.call]\np f(1,\n  k: 2,\n  &-> { 3 }\n)\n",
        "def f(*a, **k, &b) = [a, k, b&.call]\np f(1,\n  k: 2,\n  &-> { 3 })\n",
    ),
    (
        "x = [1, 2].map(\n  &:to_s).size\np x\n",
        "x = [1, 2].map(\n  &:to_s\n).size\np x\n",
    ),
    (
        "x = [1, 2].each_slice(1,\n) do |s|\n  p s\nend\n",
        "x = [1, 2].each_slice(1,) do |s|\n  p s\nend\n",
    ),
    (
        "def f(s, n) = s * n\nx = f(<<~A, 2\n  t\nA\n).size\np x\n",
        "def f(s, n) = s * n\nx = f(<<~A, 2).size\n  t\nA\np x\n",
    ),
    (
        "def f(s, n) = s * n\nx = f(<<~A, 2\n  t\nA\n)&.size\np x\n",
        "def f(s, n) = s * n\nx = f(<<~A, 2)&.size\n  t\nA\np x\n",
    ),
    (
        "def f(s, n) = s * n\np f(<<~A, 2\n  t\nA\n)\n",
        "def f(s, n) = s * n\np f(<<~A, 2)\n  t\nA\n",
    ),
    (
        "def f(a = <<~A, b = 1\n  t\nA\n)\n  a * b\nend\np f\n",
        "def f(a = <<~A, b = 1)\n  t\nA\n  a * b\nend\np f\n",
    ),
    (
        "x = \"#{[1, 2].sum(\n  0\n)}\"\np x\n",
        "x = \"#{[1, 2].sum(\n  0\n)}\"\np x\n",
    ),
    (
        "x = \"#{[1, 2].sum(0,\n)}\"\np x\n",
        "x = \"#{[1, 2].sum(0,)}\"\np x\n",
    ),
    ("p(1,\n  :caf\u{e9}\n)\n", "p(1,\n  :caf\u{e9})\n"),
];

#[test]
fn corrections_keep_tokens_ast_comments_values_and_warnings() {
    let dir = project();
    for (source, after) in RUNTIME {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", BOTH]);
        assert_ne!(out.status.code(), Some(2), "{source:?} {out:?}");
        assert_eq!(String::from_utf8_lossy(&fixed), *after, "{source:?}");
        same_tokens_ast_and_comments(source.as_bytes(), &fixed);
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &fixed).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{after:?}");
        }
    }
}

#[test]
fn nested_and_incompatible_peer_edits_never_interleave() {
    // Each edit is one contiguous replacement starting at the last child's
    // last character, so a nested brace edit (which ends there) overlaps it:
    // one pass keeps one, and repeated runs converge to stable, valid Ruby
    // with the same runtime result. The opt-in Style peers that upstream
    // declares incompatible hold this family's edits while selected.
    let dir = project();
    for (selection, source, once, result) in [
        (
            ALL,
            "x = [p(1,\n  2\n)\n]\n",
            "x = [p(1,\n  2)\n]\n",
            "x = [p(1,\n  2)]\n",
        ),
        (
            ALL,
            "p(1, [2,\n  3\n]\n)\n",
            "p(1, [2,\n  3]\n)\n",
            "p(1, [2,\n  3])\n",
        ),
        // The hash argument's comment hold (`argument?`) is upstream's; the
        // call's parenthesis still moves behind it.
        (
            ALL,
            "p(1, { a: 2,\n  b: 3 # c\n}\n)\n",
            "p(1, { a: 2,\n  b: 3 # c\n})\n",
            "p(1, { a: 2,\n  b: 3 # c\n})\n",
        ),
        (
            ALL,
            "def f(a, b = [1,\n  2\n]\n) = b\np f(0)\n",
            "def f(a, b = [1,\n  2]\n) = b\np f(0)\n",
            "def f(a, b = [1,\n  2]) = b\np f(0)\n",
        ),
        // The inner call is a `send` argument: its comment move is held, and
        // the outer parenthesis moves behind it (as upstream).
        (
            BOTH,
            "p(p(1,\n  2 # c\n)\n)\n",
            "p(p(1,\n  2 # c\n))\n",
            "p(p(1,\n  2 # c\n))\n",
        ),
        // Through a `csend` the inner edit applies first.
        (
            BOTH,
            "x = [1]\ny = x&.push(p(1,\n  2 # c\n)\n)\np y\n",
            "x = [1]\ny = x&.push(p(1,\n  2) # c\n)\np y\n",
            "x = [1]\ny = x&.push(p(1,\n  2)) # c\np y\n",
        ),
        // Style/MethodCallWithArgsParentheses (opt-in, `require_parentheses`)
        // holds the call edits while selected.
        (
            "Layout/MultilineMethodCallBraceLayout,Style/MethodCallWithArgsParentheses",
            "p(1,\n  2\n)\n",
            "p(1,\n  2\n)\n",
            "p(1,\n  2\n)\n",
        ),
        (
            "Layout/MultilineMethodCallBraceLayout,Style/MethodCallWithArgsParentheses",
            "x = [1].push 2,\n  [3,\n    4]\np x\n",
            "x = [1].push(2,\n  [3,\n    4])\np x\n",
            "x = [1].push(2,\n  [3,\n    4])\np x\n",
        ),
        // Style/MultilineMethodSignature (opt-in) holds the definition edits
        // while selected and collapses the signature itself.
        (
            "Layout/MultilineMethodDefinitionBraceLayout,Style/MultilineMethodSignature",
            "def f(a,\n  b\n)\n  a + b\nend\np f(1, 2)\n",
            "def f(a, b)\n  a + b\nend\np f(1, 2)\n",
            "def f(a, b)\n  a + b\nend\np f(1, 2)\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let (out, first) = fix(dir.path(), source.as_bytes(), &["--only", selection]);
        assert_ne!(out.status.code(), Some(2), "{source:?} {out:?}");
        assert_eq!(String::from_utf8_lossy(&first), once, "{source:?}");
        let mut after = first;
        for _ in 0..3 {
            let (out, next) = fix(dir.path(), &after, &["--only", selection]);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            after = next;
        }
        assert_eq!(String::from_utf8_lossy(&after), result, "{source:?}");
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &after).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{result:?}");
        }
    }
}

#[test]
fn layout_and_default_selections_converge_without_oscillation() {
    let dir = project();
    for (selection, source, result) in [
        (
            vec!["--only", "Layout"],
            "x = p(1,\n      2\n)\n",
            "x = p(1,\n      2)\n",
        ),
        (
            vec!["--only", "Layout"],
            "def f(a,\n      b\n)\n  a + b\nend\np f(1, 2)\n",
            "def f(a,\n      b)\n  a + b\nend\np f(1, 2)\n",
        ),
        (vec![], "p(\n  1,\n  2)\n", "p(\n  1,\n  2\n)\n"),
        (
            vec![],
            "def f(\n  abc,\n  bcd)\n  abc + bcd\nend\np f(1, 2)\n",
            "def f(\n  abc,\n  bcd\n)\n  abc + bcd\nend\np f(1, 2)\n",
        ),
        (
            vec![],
            "x = [p(1,\n       2\n      )\n]\np x\n",
            "x = [p(1,\n       2)]\np x\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let mut after = source.as_bytes().to_vec();
        for _ in 0..6 {
            let (out, next) = fix(dir.path(), &after, &selection);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            after = next;
        }
        assert_eq!(String::from_utf8_lossy(&after), result, "{source:?}");
        let mut args = selection.clone();
        args.push("--fix");
        unchanged(dir.path(), &after, &args, Some(0));
        assert_eq!(ruby(dir.path()), before);
    }
}
