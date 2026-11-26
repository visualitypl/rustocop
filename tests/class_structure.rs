//! Actual CLI lifecycle for the opt-in, detection-only
//! Layout/ClassStructure: exact-name selection only, strict TOML, both
//! directive prefixes, stdin, BOM/CRLF/legacy encodings, `DATA`, parse
//! errors, and no write under `--fix` (bytes, mtime, permissions and inode
//! kept), alone and composed with correcting cops.
use crate::support::cli::{project, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Layout/ClassStructure";

const BEFORE: &str = "class Person\n  def name; end\n  include Comparable\nend\n";

const FINDING: &str =
    "example.rb:3:3: C: `module_inclusion` is supposed to appear before `public_methods`.";

fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({COP})"))
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

#[test]
fn opt_in_by_exact_name_only_with_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    let found = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(!found.default_enabled() && !found.safe_fixes());
    assert!(!found.department_selection_enables());
    assert_eq!(found.severity(), rustocop::offense::Severity::Convention);
    assert!(found.path_policy().is_none() && found.max_setting().is_none());
    assert!(found.autocorrect_incompatible_with().is_empty());
    assert_eq!(found.ruby_versions(), Default::default());
    let dir = project();
    let enabled = format!("[cops.\"{COP}\"]\nenabled = true\n");
    for (config, selectors, n) in [
        (String::new(), vec![], 0),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Layout"], 0),
        (enabled.clone(), vec!["--only", "Layout"], 1),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Layout"], 0),
        (enabled.clone(), vec!["--except", "Layout"], 0),
        (enabled.clone(), vec![], 1),
        ("[cops.Layout]\nenabled = true\n".into(), vec![], 0),
        (
            format!("[cops.Layout]\nenabled = false\n{enabled}"),
            vec![],
            1,
        ),
        (format!("[cops.\"{COP}\"]\nenabled = false\n"), vec![], 0),
        (
            format!("[cops.\"{COP}\"]\nenabled = true\nexclude = [\"example.rb\"]\n"),
            vec![],
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), &config).unwrap();
        let out = unchanged(dir.path(), BEFORE.as_bytes(), &selectors, None);
        assert_eq!(count(&out), n, "{config:?} {selectors:?} {out:?}");
        if n == 1 {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains(FINDING), "{text}");
        }
    }
    // No public order, category, safety or count knob exists.
    for key in [
        "ExpectedOrder = [\"constants\"]",
        "Categories = { module_inclusion = [\"include\"] }",
        "SafeAutoCorrect = true",
        "Enabled = true",
        "max = 1",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            BEFORE.as_bytes(),
            &["--only", COP, "--fix"],
            Some(2),
        );
    }
}

/// Upstream's reorder is unsafe: `--fix` never moves an element, whatever the
/// source holds, and the finding stays.
#[test]
fn fix_never_writes_and_keeps_every_finding() {
    let dir = project();
    let mut latin1 = b"# encoding: ISO-8859-1\n# caf".to_vec();
    latin1.push(0xe9);
    latin1.push(b'\n');
    latin1.extend_from_slice(BEFORE.as_bytes());
    for source in [
        BEFORE.as_bytes().to_vec(),
        BEFORE.replace('\n', "\r\n").into_bytes(),
        format!("\u{feff}{BEFORE}").into_bytes(),
        latin1,
        format!("{BEFORE}p DATA.read\n__END__\ndata\n").into_bytes(),
        format!("{BEFORE}p __FILE__, $0, caller\n").into_bytes(),
        // Literal and dynamic constants and visibility sections.
        b"class A\n  def a; end\n  X = foo.freeze\n  Y = 1\n  private\n  include M\nend\n".to_vec(),
        b"class << self\n  private\n  def a; end\n  public\n  def b; end\nend\n".to_vec(),
    ] {
        for selectors in [vec!["--only", COP, "--fix"], vec!["--only", COP]] {
            let out = unchanged(dir.path(), &source, &selectors, Some(1));
            assert!(count(&out) >= 1, "{source:?} {out:?}");
        }
    }
    // Composed with the default correcting cops, the others edit and the
    // order stays.
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    let path = dir.path().join("example.rb");
    let source = "# frozen_string_literal: true\n\n# Doc.\nclass Person\n  def name; end   \n  include Comparable\nend\n";
    fs::write(&path, source).unwrap();
    let out = run(dir.path(), &["--fix", "example.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out), 1, "{out:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("end   \n", "end\n")
    );
}

#[test]
fn directives_stdin_and_parse_errors() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{BEFORE}"),
            format!("# {prefix}:disable Layout\n{BEFORE}"),
            BEFORE.replace(
                "include Comparable",
                &format!("include Comparable # {prefix}:disable {COP}"),
            ),
            format!(
                "class Person\n  def name; end\n  # {prefix}:disable {COP}\n  include Comparable\n  # {prefix}:enable {COP}\nend\n"
            ),
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix"],
                Some(0),
            );
            assert_eq!(count(&out), 0, "{source:?} {out:?}");
        }
        // A directive on another line does not cover the element.
        let source = BEFORE.replace(
            "def name; end",
            &format!("def name; end # {prefix}:disable {COP}"),
        );
        let out = unchanged(dir.path(), source.as_bytes(), &["--only", COP], Some(1));
        assert_eq!(count(&out), 1, "{out:?}");
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--stdin", "virtual.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(BEFORE.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(&FINDING.replace("example.rb", "virtual.rb")),
        "{text}"
    );
    // An invalid file is an error, never a finding or a write.
    let invalid = format!("{BEFORE}def\n");
    let out = unchanged(
        dir.path(),
        invalid.as_bytes(),
        &["--only", COP, "--fix"],
        Some(2),
    );
    assert_eq!(count(&out), 0, "{out:?}");
}

/// Many classes and elements are checked in one linear pass.
#[test]
fn bounded_many_class_file() {
    let dir = project();
    let mut source = String::new();
    for i in 0..300 {
        source.push_str(&format!(
            "class C{i}\n  def a; end\n  X = 1\n  begin\n    include M\n  end\n  def self.b; end\nend\n"
        ));
    }
    let out = unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", COP, "--fix"],
        Some(1),
    );
    assert_eq!(count(&out), 900, "{out:?}");
}
