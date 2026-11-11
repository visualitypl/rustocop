//! controlled ordinary Ruby only. Commands/command heredocs are
//! tested as source and are NEVER executed by any runtime helper.
#[macro_use]
#[path = "support/parallel_rows.rs"]
mod parallel_rows;
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
#[test]
fn changed_dispatch_unknown_arguments_conversions_and_order_withhold() {
    let d = project();
    for s in [
        "def format(s, **k); s; end; format('%{x}', x: 1)\n",
        "module Kernel; def self.format(s, **k); s; end; end; Kernel.format('%{x}', x: 1)\n",
        "class String; def %(x); self; end; end; '%{x}' % {x:1}\n",
        "using Patch; format('%{x}', x:1)\n",
        "mutate; format('%{x}', x: 1)\n",
        "format('%{x}', x: mutate)\n",
        "format('%{x}', **args)\n",
        "format('%{x}', x: value.to_s)\n",
        "Other.format('%{x}', x:1)\n",
        "other&.sprintf('%{x}', x:1)\n",
        "printf('%{x}', x:1)\n",
        "Kernel.printf('%{x}', x:1)\n",
        "format('%{x}', x: Object.new)\n",
        "format('%{x}', x: Kernel)\n",
        "format = 1; '%{x}' % {x:1}\n",
        "format('%{x}', x: 1) { change }\n",
    ] {
        unchanged(d.path(), s.as_bytes(), 1);
    }
    // A later unknown call cannot run before the formatter's argument evaluation.
    assert_eq!(
        fix(d.path(), b"format('%{x}', x: 1); unknown\n", 0),
        b"format('%<x>s', x: 1); unknown\n"
    );
}
#[test]
fn escapes_modifiers_names_and_concat_interpolation_prefixes_withhold() {
    let d = project();
    for s in [
        "format(\"%\" \"%{x}\", x:1)\n",
        "format(\"#{'%'}%{x}\", x:1)\n",
        "format(\"a%{x}\" \"%{y}\", x:1, y:2)\n",
        "format(\"\\x25%{x}\", x:1)\n",
        "format(\"%{x}\\n\", x:1)\n",
        "format('%10{x}', x:1)\n",
        "format(%q>%{x}>, x:1)\n",
    ] {
        unchanged(d.path(), s.as_bytes(), 1);
    }
    unchanged(d.path(), b"format(%qs%{x}s, x:1)\n", 2);
    for s in ["format('%{x>y}', :'x>y' => 1)\n", "format('%%{x}', x:1)\n"] {
        unchanged(d.path(), s.as_bytes(), 0);
    }
}
#[test]
fn bom_crlf_encoding_line_data_source_heap_and_metadata_barriers() {
    let d = project();
    let before = b"\xef\xbb\xbf# encoding: UTF-8\r\nformat(\"%{x}\", x: 1)\r\np __LINE__\r\n__END__\r\n%{data}\r\n";
    let after = b"\xef\xbb\xbf# encoding: UTF-8\r\nformat(\"%<x>s\", x: 1)\r\np __LINE__\r\n__END__\r\n%{data}\r\n";
    assert_eq!(fix(d.path(), before, 0), after);
    for s in [
        b"# encoding: ISO-8859-1\nformat('%{x}', x: 1)\n".as_slice(),
        b"# encoding: Windows-31J\nformat('%{x}', x: 1)\n",
    ] {
        let out = fix(d.path(), s, 0);
        assert!(String::from_utf8_lossy(&out).contains("%<x>s"));
    }
    // Edits are ASCII-only and do not reserialize the enclosing legacy literal.
    assert_eq!(
        fix(
            d.path(),
            b"# encoding: ISO-8859-1\nformat('caf\xe9:%{x}', x: 1)\n",
            0
        ),
        b"# encoding: ISO-8859-1\nformat('caf\xe9:%<x>s', x: 1)\n"
    );
    for observer in [
        "p __FILE__",
        "p method(:p).source_location",
        "p File.binread($0)",
        "p RubyVM::AbstractSyntaxTree",
        "p ObjectSpace.each_object(String).count",
        "p DATA.pos; p DATA.read.bytes\n__END__\nraw",
    ] {
        unchanged(
            d.path(),
            format!("format('%{{x}}', x: 1)\n{observer}\n").as_bytes(),
            1,
        );
    }
}
#[test]
fn heredoc_ownership_continuations_and_command_boundaries_source_only() {
    let d = project();
    let s = b"format(<<~FMT, x: 1)\n%{x}\nFMT\n";
    assert_eq!(fix(d.path(), s, 0), b"format(<<~FMT, x: 1)\n%<x>s\nFMT\n");
    assert_eq!(
        fix(d.path(), b"\"%{x}\" \\\n % {x: 1}\n", 0),
        b"\"%<x>s\" \\\n % {x: 1}\n"
    );
    for s in [
        "format(<<~FMT, x: value)\n%{x}\nFMT\n",
        "format(\"%{x}\", x: `never_execute`)\n",
        "p <<~A\n#{format('%{x}', x: 1)}\nA\n",
        "consume(<<~A, format('%{x}', x:1))\n#{mutate}\nA\n",
    ] {
        unchanged(d.path(), s.as_bytes(), 1);
    }
    for s in [
        "`echo %{x}`\n",
        "%x(echo %{x})\n",
        "<<~`CMD`\necho %{x}\nCMD\n",
        "/%{x}/\n",
    ] {
        unchanged(d.path(), s.as_bytes(), 0);
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
fn observe(executable: &str, expr: &str) -> Output {
    let (header, expr) = if expr.starts_with('#') {
        expr.split_at(expr.find('\n').unwrap() + 1)
    } else {
        ("", expr)
    };
    ruby(
        executable,
        &format!(
            "{header}begin; x = ({expr}); p [x.bytes, x.encoding.name, x.frozen?]; x << 'z'; p x.bytes; rescue => e; p [e.class.name, e.message]; end"
        ),
    )
}
fn runtime_matrix(executable: &str) {
    let d = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        for expr in [
            "format('%{x}', x: 1)",
            "Kernel.sprintf('%{x}', x: false)",
            "'%{x}' % {x: [1,2]}",
            "format('%{x}/%{y}/%%', x: 'é', y: :foo)",
            "format('%2$s/%{x}', x: 1)",
            "format('%{x}/%1$s', x: 1)",
            "format('%{x}:%<y>06.2f', x: 'a', y: 1.25)",
            "format('%<x>10.2s/%%/%{x}/%{y}', x: 'é', y: 2)",
            "format('%{x}/%%/%{y}/%<x>-10.2s', x: 'é', y: 2)",
            "'%%%%%{x}/%<y>06.2f' % {x: 1, y: 2}",
            "format('%{x}/%{x}', x: nil)",
            "format('%{x}/%{missing}', x: 1)",
            "format('%{x}', x: \"\\xff\")",
            "format(%q<%{x}>, x: :foo\u{00a0})",
            "format('%{x}', x: ?x)",
        ] {
            let s = format!("{header}{expr}\n");
            let status =
                if expr.contains("%1$") || expr.contains("%2$") || expr.contains("%{missing}") {
                    1
                } else {
                    0
                };
            let out = String::from_utf8(fix(d.path(), s.as_bytes(), status)).unwrap();
            if expr.contains("%1$") || expr.contains("%2$") {
                // Whole-format mode is mixed, even when an earlier error would
                // happen to mask the changed named-token exception spelling.
                assert_eq!(out, s, "{s}");
            } else {
                assert_ne!(out, s, "{s}");
            }
            let before = observe(executable, &s);
            let after = observe(executable, &out);
            assert!(
                before.status.success() && after.status.success(),
                "{before:?} {after:?}"
            );
            assert_eq!(before.stdout, after.stdout, "{s} -> {out}");
            assert_eq!(before.stderr, after.stderr, "{s} -> {out}");
            unchanged(d.path(), out.as_bytes(), status);
        }
    }
}
#[test]
fn controlled_runtime_values_encodings_mutability_percent_escapes_and_errors() {
    if has_ruby() {
        runtime_matrix("ruby");
    }
}
// A bounded, controlled source matrix, not application/corpus execution.
fn mixed_formats() -> Vec<String> {
    let mut formats = Vec::new();
    for directive in [
        "%s",
        "%1$s",
        "%10.2s",
        "%*.*f",
        "%1$*2$.*3$f",
        "%<x>*s",
        "%*<x>s",
        "%<x>#{w}s",
        "%<x>.#{p}s",
        "%10{x}",
        "%q",
        "%",
        "%<other",
    ] {
        formats.push(format!("{directive}/%{{x}}/%%/%{{y}}"));
        formats.push(format!("%{{x}}/%%/%{{y}}/{directive}"));
    }
    // Exact counterexamples, with no masking intervening directive.
    formats.extend(["%s/%{x}", "%1$s/%{x}", "%{x}/%s", "%{x}/%1$s"].map(str::to_owned));
    formats
}
#[test]
fn mixed_formats_cli_no_write_and_independent_safe_siblings() {
    let d = project();
    for format in mixed_formats() {
        for s in [
            format!("format('{format}', x: 1, y: 2)\n"),
            format!("'{format}' % {{x: 1, y: 2}}\n"),
        ] {
            unchanged(d.path(), s.as_bytes(), 1);
            // A safe sibling in the same file must not make mixed tokens safe.
            let source = format!("format('%{{safe}}', safe: 1)\n{s}");
            let expected = format!("format('%<safe>s', safe: 1)\n{s}");
            assert_eq!(fix(d.path(), source.as_bytes(), 1), expected.as_bytes());
        }
    }
    let mixed = b"format('%s/%{x}', x: 1)\n";
    fs::write(d.path().join("a.rb"), mixed).unwrap();
    fs::write(d.path().join("b.rb"), b"format('%{safe}', safe: 1)\n").unwrap();
    let mtime = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    cli(d.path(), &["--only", COP, "--fix", "a.rb", "b.rb"], 1);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), mixed);
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
    assert_eq!(
        fs::read(d.path().join("b.rb")).unwrap(),
        b"format('%<safe>s', safe: 1)\n"
    );
}
fn mixed_runtime_matrix(executable: &str) {
    let d = project();
    for format in mixed_formats() {
        for expr in [
            format!("format('{format}', x: 1, y: 2)"),
            format!("'{format}' % {{x: 1, y: 2}}"),
        ] {
            let s = format!("{expr}\n");
            let after = fix(d.path(), s.as_bytes(), 1);
            let before = observe(executable, &s);
            let observed = observe(executable, std::str::from_utf8(&after).unwrap());
            assert!(before.status.success(), "{s}: {before:?}");
            assert_eq!(before.status, observed.status, "{s}");
            assert_eq!(before.stdout, observed.stdout, "{s}");
            assert_eq!(before.stderr, observed.stderr, "{s}");
        }
    }
    // Show these are semantic guards, not only a conservative parser check.
    for expr in [
        "format('%s/%{x}', x: 1)",
        "format('%1$s/%{x}', x: 1)",
        "'%s/%{x}' % {x: 1}",
        "'%1$s/%{x}' % {x: 1}",
    ] {
        let before = observe(executable, expr);
        let blind = observe(executable, &expr.replace("%{x}", "%<x>s"));
        assert!(
            String::from_utf8_lossy(&before.stdout).contains("ArgumentError"),
            "{before:?}"
        );
        assert_ne!(before.stdout, blind.stdout, "{expr}");
    }
}
#[test]
fn mixed_whole_format_runtime_exception_class_and_message_preserved() {
    if has_ruby() {
        mixed_runtime_matrix("ruby");
    }
}
#[test]
fn missing_named_keys_keep_original_exception_spelling() {
    let d = project();
    for s in [
        "format('%{x}')\n",
        "format('%{x}', {})\n",
        "'%{x}' % {}\n",
        "format('%{x}', {'x' => 1})\n",
        "format('%{x}', x: 1, **{})\n",
        "format('%{x}', {x:1}, 2)\n",
    ] {
        unchanged(d.path(), s.as_bytes(), 1);
        if has_ruby() {
            let before = observe("ruby", s);
            let after = observe("ruby", &fs::read_to_string(d.path().join("a.rb")).unwrap());
            assert_eq!(before.stdout, after.stdout, "{s}");
            assert_eq!(before.stderr, after.stderr, "{s}");
        }
    }
    if has_ruby() {
        assert_ne!(
            observe("ruby", "format('%{x}', {})").stdout,
            observe("ruby", "format('%<x>s', {})").stdout
        );
    }
}
#[test]
fn controlled_counterexamples_show_why_unknown_dispatch_and_fragments_are_data() {
    if !has_ruby() {
        return;
    }
    let d = project();
    for s in [
        "def format(s, **kw); p [s,kw]; s; end; p format('%{x}', x:1)\n",
        "class String; def %(v); p [self,v]; self; end; end; p('%{x}' % {x:1})\n",
        "p format(\"#{'%'}%{x}\", x: 1)\n",
        "p format(\"%\" \"%{x}\", x: 1)\n",
        "def value; print 'once'; 1; end; p format('%{x}', x: value)\n",
    ] {
        let before = ruby("ruby", s);
        assert!(before.status.success(), "{before:?}");
        unchanged(d.path(), s.as_bytes(), 1);
        let after = ruby("ruby", &fs::read_to_string(d.path().join("a.rb")).unwrap());
        assert_eq!(before.stdout, after.stdout, "{s}");
        assert_eq!(before.stderr, after.stderr, "{s}");
        // A blind spelling change actually differs on these controlled inputs.
        if !s.contains("def value") {
            assert_ne!(
                before.stdout,
                ruby("ruby", &s.replace("%{x}", "%<x>s")).stdout,
                "{s}"
            );
        }
    }
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
fn explicit_ruby34_runtime_gate() {
    let Ok(executable) = std::env::var("RUSTOCOP_RUBY34") else {
        eprintln!("SKIP Ruby 3.4 runtime: set RUSTOCOP_RUBY34");
        return;
    };
    let out = ruby(
        &executable,
        "abort RUBY_VERSION unless RUBY_VERSION.start_with?('3.4.')",
    );
    assert!(out.status.success(), "{out:?}");
    runtime_matrix(&executable);
    mixed_runtime_matrix(&executable);
}
#[test]
fn mixed_modes_with_named_siblings_preserve_actual_composition_through_six_passes() {
    parallel_rows!(for format in [
        "%s/%{x}",
        "%1$s/%{x}",
        "%{x}/%s",
        "%{x}/%1$s",
        "%*.*f/%{x}/%%/%{y}",
        "%{x}/%%/%{y}/%10.2s",
    ] => {
        let d = project();
        for expr in [
            format!("sprintf('{format}', :x => 1, :y => 2)"),
            format!("'{format}' % {{:x => 1, :y => 2}}"),
        ] {
            for header in [
                "",
                "# frozen_string_literal: true\n",
                "# frozen_string_literal: false\n",
            ] {
                // Rescue belongs to the external runtime witness, not the
                // linted fixture: the cop intentionally withholds in files
                // with unsupported control flow. Never broaden that proof.
                let s = format!(
                    "{header}safe = sprintf('%{{safe}}/%%/%<other>06.2f', :safe => ?a, :other => 1.25)\nresult = {expr}\nsibling = 'safe'\n"
                );
                let run = |source: &str| {
                    let (header, body) = if source.starts_with('#') {
                        source.split_at(source.find('\n').unwrap() + 1)
                    } else {
                        ("", source)
                    };
                    ruby(
                        "ruby",
                        &format!(
                            "{header}begin;\n{body}\nrescue => e; p [e.class.name, e.message]; end; p [safe.bytes, safe.encoding.name, safe.frozen?]; safe << 'z'; p safe.bytes"
                        ),
                    )
                };
                let before = has_ruby().then(|| run(&s));
                for selected in [Some(COP), Some(PEERS), None] {
                    fs::write(d.path().join("a.rb"), &s).unwrap();
                    let mut stable = false;
                    for _ in 0..6 {
                        let old = fs::read(d.path().join("a.rb")).unwrap();
                        let mut args = vec!["--fix", "a.rb"];
                        if let Some(names) = selected {
                            args.extend(["--only", names]);
                        }
                        let out = cli(d.path(), &args, 1);
                        assert!(
                            !String::from_utf8_lossy(&out.stdout).contains("failed validation"),
                            "{out:?}"
                        );
                        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
                        assert!(after.contains(format), "mixed mode edited: {after}");
                        if let Some(before) = &before {
                            let observed = run(&after);
                            assert!(before.status.success(), "{before:?}");
                            assert_eq!(before.status, observed.status, "{after}");
                            assert_eq!(before.stdout, observed.stdout, "{after}");
                            assert_eq!(before.stderr, observed.stderr, "{after}");
                        }
                        // Run all six explicit passes, including stable ones.
                        stable = old == after.as_bytes();
                    }
                    assert!(stable, "{selected:?}: {s}");
                    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
                    assert!(after.contains("%<safe>s"), "safe sibling withheld: {after}");
                    if selected != Some(COP) {
                        // The cop and literal/layout peers participated.
                        assert!(!after.contains("sprintf("), "{after}");
                        assert!(after.contains("sibling = \"safe\""), "{after}");
                    }
                }
            }
        }
    });
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
