//! controlled ordinary Ruby only. Commands/command heredocs are
//! tested as source and are NEVER executed by any runtime helper.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/FormatStringToken";
const PEERS: &str = "Style/FormatStringToken,Style/FormatString,Style/CommandLiteral,Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation,Style/EmptyStringInsideInterpolation,Style/RedundantInterpolation,Style/RedundantInterpolationUnfreeze,Style/RedundantStringEscape,Style/RedundantHeredocDelimiterQuotes,Style/EmptyHeredoc,Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(status), "{args:?}: {o:?}");
    o
}
fn fix(dir: &Path, source: &[u8], status: i32) -> Vec<u8> {
    fs::write(dir.join("a.rb"), source).unwrap();
    cli(dir, &["--only", COP, "--fix", "a.rb"], status);
    fs::read(dir.join("a.rb")).unwrap()
}
fn unchanged(dir: &Path, source: &[u8], status: i32) {
    fs::write(dir.join("a.rb"), source).unwrap();
    let mtime = fs::metadata(dir.join("a.rb")).unwrap().modified().unwrap();
    cli(dir, &["--only", COP, "--fix", "a.rb"], status);
    assert_eq!(fs::read(dir.join("a.rb")).unwrap(), source);
    assert_eq!(
        fs::metadata(dir.join("a.rb")).unwrap().modified().unwrap(),
        mtime
    );
}
#[test]
fn default_on_safe_metadata_selectors_stdin_and_lint_never_write() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    let d = project();
    let s = b"format(\"%{x}\", x: 1)\n";
    fs::write(d.path().join("a.rb"), s).unwrap();
    let mtime = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    for args in [vec!["a.rb"], vec!["--only", COP, "a.rb"]] {
        let out = cli(d.path(), &args, 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
    }
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
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
}
#[test]
fn config_enable_only_except_excludes_and_strict_fixed_policy() {
    let d = project();
    let s = b"format(\"%{x}\", x: 1)\n";
    for config in [
        "[cops.Style]\nenabled = false\n".into(),
        // Keep this token-cop disable test independent of the newly default-on
        // detection-only RedundantFormat rule for this fully literal formatter.
        format!(
            "[cops.\"{COP}\"]\nenabled = false\n[cops.\"Style/RedundantFormat\"]\nenabled = false\n"
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        fs::write(d.path().join("a.rb"), s).unwrap();
        cli(d.path(), &["--fix", "a.rb"], 0);
        cli(
            d.path(),
            &["--only", COP, "--except", COP, "--fix", "a.rb"],
            0,
        );
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), s);
        assert_eq!(fix(d.path(), s, 0), b"format(\"%<x>s\", x: 1)\n");
        unchanged(d.path(), b"format(\"%<x>s\", x: 1)\n", 0);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    fs::write(d.path().join("a.rb"), s).unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(
        fs::read(d.path().join("a.rb")).unwrap(),
        b"format(\"%<x>s\", x: 1)\n"
    );
    for config in [
        "exclude = [\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), s, 0);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle = \"template\"\n"),
    )
    .unwrap();
    unchanged(d.path(), s, 2);
}
#[test]
fn directives_clean_errors_and_raw_data_preserve_bytes_and_mtime() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "FormatStringToken", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\nformat(\"%{{x}}\", x: 1)\n"),
                format!("# {prefix}:disable-next {target}\nformat(\"%{{x}}\", x: 1)\n"),
                format!("format(\"%{{x}}\", x: 1) # {prefix}:disable {target}\n"),
            ] {
                unchanged(d.path(), s.as_bytes(), 0);
            }
        }
    }
    for (s, status) in [
        ("format(\"%<x>s\", x: 1)\n", 0),
        ("format(\n", 2),
        ("\"%{x}\"\n", 1),
        ("redirect(\"%{x}\")\n", 1),
        ("format(\"%s %s\", 1, 2)\n", 1),
        ("format(\"%s\", 1)\n", 0),
    ] {
        unchanged(d.path(), s.as_bytes(), status);
    }
}
fn ruby(executable: &str, source: &str) -> Output {
    // Controlled ordinary expressions ONLY; never commands or corpus code.
    Command::new(executable)
        .args(["-e", source])
        .output()
        .unwrap()
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
// Parse-only evidence with explicit target 3.4, independent of host runtime.
// Normalize only the intended format-string spelling; preserve every other
// AST field, node kind, decoded byte, encoding, flag and binding identity.
const PRISM_FACTS: &str = r#"
require 'prism'
parsed = Prism.parse(File.binread(ARGV[0]), version: '3.4')
abort parsed.errors.map(&:message).join('\n') unless parsed.success?
def collect(node)
  fields = node.deconstruct_keys(nil).reject { |k,_| k == :node_id || k == :location || k.to_s.end_with?('_loc') }
  fields = fields.map do |k,v|
    value = case v
    when Prism::Node then collect(v)
    when Array then v.map { |c| c.is_a?(Prism::Node) ? collect(c) : c }
    when String
      bytes = node.type == :string_node && k == :unescaped ? v.b.gsub(/%<([A-Za-z0-9_]+)>s/n, '%{\1}') : v.b
      [bytes.bytes, v.encoding.name]
    else v
    end
    [k,value]
  end
  [node.type, node.send(:flags), fields]
end
p collect(parsed.value)
"#;
#[test]
fn explicit_prism34_preserves_ast_binding_flags_decoded_bytes_and_encodings() {
    let d = project();
    let path = d.path().join("a.rb");
    let snapshot = || {
        let out = Command::new("ruby")
            .args(["-e", PRISM_FACTS])
            .arg(&path)
            .output()
            .ok()?;
        if !out.status.success()
            && String::from_utf8_lossy(&out.stderr).contains("cannot load such file -- prism")
        {
            return None;
        }
        assert!(out.status.success(), "{out:?}");
        Some(out.stdout)
    };
    for s in [
        b"# frozen_string_literal: true\nresult = format('%{x}', x: 1)\np result\n".as_slice(),
        b"# frozen_string_literal: false\nresult = '%{x}' % {x: false}\np result\n",
        b"\xef\xbb\xbf# encoding: UTF-8\r\nresult = Kernel.format('%{x}', x: 'caf\xc3\xa9')\r\np result\r\n",
        b"# encoding: Windows-31J\nresult = format('%{x}', x: '\x83\x5c')\np result\n",
        b"# encoding: ISO-8859-1\nresult = format('caf\xe9:%{x}', x: 1)\np result\n",
        b"# encoding: US-ASCII\nresult = format('%{x}', x: \"\\u{e9}\")\np result\n",
        b"result = format(<<~FMT, x: 1)\n%{x}\nFMT\np result\n",
    ] {
        fs::write(&path, s).unwrap();
        let Some(before) = snapshot() else { eprintln!("SKIP explicit Prism 3.4: Ruby/Prism unavailable"); return };
        let after = fix(d.path(), s, 0);
        assert_ne!(after, s);
        assert_eq!(Some(before), snapshot(), "{:?}", String::from_utf8_lossy(s));
    }
}
#[test]
fn controlled_legacy_encoded_files_bom_crlf_line_and_data_keep_runtime_observations() {
    if !has_ruby() {
        return;
    }
    let d = project();
    for (header, body) in [
        (
            b"# encoding: ISO-8859-1\n".as_slice(),
            b"caf\xe9:%{x}".as_slice(),
        ),
        (
            b"# encoding: Windows-31J\n".as_slice(),
            b"\x83\x5c:%{x}".as_slice(),
        ),
        (
            b"\xef\xbb\xbf# encoding: UTF-8\r\n".as_slice(),
            b"caf\xc3\xa9:%{x}".as_slice(),
        ),
    ] {
        let source = [header, b"result = format('", body, b"', x: 1)\r\np [result.bytes, result.encoding.name, result.frozen?, __LINE__]\r\n__END__\r\nraw %{data}\r\n"].concat();
        fs::write(d.path().join("a.rb"), &source).unwrap();
        // These exact synthetic files contain no commands/application code.
        let run = || {
            Command::new("ruby")
                .arg(d.path().join("a.rb"))
                .output()
                .unwrap()
        };
        let before = run();
        assert!(before.status.success(), "{before:?}");
        let after = fix(d.path(), &source, 0);
        assert_ne!(after, source);
        let observed = run();
        assert_eq!(before.status, observed.status);
        assert_eq!(before.stdout, observed.stdout);
        assert_eq!(before.stderr, observed.stderr);
    }
}
#[test]
fn actual_formatstring_literal_peers_defaults_and_layout_preserve_each_pass() {
    let d = project();
    let s = "values = [sprintf('%{x}', :x => ?a), '%{x}/%{y}' % {x: %Q(hi), y: 2}]\np values\nsibling = 'safe'\np sibling\n";
    let before = has_ruby().then(|| ruby("ruby", s));
    for selected in [Some(COP), Some(PEERS), None] {
        fs::write(d.path().join("a.rb"), s).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(names) = selected {
                args.extend(["--only", names]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            assert!(
                !String::from_utf8_lossy(&out.stdout).contains("failed validation"),
                "{out:?}"
            );
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            if let Some(before) = &before {
                let observed = ruby("ruby", &after);
                assert_eq!(before.status, observed.status, "{after}");
                assert_eq!(before.stdout, observed.stdout, "{after}");
                assert_eq!(before.stderr, observed.stderr, "{after}");
            }
            if old == after.as_bytes() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(
            after.contains("%<x>s") && after.contains("%<y>s"),
            "{after}"
        );
        if selected != Some(COP) {
            assert!(after.contains("sibling = \"safe\""), "{after}");
        }
    }
}
