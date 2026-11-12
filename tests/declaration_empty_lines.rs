//! Actual CLI lifecycle for
//! Layout/EmptyLinesAfterModuleInclusion, Layout/EmptyLinesAroundAttributeAccessor
//! and Layout/EmptyLinesAroundAccessModifier.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const INCLUSION: &str = "Layout/EmptyLinesAfterModuleInclusion";
const ACCESSOR: &str = "Layout/EmptyLinesAroundAttributeAccessor";
const MODIFIER: &str = "Layout/EmptyLinesAroundAccessModifier";

/// `(cop, before, after, first finding)`: correction witnesses.
const WITNESSES: &[(&str, &str, &str, &str)] = &[
    (
        INCLUSION,
        "class Foo\n  include Bar\n  def baz; end\nend\n",
        "class Foo\n  include Bar\n\n  def baz; end\nend\n",
        "example.rb:2:3: C: Add an empty line after module inclusion.",
    ),
    (
        ACCESSOR,
        "class Foo\n  attr_reader :bar\n  def baz; end\nend\n",
        "class Foo\n  attr_reader :bar\n\n  def baz; end\nend\n",
        "example.rb:2:3: C: Add an empty line after attribute accessor.",
    ),
    (
        MODIFIER,
        "class Foo\n  def bar; end\n  private\n  def baz; end\nend\n",
        "class Foo\n  def bar; end\n\n  private\n\n  def baz; end\nend\n",
        "example.rb:3:3: C: Keep a blank line before and after `private`.",
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

/// The native Ruby 3.4 result of `example.rb`, when `RUSTOCOP_RUBY34` is
/// configured.
fn ruby(root: &Path) -> Option<String> {
    let ruby = crate::support::runtime::ruby().ok()?;
    let run = Command::new(ruby)
        .current_dir(root)
        .args(["--parser=prism", "-W0", "example.rb"])
        .output()
        .unwrap();
    assert!(run.status.success(), "{run:?}");
    Some(String::from_utf8_lossy(&run.stdout).into_owned())
}

#[test]
fn metadata_defaults_selectors_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    for &(cop, before, _, finding) in WITNESSES {
        let found = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(found.default_enabled() && found.safe_fixes(), "{cop}");
        assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
        assert!(found.path_policy().is_none() && found.max_setting().is_none());
        assert!(found.autocorrect_incompatible_with().is_empty());
        assert!(found.department_selection_enables());
        assert_eq!(found.ruby_versions(), Default::default());
        for (config, selectors, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Layout"], 1),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (String::new(), vec!["--except", "Layout"], 0),
            (String::new(), vec!["--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            ("[cops.Layout]\nenabled = false\n".into(), vec![], 0),
            (
                format!("[cops.Layout]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(dir.path(), before.as_bytes(), &selectors, None);
            assert_eq!(count(&out, cop), n, "{cop} {selectors:?} {out:?}");
            if n == 1 {
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(text.contains(finding), "{text}");
            }
        }
        // No public style, allowance or count knob exists.
        for key in [
            "AllowAliasSyntax = false",
            "AllowedMethods = []",
            "EnforcedStyle = \"only_before\"",
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
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    // An unknown lookalike name is rejected, never admitted.
    let out = unchanged(
        dir.path(),
        WITNESSES[0].1.as_bytes(),
        &["--only", "Layout/EmptyLinesAfterModuleInclusions"],
        Some(2),
    );
    assert_eq!(count(&out, INCLUSION), 0);
}

#[test]
fn lint_never_writes_and_fix_writes_the_canonical_edit_once() {
    let dir = project();
    for &(cop, before, after, _) in WITNESSES {
        let out = unchanged(dir.path(), before.as_bytes(), &["--only", cop], Some(1));
        assert_eq!(count(&out, cop), 1, "{out:?}");
        let (out, fixed) = fix(dir.path(), before.as_bytes(), &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, after.as_bytes(), "{cop}");
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", cop, "--fix"],
            Some(0),
        );
        // CRLF line endings and a UTF-8 BOM are preserved around the edit.
        let (_, fixed) = fix(
            dir.path(),
            before.replace('\n', "\r\n").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(fixed, after.replace('\n', "\r\n").as_bytes(), "{cop}");
        let (_, fixed) = fix(
            dir.path(),
            format!("\u{feff}{before}").as_bytes(),
            &["--only", cop],
        );
        assert_eq!(fixed, format!("\u{feff}{after}").as_bytes(), "{cop}");
        // A legacy-encoded comment keeps its bytes.
        let mut legacy = b"# encoding: ISO-8859-15\n# \xa4\n".to_vec();
        legacy.extend_from_slice(before.as_bytes());
        let mut legacy_after = b"# encoding: ISO-8859-15\n# \xa4\n".to_vec();
        legacy_after.extend_from_slice(after.as_bytes());
        let (out, fixed) = fix(dir.path(), &legacy, &["--only", cop]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fixed, legacy_after, "{cop}");
        // Each listed source observer keeps the finding and the file.
        for observer in [
            "p __FILE__",
            "p __dir__",
            "p $0",
            "p $PROGRAM_NAME",
            "p caller",
            "p caller_locations",
            "p method(:p).source_location",
            "p SCRIPT_LINES__",
            "p __LINE__",
        ] {
            let held = format!("{before}{observer}\n");
            let out = unchanged(
                dir.path(),
                held.as_bytes(),
                &["--only", cop, "--fix"],
                Some(1),
            );
            assert_eq!(count(&out, cop), 1, "{observer} {out:?}");
        }
        let held = format!("{before}p DATA.read\n__END__\nx\n");
        let out = unchanged(
            dir.path(),
            held.as_bytes(),
            &["--only", cop, "--fix"],
            Some(1),
        );
        assert_eq!(count(&out, cop), 1, "{out:?}");
    }
}

#[test]
fn directives_stdin_and_parse_errors() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for (cop, source) in [
            (
                INCLUSION,
                format!("include A # {prefix}:disable {INCLUSION}\nx\n"),
            ),
            (
                INCLUSION,
                format!("# {prefix}:disable Layout\ninclude A\nx\n"),
            ),
            (
                ACCESSOR,
                format!("attr_reader :a # {prefix}:disable {ACCESSOR}\nx\n"),
            ),
            (
                ACCESSOR,
                format!("# {prefix}:disable {ACCESSOR}\nattr_reader :a\nx\n"),
            ),
            (
                MODIFIER,
                format!("x\nprivate # {prefix}:disable {MODIFIER}\ny\n"),
            ),
            (
                MODIFIER,
                format!("# {prefix}:disable Layout\nx\nprotected\ny\n"),
            ),
            // An enable directive followed by a blank line separates.
            (
                INCLUSION,
                format!(
                    "# {prefix}:disable Style/Foo\ninclude A\n# {prefix}:enable Style/Foo\n\nx\n"
                ),
            ),
            (
                ACCESSOR,
                format!(
                    "# {prefix}:disable Style/Foo\nattr_reader :a\n# {prefix}:enable Style/Foo\n\nx\n"
                ),
            ),
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out, cop), 0, "{source:?}");
        }
        // The insertion goes after the enable directive in either spelling.
        for (cop, declaration) in [(INCLUSION, "include A"), (ACCESSOR, "attr_reader :a")] {
            let source = format!(
                "# {prefix}:disable Style/Foo\n{declaration}\n# {prefix}:enable Style/Foo\nx\n"
            );
            let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", cop]);
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            assert_eq!(
                String::from_utf8_lossy(&fixed),
                format!(
                    "# {prefix}:disable Style/Foo\n{declaration}\n# {prefix}:enable Style/Foo\n\nx\n"
                )
            );
        }
    }
    // stdin reports at the supplied path and never writes.
    for (cop, source, expected) in [
        (INCLUSION, "include A\nx\n", "virtual.rb:1:1: C: "),
        (ACCESSOR, "attr :a\nx\n", "virtual.rb:1:1: C: "),
        (MODIFIER, "x\npublic\ny\n", "virtual.rb:2:1: C: "),
    ] {
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
            .write_all(source.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains(expected), "{text}");
        assert!(!dir.path().join("virtual.rb").exists());
    }
    // An invalid file is an error, never a finding or a write.
    for &(cop, before, _, _) in WITNESSES {
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

/// The insertions keep the program's result; held edits keep the file.
#[test]
fn insertions_keep_meaning() {
    let dir = project();
    let selection = format!("{INCLUSION},{ACCESSOR}");
    for (source, fixed, result) in [
        (
            "module M\n  def m = :m\nend\nclass C\n  include M\n  attr_accessor :a\n  def initialize\n    @a = 1\n  end\nend\nc = C.new\np [c.m, c.a]\n",
            Some(
                "module M\n  def m = :m\nend\nclass C\n  include M\n\n  attr_accessor :a\n\n  def initialize\n    @a = 1\n  end\nend\nc = C.new\np [c.m, c.a]\n",
            ),
            "[:m, 1]\n",
        ),
        // A sibling on the accessor's line is accepted; the data section
        // after `__END__` is never touched.
        (
            "class C\n  attr_reader :a; X = \"x\ny\"\n  p X\nend\n",
            None,
            "\"x\\ny\"\n",
        ),
        (
            "class C\n  attr_reader :a\n  p 1\nend\n__END__\nattr_reader :b\nx\n",
            Some("class C\n  attr_reader :a\n\n  p 1\nend\n__END__\nattr_reader :b\nx\n"),
            "1\n",
        ),
        // The heredoc body follows the declaration's line: held.
        (
            "class C\n  attr_reader :a, <<~X.strip.to_sym\n    b\n  X\n  p instance_methods(false).sort\nend\n",
            None,
            "[:a, :b]\n",
        ),
    ] {
        fs::write(dir.path().join("example.rb"), source).unwrap();
        if let Some(before) = ruby(dir.path()) {
            assert_eq!(before, result, "{source:?}");
        }
        match fixed {
            Some(fixed) => {
                let (out, after) = fix(dir.path(), source.as_bytes(), &["--only", &selection]);
                assert_eq!(out.status.code(), Some(0), "{out:?}");
                assert_eq!(String::from_utf8_lossy(&after), fixed);
            }
            None => {
                let out = unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", &selection, "--fix"],
                    None,
                );
                // Held with the finding, or (same-line sibling) no finding.
                assert_eq!(
                    out.status.code(),
                    Some(count(&out, ACCESSOR) as i32),
                    "{out:?}"
                );
            }
        }
        if let Some(after) = ruby(dir.path()) {
            assert_eq!(after, result, "{source:?}");
        }
    }
}

/// Inclusion and accessor insertions apply together in one pass with the
/// other line-break cops, and the default selection converges without
/// changing the program's result.
#[test]
fn declaration_insertions_compose() {
    let dir = project();
    let (out, fixed) = fix(
        dir.path(),
        b"class C\n  include Comparable\n  attr_reader :a\n  def f(x)\n    return 1 if x\n    2\n  end\nend\n",
        &[
            "--only",
            "Layout/EmptyLinesAfterModuleInclusion,Layout/EmptyLinesAroundAttributeAccessor,Layout/EmptyLineAfterGuardClause",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&fixed),
        "class C\n  include Comparable\n\n  attr_reader :a\n\n  def f(x)\n    return 1 if x\n\n    2\n  end\nend\n"
    );
    let source = b"# frozen_string_literal: true\nclass C\n  include Comparable\n  extend Enumerable\n  attr_accessor :a\n  attr_writer :b\n  alias c a\n  def initialize = @a = 1\n  def <=>(other) = a <=> other.a\nend\np C.new.a\n";
    let path = dir.path().join("example.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    if let Some(before) = &before {
        assert_eq!(before, "1\n");
    }
    let mut current = source.to_vec();
    for _ in 0..4 {
        let (_, fixed) = fix(dir.path(), &current, &[]);
        assert_eq!(ruby(dir.path()), before);
        if fixed == current {
            break;
        }
        current = fixed;
    }
    let out = unchanged(dir.path(), &current, &["--fix"], None);
    assert_eq!(count(&out, INCLUSION) + count(&out, ACCESSOR), 0, "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&current),
        "# frozen_string_literal: true\n\nclass C\n  include Comparable\n  extend Enumerable\n\n  attr_accessor :a\n  attr_writer :b\n  alias c a\n  def initialize = @a = 1\n  def <=>(other) = a <=> other.a\nend\np C.new.a\n"
    );
}

/// The access modifier's state is per file and per pass: a class in one
/// file never reaches another, and each fix pass re-reads the new lines.
#[test]
fn access_modifier_state_is_per_file_and_pass() {
    let dir = project();
    let class_first = "class A\nend\n";
    let modifier = "x\nprivate\ny\n";
    for (first, second) in [("a.rb", "b.rb"), ("b.rb", "a.rb")] {
        fs::write(dir.path().join(first), class_first).unwrap();
        fs::write(dir.path().join(second), modifier).unwrap();
        let out = run(dir.path(), &["--only", MODIFIER, "a.rb", "b.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains(&format!(
                "{second}:2:1: C: Keep a blank line before and after `private`."
            )),
            "{text}"
        );
        assert_eq!(count(&out, MODIFIER), 1, "{text}");
        fs::remove_file(dir.path().join(first)).unwrap();
        fs::remove_file(dir.path().join(second)).unwrap();
    }
    // Two modifiers corrected in one pass; the next pass finds nothing.
    let source = "class A\n  x\n  protected\n  def a; end\n\n  y\n  private\n  def b; end\nend\n";
    let (out, fixed) = fix(dir.path(), source.as_bytes(), &["--only", MODIFIER]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&fixed),
        "class A\n  x\n\n  protected\n\n  def a; end\n\n  y\n\n  private\n\n  def b; end\nend\n"
    );
    // A block's sole modifier keeps its finding without an edit.
    let out = unchanged(
        dir.path(),
        b"Module.new do\n  private\nend\n",
        &["--only", MODIFIER, "--fix"],
        Some(1),
    );
    assert_eq!(count(&out, MODIFIER), 1, "{out:?}");
}

/// The three cops and the other default-on Layout cops correct a class
/// together; the default selection converges and the program's result is
/// unchanged.
#[test]
fn class_layout_converges_with_the_default_selection() {
    let dir = project();
    let source = "class C\n  include Comparable\n  attr_reader :a\n  def initialize(a)\n    @a = a\n  end\n  def <=>(other)\n    a <=> other.a\n  end\n  protected\n  def x\n    1\n  end\n  private\n  def y = 2\nend\np C.new(1) < C.new(2)\n";
    let path = dir.path().join("example.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    if let Some(before) = &before {
        assert_eq!(before, "true\n");
    }
    let mut current = source.as_bytes().to_vec();
    for _ in 0..4 {
        let (_, fixed) = fix(dir.path(), &current, &[]);
        assert_eq!(ruby(dir.path()), before);
        if fixed == current {
            break;
        }
        current = fixed;
    }
    let out = unchanged(dir.path(), &current, &["--fix"], None);
    assert_eq!(
        count(&out, INCLUSION) + count(&out, ACCESSOR) + count(&out, MODIFIER),
        0,
        "{out:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&current),
        "class C\n  include Comparable\n\n  attr_reader :a\n\n  def initialize(a)\n    @a = a\n  end\n\n  def <=>(other)\n    a <=> other.a\n  end\n\n  protected\n\n  def x\n    1\n  end\n\n  private\n\n  def y = 2\nend\np C.new(1) < C.new(2)\n"
    );
    // The same insertions in one pass when only the three cops run.
    let (out, fixed) = fix(
        dir.path(),
        source.as_bytes(),
        &["--only", &format!("{INCLUSION},{ACCESSOR},{MODIFIER}")],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&fixed),
        "class C\n  include Comparable\n\n  attr_reader :a\n\n  def initialize(a)\n    @a = a\n  end\n  def <=>(other)\n    a <=> other.a\n  end\n\n  protected\n\n  def x\n    1\n  end\n\n  private\n\n  def y = 2\nend\np C.new(1) < C.new(2)\n"
    );
}
