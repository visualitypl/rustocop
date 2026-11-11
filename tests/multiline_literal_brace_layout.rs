//! Actual CLI lifecycle for the default-on
//! Layout/MultilineArrayBraceLayout and Layout/MultilineHashBraceLayout
//! (fixed `EnforcedStyle: symmetrical`), alone, together, and with the Layout
//! and default selections and their upstream-incompatible peers.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const ARRAY: &str = "Layout/MultilineArrayBraceLayout";
const HASH: &str = "Layout/MultilineHashBraceLayout";
const BOTH: &str = "Layout/MultilineArrayBraceLayout,Layout/MultilineHashBraceLayout";

/// (cop, offending source, canonical correction, offense line:col)
const CASES: [(&str, &str, &str, &str); 4] = [
    (ARRAY, "a = [1,\n  2\n]\n", "a = [1,\n  2]\n", "3:1"),
    (ARRAY, "a = [\n  1,\n  2]\n", "a = [\n  1,\n  2\n]\n", "3:4"),
    (
        HASH,
        "a = { b: 1,\n  c: 2\n}\n",
        "a = { b: 1,\n  c: 2}\n",
        "3:1",
    ),
    (
        HASH,
        "a = {\n  b: 1,\n  c: 2 }\n",
        "a = {\n  b: 1,\n  c: 2 \n}\n",
        "3:8",
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
    let ruby = std::env::var("RUSTOCOP_RUBY34").ok()?;
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
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
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
        assert!(found.autocorrect_incompatible_with().is_empty(), "{cop}");
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
            "EnforcedStyle = \"new_line\"",
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
    // Neighbouring names are rejected, never admitted (the call and
    // definition brace layout cops are separate).
    let dir = project();
    for name in [
        "Layout/MultilineArrayBracketLayout",
        "Layout/MultilineLiteralBraceLayout",
        "Layout/MultilineHashBracesLayout",
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
        // CRLF: a moved bracket keeps every `\r\n`; an inserted break is
        // upstream's bare `\n`.
        let crlf = before.replace('\n', "\r\n");
        let (_, fixed) = fix(dir.path(), crlf.as_bytes(), &["--only", cop]);
        let fixed = String::from_utf8(fixed).unwrap();
        if after.matches('\n').count() < before.matches('\n').count() {
            assert_eq!(fixed, after.replace('\n', "\r\n"), "{cop}");
        } else {
            let mut expected = before.replace('\n', "\r\n");
            let bracket = expected.rfind(['}', ']']).unwrap();
            expected.insert(bracket, '\n');
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
    // Upstream holds and unsafe moves keep the finding without a write.
    for (cop, source) in [
        (ARRAY, "x = [1,\n  2 # c\n].size\n"),
        (HASH, "p({ a: 1,\n  b: 2 # c\n})\n"),
        (ARRAY, "x = [1, [\n  2\n  # c\n  ]\n]\n"),
        (ARRAY, "x = [1,\n  2 \\\n]\n"),
        (HASH, "x = { a: 1,\n  b: 2\n  # c\n}.size\n"),
        (ARRAY, "x = [1, # a\n  2 # b\n] # c\n"),
        (ARRAY, "x = [1, # a\n  2 # b\n], <<~A\n  t\nA\n"),
        (HASH, "x = [{ a: 1, # a\n  b: 2 # b\n}, \"t\nu\"]\n"),
        (ARRAY, "x = [1,\n  <<~A, 2\n  t\nA\n]\n"),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix"],
            None,
        );
        assert_eq!(count(&out, cop), 1, "{source:?} {out:?}");
    }
    // A legacy-encoded literal keeps its bytes.
    let latin = b"# encoding: iso-8859-1\nx = [1,\n  \"\xe9\"\n]\n";
    let (out, fixed) = fix(dir.path(), latin, &["--only", ARRAY]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fixed, b"# encoding: iso-8859-1\nx = [1,\n  \"\xe9\"]\n");
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for (cop, before, _, _) in CASES {
            let line = before.lines().last().unwrap();
            let disabled = before.replacen(
                &format!("{line}\n"),
                &format!("{line} # {prefix}:disable {cop}\n"),
                1,
            );
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
    let broken = b"x = [1,\n  2\n";
    let out = unchanged(dir.path(), broken, &["--only", BOTH, "--fix"], Some(2));
    assert_eq!(count(&out, ARRAY), 0, "{out:?}");
}

/// Programs whose runtime results (stdout and `-w` warnings), tokens, AST and
/// comments both cops' corrections must keep, with their single-run results.
const RUNTIME: &[(&str, &str)] = &[
    ("x = [1,\n  2\n]\np x\n", "x = [1,\n  2]\np x\n"),
    ("x = [\n  1,\n  2]\np x\n", "x = [\n  1,\n  2\n]\np x\n"),
    (
        "x = [1, # a\n  2 # b\n]\np x\n",
        "x = [1, # a\n  2] # b\np x\n",
    ),
    ("x = [1,\n  2,\n]\np x\n", "x = [1,\n  2,]\np x\n"),
    (
        "x = [1,\n  2\n  # c\n]\np x\n",
        "x = [1,\n  2]\n  # c\np x\n",
    ),
    ("p [1,\n  2\n].size\n", "p [1,\n  2].size\n"),
    ("p %w[a\n  b\n]\n", "p %w[a\n  b]\n"),
    ("p %i(a\n  b\n)\n", "p %i(a\n  b)\n"),
    (
        "h = { a: 1, # a\n  b: [2, # b\n    3 # c\n  ], c: 4 }\np h\n",
        "h = { a: 1, # a\n  b: [2, # b\n    3], c: 4 } # c\np h\n",
    ),
    (
        "h = { a: 1,\n  b: <<~A\n    t\n  A\n}\np h\n",
        "h = { a: 1,\n  b: <<~A\n    t\n  A\n}\np h\n",
    ),
    (
        "h = { a: 1,\n  b: { c: <<~A\n    t\n  A\n  }\n}\np h\n",
        "h = { a: 1,\n  b: { c: <<~A\n    t\n  A\n  }}\np h\n",
    ),
    (
        "def f(**k) = k\np f(**{ a: 1,\n  b: 2\n})\n",
        "def f(**k) = k\np f(**{ a: 1,\n  b: 2})\n",
    ),
    (
        "x = \"#{[1,\n  2\n].sum}\"\np x\n",
        "x = \"#{[1,\n  2].sum}\"\np x\n",
    ),
    ("p [1,\n  :caf\u{e9}\n]\n", "p [1,\n  :caf\u{e9}]\n"),
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
    // Upstream orders these pairs through `autocorrect_incompatible_with`.
    // Here each edit is one contiguous replacement starting at the last
    // element's last character, so the nested brace edit (which ends there)
    // and the peers' whole-literal edits always overlap it: one pass keeps
    // one, and repeated runs converge to stable, valid Ruby with the same
    // runtime result.
    let dir = project();
    for (selection, source, once, result) in [
        // A nested hash's comment path next to the array's bracket move.
        (
            BOTH,
            "x = [{ a: 1,\n  b: 2 # c\n}\n]\np x\n",
            "x = [{ a: 1,\n  b: 2} # c\n]\np x\n",
            "x = [{ a: 1,\n  b: 2}] # c\np x\n",
        ),
        (
            BOTH,
            "x = { a: [1,\n  2 # c\n]\n}\np x\n",
            "x = { a: [1,\n  2] # c\n}\np x\n",
            "x = { a: [1,\n  2]} # c\np x\n",
        ),
        (
            BOTH,
            "x = [[1,\n  2\n]\n]\np x\n",
            "x = [[1,\n  2]\n]\np x\n",
            "x = [[1,\n  2]]\np x\n",
        ),
        // Style/RedundantDoubleSplatHashBraces removes `**{` and `}` in one
        // edit over the whole hash; the line-break insertion is dropped.
        (
            "Layout/MultilineHashBraceLayout,Style/RedundantDoubleSplatHashBraces",
            "def f(**k) = k\np f(**{\n  a: 1,\n  b: 2 })\n",
            "def f(**k) = k\np f(\n  a: 1,\n  b: 2)\n",
            "def f(**k) = k\np f(\n  a: 1,\n  b: 2)\n",
        ),
        // It holds its own edit for a closing brace on a later line, and
        // applies it once the brace has moved.
        (
            "Layout/MultilineHashBraceLayout,Style/RedundantDoubleSplatHashBraces",
            "def f(**k) = k\np f(**{ a: 1,\n  b: 2\n})\n",
            "def f(**k) = k\np f(**{ a: 1,\n  b: 2})\n",
            "def f(**k) = k\np f(a: 1,\n  b: 2)\n",
        ),
        // Style/HashAsLastArrayItem wraps a keyword hash element whole.
        (
            "Layout/MultilineArrayBraceLayout,Style/HashAsLastArrayItem",
            "x = [1, a: 1,\n  b: 2\n]\np x\n",
            "x = [1, {a: 1,\n  b: 2}\n]\np x\n",
            "x = [1, {a: 1,\n  b: 2}]\np x\n",
        ),
        // Style/ConcatArrayLiterals' correction is unsafe and never applied.
        (
            "Layout/MultilineArrayBraceLayout,Style/ConcatArrayLiterals",
            "x = []\nx.concat([1,\n  2\n])\np x\n",
            "x = []\nx.concat([1,\n  2])\np x\n",
            "x = []\nx.concat([1,\n  2])\np x\n",
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
            "a = [1,\n     2\n]\np a\n",
            "a = [1,\n     2]\np a\n",
        ),
        (
            vec!["--only", "Layout"],
            "a = {\n  b: 1,\n  c: [2,\n      3]}\np a\n",
            "a = {\n  b: 1,\n  c: [2,\n      3]\n}\np a\n",
        ),
        (
            vec![],
            "x = [{ a: 1,\n       b: 2 # c\n     }\n]\np x\n",
            "x = [{ a: 1,\n       b: 2 }] # c\np x\n",
        ),
        (
            vec![],
            "x = [\n  [1,\n   2\n  ]\n]\np x\n",
            "x = [\n  [1,\n   2]\n]\np x\n",
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
