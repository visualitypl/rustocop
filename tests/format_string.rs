//! controlled ordinary formatting fixtures only. Command literals are
//! source/AST-only and are NEVER executed (including through runtime helpers).
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/FormatString";
const LITERAL_PEERS: &str = "Style/FormatString,Style/CommandLiteral,Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation,Style/EmptyStringInsideInterpolation,Style/RedundantInterpolation,Style/RedundantInterpolationUnfreeze,Style/RedundantStringEscape,Style/RedundantHeredocDelimiterQuotes,Style/EmptyHeredoc,Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
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
fn fix(dir: &Path, source: &[u8], status: i32) -> Vec<u8> {
    fs::write(dir.join("a.rb"), source).unwrap();
    cli(dir, &["--only", COP, "--fix", "a.rb"], status);
    fs::read(dir.join("a.rb")).unwrap()
}
#[test]
fn default_on_lint_stdin_and_selection_no_write() {
    let registry = rustocop::cops::Registry::new();
    let c = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled() && c.safe_fixes());
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(c.path_policy().is_none());
    let d = project();
    let source = b"p \"%d\" % 1\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    let mtime = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    for args in [vec!["a.rb"], vec!["--only", COP, "a.rb"]] {
        let o = cli(d.path(), &args, 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(COP));
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
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
}
#[test]
fn config_only_except_and_canonical_idempotence() {
    let d = project();
    for config in [
        "[cops.Style]\nenabled = false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let source = b"p \"%d\" % 1\n";
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--fix", "a.rb"], 0);
        cli(
            d.path(),
            &["--only", COP, "--except", COP, "--fix", "a.rb"],
            0,
        );
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), source);
        assert_eq!(fix(d.path(), source, 0), b"p format(\"%d\", 1)\n");
        unchanged(d.path(), b"p format(\"%d\", 1)\n", 0);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    fs::write(d.path().join("a.rb"), b"sprintf(\"%s\", 1)\n").unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(
        fs::read(d.path().join("a.rb")).unwrap(),
        b"format(\"%s\", 1)\n"
    );
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle = \"percent\"\n"),
    )
    .unwrap();
    unchanged(d.path(), b"sprintf(\"%s\", 1)\n", 2);
}
#[test]
fn directives_excludes_clean_errors_and_withheld_keep_bytes_mtime() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "FormatString", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\n\"%s\" % 1\n"),
                format!("# {prefix}:disable-next {target}\n\"%s\" % 1\n"),
                format!("\"%s\" % 1 # {prefix}:disable {target}\n"),
            ] {
                unchanged(d.path(), s.as_bytes(), 0);
            }
        }
    }
    for (source, status) in [
        (b"format(\"%s\", 1)\n".as_slice(), 0),
        (b"sprintf(\n", 2),
        (b"\"%s\" % value\n", 1),
    ] {
        unchanged(d.path(), source, status);
    }
    for config in [
        "exclude = [\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), b"\"%s\" % 1\n", 0);
    }
}
#[test]
fn bom_crlf_line_data_encoding_and_source_observers() {
    let d = project();
    let before =
        b"\xef\xbb\xbf# encoding: UTF-8\r\n\"%s\" % 1\r\np __LINE__\r\n__END__\r\npayload %s\r\n";
    let after = b"\xef\xbb\xbf# encoding: UTF-8\r\nformat(\"%s\", 1)\r\np __LINE__\r\n__END__\r\npayload %s\r\n";
    assert_eq!(fix(d.path(), before, 0), after);
    for observer in [
        "p __FILE__",
        "p method(:p).source_location",
        "p File.binread($0)",
        "p RubyVM::AbstractSyntaxTree",
        "p ObjectSpace.each_object(String).count",
        "p DATA.pos; p DATA.read.bytes\n__END__\nraw",
    ] {
        unchanged(d.path(), format!("\"%s\" % 1\n{observer}\n").as_bytes(), 1);
    }
    for prefix in [
        b"# encoding: ISO-8859-1\n".as_slice(),
        b"# encoding: Windows-31J\n",
    ] {
        assert_eq!(
            fix(d.path(), &[prefix, b"sprintf(\"%s\", 1)\n"].concat(), 0),
            [prefix, b"format(\"%s\", 1)\n"].concat()
        );
    }
    unchanged(d.path(), b"# encoding: ISO-8859-1\n\"caf\xe9:%s\" % 1\n", 1);
    assert_eq!(
        fix(
            d.path(),
            b"# encoding: ISO-8859-1\nsprintf(\"caf\xe9:%s\", 1)\n",
            0
        ),
        b"# encoding: ISO-8859-1\nformat(\"caf\xe9:%s\", 1)\n"
    );
}
#[test]
fn heredoc_ownership_continuations_and_command_boundaries_source_only() {
    let d = project();
    for source in [
        "<<~FMT % 1\n  %s\nFMT\n",
        "\"%s\" % <<~ARG\n  value\nARG\n",
        "p <<~FMT % [<<~ARG], <<~OTHER\n%s\nFMT\nvalue\nARG\nother\nOTHER\n",
        "\"%s\" \\\n % 1\n",
        "\"%s\" % `never_execute`\n",
        "consume(<<~A, \"%s\" % 1)\n#{mutate}\nA\n",
        "consume(<<~A, sprintf('%s', 1))\n#{mutate}\nA\n",
    ] {
        unchanged(d.path(), source.as_bytes(), 1);
    }
    unchanged(d.path(), b"`never_execute` % 1\n", 0);
    let before = b"sprintf(<<~FMT, <<~ARG)\n%s\nFMT\nvalue\nARG\n";
    assert_eq!(
        fix(d.path(), before, 0),
        b"format(<<~FMT, <<~ARG)\n%s\nFMT\nvalue\nARG\n"
    );
    let before = b"p <<~TEXT\n  #{\"%s\" % 1}\nTEXT\n";
    unchanged(d.path(), before, 1); // Deferred-body work is deliberately withheld.
}
fn ruby(executable: &str, code: &str) -> Output {
    // All callers use controlled ordinary fixtures, NEVER command literals.
    Command::new(executable)
        .args(["-e", code])
        .output()
        .unwrap()
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn observe(executable: &str, expr: &str) -> Output {
    let (header, expr) = if expr.starts_with('#') {
        let end = expr.find('\n').unwrap() + 1;
        expr.split_at(end)
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
            "\"%s\" % 1",
            "\"%s\" % :foo\u{00a0}",
            "\"%s\" % [:foo\u{00a0}]",
            "\"%s\" % [ \t:foo\u{00a0} \t]",
            "\"%s/%s\" % [:foo\u{00a0}, :bar\u{3000}]",
            "\"%{a}\" % {a: :foo\u{00a0}}",
            "\"%{a}\".%(a: :foo\u{00a0})",
            "\"%s/%s\" % [1, 2]",
            "\"%s\" % [[1,2]]",
            "\"%s\" % {a: 1}",
            "\"%<a>d/%{b}\" % {a: 1, b: \"x\"}",
            "\"%s\" % {}",
            "\"%%\" % []",
            "\"%2$s/%1$s/%%\" % [\"a\", \"b\"]",
            "\"%*.*f\" % [8, 2, 1.25]",
            "\"\\u00e9:%s\" % \"\\xff\"",
            "\"%d\" % nil",
            "\"%d\" % []",
            "\"%s\" % [nil]",
            "sprintf(\"%s\", [1, 2])",
            "sprintf(\"%<a>d\", a: 1)",
            "\"#{1}:%s\" % false",
        ] {
            let source = format!("{header}{expr}\n");
            let after = String::from_utf8(fix(d.path(), source.as_bytes(), 0)).unwrap();
            assert_ne!(source, after, "{source}");
            let old = observe(executable, &source);
            let new = observe(executable, &after);
            assert!(
                old.status.success() && new.status.success(),
                "{old:?} {new:?}"
            );
            assert_eq!(old.stdout, new.stdout, "{source} -> {after}");
            assert_eq!(old.stderr, new.stderr, "{source} -> {after}");
        }
    }
}
#[test]
fn controlled_runtime_values_encoding_mutability_array_spread_and_errors() {
    if has_ruby() {
        runtime_matrix("ruby");
    }
}
// Ordinary literal expressions only: no command literals or application code.
fn whitespace_token_expressions() -> Vec<String> {
    let mut tokens = vec![];
    for value in [' ', '\t', '\x0b', '\x0c', '\r'] {
        tokens.push(format!("?\\{value}"));
    }
    for delimiter in [' ', '\t'] {
        tokens.push(format!("%q{delimiter}hello{delimiter}"));
        tokens.push(format!("%s{delimiter}foo{delimiter}"));
    }
    tokens.extend([":foo\u{00a0}".into(), ":foo\u{3000}".into()]);
    tokens
        .into_iter()
        .flat_map(|token| {
            [
                format!("\"%s\" % {token}"),
                format!("\"%s\" % [{token}]"),
                format!("\"%s/%s\" % [1, {token}]"),
                format!("\"%s\" % {{x: {token}}}"),
                format!("\"%{{x}}\" % {{a: 1, x: {token}}}"),
                format!("\"%{{x}}\".%(x: {token})"),
            ]
        })
        .collect()
}
fn whitespace_token_runtime_matrix(executable: Option<&str>) {
    let d = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        for expr in whitespace_token_expressions() {
            // Keep a CR character value distinct from a CRLF line ending.
            let separator = if expr.ends_with('\r') { "\t" } else { "" };
            let source = format!("{header}{expr}{separator}\n");
            let after = String::from_utf8(fix(d.path(), source.as_bytes(), 0)).unwrap();
            assert_ne!(source, after, "{source:?}");
            assert!(after.contains("format("), "{after:?}");
            unchanged(d.path(), after.as_bytes(), 0);
            if let Some(executable) = executable {
                let old = observe(executable, &source);
                let new = observe(executable, &after);
                assert!(
                    old.status.success() && new.status.success(),
                    "{source:?}: {old:?} {new:?}"
                );
                assert_eq!(old.stdout, new.stdout, "{source:?} -> {after:?}");
                assert_eq!(old.stderr, new.stderr, "{source:?} -> {after:?}");
            }
        }
    }
}
#[test]
fn whitespace_token_cli_fixes_preserve_runtime_bytes_encoding_and_mutability() {
    whitespace_token_runtime_matrix(has_ruby().then_some("ruby"));
}
#[test]
fn whitespace_tokens_real_cli_selected_peers_and_defaults_preserve_each_pass() {
    let d = project();
    let runtime_available = has_ruby();
    for expr in whitespace_token_expressions() {
        let separator = if expr.ends_with('\r') { "\t" } else { "" };
        let source = format!("p {expr}{separator}\nsibling = 'safe'\np sibling\n");
        let before = runtime_available.then(|| ruby("ruby", &source));
        if let Some(before) = &before {
            assert!(before.status.success(), "{source:?}: {before:?}");
        }
        for selected in [Some(COP), Some(LITERAL_PEERS), None] {
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let mut stable = false;
            for pass in 0..6 {
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
                assert!(
                    matches!(out.status.code(), Some(0 | 1)),
                    "{selected:?} {source:?}: {out:?}"
                );
                assert!(
                    !String::from_utf8_lossy(&out.stdout).contains("failed validation"),
                    "{out:?}"
                );
                let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
                // Outer canonical formatting wins overlap against literal peers
                // on the first pass, not just after they respell the operand.
                assert!(
                    after.contains("p format("),
                    "pass {pass}, {selected:?}: {after:?}"
                );
                if selected != Some(COP) {
                    // An independent sibling fix commits in the same transaction.
                    assert!(after.contains("sibling = \"safe\""), "{after:?}");
                }
                if let Some(before) = &before {
                    let observed = ruby("ruby", &after);
                    assert_eq!(
                        before.status, observed.status,
                        "{source:?} -> {after:?}: {observed:?}"
                    );
                    assert_eq!(before.stdout, observed.stdout, "{source:?} -> {after:?}");
                    assert_eq!(before.stderr, observed.stderr, "{source:?} -> {after:?}");
                }
                if after.as_bytes() == old {
                    stable = true;
                    break;
                }
            }
            assert!(stable, "{selected:?}: {source:?}");
        }
    }
}
#[test]
fn whitespace_token_and_independent_safe_sibling_commit_together() {
    let d = project();
    for expr in whitespace_token_expressions() {
        let separator = if expr.ends_with('\r') { "\t" } else { "" };
        let source = format!("p {expr}{separator}\nsibling = 'safe'\np sibling\n");
        fs::write(d.path().join("a.rb"), &source).unwrap();
        let out = cli(
            d.path(),
            &[
                "--only",
                "Style/FormatString,Style/StringLiterals",
                "--fix",
                "a.rb",
            ],
            0,
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("2 corrections applied"),
            "{source:?}: {out:?}"
        );
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(
            after.contains("p format(") && after.contains("sibling = \"safe\""),
            "{after:?}"
        );
        let mtime = fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        cli(
            d.path(),
            &[
                "--only",
                "Style/FormatString,Style/StringLiterals",
                "--fix",
                "a.rb",
            ],
            0,
        );
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after.as_bytes());
        assert_eq!(
            fs::metadata(d.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            mtime
        );
    }
}
#[test]
fn explicit_ruby34_runtime_gate() {
    let Ok(executable) = std::env::var("RUSTOCOP_RUBY34") else {
        eprintln!("Ruby 3.4 runtime gate skipped: set RUSTOCOP_RUBY34");
        return;
    };
    let version = ruby(&executable, "abort unless RUBY_VERSION.start_with?('3.4.')");
    assert!(version.status.success(), "{version:?}");
    runtime_matrix(&executable);
    whitespace_token_runtime_matrix(Some(&executable));
}
#[test]
fn runtime_counterexamples_to_overbroad_conversion_dispatch_and_order_fixes() {
    if !has_ruby() {
        return;
    }
    let d = project();
    for (prefix, expr, naive) in [
        (
            "a = Object.new; def a.to_s; [1,2]; end;",
            "\"%s\" % a.to_s",
            "format(\"%s\", a.to_s)",
        ),
        (
            "def format(*); 'custom'; end;",
            "\"%s\" % 1",
            "format(\"%s\", 1)",
        ),
        (
            "def sprintf(*); 'custom'; end;",
            "sprintf('%s', 1)",
            "format('%s', 1)",
        ),
        (
            "class String; def %(x); 'custom'; end; end;",
            "\"%s\" % 1",
            "format(\"%s\", 1)",
        ),
        (
            "a = Object.new; def a.%(x); 'custom'; end;",
            "a % [1]",
            "format(a, 1)",
        ),
        (
            "def conversion; Object.class_eval { define_method(:format) { |*| 'custom' } }; 1; end;",
            "\"%s\" % conversion",
            "format(\"%s\", conversion)",
        ),
    ] {
        let before = format!("{prefix}\nbegin; p ({expr}); rescue => e; p e.class.name; end\n");
        unchanged(d.path(), before.as_bytes(), 1);
        let after = before.replace(expr, naive);
        let old = ruby("ruby", &before);
        let new = ruby("ruby", &after);
        assert!(
            old.status.success() && new.status.success(),
            "{old:?} {new:?}"
        );
        assert_ne!(old.stdout, new.stdout, "{before} -> {after}");
    }
    // Unknown argument/interpolation effects execute once and in the original
    // order, including raising before subsequent arguments. No speculative edits.
    let source = "$events=[]\ndef mark(x); $events << x; x; end\nbegin; p \"%s:%s\" % [mark(1), mark(2)]; p sprintf(mark('%s'), mark(3)); rescue => e; p e.class; end\np $events\n";
    unchanged(d.path(), source.as_bytes(), 1);
    assert_eq!(
        ruby("ruby", source).stdout,
        b"\"1:2\"\n\"3\"\n[1, 2, \"%s\", 3]\n"
    );
}
#[test]
fn fixed_34_literal_payload_flags_and_lexical_binding_evidence() {
    use rustocop::cops::walk::{NodeVisitor, walk};
    fn facts(source: &[u8]) -> Vec<String> {
        struct Facts(Vec<String>);
        impl<'pr> NodeVisitor<'pr> for Facts {
            fn enter(&mut self, n: ruby_prism::Node<'pr>) {
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
            }
        }
        let parsed = ruby_prism::parse(source);
        assert_eq!(parsed.errors().count(), 0);
        let mut facts = Facts(vec![]);
        walk(&mut facts, &parsed.node());
        facts.0
    }
    let d = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        for expr in [
            "\"%s\" % \"é\"",
            "\"%s/%s\" % [\"a\", \"\\xff\"]",
            "sprintf(\"%s\", \"x\")",
            "\"%<a>s\" % {a: \"x\"}",
        ] {
            let source = format!("{header}result = {expr}\np result\n");
            let after = fix(d.path(), source.as_bytes(), 0);
            assert_ne!(source.as_bytes(), after);
            assert_eq!(facts(source.as_bytes()), facts(&after), "{source}");
        }
    }
}
// Parse only, with an explicit 3.4 target independent of the host Ruby runtime.
const PRISM_FACTS: &str = r#"
require 'prism'
parsed = Prism.parse(File.binread(ARGV[0]), version: '3.4')
abort parsed.errors.map(&:message).join('\n') unless parsed.success?
def collect(node, result)
  fields = node.deconstruct_keys(nil)
  if [:string_node, :symbol_node, :integer_node, :float_node,
      :local_variable_read_node, :local_variable_write_node].include?(node.type)
    kept = fields.reject { |k,v| k == :node_id || k == :location || k.to_s.end_with?('_loc') || v.is_a?(Prism::Node) }
    kept = kept.map { |k,v| [k, v.is_a?(String) ? [v.bytes, v.encoding.name] : v] }
    result << [node.type, node.send(:flags), kept]
  end
  node.compact_child_nodes.each { |child| collect(child, result) }
end
result = []; collect(parsed.value, result); p result
"#;
#[test]
fn explicit_prism34_decoded_values_flags_encodings_and_local_identities_match() {
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
    let mut sources = [
        b"# frozen_string_literal: true\nresult = \"%s/%s\" % [\"a\", \"\\xff\"]\np result\n".as_slice(),
        b"# frozen_string_literal: false\nresult = \"%<a>s\" % {a: \"x\"}\np result\n",
        b"\xef\xbb\xbf# encoding: UTF-8\r\nresult = sprintf(\"%s\", \"caf\xc3\xa9\")\r\np result\r\n",
        b"# encoding: Windows-31J\nresult = sprintf(\"%s\", \"\x83\x5c\")\np result\n",
        b"# encoding: US-ASCII\nresult = \"%s\" % \"\\u{e9}\"\np result\n",
    ].into_iter().map(Vec::from).collect::<Vec<_>>();
    sources.extend(whitespace_token_expressions().into_iter().map(|expr| {
        let separator = if expr.ends_with('\r') { "\t" } else { "" };
        format!("result = {expr}{separator}\np result\n").into_bytes()
    }));
    for source in sources {
        let source = source.as_slice();
        fs::write(&path, source).unwrap();
        let Some(before) = snapshot() else {
            eprintln!("SKIP explicit Prism 3.4: Ruby/Prism unavailable");
            return;
        };
        let after = fix(d.path(), source, 0);
        assert_ne!(source, after);
        assert_eq!(
            Some(before),
            snapshot(),
            "{:?}",
            String::from_utf8_lossy(source)
        );
    }
}
#[test]
fn uncertain_literal_peers_compose_without_disguising_withheld_formatting() {
    let d = project();
    let source =
        "[sprintf('%s', ?x), \"#@name\", +\"#{1}\", \"#{true ? 'yes' : nil}\", %x(not_executed)]\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    for _ in 0..6 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        cli(d.path(), &["--fix", "a.rb"], 1);
        if old == fs::read(d.path().join("a.rb")).unwrap() {
            break;
        }
    }
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(after.contains("sprintf(\"%s\", \"x\")"), "{after}");
    assert!(
        after.contains("#{@name}") && after.contains("`not_executed`"),
        "{after}"
    );
    let result = cli(d.path(), &["--only", COP, "a.rb"], 1);
    assert!(String::from_utf8_lossy(&result.stdout).contains("Favor `format` over `sprintf`."));
}
#[test]
fn runtime_actual_defaults_and_selected_peers_preserve_values_each_pass() {
    if !has_ruby() {
        return;
    }
    let d = project();
    let source = "values = [sprintf('%s', ?x), '%s' % [%Q(hi)], '%<a>s' % {:a => 'x'}, %W(a b), %q(hi)]\np values\n";
    let before = ruby("ruby", source);
    assert!(before.status.success());
    for selected in [
        None,
        Some(
            "Style/FormatString,Style/StringLiterals,Style/HashSyntax,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/PercentLiteralDelimiters,Layout",
        ),
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
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
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            let observed = ruby("ruby", &after);
            assert_eq!(before.stdout, observed.stdout, "{selected:?}: {after}");
            assert_eq!(before.stderr, observed.stderr);
            assert_eq!(before.status, observed.status);
            if after.as_bytes() == old {
                stable = true;
                break;
            }
        }
        assert!(stable);
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(!after.contains("sprintf"), "{after}");
        assert!(after.contains("format("), "{after}");
    }
}
#[test]
fn actual_default_and_literal_original_style_layout_composition_is_bounded() {
    let d = project();
    let selected = LITERAL_PEERS;
    // Source only: never execute this curated composition fixture.
    let source = "[sprintf('%s', ?x), '%s' % [%Q(hi)], '%<a>s' % { :a => 'x' }, %W(a b), %q(hello), %Q[hello], %!hello!, :'word', \"\\#\"]\n";
    for selectors in [None, Some(selected)] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut previous = source.as_bytes().to_vec();
        let mut stable = false;
        for _ in 0..6 {
            let mut args = vec!["--fix", "a.rb"];
            if let Some(s) = selectors {
                args.extend(["--only", s]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            let after = fs::read(d.path().join("a.rb")).unwrap();
            if after == previous {
                stable = true;
                break;
            }
            previous = after;
        }
        assert!(
            stable,
            "{selectors:?}: {}",
            String::from_utf8_lossy(&previous)
        );
        let text = String::from_utf8(previous).unwrap();
        assert!(text.contains("format(\"%s\", \"x\")"), "{text}");
        assert!(text.contains("format(\"%s\", \"hi\")"), "{text}");
        // Nonempty hash interiors retain their original leading space.
        assert!(text.contains("format(\"%<a>s\",  a: \"x\")"), "{text}");
        assert!(!text.contains("sprintf"), "{text}");
    }
}
