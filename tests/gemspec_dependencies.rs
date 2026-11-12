//! Gemspec-dependencies family through the actual
//! CLI: supplied paths, selectors, opt-in, directives, encodings, fatal
//! inputs, and the guarded corrections' no-write and atomic guarantees.
//! Gemspecs and Gemfiles are linted, never evaluated.
use crate::support::stdin;
use rustocop::cops::Registry;
use std::{fs, path::Path, process::Command};

const RUNTIME: &str = "Gemspec/AddRuntimeDependency";
const VERSION: &str = "Gemspec/DependencyVersion";
const DEVELOPMENT: &str = "Gemspec/DevelopmentDependencies";
const DEPRECATED: &str = "Gemspec/DeprecatedAttributeAssignment";
const ALL: [&str; 4] = [RUNTIME, VERSION, DEVELOPMENT, DEPRECATED];

/// One finding for each of the four cops.
const GEMSPEC: &str = "Gem::Specification.new do |s|\n  s.date = '2024-01-01'\n  s.add_runtime_dependency 'a', '~> 1'\n  s.add_dependency 'b'\n  s.add_development_dependency 'c', '~> 2'\nend\n";
const ENVELOPE: &str = "if false\n  Gem::Specification.new do |s|\n    s.test_files = Dir['t/*']\n    s.add_runtime_dependency 'a', '~> 1'\n  end\nend\n";
const ENVELOPE_FIXED: &str =
    "if false\n  Gem::Specification.new do |s|\n    s.add_dependency 'a', '~> 1'\n  end\nend\n";

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

fn counts(out: &std::process::Output) -> [usize; 4] {
    ALL.map(|cop| count(out, cop))
}

#[test]
fn registry_metadata_defaults_and_no_knobs() {
    let registry = Registry::new();
    for (name, enabled, safe) in [
        (RUNTIME, true, true),
        (VERSION, false, false),
        (DEVELOPMENT, true, false),
        (DEPRECATED, true, true),
    ] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert_eq!(cop.default_enabled(), enabled, "{name}");
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
            "enforced_style='forbidden'",
            "allowed_gems=['a']",
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
}

#[test]
fn supplied_paths_selectors_config_and_directives_cannot_bypass_gates() {
    let d = project();
    let run = |args: &[&str], path: &str, source: &str| {
        let mut all = args.to_vec();
        all.extend(["--stdin", path]);
        counts(&cli(d.path(), &all, Some(source.as_bytes())))
    };
    // Default: DependencyVersion is opt-in; the other three are on.
    assert_eq!(run(&[], "a.gemspec", GEMSPEC), [1, 0, 1, 1]);
    let selected = ALL.join(",");
    for (path, expected) in [
        ("a.gemspec", [1, 1, 1, 1]),
        ("nested/.x.gemspec", [1, 1, 1, 1]),
        ("Gemfile", [0, 0, 1, 0]),
        ("nested/gems.rb", [0, 0, 1, 0]),
        ("x.gemfile", [0, 0, 0, 0]),
        ("Gemfile.lock", [0, 0, 0, 0]),
        ("a.rb", [0, 0, 0, 0]),
        ("gemspec", [0, 0, 0, 0]),
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
            format!("[cops.\"{VERSION}\"]\nenabled=true\n"),
            vec![],
            [1, 1, 1, 1],
        ),
        (
            "[cops.Gemspec]\nenabled=false\n".to_owned(),
            vec![],
            [0, 0, 0, 0],
        ),
        (
            "[cops.Gemspec]\nenabled=false\n".to_owned(),
            vec!["--only", VERSION],
            [0, 1, 0, 0],
        ),
        (String::new(), vec!["--except", RUNTIME], [0, 0, 1, 1]),
        (String::new(), vec!["--only", DEPRECATED], [0, 0, 0, 1]),
        ("exclude=['a.gemspec']\n".to_owned(), vec![], [0, 0, 0, 0]),
        (
            format!("[cops.\"{DEVELOPMENT}\"]\nexclude=['a.gemspec']\n"),
            vec![],
            [1, 0, 0, 1],
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
            ("all", [0, 0, 0, 0]),
            ("Gemspec", [0, 0, 0, 0]),
            (RUNTIME, [0, 1, 1, 1]),
            ("Bundler", [1, 1, 1, 1]),
        ] {
            let source = format!("# {prefix}:disable {target}\n{GEMSPEC}").replace('\n', "\r\n");
            assert_eq!(run(&["--only", &selected], "a.gemspec", &source), expected);
        }
        // A trailing directive on the reported line only.
        let source = GEMSPEC.replace(
            "'2024-01-01'\n",
            &format!("'2024-01-01' # {prefix}:disable {DEPRECATED}\n"),
        );
        assert_eq!(run(&[], "a.gemspec", &source), [1, 0, 1, 0]);
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
        format!("{GEMSPEC}__END__\ns.date = 1\ninvalid ruby {{[").into_bytes(),
    ] {
        let out = cli(
            d.path(),
            &["--only", &selected, "--stdin", "a.gemspec"],
            Some(&source),
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(counts(&out), [1, 1, 1, 1], "{out:?}");
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
        assert_eq!(counts(&out), [0, 0, 0, 0], "{out:?}");
    }
}

#[test]
fn supplied_inputs_are_discovered_but_never_evaluated() {
    let d = project();
    let guard = "File.write('EXECUTED', 'never')\nraise 'do not execute'\n";
    fs::create_dir(d.path().join("app")).unwrap();
    fs::write(d.path().join("app/x.gemspec"), format!("{guard}{GEMSPEC}")).unwrap();
    fs::write(
        d.path().join("Gemfile"),
        format!("{guard}add_development_dependency 'x'\n"),
    )
    .unwrap();
    fs::write(d.path().join("app/x.rb"), GEMSPEC).unwrap();
    for fix in [false, true] {
        let selected = ALL.join(",");
        let mut args = vec!["--only", selected.as_str(), "."];
        if fix {
            args.push("--fix");
        }
        let out = cli(d.path(), &args, None);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(counts(&out), [1, 1, 2, 1], "{out:?}");
        assert!(!d.path().join("EXECUTED").exists());
        assert_eq!(
            fs::read_to_string(d.path().join("app/x.gemspec")).unwrap(),
            format!("{guard}{GEMSPEC}")
        );
    }
}

#[test]
fn guarded_corrections_compose_with_atomic_no_write_guarantees() {
    let d = project();
    let path = d.path().join("a.gemspec");
    // Live findings keep their bytes; a peer's fix still applies.
    for (only, written) in [
        (format!("{RUNTIME},{DEPRECATED}"), GEMSPEC.to_owned()),
        (
            format!("{RUNTIME},{DEPRECATED},Style/StringLiterals"),
            GEMSPEC.replace('\'', "\""),
        ),
    ] {
        fs::write(&path, GEMSPEC).unwrap();
        let out = cli(d.path(), &["--only", &only, "--fix", "a.gemspec"], None);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), written);
    }
    // In the closed inactive envelope both canonical edits apply in one run,
    // then the file is stable and is not rewritten.
    fs::write(&path, ENVELOPE).unwrap();
    // The four cops of this family (the department has grown since).
    let only = ALL.join(",");
    let flags = ["--only", only.as_str(), "--fix", "a.gemspec"];
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
    // Removals are checked together: two that each keep a local read would
    // together leave it unused and warned about, so one is held.
    let shared = "if false\n  x = 1\n  Gem::Specification.new do |s|\n    s.date = x\n  end\n  Gem::Specification.new do |t|\n    t.date = x\n  end\nend\n";
    fs::write(&path, shared).unwrap();
    let out = cli(d.path(), &flags, None);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        shared.replacen("    s.date = x\n", "", 1)
    );
    // Listed observers, data sections and directives hold the edits.
    for source in [
        ENVELOPE.replace("Dir['t/*']", "__FILE__"),
        format!("{ENVELOPE}__END__\n"),
        ENVELOPE
            .replace("'t/*']\n", "'t/*'] # rubocop:disable Gemspec\n")
            .replace("'~> 1'\n", "'~> 1' # rustocop:disable Gemspec\n"),
    ] {
        fs::write(&path, &source).unwrap();
        cli(d.path(), &flags, None);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
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
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
