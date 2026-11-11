//! real CLI, guarded source wrapping and controlled native witnesses.
//! Pinned examples port RuboCop (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{fs, path::Path, process::Command};
const ASSIGN: &str = "Lint/AmbiguousAssignment";
const PREC: &str = "Lint/AmbiguousOperatorPrecedence";
const BOTH: &str = "Lint/AmbiguousAssignment,Lint/AmbiguousOperatorPrecedence";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn fix(root: &Path, before: &[u8], after: &[u8], status: i32) {
    let path = root.join("lib/a.rb");
    fs::write(&path, before).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, &["--only", BOTH, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(status), "{out:?}\n{before:?}");
    assert_eq!(fs::read(&path).unwrap(), after, "{before:?}\n{out:?}");
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.permissions(), new.permissions());
    if before == after {
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    }
}
fn findings(out: &std::process::Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(cop))
        .count()
}

#[test]
fn defaults_selectors_department_precedence_and_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = b"x =- y\na + b * c\n";
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    for (config, selectors, expected) in [
        (String::new(), vec![], (1, 1)),
        (String::new(), vec!["--only", BOTH], (1, 1)),
        (
            String::new(),
            vec!["--only", "Lint", "--except", PREC],
            (1, 0),
        ),
        (
            String::new(),
            vec!["--only", PREC, "--except", PREC],
            (0, 0),
        ),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], (0, 0)),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{PREC}\"]\nenabled = true\n"),
            vec![],
            (0, 1),
        ),
        (
            format!("[cops.\"{PREC}\"]\nenabled = false\n"),
            vec!["--only", PREC],
            (0, 1),
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        let out = run(dir.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(
            (findings(&out, ASSIGN), findings(&out, PREC)),
            expected,
            "{out:?}"
        );
        if expected.0 == 1 {
            assert!(
                String::from_utf8_lossy(&out.stdout)
                    .contains("lib/a.rb:1:3: E: Suspicious assignment")
            );
        }
        if expected.1 == 1 {
            assert!(
                String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:2:5: E: Wrap expressions")
            );
        }
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            old.modified().unwrap()
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions(),
            old.permissions()
        );
    }
}

#[test]
fn fixed_policy_and_only_real_names_are_admitted() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 582);
    for (name, safe) in [(ASSIGN, false), (PREC, true)] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled());
        assert_eq!(cop.safe_fixes(), safe);
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
        assert!(cop.max_setting().is_none());
    }
    assert!(registry.has_cop("Lint/BooleanSymbol"));
}

#[test]
fn strict_toml_exclusions_and_path_provenance() {
    let dir = project();
    for key in [
        "max = 80",
        "EnforcedStyle = 'other'",
        "include = ['**/*']",
        "enabled = 'true'",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{PREC}\"]\n{key}\n"),
        )
        .unwrap();
        fix(dir.path(), b"a + b * c\n", b"a + b * c\n", 2);
    }
    for config in [
        "exclude = ['lib/a.rb']\n".into(),
        format!("[cops.\"{PREC}\"]\nexclude = ['lib/a.rb']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fix(dir.path(), b"a + b * c\n", b"a + b * c\n", 0);
        let out = run(&dir.path().join("lib"), &["--only", PREC, "--fix", "a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    // No upstream cop path gate: even explicit unusual extensions are eligible.
    fs::write(dir.path().join("lib/example.arb"), "a + b * c\n").unwrap();
    let out = run(dir.path(), &["--only", PREC, "lib/example.arb"]);
    assert_eq!(findings(&out, PREC), 1);
}

#[test]
fn both_directive_prefixes_multiline_next_and_push_scopes() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Lint\nx =- y\na + b * c\n"),
            format!("# {prefix}:disable-next {PREC}\na + b *\n c\n"),
            format!("a + b *\n c # {prefix}:disable {PREC}\n"),
            format!("# {prefix}:push -{PREC}\na + b * c\n# {prefix}:pop\n"),
        ] {
            fix(dir.path(), source.as_bytes(), source.as_bytes(), 0);
        }
        let before = format!(
            "# {prefix}:disable {PREC}\na + b * c\n# {prefix}:enable-next {PREC}\na + b * c\n# {prefix}:push +{PREC}\na + b * c\n# {prefix}:pop\na + b * c\n"
        );
        let after = before
            .lines()
            .enumerate()
            .map(|(i, l)| if i == 3 || i == 5 { "a + (b * c)" } else { l })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fix(dir.path(), before.as_bytes(), after.as_bytes(), 0);
    }
}

#[test]
fn canonical_and_every_pinned_nested_correction_converge_explicitly() {
    let dir = project();
    fix(dir.path(), b"a + b * c\n", b"a + (b * c)\n", 0);
    fix(
        dir.path(),
        b"a && b || c\na || b && c\na && b * c\na * b && c\n",
        b"(a && b) || c\na || (b && c)\na && (b * c)\n(a * b) && c\n",
        0,
    );
    for (before, after) in [
        ("(a + b * c ** d)\n", "(a + (b * (c ** d)))\n"),
        (
            "a ** b * c / d % e + f - g << h >> i & j | k ^ l && m || n\n",
            "(((((((a ** b) * c / d % e) + f - g) << h >> i) & j) | k ^ l) && m) || n\n",
        ),
        (
            "a || b && c | d ^ e & f << g >> h + i - j * k / l % n ** m\n",
            "a || (b && (c | d ^ (e & (f << g >> (h + i - (j * k / l % (n ** m)))))))\n",
        ),
    ] {
        fs::write(dir.path().join("lib/a.rb"), before).unwrap();
        let mut clean = false;
        for _ in 0..8 {
            let out = run(dir.path(), &["--only", PREC, "--fix", "lib/a.rb"]);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            if out.status.success() {
                clean = true;
                break;
            }
        }
        assert!(clean);
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            after
        );
        fix(dir.path(), after.as_bytes(), after.as_bytes(), 0);
    }
}

#[test]
fn detection_only_and_uncertain_edits_preserve_bytes_mtime_and_permissions() {
    let dir = project();
    for source in [
        "x =- y\nx =+ y\nx =* y\nx =! y\n",
        "a + b * c; File.binread(__FILE__)\n",
        "a + b * c; DATA.pos\n__END__\nraw\n",
        "a + b * c; RubyVM::InstructionSequence.of(method(:f))\n",
        "a + b * c; ObjectSpace.each_object { |o| p o }\n",
    ] {
        fix(dir.path(), source.as_bytes(), source.as_bytes(), 1);
    }
    let legacy = b"# encoding: Windows-1252\nx + \"\xe9\" * 2\n";
    fix(dir.path(), legacy, legacy, 1);
    // An unrelated UTF-8 expression in that same file still corrects.
    fix(
        dir.path(),
        b"# encoding: Windows-1252\nx + \"\xe9\" * 2\na + b * c\n",
        b"# encoding: Windows-1252\nx + \"\xe9\" * 2\na + (b * c)\n",
        1,
    );
}

#[test]
fn comments_bom_crlf_line_data_and_heredoc_values() {
    let dir = project();
    fix(dir.path(), b"\xef\xbb\xbf# encoding: utf-8\r\na + b * # keep\r\n c # tail\r\np __LINE__\r\n__END__\r\nraw\x00\r\n", b"\xef\xbb\xbf# encoding: utf-8\r\na + (b * # keep\r\n c) # tail\r\np __LINE__\r\n__END__\r\nraw\x00\r\n", 0);
    fix(
        dir.path(),
        b"x = <<~TEXT\na + b * c\nTEXT\na + b * c\n",
        b"x = <<~TEXT\na + b * c\nTEXT\na + (b * c)\n",
        0,
    );
    fix(
        dir.path(),
        b"a + b * <<~TEXT\nraw\nTEXT\n",
        b"a + (b * <<~TEXT)\nraw\nTEXT\n",
        0,
    );
}

#[test]
fn stdin_path_is_display_and_exclusion_provenance_never_a_write_target() {
    use std::io::Write;
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "sentinel\n").unwrap();
    for (config, count) in [
        (String::new(), 1),
        (format!("[cops.\"{PREC}\"]\nexclude = ['lib/a.rb']\n"), 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", PREC, "--stdin", "lib/a.rb"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"a + b * c\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(findings(&out, PREC), count, "{out:?}");
        if count == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:1:5: E:"));
        }
    }
    let out = run(
        dir.path(),
        &["--only", PREC, "--stdin", "lib/a.rb", "--fix"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(fs::read_to_string(path).unwrap(), "sentinel\n");
}

#[test]
fn invalid_source_remains_atomic_and_symlinks_keep_permissions() {
    let dir = project();
    fix(
        dir.path(),
        b"a + b * c\ndef broken(\n",
        b"a + b * c\ndef broken(\n",
        2,
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, "a + b * c\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
        symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
        let out = run(
            dir.path(),
            &["--only", PREC, "--fix", "lib/alias.rb", "lib/a.rb"],
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert!(
            fs::symlink_metadata(dir.path().join("lib/alias.rb"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o751
        );
        assert_eq!(fs::read_to_string(path).unwrap(), "a + (b * c)\n");
    }
}

fn observe(ruby: &Path, source: &str) -> Vec<u8> {
    let out = Command::new(ruby)
        .args(["--disable-gems", "-e", source])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}\n{source}");
    out.stdout
}
#[test]
fn native_runtime_binding_dispatch_evaluation_flow_values_and_exceptions() {
    let dir = project();
    // Controlled code only, never spec/corpus execution. Custom operators,
    // lexical refinement, locals, short circuit, exceptions and ensure.
    let before = "$events = []\nclass Value\n  def initialize(n); @n = n; end\n  def *(other); $events << :multiply; self; end\n  def +(other); $events << :add; self; end\nend\nmodule Refined\n  refine Value do\n    def +(other); $events << :refined_add; self; end\n  end\nend\nusing Refined\ndef make(n); $events << n; Value.new(n); end\na = make(:a)\nb = make(:b)\nc = make(:c)\nresult = a + b * c\np result.equal?(a)\nx = nil\np (false || (x = true) && :chosen)\np x\np (true || make(:never) && make(:also_never))\nbegin\n  a + b * raise('boom')\nrescue RuntimeError => e\n  p e.message\nensure\n  $events << :ensure\nend\np $events\np __LINE__\n";
    fs::write(dir.path().join("lib/a.rb"), before).unwrap();
    let out = run(dir.path(), &["--only", PREC, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let after = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
    assert!(after.contains("a + (b * c)"));
    assert!(after.contains("false || ((x = true) && :chosen)"));
    let mut rubies = vec![std::path::PathBuf::from("ruby")];
    if let Some(ruby) = std::env::var_os("RUSTOCOP_RUBY34") {
        rubies.push(ruby.into());
    }
    for ruby in rubies {
        if ruby == Path::new("ruby") && Command::new(&ruby).arg("-v").output().is_err() {
            continue;
        }
        let expected = observe(&ruby, before);
        assert_eq!(expected, b"true\n:chosen\ntrue\ntrue\n\"boom\"\n[:a, :b, :c, :multiply, :refined_add, :ensure]\n31\n");
        assert_eq!(observe(&ruby, &after), expected, "{ruby:?}");
    }
}

#[test]
fn native_comments_encoding_heredoc_lines_and_nonlocal_flow() {
    let dir = project();
    for before in [
        "\u{feff}# encoding: utf-8\r\nx = \"é\"\r\np x + \"a\" * # keep\r\n 2\r\np __LINE__\r\n__END__\r\nraw\r\n",
        "p \"prefix:\" + <<~TEXT * 2\nraw\nTEXT\np __LINE__\n__END__\nraw\n",
        "def f; false || true && (return :done); :bad; end\np f\np [1].map { false || true && (next :next) }\np [1].each { false || true && (break :break) }\n",
    ] {
        fs::write(dir.path().join("lib/a.rb"), before).unwrap();
        let out = run(dir.path(), &["--only", PREC, "--fix", "lib/a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let after = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
        assert_ne!(before, after);
        let mut rubies = vec![std::path::PathBuf::from("ruby")];
        if let Some(ruby) = std::env::var_os("RUSTOCOP_RUBY34") {
            rubies.push(ruby.into());
        }
        for ruby in rubies {
            if ruby == Path::new("ruby") && Command::new(&ruby).arg("-v").output().is_err() {
                continue;
            }
            assert_eq!(
                observe(&ruby, before),
                observe(&ruby, &after),
                "{ruby:?}\n{before}"
            );
        }
    }
}

#[test]
fn lint_style_layout_composition_has_no_default_redundant_parens_cycle() {
    let dir = project();
    let before = "def f(a, b, c)\n  a+b*c\nend\n";
    fs::write(dir.path().join("lib/a.rb"), before).unwrap();
    let selected =
        "Lint/AmbiguousOperatorPrecedence,Style/RedundantParentheses,Layout/SpaceAroundOperators";
    let first = run(dir.path(), &["--only", selected, "--fix", "lib/a.rb"]);
    assert_ne!(first.status.code(), Some(2));
    let second = run(dir.path(), &["--only", selected, "--fix", "lib/a.rb"]);
    assert_eq!(second.status.code(), Some(1), "{second:?}");
    assert_eq!(findings(&second, PREC), 1);
    let stable = fs::read(dir.path().join("lib/a.rb")).unwrap();
    let third = run(dir.path(), &["--only", selected, "--fix", "lib/a.rb"]);
    assert_eq!(third.status.code(), Some(1));
    assert_eq!(findings(&third, PREC), 1);
    assert_eq!(stable, fs::read(dir.path().join("lib/a.rb")).unwrap());
    assert_eq!(stable, b"def f(a, b, c)\n  a + b * c\nend\n");
}

#[test]
fn explicit_operator_arguments_withhold_wraps_with_redundant_parentheses_selected() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let pair = format!("{PREC},Style/RedundantParentheses");
    for args in [
        vec!["--only", pair.as_str(), "--fix", "lib/a.rb"],
        vec!["--fix", "lib/a.rb"],
    ] {
        for before in ["a.+(b * c)\n", "a.+((b * c))\n"] {
            fs::write(&path, before).unwrap();
            // The peer may remove existing redundant grouping once, but must
            // never oscillate back to it. Diagnostics remain on every pass.
            for _ in 0..3 {
                let old = fs::metadata(&path).unwrap();
                let bytes = fs::read(&path).unwrap();
                let out = run(dir.path(), &args);
                assert_eq!(out.status.code(), Some(1), "{out:?}");
                assert_eq!(findings(&out, PREC), 1, "{out:?}");
                assert_eq!(fs::read(&path).unwrap(), b"a.+(b * c)\n", "{out:?}");
                let new = fs::metadata(&path).unwrap();
                assert_eq!(old.permissions(), new.permissions());
                if bytes == b"a.+(b * c)\n" {
                    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
                }
            }
        }
    }
    // Explicit exclusion wins over selection: canonical correction remains
    // available without the incompatible peer, not a detection-only cop.
    fs::write(&path, "a.+(b * c)\n").unwrap();
    let out = run(
        dir.path(),
        &[
            "--only",
            &pair,
            "--except",
            "Style/RedundantParentheses",
            "--fix",
            "lib/a.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), b"a.+((b * c))\n");
}
