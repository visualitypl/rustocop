//! commands are NEVER executed, including via a stubbed backtick.
//! All witnesses below inspect source or Prism ASTs only.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/CommandLiteral";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
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
fn unchanged(dir: &Path, source: &[u8], status: i32) {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    cli(
        dir,
        &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
        status,
    );
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}
fn fix(dir: &Path, source: &[u8], status: i32) -> Vec<u8> {
    fs::write(dir.join("a.rb"), source).unwrap();
    cli(
        dir,
        &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
        status,
    );
    fs::read(dir.join("a.rb")).unwrap()
}

#[test]
fn default_on_registry_lint_only_and_stdin_never_write() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.safe_fixes() && cop.default_enabled());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    let dir = project();
    let path = dir.path().join("a.rb");
    let source = b"p %x(not_executed)\n";
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    for args in [vec!["a.rb"], vec!["--only", COP, "a.rb"]] {
        let out = cli(dir.path(), &args, 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}
#[test]
fn config_selectors_and_canonical_one_pass_idempotence() {
    let dir = project();
    for config in [
        "[cops.Style]\nenabled = false\n".to_string(),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let source = b"p %x(not_executed)\n";
        fs::write(dir.path().join("a.rb"), source).unwrap();
        cli(dir.path(), &["--fix", "a.rb"], 0);
        cli(
            dir.path(),
            &["--only", COP, "--except", COP, "--fix", "a.rb"],
            0,
        );
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), source);
        assert_eq!(fix(dir.path(), source, 0), b"p `not_executed`\n");
        unchanged(dir.path(), b"p `not_executed`\n", 0);
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    fs::write(dir.path().join("a.rb"), b"%x(not_executed)\n").unwrap();
    cli(dir.path(), &["--fix", "a.rb"], 0);
    assert_eq!(
        fs::read(dir.path().join("a.rb")).unwrap(),
        b"`not_executed`\n"
    );
}
#[test]
fn directive_aliases_and_excludes_keep_bytes_and_mtime() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "CommandLiteral", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n%x(no)\n"),
                format!("# {prefix}:disable-next {target}\n%x(no)\n"),
                format!("%x(no) # {prefix}:disable {target}\n"),
            ] {
                unchanged(dir.path(), source.as_bytes(), 0);
            }
        }
    }
    for config in [
        "exclude = [\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(dir.path(), b"%x(no)\n", 0);
    }
}
#[test]
fn clean_errors_inner_backticks_and_unsupported_options_never_write() {
    let dir = project();
    for (source, status) in [
        (b"`no`\n".as_slice(), 0),
        (b"%x(\n", 2),
        (b"`echo \\`no\\``\n", 1),
        (b"%x(echo `no`)\n", 0),
    ] {
        unchanged(dir.path(), source, status);
    }
    for key in ["EnforcedStyle", "AllowInnerBackticks"] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key} = \"other\"\n"),
        )
        .unwrap();
        unchanged(dir.path(), b"%x(no)\n", 2);
    }
}
#[test]
fn bom_crlf_line_values_and_heredoc_body_boundaries_survive() {
    let dir = project();
    let source = b"\xef\xbb\xbf# encoding: UTF-8\r\np %x(no), <<~TXT\r\n  static\r\n  #{%x(#{value})}\r\nTXT\r\np __LINE__\r\n";
    let expected = b"\xef\xbb\xbf# encoding: UTF-8\r\np `no`, <<~TXT\r\n  static\r\n  #{`#{value}`}\r\nTXT\r\np __LINE__\r\n";
    assert_eq!(fix(dir.path(), source, 0), crate::support::lf::lf(expected));
    // A command heredoc itself is excluded, but its real command child isn't.
    assert_eq!(
        fix(dir.path(), b"p <<~`SH`\n  #{%x(no)}\nSH\n", 0),
        b"p <<~`SH`\n  #{`no`}\nSH\n"
    );
    // Deferred heredocs belong to interpolation, not the envelope's physical tail.
    assert_eq!(
        fix(dir.path(), b"p %x(#{<<~A}), <<~B\ntext\nA\nother\nB\n", 0),
        b"p `#{<<~A}`, <<~B\ntext\nA\nother\nB\n"
    );
}
#[test]
fn disabled_body_spans_never_receive_partial_delimiter_edits() {
    let dir = project();
    unchanged(
        dir.path(),
        b"%x(\n#{1 # rubocop:disable Style/CommandLiteral\n}\n)\n",
        0,
    );
}
#[test]
fn legacy_encoding_trail_backticks_are_not_syntax_characters() {
    let dir = project();
    // Windows-31J チ is 0x83 0x60. Raw byte matching would reverse both results.
    unchanged(dir.path(), b"# encoding: Windows-31J\n`\x83\x60`\n", 0);
    unchanged(dir.path(), b"# encoding: Windows-31J\n%x(\x83\x60)\n", 1); // UTF-8 envelope cannot represent this edit.
    unchanged(
        dir.path(),
        b"# encoding: Windows-31J\n%x(\x83\x60 `no`)\n",
        0,
    );
    unchanged(dir.path(), b"# encoding: ISO-8859-1\n%x(caf\xe9)\n", 1);
    assert_eq!(
        fix(dir.path(), b"# encoding: Windows-31J\n%x(no)\n", 0),
        b"# encoding: Windows-31J\n`no`\n"
    );
}

// ruby-prism's Rust API exposes a full child walk. Compare executable kinds,
// scope variable identities, call/block ownership, decoded fragments and flags.
fn shape(bytes: &[u8]) -> Vec<String> {
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Shape(Vec<String>);
    impl<'pr> NodeVisitor<'pr> for Shape {
        fn enter(&mut self, n: ruby_prism::Node<'pr>) {
            self.0.push(format!("{:?}", std::mem::discriminant(&n)));
            if let Some(s) = n.as_x_string_node() {
                self.0.push(format!(
                    "cmd:{:?}:{}:{}",
                    s.unescaped(),
                    s.is_forced_utf8_encoding(),
                    s.is_forced_binary_encoding()
                ));
            }
            if let Some(s) = n.as_string_node() {
                self.0.push(format!(
                    "str:{:?}:{}:{}:{}:{}",
                    s.unescaped(),
                    s.is_forced_utf8_encoding(),
                    s.is_forced_binary_encoding(),
                    s.is_frozen(),
                    s.is_mutable()
                ));
            }
            if let Some(s) = n.as_local_variable_read_node() {
                self.0
                    .push(format!("read:{:?}:{}", s.name().as_slice(), s.depth()));
            }
            if let Some(s) = n.as_local_variable_write_node() {
                self.0
                    .push(format!("write:{:?}:{}", s.name().as_slice(), s.depth()));
            }
            if let Some(s) = n.as_call_node() {
                self.0.push(format!("call:{:?}", s.name().as_slice()));
            }
        }
        fn leave(&mut self) {
            self.0.push("end".into());
        }
    }
    let parsed = ruby_prism::parse(bytes);
    assert_eq!(
        parsed.errors().count(),
        0,
        "{:?}",
        String::from_utf8_lossy(bytes)
    );
    let mut result = Shape::default();
    walk(&mut result, &parsed.node());
    result.0
}
#[test]
fn main_ast_command_payloads_bindings_call_block_ownership_and_flags_match() {
    let dir = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
        "# encoding: US-ASCII\n",
    ] {
        for body in [
            "x = 1; xs.map { %x(#{x}-#{it}) }; xs.map { %x(#{_1}) }\n",
            "consume %x(foo), x do |arg|\n  use(arg)\nend\n",
            "consume(%x(foo)) { |arg| use(arg) }\n",
            "obj&.consume(%x(foo)).next\n",
            "x = %x(#{mark(1)}-#{mark(2)}-#@ivar-#$global)\n",
            "x = %x(#{defined?(name)}-#{name = 1}-#{name})\n",
            "[%x(\\t\\n\\x41\\u{42}\\101\\M-a\\C-a), %x(a\\)b), %x!a\\!b!, %x(a\\\\)]\n",
            "%x'#{mark(1)}\\n'\n",
            "p %x(#{<<~A}), <<~B\ntext\nA\nother\nB\n",
        ] {
            let source = format!("{header}{body}");
            let after = fix(dir.path(), source.as_bytes(), 0);
            assert_ne!(after, source.as_bytes());
            assert_eq!(shape(source.as_bytes()), shape(&after), "{source}");
        }
    }
}
#[test]
fn every_printable_punctuation_delimiter_and_escape_keeps_command_payload() {
    let dir = project();
    let mut literals = Vec::new();
    for delimiter in (b'!'..=b'~')
        .filter(u8::is_ascii_punctuation)
        .filter(|&b| b != b'\\')
    {
        let close = match delimiter {
            b'(' => b')',
            b'[' => b']',
            b'{' => b'}',
            b'<' => b'>',
            b => b,
        };
        for body in [
            "abc".to_string(),
            format!("a\\{}b", char::from(close)),
            "\\n\\u{41}".into(),
            format!("a\\M-\\{}b", char::from(close)),
            format!("a\\C-\\{}b", char::from(close)),
            format!("a\\c\\{}b", char::from(close)),
        ] {
            let source = format!("%x{}{body}{}\n", char::from(delimiter), char::from(close));
            // Inner raw backticks intentionally disallow the percent->backtick edit.
            let after = fix(dir.path(), source.as_bytes(), 0);
            assert_eq!(shape(source.as_bytes()), shape(&after), "{source}");
            literals.push(source.trim_end().to_string());
        }
    }
    let source = format!("[{}]\n", literals.join(",\n"));
    let path = dir.path().join("a.rb");
    fs::write(&path, &source).unwrap();
    let before = explicit_shape(&path);
    fix(dir.path(), source.as_bytes(), 0);
    if let Some(before) = before {
        assert_eq!(Some(before), explicit_shape(&path));
    }
}

// Explicit version 3.4 comparison independent of the installed Ruby runtime.
// This code parses fixtures. It does NOT eval/load/execute them.
const PRISM_SHAPE: &str = r#"
require 'prism'
def shape(x)
  case x
  when Prism::Node
    fields = x.deconstruct_keys(nil).reject { |k,v| k == :node_id || k == :location || k.to_s.end_with?('_loc') }
    [x.type, x.send(:flags), fields.map { |k,v| [k, shape(v)] }]
  when String then [x.bytes, x.encoding.name]
  when Array then x.map { |v| shape(v) }
  else x
  end
end
parsed = Prism.parse(File.binread(ARGV[0]), version: '3.4')
abort parsed.errors.map(&:message).join('\n') unless parsed.success?
p shape(parsed.value)
"#;
fn explicit_shape(path: &Path) -> Option<Vec<u8>> {
    let out = Command::new("ruby")
        .args(["-e", PRISM_SHAPE])
        .arg(path)
        .output()
        .ok()?;
    // Missing optional Prism is a genuine prerequisite skip, not parse success.
    if !out.status.success()
        && String::from_utf8_lossy(&out.stderr).contains("cannot load such file -- prism")
    {
        return None;
    }
    assert!(out.status.success(), "{out:?}");
    Some(out.stdout)
}
#[test]
fn explicit_fixed_34_full_fields_payload_encodings_flags_and_scopes_match() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for source in [
        b"# frozen_string_literal: true\nx=1\nxs.map { [%x(), %x'#{x}-#{it}', %x(\\u{41}\\xff)] }\nxs.map { %x(#{_1}) }\n".as_slice(),
        b"# frozen_string_literal: false\nconsume %x(#{mark(1)}-#{mark(2)}), x do |arg|\n use(arg)\nend\n",
        b"\xef\xbb\xbf# encoding: UTF-8\r\nx = %x(caf\xc3\xa9)\r\nx = %x(#{x})\r\n",
        b"# encoding: Windows-31J\n[%x(\\u{41}), %x(abc)]\n",
        b"# encoding: US-ASCII\n[%x(\\u{e9}), %x(\\xFF)]\n",
    ] {
        fs::write(&path, source).unwrap();
        let Some(before) = explicit_shape(&path) else { eprintln!("SKIP explicit Prism 3.4: Ruby/Prism unavailable"); return; };
        let after = fix(dir.path(), source, 0);
        assert_ne!(source, after);
        assert_eq!(Some(before), explicit_shape(&path), "{:?}", String::from_utf8_lossy(source));
    }
}
#[test]
fn nested_percent_candidates_defer_without_quadratic_envelopes_or_partial_edits() {
    let dir = project();
    let source = b"%x(#{%x(no)})\n";
    let first = fix(dir.path(), source, 1);
    assert_eq!(first, b"`#{%x(no)}`\n");
    assert_eq!(shape(source), shape(&first));
    // The nested percent command becomes backticks; the outer raw-policy
    // offense then remains uncorrectable, rather than oscillating.
    let second = fix(dir.path(), &first, 1);
    assert_eq!(second, b"`#{`no`}`\n");
    assert_eq!(shape(source), shape(&second));
    unchanged(dir.path(), &second, 1);
}
#[test]
fn actual_defaults_and_original_style_peers_converge_within_six_passes() {
    let dir = project();
    let path = dir.path().join("a.rb");
    let source = "[%x[#{mark(1)}-#{mark(2)}], %x'\\n', %x(a\\)b), %x!a\\!b!, %x(#{%x(nested)})]\n";
    let peers = format!(
        "{COP},Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation,Style/EmptyStringInsideInterpolation,Style/RedundantInterpolation,Style/RedundantInterpolationUnfreeze,Style/RedundantStringEscape,Style/RedundantHeredocDelimiterQuotes,Style/EmptyHeredoc,Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    for selected in [Some(COP), Some(peers.as_str()), None] {
        fs::write(&path, source).unwrap();
        let before = explicit_shape(&path);
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(&path).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(names) = selected {
                args.extend(["--only", names]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_eq!(shape(source.as_bytes()), shape(&fs::read(&path).unwrap()));
            if let Some(ref before) = before {
                assert_eq!(Some(before.clone()), explicit_shape(&path));
            }
            if old == fs::read(&path).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        let after = fs::read_to_string(&path).unwrap();
        assert_ne!(after, source);
        assert!(!after.contains("%x"), "{after}");
    }
}
