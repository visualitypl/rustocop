//! real CLI and independent literal semantic observations.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const Q: &str = "Style/RedundantPercentQ";
const W: &str = "Style/RedundantCapitalW";
const CHAR: &str = "Style/CharacterLiteral";
const ALL: &str = "Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral";
const CASES: [(&str, &str, &str); 3] = [
    (Q, "p %q(hi)\n", "p 'hi'\n"),
    (W, "p %W(cat dog)\n", "p %w(cat dog)\n"),
    (CHAR, "p ?x\n", "p 'x'\n"),
];
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
fn unchanged(dir: &Path, name: &str, source: &[u8], status: i32) {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir, &["--only", name, "--fix", "a.rb"], status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap()
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}

#[test]
fn registry_defaults_versions_and_lint_only_never_write() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    for (name, before, _) in CASES {
        let cops: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == name)
            .collect();
        assert_eq!(cops.len(), 1);
        assert!(cops[0].safe_fixes() && cops[0].default_enabled());
        assert!(cops[0].path_policy().is_none());
        assert_eq!(cops[0].ruby_versions(), Default::default());
        fs::write(dir.path().join("a.rb"), before).unwrap();
        let mtime = fs::metadata(dir.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        let out = cli(dir.path(), &["a.rb"], 1);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(name),
            "{out:?}"
        );
        cli(dir.path(), &["--only", name, "a.rb"], 1);
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), before);
        assert_eq!(
            fs::metadata(dir.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            mtime
        );
    }
}

#[test]
fn selectors_config_and_canonical_single_pass_fixes() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (name, before, after) in CASES {
        for config in [
            CASES
                .iter()
                .map(|(cop, _, _)| format!("[cops.\"{cop}\"]\nenabled = false\n"))
                .collect::<String>(),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            cli(
                dir.path(),
                &["--only", ALL, "--except", name, "--fix", "a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            cli(dir.path(), &["--only", name, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(&path).unwrap(), after);
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", name, "--fix", "a.rb"], 0);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
        )
        .unwrap();
        fs::write(&path, before).unwrap();
        cli(dir.path(), &["--fix", "a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }
}

#[test]
fn directives_excludes_and_stdin_preserve_bytes() {
    let dir = project();
    for (name, before, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [name, name.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{before}"),
                    format!("{} # {prefix}:disable {target}\n", before.trim_end()),
                    format!("# {prefix}:disable-next {target}\n{before}"),
                ] {
                    unchanged(dir.path(), name, source.as_bytes(), 0);
                }
            }
        }
        for config in [
            "exclude = [\"a.rb\"]\n".into(),
            format!("[cops.\"{name}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(dir.path(), name, before.as_bytes(), 0);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", name, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), before);
    }
}

#[test]
fn individual_uncertain_edits_clean_errors_and_source_observers_never_write() {
    let dir = project();
    for (name, source, status) in [
        (Q, r#"%Q(\"hi\")"#, 1),
        (Q, "%q(it's #@name)", 1),
        (Q, "%q(it's #$name)", 1),
        (Q, "%Q\nhello\n", 1),
        (W, r#"%W(\"hi\")"#, 1),
        (CHAR, "?\\\n", 1),
        (ALL, "p %q(it's #{foo}); p %W(#{foo}); p ?\\C-a\n", 0),
        (ALL, "p %Q(\n", 2),
    ] {
        unchanged(dir.path(), name, source.as_bytes(), status);
    }
    for (name, source, _) in CASES {
        for observer in [
            "p __FILE__",
            "p errors.map(&:backtrace_locations)",
            "p method(:p).source_location",
            "p RubyVM::AbstractSyntaxTree",
            "p File.binread($0)",
        ] {
            unchanged(
                dir.path(),
                name,
                format!("{source}{observer}\n").as_bytes(),
                1,
            );
        }
    }
}

#[test]
fn disabled_multiline_envelopes_remain_atomic() {
    let dir = project();
    unchanged(
        dir.path(),
        Q,
        b"%Q(\n#{1 # rubocop:disable Style/RedundantPercentQ\n}\n)\n",
        0,
    );
    // W edits only its prefix, but the offense spans the complete array.
    unchanged(
        dir.path(),
        W,
        b"%W(\nhi\n) # rubocop:disable Style/RedundantCapitalW\n",
        0,
    );
}

#[test]
fn bom_crlf_heredoc_indentation_line_and_data_payloads_survive() {
    let dir = project();
    let source = "\u{feff}# encoding: UTF-8\r\np [%q(café), %W(a b), ?é]\r\np <<~X\r\n  %q(not code)\r\n  #{%Q(#{1})}\r\nX\r\np __LINE__\r\n__END__\r\n%q(raw)\r\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    cli(dir.path(), &["--only", ALL, "--fix", "a.rb"], 0);
    let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
    assert!(after.contains("p ['café', %w(a b), 'é']\r\n"), "{after}");
    assert!(after.contains("  %q(not code)\r\n  #{\"#{1}\"}\r\n"));
    assert_eq!(
        source.split("__END__").nth(1),
        after.split("__END__").nth(1)
    );
    assert!(after.starts_with('\u{feff}'));
    assert_eq!(
        source.bytes().filter(|&b| b == b'\n').count(),
        after.bytes().filter(|&b| b == b'\n').count()
    );
    assert!(
        after
            .split_inclusive('\n')
            .all(|line| line.ends_with("\r\n"))
    );
}

#[test]
fn legacy_encoding_envelopes_are_withheld_but_ascii_prefixes_apply() {
    let dir = project();
    unchanged(dir.path(), Q, b"# encoding: ISO-8859-1\np %q(caf\xe9)\n", 1);
    unchanged(dir.path(), CHAR, b"# encoding: ISO-8859-1\np ?\xe9\n", 1);
    let before = b"# encoding: ISO-8859-1\np %W(caf\xe9)\n";
    fs::write(dir.path().join("a.rb"), before).unwrap();
    cli(dir.path(), &["--only", W, "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read(dir.path().join("a.rb")).unwrap(),
        b"# encoding: ISO-8859-1\np %w(caf\xe9)\n"
    );
    if has_ruby() {
        let after = ruby(dir.path());
        fs::write(dir.path().join("a.rb"), before).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}

#[test]
fn windows_31j_trailing_backslash_bytes_are_not_ruby_escapes() {
    let dir = project();
    // ソ is 83 5c in Windows-31J: its second byte is not an escape.
    for (name, literal, fixable) in [
        (Q, b"%q(\x83\x5c)".as_slice(), false),
        (Q, b"%Q(\"\x83\x5c\")".as_slice(), false),
        (Q, b"%Q(\"\x83\x5c\nb\\n\")".as_slice(), false),
        (W, b"%W(\x83\x5c)".as_slice(), true),
        (W, b"%W(\x83\x5c\\\\)".as_slice(), true),
    ] {
        let source = [b"# encoding: Windows-31J\np ", literal, b"\n"].concat();
        fs::write(dir.path().join("a.rb"), &source).unwrap();
        let out = cli(dir.path(), &["--only", name, "a.rb"], 1);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("a.rb:2:3:"), "{stdout}");
        assert!(stdout.contains(name), "{stdout}");
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), source);
        if fixable {
            let before = has_ruby().then(|| ruby(dir.path()));
            cli(dir.path(), &["--only", name, "--fix", "a.rb"], 0);
            let mut expected = source.clone();
            let prefix = expected.windows(2).position(|w| w == b"%W").unwrap();
            expected[prefix + 1] = b'w';
            assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), expected);
            if let Some(before) = before {
                let after = ruby(dir.path());
                assert!(before.status.success() && after.status.success());
                assert_eq!(before.stdout, after.stdout);
                assert_eq!(before.stderr, after.stderr);
            }
        } else {
            unchanged(dir.path(), name, &source, 1);
        }
    }
    // A real backslash immediately after ソ still activates the escape rules.
    for (name, literal) in [
        (Q, b"%q(\x83\x5c\\n)".as_slice()),
        (Q, b"%Q(\"\x83\x5c\\n\")".as_slice()),
        (W, b"%W(\x83\x5c\\n)".as_slice()),
    ] {
        let source = [b"# encoding: Windows-31J\np ", literal, b"\n"].concat();
        unchanged(dir.path(), name, &source, 0);
    }
}

#[test]
fn source_only_command_literals_keep_command_semantics_without_execution() {
    let dir = project();
    unchanged(
        dir.path(),
        ALL,
        b"[%x(not_executed), %x(#{value}), `%q(hi)`, `%W(a b)`]\n",
        0,
    );
    let source = "[%x(#{%Q(hi)}), `#{?x}`]\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    cli(dir.path(), &["--only", ALL, "--fix", "a.rb"], 0);
    let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
    assert_eq!(after, "[%x(#{\"hi\"}), `#{'x'}`]\n");
    for text in [source, &after] {
        let parsed = ruby_prism::parse(text.as_bytes());
        let values = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap();
        assert!(
            values
                .as_array_node()
                .unwrap()
                .elements()
                .iter()
                .all(|n| n.as_interpolated_x_string_node().is_some())
        );
    }
}

#[test]
fn controlled_ruby_preserves_bytes_encodings_frozen_mutable_and_interpolation_order() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for frozen in ["true", "false"] {
        let source = format!(
            r#"# frozen_string_literal: {frozen}
$events = []
def mark(x)
  $events << x
  x
end
def literal
  ?a
end
values = [%q(hi), %q('hi'), %q("hi"), %Q(hi), %Q("hi"),
          %Q(a\)b), %Q!a\!b!, %Q(\u{{41}}), %Q(\xff), %q(\\foo\\),
          %Q(#{{mark(1)}}-#{{mark(2)}}), %W(a b), %W(a\\b c),
          %W(a\ b c), %W(a\
b), %W(a\nb), %W(#{{mark(3)}}),
          ?é, ?😀, ?x, ?\n, ?\t, ?\s, ?\\, ?\', ?", ?', ?\#, ?\!, ?\é,
          ?\C-\M-d, ?\u{{1f600}}, ?\123, %q(it's #@name), %Q(\"hi\"), %W(\"hi\")]
values.each do |v|
  if v.is_a?(String)
    p [v.bytes, v.encoding.name, v.frozen?]
  else
    p v.map {{ |s| [s.bytes, s.encoding.name, s.frozen?] }}
  end
end
p [literal.equal?(literal), literal.frozen?, $events, __LINE__]
"#
        );
        for selected in [Q, W, CHAR, ALL] {
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let before = ruby(dir.path());
            assert!(before.status.success(), "{before:?}");
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", selected, "--fix", "a.rb"])
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            let after = ruby(dir.path());
            assert_eq!(before.status, after.status);
            assert_eq!(before.stdout, after.stdout, "{selected}: {source}");
            assert_eq!(before.stderr, after.stderr, "{selected}");
            assert_ne!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        }
    }
}

#[test]
fn withheld_naive_quote_and_word_switches_have_runtime_counterexamples() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for (name, original, naive) in [
        (Q, "p %q(it's #@name)\n", "p \"it's #@name\"\n"),
        (Q, "p %Q(\\\"hi\\\")\n", "p '\\\"hi\\\"'\n"),
        (W, "p %W(\\\"hi\\\")\n", "p %w(\\\"hi\\\")\n"),
        (CHAR, "p ?\\\n", "p \"\\\n\"\n"),
    ] {
        unchanged(dir.path(), name, original.as_bytes(), 1);
        let before = ruby(dir.path());
        fs::write(dir.path().join("a.rb"), naive).unwrap();
        let after = ruby(dir.path());
        assert!(
            before.status.success() && after.status.success(),
            "{before:?}: {after:?}"
        );
        assert_ne!(before.stdout, after.stdout, "{name}: {original}");
    }
}

#[test]
fn fixed_34_main_parse_binding_payload_and_encoding_flags_match() {
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Shape(Vec<String>);
    impl<'pr> NodeVisitor<'pr> for Shape {
        fn enter(&mut self, n: ruby_prism::Node<'pr>) {
            self.0.push(format!("{:?}", std::mem::discriminant(&n)));
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
            if let Some(n) = n.as_local_variable_read_node() {
                self.0
                    .push(format!("local:{:?}:{}", n.name().as_slice(), n.depth()));
            }
            if let Some(n) = n.as_call_node() {
                self.0.push(format!("call:{:?}", n.name().as_slice()));
            }
        }
        fn leave(&mut self) {
            self.0.push("end".into());
        }
    }
    let dir = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
        "# shareable_constant_value: literal\n",
    ] {
        let source = format!(
            "{header}x = 1\nxs.map {{ [%q(hi), %Q[#{{it}}], %Q[#{{x}}], %W(a b), ?é, ?\\n, ?\\#, ?\\é, ?\\0, ?\\a, ?\\u0041, %Q(\\u{{41}}), %Q(\\xff)] }}\nxs.map {{ [%Q[#{{_1}}], %W(#{{_1}})] }}\n"
        );
        for selected in [Q, W, CHAR, ALL] {
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            cli(dir.path(), &["--only", selected, "--fix", "a.rb"], 0);
            let after = fs::read(dir.path().join("a.rb")).unwrap();
            let original = ruby_prism::parse(source.as_bytes());
            let corrected = ruby_prism::parse(&after);
            assert_eq!(original.errors().count(), 0);
            assert_eq!(corrected.errors().count(), 0);
            let mut a = Shape::default();
            let mut b = Shape::default();
            walk(&mut a, &original.node());
            walk(&mut b, &corrected.node());
            assert_eq!(a.0, b.0, "{selected}: {}", String::from_utf8_lossy(&after));
        }
    }
}

#[test]
fn data_positions_withhold_length_changes_and_keep_equal_length_word_fixes() {
    let dir = project();
    for (name, before, _) in CASES {
        let source = format!("{before}p DATA.pos; p DATA.read.bytes\n__END__\nraw\\payload\n");
        fs::write(dir.path().join("a.rb"), &source).unwrap();
        let before = has_ruby().then(|| ruby(dir.path()));
        cli(
            dir.path(),
            &["--only", name, "--fix", "a.rb"],
            i32::from(name != W),
        );
        let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
        assert_eq!(after == source, name != W);
        if let Some(before) = before {
            let after = ruby(dir.path());
            assert!(before.status.success());
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
        }
    }
}

#[test]
fn curated_composition_with_115_string_literals_and_existing_style_layout_stabilizes() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let source = "def f\n  return ({ :a => ?x });\nend\np [f, %q(hello), %Q(hi), %Q(#{1}), %W(one two), %q(#{not_called}), %Q(\\u{41}), %Q!a\\!b!, %Q[\\t], ?é, ?\\n]\n";
    let peers = format!(
        "{ALL},Style/BarePercentLiterals,Style/PercentQLiterals,Style/PercentLiteralDelimiters,Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout"
    );
    for selected in [Some(peers.as_str()), None] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success());
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(dir.path().join("a.rb")).unwrap();
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
            let after = ruby(dir.path());
            assert_eq!(before.status, after.status);
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
            if old == fs::read(dir.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
        assert_ne!(after, source);
        assert!(after.contains("%w[one two]"), "{after}");
        assert!(after.contains("'#{not_called}'"), "{after}");
    }
}

#[test]
fn unicode_columns_and_strict_no_options_policy() {
    let dir = project();
    for (name, before, _) in CASES {
        fs::write(dir.path().join("a.rb"), format!("p \"é\"; {before}")).unwrap();
        let out = cli(dir.path(), &["--only", name, "a.rb"], 1);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("a.rb:1:10:"),
            "{out:?}"
        );
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{name}\"]\nEnforcedStyle = \"other\"\n"),
        )
        .unwrap();
        let before = fs::read(dir.path().join("a.rb")).unwrap();
        cli(dir.path(), &["--only", name, "--fix", "a.rb"], 2);
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), before);
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
