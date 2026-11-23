//! Execute only controlled ordinary Ruby fixtures; command literals
//! and pointer unpack directives are strictly source/AST-only.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/UnpackFirst";
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
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    out
}
fn fix(dir: &Path, source: &[u8], status: i32) -> Vec<u8> {
    fs::write(dir.join("a.rb"), source).unwrap();
    cli(dir, &["--only", COP, "--fix", "a.rb"], status);
    fs::read(dir.join("a.rb")).unwrap()
}
#[test]
fn real_defaults_ruby_gate_diagnostics_selectors_stdin_and_no_write() {
    let r = rustocop::cops::Registry::new();
    let cop = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(
        cop.ruby_versions().minimum,
        Some(rustocop::cops::policy::RubyVersion(2, 4))
    );
    let d = project();
    let s = b"\"abc\".unpack(\"h*\").first\n";
    for args in [vec!["a.rb"], vec!["--only", COP, "a.rb"]] {
        let out = unchanged(d.path(), s, &args, 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:1:7: C: Use `unpack1(\"h*\")` instead of `unpack(\"h*\").first`. (Style/UnpackFirst)"));
    }
    unchanged(
        d.path(),
        s,
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(s).unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), s);
    assert_eq!(fix(d.path(), s, 0), b"\"abc\".unpack1(\"h*\")\n");
}
#[test]
fn config_excludes_strict_settings_and_directive_forms() {
    let d = project();
    let s = b"\"abc\".unpack(\"h*\").first\n";
    for config in [
        "[cops.Style]\nenabled = false\n",
        "[cops.\"Style/UnpackFirst\"]\nenabled = false\n",
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), s, &["--fix", "a.rb"], 0);
        unchanged(d.path(), s, &["--only", COP, "a.rb"], 1);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.Style]\nenabled = false\n[cops.\"Style/UnpackFirst\"]\nenabled = true\n",
    )
    .unwrap();
    unchanged(d.path(), s, &["a.rb"], 1);
    for config in [
        "exclude = [\"a.rb\"]\n",
        "[cops.\"Style/UnpackFirst\"]\nexclude = [\"a.rb\"]\n",
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), s, &["--only", COP, "--fix", "a.rb"], 0);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Style/UnpackFirst\"]\nEnforcedStyle = 'first'\n",
    )
    .unwrap();
    unchanged(d.path(), s, &["--only", COP, "--fix", "a.rb"], 2);
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "UnpackFirst", "Style", "all"] {
            for source in [
                format!(
                    "# {prefix}:disable {target}\n{}",
                    String::from_utf8_lossy(s)
                ),
                format!(
                    "{} # {prefix}:disable {target}\n",
                    String::from_utf8_lossy(s).trim()
                ),
                format!(
                    "# {prefix}:disable-next {target}\n{}",
                    String::from_utf8_lossy(s)
                ),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
        }
    }
}
#[test]
fn bom_crlf_encoding_frozen_line_and_data_bytes_survive() {
    let d = project();
    for (source, after) in [
        (
            b"\xef\xbb\xbf\"abc\".unpack(\"h*\").first\r\np __LINE__\r\n__END__\r\n\xffpayload\r\n"
                .as_slice(),
            b"\xef\xbb\xbf\"abc\".unpack1(\"h*\")\r\np __LINE__\r\n__END__\r\n\xffpayload\r\n"
                .as_slice(),
        ),
        (
            b"# encoding: ISO-8859-1\n'\xe9'.unpack('H*')[0]\n",
            b"# encoding: ISO-8859-1\n'\xe9'.unpack1('H*')\n",
        ),
        (
            b"# encoding: Windows-31J\n'\x83\\'.unpack('a*').at(0)\n",
            b"# encoding: Windows-31J\n'\x83\\'.unpack1('a*')\n",
        ),
        (
            b"# frozen_string_literal: true\n\"a\\xFF\".unpack('a*').slice(0)\n",
            b"# frozen_string_literal: true\n\"a\\xFF\".unpack1('a*')\n",
        ),
    ] {
        assert_eq!(fix(d.path(), source, 0), crate::support::lf::lf(after));
    }
}
#[test]
fn controlled_legacy_source_bytes_keep_result_encoding_and_mutability() {
    if !Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    let d = project();
    for (header, body) in [
        (
            b"# encoding: ISO-8859-1\n".as_slice(),
            b"'\xe9'.unpack('a*').first\n".as_slice(),
        ),
        (
            b"# encoding: Windows-31J\n".as_slice(),
            b"'\x83\\'.unpack('a*').at(0)\n".as_slice(),
        ),
        (
            b"# encoding: ASCII-8BIT\n".as_slice(),
            b"\"\\xFF\".unpack('H*')[0]\n".as_slice(),
        ),
    ] {
        let before = [header, body].concat();
        let after = fix(d.path(), &before, 0);
        assert_ne!(after, before);
        let observe = |s: &[u8]| {
            ruby("ruby", &[header, b"value = ", &s[header.len()..], b"p [value.bytes, value.encoding.name, value.frozen?, value.concat('!').bytes]\n"].concat())
        };
        let a = observe(&before);
        let b = observe(&after);
        assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
        assert_eq!((a.stdout, a.stderr), (b.stdout, b.stderr));
    }
}
fn ruby(exe: &str, source: &[u8]) -> Output {
    let mut child = Command::new(exe)
        .args(["-W0", "--disable-gems", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
fn runtime_matrix(exe: &str) {
    let d = project();
    let mut changed = 0;
    for mode in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        for receiver in ["'abc'", "''", "\"é\\0\\xFF\""] {
            for format in [
                "'a*'", "'A'", "'Z0'", "'h*'", "'H2'", "'b8'", "'B*'", "'C'", "'c1'",
            ] {
                let source = format!("{mode}{receiver}&.unpack({format}).first\n");
                let after = fix(d.path(), source.as_bytes(), 0);
                assert_ne!(after, source.as_bytes());
                changed += 1;
                let observe = |s: &[u8]| {
                    let s = String::from_utf8_lossy(s);
                    let body = s
                        .lines()
                        .filter(|l| !l.starts_with('#'))
                        .collect::<Vec<_>>()
                        .join("\n");
                    ruby(exe, format!("{mode}begin\nv = {body}\nif v.is_a?(String)\np [v.bytes, v.encoding.name, v.frozen?, v.concat('!').bytes]\nelse\np [v.class.name, v]\nend\nrescue => e\np [e.class.name, e.message]\nend\n").as_bytes())
                };
                let a = observe(source.as_bytes());
                let b = observe(&after);
                assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
                assert_eq!((a.stdout, a.stderr), (b.stdout, b.stderr), "{source}");
            }
        }
    }
    assert_eq!(changed, 81);
}
#[test]
fn controlled_host_values_encoding_mutability_and_frozen_modes() {
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        runtime_matrix("ruby");
    }
}
#[test]
fn explicit_native_ruby34_runtime_gate() {
    let Ok(exe) = crate::support::runtime::ruby() else {
        eprintln!("SKIP: set RUSTOCOP_RUBY34 for native 3.4");
        return;
    };
    let out = Command::new(&exe).arg("--version").output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).starts_with(crate::support::runtime::banner_prefix())
    );
    runtime_matrix(&exe);
    // Since Ruby 3.3, even a trailing unknown directive is an exception.
    let observe = |s: &str| {
        ruby(
            &exe,
            format!("begin; p({s}); rescue => e; p e.class.name; end").as_bytes(),
        )
    };
    for (a, b) in [
        ("'abc'.unpack('C?').first", "'abc'.unpack1('C?')"),
        ("'abc'.unpack('CX9').first", "'abc'.unpack1('CX9')"),
        ("\"a\\xFF\".unpack('U*').first", "\"a\\xFF\".unpack1('U*')"),
    ] {
        assert_ne!(observe(a).stdout, observe(b).stdout);
    }
}
#[test]
fn selected_peers_and_actual_defaults_stabilize_in_six_explicit_passes() {
    let d = project();
    let source = b"[ 'abc'.unpack('h*').first, ' x '.lstrip.rstrip, 'name'.intern, '%s' % 1, format('%{x}', x: 1), %q[word], +\"#{1}\", \"a\" \\\n \"b\" ]\n";
    let peers = format!(
        "{COP},Style/StringChars,Style/StringMethods,Style/Strip,Style/FormatString,Style/FormatStringToken,Style/RedundantFormat,Style/StringConcatenation,Style/LineEndConcatenation,Style/RedundantLineContinuation,Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation,Style/EmptyStringInsideInterpolation,Style/RedundantInterpolation,Style/RedundantInterpolationUnfreeze,Style/RedundantStringEscape,Style/RedundantHeredocDelimiterQuotes,Style/EmptyHeredoc,Style/CommandLiteral,Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    for selected in [Some(COP), Some(peers.as_str()), None] {
        let mut old = source.to_vec();
        let mut stable = false;
        for _ in 0..6 {
            fs::write(d.path().join("a.rb"), &old).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(sel) = selected {
                args.extend(["--only", sel]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            let new = fs::read(d.path().join("a.rb")).unwrap();
            if Command::new("ruby")
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                let obs = |s: &[u8]| {
                    ruby(
                        "ruby",
                        format!("p(begin; {}; end)", String::from_utf8_lossy(s)).as_bytes(),
                    )
                };
                let a = obs(source);
                let b = obs(&new);
                assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
                assert_eq!(
                    (a.stdout, a.stderr),
                    (b.stdout, b.stderr),
                    "{}",
                    String::from_utf8_lossy(&new)
                );
            }
            stable = old == new;
            old = new;
        }
        assert!(stable, "{}", String::from_utf8_lossy(&old));
        assert!(String::from_utf8_lossy(&old).contains(".unpack1("));
    }
}
