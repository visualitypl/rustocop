//! Actual CLI lifecycle for the default-on
//! Layout/SpaceAfterColon, Layout/SpaceAfterMethodName, Layout/SpaceAfterNot,
//! Layout/SpaceAroundKeyword and Layout/SpaceAroundMethodCallOperator, alone,
//! together and with the Layout and default selections.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COLON: &str = "Layout/SpaceAfterColon";
const METHOD_NAME: &str = "Layout/SpaceAfterMethodName";
const NOT: &str = "Layout/SpaceAfterNot";
const KEYWORD: &str = "Layout/SpaceAroundKeyword";
const CALL: &str = "Layout/SpaceAroundMethodCallOperator";

/// (cop, offending source, canonical correction, line:col)
const CASES: &[(&str, &str, &str, &str)] = &[
    (COLON, "x = {a:1}\np x\n", "x = {a: 1}\np x\n", "1:7"),
    (
        METHOD_NAME,
        "def f (x) = x\np f(1)\n",
        "def f(x) = x\np f(1)\n",
        "1:6",
    ),
    (NOT, "x = ! nil\np x\n", "x = !nil\np x\n", "1:5"),
    (
        KEYWORD,
        "x = 1if true\np x\n",
        "x = 1 if true\np x\n",
        "1:6",
    ),
    (CALL, "x = [1] .size\np x\n", "x = [1].size\np x\n", "1:8"),
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
/// must be equal.
fn same_shape(before: &[u8], after: &[u8]) {
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
        assert!(found.autocorrect_incompatible_with().is_empty(), "{cop}");
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
            "EnforcedStyle = \"no_space\"",
            "SupportedStyles = []",
            "AllowForAlignment = true",
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
        "Layout/SpaceAfterColons",
        "Style/SpaceAfterColon",
        "Layout/SpaceAfterMethod",
        "Layout/SpaceAfterBang",
        "Layout/SpaceAfter",
        "Layout/SpaceAroundKeywords",
        "Layout/SpaceAroundMethodCallOperators",
        "Style/SpaceAroundKeyword",
    ] {
        unchanged(dir.path(), b"x = 1\n", &["--only", name], Some(2));
    }
}

#[test]
fn lint_never_writes_and_fix_writes_the_canonical_edit_once() {
    let dir = project();
    for &(cop, before, after, _) in CASES {
        let out = unchanged(dir.path(), before.as_bytes(), &["--only", cop], Some(1));
        assert_eq!(count(&out, cop), 1, "{out:?}");
        let (out, fixed) = fix(dir.path(), before.as_bytes(), &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&fixed), after);
        same_shape(before.as_bytes(), &fixed);
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
        let (_, fixed) = fix(
            dir.path(),
            before.replace('\n', "\r\n").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(String::from_utf8_lossy(&fixed), after.replace('\n', "\r\n"));
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
        // A one-line edit keeps `__LINE__`.
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
        // The gap holds a comment or an embedded document.
        (NOT, "x = ! # c\n  nil\np x\n"),
        (NOT, "x = !\n=begin\nc\n=end\n  nil\np x\n"),
        // Removing a line break would move a later `__LINE__`.
        (NOT, "x = !\n  nil\np __LINE__\n"),
        // A blank would change how Ruby lexes the next token.
        (KEYWORD, "def f(*)\n  super*[1]\nend\np f\n"),
        (KEYWORD, "def f(a)\n  [a]\nend\np f &begin :to_s end\n"),
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
            COLON,
            &b"# encoding: iso-8859-1\nx = {a:'\xe9'}\n"[..],
            &b"# encoding: iso-8859-1\nx = {a: '\xe9'}\n"[..],
        ),
        (
            COLON,
            b"# encoding: binary\nx = {a:\"\xff\xfe\"}\n",
            b"# encoding: binary\nx = {a: \"\xff\xfe\"}\n",
        ),
        (
            METHOD_NAME,
            b"# encoding: shift_jis\ndef f (x) = '\x95\\'\n",
            b"# encoding: shift_jis\ndef f(x) = '\x95\\'\n",
        ),
        (
            NOT,
            b"# encoding: binary\nx = ! \"\xff\"\n",
            b"# encoding: binary\nx = !\"\xff\"\n",
        ),
        (
            KEYWORD,
            b"# encoding: binary\nx = \"\xff\"if 1\n",
            b"# encoding: binary\nx = \"\xff\" if 1\n",
        ),
        (
            KEYWORD,
            b"# encoding: shift_jis\nx = '\x95\\'if 1\n",
            b"# encoding: shift_jis\nx = '\x95\\' if 1\n",
        ),
        (
            CALL,
            b"# encoding: iso-8859-1\nx = '\xe9' .size\n",
            b"# encoding: iso-8859-1\nx = '\xe9'.size\n",
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

/// Programs whose runtime results (stdout and `-w` warnings) each cop's
/// correction must keep, with their exact corrected bytes.
const RUNTIME: &[(&str, &str, &str)] = &[
    (
        COLON,
        "def m(a:1, b:-2) = [a, b]\nh = {c:?d, \"e\":%w[f], g:/h/}\np m, m(a:3), h\nf = ->(x:1) { x }\np f.(x:2)\n",
        "def m(a: 1, b: -2) = [a, b]\nh = {c: ?d, \"e\": %w[f], g: /h/}\np m, m(a: 3), h\nf = ->(x: 1) { x }\np f.(x: 2)\n",
    ),
    (
        COLON,
        "case {k:1}\nin {k:Integer => n}\n  p n\nend\n",
        "case {k: 1}\nin {k: Integer => n}\n  p n\nend\n",
    ),
    (
        METHOD_NAME,
        "class A\n  def f (x, y = 1) = x + y\n  def self.g (*a) = a\n  def + (o) = o\n  def []= (k, v); p [k, v]; end\nend\np A.new.f(1), A.g(2), A.new + 3\nA.new[4] = 5\n",
        "class A\n  def f(x, y = 1) = x + y\n  def self.g(*a) = a\n  def +(o) = o\n  def []=(k, v); p [k, v]; end\nend\np A.new.f(1), A.g(2), A.new + 3\nA.new[4] = 5\n",
    ),
    (
        NOT,
        "a = [1]\np ! a.empty?, !  (a.size > 1), ! -1.zero?, !\\\n  nil\np(! a.map do |x| x end.empty?)\n",
        "a = [1]\np !a.empty?, !(a.size > 1), !-1.zero?, !nil\np(!a.map do |x| x end.empty?)\n",
    ),
    (
        KEYWORD,
        "def f(a)\n  return-a if a.zero?\n  [1].each do|x|p x end\n  r = begin\"b\"end\n  r = if a then\"t\" else \"e\"end\n  q = (a in\"q\")\n  p r, (defined?a), (not\"x\"), q\n  1while false\n  Integer(\"z\") rescue\"r\"\nend\np f(1), f(0)\nclass B < Struct.new(:v)\n  def to_s = super\"\"\nend\nBEGIN{ p :begin }\n",
        "def f(a)\n  return -a if a.zero?\n  [1].each do |x|p x end\n  r = begin \"b\" end\n  r = if a then \"t\" else \"e\" end\n  q = (a in \"q\")\n  p r, (defined? a), (not \"x\"), q\n  1 while false\n  Integer(\"z\") rescue \"r\"\nend\np f(1), f(0)\nclass B < Struct.new(:v)\n  def to_s = super \"\"\nend\nBEGIN { p :begin }\n",
    ),
    (
        CALL,
        "a = [3, 1]\np a .sort. first, a&. size, a\n  . map { _1 + 1 } .sum\np Float:: MAX > 1, :: Integer\nX = ::\tFloat\np X:: INFINITY\n",
        "a = [3, 1]\np a.sort.first, a&.size, a\n  .map { _1 + 1 }.sum\np Float::MAX > 1, ::Integer\nX = ::Float\np X::INFINITY\n",
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
    let source = "def f (a:1)\n  ! a .nil?if true\nend\np f(a:nil), {b:2}, [1] .size\n";
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
        (COLON, "x = {a:1}\n", "x = {a: 1}\n"),
        (METHOD_NAME, "def f (x) = x\n", "def f(x) = x\n"),
        (NOT, "x = ! nil\n", "x = !nil\n"),
        (KEYWORD, "x = 1if a\n", "x = 1 if a\n"),
        (CALL, "x = a .b\n", "x = a.b\n"),
    ] {
        let source = unit.repeat(4000);
        let out = unchanged(dir.path(), source.as_bytes(), &["--only", cop], Some(1));
        assert_eq!(count(&out, cop), 4000, "{cop}");
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, fixed_unit.repeat(4000).as_bytes(), "{cop}");
    }
}
