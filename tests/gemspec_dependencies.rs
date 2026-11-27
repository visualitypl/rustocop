//! Gemspec-dependencies family through the actual
//! CLI: supplied paths, selectors, opt-in, directives, encodings, fatal
//! inputs, and the guarded corrections' no-write and atomic guarantees.
//! Gemspecs and Gemfiles are linted, never evaluated.
use crate::support::cli::project;
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
            rustocop::offense::Severity::for_cop(name),
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
            "Include=['**/*']",
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
