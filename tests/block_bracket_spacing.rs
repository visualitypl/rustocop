//! (family block-bracket-spacing): actual CLI
//! lifecycle for the default-on Layout/SpaceAroundBlockParameters,
//! Layout/SpaceInLambdaLiteral, Layout/SpaceBeforeBlockBraces and
//! Layout/SpaceBeforeBrackets, alone, together and with the Layout and
//! default selections, the configured Layout/SpaceInsideParens dependency
//! and the per-pass precedence of the upstream-incompatible
//! Style/BlockDelimiters and Style/SymbolProc.
use crate::support::cli::{count, project, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const PARAMS: &str = "Layout/SpaceAroundBlockParameters";
const LAMBDA: &str = "Layout/SpaceInLambdaLiteral";
const BRACES: &str = "Layout/SpaceBeforeBlockBraces";
const BRACKETS: &str = "Layout/SpaceBeforeBrackets";

/// (cop, offending source, canonical correction, line:col)
const CASES: &[(&str, &str, &str, &str)] = &[
    (
        PARAMS,
        "[1].each { | x| p x }\n",
        "[1].each { |x| p x }\n",
        "1:13",
    ),
    (
        LAMBDA,
        "f = -> (x) { x }\np f.(1)\n",
        "f = ->(x) { x }\np f.(1)\n",
        "1:7",
    ),
    (
        BRACES,
        "x = [1].map{ 2 }\np x\n",
        "x = [1].map { 2 }\np x\n",
        "1:12",
    ),
    (
        BRACKETS,
        "@a = [1]\np @a [0]\n",
        "@a = [1]\np @a[0]\n",
        "2:5",
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
    args.extend(["--fix", "--fix-passes", "1", "example.rb"]);
    let out = run(root, &args);
    (out, fs::read(&path).unwrap())
}

/// `example.rb`'s output and `-w` warnings under native Ruby
/// 3.4, when available. Only these authored programs are executed.
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

/// Native Ruby 3.4's Prism view of both sides (never executed): the token
/// stream without layout tokens, the AST without locations and the comments
/// must be equal.
fn same_shape(before: &[u8], after: &[u8]) {
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
            [token.type, token.value] unless layout.include?(token.type)
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
fn default_on_metadata_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    for (cop, ..) in CASES {
        let found = registry.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert!(found.default_enabled(), "{cop}");
        assert!(found.department_selection_enables(), "{cop}");
        assert!(found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        assert!(found.ruby_versions().minimum.is_none());
        assert!(found.ruby_versions().maximum.is_none());
        assert!(found.autocorrect_incompatible_with().is_empty(), "{cop}");
        let deferred: &[&str] = if *cop == BRACES {
            &["Style/BlockDelimiters", "Style/SymbolProc"]
        } else {
            &[]
        };
        assert_eq!(found.autocorrect_deferred_to(), deferred);
    }
    let dir = project();
    for &(cop, before, _, position) in CASES {
        for (config, selectors, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Layout"], 1),
            (String::new(), vec!["--only", "Style"], 0),
            (String::new(), vec!["--except", cop], 0),
            (String::new(), vec!["--except", "Layout"], 0),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
            (
                "[cops.Layout]\nenabled = false\n".into(),
                vec!["--only", cop],
                1,
            ),
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
            (
                "exclude = [\"example.rb\"]\n".into(),
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
            "EnforcedStyle = \"require_space\"",
            "EnforcedStyleInsidePipes = \"space\"",
            "EnforcedStyleForEmptyBraces = \"no_space\"",
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
        "Layout/SpaceAroundBlockParameter",
        "Layout/SpaceInLambdaLiterals",
        "Style/SpaceInLambdaLiteral",
        "Layout/SpaceAroundBlockArguments",
        "Layout/SpaceInLambda",
        "Layout/SpaceBeforeBlockBrace",
        "Layout/SpaceBeforeBracket",
        "Style/SpaceBeforeBrackets",
    ] {
        unchanged(dir.path(), b"x = 1\n", &["--only", name], Some(2));
    }
}

#[test]
fn uncertain_edits_keep_the_finding_and_the_bytes() {
    let dir = project();
    for (cop, source) in [
        // Removing the `;` would turn the block-local variable into a
        // parameter.
        (PARAMS, "x = 1\n[2].each { |;x| x = 3 }\np x\n"),
        // Removing a line break would move a later `__LINE__`.
        (LAMBDA, "f = -> \\\n  (x) { x }\np __LINE__\n"),
        (BRACKETS, "a = [1]\na \\\n  [0]\np __LINE__\n"),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix"],
            Some(1),
        );
        assert_eq!(count(&out, cop), 1, "{source:?} {out:?}");
    }
    // Legacy and BINARY bytes elsewhere in the file are kept byte for byte:
    // every payload is a blank or nothing, never decoded source text.
    for (cop, before, after) in [
        (
            PARAMS,
            &b"# encoding: iso-8859-1\n[1].each { | x|p '\xe9' }\n"[..],
            &b"# encoding: iso-8859-1\n[1].each { |x| p '\xe9' }\n"[..],
        ),
        (
            PARAMS,
            b"# encoding: binary\n[1].each { |x,  y| \"\xff\xfe\" }\n",
            b"# encoding: binary\n[1].each { |x, y| \"\xff\xfe\" }\n",
        ),
        (
            PARAMS,
            b"# encoding: shift_jis\n[1].each { |x |'\x95\\' }\n",
            b"# encoding: shift_jis\n[1].each { |x| '\x95\\' }\n",
        ),
        (
            LAMBDA,
            b"# encoding: binary\nf = -> (x) { \"\xff\" }\n",
            b"# encoding: binary\nf = ->(x) { \"\xff\" }\n",
        ),
        (
            LAMBDA,
            b"# encoding: shift_jis\nf = -> (x) { '\x95\\' }\n",
            b"# encoding: shift_jis\nf = ->(x) { '\x95\\' }\n",
        ),
        (
            BRACES,
            b"# encoding: binary\nx = [1].map{ \"\xff\" }\n",
            b"# encoding: binary\nx = [1].map { \"\xff\" }\n",
        ),
        (
            BRACES,
            b"# encoding: iso-8859-1\nx = ['\xe9'].map{ _1 }\n",
            b"# encoding: iso-8859-1\nx = ['\xe9'].map { _1 }\n",
        ),
        (
            BRACKETS,
            b"# encoding: shift_jis\nx = '\x95\\' [0]\n",
            b"# encoding: shift_jis\nx = '\x95\\'[0]\n",
        ),
        (
            BRACKETS,
            b"# encoding: binary\nx = \"\xff\xfe\" [1]\n",
            b"# encoding: binary\nx = \"\xff\xfe\"[1]\n",
        ),
    ] {
        let (out, fixed) = fix(dir.path(), before, &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, after, "{cop}");
    }
}

#[test]
fn both_directive_prefixes_disable_findings_and_edits() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for &(cop, before, _, _) in CASES {
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
    for &(cop, before, _, position) in CASES {
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

#[test]
fn lambda_parentheses_follow_configured_space_inside_parens_not_selection() {
    let dir = project();
    let source = "f = ->( x) { x }\np f.(1)\n";
    for (config, selectors, n) in [
        ("", vec!["--only", PARAMS], 0),
        ("", vec!["--except", "Layout/SpaceInsideParens"], 0),
        ("", vec![], 0),
        (
            "[cops.\"Layout/SpaceInsideParens\"]\nenabled = false\n",
            vec!["--only", PARAMS],
            1,
        ),
        (
            "[cops.\"Layout/SpaceInsideParens\"]\nenabled = false\n",
            vec![],
            1,
        ),
        (
            "[cops.\"Layout/SpaceInsideParens\"]\nenabled = false\n",
            vec!["--only", &format!("{PARAMS},Layout/SpaceInsideParens")],
            1,
        ),
        (
            "[cops.Layout]\nenabled = false\n",
            vec!["--only", PARAMS],
            1,
        ),
        (
            "[cops.Layout]\nenabled = false\n[cops.\"Layout/SpaceInsideParens\"]\nenabled = true\n",
            vec!["--only", PARAMS],
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let out = unchanged(dir.path(), source.as_bytes(), &selectors, None);
        assert_eq!(count(&out, PARAMS), n, "{config:?} {selectors:?} {out:?}");
        if n == 1 {
            let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", PARAMS]);
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            assert_eq!(fixed, b"f = ->(x) { x }\np f.(1)\n");
        }
    }
}

/// Programs whose runtime results (stdout and `-w` warnings) each cop's
/// correction must keep, with their exact corrected bytes.
const RUNTIME: &[(&str, &str, &str)] = &[
    (
        PARAMS,
        "h = {a: 1, b: 2}\nh.each { | k,  v |p [k, v] }\n[[1, [2, 3]]].each do |a,  (b,  c),  d = 4,  *e,  f: 5,  **g,  &h|p [a, b, c, d, e, f, g, h] end\nx = 10\n[1].each { |y;  x|x = y; p x }\np x\nf = ->(a,  b) { a + b }\np f.(1, 2)\n[[1, 2]].each { |a, |p a }\n[3].each do |a|rescue; p a end\n",
        "h = {a: 1, b: 2}\nh.each { |k, v| p [k, v] }\n[[1, [2, 3]]].each do |a, (b, c), d = 4, *e, f: 5, **g, &h| p [a, b, c, d, e, f, g, h] end\nx = 10\n[1].each { |y; x| x = y; p x }\np x\nf = ->(a, b) { a + b }\np f.(1, 2)\n[[1, 2]].each { |a,| p a }\n[3].each do |a| rescue; p a end\n",
    ),
    (
        LAMBDA,
        "g = -> (a, *b, k: 1, &c) { [a, b, k, c] }\np g.(1, 2, k: 3)\nm = ->\ta, b { a * b }\np m.(2, 3)\nn = -> (x = -> (y) { y }) do x.(4) end\np n.()\n",
        "g = ->(a, *b, k: 1, &c) { [a, b, k, c] }\np g.(1, 2, k: 3)\nm = ->a, b { a * b }\np m.(2, 3)\nn = ->(x = ->(y) { y }) do x.(4) end\np n.()\n",
    ),
    (
        BRACES,
        "a = [1, 2]\np a.map{ _1 * 2 }, a.map{ it }, a.sum{|x| x }, a&.map{ 1 }\nf = ->{ 3 }\ng = ->(x){ x + 1 }\nh = -> x{ x }\np f.(), g.(1), h.(2)\ndef m(&b) = b.()\np m{ :blk }\nmap = 1\np [map, [4].map{ map }]\nclass C\n  def each = yield(1)\nend\nclass D < C\n  def each = super{ |v| v }\nend\np D.new.each{ |v| v }\n",
        "a = [1, 2]\np a.map { _1 * 2 }, a.map { it }, a.sum {|x| x }, a&.map { 1 }\nf = -> { 3 }\ng = ->(x) { x + 1 }\nh = -> x { x }\np f.(), g.(1), h.(2)\ndef m(&b) = b.()\np m { :blk }\nmap = 1\np [map, [4].map { map }]\nclass C\n  def each = yield(1)\nend\nclass D < C\n  def each = super { |v| v }\nend\np D.new.each { |v| v }\n",
    ),
    (
        BRACKETS,
        "a = [[1, 2], [3]]\nh = {k: 1}\np a [0], a [0] [1], h [:k]\nh [:k] += 1\nh [:j] ||= 5\nh [:j] &&= 6\na [1], b = [9], 2\np a, h, b\n@x = 'str'\np @x [1], :sym [0], 'ab' [1], [[7]].map { _1 [0] }\nfor a [2] in [8]; end\np a\n",
        "a = [[1, 2], [3]]\nh = {k: 1}\np a[0], a[0][1], h[:k]\nh[:k] += 1\nh[:j] ||= 5\nh[:j] &&= 6\na[1], b = [9], 2\np a, h, b\n@x = 'str'\np @x[1], :sym[0], 'ab'[1], [[7]].map { _1[0] }\nfor a[2] in [8]; end\np a\n",
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
        same_shape(source.as_bytes(), &fixed);
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &fixed).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{after:?}");
        }
    }
}

#[test]
fn layout_and_default_selections_converge_without_oscillation() {
    let dir = project();
    let source =
        "f = -> ( a,  b ){a+b}\n[[1, 2]].each{| x,y |p f.(x,y)}\nh = {k: [1]}\np h [:k] [0]\n";
    let cops = CASES.iter().map(|c| c.0).collect::<Vec<_>>().join(",");
    for selection in [
        vec!["--only", cops.as_str()],
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
        for (cop, ..) in CASES {
            assert_eq!(count(&out, cop), 0, "{selection:?} {out:?}");
        }
        if let Some(before) = before {
            fs::write(dir.path().join("example.rb"), &after).unwrap();
            assert_eq!(ruby(dir.path()), Some(before), "{selection:?}");
        }
    }
}

#[test]
fn many_offenses_stay_linear() {
    let dir = project();
    for (cop, unit, fixed_unit) in [
        (PARAMS, "f { | x| x }\n", "f { |x| x }\n"),
        (LAMBDA, "f = -> (x) { x }\n", "f = ->(x) { x }\n"),
        (BRACES, "f{ 1 }\n", "f { 1 }\n"),
        (BRACKETS, "@a [0]\n", "@a[0]\n"),
    ] {
        let source = unit.repeat(4000);
        let out = unchanged(dir.path(), source.as_bytes(), &["--only", cop], Some(1));
        assert_eq!(count(&out, cop), 4000, "{cop}");
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, fixed_unit.repeat(4000).as_bytes(), "{cop}");
    }
}

#[test]
fn upstream_incompatible_companions_take_precedence_for_one_pass() {
    let dir = project();
    // Neither companion edits a one-line block: the brace edit applies with
    // both selected and in the default run.
    let source = "x = [1].map{ 2 }\np x\n";
    for selectors in [
        vec![
            "--only",
            "Layout/SpaceBeforeBlockBraces,Style/BlockDelimiters",
        ],
        vec!["--only", "Layout/SpaceBeforeBlockBraces,Style/SymbolProc"],
        vec![],
    ] {
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &selectors);
        assert_eq!(out.status.code(), Some(0), "{selectors:?} {out:?}");
        assert_eq!(fixed, b"x = [1].map { 2 }\np x\n", "{selectors:?}");
    }
    // Style/BlockDelimiters edits this file: every brace edit waits for the
    // next explicit pass, as upstream's Team skips the corrector.
    let only = [
        "--only",
        "Layout/SpaceBeforeBlockBraces,Style/BlockDelimiters",
    ];
    let source = b"x = [1].map{ |v|\n  v\n}\ny = [2].map{ 3 }\np x, y\n";
    let (out, first) = fix(dir.path(), source, &only);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        first,
        b"x = [1].map do |v|\n  v\nend\ny = [2].map{ 3 }\np x, y\n"
    );
    assert_eq!(count(&out, BRACES), 1, "{out:?}");
    let (out, second) = fix(dir.path(), &first, &only);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        second,
        b"x = [1].map do |v|\n  v\nend\ny = [2].map { 3 }\np x, y\n"
    );
    // A directive-disabled companion edit defers nothing.
    let disabled = b"# rubocop:disable Style/BlockDelimiters\nx = [1].map{ |v|\n  v\n}\np x\n";
    let (out, fixed) = fix(dir.path(), disabled, &only);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fixed,
        b"# rubocop:disable Style/BlockDelimiters\nx = [1].map { |v|\n  v\n}\np x\n"
    );
}
