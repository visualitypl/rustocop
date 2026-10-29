//! detection only; controlled Ruby witnesses are separate from the
//! fixed Ruby 3.4 parser/reference policy. Commands are NEVER executed.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/LineEndConcatenation";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    out
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    out
}
#[test]
fn default_detection_only_cli_stdin_and_exact_diagnostic() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    let d = project();
    let source = b"\"a\" +\n  \"b\"\n";
    for args in [
        vec!["a.rb"],
        vec!["--fix", "a.rb"],
        vec!["--only", COP, "a.rb"],
        vec!["--only", COP, "--fix", "a.rb"],
    ] {
        let out = unchanged(d.path(), source, &args, 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:1:5: C: Use `\\` instead of `+` to concatenate multiline strings. (Style/LineEndConcatenation)"));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), source);
}
#[test]
fn selectors_config_enabling_excludes_and_strict_options() {
    let d = project();
    let source = b"\"a\" +\n \"b\"\n";
    unchanged(
        d.path(),
        source,
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled = false\n".to_string(),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--fix", "a.rb"], 0);
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    unchanged(d.path(), source, &["--fix", "a.rb"], 1);
    for config in [
        "exclude = [\"a.rb\"]\n".to_string(),
        format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 0);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle = \"backslash\"\n"),
    )
    .unwrap();
    unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 2);
}
#[test]
fn directives_operator_line_ranges_and_no_write_clean_or_error() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "LineEndConcatenation", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n\"a\" +\n \"b\"\n"),
                format!("# {prefix}:disable-next {target}\n\"a\" +\n \"b\"\n"),
                format!("\"a\" + # {prefix}:disable {target}\n \"b\"\n"),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
        }
        // The report covers only the operator, not the whole concatenation.
        let source = format!("\"a\" +\n \"b\" # {prefix}:disable {COP}\n");
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        let source = format!(
            "# {prefix}:disable {COP}\n\"a\" +\n \"b\"\n# {prefix}:enable {COP}\n\"c\" <<\n \"d\"\n"
        );
        let out = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:5:5:"));
    }
    for (source, status) in [
        ("\"a\" + \"b\"\n", 0),
        ("\"a\" +\n \"b\".reverse\n", 0),
        ("\"a\" + (\n", 2),
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            status,
        );
    }
}
#[test]
fn legacy_encoding_bom_crlf_frozen_line_data_and_commands_are_unchanged() {
    let d = project();
    for source in [
        b"\xef\xbb\xbf\"a\" +\r\n \"b\"\r\np __LINE__\r\n__END__\r\n\"data\" +\r\n\"data\"\r\n"
            .as_slice(),
        b"# encoding: ISO-8859-1\n'\xe9' +\n '\xe8'\n",
        b"# encoding: Windows-31J\n'\x83\\' +\n '\x83\\'\n",
        b"# frozen_string_literal: true\n\"a\" <<\n \"b\"\n",
        b"# frozen_string_literal: false\n\"a\" <<\n \"b\"\n",
        b"# frozen_string_literal: chilled\n\"a\" +\n \"b\"\n",
        b"%x\"never executed\" <<\n \"b\"\n",
        b"consume(<<~TXT, \"a\" +\nbody\nTXT\n \"b\")\n",
        b"x = <<~TXT\n#{\"a\" +\n \"b\"}\nTXT\n",
    ] {
        let out = unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(COP),
            "{out:?}"
        );
        assert!(out.stderr.is_empty(), "{out:?}");
    }
    for source in [
        b"`ignored` +\n \"b\"\n".as_slice(),
        b"\"a\" + <<~`CMD`\nignored\nCMD\n",
    ] {
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 0);
    }
}
#[test]
fn independent_safe_sibling_and_incompatible_unsafe_peer() {
    let d = project();
    let source = b"\"#{value}\" <<\n  \"b\"\n'a'\n";
    let path = d.path().join("a.rb");
    fs::write(&path, source).unwrap();
    let out = cli(
        d.path(),
        &[
            "--only",
            "Style/LineEndConcatenation,Style/RedundantInterpolation,Style/StringLiterals",
            "--fix",
            "a.rb",
        ],
        1,
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"\"#{value}\" <<\n  \"b\"\n\"a\"\n"
    );
    // A second explicit pass has nothing safe left to do.
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    cli(
        d.path(),
        &[
            "--only",
            "Style/LineEndConcatenation,Style/RedundantInterpolation,Style/StringLiterals",
            "--fix",
            "a.rb",
        ],
        1,
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
}
#[test]
fn controlled_ruby_witnesses_explain_why_no_rewrite_is_safe() {
    if !Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    // Only these controlled ordinary Ruby expressions execute, never commands.
    let script = r##"
require 'json'
events = []
x = "a" +
  "#{events << :rhs; 'b'}"
raise unless x == 'ab' && events == [:rhs]
array = []
array << 'a' <<
  'b'
raise unless array == ['a', 'b'] # lexical predecessor does not prove String
# + gives a mutable result even with frozen operands; implicit literals don't.
plus = eval("# frozen_string_literal: true\n'a' +\n'b'")
implicit = eval("# frozen_string_literal: true\n'a' \\\n'b'")
raise unless plus == implicit && !plus.frozen? && implicit.frozen?
# << mutates and can raise, even though implicit concatenation parses cleanly.
begin
  eval("# frozen_string_literal: true\n'a' <<\n'b'")
  raise 'missing FrozenError'
rescue FrozenError
end
# Dispatch can be overridden; implicit concatenation does not call String#+.
class String
  alias original_plus +
  def +(other); [:custom_plus, self, other]; end
end
raise unless eval("'a' +\n'b'") == [:custom_plus, 'a', 'b']
raise unless eval("'a' \\\n'b'") == 'ab'
puts JSON.generate([events, array, plus, implicit, plus.encoding.name])
"##;
    let out = Command::new("ruby").args(["-e", script]).output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        "[[\"rhs\"],[\"a\",\"b\"],\"ab\",\"ab\",\"UTF-8\"]"
    );
}
