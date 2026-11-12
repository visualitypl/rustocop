//! fixed array policies, no-write controls, and controlled ordinary Ruby.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const CONSTRUCTOR: &str = "Style/RedundantArrayConstructor";
const COPS: [(&str, &str, bool); 3] = [
    ("Style/NegativeArrayIndex", "arr[arr.size - 2]\n", false),
    (CONSTRUCTOR, "Array.new([])\n", true),
    ("Style/RedundantArrayFlatten", "x.flatten(1).join\n", false),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    assert!(out.stderr.is_empty() || code == 2, "{out:?}");
    out
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let out = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    out
}
#[test]
fn exact_registry_defaults_safe_capabilities_and_fixed_34_gates() {
    use rustocop::cops::{
        Registry,
        policy::{DEFAULT_RUBY_VERSION, RubyVersion},
    };
    assert_eq!(DEFAULT_RUBY_VERSION, RubyVersion(3, 4));
    let registry = Registry::new();
    for (name, _, safe) in COPS {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled());
        assert_eq!(cop.safe_fixes(), safe);
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
    }
}
#[test]
fn default_only_except_enabling_disabling_excludes_and_strict_config_no_writes() {
    let d = project();
    for (cop, s, _) in COPS {
        // Isolate this cop's disable/exclude controls from default-on
        // EmptyLiteral, which independently owns the empty lookalike.
        let s = if cop == CONSTRUCTOR {
            "Array.new([1])\n"
        } else {
            s
        };
        let o = unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled = false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "exclude = [\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nTargetRubyVersion = 4.0\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 2);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn both_directive_prefixes_and_all_scopes_protect_bytes_and_mtime() {
    let d = project();
    for (cop, source, _) in COPS {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
            let s = format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            let out = unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&out.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (source, code) in [("[]\n", 0), ("(\n", 2)] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
    // A directive inside a multiline constructor cannot be erased by a fix.
    unchanged(
        d.path(),
        b"Array.new(\n [] # rubocop:disable Style/RedundantArrayConstructor\n)\n",
        &["--only", CONSTRUCTOR, "--fix", "a.rb"],
        1,
    );
}
#[test]
fn typed_getters_and_one_child_wrappers_fix_without_writing_targets_or_aborting() {
    let d = project();
    let only =
        "Style/NegativeArrayIndex,Style/RedundantArrayConstructor,Style/RedundantArrayFlatten";
    let sources = [
        "Array[0]",
        "::Array[0]",
        "Array[]",
        "Array[0, 1]",
        "arr[arr.length - 1]",
        "arr[0..arr.length - 1]",
        "arr[0..[arr.length - 1]]",
        "arr[0..defined?(arr.length - 1)]",
        "arr[0..begin arr.length - 1 end]",
    ];
    for op in ["+=", "||=", "&&="] {
        for getter in sources {
            let source = format!("{getter} {op} 1\n");
            for fix in [false, true] {
                // Even while another file has a canonical safe edit, the
                // getter target must retain bytes+mtime, report, and not abort.
                fs::write(d.path().join("safe.rb"), "Array.new([])\n").unwrap();
                let mut args = vec!["--only", only];
                if fix {
                    args.push("--fix");
                }
                args.extend(["a.rb", "safe.rb"]);
                let out = unchanged(d.path(), source.as_bytes(), &args, 1);
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains("a.rb:1:"),
                    "{out:?}"
                );
                assert_eq!(
                    fs::read_to_string(d.path().join("safe.rb")).unwrap(),
                    if fix { "[]\n" } else { "Array.new([])\n" }
                );
            }
        }
    }
    // Explicit .[] calls remain supported in expression position. Ruby 3.4
    // rejects them as compound targets (in both Prism and the pinned oracle),
    // so do not invent/reparse a getter from an invalid source.
    for source in [
        "Array.[](0) += 1\n",
        "Array.[](0) ||= 1\n",
        "Array.[](0) &&= 1\n",
        "arr.[](arr.length - 1) += 1\n",
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", only, "--fix", "a.rb"],
            2,
        );
    }
    for source in [
        "Array[0] = 1\n",
        "arr[arr.length - 1] = 1\n",
        "arr[0..[arr.length - 1]] = 1\n",
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", only, "--fix", "a.rb"],
            0,
        );
    }
    for source in [
        "def f; arr[0..yield(arr.length - 1)]; end\n",
        "def f; arr[0..super(arr.length - 1)]; end\n",
        "Array.[](0)\n",
    ] {
        // Explicit constructor sends can have ordinary safe edits; use no-fix
        // here, while yield/super stay uncorrectable even when fixing.
        let fix = !source.starts_with("Array");
        let args = if fix {
            vec!["--only", only, "--fix", "a.rb"]
        } else {
            vec!["--only", only, "a.rb"]
        };
        unchanged(d.path(), source.as_bytes(), &args, 1);
    }
}
#[test]
fn multi_statement_parenthesized_ranges_report_without_writes_and_keep_safe_siblings() {
    let d = project();
    let only = "Style/NegativeArrayIndex,Style/RedundantArrayConstructor";
    for suffix in ["", " += rhs", " ||= rhs", " &&= rhs"] {
        let source = format!("arr[(0..arr.length - 1; 0)]{suffix}\n");
        for fix in [false, true] {
            fs::write(d.path().join("safe.rb"), "Array.new([])\n").unwrap();
            let mut args = vec!["--only", only];
            if fix {
                args.push("--fix");
            }
            args.extend(["a.rb", "safe.rb"]);
            let out = unchanged(d.path(), source.as_bytes(), &args, 1);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(stdout.contains("a.rb:1:9:"), "{stdout}");
            assert!(stdout.contains("(Style/NegativeArrayIndex)"), "{stdout}");
            assert_eq!(
                fs::read_to_string(d.path().join("safe.rb")).unwrap(),
                if fix { "[]\n" } else { "Array.new([])\n" }
            );
        }
    }
}
#[test]
fn stdin_reports_without_mutating_the_file() {
    let d = project();
    let path = d.path().join("a.rb");
    fs::write(&path, "unchanged\n").unwrap();
    for (cop, source, _) in COPS {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        assert_eq!(fs::read_to_string(&path).unwrap(), "unchanged\n");
    }
}
#[test]
fn canonical_single_pass_and_safe_siblings_do_not_enable_unsafe_edits() {
    let d = project();
    let only = "Style/NegativeArrayIndex,Style/RedundantArrayConstructor,Style/RedundantArrayFlatten,Style/StringLiterals";
    let before = "x = Array.new(['x'])\narr[arr.size - 2]\nx.flatten.join\n";
    let expected = "x = [\"x\"]\narr[arr.size - 2]\nx.flatten.join\n";
    fs::write(d.path().join("a.rb"), before).unwrap();
    // Overlapping literal/constructor edits are atomic, and converge within two
    // explicit passes without changing the CLI's single-pass contract.
    for _ in 0..2 {
        cli(d.path(), &["--only", only, "--fix", "a.rb"], 1);
    }
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
    unchanged(
        d.path(),
        expected.as_bytes(),
        &["--only", only, "--fix", "a.rb"],
        1,
    );
    for (s, after) in [
        ("Array.new([])\n", "[]\n"),
        (
            "# frozen_string_literal: false\nArray['x']\n",
            "# frozen_string_literal: false\n['x']\n",
        ),
        ("Array(%w[a\\ b])\n", "%w[a\\ b]\n"),
    ] {
        fs::write(d.path().join("a.rb"), s).unwrap();
        cli(d.path(), &["--only", CONSTRUCTOR, "--fix", "a.rb"], 0);
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        unchanged(
            d.path(),
            after.as_bytes(),
            &["--only", CONSTRUCTOR, "--fix", "a.rb"],
            0,
        );
    }
}
#[test]
fn constructor_string_context_holds_are_individual_and_do_not_abort_safe_siblings() {
    let d = project();
    for (magic, expressions) in [
        (
            "",
            vec![
                "Array[\"x\"]",
                "Array[?x]",
                "::Array['x']",
                "Array.[](%q(x))",
                "Array[1, \"x\", :x]",
                "Array[\"x\" \"y\"]",
            ],
        ),
        (
            "# frozen_string_literal: false\n",
            vec!["Array[\"x\" \"y\"]"],
        ),
    ] {
        for expr in expressions {
            let source = format!("{magic}{expr}\n");
            for fix in [false, true] {
                let mut args = vec!["--only", CONSTRUCTOR];
                if fix {
                    args.push("--fix");
                }
                args.push("a.rb");
                let out = unchanged(d.path(), source.as_bytes(), &args, 1);
                assert!(String::from_utf8_lossy(&out.stdout).contains(CONSTRUCTOR));
            }
            let mixed = format!("{source}Array.new([?x])\nArray[1, :x]\n");
            fs::write(d.path().join("a.rb"), mixed).unwrap();
            let out = cli(d.path(), &["--only", CONSTRUCTOR, "--fix", "a.rb"], 1);
            assert!(String::from_utf8_lossy(&out.stdout).contains(CONSTRUCTOR));
            let expected = format!("{source}[?x]\n[1, :x]\n");
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
            unchanged(
                d.path(),
                expected.as_bytes(),
                &["--only", CONSTRUCTOR, "--fix", "a.rb"],
                1,
            );
        }
    }
    // The existing magic-comment parser, not source substring matching, owns
    // policy. Unrecognized or ineligible directives do not establish it.
    for source in [
        "# frozen_string_literal: maybe\nArray[\"x\"]\n",
        "1\n# frozen_string_literal: false\nArray[\"x\"]\n",
        "'# frozen_string_literal: false'\nArray[\"x\"]\n",
        "\u{feff}Array[\"x\"]\r\n__LINE__\r\n__END__\r\npayload\0\r\n",
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", CONSTRUCTOR, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn encoded_literals_bom_crlf_line_data_heredocs_commands_and_observer_guards() {
    let d = project();
    for (cop, s, safe) in COPS {
        let s = format!(
            "\u{feff}{}__LINE__\r\n__END__\r\nraw\0\r\n",
            s.replace('\n', "\r\n")
        );
        if !safe {
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
        } else {
            fs::write(d.path().join("a.rb"), s).unwrap();
            cli(d.path(), &["--only", cop, "--fix", "a.rb"], 0);
            assert_eq!(
                fs::read(d.path().join("a.rb")).unwrap(),
                b"\xef\xbb\xbf[]\r\n__LINE__\r\n__END__\r\nraw\0\r\n"
            );
        }
    }
    for (cop, s) in [
        (
            CONSTRUCTOR,
            b"# encoding: ISO-8859-1\nArray.new(%w[\xe9 a\\ b])\n".as_slice(),
        ),
        (
            CONSTRUCTOR,
            b"# encoding: Windows-31J\nArray(%w[\x83\\ a])\n",
        ),
        (CONSTRUCTOR, b"Array.new([`never execute`])\n"),
        (CONSTRUCTOR, b"Array.new([<<~TEXT])\nx\nTEXT\n"),
        (
            CONSTRUCTOR,
            b"Array.new([])\np DATA.pos\n__END__\npayload\n",
        ),
        (CONSTRUCTOR, b"Array.new([])\np File.binread(__FILE__)\n"),
        (CONSTRUCTOR, b"Array.new([])\np GC.stat\n"),
        (
            "Style/NegativeArrayIndex",
            b"# encoding: ISO-8859-1\n%w[\xe9][%w[\xe9].size - 1]\n",
        ),
        (
            "Style/RedundantArrayFlatten",
            b"# encoding: Windows-31J\n%w[\x83\\].flatten.join\n",
        ),
    ] {
        unchanged(d.path(), s, &["--only", cop, "--fix", "a.rb"], 1);
    }
}
#[test]
fn six_thousand_candidates_each_have_bounded_main_parse_work() {
    let d = project();
    for (cop, s, safe) in COPS {
        let s = s.repeat(6000);
        if safe {
            fs::write(d.path().join("a.rb"), s).unwrap();
            cli(d.path(), &["--only", cop, "--fix", "a.rb"], 0);
            assert_eq!(
                fs::read_to_string(d.path().join("a.rb")).unwrap(),
                "[]\n".repeat(6000)
            );
        } else {
            let out = unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&out.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                6000
            );
        }
    }
}
#[test]
fn six_thousand_compound_getters_use_the_main_ast_once() {
    let d = project();
    for (cop, source) in [
        (CONSTRUCTOR, "Array[0] ||= 1\n"),
        (
            "Style/NegativeArrayIndex",
            "arr[0..[arr.length - 1]] += 1\n",
        ),
    ] {
        let out = unchanged(
            d.path(),
            source.repeat(6000).as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout)
                .matches(&format!("({cop})"))
                .count(),
            6000
        );
    }
}
fn observation(ruby: &str, s: &str) -> (Vec<u8>, Vec<u8>) {
    let out = Command::new(ruby)
        .args(["-W:deprecated", "-e", s])
        .output()
        .unwrap();
    assert!(out.status.success(), "{ruby}: {s}: {out:?}");
    (out.stdout, out.stderr)
}
fn runtime_canonicals(ruby: &str) {
    let d = project();
    for expr in [
        "Array.new([])",
        "::Array.new([])",
        "Array[]",
        "::Array[]",
        "Array([])",
        "Array.new(['foo', 'bar', 'baz'])",
        "Array['foo', 'bar', 'baz']",
        "Array(['foo', 'bar', 'baz'])",
        "Array.new([nil, false, [1], {a: 1}])",
        "Array(%w[a\\ b c\\]d])",
        "Array.new(%i[a b])",
        "Array.new(%W[a b])",
        "Array.[](1, [2])",
        "Array.new([?\\ ])",
        "Array.new([\"\\xFF\"])",
        "Array[\"x\"]",
        "::Array[?x]",
        "Array.[](%q(x))",
        "Array[%Q(x)]",
        "Array[\"x\" \"y\"]",
        "Array[(\"x\")]",
        "Array[(?x)]",
        "Array[(\"x\" \"y\")]",
        "Array[[\"x\"]]",
        "Array[%w[x]]",
        "Array[{a: \"x\"}]",
        "Array[\"x\".strip]",
        "Array.new([?x])",
        "Array([\"x\"])",
        "Array.new([\"x\" \"y\"])",
        "Array[1, nil, :x]",
        "Array[\"#{1}\"]",
        "Array[\"x#{1}\"]",
        "Array[\"x\" \"#{1}\"]",
    ] {
        for magic in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            let before = format!("{magic}x = {expr}\n");
            let held = (magic.is_empty()
                && matches!(
                    expr,
                    "Array['foo', 'bar', 'baz']"
                        | "Array[\"x\"]"
                        | "::Array[?x]"
                        | "Array.[](%q(x))"
                        | "Array[%Q(x)]"
                ))
                || (magic != "# frozen_string_literal: true\n"
                    && matches!(
                        expr,
                        "Array[\"x\" \"y\"]"
                            | "Array[\"#{1}\"]"
                            | "Array[\"x#{1}\"]"
                            | "Array[\"x\" \"#{1}\"]"
                    ));
            fs::write(d.path().join("a.rb"), &before).unwrap();
            let time = fs::metadata(d.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap();
            cli(
                d.path(),
                &["--only", CONSTRUCTOR, "--fix", "a.rb"],
                i32::from(held),
            );
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            if held {
                assert_eq!(before, after, "{expr}");
                assert_eq!(
                    fs::metadata(d.path().join("a.rb"))
                        .unwrap()
                        .modified()
                        .unwrap(),
                    time
                );
            } else {
                assert_ne!(before, after, "{expr}");
            }
            // Runtime-only recursive child mutation observes chilled warnings,
            // not just the outer array's value/mutability. Compare all output,
            // including encoding/frozen flags and rescued frozen errors.
            let suffix = r#"p [x, x.frozen?]
visit = ->(v) do
  case v
  when String
    p [v.bytes, v.encoding.name, v.frozen?]
    begin; v << '!'; rescue => e; p e.class.name; end
  when Array
    v.each { |child| visit.call(child) }
  when Hash
    v.each_value { |child| visit.call(child) }
  end
end
visit.call(x)
y = x; x << :tail; p [x, y.equal?(x)]
"#;
            assert_eq!(
                observation(ruby, &format!("{before}{suffix}")),
                observation(ruby, &format!("{after}{suffix}")),
                "{ruby}: {magic}{expr}"
            );
        }
    }
    // Frozen/chilled literal element flags and warnings survive wrapper removal.
    for magic in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        let before = format!("{magic}x = Array.new(['x'])\n");
        // The rescue harness is runtime-only; unsupported control-flow nodes
        // in the linted file deliberately invalidate the closed-prefix proof.
        let suffix = "p [x[0].bytes, x[0].encoding.name, x[0].frozen?]\nbegin; x[0] << '!'; rescue => e; p e.class.name; end\np x\n";
        fs::write(d.path().join("a.rb"), &before).unwrap();
        cli(d.path(), &["--only", CONSTRUCTOR, "--fix", "a.rb"], 0);
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert_ne!(before, after);
        assert_eq!(
            observation(ruby, &format!("{before}{suffix}")),
            observation(ruby, &format!("{after}{suffix}"))
        );
    }
}
#[test]
fn native_chilled_constructor_context_counterexample_and_cli_no_write() {
    if crate::support::runtime::skips("Ruby 4.0 warns about the chilled string in both forms") {
        return;
    }
    let Ok(ruby) = crate::support::runtime::ruby() else {
        eprintln!("SKIP native Ruby 3.4; set RUSTOCOP_RUBY34");
        return;
    };
    let d = project();
    for literal in ["\"x\"", "?x"] {
        let before = format!("v = Array[{literal}]\n");
        let naive = format!("v = [{literal}]\n");
        // Append the mutation harness only at runtime: closed dispatch does not
        // claim to prove these later calls. Both frozen? values are false, but
        // Ruby 3.4's call-argument vs literal-element compilation changes warnings.
        let harness = "v[0] << \"!\"; p [v[0].bytes, v[0].frozen?]\n";
        let old = observation(&ruby, &format!("{before}{harness}"));
        let new = observation(&ruby, &format!("{naive}{harness}"));
        assert_eq!(old.0, new.0);
        assert!(old.1.is_empty(), "{old:?}");
        assert!(String::from_utf8_lossy(&new.1).contains("literal string will be frozen"));
        let out = unchanged(
            d.path(),
            before.as_bytes(),
            &["--only", CONSTRUCTOR, "--fix", "a.rb"],
            1,
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains(CONSTRUCTOR));
        assert_eq!(
            old,
            observation(
                &ruby,
                &format!(
                    "{}{harness}",
                    fs::read_to_string(d.path().join("a.rb")).unwrap()
                )
            )
        );
    }
}
#[test]
fn native_adjacent_string_constructor_context_counterexample_and_cli_no_write() {
    if crate::support::runtime::skips(
        "Ruby 4.0 no longer freezes adjacent string literals in an array literal",
    ) {
        return;
    }
    let Ok(ruby) = crate::support::runtime::ruby() else {
        eprintln!("SKIP native Ruby 3.4; set RUSTOCOP_RUBY34");
        return;
    };
    let d = project();
    for magic in ["", "# frozen_string_literal: false\n"] {
        let before = format!("{magic}v = Array[\"x\" \"y\"]\n");
        let naive = format!("{magic}v = [\"x\" \"y\"]\n");
        let harness =
            "p v[0].frozen?; begin; v[0] << \"!\"; rescue => e; p e.class.name; end; p v\n";
        let old = observation(&ruby, &format!("{before}{harness}"));
        let new = observation(&ruby, &format!("{naive}{harness}"));
        assert_eq!(old.0, b"false\n[\"xy!\"]\n");
        assert_eq!(new.0, b"true\n\"FrozenError\"\n[\"xy\"]\n");
        unchanged(
            d.path(),
            before.as_bytes(),
            &["--only", CONSTRUCTOR, "--fix", "a.rb"],
            1,
        );
    }
}
const WITNESSES: &str = r#"
a = [1]; raise unless a[a.size - 2] == 1 && a[-2] == nil
h = {0 => :zero}; raise unless h[h.size - 1] == :zero && h[-1] == nil
class ArrayLike
  def flatten(*); :flattened; end
  def join; :not_flattened; end
end
x = ArrayLike.new; raise if x.flatten == x.join
class Converts
  def to_ary; [1, 2]; end
  def to_s; 'string'; end
end
x = [Converts.new]; raise unless x.flatten(0).join == '12' && x.join == '12'
events = []; x.flatten((events << :depth; 1)).join; raise unless events == [:depth]
# Effects/alias capture/splats must not be treated as closed literal proofs.
events = []; x = Array.new([events << :first, events << :second]); raise unless events == [:first, :second]
x = []; raise unless Array(x).equal?(x) && !Array.new(x).equal?(x)
x = Array.new([element = []]); x[0] << 1; raise unless element == [1]
class Coercible
  def to_a; [3]; end
  def to_ary; [4]; end
end
raise unless [*Coercible.new] == [3] && Array(Coercible.new) == [4]
class Array
  def self.new(x); :overridden; end
end
raise unless Array.new([]) == :overridden && [] != :overridden
puts 'array constructor witnesses ok'
"#;
fn witnesses(ruby: &str) {
    assert_eq!(
        observation(ruby, WITNESSES).0,
        b"array constructor witnesses ok\n"
    );
    runtime_canonicals(ruby);
}
#[test]
fn controlled_host_runtime_canonicals_mutation_warnings_and_unsafe_counterexamples() {
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        witnesses("ruby");
    }
}
#[test]
fn controlled_fixed_34_runtime_canonicals_mutation_warnings_and_unsafe_counterexamples() {
    if let Ok(ruby) = crate::support::runtime::ruby() {
        assert!(
            String::from_utf8_lossy(&observation(&ruby, "print RUBY_VERSION").0)
                .starts_with(crate::support::runtime::version_prefix())
        );
        witnesses(&ruby);
    } else {
        eprintln!("SKIP native Ruby 3.4; set RUSTOCOP_RUBY34");
    }
}
#[test]
fn curated_original_array_literal_and_layout_peers_stabilize_in_six_explicit_passes() {
    let d = project();
    let source = "x = Array.new([{'a' => 'x'}]); p x\n";
    for only in [
        Some(
            "Style/NegativeArrayIndex,Style/RedundantArrayConstructor,Style/RedundantArrayFlatten",
        ),
        Some(
            "Style/NegativeArrayIndex,Style/RedundantArrayConstructor,Style/RedundantArrayFlatten,Style/ArrayCoercion,Style/ArrayFirstLast,Style/ArrayIntersect,Style/ArrayJoin,Style/ArrayIntersectWithSingleElement,Style/ConcatArrayLiterals,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Style/MethodCallWithArgsParentheses,Style/MethodCallWithoutArgsParentheses,Style/TrailingCommaInArrayLiteral,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let args = only.map_or(vec!["--fix", "a.rb"], |s| {
                vec!["--only", s, "--fix", "a.rb"]
            });
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            let after = fs::read(d.path().join("a.rb")).unwrap();
            for ruby in std::iter::once("ruby".to_string()).chain(crate::support::runtime::ruby()) {
                if Command::new(&ruby).arg("--version").output().is_ok() {
                    assert_eq!(
                        observation(&ruby, source),
                        observation(&ruby, std::str::from_utf8(&after).unwrap())
                    );
                }
            }
            if after == before {
                stable = true;
                break;
            }
        }
        assert!(stable, "{only:?}");
        assert!(
            !fs::read_to_string(d.path().join("a.rb"))
                .unwrap()
                .contains("Array.new")
        );
    }
}
