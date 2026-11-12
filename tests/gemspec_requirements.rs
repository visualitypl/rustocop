//! Gemspec-requirements family through the actual CLI:
//! supplied paths, selectors, config, directives, encodings, fatal inputs,
//! the location-free missing finding, and RequireMFA's guarded corrections
//! with their no-write and atomic guarantees. Gemspecs are linted, never
//! evaluated.
use rustocop::cops::Registry;
#[path = "support/stdin.rs"]
mod stdin;
use std::{fs, path::Path, process::Command};

const MFA: &str = "Gemspec/RequireMFA";
const REQUIRED: &str = "Gemspec/RequiredRubyVersion";
const GLOBALS: &str = "Gemspec/RubyVersionGlobalsUsage";
const ALL: [&str; 3] = [MFA, REQUIRED, GLOBALS];

/// One finding for each cop: no MFA metadata, a 3.3 requirement, one
/// RUBY_VERSION.
const GEMSPEC: &str = "Gem::Specification.new do |s|\n  s.required_ruby_version = '>= 3.3'\n  s.add_dependency 'a' if RUBY_VERSION >= '3'\nend\n";
const MISSING: &str = "`required_ruby_version` should be specified.";
const ENVELOPE: &str = "if false\n  Gem::Specification.new do |s|\n    s.metadata['a'] = 'b'\n    s.required_ruby_version = '>= 3.4'\n  end\nend\n";
const ENVELOPE_FIXED: &str = "if false\n  Gem::Specification.new do |s|\n    s.metadata['a'] = 'b'\ns.metadata['rubygems_mfa_required'] = 'true'\n    s.required_ruby_version = '>= 3.4'\n  end\nend\n";

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}

fn cli(root: &Path, args: &[&str], input: Option<&[u8]>) -> std::process::Output {
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        stdin::write_stdin(child.stdin.take().unwrap(), input, args.contains(&"--fix")).unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

fn count(out: &std::process::Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!(" ({cop})\n"))
        .count()
}

fn counts(out: &std::process::Output) -> [usize; 3] {
    ALL.map(|cop| count(out, cop))
}

#[test]
fn registry_metadata_defaults_and_no_knobs() {
    let registry = Registry::new();
    for (name, safe) in [(MFA, true), (REQUIRED, false), (GLOBALS, false)] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled(), "{name}");
        assert_eq!(cop.safe_fixes(), safe, "{name}");
        assert_eq!(
            cop.severity(),
            rustocop::offense::Severity::Convention,
            "{name}"
        );
        assert!(cop.max_setting().is_none(), "{name}");
        assert_eq!(cop.ruby_versions(), Default::default(), "{name}");
        assert!(cop.path_policy().is_some(), "{name}");
    }
    let d = project();
    for name in ALL {
        for key in [
            "target_ruby_version='3.3'",
            "enforced_style='x'",
            "include=['**/*']",
            "max=1",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{name}\"]\n{key}\n"),
            )
            .unwrap();
            // A rejected config exits before reading stdin: send none.
            let out = cli(d.path(), &["--stdin", "a.gemspec"], Some(b""));
            assert_eq!(out.status.code(), Some(2), "{name} {key}: {out:?}");
        }
    }
    fs::write(
        d.path().join("rustocop.toml"),
        "target_ruby_version='3.3'\n",
    )
    .unwrap();
    let out = cli(d.path(), &["--stdin", "a.gemspec"], Some(b""));
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[test]
fn supplied_paths_selectors_config_and_directives_cannot_bypass_gates() {
    let d = project();
    let run = |args: &[&str], path: &str, source: &str| {
        let mut all = args.to_vec();
        all.extend(["--stdin", path]);
        counts(&cli(d.path(), &all, Some(source.as_bytes())))
    };
    // All three are on by default.
    assert_eq!(run(&[], "a.gemspec", GEMSPEC), [1, 1, 1]);
    let selected = ALL.join(",");
    for (path, expected) in [
        ("a.gemspec", [1, 1, 1]),
        ("nested/.x.gemspec", [1, 1, 1]),
        ("Gemfile", [0, 0, 0]),
        ("nested/gems.rb", [0, 0, 0]),
        ("x.gemfile", [0, 0, 0]),
        ("Gemfile.lock", [0, 0, 0]),
        ("a.rb", [0, 0, 0]),
        ("gemspec", [0, 0, 0]),
    ] {
        for selector in [selected.as_str(), "Gemspec"] {
            assert_eq!(
                run(&["--only", selector], path, GEMSPEC),
                expected,
                "{path} {selector}"
            );
        }
    }
    for (config, flags, expected) in [
        (
            "[cops.Gemspec]\nenabled=false\n".to_owned(),
            vec![],
            [0, 0, 0],
        ),
        (
            "[cops.Gemspec]\nenabled=false\n".to_owned(),
            vec!["--only", REQUIRED],
            [0, 1, 0],
        ),
        (
            format!("[cops.\"{MFA}\"]\nenabled=false\n"),
            vec![],
            [0, 1, 1],
        ),
        (String::new(), vec!["--except", MFA], [0, 1, 1]),
        (String::new(), vec!["--only", GLOBALS], [0, 0, 1]),
        ("exclude=['a.gemspec']\n".to_owned(), vec![], [0, 0, 0]),
        (
            format!("[cops.\"{REQUIRED}\"]\nexclude=['a.gemspec']\n"),
            vec![],
            [1, 0, 1],
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), &config).unwrap();
        assert_eq!(
            run(&flags, "a.gemspec", GEMSPEC),
            expected,
            "{flags:?} {config}"
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for (target, expected) in [
            ("all", [0, 0, 0]),
            ("Gemspec", [0, 0, 0]),
            (MFA, [0, 1, 1]),
            (GLOBALS, [1, 1, 0]),
            ("Bundler", [1, 1, 1]),
        ] {
            let source = format!("# {prefix}:disable {target}\n{GEMSPEC}").replace('\n', "\r\n");
            assert_eq!(run(&["--only", &selected], "a.gemspec", &source), expected);
        }
        // Trailing directives on the reported lines only.
        let source = GEMSPEC
            .replace(
                "'>= 3.3'\n",
                &format!("'>= 3.3' # {prefix}:disable {REQUIRED}\n"),
            )
            .replace(
                ">= '3'\n",
                &format!(">= '3' # {prefix}:disable {GLOBALS}\n"),
            );
        assert_eq!(run(&[], "a.gemspec", &source), [1, 0, 0]);
        // The missing finding is location-free: a directive disables it only
        // where it covers the first line, like the pinned global offense.
        for (source, expected) in [
            (format!("# {prefix}:disable {REQUIRED}\nx = 1\n"), 0),
            (format!("x = 1 # {prefix}:disable {REQUIRED}\n"), 0),
            (format!("\n# {prefix}:disable {REQUIRED}\nx = 1\n"), 1),
            (format!("# {prefix}:disable-next {REQUIRED}\nx = 1\n"), 1),
        ] {
            assert_eq!(
                run(&["--only", REQUIRED], "a.gemspec", &source)[1],
                expected,
                "{source}"
            );
        }
    }
}

#[test]
fn missing_finding_is_global_for_empty_and_comment_only_gemspecs() {
    let d = project();
    for source in [
        &b""[..],
        b"# comment only\n",
        b"\xef\xbb\xbf# c\r\n",
        b"\n\n",
        b"__END__\nspec.required_ruby_version = '>= 3.4'\n",
        b"Gem::Specification.new do |s|\nend\n",
    ] {
        let out = cli(
            d.path(),
            &["--only", REQUIRED, "--stdin", "x.gemspec"],
            Some(source),
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).lines().next().unwrap(),
            format!("x.gemspec: C: {MISSING} ({REQUIRED})"),
            "{source:?}"
        );
        assert_eq!(count(&out, REQUIRED), 1);
        // GitHub annotations carry no line for it either.
        let out = cli(
            d.path(),
            &[
                "--only",
                REQUIRED,
                "--format",
                "github",
                "--stdin",
                "x.gemspec",
            ],
            Some(source),
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains(MISSING) && !text.contains("line="), "{text}");
    }
    // Only supplied gemspecs; a fatal input gets no ordinary finding.
    for (path, source, code) in [
        ("Gemfile", &b""[..], 0),
        ("x.rb", b"", 0),
        ("x.gemspec", b"def (", 2),
        ("x.gemspec", b"# coding: no-such-encoding\n", 2),
    ] {
        let out = cli(
            d.path(),
            &["--only", REQUIRED, "--stdin", path],
            Some(source),
        );
        assert_eq!(out.status.code(), Some(code), "{path} {out:?}");
        assert_eq!(count(&out, REQUIRED), 0, "{out:?}");
    }
}

#[test]
fn encodings_data_sections_and_fatal_inputs_compose() {
    let d = project();
    let selected = ALL.join(",");
    for source in [
        format!("\u{feff}{GEMSPEC}").into_bytes(),
        GEMSPEC.replace('\n', "\r\n").into_bytes(),
        format!("# coding: Windows-1252\n{GEMSPEC}").into_bytes(),
        format!("{GEMSPEC}__END__\nRUBY_VERSION\ninvalid ruby {{[").into_bytes(),
    ] {
        let out = cli(
            d.path(),
            &["--only", &selected, "--stdin", "a.gemspec"],
            Some(&source),
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(counts(&out), [1, 1, 1], "{out:?}");
    }
    for source in [
        format!("{GEMSPEC}\ndef (").into_bytes(),
        format!("# coding: no-such-encoding\n{GEMSPEC}").into_bytes(),
    ] {
        let out = cli(
            d.path(),
            &["--only", &selected, "--stdin", "a.gemspec"],
            Some(&source),
        );
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(counts(&out), [0, 0, 0], "{out:?}");
    }
}

#[test]
fn supplied_inputs_are_discovered_but_never_evaluated() {
    let d = project();
    let guard = "File.write('EXECUTED', 'never')\nraise 'do not execute'\n";
    fs::create_dir(d.path().join("app")).unwrap();
    fs::write(d.path().join("app/x.gemspec"), format!("{guard}{GEMSPEC}")).unwrap();
    fs::write(d.path().join("app/x.rb"), GEMSPEC).unwrap();
    fs::write(d.path().join("Gemfile"), GEMSPEC).unwrap();
    for fix in [false, true] {
        let selected = ALL.join(",");
        let mut args = vec!["--only", selected.as_str(), "."];
        if fix {
            args.push("--fix");
        }
        let out = cli(d.path(), &args, None);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(counts(&out), [1, 1, 1], "{out:?}");
        assert!(!d.path().join("EXECUTED").exists());
        assert_eq!(
            fs::read_to_string(d.path().join("app/x.gemspec")).unwrap(),
            format!("{guard}{GEMSPEC}")
        );
    }
}

#[test]
fn guarded_corrections_compose_with_atomic_no_write_guarantees() {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    let d = project();
    let path = d.path().join("a.gemspec");
    // Live findings keep their bytes, mode and inode; a peer's fix applies.
    for (only, written) in [
        (ALL.join(","), GEMSPEC.to_owned()),
        (
            format!("{},Style/StringLiterals", ALL.join(",")),
            GEMSPEC.replace('\'', "\""),
        ),
    ] {
        fs::write(&path, GEMSPEC).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = cli(d.path(), &["--only", &only, "--fix", "a.gemspec"], None);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(counts(&out), [1, 1, 1], "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), written);
        if written == GEMSPEC {
            let new = fs::metadata(&path).unwrap();
            assert_eq!(old.modified().unwrap(), new.modified().unwrap());
            assert_eq!(old.permissions(), new.permissions());
            #[cfg(unix)]
            assert_eq!(old.ino(), new.ino());
        }
    }
    // In the closed inactive envelope the canonical directive is inserted;
    // the file is then clean and stable, and not rewritten again.
    fs::write(&path, ENVELOPE).unwrap();
    let flags = ["--only", "Gemspec", "--fix", "a.gemspec"];
    let out = cli(d.path(), &flags, None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), ENVELOPE_FIXED);
    let old = fs::metadata(&path).unwrap();
    let out = cli(d.path(), &flags, None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    // Value replacement and pair insertion in one run, across two blocks.
    let source = "if false\n  Gem::Specification.new do |s|\n    s.metadata = { 'rubygems_mfa_required' => 'false' }\n  end\n  Gem::Specification.new do |t|\n    t.metadata = {\n      'a' => 'b'\n    }\n  end\nend\n";
    fs::write(&path, source).unwrap();
    let out = cli(d.path(), &["--only", MFA, "--fix", "a.gemspec"], None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("=> 'false'", "=> 'true'").replace(
            "'a' => 'b'\n",
            "'a' => 'b',\n'rubygems_mfa_required' => 'true'\n"
        )
    );
    // An enclosing block and its specification block both want the same
    // pair: it is inserted once, and the result has no duplicated key.
    let source = "if false\n  foo do\n    Gem::Specification.new do |s|\n      s.metadata = { 'a' => 'b' }\n    end\n  end\nend\n";
    fs::write(&path, source).unwrap();
    let out = cli(d.path(), &["--only", MFA, "--fix", "a.gemspec"], None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace(
            "'a' => 'b' }",
            "'a' => 'b',\n'rubygems_mfa_required' => 'true' }"
        )
    );
    // Listed observers, data sections, directives and held shapes keep the
    // bytes; the version cops never edit.
    for source in [
        ENVELOPE.replace("'b'", "__FILE__"),
        ENVELOPE.replace("'b'", "caller"),
        format!("{ENVELOPE}__END__\n"),
        ENVELOPE.replace("    s.metadata['a'] = 'b'\n", "    s.metadata['a'] = 'b' # keep\n"),
        ENVELOPE.replace("  Gem::", "  # rubocop:disable Gemspec/RequireMFA\n  Gem::"),
        ENVELOPE.replace("'b'", "<<~E\n      b\n    E"),
        "if false\n  Gem::Specification.new do |s|\n    s.metadata['rubygems_mfa_required'] = 'false'\n    s.required_ruby_version = '>= 2.7'\n    RUBY_VERSION\n  end\nend\n__END__\n".to_owned(),
    ] {
        fs::write(&path, &source).unwrap();
        let old = fs::metadata(&path).unwrap();
        cli(d.path(), &flags, None);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
    }
    // Stdin is never written; a read-only file is reported, not changed.
    fs::write(&path, b"sentinel").unwrap();
    let out = cli(
        d.path(),
        &["--only", "Gemspec", "--stdin", "a.gemspec", "--fix"],
        Some(ENVELOPE.as_bytes()),
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
        "{out:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), b"sentinel");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(&path, ENVELOPE).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = cli(d.path(), &flags, None);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), ENVELOPE);
        let new = fs::metadata(&path).unwrap();
        assert_eq!((old.ino(), old.mtime_nsec()), (new.ino(), new.mtime_nsec()));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
}
