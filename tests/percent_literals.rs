//! percent spelling, atomic edits, CLI controls and semantic witnesses.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const BARE: &str = "Style/BarePercentLiterals";
const DELIM: &str = "Style/PercentLiteralDelimiters";
const Q: &str = "Style/PercentQLiterals";
const ALL: &str = "Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals";
const CASES: [(&str, &str, &str); 3] = [
    (BARE, "p %Q(#{value})\n", "p %(#{value})\n"),
    (DELIM, "p %w(one two)\n", "p %w[one two]\n"),
    (Q, "p %Q(hi)\n", "p %q(hi)\n"),
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

#[test]
fn exact_registry_defaults_and_lint_only_never_write() {
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    let path = dir.path().join("a.rb");
    for (name, before, _) in CASES {
        let cops: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == name)
            .collect();
        assert_eq!(cops.len(), 1);
        assert!(cops[0].safe_fixes() && cops[0].default_enabled());
        assert!(cops[0].path_policy().is_none());
        fs::write(&path, before).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let out = cli(dir.path(), &["a.rb"], 1);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(name),
            "{out:?}"
        );
        cli(dir.path(), &["--only", name, "a.rb"], 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn selectors_config_and_canonical_single_pass_fixes() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (name, before, after) in CASES {
        for config in [
            format!(
                "{}[cops.\"Style/RedundantPercentQ\"]\nenabled = false\n[cops.\"Style/RedundantInterpolation\"]\nenabled = false\n",
                CASES
                    .iter()
                    .map(|(cop, _, _)| format!("[cops.\"{cop}\"]\nenabled = false\n"))
                    .collect::<String>()
            ),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            cli(dir.path(), &["--fix", "a.rb"], 0);
            cli(
                dir.path(),
                &["--only", name, "--except", name, "--fix", "a.rb"],
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
    let path = dir.path().join("a.rb");
    for (name, before, _) in CASES {
        for prefix in ["rustocop", "rubocop"] {
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
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }
}

#[test]
fn delimiter_envelope_respects_disabled_body_lines_atomically() {
    let dir = project();
    // A source enable resumes reporting on the final line, but the opening and
    // closing edits must never be split across a disabled interpolation body.
    let source = "%W(\n#{1 # rubocop:disable Style/PercentLiteralDelimiters\n}\n)\n";
    unchanged(dir.path(), DELIM, source.as_bytes(), 0);
    // Q/bare highlights own only their opening token. Their prefix-only edits
    // do not touch a disabled body and retain all physical line positions.
    let source = "%Q(\n#{1 # rubocop:disable Style/BarePercentLiterals\n}\n)\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    cli(dir.path(), &["--only", BARE, "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        source.replacen("%Q", "%", 1)
    );
}

#[test]
fn individual_prefix_source_data_and_encoding_withholds_keep_offenses() {
    let dir = project();
    for source in [
        "foo %Q=hello=\n",
        "p %Q(a); p DATA.pos\n__END__\nraw\n",
        "p %Q(a); p Object.const_get(:DATA).pos\n__END__\nraw\n",
    ] {
        unchanged(dir.path(), BARE, source.as_bytes(), 1);
    }
    for (name, before, _) in CASES {
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
                format!("{before}{observer}\n").as_bytes(),
                1,
            );
        }
    }
    // Prefix-only ASCII corrections never transcode legacy literal bytes.
    for (name, before, after) in [
        (
            BARE,
            b"# encoding: ISO-8859-1\r\np %Q(caf\xe9)\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\np %(caf\xe9)\r\n".as_slice(),
        ),
        (
            Q,
            b"# encoding: ISO-8859-1\r\np %Q(caf\xe9)\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\np %q(caf\xe9)\r\n".as_slice(),
        ),
    ] {
        fs::write(dir.path().join("a.rb"), before).unwrap();
        cli(dir.path(), &["--only", name, "--fix", "a.rb"], 0);
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), after);
    }
    unchanged(
        dir.path(),
        DELIM,
        b"# encoding: ISO-8859-1\r\np %w(caf\xe9)\r\n",
        1,
    );
    // Equal-length case/delimiter edits preserve observable DATA offsets.
    fs::write(
        dir.path().join("a.rb"),
        "p %Q[hi]; p DATA.pos\n__END__\nraw\n",
    )
    .unwrap();
    cli(
        dir.path(),
        &["--only", &format!("{Q},{DELIM}"), "--fix", "a.rb"],
        0,
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        "p %q(hi); p DATA.pos\n__END__\nraw\n"
    );
}

#[test]
fn bom_crlf_heredoc_indentation_and_data_payloads_survive() {
    let dir = project();
    let source = "\u{feff}# encoding: UTF-8\r\np %Q[café]\r\np %w(one two)\r\np <<~X\r\n  %Q[not code]\r\n  #{%Q[#{1}]}\r\nX\r\np __LINE__\r\n__END__\r\n%Q[raw]\r\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    cli(dir.path(), &["--only", ALL, "--fix", "a.rb"], 0);
    cli(dir.path(), &["--only", ALL, "--fix", "a.rb"], 0);
    let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
    assert!(after.starts_with('\u{feff}'));
    assert!(after.contains("p %(café)\r\n"));
    assert!(after.contains("p %w[one two]\r\n"));
    assert!(after.contains("  %Q[not code]\r\n  #{%(#{1})}\r\n"));
    assert_eq!(
        source.split("__END__").nth(1),
        after.split("__END__").nth(1)
    );
    assert_eq!(
        source.bytes().filter(|&b| b == b'\n').count(),
        after.bytes().filter(|&b| b == b'\n').count()
    );
    assert!(
        after
            .split('\n')
            .take(after.lines().count())
            .all(|line| line.ends_with('\r'))
    );
}

#[test]
fn command_literals_are_source_only_and_keep_command_ast_kinds() {
    let dir = project();
    let source = "[%x[not_executed], %x[#{value}]]\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    cli(dir.path(), &["--only", ALL, "--fix", "a.rb"], 0);
    let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
    assert_eq!(after, "[%x(not_executed), %x(#{value})]\n");
    for text in [source, &after] {
        let parsed = ruby_prism::parse(text.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let program = parsed.node();
        let values = program
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap();
        let elements: Vec<_> = values.as_array_node().unwrap().elements().iter().collect();
        assert!(elements[0].as_x_string_node().is_some());
        assert!(elements[1].as_interpolated_x_string_node().is_some());
    }
}

#[path = "support/source_getter.rs"]
mod source_getter;

fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap()
}

#[test]
fn controlled_ruby_observes_decoded_bytes_encoding_frozen_and_evaluation_order() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    for (frozen, selected) in ["true", "false"]
        .into_iter()
        .flat_map(|frozen| [BARE, DELIM, Q, ALL].map(|selected| (frozen, selected)))
    {
        let source = format!(
            r#"# frozen_string_literal: {frozen}
$events = []
def mark(x)
  $events << x
  x
end
def literal
  %Q[static]
end
values = [%Q[café], %Q!a\!b!, %Q(a\\), %Q[\t], %Q[\u{{41}}],
          %Q[#{{mark(1)}}-#{{mark(2)}}], %W(#{{mark(3)}} a\)b), %I(#{{mark(4)}} a\)b),
          %w(one two), %i(one two), %s[symbol], %r[a(b)c]im,
          %q!a\!b!, %w(\(a\) b), %r!a\!b!]
values.each do |value|
  case value
  when String then p [value.bytes, value.encoding.name, value.frozen?]
  when Array then p value.map {{ |v| [v.to_s.bytes, v.to_s.encoding.name] }}
  when Symbol then p [value.to_s.bytes, value.encoding.name]
  when Regexp then p [value.source.bytes, value.options, value.encoding.name, !!(value =~ "a(b)c")]
  end
end
p [literal.equal?(literal), literal.frozen?, $events, __LINE__]
"#
        );
        let source = source_getter::pattern_sibling(dir.path(), "a.rb", &source, selected);
        fs::write(dir.path().join("a.rb"), &source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success(), "{before:?}");
        let first = cli(
            dir.path(),
            &["--only", selected, "--fix", "a.rb"],
            i32::from(selected == ALL), // Escaped-delimiter edit defers behind bare Q removal.
        );
        assert!(String::from_utf8_lossy(&first.stdout).contains("corrections applied"));
        cli(dir.path(), &["--only", selected, "--fix", "a.rb"], 0);
        let after = ruby(dir.path());
        assert_eq!(before.stdout, after.stdout, "{source}");
        assert_eq!(before.stderr, after.stderr);
        assert_eq!(
            fs::read_to_string(dir.path().join("a.rb"))
                .unwrap()
                .contains("%w[one two]"),
            matches!(selected, DELIM | ALL)
        );
    }
}

#[test]
fn escaped_delimiters_preserve_values_with_default_and_selected_composed_fixes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("a.rb");
    let source = "p [%Q=a\\=b=, %Q!a\\!b!, %Q[a\\]b], %Q[a\\[b], %Q!a\\\\b!]\n";
    let pair = format!("{DELIM},{Q}");
    for selected in [None, Some(pair.as_str()), Some(ALL)] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success(), "{before:?}");
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
            let after = ruby(dir.path());
            assert_eq!(before.status, after.status, "{selected:?}: {after:?}");
            assert_eq!(before.stdout, after.stdout, "{selected:?}");
            assert_eq!(before.stderr, after.stderr, "{selected:?}");
            if old == fs::read(&path).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        assert_ne!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn controlled_legacy_encoding_values_and_mutability_survive_prefix_corrections() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let source = b"# encoding: ISO-8859-1\n# frozen_string_literal: false\ns = %Q[caf\xe9]\ns.concat(\"!\")\np [s.bytes, s.encoding.name, s.frozen?]\n";
    for (selected, status) in [(BARE, 0), (Q, 0), (DELIM, 1), (ALL, 1)] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success(), "{before:?}");
        cli(dir.path(), &["--only", selected, "--fix", "a.rb"], status);
        let after = ruby(dir.path());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        let corrected = fs::read(dir.path().join("a.rb")).unwrap();
        assert!(corrected.contains(&0xe9));
        if selected == DELIM {
            assert_eq!(corrected, source);
        } else {
            assert_ne!(corrected, source);
        }
    }
}

#[test]
fn ruby_data_positions_source_reads_and_ambiguous_prefix_counterexamples() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    for (selected, source, status) in [
        (
            BARE,
            "p %Q(hi); p DATA.pos; p DATA.read.bytes\n__END__\nraw\\payload\n",
            1,
        ),
        (
            Q,
            "p %Q(hi); p DATA.pos; p DATA.read.bytes\n__END__\nraw\\payload\n",
            0,
        ),
        (
            DELIM,
            "p %w(one two); p DATA.pos; p DATA.read.bytes\n__END__\nraw\\payload\n",
            0,
        ),
        (ALL, "p %Q[hi]; p File.binread($PROGRAM_NAME).bytes\n", 1),
        (BARE, "def foo(x); p x; end\nfoo %Q=hi=\n", 1),
        (BARE, "def foo(x); p x; end\nfoo %Q hi \n", 1),
        (BARE, "def foo(x); p x; end\nfoo %Q\thi\t\n", 1),
        (BARE, "def foo(x); p x; end\nfoo %Q\nhi\n;p __LINE__\n", 1),
        (DELIM, "p(%Q\nhi\n)\np __LINE__\n", 1),
        (Q, "p(%Q\nhi\n)\np __LINE__\n", 0),
    ] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success(), "{before:?}");
        cli(dir.path(), &["--only", selected, "--fix", "a.rb"], status);
        let after = ruby(dir.path());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        if selected == BARE {
            fs::write(dir.path().join("a.rb"), source.replacen("%Q", "%", 1)).unwrap();
            let naive = ruby(dir.path());
            assert!(before.stdout != naive.stdout || before.status != naive.status);
        }
    }
}

#[test]
fn fixed_ruby_34_bindings_decoded_payloads_and_literal_flags_are_preserved() {
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Shape(Vec<String>);
    impl<'pr> NodeVisitor<'pr> for Shape {
        fn enter(&mut self, node: ruby_prism::Node<'pr>) {
            self.0.push(format!("{:?}", std::mem::discriminant(&node)));
            if let Some(n) = node.as_string_node() {
                self.0.push(format!(
                    "str:{:?}:{}:{}:{}:{}",
                    n.unescaped(),
                    n.is_forced_utf8_encoding(),
                    n.is_forced_binary_encoding(),
                    n.is_frozen(),
                    n.is_mutable()
                ));
            }
            if let Some(n) = node.as_symbol_node() {
                self.0.push(format!("sym:{:?}", n.unescaped()));
            }
            if let Some(n) = node.as_regular_expression_node() {
                self.0.push(format!("regexp:{:?}", n.unescaped()));
            }
            if let Some(n) = node.as_local_variable_read_node() {
                self.0
                    .push(format!("local:{:?}:{}", n.name().as_slice(), n.depth()));
            }
            if let Some(n) = node.as_call_node() {
                self.0.push(format!("call:{:?}", n.name().as_slice()));
            }
        }
        fn leave(&mut self) {
            self.0.push("end".into());
        }
    }
    let dir = project();
    for frozen in ["true", "false"] {
        let source = format!(
            "# frozen_string_literal: {frozen}\nx = 1\nxs.map {{ [%Q[hello], %Q[#{{it}}], %Q[#{{x}}], %w(one two), %r[a(b)c]im, %s[symbol], %Q=a\\=b=, %Q!a\\!b!] }}\nxs.map {{ [%Q[#{{_1}}], %W(#{{_1}})] }}\n"
        );
        let pair = format!("{DELIM},{Q}");
        for selected in [BARE, DELIM, Q, ALL, pair.as_str()] {
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            cli(
                dir.path(),
                &["--only", selected, "--fix", "a.rb"],
                // Ambiguous %= prefix or an escaped-delimiter overlap stays reported.
                i32::from(matches!(selected, BARE | ALL)),
            );
            let after = fs::read(dir.path().join("a.rb")).unwrap();
            let original = ruby_prism::parse(source.as_bytes());
            let corrected = ruby_prism::parse(&after);
            assert_eq!(original.errors().count(), 0);
            assert_eq!(corrected.errors().count(), 0);
            let mut a = Shape::default();
            let mut b = Shape::default();
            walk(&mut a, &original.node());
            walk(&mut b, &corrected.node());
            assert_eq!(a.0, b.0, "{selected}: {source}");
        }
    }
}

#[test]
fn unicode_scalar_coordinates_and_strict_fixed_policy_config() {
    let dir = project();
    fs::write(dir.path().join("a.rb"), "p \"é\"; %Q[hi]\n").unwrap();
    for name in [BARE, DELIM, Q] {
        let out = cli(dir.path(), &["--only", name, "a.rb"], 1);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("a.rb:1:8:"),
            "{out:?}"
        );
    }
    for (name, key) in [
        (BARE, "EnforcedStyle"),
        (DELIM, "PreferredDelimiters"),
        (Q, "EnforcedStyle"),
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{name}\"]\n{key} = \"other\"\n"),
        )
        .unwrap();
        let before = fs::read(dir.path().join("a.rb")).unwrap();
        cli(dir.path(), &["--only", name, "--fix", "a.rb"], 2);
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), before);
    }
}

#[test]
fn clean_error_and_withheld_files_are_not_rewritten() {
    let dir = project();
    unchanged(dir.path(), &format!("{DELIM},{Q}"), b"p [%Q(), %s[]]\n", 0);
    for (source, status) in [
        ("p %(hi); p %q(hi); p %w[hi]; p %r{hi}\n", 0),
        ("p %q!a\\!b!\n", 0),
        ("p %Q(\n", 2),
    ] {
        unchanged(dir.path(), ALL, source.as_bytes(), status);
    }
}

#[test]
fn curated_percent_and_existing_default_style_layout_composition_converges() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let source = "def f\n  return ({ :a => 'hi' });\nend\np [f, %Q[hello], %Q[#{1}], %w(one two), %Q(\\t), %Q=custom=, %Q #{2} ]\n";
    for only in [
        Some(format!(
            "{ALL},Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout"
        )),
        None,
    ] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path());
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(dir.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(ref names) = only {
                args.extend(["--only", names]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            let after = ruby(dir.path());
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
            if old == fs::read(dir.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable);
        let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
        if only.is_some() {
            assert!(after.contains("%(hello)"), "{after}");
            assert!(after.contains("%(#{1})"), "{after}");
            assert!(after.contains("%q(custom)"), "{after}");
            assert!(after.contains("%(#{2})"), "{after}");
        } else {
            // the default RedundantPercentQ competes for the original
            // envelope, before canonical StringLiterals on the next pass.
            assert!(after.contains("\"hello\""), "{after}");
            assert!(after.contains("\"#{1}\""), "{after}");
            assert!(after.contains("\"custom\""), "{after}");
            assert!(after.contains("\"#{2}\""), "{after}");
        }
        assert!(after.contains("%w[one two]"), "{after}");
    }
}
