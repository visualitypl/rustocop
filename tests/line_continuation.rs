//! Actual CLI lifecycle for the default-on
//! Layout/LineContinuationLeadingSpace, Layout/LineContinuationSpacing and
//! Layout/LineEndStringConcatenationIndentation, alone, together, with
//! Style/StringLiterals and with the Layout and default selections.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const LEADING: &str = "Layout/LineContinuationLeadingSpace";
const SPACING: &str = "Layout/LineContinuationSpacing";
const INDENT: &str = "Layout/LineEndStringConcatenationIndentation";

/// (cop, offending source, canonical correction, line:col)
const CASES: [(&str, &str, &str, &str); 3] = [
    (
        LEADING,
        "x = \"a\" \\\n    \" b\"\np x\n",
        "x = \"a \" \\\n    \"b\"\np x\n",
        "2:6",
    ),
    (
        SPACING,
        "x = 1\\\n  + 2\np x\n",
        "x = 1 \\\n  + 2\np x\n",
        "1:6",
    ),
    (
        INDENT,
        "x = \"a\" \\\n  \"b\"\np x\n",
        "x = \"a\" \\\n    \"b\"\np x\n",
        "2:3",
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

/// `example.rb`'s output and `-w` warnings under native Ruby
/// 3.4, when available. Only these authored programs are executed.
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

/// Native Ruby 3.4's Prism view of both sides (never executed): the token
/// stream without layout tokens, the AST without locations and the comments
/// must be equal, except that `values_only` compares string tokens and
/// string nodes by their concatenated unescaped value (the leading-space move
/// changes which literal holds the blanks, not the value).
fn same_shape(before: &[u8], after: &[u8], values_only: bool) {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("before.rb"), before).unwrap();
    fs::write(dir.path().join("after.rb"), after).unwrap();
    let script = r##"
        require "prism"
        layout = %i[NEWLINE IGNORED_NEWLINE EOF COMMENT EMBDOC_BEGIN EMBDOC_LINE EMBDOC_END]
        values_only = ARGV.shift == "values"
        strings = %i[STRING_BEGIN STRING_CONTENT STRING_END]
        shape = ARGV.map do |path|
          source = File.binread(path)
          parsed = Prism.parse(source, version: "3.4")
          abort("#{path}: #{parsed.errors.map(&:message)}") unless parsed.errors.empty?
          tokens = Prism.lex(source, version: "3.4").value.filter_map do |token, _state|
            next if layout.include?(token.type)
            next if values_only && strings.include?(token.type)
            [token.type, token.value]
          end
          tree = parsed.value.inspect.gsub(/\(\d+,\d+\)-\(\d+,\d+\)/, "")
          if values_only
            values = []
            walk = ->(node) do
              values << node.unescaped if node.is_a?(Prism::StringNode)
              node.compact_child_nodes.each { walk.(_1) }
            end
            walk.(parsed.value)
            tree = values.join
          end
          [tokens, tree, parsed.comments.map { |c| c.location.slice }.sort]
        end
        abort("token, AST or comment change") unless shape[0] == shape[1]
    "##;
    let mode = if values_only { "values" } else { "exact" };
    let out = Command::new(ruby)
        .current_dir(dir.path())
        .args(["-e", script, mode, "before.rb", "after.rb"])
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
fn default_on_metadata_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    for cop in [LEADING, SPACING, INDENT] {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(found.default_enabled(), "{cop}");
        assert!(found.department_selection_enables(), "{cop}");
        assert!(found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        assert!(found.autocorrect_incompatible_with().is_empty(), "{cop}");
    }
    let dir = project();
    for (cop, before, _, position) in CASES {
        for (config, selectors, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Layout"], 1),
            (String::new(), vec!["--only", "Style"], 0),
            (String::new(), vec!["--except", cop], 0),
            (String::new(), vec!["--except", "Layout"], 0),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.\"{cop}\"]\nexclude = [\"example.rb\"]\n"),
                vec![],
                0,
            ),
            (
                format!("[cops.\"{cop}\"]\nexclude = [\"example.rb\"]\n"),
                vec!["--only", cop],
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
        for key in [
            "EnforcedStyle = \"leading\"",
            "IndentationWidth = 1",
            "SupportedStyles = []",
            "max = 1",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(dir.path(), before.as_bytes(), &["--fix"], Some(2));
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for name in [
        "Layout/LineContinuationLeadingSpaces",
        "Layout/LineContinuation",
        "Style/LineContinuationSpacing",
        "Layout/LineEndStringConcatenation",
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
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&fixed), after);
        same_shape(before.as_bytes(), &fixed, cop == LEADING);
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", cop, "--fix"],
            Some(0),
        );
        let (_, fixed) = fix(
            dir.path(),
            format!("\u{feff}{before}").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(fixed, format!("\u{feff}{after}").as_bytes());
        // CRLF: upstream's raw-line patterns need `\` right before `\n`;
        // the alignment cop works on columns and keeps the line endings.
        let crlf = before.replace('\n', "\r\n");
        if cop == INDENT {
            let (_, fixed) = fix(dir.path(), crlf.as_bytes(), &["--only", cop]);
            assert_eq!(String::from_utf8_lossy(&fixed), after.replace('\n', "\r\n"));
        } else {
            let out = unchanged(
                dir.path(),
                crlf.as_bytes(),
                &["--only", cop, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out, cop), 0, "{out:?}");
        }
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
        // Out-of-scope readers do not hold the edit.
        let (out, fixed) = fix(
            dir.path(),
            format!("{before}p File.read('example.rb')\n").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(
            fixed,
            format!("{after}p File.read('example.rb')\n").as_bytes()
        );
        // No edit changes a line count: `__LINE__` keeps its value.
        let (out, fixed) = fix(
            dir.path(),
            format!("{before}p __LINE__\n").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, format!("{after}p __LINE__\n").as_bytes());
    }
}

#[test]
fn uncertain_edits_keep_the_finding_and_the_bytes() {
    let dir = project();
    for (cop, source) in [
        // A missing blank after a sign changes what the lexer reads.
        (SPACING, "p 1 -\\\n1\n"),
        (SPACING, "x = :\"a  \\\nb\"\n"),
        // Upstream's quote pattern fails after another delimiter.
        (LEADING, "x = %q(a) \\\n  \" b\"\n"),
        (LEADING, "x = 'a' \\\n  '  \n  b'\n"),
        // A part that does not begin its line.
        (INDENT, "x = 'a' \\\n    'b'   'c'\n"),
        // Inside a heredoc interpolation.
        (INDENT, "x = <<~A\n  #{'a' \\\n  'b'}\nA\n"),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix"],
            Some(1),
        );
        assert_eq!(count(&out, cop), 1, "{source:?} {out:?}");
    }
    // Legacy bytes between two edits of one run are never copied lossily:
    // the run keeps its finding and its bytes.
    let latin = b"# encoding: iso-8859-1\nx = 'a' \\\n'\xe9' \\\n'b'\n";
    let out = unchanged(dir.path(), latin, &["--only", INDENT, "--fix"], Some(1));
    assert_eq!(count(&out, INDENT), 1, "{out:?}");
    // One-line edits in a legacy-encoded file keep every other byte.
    for (cop, before, after) in [
        (
            INDENT,
            &b"# encoding: iso-8859-1\nx = '\xe9' \\\n'b'\n"[..],
            &b"# encoding: iso-8859-1\nx = '\xe9' \\\n    'b'\n"[..],
        ),
        (
            LEADING,
            b"# encoding: iso-8859-1\nx = '\xe9' \\\n    ' b'\n",
            b"# encoding: iso-8859-1\nx = '\xe9 ' \\\n    'b'\n",
        ),
        (
            SPACING,
            b"# encoding: iso-8859-1\nx = '\xe9'\\\n  'b'\n",
            b"# encoding: iso-8859-1\nx = '\xe9' \\\n  'b'\n",
        ),
    ] {
        let (out, fixed) = fix(dir.path(), before, &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, after, "{cop}");
    }
    // A legacy double-byte character can end in the `\` byte: it is not a
    // backslash, as in upstream's character-based match.
    let sjis = b"# encoding: shift_jis\nx = :\x95\\\n";
    let out = unchanged(dir.path(), sjis, &["--only", SPACING, "--fix"], Some(0));
    assert_eq!(count(&out, SPACING), 0, "{out:?}");
    // A real backslash after such a character is reported and corrected.
    let sjis = b"# encoding: shift_jis\nx = %w[\x95\\]\\\n  .size\n";
    let (out, fixed) = fix(dir.path(), sjis, &["--only", SPACING]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fixed,
        b"# encoding: shift_jis\nx = %w[\x95\\] \\\n  .size\n"
    );
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for (cop, before, _, _) in CASES {
            let block = format!("# {prefix}:disable {cop}\n{before}# {prefix}:enable {cop}\n");
            let out = unchanged(
                dir.path(),
                block.as_bytes(),
                &["--only", cop, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out, cop), 0, "{out:?}");
            let department = format!("# {prefix}:disable Layout\n{before}");
            let out = unchanged(
                dir.path(),
                department.as_bytes(),
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
        let broken = format!("{before}foo(\n");
        let out = unchanged(
            dir.path(),
            broken.as_bytes(),
            &["--only", cop, "--fix"],
            Some(2),
        );
        assert_eq!(count(&out, cop), 0, "{out:?}");
    }
}

/// Programs whose runtime results (stdout and `-w` warnings) each cop's
/// correction must keep.
const RUNTIME: &[(&str, &str, &str)] = &[
    (
        LEADING,
        "x = 'a' \\\n  \" \\t#{1}\" \\\n  '  c'\np x, x.encoding\n",
        "x = 'a ' \\\n  \"\\t#{1}  \" \\\n  'c'\np x, x.encoding\n",
    ),
    (
        LEADING,
        "def f\n  \"a#{2}\" \\\n    \"\\t b\" \\\n    ' c'\nend\np f, f.frozen?\n",
        "def f\n  \"a#{2}\" \\\n    \"\\t b \" \\\n    'c'\nend\np f, f.frozen?\n",
    ),
    (
        SPACING,
        "x = [1,\\\n  2].sum\\\n  .succ\np x\nputs 'a\\\nb'\n",
        "x = [1, \\\n  2].sum \\\n  .succ\np x\nputs 'a\\\nb'\n",
    ),
    (
        SPACING,
        "def f(a)= a\np f 1  \\\n  -1\np(1 ==\\\n  1)\n",
        "def f(a)= a\np f 1 \\\n  -1\np(1 == \\\n  1)\n",
    ),
    (
        INDENT,
        "p 'a' \\\n  \"b#{1}\" \\\n'c'\nh = { k: 'x' \\\n  'y' }\np h\n",
        "p 'a' \\\n  \"b#{1}\" \\\n  'c'\nh = { k: 'x' \\\n         'y' }\np h\n",
    ),
    (
        INDENT,
        "def f\n  'a' \\\n  'b'\nend\np f\n",
        "def f\n  'a' \\\n    'b'\nend\np f\n",
    ),
];

#[test]
fn corrections_keep_tokens_ast_comments_values_and_warnings() {
    let dir = project();
    for (cop, source, after) in RUNTIME {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{source:?} {out:?}");
        assert_eq!(String::from_utf8_lossy(&fixed), *after, "{source:?}");
        same_shape(source.as_bytes(), &fixed, *cop == LEADING);
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &fixed).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{after:?}");
        }
    }
}

#[test]
fn string_literals_ordering_edge_composes_by_overlap_selection() {
    // Upstream's autocorrect_incompatible_with keeps Style/StringLiterals
    // from re-quoting a part from the original source in the same round as
    // the move. Here the two edits of the same parts overlap, one pass
    // applies one of them and the next pass the other.
    let dir = project();
    let selection = format!("{LEADING},Style/StringLiterals");
    let source = "x = 'a' \\\n  ' b'\np x\n";
    fs::write(dir.path().join("example.rb"), source).unwrap();
    let before = ruby(dir.path());
    let (out, first) = fix(dir.path(), source.as_bytes(), &["--only", &selection]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&first),
        "x = \"a\" \\\n  \" b\"\np x\n"
    );
    let (out, second) = fix(dir.path(), &first, &["--only", &selection]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&second),
        "x = \"a \" \\\n  \"b\"\np x\n"
    );
    unchanged(
        dir.path(),
        &second,
        &["--only", &selection, "--fix"],
        Some(0),
    );
    if let Some(before) = before {
        fs::write(dir.path().join("example.rb"), &second).unwrap();
        assert_eq!(ruby(dir.path()), Some(before));
    }
}

#[test]
fn layout_and_default_selections_converge_without_oscillation() {
    let dir = project();
    let source = "x = 'a'\\\n  '  b' \\\n'c'\np x\ny = 1  \\\n  + 2\np y\n";
    for selection in [
        vec![
            "--only",
            "Layout/LineContinuationLeadingSpace,Layout/LineContinuationSpacing,Layout/LineEndStringConcatenationIndentation",
        ],
        vec!["--only", "Layout"],
        vec![],
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let mut after = source.as_bytes().to_vec();
        for _ in 0..6 {
            let (out, next) = fix(dir.path(), &after, &selection);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            after = next;
        }
        let mut args = selection.clone();
        args.push("--fix");
        let out = unchanged(dir.path(), &after, &args, None);
        for cop in [LEADING, SPACING, INDENT] {
            assert_eq!(count(&out, cop), 0, "{selection:?} {out:?}");
        }
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &after).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{selection:?}");
        }
    }
}

#[test]
fn many_continuations_and_parts_stay_linear() {
    let dir = project();
    // 4,000 offending continuations among 4,000 ignored literals.
    let source = "x = 1\\\n  + 2\ny = 'a  \\\nb'\n".repeat(4000);
    let out = unchanged(dir.path(), source.as_bytes(), &["--only", SPACING], Some(1));
    assert_eq!(count(&out, SPACING), 4000);
    let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", SPACING]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fixed, source.replace("1\\\n", "1 \\\n").as_bytes());
    // One concatenation of 3,000 parts: one misaligned run, 2,999 moves.
    let mut source = String::from("x = 'a' \\\n");
    for _ in 0..2999 {
        source.push_str("  ' b' \\\n");
    }
    source.push_str("  'c'\n");
    let out = unchanged(dir.path(), source.as_bytes(), &["--only", LEADING], Some(1));
    assert_eq!(count(&out, LEADING), 2999);
    let out = unchanged(dir.path(), source.as_bytes(), &["--only", INDENT], Some(1));
    assert_eq!(count(&out, INDENT), 1);
    let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", INDENT]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fixed, source.replace("\n  '", "\n    '").as_bytes());
}
