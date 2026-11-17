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
