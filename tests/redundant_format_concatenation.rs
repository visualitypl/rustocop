//! detection-only rules. Only controlled ordinary Ruby witnesses run;
//! command literals and command heredocs below are NEVER executed.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const FORMAT: &str = "Style/RedundantFormat";
const CONCAT: &str = "Style/StringConcatenation";
const BOTH: &str = "Style/RedundantFormat,Style/StringConcatenation";
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
fn default_enabled_detection_only_exact_positions_and_stdin() {
    let registry = rustocop::cops::Registry::new();
    for name in [FORMAT, CONCAT] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled());
        assert!(!cop.safe_fixes());
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
    }
    let d = project();
    let source = b"format(\"hello\")\nx + \"y\"\n";
    for args in [
        vec!["a.rb"],
        vec!["--fix", "a.rb"],
        vec!["--only", BOTH, "a.rb"],
        vec!["--only", BOTH, "--fix", "a.rb"],
    ] {
        let o = unchanged(d.path(), source, &args, 1);
        let stdout = String::from_utf8_lossy(&o.stdout);
        assert!(stdout.contains(
            "a.rb:1:1: C: Use `\"hello\"` directly instead of `format`. (Style/RedundantFormat)"
        ));
        assert!(stdout.contains("a.rb:2:1: C: Prefer string interpolation to string concatenation. (Style/StringConcatenation)"));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", BOTH, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), source);
}
#[test]
fn selectors_config_enable_disable_excludes_and_strict_policy() {
    let d = project();
    let source = b"format(\"hello\")\nx + \"y\"\n";
    for cop in [FORMAT, CONCAT] {
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let out = unchanged(
            d.path(),
            source,
            &["--only", BOTH, "--except", cop, "--fix", "a.rb"],
            1,
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!stdout.contains(&format!("({cop})")));
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        let out = unchanged(d.path(), source, &["--fix", "a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        fs::write(
            d.path().join("rustocop.toml"),
            "[cops.Style]\nenabled = false\n",
        )
        .unwrap();
        unchanged(d.path(), source, &["--fix", "a.rb"], 0);
        unchanged(d.path(), source, &["--only", cop, "--fix", "a.rb"], 1);
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        )
        .unwrap();
        let out = unchanged(d.path(), source, &["--fix", "a.rb"], 1);
        assert!(!String::from_utf8_lossy(&out.stdout).contains(&format!("({cop})")));
        unchanged(d.path(), source, &["--only", cop, "--fix", "a.rb"], 1);
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        )
        .unwrap();
        unchanged(d.path(), source, &["--only", cop, "--fix", "a.rb"], 0);
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nMode = \"conservative\"\n"),
        )
        .unwrap();
        unchanged(d.path(), source, &["--only", cop, "--fix", "a.rb"], 2);
    }
    fs::write(d.path().join("rustocop.toml"), "exclude = [\"a.rb\"]\n").unwrap();
    unchanged(d.path(), source, &["--only", BOTH, "--fix", "a.rb"], 0);
}
#[test]
fn directives_multiline_ranges_and_error_files_never_write() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for (cop, body) in [(FORMAT, "format(\"hello\")"), (CONCAT, "x + \"y\"")] {
            for target in [cop, cop.split('/').nth(1).unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{body}\n"),
                    format!("# {prefix}:disable-next {target}\n{body}\n"),
                    format!("{body} # {prefix}:disable {target}\n"),
                ] {
                    unchanged(
                        d.path(),
                        s.as_bytes(),
                        &["--only", BOTH, "--fix", "a.rb"],
                        0,
                    );
                }
            }
        }
        // Any disabled line of a full expression suppresses it, but a block's
        // body is not part of a send's offense range.
        let s = format!("x +\n  \"y\" # {prefix}:disable {CONCAT}\n");
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", BOTH, "--fix", "a.rb"],
            0,
        );
        let s = format!("format('x') do\n # {prefix}:disable {FORMAT}\nend\n");
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", BOTH, "--fix", "a.rb"],
            1,
        );
    }
    for (s, status) in [
        ("format('%%')\nx+y\n", 0),
        ("format(\n", 2),
        ("\"#{x}\" + y\n", 0),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", BOTH, "--fix", "a.rb"],
            status,
        );
    }
}
#[test]
fn literal_encodings_bom_crlf_data_commands_and_frozen_mutable_modes() {
    let d = project();
    for source in [
        b"\xef\xbb\xbfformat(\"hello\")\r\nx + \"y\"\r\np __LINE__\r\n__END__\r\nformat('data')\r\n".as_slice(),
        b"# encoding: ISO-8859-1\nformat('\xe9')\nx + '\xe9'\n",
        b"# encoding: Windows-31J\nformat('\x83\\')\nx + '\x83\\'\n",
        b"# frozen_string_literal: true\nformat('hello').concat('!')\nx + 'y'\n",
        b"# frozen_string_literal: false\nformat('hello')\nx + 'y'\n",
        b"# frozen_string_literal: true\nformat('%s', nil)\nformat('%2$s %1$s', 'a', 'b')\n",
        b"format(<<~MSG)\n hello\nMSG\n\"x\" + <<~TEXT\n hello\nTEXT\n",
        b"`echo ignored` + 'x'\nformat(%x(echo ignored))\n",
        b"'x' + <<~`CMD`\n echo ignored\nCMD\n",
    ] {
        unchanged(d.path(),source,&["--only",BOTH,"--fix","a.rb"],1);
    }
}
#[test]
fn unsafe_dispatch_and_argument_effects_are_reported_not_rewritten() {
    let d = project();
    for source in [
        "class String; def +(other); :custom; end; end\n'x' + 'y'\n",
        "Pathname.new('/') + 'test'\n",
        "def format(s); [s]; end\nformat('hello')\n",
        "format('%s', \"#{effect}\")\n",
        "format('%2$s %1$s', \"#{first}\", \"#{second}\")\n",
        "Other.format('%d', 5)\n",
        "module M; Kernel = Object.new; Kernel.format('hello'); end\n",
        "format(CONSTANT)\n",
        "'a' + (condition ? 'b' : 'c')\n",
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", BOTH, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn imaginary_zero_cli_detection_and_no_write() {
    let d = project();
    let inexact = b"format(\"%d\", 0.0i)\nformat(\"%f\", -0.0i)\nformat(\"%<x>d\", x: (0.0i))\nformat(\"%f\", (5+0.0i))\nformat(\"%*.*f\", 5, -0.0i, 1)\n";
    let out = unchanged(d.path(), inexact, &["--only", BOTH, "--fix", "a.rb"], 0);
    assert!(out.stderr.is_empty(), "{out:?}");
    // RuboCop 1.92.0 raises on `Complex("0.0ri")`, so line 2 has no offense.
    let exact = b"format(\"%d\", 0i)\nformat(\"%f\", 0.0ri)\nx + \"y\"\n";
    let out = unchanged(d.path(), exact, &["--only", BOTH, "--fix", "a.rb"], 1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(
        "a.rb:1:1: C: Use `\"0\"` directly instead of `format`. (Style/RedundantFormat)"
    ));
    assert!(!stdout.contains("a.rb:2:1"));
    assert!(stdout.contains("a.rb:3:1: C: Prefer string interpolation to string concatenation. (Style/StringConcatenation)"));
    assert!(out.stderr.is_empty(), "{out:?}");
}
#[test]
fn controlled_ruby_complex_conversion_requires_exact_imaginary_zero() {
    if !Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    // Controlled numeric primitives only; host runtime evidence is separate
    // from the fixed Ruby 3.4 Prism/reference-cop comparison.
    let source = r#"
[0.0i, -0.0i, 0e2i, (5+0.0i), (5+-0.0i)].each do |value|
  raise 'not a float imaginary part' unless value.imaginary.is_a?(Float)
  raise 'Integer accepted inexact zero' unless Integer(value, exception: false).nil?
  raise 'Float accepted inexact zero' unless Float(value, exception: false).nil?
  begin
    format('%d', value)
    raise 'format accepted inexact zero'
  rescue RangeError
  end
end
[0i, -0i, 0ri, 0.0ri, (5+0i), (5+0ri)].each do |value|
  raise 'Integer rejected exact zero' unless Integer(value, exception: false)
  raise 'Float rejected exact zero' unless Float(value, exception: false)
  format('%d', value)
  format('%f', value)
end
"#;
    let out = Command::new("ruby").args(["-e", source]).output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}
#[test]
fn controlled_ruby_witnesses_explain_detection_only_contract() {
    if !Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    let d = project();
    // This is a fixed, controlled ordinary-Ruby fixture, never corpus code.
    let source = r##"# frozen_string_literal: true
require 'pathname'
events = []
fmt = format('hello')
fmt << '!'
p [fmt.bytes, fmt.frozen?, 'hello'.frozen?]
p [format('%%').bytes, '%%'.bytes]
p [format('%2$s %1$s', "#{events << 1; 'first'}", "#{events << 2; 'second'}"), events]
p [(Pathname.new('/a') + 'b').to_s, "#{Pathname.new('/a')}b"]
p [format('%s', nil).bytes, format('%*.*f', 5, 2, 0.5), __LINE__]
"##;
    let before = Command::new("ruby").arg("-e").arg(source).output().unwrap();
    assert!(before.status.success(), "{before:?}");
    assert_eq!(
        String::from_utf8(before.stdout.clone()).unwrap(),
        "[[104, 101, 108, 108, 111, 33], false, true]\n[[37], [37, 37]]\n[\"second first\", [1, 2]]\n[\"/a/b\", \"/ab\"]\n[[], \" 0.50\", 10]\n"
    );
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", BOTH, "--fix", "a.rb"],
        1,
    );
    let after = Command::new("ruby")
        .arg("-e")
        .arg(String::from_utf8(fs::read(d.path().join("a.rb")).unwrap()).unwrap())
        .output()
        .unwrap();
    assert_eq!(
        (after.status.code(), after.stdout, after.stderr),
        (before.status.code(), before.stdout, before.stderr)
    );
}
#[test]
fn safe_peers_can_correct_without_unsafe_string_rewrites() {
    let d = project();
    let source = b"format('hello'); x + 'y'\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    cli(
        d.path(),
        &[
            "--only",
            "Style/RedundantFormat,Style/StringConcatenation,Style/StringLiterals,Style/FormatString,Style/Semicolon,Layout",
            "--fix",
            "--fix-passes",
            "1",
            "a.rb",
        ],
        1,
    );
    assert_eq!(
        fs::read(d.path().join("a.rb")).unwrap(),
        b"format(\"hello\")\n x + \"y\"\n"
    );
    for _ in 0..5 {
        cli(
            d.path(),
            &[
                "--only",
                "Style/RedundantFormat,Style/StringConcatenation,Style/StringLiterals,Style/FormatString,Style/Semicolon,Layout",
                "--fix",
                "--fix-passes",
                "1",
                "a.rb",
            ],
            1,
        );
    }
    // Layout/IndentationConsistency aligns the split statement.
    assert_eq!(
        fs::read(d.path().join("a.rb")).unwrap(),
        b"format(\"hello\")\nx + \"y\"\n"
    );
    unchanged(
        d.path(),
        b"format(\"hello\")\nx + \"y\"\n",
        &["--only", BOTH, "--fix", "--fix-passes", "1", "a.rb"],
        1,
    );
}
