//! CLI controls, pinned corrections and controlled Ruby witnesses.
//! RuboCop fixtures: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const COP: &str = "Style/ParallelAssignment";
const BEFORE: &str = "a,b=1,2\n";
const AFTER: &str = "a = 1\nb = 2\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], count: usize) {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(root, args);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(COP))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), time);
}
fn fix(root: &Path, before: &str, after: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let o = cli(root, &["--only", COP, "--fix", "a.rb"]);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        fs::read_to_string(root.join("a.rb")).unwrap(),
        after,
        "{o:?}"
    );
}
#[test]
fn defaults_selectors_and_config_never_implicitly_write() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    let rule = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(rule.default_enabled() && rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    for (config, selectors, count) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Style", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (format!("[cops.\"{COP}\"]\nenabled=false\n"), vec![], 0),
        (
            format!("[cops.\"{COP}\"]\nenabled=false\n"),
            vec!["--only", COP],
            1,
        ),
        (
            format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
            vec![],
            1,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("a.rb");
        unchanged(d.path(), BEFORE.as_bytes(), &args, count);
    }
}
#[test]
fn excludes_and_directives_protect_whole_corrections() {
    let d = project();
    for config in [
        "exclude=['a.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=['a.rb']\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            BEFORE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            let source = format!("# {prefix}:disable {target}\n{BEFORE}");
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
        fix(
            d.path(),
            &format!("# {prefix}:disable {COP}\n{BEFORE}# {prefix}:enable {COP}\n{BEFORE}"),
            &format!("# {prefix}:disable {COP}\n{BEFORE}# {prefix}:enable {COP}\n{AFTER}"),
        );
        let source = format!("a,b=1,2 # {prefix}:disable Lint/Debugger\n");
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn all_forty_eight_pinned_correction_calls_are_individually_accounted_for() {
    let d = project();
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/parallel_assignment_pinned.json")).unwrap();
    let canonical = [
        7, 20, 83, 107, 256, 269, 282, 295, 307, 319, 384, 435, 467, 483, 497, 513, 527, 543, 557,
        625,
    ];
    let held = [
        33, 46, 59, 71, 95, 122, 134, 146, 159, 172, 184, 196, 332, 344, 356, 368, 397, 414, 449,
        573, 589, 605, 645, 663, 681, 703, 725, 739,
    ];
    let mut applied = Vec::new();
    let mut withheld = Vec::new();
    for c in cases {
        let Some(after) = c["correction"].as_str() else {
            continue;
        };
        let before = c["source"].as_str().unwrap();
        let line = c["spec_line"].as_u64().unwrap();
        if canonical.contains(&line) {
            fix(d.path(), before, after);
            unchanged(
                d.path(),
                after.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
            applied.push(line);
        } else {
            assert!(held.contains(&line), "unaccounted spec line {line}");
            unchanged(
                d.path(),
                before.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                1,
            );
            withheld.push(line);
        }
    }
    assert_eq!(applied, canonical);
    assert_eq!(withheld, held);
}
#[test]
fn bom_crlf_encoding_comments_lines_and_data_envelopes() {
    let d = project();
    fix(
        d.path(),
        "\u{feff}# coding: UTF-8\r\na,b=1,2\r\n__END__\r\npayload é\r\n",
        "\u{feff}# coding: UTF-8\r\na = 1\r\nb = 2\r\n__END__\r\npayload é\r\n",
    );
    fix(
        d.path(),
        "# coding: ISO-8859-1\na,b=1,2\n",
        "# coding: ISO-8859-1\na = 1\nb = 2\n",
    );
    fix(
        d.path(),
        "a,b=1,2 # attached comment\n",
        "a = 1\nb = 2 # attached comment\n",
    );
    for source in [
        b"a,b=1,2\np __LINE__\n".as_slice(),
        b"a,b=__LINE__,2\n".as_slice(),
        b"a,b=1,2\np DATA.pos\n__END__\npayload\n".as_slice(),
        b"a,b=[1, # comment\n2],3\n".as_slice(),
        b"# coding: ISO-8859-1\na,b='\xe9',2\n".as_slice(),
        b"# coding: ISO-8859-1\na,b=%W(\\u{e9} x)\n".as_slice(),
        b"a,b=%W(\\r x)\n".as_slice(),
    ] {
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    }
}
#[test]
fn clean_unsafe_and_error_files_are_never_rewritten() {
    let d = project();
    for (source, count) in [
        ("a,b=b,a\n", 0),
        ("a,b=foo(),bar()\n", 1),
        ("def f\n a,b=1,2\nend\n", 1),
        ("a,b=1,raise('no')\n", 1),
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            count,
        );
    }
    fs::write(d.path().join("a.rb"), "a,b=1,2\ndef\n").unwrap();
    let old = fs::read(d.path().join("a.rb")).unwrap();
    let time = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        cli(d.path(), &["--only", COP, "--fix", "a.rb"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), old);
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        time
    );
}
#[test]
fn stdin_has_exact_positions_and_never_writes() {
    let d = project();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"  a,b=1,2\n")
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        text.contains("a.rb:1:3:")
            && text.contains(COP)
            && text.contains("Do not use parallel assignment."),
        "{text}"
    );
    assert!(!d.path().join("a.rb").exists());
}
#[test]
fn controlled_evaluation_binding_setter_return_rescue_and_result_witnesses() {
    let source = r#"$trace = []
def rhs(value)
  $trace << [:rhs, value]
  value
end
class Target
  def initialize; @a = :old; end
  def a; $trace << :get; @a; end
  def a=(v); $trace << [:set, v]; @a = v; :wrong_return; end
end
a = 8
a,b=1,a
p [a,b]
c,d=%w(it's fine)
p [c,d]
e,f=%i(foo-bar baz)
p [e,f]
g,h=1,2 if true
p [g,h]
i,j=1,2 unless false
p [i,j]
x,y=rhs(:left),rhs(:right)
p [x,y]
target = Target.new
p(target.a, x = rhs(:next), rhs(:last))
target.a,x=rhs(:setter),target.a
p [target.a,x]
p((x,y=1,false))
def result
  a,b=1,false
end
p result
captured = nil
2.times do |index|
  captured,local=index,2
  p [captured,local]
end
p captured
p 1.times.map { |; captured| captured,other=3,4 }
a,a=1,a
p a
begin
  a,b=rhs(:before_raise),raise('fail')
rescue RuntimeError
  p [a,b]
end
a,b=raise('fail'),2 rescue [:fallback, false]
p [a,b]
class Memo
  def initialize; @a = false; @b = nil; end
  def read
    @a,@b=(@a || :new),(@b || :memo)
    [@a,@b]
  end
end
p Memo.new.read
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", COP, "--fix", "a.rb"]);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    assert!(
        after.contains("b = a\na = 1")
            && after.contains("target.a,x=")
            && after.contains("a,a=1,a"),
        "{after}"
    );
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(r) = crate::support::runtime::ruby() {
        rubies.push(r);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby)
            .arg(d.path().join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby)
            .arg(d.path().join("a.rb"))
            .output()
            .unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
        assert_eq!(before.stderr, after.stderr, "{ruby}");
        let observed = String::from_utf8_lossy(&before.stdout);
        assert!(
            observed.contains("[:fallback, false]")
                && observed.contains("[1, false]")
                && observed.contains("[:new, :memo]"),
            "{observed}"
        );
    }
}
#[test]
fn mixed_style_layout_and_default_fixes_stabilize_within_eight_explicit_passes() {
    let d = project();
    let before = "a,b=[1,2],{ :key => 'value' };\np [a,b]\n";
    let selected = format!(
        "{COP},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    for selector in [Some(selected.as_str()), Some("Style,Layout"), None] {
        fs::write(d.path().join("a.rb"), before).unwrap();
        let mut stable = false;
        for _ in 0..8 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(s) = selector {
                args.extend(["--only", s]);
            }
            let o = cli(d.path(), &args);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if fs::read(d.path().join("a.rb")).unwrap() == old {
                stable = true;
                break;
            }
        }
        assert!(stable);
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(
            after.contains("a = [1, 2]") && after.contains("key: \"value\""),
            "{after}"
        );
    }
}
