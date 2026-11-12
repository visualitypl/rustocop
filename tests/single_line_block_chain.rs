//! Actual CLI lifecycle for the opt-in Layout/SingleLineBlockChain,
//! alone and with the Layout and default selections and its upstream
//! pass-ordering Style peers.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Layout/SingleLineBlockChain";

/// (offending source, canonical correction, CRLF correction, line:col)
const CASES: [(&str, &str, &str, &str); 3] = [
    (
        "x = [1, 2].map { |v| v * 2 }.sum\n",
        "x = [1, 2].map { |v| v * 2 }\n.sum\n",
        "x = [1, 2].map { |v| v * 2 }\r\n.sum\r\n",
        "1:29",
    ),
    (
        "x = [1].map { _1 }&.first\n",
        "x = [1].map { _1 }\n&.first\n",
        "x = [1].map { _1 }\r\n&.first\r\n",
        "1:19",
    ),
    (
        "f = ->(v) { v }.(1)\n",
        "f = ->(v) { v }\n.(1)\n",
        "f = ->(v) { v }\r\n.(1)\r\n",
        "1:16",
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
fn opt_in_metadata_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    let found = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(!found.default_enabled());
    assert!(!found.department_selection_enables());
    assert!(found.safe_fixes());
    assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
    assert!(found.path_policy().is_none() && found.max_setting().is_none());
    // Upstream's pass-ordering peers are default-on and not declared.
    assert!(found.autocorrect_incompatible_with().is_empty());
    let dir = project();
    for (before, _, _, position) in CASES {
        for (config, selectors, n) in [
            (String::new(), vec![], 0),
            (String::new(), vec!["--only", COP], 1),
            // Opt-in: a department selection never enables it.
            (String::new(), vec!["--only", "Layout"], 0),
            ("[cops.Layout]\nenabled = true\n".into(), vec![], 0),
            (String::new(), vec!["--only", COP, "--except", COP], 0),
            (String::new(), vec!["--only", COP, "--except", "Layout"], 0),
            (format!("[cops.\"{COP}\"]\nenabled = true\n"), vec![], 1),
            (
                format!("[cops.\"{COP}\"]\nenabled = true\n"),
                vec!["--only", "Layout"],
                1,
            ),
            (
                format!("[cops.\"{COP}\"]\nenabled = true\n"),
                vec!["--except", COP],
                0,
            ),
            (
                format!("[cops.Layout]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (
                format!("[cops.\"{COP}\"]\nenabled = true\nexclude = [\"example.rb\"]\n"),
                vec![],
                0,
            ),
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
    for key in ["EnforcedStyle = \"x\"", "AllowedMethods = []", "max = 1"] {
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
    let dir = project();
    for name in [
        "Layout/SingleLineBlockChains",
        "Layout/SingleLineBlock",
        "Style/SingleLineBlockChain",
    ] {
        unchanged(dir.path(), b"x = 1\n", &["--only", name], Some(2));
    }
}

#[test]
fn lint_never_writes_and_fix_writes_the_canonical_edit_once() {
    let dir = project();
    for (before, after, crlf, _) in CASES {
        let out = unchanged(dir.path(), before.as_bytes(), &["--only", COP], Some(1));
        assert_eq!(count(&out, COP), 1, "{out:?}");
        let (out, fixed) = fix(dir.path(), before.as_bytes(), &["--only", COP]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&fixed), after);
        same_tokens_ast_and_comments(before.as_bytes(), &fixed);
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", COP, "--fix"],
            Some(0),
        );
        let (_, fixed) = fix(
            dir.path(),
            format!("\u{feff}{before}").as_bytes(),
            &["--only", COP],
        );
        assert_eq!(fixed, format!("\u{feff}{after}").as_bytes());
        // CRLF: the inserted break keeps the line's `\r\n` (upstream inserts
        // a bare `\n`).
        let (_, fixed) = fix(
            dir.path(),
            before.replace('\n', "\r\n").as_bytes(),
            &["--only", COP],
        );
        assert_eq!(String::from_utf8_lossy(&fixed), crlf);
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
                &["--only", COP, "--fix"],
                Some(1),
            );
            assert_eq!(count(&out, COP), 1, "{out:?}");
        }
        let data = format!("{before}p DATA.read\n__END__\nz\n");
        unchanged(
            dir.path(),
            data.as_bytes(),
            &["--only", COP, "--fix"],
            Some(1),
        );
        let lines = format!("{before}p __LINE__\n");
        unchanged(
            dir.path(),
            lines.as_bytes(),
            &["--only", COP, "--fix"],
            Some(1),
        );
        let (out, fixed) = fix(
            dir.path(),
            format!("p __LINE__\n{before}").as_bytes(),
            &["--only", COP],
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, format!("p __LINE__\n{after}").as_bytes());
    }
    for source in [
        "x = [1].map { 1 }::size\n",
        "p(<<~T, [1].map { 1 }.size)\n  t\nT\n",
        "x = [1].map { 1 }.size # rubocop:disable Lint/Debugger\n",
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix"],
            Some(1),
        );
        assert_eq!(count(&out, COP), 1, "{source:?} {out:?}");
    }
    // A legacy-encoded block keeps its bytes.
    let latin = b"# encoding: iso-8859-1\nx = [\"\xe9\"].map { |v| v }.size\n";
    let (out, fixed) = fix(dir.path(), latin, &["--only", COP]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fixed,
        b"# encoding: iso-8859-1\nx = [\"\xe9\"].map { |v| v }\n.size\n"
    );
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for (before, _, _, _) in CASES {
            let disabled = format!("{} # {prefix}:disable {COP}\n", before.trim_end());
            let out = unchanged(
                dir.path(),
                disabled.as_bytes(),
                &["--only", COP, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out, COP), 0, "{out:?}");
            let block = format!("# {prefix}:disable {COP}\n{before}# {prefix}:enable {COP}\n");
            let out = unchanged(
                dir.path(),
                block.as_bytes(),
                &["--only", COP, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out, COP), 0, "{out:?}");
        }
    }
}

#[test]
fn stdin_reports_positions_and_parse_errors_report_nothing() {
    let dir = project();
    for (before, _, _, position) in CASES {
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
        b"x = [1].map { 1 }.size(\n",
        &["--only", COP, "--fix"],
        Some(2),
    );
    assert_eq!(count(&out, COP), 0, "{out:?}");
}

/// Programs whose runtime results (stdout and `-w` warnings), tokens, AST and
/// comments the correction must keep.
const RUNTIME: &[(&str, &str)] = &[
    (
        "p [1, 2].map { |v| v * 2 }.sum\n",
        "p [1, 2].map { |v| v * 2 }\n.sum\n",
    ),
    (
        "p [3, 1].sort_by { it }.first(1).size\n",
        "p [3, 1].sort_by { it }\n.first(1).size\n",
    ),
    (
        "x = nil\np x&.then { 1 }&.succ\n",
        "x = nil\np x&.then { 1 }\n&.succ\n",
    ),
    ("p(->(v) { v + 1 }.(1))\n", "p(->(v) { v + 1 }\n.(1))\n"),
    (
        "h = Struct.new(:a).new(1)\n[1].each { h }.size.then { h.a += 1 }\np h.a\n",
        "h = Struct.new(:a).new(1)\n[1].each { h }\n.size.then { h.a += 1 }\np h.a\n",
    ),
    (
        "p [1].map { _1 }.join(<<~S)\n  ,\nS\n",
        "p [1].map { _1 }\n.join(<<~S)\n  ,\nS\n",
    ),
];

#[test]
fn corrections_keep_tokens_ast_comments_values_and_warnings() {
    let dir = project();
    for (source, after) in RUNTIME {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        let before = ruby(dir.path());
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", COP]);
        assert_eq!(out.status.code(), Some(0), "{source:?} {out:?}");
        assert_eq!(String::from_utf8_lossy(&fixed), *after, "{source:?}");
        same_tokens_ast_and_comments(source.as_bytes(), &fixed);
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &fixed).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{after:?}");
        }
    }
}

#[test]
fn upstream_pass_ordering_peers_compose() {
    // Style/NilComparison (safe) only rewrites the operator form `x ==
    // nil`, never the dotted `.==(nil)` call this cop breaks. Style/MapToHash,
    // Style/FloatDivision and Style/RedundantSort are unsafe and never edit.
    let dir = project();
    for (selection, source, result) in [
        (
            "Layout/SingleLineBlockChain,Style/NilComparison",
            "p [1].find { _1 > 1 }.==(nil)\n",
            "p [1].find { _1 > 1 }\n.==(nil)\n",
        ),
        (
            "Layout/SingleLineBlockChain,Style/MapToHash",
            "p [1].map { |x| [x, x] }.to_h\n",
            "p [1].map { |x| [x, x] }\n.to_h\n",
        ),
        (
            "Layout/SingleLineBlockChain,Style/FloatDivision",
            "p [1].sum { _1 }.fdiv(2)\n",
            "p [1].sum { _1 }\n.fdiv(2)\n",
        ),
        (
            "Layout/SingleLineBlockChain,Style/RedundantSort",
            "p [2, 1].sort_by { _1 }.first\n",
            "p [2, 1].sort_by { _1 }\n.first\n",
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
        let out = unchanged(dir.path(), &after, &["--only", COP], Some(0));
        assert_eq!(count(&out, COP), 0, "{out:?}");
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &after).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{result:?}");
        }
    }
}

#[test]
fn layout_and_default_selections_converge_without_oscillation() {
    let dir = project();
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    for (selection, source, result) in [
        (
            vec!["--only", "Layout"],
            "x = [1, 2].map { |v| v * 2 }.sum\np x\n",
            "x = [1, 2].map { |v| v * 2 }\n          .sum\np x\n",
        ),
        (
            vec![],
            "x = [1, 2].map { |v| v * 2 }.sum\np x\ny = [3].map { _1 }&.first\np y\n",
            "x = [1, 2].map { |v| v * 2 }\n          .sum\np x\ny = [3].map { _1 }\n       &.first\np y\n",
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
