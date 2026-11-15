//! Actual CLI lifecycle for the opt-in Layout/RedundantLineBreak
//! (fixed `InspectBlocks: false`), its configured Layout/LineLength and
//! Layout/SingleLineBlockChain dependencies, and its compositions.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Layout/RedundantLineBreak";
const ENABLE: &str = "[cops.\"Layout/RedundantLineBreak\"]\nenabled = true\n";

/// (offending source, canonical correction, line:col)
const CASES: [(&str, &str, &str); 3] = [
    ("x = foo(1,\n  2)\n", "x = foo(1, 2)\n", "1:1"),
    (
        "x = [1, 2]\n  .map(&:to_s)\n  .size\n",
        "x = [1, 2].map(&:to_s).size\n",
        "1:1",
    ),
    (
        "if x\n  y = 'a' \\\n    'b'\nend\n",
        "if x\n  y = 'ab'\nend\n",
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
fn opt_in_metadata_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    let found = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(!found.default_enabled());
    assert!(!found.department_selection_enables());
    assert!(found.safe_fixes());
    assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
    assert!(found.path_policy().is_none() && found.max_setting().is_none());
    // Upstream's pass-ordering peer (Style/CombinableLoops lists this cop)
    // is not declared, and this cop declares none.
    assert!(found.autocorrect_incompatible_with().is_empty());
    let dir = project();
    for (before, _, position) in CASES {
        for (config, selectors, n) in [
            (String::new(), vec![], 0),
            (String::new(), vec!["--only", COP], 1),
            (String::new(), vec!["--only", "Layout"], 0),
            ("[cops.Layout]\nenabled = true\n".into(), vec![], 0),
            (String::new(), vec!["--only", COP, "--except", COP], 0),
            (String::new(), vec!["--only", COP, "--except", "Layout"], 0),
            (ENABLE.into(), vec![], 1),
            (ENABLE.into(), vec!["--only", "Layout"], 1),
            (ENABLE.into(), vec!["--except", COP], 0),
            (
                format!("[cops.Layout]\nenabled = false\n{ENABLE}"),
                vec![],
                1,
            ),
            (format!("{ENABLE}exclude = [\"example.rb\"]\n"), vec![], 0),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(dir.path(), before.as_bytes(), &selectors, None);
            assert_eq!(count(&out, COP), n, "{selectors:?} {out:?}");
            if n == 1 {
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(
                    text.contains(&format!("example.rb:{position}: C: ")),
                    "{text}"
                );
            }
        }
    }
    // No public knobs: InspectBlocks is fixed false.
    for key in ["InspectBlocks = true", "InspectBlocks = false", "max = 80"] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            CASES[0].0.as_bytes(),
            &["--only", COP, "--fix"],
            Some(2),
        );
    }
    for name in [
        "Layout/RedundantLineBreaks",
        "Layout/RedundantLinebreak",
        "Style/RedundantLineBreak",
    ] {
        unchanged(dir.path(), b"x = 1\n", &["--only", name], Some(2));
    }
}

#[test]
fn configured_line_length_and_single_line_block_chain_decide_detection() {
    let dir = project();
    let long = format!("x = foo(1,\n  {})\n", "y".repeat(100));
    let chained = "e.select { |i| i.cond? }\n  .join\n";
    for (config, long_n, chained_n) in [
        ("", 1, 1),
        ("[cops.\"Layout/LineLength\"]\nmax = 100\n", 0, 1),
        (
            "[cops.\"Layout/LineLength\"]\nmax = 100\nenabled = false\n",
            1,
            1,
        ),
        ("[cops.Layout]\nenabled = false\n", 1, 1),
        (
            "[cops.\"Layout/SingleLineBlockChain\"]\nenabled = true\n",
            1,
            0,
        ),
        // Configured, not selected: --except does not change the deferral.
        (
            "[cops.\"Layout/SingleLineBlockChain\"]\nenabled = false\n",
            1,
            1,
        ),
        ("[cops.Layout]\nenabled = true\n", 1, 1),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for selectors in [
            vec!["--only", COP],
            vec![
                "--only",
                COP,
                "--except",
                "Layout/SingleLineBlockChain,Layout/LineLength",
            ],
        ] {
            let out = unchanged(dir.path(), long.as_bytes(), &selectors, None);
            assert_eq!(count(&out, COP), long_n, "{config:?} {out:?}");
            let out = unchanged(dir.path(), chained.as_bytes(), &selectors, None);
            assert_eq!(count(&out, COP), chained_n, "{config:?} {out:?}");
        }
    }
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for (before, _, _) in CASES {
            let block = format!("# {prefix}:disable {COP}\n{before}# {prefix}:enable {COP}\n");
            let out = unchanged(
                dir.path(),
                block.as_bytes(),
                &["--only", COP, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out, COP), 0, "{out:?}");
        }
        // A directive comment on the expression's lines is a comment within
        // it: nothing is reported (upstream's `comment_within?`).
        let inline = format!("x = foo(1, # {prefix}:disable Lint/Debugger\n  2)\n");
        let out = unchanged(
            dir.path(),
            inline.as_bytes(),
            &["--only", COP, "--fix"],
            Some(0),
        );
        assert_eq!(count(&out, COP), 0, "{out:?}");
    }
}

#[test]
fn stdin_reports_positions_and_parse_errors_report_nothing() {
    let dir = project();
    for (before, _, position) in CASES {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "x.rb"])
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
    let out = unchanged(
        dir.path(),
        b"x = foo(1,\n  2\n",
        &["--only", COP, "--fix"],
        Some(2),
    );
    assert_eq!(count(&out, COP), 0, "{out:?}");
}

/// Programs whose runtime results (stdout and `-w` warnings) the joins must
/// keep, with their single-run results; `true` also requires the same
/// layout-free token stream, location-free AST and comments (a folded
/// string changes tokens and the tree, not the value).
const RUNTIME: &[(&str, &str, bool)] = &[
    (
        "x = [1, 2]\n  .map(&:to_s)\n  .join\np x\n",
        "x = [1, 2].map(&:to_s).join\np x\n",
        true,
    ),
    // The parser block starts at its receiver: a multi-line block is not
    // inspected at the fixed `InspectBlocks: false`.
    (
        "x = [1, 2]\n  .map { |v| v * 2 }\n  .sum\np x\n",
        "x = [1, 2]\n  .map { |v| v * 2 }\n  .sum\np x\n",
        true,
    ),
    // A command call's argument does not continue on the next line: the
    // join would reparse differently, so nothing is reported.
    (
        "p [1, 2]\n  .map { |v| v * 2 }\n  .size\n",
        "p [1, 2]\n  .map { |v| v * 2 }\n  .size\n",
        true,
    ),
    (
        "def f(a, b) = a + b\nx = f(1,\n  2)\np x\n",
        "def f(a, b) = a + b\nx = f(1, 2)\np x\n",
        true,
    ),
    (
        "class A\n  def g\n    [1,\n     2].sum\n  end\nend\np A.new.g\n",
        "class A\n  def g\n    [1, 2].sum\n  end\nend\np A.new.g\n",
        true,
    ),
    (
        "x = 'a' \\\n  'b'\np x, x.frozen?\n",
        "x = 'ab'\np x, x.frozen?\n",
        false,
    ),
    (
        "# frozen_string_literal: true\nx = \"a#{1}\" \\\n  \"b\"\np x\n",
        "# frozen_string_literal: true\nx = \"a#{1}b\"\np x\n",
        false,
    ),
    ("p(%w[a\n  b])\n", "p(%w[a b])\n", true),
    (
        "a = [1]\na &&= a\n  .map(&:succ)\np a\n",
        "a = [1]\na &&= a.map(&:succ)\np a\n",
        true,
    ),
    (
        "x = true && \\\n  false\np x\n",
        "x = true && false\np x\n",
        true,
    ),
    (
        "p({ a: 1,\n  b: 2 }[:b])\n",
        "p({ a: 1, b: 2 }[:b])\n",
        true,
    ),
];

#[test]
fn corrections_keep_values_warnings_and_where_unfolded_tokens_ast_and_comments() {
    let dir = project();
    for (source, after, same_tree) in RUNTIME {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", COP]);
        assert_eq!(out.status.code(), Some(0), "{source:?} {out:?}");
        assert_eq!(String::from_utf8_lossy(&fixed), *after, "{source:?}");
        if *same_tree {
            same_tokens_ast_and_comments(source.as_bytes(), &fixed);
        }
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &fixed).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{after:?}");
        }
    }
}

#[test]
fn single_line_block_chain_takes_precedence_without_oscillation() {
    // With both enabled, SingleLineBlockChain breaks the chain and this cop
    // defers to it; joining never undoes the break.
    let dir = project();
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("{ENABLE}[cops.\"Layout/SingleLineBlockChain\"]\nenabled = true\n"),
    )
    .unwrap();
    for (selection, source, result) in [
        (
            vec![
                "--only",
                "Layout/RedundantLineBreak,Layout/SingleLineBlockChain",
            ],
            "x = [1, 2].map { |v| v * 2 }.sum\np x\n",
            "x = [1, 2].map { |v| v * 2 }\n.sum\np x\n",
        ),
        (
            vec![
                "--only",
                "Layout/RedundantLineBreak,Layout/SingleLineBlockChain",
            ],
            "x = [1, 2].map { |v| v * 2 }\n  .sum\np x\n",
            "x = [1, 2].map { |v| v * 2 }\n  .sum\np x\n",
        ),
        (
            vec![],
            "x = [1, 2].map { |v| v * 2 }.sum\np x\np(foo(1,\n  2)) if false\n",
            "x = [1, 2].map { |v| v * 2 }\n          .sum\np x\np(foo(1, 2)) if false\n",
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
        // Stable: no further edit (Lint findings may remain).
        let out = unchanged(dir.path(), &after, &args, None);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("0 corrections applied"),
            "{out:?}"
        );
        assert_eq!(ruby(dir.path()), before);
    }
}

#[test]
fn layout_and_default_selections_converge_without_oscillation() {
    let dir = project();
    fs::write(dir.path().join("rustocop.toml"), ENABLE).unwrap();
    for (selection, source, result) in [
        (
            vec!["--only", "Layout"],
            "def foo(*) = 1\nx = foo(\n  1,\n)\ny = [1,\n  2].sum\np x, y\n",
            "def foo(*) = 1\nx = foo(1,)\ny = [1, 2].sum\np x, y\n",
        ),
        (
            vec![],
            "def foo(*) = 1\nx = foo(\n  1,\n)\np x\nitems = [1]\nitems.each { p 1 }\nitems.each { p 2 }\n",
            "def foo(*) = 1\nx = foo(1)\np x\nitems = [1]\nitems.each { p 1 }\nitems.each { p 2 }\n",
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
        // Stable: no further edit (Lint findings may remain).
        let out = unchanged(dir.path(), &after, &args, None);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("0 corrections applied"),
            "{out:?}"
        );
        assert_eq!(ruby(dir.path()), before);
    }
}

#[test]
fn many_top_level_candidates_stay_bounded() {
    // Each reported expression is verified by one reparse of its scope (the
    // whole file at the top level), as upstream.
    let dir = project();
    let source: String = (0..400).map(|i| format!("foo({i},\n  {i})\n")).collect();
    let start = std::time::Instant::now();
    let out = unchanged(dir.path(), source.as_bytes(), &["--only", COP], Some(1));
    assert_eq!(count(&out, COP), 400);
    assert!(start.elapsed() < std::time::Duration::from_secs(60));
}
