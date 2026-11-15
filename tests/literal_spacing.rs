//! (family literal-spacing): actual CLI lifecycle for
//! the default-on Layout/SpaceInsideArrayPercentLiteral,
//! Layout/SpaceInsidePercentLiteralDelimiters, Layout/SpaceInsideRangeLiteral
//! and Layout/SpaceInsideStringInterpolation, alone, together and with the
//! Layout and default selections. No `%x` or backtick command is ever
//! executed: command literals are only linted.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const ARRAY: &str = "Layout/SpaceInsideArrayPercentLiteral";
const DELIMITERS: &str = "Layout/SpaceInsidePercentLiteralDelimiters";
const RANGE: &str = "Layout/SpaceInsideRangeLiteral";
const INTERPOLATION: &str = "Layout/SpaceInsideStringInterpolation";

/// (cop, offending source, canonical correction, line:col)
const CASES: &[(&str, &str, &str, &str)] = &[
    (ARRAY, "x = %w(a  b)\np x\n", "x = %w(a b)\np x\n", "1:9"),
    (
        DELIMITERS,
        "x = %w( a b)\np x\n",
        "x = %w(a b)\np x\n",
        "1:8",
    ),
    (RANGE, "x = 1 .. 2\np x\n", "x = 1..2\np x\n", "1:5"),
    (
        INTERPOLATION,
        "x = 1\np \"a#{ x}\"\n",
        "x = 1\np \"a#{x}\"\n",
        "2:7",
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
/// 3.4, when available. Only these authored programs are executed; none of
/// them runs a command.
fn ruby(root: &Path) -> Option<String> {
    let ruby = crate::support::runtime::ruby().ok()?;
    let source = fs::read(root.join("example.rb")).unwrap();
    assert!(
        !source.contains(&b'`') && !source.windows(2).any(|w| w == b"%x"),
        "authored runtime programs never hold a command"
    );
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
/// stream without layout tokens (a `%w` separator is layout), the AST
/// without locations or their slices, the comments and the source encoding
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
        layout = %i[NEWLINE IGNORED_NEWLINE EOF COMMENT EMBDOC_BEGIN EMBDOC_LINE EMBDOC_END WORDS_SEP]
        shape = ARGV.map do |path|
          source = File.binread(path)
          parsed = Prism.parse(source, version: "3.4")
          abort("#{path}: #{parsed.errors.map(&:message)}") unless parsed.errors.empty?
          tokens = Prism.lex(source, version: "3.4").value.filter_map do |token, _state|
            [token.type, token.value.b] unless layout.include?(token.type)
          end
          # Locations and their slices (`[1 ..]` of an index call's
          # message) are layout; the tokens above carry every value.
          tree = parsed.value.inspect.b.gsub(/\(\d+,\d+\)-\(\d+,\d+\)/n, "").gsub(/^(.*_loc:) += .*$/n, '\\1')
          [tokens, tree, parsed.comments.map { |c| c.location.slice.b }.sort, parsed.source.source.encoding]
        end
        abort("token, AST, comment or encoding change") unless shape[0] == shape[1]
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
        assert!(found.autocorrect_deferred_to().is_empty(), "{cop}");
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
            "EnforcedStyle = \"space\"",
            "SupportedStyles = []",
            "max = 1",
            "AllowForAlignment = true",
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
        "Layout/SpaceInsideArrayPercentLiterals",
        "Layout/SpaceInsidePercentLiteralDelimiter",
        "Layout/SpaceInsideRangeLiterals",
        "Layout/SpaceInsideStringInterpolations",
        "Style/SpaceInsideRangeLiteral",
        "Layout/SpaceInsideInterpolation",
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
    for (cop, source, n) in [
        // The trimmed blanks are part of the command string (never run).
        (DELIMITERS, "x = %x( ls )\n", 2),
        (DELIMITERS, "x = %x( )\n", 1),
        (DELIMITERS, "x = %x(\n)\n", 1),
        // NUL is a `%w` element that `String#strip` drops.
        (DELIMITERS, "p %w(\0)\n", 1),
        // A run inside interpolated code, not a separator.
        (ARRAY, "x = %W(#{\"a  b\"} c)\np x\n", 1),
        // `...` at the end of a line warns.
        (RANGE, "x = 0 ...\n  10\np x\n", 1),
        // A line continuation before the operator.
        (RANGE, "x = 0 \\\n  .. 10\np x\n", 1),
        // A comment line between the operator and the operand.
        (RANGE, "x = 0 ..\n  # c\n  10\np x\n", 1),
        // No blank at the actual operator (upstream's text test).
        (RANGE, "x = 'a .. b'..'z'\np x\n", 1),
    ] {
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix"],
            Some(1),
        );
        assert_eq!(count(&out, cop), n, "{source:?} {out:?}");
    }
}

#[test]
fn legacy_and_binary_bytes_are_kept_byte_for_byte() {
    let dir = project();
    // Every payload is one ASCII space, an operator or nothing, never
    // decoded source text.
    for (cop, before, after) in [
        (
            ARRAY,
            &b"# encoding: shift_jis\nx = %w(\x95\x5c  b)\np x.size\n"[..],
            &b"# encoding: shift_jis\nx = %w(\x95\x5c b)\np x.size\n"[..],
        ),
        (
            ARRAY,
            b"# encoding: binary\nx = %w(\xff  \xfe)\np x.size\n",
            b"# encoding: binary\nx = %w(\xff \xfe)\np x.size\n",
        ),
        (
            ARRAY,
            b"# encoding: iso-8859-1\nx = %i(\xe9  b)\np x.size\n",
            b"# encoding: iso-8859-1\nx = %i(\xe9 b)\np x.size\n",
        ),
        (
            DELIMITERS,
            b"# encoding: shift_jis\nx = %w(\x95\x5c )\np x.size\n",
            b"# encoding: shift_jis\nx = %w(\x95\x5c)\np x.size\n",
        ),
        (
            DELIMITERS,
            b"# encoding: binary\nx = %W( \xff#{1} )\np x.size\n",
            b"# encoding: binary\nx = %W(\xff#{1})\np x.size\n",
        ),
        (
            RANGE,
            b"# encoding: binary\nx = \"\xff\" .. \"\xfe\"\np x.begin.bytesize\n",
            b"# encoding: binary\nx = \"\xff\"..\"\xfe\"\np x.begin.bytesize\n",
        ),
        (
            RANGE,
            b"# encoding: shift_jis\nx = '\x95\x5c' .. 'b'\np x.end\n",
            b"# encoding: shift_jis\nx = '\x95\x5c'..'b'\np x.end\n",
        ),
        (
            INTERPOLATION,
            b"# encoding: binary\nx = \"\xff#{ 1 }\"\np x.bytesize\n",
            b"# encoding: binary\nx = \"\xff#{1}\"\np x.bytesize\n",
        ),
        (
            INTERPOLATION,
            b"# encoding: shift_jis\nx = \"\x95\x5c#{ 1}\"\np x.bytesize\n",
            b"# encoding: shift_jis\nx = \"\x95\x5c#{1}\"\np x.bytesize\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), before).unwrap();
        let output = ruby(dir.path());
        let (out, fixed) = fix(dir.path(), before, &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{cop} {out:?}");
        assert_eq!(fixed, after, "{cop}");
        same_shape(before, &fixed);
        if let Some(output) = output {
            assert_eq!(ruby(dir.path()), Some(output), "{cop}");
        }
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
/// correction must keep, with their exact corrected bytes. None holds a
/// command.
const RUNTIME: &[(&str, &str, &str)] = &[
    (
        ARRAY,
        "a = 1\np %w(x  y   z), %i(p  q), %W(#{a}  b\\  c  d), %I(e  f#{a}  g)\np %w[h\\   i], %w!j  k!, %w{l  m\n  n  o}\n",
        "a = 1\np %w(x y z), %i(p q), %W(#{a} b\\  c d), %I(e f#{a} g)\np %w[h\\  i], %w!j k!, %w{l m\n  n o}\n",
    ),
    (
        DELIMITERS,
        "a = 1\np %w( x y  ), %i(  p ), %W( #{a} b\\  ), %I(  )\np %w[\\ h i\\  ], %w!  !, %w{\n}, %i( \t )\n",
        "a = 1\np %w(x y), %i(p), %W(#{a} b\\ ), %I()\np %w[\\ h i\\ ], %w!!, %w{}, %i()\n",
    ),
    (
        RANGE,
        "a = 2\nb = [1 .. a, 1... 3, (.. a), (a ..), 'a' .. 'c']\np b, b.first.to_a\nc = 0 ..\n  a\np c, (1 .. 2).sum\ncase 3\nin 1 .. 5 then p :in\nend\np [1, 2, 3][1 ..]\n",
        "a = 2\nb = [1..a, 1...3, (..a), (a..), 'a'..'c']\np b, b.first.to_a\nc = 0..a\np c, (1..2).sum\ncase 3\nin 1..5 then p :in\nend\np [1, 2, 3][1..]\n",
    ),
    (
        INTERPOLATION,
        "a = 1\np \"#{ a }\", :\"s#{ a}\", /r#{a }/.source, \"#{ \"#{ a }\" }\"\nputs <<~T\n  x #{ a; a + 1 } y\nT\np \"#{ [a].map { _1 } }\", %W(#{ a } b), \"#{\ta\t}\"\n",
        "a = 1\np \"#{a}\", :\"s#{a}\", /r#{a}/.source, \"#{\"#{a}\"}\"\nputs <<~T\n  x #{a; a + 1} y\nT\np \"#{[a].map { _1 }}\", %W(#{a} b), \"#{a}\"\n",
    ),
    // The blank of `?\ ` and the delimiters of `% x ` are code, not layout.
    (
        INTERPOLATION,
        "p \"#{ ?\\ }}\", \"#{?\\ }\", \"#{% x }\", \"#{ % x  }\"\n",
        "p \"#{?\\ }}\", \"#{?\\ }\", \"#{% x }\", \"#{% x }\"\n",
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
    let source = "a = 1\nx = %w(  p  q  )\ny = (a .. 3)\nz = \"#{ a }-#{ x.size }\"\nw = %i( )\np x, y, z, w\n";
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
        (ARRAY, "%w(a  b)\n", "%w(a b)\n"),
        (DELIMITERS, "%w( a)\n", "%w(a)\n"),
        (RANGE, "(1 .. 2)\n", "(1..2)\n"),
        (INTERPOLATION, "\"#{ 1}\"\n", "\"#{1}\"\n"),
    ] {
        let source = unit.repeat(4000);
        let out = unchanged(dir.path(), source.as_bytes(), &["--only", cop], Some(1));
        assert_eq!(count(&out, cop), 4000, "{cop}");
        let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, fixed_unit.repeat(4000).as_bytes(), "{cop}");
    }
    // One literal with many runs and one long range operand.
    let words = format!("x = %w({})\n", "a  ".repeat(20000));
    let out = unchanged(dir.path(), words.as_bytes(), &["--only", ARRAY], Some(1));
    assert_eq!(count(&out, ARRAY), 19999);
}
