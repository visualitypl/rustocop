//! Actual CLI lifecycle for the default-on
//! Layout/MultilineBlockLayout and Layout/BlockEndNewline, alone, together,
//! and with the Layout and default selections and MultilineBlockLayout's
//! upstream pass-ordering Style peers.
use crate::support::cli::{count, project, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const MBL: &str = "Layout/MultilineBlockLayout";
const BEN: &str = "Layout/BlockEndNewline";
const BOTH: &str = "Layout/MultilineBlockLayout,Layout/BlockEndNewline";

/// (cop, offending source, canonical correction, CRLF correction, offense
/// line:col)
const CASES: [(&str, &str, &str, &str, &str); 4] = [
    (
        MBL,
        "test do foo\nend\n",
        "test do \n  foo\nend\n",
        "test do \r\n  foo\r\nend\r\n",
        "1:9",
    ),
    (
        MBL,
        "test do |x,\n  y|\n  x\nend\n",
        "test do |x, y|\n  x\nend\n",
        "test do |x, y|\r\n  x\r\nend\r\n",
        "1:9",
    ),
    (
        BEN,
        "test do\n  foo end\n",
        "test do\n  foo\nend\n",
        "test do\r\n  foo\r\nend\r\n",
        "2:7",
    ),
    (
        BEN,
        "test {\n  foo(<<~EOS) }\n  t\nEOS\n",
        "test {\n  foo(<<~EOS)\n  t\nEOS\n}\n",
        "test {\r\n  foo(<<~EOS)\r\n  t\r\nEOS\r\n}\r\n",
        "2:15",
    ),
];

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
    for (cop, before, _, _, position) in CASES {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(found.default_enabled(), "{cop}");
        assert!(found.department_selection_enables(), "{cop}");
        assert!(found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        // Upstream's pass-ordering peers are default-on and not declared.
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
        // No public knobs exist.
        for key in [
            "InspectBlocks = true",
            "EnforcedStyle = \"x\"",
            "AllowedMethods = []",
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
        "Layout/MultilineBlocksLayout",
        "Layout/BlockEndNewLine",
        "Style/BlockEndNewline",
        "Layout/BlockEnd",
    ] {
        unchanged(dir.path(), b"x = 1\n", &["--only", name], Some(2));
    }
}

#[test]
fn line_length_comes_from_the_configured_layout_line_length() {
    // 14 + 1 + 11 + 96 = 122 > 120: the split parameters are necessary.
    let long = "some_result = lambda do |\n  so_many,\n  parameters,\n  it_will,\n  be_too_long,\n  for_one_line,\n  line_length,\n  has_increased,\n  add_3_more|\n  do_something\nend\n";
    let dir = project();
    for (config, n) in [
        ("", 0),
        ("[cops.\"Layout/LineLength\"]\nmax = 122\n", 1),
        ("[cops.\"Layout/LineLength\"]\nmax = 121\n", 1),
        ("[cops.\"Layout/LineLength\"]\nmax = 120\n", 0),
        ("[cops.\"Layout/LineLength\"]\nenabled = false\n", 1),
        // Independent of CLI selection of the length cop.
        (
            "[cops.\"Layout/LineLength\"]\nmax = 200\nenabled = false\n",
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for selectors in [
            vec!["--only", MBL],
            vec!["--only", MBL, "--except", "Layout/LineLength"],
        ] {
            let out = unchanged(dir.path(), long.as_bytes(), &selectors, None);
            assert_eq!(count(&out, MBL), n, "{config:?} {out:?}");
        }
    }
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for (cop, before, _, _, position) in CASES {
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
    for (cop, before, _, _, position) in CASES {
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
    for broken in [
        &b"test do foo\n"[..],
        b"test {\n  foo }\n}\n",
        b"test do |x,\n  y| foo\n",
    ] {
        let out = unchanged(dir.path(), broken, &["--only", BOTH, "--fix"], Some(2));
        assert_eq!(count(&out, MBL) + count(&out, BEN), 0, "{out:?}");
    }
}

/// Programs whose runtime results (stdout and `-w` warnings), tokens, AST and
/// comments both cops' corrections must keep, with their single-run results.
const RUNTIME: &[(&str, &str)] = &[
    (
        "[1, 2].each do |x| p x\nend\n",
        "[1, 2].each do |x| \n  p x\nend\n",
    ),
    (
        "[[1, 2]].each do |(a,\n  b)| p a + b\nend\n",
        "[[1, 2]].each do |(a, b)|\n  p a + b\nend\n",
    ),
    (
        "p([[1, 2]].map { |a,\n  | a })\n",
        "p([[1, 2]].map { |a,|\n    a\n})\n",
    ),
    ("p([1].map {\n  _1 + 1 })\n", "p([1].map {\n  _1 + 1\n})\n"),
    (
        "p([1].map {\n  it * 3 }.sum)\n",
        "p([1].map {\n  it * 3\n}.sum)\n",
    ),
    (
        "[3].each {\n  p(<<~X) }\n  t\nX\n",
        "[3].each {\n  p(<<~X)\n  t\nX\n}\n",
    ),
    (
        "f = ->(x) { x * 2\n}\np f.(1)\n",
        "f = ->(x) { \n      x * 2\n}\np f.(1)\n",
    ),
    (
        "[1].each do |x| raise x.to_s\nrescue => e\n  p e.message\nend\n",
        "[1].each do |x| \n  raise x.to_s\nrescue => e\n  p e.message\nend\n",
    ),
    (
        "[1].each { |x| p x\n  p x + 1 }\n",
        "[1].each { |x| \n  p x\n  p x + 1\n}\n",
    ),
    (
        "def f\n  yield 1, 2\nend\nf do |a, # first\n  b| p [a, b]\nend\n",
        "def f\n  yield 1, 2\nend\nf do |a, # first\n  b| p [a, b]\nend\n",
    ),
    (
        "x = [1].map do |v,\n  w = 2| v + w end\np x\n",
        "x = [1].map do |v, w = 2|\n      v + w\nend\np x\n",
    ),
];

#[test]
fn corrections_keep_tokens_ast_comments_values_and_warnings() {
    let dir = project();
    for (source, after) in RUNTIME {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", BOTH]);
        assert_ne!(out.status.code(), Some(2), "{source:?} {out:?}");
        let mut after_all = fixed;
        for _ in 0..3 {
            let (out, next) = fix(dir.path(), &after_all, &["--only", BOTH]);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            after_all = next;
        }
        assert_eq!(String::from_utf8_lossy(&after_all), *after, "{source:?}");
        same_tokens_ast_and_comments(source.as_bytes(), &after_all);
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &after_all).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{after:?}");
        }
    }
}

#[test]
fn upstream_pass_ordering_peers_compose() {
    // Upstream's `autocorrect_incompatible_with` peers are default-on and not
    // declared: their edits and this cop's compose (or overlap, one per
    // pass) into stable, valid Ruby with the same runtime result.
    let dir = project();
    for (selection, source, result) in [
        // Style/Lambda rewrites the multi-line `->` to `lambda` while the
        // body line break is inserted after `do`.
        (
            "Layout/MultilineBlockLayout,Style/Lambda",
            "f = ->(x) do x\nend\np f.(1)\n",
            "f = lambda do |x| \n      x\nend\np f.(1)\n",
        ),
        // Style/HashEachMethods is unsafe: it never edits.
        (
            "Layout/MultilineBlockLayout,Style/HashEachMethods",
            "h = { a: 1 }\nh.each do |k,\n  _| p k\nend\n",
            "h = { a: 1 }\nh.each do |k, _|\n  p k\nend\n",
        ),
        (
            "Layout/MultilineBlockLayout,Style/HashEachMethods",
            "h = { a: 1 }\nh.each { |k, v| p k\n}\n",
            "h = { a: 1 }\nh.each { |k, v| \n  p k\n}\n",
        ),
        (
            "Layout/MultilineBlockLayout,Style/ItBlockParameter",
            "[1].each { |it| p it\n}\n",
            "[1].each { |it| \n  p it\n}\n",
        ),
        (
            "Layout/MultilineBlockLayout,Style/ItBlockParameter",
            "[1].each { |it,\n  | p it\n}\n",
            "[1].each { |it,|\n  p it\n}\n",
        ),
        // Style/CombinableLoops (whose upstream list names
        // Layout/BlockEndNewline) is unsafe: it never edits.
        (
            "Layout/BlockEndNewline,Style/CombinableLoops",
            "items = [1]\nitems.each { p 1 }\nitems.each { p 2 }\nitems.each do |x|\n  p x end\nitems.each do |x|\n  p x + 1 end\n",
            "items = [1]\nitems.each { p 1 }\nitems.each { p 2 }\nitems.each do |x|\n  p x\nend\nitems.each do |x|\n  p x + 1\nend\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let mut after = source.as_bytes().to_vec();
        for _ in 0..4 {
            let (out, next) = fix(dir.path(), &after, &["--only", selection]);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            after = next;
        }
        assert_eq!(String::from_utf8_lossy(&after), result, "{source:?}");
        let out = unchanged(dir.path(), &after, &["--only", MBL], Some(0));
        assert_eq!(count(&out, MBL), 0, "{out:?}");
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
            "[1, 2].each do |x| p x\nend\n",
            "[1, 2].each do |x|\n  p x\nend\n",
        ),
        (
            vec!["--only", "Layout"],
            "[1].each { |x,\n  y| p x\n  p y }\n",
            "[1].each { |x, y|\n  p x\n  p y\n}\n",
        ),
        (
            vec![],
            "x = [1].map do |v|\n  v + 1 end\np x\n",
            "x = [1].map do |v|\n  v + 1\nend\np x\n",
        ),
        (
            vec![],
            "p([[1, 2]].map { |(a,\n  b)| a + b })\n",
            "p([[1, 2]].map do |(a, b)|\n  a + b\nend)\n",
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

#[test]
fn parameter_bytes_that_are_not_utf8_keep_the_finding_and_the_file() {
    // A BINARY or legacy parameter byte that is not UTF-8 cannot be
    // copied into the canonical join, so the finding stays and the file
    // keeps its bytes, mtime, permissions and inode.
    let dir = project();
    for source in [
        b"# coding: BINARY\ndef test\n yield\nend\np(test do |x = \"\xff\",\n  y|\n  [x,y]\nend)\n"
            .as_slice(),
        b"# coding: BINARY\ndef test\n yield\nend\np(test do |k: \"\xfe\x80\",\n  y: 2|\n  [k, y]\nend)\n",
        b"# coding: BINARY\ndef test\n yield [1, 2]\nend\np(test do |(a, b), c = :\xff,\n  d|\n  [a, b, c, d]\nend)\n",
        b"# coding: BINARY\ndef test\n yield 1, 2\nend\np(test do |a\xff,\n  b|\n  [a\xff, b]\nend)\n",
        b"# encoding: iso-8859-1\ndef test\n yield\nend\np(test do |x = \"\xe9\",\n  y| [x, y]\nend)\n",
    ] {
        for selection in [MBL, BOTH] {
            let out = unchanged(dir.path(), source, &["--only", selection, "--fix"], Some(1));
            assert!(count(&out, MBL) >= 1, "{source:?} {out:?}");
        }
    }
    // ASCII parameters in a BINARY file, and valid UTF-8 bytes in BINARY
    // parameters, keep the canonical correction with their bytes as written.
    for (source, after) in [
        (
            b"# coding: BINARY\ndef test\n yield\nend\np(test do |x = \"a\",\n  y| [x, y, \"\xff\"]\nend)\n"
                .as_slice(),
            b"# coding: BINARY\ndef test\n yield\nend\np(test do |x = \"a\", y|\n    [x, y, \"\xff\"]\nend)\n"
                .as_slice(),
        ),
        (
            b"# coding: BINARY\ndef test\n yield\nend\np(test do |x = \"\xc3\xa9\",\n  y|\n  [x, y]\nend)\n",
            b"# coding: BINARY\ndef test\n yield\nend\np(test do |x = \"\xc3\xa9\", y|\n  [x, y]\nend)\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let (out, fixed) = fix(dir.path(), source, &["--only", MBL]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, after, "{source:?}");
        if let Some(before) = before {
            assert_eq!(ruby(dir.path()), Some(before), "{after:?}");
        }
    }
}
