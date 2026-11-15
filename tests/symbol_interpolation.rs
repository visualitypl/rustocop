//! real CLI controls and independent, controlled Ruby observations.
use crate::support::source_getter;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const Q: &str = "Style/QuotedSymbols";
const S: &str = "Style/SymbolLiteral";
const V: &str = "Style/VariableInterpolation";
const ALL: &str = "Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation";
const CASES: [(&str, &str, &str); 3] = [
    (Q, "p :'a-b'\n", "p :\"a-b\"\n"),
    (S, "p :\"word\"\n", "p :word\n"),
    (V, "p \"#@word\"\n", "p \"#{@word}\"\n"),
];
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
fn unchanged(dir: &Path, name: &str, source: &[u8], status: i32) {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let mtime = fs::metadata(&p).unwrap().modified().unwrap();
    cli(dir, &["--only", name, "--fix", "a.rb"], status);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), mtime);
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap()
}
fn same_runtime(before: &Output, after: &Output) {
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.status, after.status, "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn registered_default_safe_and_lint_only_never_writes() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (name, before, _) in CASES {
        let cops: Vec<_> = r.cops().iter().filter(|c| c.name() == name).collect();
        assert_eq!(cops.len(), 1);
        assert!(cops[0].default_enabled() && cops[0].safe_fixes());
        assert!(cops[0].path_policy().is_none());
        assert_eq!(cops[0].ruby_versions(), Default::default());
        fs::write(d.path().join("a.rb"), before).unwrap();
        let mtime = fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        for args in [vec!["a.rb"], vec!["--only", name, "a.rb"]] {
            let out = cli(d.path(), &args, 1);
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(name),
                "{out:?}"
            );
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), before);
            assert_eq!(
                fs::metadata(d.path().join("a.rb"))
                    .unwrap()
                    .modified()
                    .unwrap(),
                mtime
            );
        }
    }
}
#[test]
fn selectors_enablement_and_canonical_single_pass_fixes() {
    let d = project();
    let p = d.path().join("a.rb");
    for (name, before, after) in CASES {
        for config in [
            // Style/StringLiterals stays on: disabled, it means single quotes.
            "[cops.Style]\nenabled = false\n[cops.\"Style/StringLiterals\"]\nenabled = true\n"
                .to_owned(),
            CASES
                .iter()
                .map(|(n, _, _)| format!("[cops.\"{n}\"]\nenabled = false\n"))
                .collect(),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(&p, before).unwrap();
            cli(
                d.path(),
                &["--only", ALL, "--except", name, "--fix", "a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(&p).unwrap(), before);
            cli(d.path(), &["--only", name, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(&p).unwrap(), after);
            let mtime = fs::metadata(&p).unwrap().modified().unwrap();
            cli(d.path(), &["--only", name, "--fix", "a.rb"], 0);
            assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), mtime);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"Style/StringLiterals\"]\nenabled = true\n[cops.\"{name}\"]\nenabled = true\n"),
        )
        .unwrap();
        fs::write(&p, before).unwrap();
        cli(d.path(), &["--fix", "a.rb"], 0);
        assert_eq!(fs::read_to_string(&p).unwrap(), after);
    }
}
#[test]
fn quotes_follow_the_configured_string_cop_and_not_the_selection() {
    // As in RuboCop: the quotes of Style/StringLiterals, and single quotes
    // when that cop is disabled. `--only` leaves the config as it is.
    let d = project();
    for (config, before, after) in [
        ("", "p :'a-b'\n", "p :\"a-b\"\n"),
        (
            "[cops.\"Style/StringLiterals\"]\nenabled = false\n",
            "p :\"a-b\"\n",
            "p :'a-b'\n",
        ),
        (
            "[cops.Style]\nenabled = false\n",
            "p :\"a-b\"\n",
            "p :'a-b'\n",
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        fs::write(d.path().join("a.rb"), before).unwrap();
        cli(d.path(), &["--only", Q, "--fix", "a.rb"], 0);
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
    }
}
#[test]
fn directives_excludes_stdin_and_strict_options() {
    let d = project();
    for (name, before, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [name, name.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{before}"),
                    format!("{} # {prefix}:disable {target}\n", before.trim_end()),
                    format!("# {prefix}:disable-next {target}\n{before}"),
                ] {
                    unchanged(d.path(), name, source.as_bytes(), 0);
                }
            }
        }
        for config in [
            "exclude = [\"a.rb\"]\n".into(),
            format!("[cops.\"{name}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), name, before.as_bytes(), 0);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", name, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        assert_eq!(c.wait_with_output().unwrap().status.code(), Some(1));
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), before);
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{name}\"]\nEnforcedStyle = \"other\"\n"),
        )
        .unwrap();
        unchanged(d.path(), name, before.as_bytes(), 2);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn unicode_positions_and_no_write_clean_error_or_boundary_hazard() {
    let d = project();
    for (name, src, col) in [
        (Q, "p \"é\"; :'a-b'\n", 8),
        (S, "p \"é\"; :\"word\"\n", 8),
        (V, "p \"é#@word\"\n", 6),
    ] {
        fs::write(d.path().join("a.rb"), src).unwrap();
        let o = cli(d.path(), &["--only", name, "a.rb"], 1);
        assert!(
            String::from_utf8_lossy(&o.stdout).contains(&format!("a.rb:1:{col}:")),
            "{o:?}"
        );
    }
    for (name, src, status) in [
        (ALL, "p [:word, :\"two words\", :'\\n', '\\#@x']\n", 0),
        (ALL, "p :\"unterminated\n", 2),
        (S, "p :\"a\"?1:2\n", 1),
        (S, "p :\"a\"and true\n", 1),
    ] {
        unchanged(d.path(), name, src.as_bytes(), status);
    }
}
#[test]
fn multiline_symbol_and_label_directive_envelopes_are_atomic() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("p :'a\n' # {prefix}:disable {Q}\n"),
            format!("p 'a\nb': 1 # {prefix}:disable {Q}\n"),
        ] {
            unchanged(d.path(), Q, source.as_bytes(), 0);
        }
    }
}
#[test]
fn unknown_escape_quote_switches_have_parseable_runtime_counterexamples() {
    if !has_ruby() {
        return;
    }
    let d = project();
    for escape in ["q", "v", "8"] {
        let before = format!("p :'a\\{escape}'.to_s.bytes\n");
        fs::write(d.path().join("a.rb"), &before).unwrap();
        let original = ruby(d.path());
        cli(d.path(), &["--only", Q, "--fix", "a.rb"], 0);
        same_runtime(&original, &ruby(d.path()));
        let naive = format!("p :\"a\\{escape}\".to_s.bytes\n");
        assert_eq!(ruby_prism::parse(naive.as_bytes()).errors().count(), 0);
        fs::write(d.path().join("a.rb"), naive).unwrap();
        let changed = ruby(d.path());
        assert!(changed.status.success());
        assert_ne!(original.stdout, changed.stdout);
    }
}
#[test]
fn known_source_observers_keep_diagnostics_but_no_edits() {
    let d = project();
    for (name, source, _) in CASES {
        for observer in [
            "p __FILE__",
            "p File.binread($0)",
            "p method(:p).source_location",
            "p errors.map(&:backtrace_locations)",
            "p RubyVM::AbstractSyntaxTree",
        ] {
            unchanged(
                d.path(),
                name,
                format!("{source}{observer}\n").as_bytes(),
                1,
            );
        }
    }
}
#[test]
fn corrections_never_create_early_heredoc_terminators() {
    let d = project();
    for (cop, setup, original, terminator, expected) in [
        (V, "@x = \"abc\"\n", "#@x", "#{@x}", "\"abc\\n\"\n"),
        (S, "", "#{:\"word\"}", "#{:word}", "\"word\\n\"\n"),
        (
            V,
            "@x = \"a\"\n@y = \"b\"\n",
            "#@x#@y",
            "#{@x}#{@y}",
            "\"ab\\n\"\n",
        ),
        (
            S,
            "",
            "#{:\"word\"}#{:\"tail\"}",
            "#{:word}#{:tail}",
            "\"wordtail\\n\"\n",
        ),
        (
            ALL,
            "@x = \"a\"\n",
            "#@x#{:\"word\"}",
            "#{@x}#{:word}",
            "\"aword\\n\"\n",
        ),
    ] {
        for (op, indent) in [("<<", ""), ("<<-", "  "), ("<<~", "\t")] {
            for newline in ["\n", "\r\n"] {
                // For <<- keep body text unindented; its indentation is data.
                let body_indent = if op == "<<~" { indent } else { "" };
                let source = format!(
                    "{setup}p {op}\"{terminator}\"\n{body_indent}{original}\n{indent}{terminator}\n"
                )
                .replace('\n', newline);
                for selected in [Some(cop), Some(ALL), None] {
                    fs::write(d.path().join("a.rb"), &source).unwrap();
                    let before = has_ruby().then(|| ruby(d.path()));
                    if let Some(before) = &before {
                        assert!(before.status.success(), "{source}: {before:?}");
                        assert_eq!(before.stdout, expected.as_bytes(), "{source}");
                    }
                    if let Some(selected) = selected {
                        unchanged(d.path(), selected, source.as_bytes(), 1);
                    } else {
                        let mtime = fs::metadata(d.path().join("a.rb"))
                            .unwrap()
                            .modified()
                            .unwrap();
                        // Historical no-write witness assumes fixed labels;
                        // The heredoc delimiter cops separately prove default label-renaming composition.
                        // The heredoc indentation cops would re-indent
                        // these witnesses; their own tests prove those edits.
                        cli(
                            d.path(),
                            &[
                                "--except",
                                "Naming/HeredocDelimiterCase,Layout/HeredocIndentation,Layout/ClosingHeredocIndentation",
                                "--fix",
                                "a.rb",
                            ],
                            1,
                        );
                        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), source.as_bytes());
                        assert_eq!(
                            fs::metadata(d.path().join("a.rb"))
                                .unwrap()
                                .modified()
                                .unwrap(),
                            mtime
                        );
                    }
                    if let Some(before) = before {
                        same_runtime(&before, &ruby(d.path()));
                    }
                }
            }
        }
    }
}
#[test]
fn heredoc_guard_handles_nested_and_command_literals_without_execution() {
    let d = project();
    for source in [
        "p <<~`#{@x}`\n#@x\n#{@x}\n",
        "p <<~`#{:word}`\n#{:\"word\"}\n#{:word}\n",
    ] {
        // Commands are deliberately never executed, even as validation.
        unchanged(d.path(), ALL, source.as_bytes(), 1);
    }
    // The central physical-line proof can now establish that the nested quotes
    // remain a witness: this line cannot become the outer terminator.
    fs::write(d.path().join("a.rb"), "p <<~\"#{@x}\"\n#{\"#@x\"}\n#{@x}\n").unwrap();
    cli(d.path(), &["--only", ALL, "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "p <<~\"#{@x}\"\n#{\"#{@x}\"}\n#{@x}\n"
    );
}
#[test]
fn ordinary_quoted_heredocs_still_receive_canonical_corrections() {
    let d = project();
    for delimiter in ["X", "\"X\""] {
        let source = format!("@x = 1\np <<~{delimiter}\n  #@x #{{:\"word\"}}\nX\n");
        fs::write(d.path().join("a.rb"), &source).unwrap();
        let before = has_ruby().then(|| ruby(d.path()));
        cli(d.path(), &["--only", ALL, "--fix", "a.rb"], 0);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            source.replace("#@x", "#{@x}").replace(":\"word\"", ":word")
        );
        if let Some(before) = before {
            same_runtime(&before, &ruby(d.path()));
        }
    }
}
#[test]
fn bom_crlf_heredoc_indentation_line_and_data_bytes_survive() {
    let d = project();
    let src = "\u{feff}# encoding: UTF-8\r\n@word = 1\r\np [:'café', :\"word\"]\r\np <<~X\r\n  #@word\r\n  :'not-code'\r\nX\r\np __LINE__\r\n__END__\r\n#@raw :\"data\"\r\n";
    fs::write(d.path().join("a.rb"), src).unwrap();
    let before = has_ruby().then(|| ruby(d.path()));
    cli(d.path(), &["--only", ALL, "--fix", "a.rb"], 0);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(after.contains("p [:\"café\", :word]\r\n"), "{after}");
    assert!(after.contains("  #{@word}\r\n  :'not-code'\r\n"));
    assert!(after.starts_with('\u{feff}'));
    assert!(after.split_inclusive('\n').all(|l| l.ends_with("\r\n")));
    assert_eq!(src.split("__END__").nth(1), after.split("__END__").nth(1));
    assert_eq!(
        src.bytes().filter(|&b| b == b'\n').count(),
        after.bytes().filter(|&b| b == b'\n').count()
    );
    if let Some(before) = before {
        same_runtime(&before, &ruby(d.path()));
    }
}
#[test]
fn legacy_encodings_keep_non_utf8_envelopes_and_fix_ascii_spans() {
    let d = project();
    for (name, src) in [
        (Q, b"# encoding: ISO-8859-1\np :'caf\xe9'\n".as_slice()),
        (V, b"# encoding: ISO-8859-1\np \"#@caf\xe9\"\n".as_slice()),
        (Q, b"# encoding: Windows-31J\np :'\x83\x5c'\n".as_slice()),
    ] {
        unchanged(d.path(), name, src, 1);
    }
    for (name, src, after) in [
        (
            Q,
            b"# encoding: ISO-8859-1\np :'a-b'\n".as_slice(),
            b"# encoding: ISO-8859-1\np :\"a-b\"\n".as_slice(),
        ),
        (
            S,
            b"# encoding: ISO-8859-1\np :\"word\"\n".as_slice(),
            b"# encoding: ISO-8859-1\np :word\n".as_slice(),
        ),
        (
            V,
            b"# encoding: ISO-8859-1\np \"caf\xe9 #@word\"\n".as_slice(),
            b"# encoding: ISO-8859-1\np \"caf\xe9 #{@word}\"\n".as_slice(),
        ),
    ] {
        fs::write(d.path().join("a.rb"), src).unwrap();
        let before = has_ruby().then(|| ruby(d.path()));
        cli(d.path(), &["--only", name, "--fix", "a.rb"], 0);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
        if let Some(before) = before {
            same_runtime(&before, &ruby(d.path()));
        }
    }
    // The trailing 5c of ソ is not a backslash; a REAL following backslash
    // still activates the single-quoted escape lookalike and is accepted.
    unchanged(
        d.path(),
        Q,
        b"# encoding: Windows-31J\np :'\x83\x5c\\n'\n",
        0,
    );
}
#[test]
fn commands_are_ast_and_source_only_never_executed() {
    let d = project();
    let src = "[%x[not_executed #@x], `not_executed #$x`, %x(#{:'a-b'})]\n";
    fs::write(d.path().join("a.rb"), src).unwrap();
    cli(d.path(), &["--only", ALL, "--fix", "a.rb"], 0);
    let after = fs::read(d.path().join("a.rb")).unwrap();
    assert_eq!(
        after,
        b"[%x[not_executed #{@x}], `not_executed #{$x}`, %x(#{:\"a-b\"})]\n"
    );
    for source in [src.as_bytes(), &after] {
        let p = ruby_prism::parse(source);
        assert_eq!(p.errors().count(), 0);
        let a = p
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap();
        assert!(
            a.as_array_node()
                .unwrap()
                .elements()
                .iter()
                .all(|n| n.as_interpolated_x_string_node().is_some())
        );
    }
}
#[test]
fn controlled_runtime_preserves_symbol_values_and_encodings_for_unusual_escapes() {
    if !has_ruby() {
        return;
    }
    let d = project();
    for header in [
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        let src = format!(
            r#"{header}symbols = [:'a', :"word", :'a-b', :'o\'clock', :'foo\\bar', :'a\q', :'a\v', :'a\8', :'a\\\n', :'café', :'if', :'BEGIN', :'__END__', :"foo?", :"+", :"\u0041", :"\x7f", :'#@literal', :'\n']
p symbols.map {{ |s| [s.to_s.bytes, s.encoding.name, s.frozen?] }}
p [{{ 'a-b': 1 }}, {{ :'a' => 2 }}]
p __LINE__
"#
        );
        for name in [Q, S, ALL] {
            fs::write(d.path().join("a.rb"), &src).unwrap();
            let before = ruby(d.path());
            // Overlapping quote/plain-symbol edits defer deterministically;
            // the public CLI intentionally performs just one pass.
            cli(
                d.path(),
                &["--only", name, "--fix", "--fix-passes", "1", "a.rb"],
                i32::from(name == ALL),
            );
            assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), src);
            same_runtime(&before, &ruby(d.path()));
        }
    }
}
#[test]
fn controlled_runtime_preserves_interpolation_count_order_backrefs_and_regexp_o() {
    if !has_ruby() {
        return;
    }
    let d = project();
    for frozen in ["true", "false"] {
        let src = format!(
            r##"# frozen_string_literal: {frozen}
$events = []
class Value
  def initialize(id)
    @id = id
  end
  def to_s
    $events << @id
    @id.to_s
  end
end
@x = Value.new(1)
$g = Value.new(2)
class Frame
  @@c = Value.new(3)
  def run
    ["#@@c", :"#@@c", %W[x#@@c], %I[x#@@c], /#@@c/]
  end
end
/(a)(b)/ =~ "ab"
values = ["#@x#$g#@x", :"#@x#$g", %W[#@x #$g], %I[#@x #$g], /#@x#$g/i,
          "#$1|#$2|#$+|#$&|#$`|#$'", "\#@x", '#$g', "#{{'inner'}}#@x"]
values += Frame.new.run
2.times do |i|
  @cached = Value.new(i + 4)
  values << /#@cached/o
end
values << <<~X
  #@x-#$g
  '#@x'
X
values.each do |v|
  if v.is_a?(Array)
    p v.map {{ |s| [s.to_s.bytes, s.encoding.name, s.frozen?] }}
  elsif v.is_a?(Regexp)
    p [v.source.bytes, v.encoding.name, v.options]
  else
    p [v.to_s.bytes, v.encoding.name, v.frozen?]
  end
end
p [$events, __LINE__]
"##
        );
        let src = source_getter::pattern_sibling(d.path(), "a.rb", &src, V);
        fs::write(d.path().join("a.rb"), &src).unwrap();
        let before = ruby(d.path());
        cli(d.path(), &["--only", V, "--fix", "a.rb"], 0);
        assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), src);
        same_runtime(&before, &ruby(d.path()));
    }
}
#[test]
fn fixed_34_binding_literals_flags_and_interpolation_reads_match() {
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Shape(Vec<String>);
    impl<'pr> NodeVisitor<'pr> for Shape {
        fn enter(&mut self, n: ruby_prism::Node<'pr>) {
            if !matches!(
                n,
                ruby_prism::Node::EmbeddedVariableNode { .. }
                    | ruby_prism::Node::EmbeddedStatementsNode { .. }
                    | ruby_prism::Node::StatementsNode { .. }
            ) {
                self.0.push(format!("{:?}", std::mem::discriminant(&n)));
            }
            if let Some(s) = n.as_symbol_node() {
                self.0.push(format!(
                    "sym:{:?}:{}:{}:{}",
                    s.unescaped(),
                    s.is_forced_utf8_encoding(),
                    s.is_forced_binary_encoding(),
                    s.is_forced_us_ascii_encoding()
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
                    .push(format!("local:{:?}:{}", s.name().as_slice(), s.depth()));
            }
            if let Some(s) = n.as_call_node() {
                self.0.push(format!("call:{:?}", s.name().as_slice()));
            }
            if let Some(s) = n.as_instance_variable_read_node() {
                self.0.push(format!("ivar:{:?}", s.name().as_slice()));
            }
            if let Some(s) = n.as_global_variable_read_node() {
                self.0.push(format!("global:{:?}", s.name().as_slice()));
            }
        }
    }
    let d = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
        "# shareable_constant_value: literal\n",
    ] {
        let src = format!(
            r##"{header}x = 1
xs.map {{ [:'a', :'o\'clock', :'foo\\bar', :'a\q', :'café', :"word", "#@x#{{x}}", /#$g/o, %I[#@x], %W[#$g], "#$_#$.#{{it}}"] }}
xs.map {{ "#@x#{{_1}}" }}
"##
        );
        for selected in [Q, S, V, ALL] {
            fs::write(d.path().join("a.rb"), &src).unwrap();
            cli(
                d.path(),
                &["--only", selected, "--fix", "--fix-passes", "1", "a.rb"],
                i32::from(selected == ALL),
            );
            let after = fs::read(d.path().join("a.rb")).unwrap();
            let a = ruby_prism::parse(src.as_bytes());
            let b = ruby_prism::parse(&after);
            assert_eq!(a.errors().count(), 0);
            assert_eq!(b.errors().count(), 0);
            let mut before = Shape::default();
            let mut corrected = Shape::default();
            walk(&mut before, &a.node());
            walk(&mut corrected, &b.node());
            assert_eq!(
                before.0,
                corrected.0,
                "{selected}: {}",
                String::from_utf8_lossy(&after)
            );
        }
    }
}
#[test]
fn curated_default_and_peer_composition_stabilizes_within_six_explicit_passes() {
    if !has_ruby() {
        return;
    }
    let d = project();
    let src = "@word = 1\ndef f\n  return ({ :'word' => ?x, :'a-b' => %q(hi) });\nend\np [f, %Q(hi), %Q(#{1}), %W(one two), %q(#{not_called}), :\"word\", :'o\\'clock', \"#@word\"]\n";
    let peers = format!(
        "{ALL},Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout"
    );
    for selected in [Some(peers.as_str()), None] {
        fs::write(d.path().join("a.rb"), src).unwrap();
        let before = ruby(d.path());
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
            same_runtime(&before, &ruby(d.path()));
            if old == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert_ne!(after, src);
        for expected in ["\"a-b\":", "%w[one two]", "\"#{@word}\"", ":word"] {
            assert!(after.contains(expected), "{after}");
        }
    }
}
