//! Ordered-dependencies family: source-only fixtures from
//! the pinned RuboCop (MIT, (c) 2012-26 Bozhidar Batsov), replayed through
//! the pipeline and the actual CLI. Gemfiles and gemspecs are linted, never
//! evaluated.
use rustocop::{config::ResolvedConfig, cops::Registry, offense::Severity, pipeline};
#[macro_use]
#[path = "support/parallel_rows.rs"]
mod parallel_rows;
#[path = "support/stdin.rs"]
mod stdin;
use std::{fs, path::Path, process::Command};

const GEMS: &str = "Bundler/OrderedGems";
const DEPS: &str = "Gemspec/OrderedDependencies";

fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/ordered_dependencies.json")).unwrap()
}

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}

fn bytes(row: &serde_json::Value, key: &str) -> Option<Vec<u8>> {
    row.get(key)
        .map(|value| serde_json::from_value(value.clone()).unwrap())
}

fn findings(row: &serde_json::Value) -> Vec<(u32, u32, String)> {
    serde_json::from_value(
        row.get("findings")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .unwrap()
}

fn origin(row: &serde_json::Value) -> &str {
    row["origin"].as_str().unwrap().split(':').next().unwrap()
}

/// The project edit: the upstream canonical swap, applied only in the
/// `envelope` rows (closed top-level `if false ... end` programs whose
/// declarations are whole movable lines). Line endings are preserved (the
/// reference buffer folds CRLF). Every other upstream edit is held.
fn project_after(row: &serde_json::Value) -> Option<Vec<u8>> {
    if origin(row) != "envelope" {
        return None;
    }
    let after =
        String::from_utf8(bytes(row, "after").expect("envelope rows are corrected")).unwrap();
    let source = bytes(row, "bytes").unwrap();
    let crlf = source.windows(2).any(|w| w == b"\r\n");
    Some(
        if crlf {
            after.replace('\n', "\r\n")
        } else {
            after
        }
        .into_bytes(),
    )
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

/// Offense lines of the text format for `cop` (messages may span lines).
fn count(out: &std::process::Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!(" ({cop})\n"))
        .count()
}

#[test]
fn fixture_covers_every_main_source_control_and_envelope_example() {
    let rows = rows();
    for name in [GEMS, DEPS] {
        let origins: Vec<_> = rows
            .iter()
            .filter(|r| r["cop"] == name)
            .map(origin)
            .collect();
        for kind in ["main", "source", "control", "envelope"] {
            assert!(origins.contains(&kind), "{name} {kind}");
        }
    }
    let main = |name: &str| {
        rows.iter()
            .filter(|r| r["cop"] == name && origin(r) == "main")
            .count()
    };
    // Every pinned main example (three shared-example expansions included).
    assert_eq!((main(GEMS), main(DEPS)), (18, 19));
    assert!(rows.iter().all(|r| r.get("parser_errors").is_none()));
    // Only the 16 envelope rows are corrected; every other upstream edit is held.
    let upstream = rows.iter().filter(|r| r.get("after").is_some()).count();
    let project = rows.iter().filter(|r| project_after(r).is_some()).count();
    assert_eq!((upstream, project), (86, 16));
}

#[test]
fn all_fixture_rows_match_the_exact_pinned_reference() {
    let d = project();
    let registry = Registry::new();
    for row in rows() {
        let cop = row["cop"].as_str().unwrap();
        let path = row["path"].as_str().unwrap();
        let config = ResolvedConfig::default();
        let selected = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
        let source = bytes(&row, "bytes").unwrap();
        let report = pipeline::lint_one(
            &d.path().join(path),
            Path::new(path),
            Some(&source),
            &selected,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        assert!(report.errors.is_empty(), "{row}: {report:?}");
        let mut found: Vec<_> = report
            .result
            .offenses
            .into_iter()
            .map(|o| {
                assert_eq!(o.cop, cop);
                (o.line, o.col, o.message)
            })
            .collect();
        found.sort();
        assert_eq!(found, findings(&row), "{row}");
    }
}

#[test]
fn fixture_cli_lint_and_fix_match_the_reference_and_held_edits_never_write() {
    use std::io::Read;
    parallel_rows!(for row in rows() => {
        let d = project();
        let cop = row["cop"].as_str().unwrap();
        let name = row["path"].as_str().unwrap();
        let path = d.path().join(name);
        let source = bytes(&row, "bytes").unwrap();
        let after = project_after(&row);
        let expected = findings(&row);
        for fix in [false, true] {
            fs::write(&path, &source).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
            }
            let old = fs::metadata(&path).unwrap();
            let mut held = fs::File::open(&path).unwrap();
            let mut args = vec!["--only", cop];
            if fix {
                args.extend(["--fix", "--fix-passes", "1"]);
            }
            args.push(name);
            let out = cli(d.path(), &args, None);
            assert!(out.stderr.is_empty(), "{row}: {out:?}");
            let stdout = String::from_utf8(out.stdout.clone()).unwrap();
            let corrected = fix && after.is_some();
            // A corrected finding is not listed again; what the single pass
            // leaves in the written source is (the pinned reference's).
            let left = row.get("after_findings").map(|_| {
                serde_json::from_value::<Vec<(u32, u32, String)>>(row["after_findings"].clone()).unwrap()
            });
            let remaining = if corrected { &left.as_ref().unwrap()[..] } else { &expected[..] };
            let listed: String = remaining
                .iter()
                .map(|(line, col, message)| format!("{name}:{line}:{col}: C: {message} ({cop})\n"))
                .collect();
            assert!(stdout.starts_with(&listed), "fix={fix}: {row}: {stdout}");
            assert_eq!(count(&out, cop), remaining.len(), "fix={fix}: {row}: {stdout}");
            assert_eq!(
                out.status.code(),
                Some(i32::from(!remaining.is_empty())),
                "fix={fix}: {row}: {out:?}"
            );
            let mut held_bytes = Vec::new();
            held.read_to_end(&mut held_bytes).unwrap();
            assert_eq!(held_bytes, source);
            let new = fs::metadata(&path).unwrap();
            assert_eq!(old.permissions(), new.permissions());
            if corrected {
                let written = fs::read(&path).unwrap();
                assert_eq!(written, after.clone().unwrap(), "{row}");
                // The edit is a pure reorder of whole lines.
                let lines = |b: &[u8]| {
                    let mut l: Vec<Vec<u8>> = b.split_inclusive(|c| *c == b'\n').map(<[u8]>::to_vec).collect();
                    l.sort();
                    l
                };
                assert_eq!(lines(&written), lines(&source), "{row}");
                continue;
            }
            assert_eq!(fs::read(&path).unwrap(), source);
            assert_eq!(old.modified().unwrap(), new.modified().unwrap());
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(
                    (old.ino(), old.dev(), old.mtime_nsec()),
                    (new.ino(), new.dev(), new.mtime_nsec())
                );
            }
        }
    });
}

#[test]
fn registry_admits_the_family_with_fixed_metadata() {
    let registry = Registry::new();
    for (name, department) in [(GEMS, "Bundler"), (DEPS, "Gemspec")] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled(), "{name}");
        assert!(cop.safe_fixes(), "{name}");
        assert_eq!(cop.severity(), Severity::Convention, "{name}");
        assert!(cop.max_setting().is_none(), "{name}");
        assert_eq!(cop.ruby_versions(), Default::default(), "{name}");
        assert!(cop.path_policy().is_some(), "{name}");
        assert!(name.starts_with(department));
    }
}

const GEMFILE: &str = "gem 'b'\ngem 'a'\n";
const GEMSPEC: &str =
    "Gem::Specification.new do |spec|\n  spec.add_dependency 'b'\n  spec.add_dependency 'a'\nend\n";

#[test]
fn supplied_paths_selectors_config_and_directives_cannot_bypass_gates() {
    let d = project();
    for (path, gems, deps) in [
        ("Gemfile", 1, 0),
        ("nested/Gemfile", 1, 0),
        ("x.gemfile", 1, 0),
        ("gems.rb", 1, 0),
        ("a.gemspec", 0, 1),
        (".hidden/a.gemspec", 0, 1),
        ("a.rb", 0, 0),
        ("Gemfile.rb", 0, 0),
        ("gemspec", 0, 0),
        ("Gemfile.lock", 0, 0),
    ] {
        for selector in [format!("{GEMS},{DEPS}"), "Bundler,Gemspec".to_owned()] {
            let source = format!("{GEMFILE}{GEMSPEC}");
            let out = cli(
                d.path(),
                &["--only", &selector, "--stdin", path],
                Some(source.as_bytes()),
            );
            assert_eq!(
                (count(&out, GEMS), count(&out, DEPS)),
                (gems, deps),
                "{path}: {out:?}"
            );
        }
    }
    // Default-on: no selector at all still reports both.
    let run = |args: &[&str], path: &str, source: &str| {
        let mut all = args.to_vec();
        all.extend(["--stdin", path]);
        let out = cli(d.path(), &all, Some(source.as_bytes()));
        (count(&out, GEMS), count(&out, DEPS))
    };
    assert_eq!(run(&[], "Gemfile", GEMFILE), (1, 0));
    assert_eq!(run(&[], "a.gemspec", GEMSPEC), (0, 1));
    for (config, flags, expected) in [
        ("[cops.Bundler]\nenabled=false\n", vec![], (0, 1)),
        ("[cops.Gemspec]\nenabled=false\n", vec![], (1, 0)),
        (
            &format!("[cops.\"{GEMS}\"]\nenabled=false\n"),
            vec![],
            (0, 1),
        ),
        ("", vec!["--except", GEMS], (0, 1)),
        ("", vec!["--except", "Gemspec"], (1, 0)),
        ("", vec!["--only", GEMS], (1, 0)),
        (
            "[cops.Bundler]\nenabled=false\n",
            vec!["--only", "Bundler"],
            (1, 0),
        ),
        ("exclude=['Gemfile']\n", vec![], (0, 1)),
        (
            &format!("[cops.\"{DEPS}\"]\nexclude=['a.gemspec']\n"),
            vec![],
            (1, 0),
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let found = (
            run(&flags, "Gemfile", GEMFILE).0,
            run(&flags, "a.gemspec", GEMSPEC).1,
        );
        assert_eq!(found, expected, "{flags:?}: {config}");
    }
    // Fixed TreatCommentsAsGroupSeparators/ConsiderPunctuation: no knobs.
    for key in [
        "treat_comments_as_group_separators=false",
        "consider_punctuation=true",
        "include=['**/*']",
        "max=1",
    ] {
        for name in [GEMS, DEPS] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{name}\"]\n{key}\n"),
            )
            .unwrap();
            // A rejected config exits before reading stdin: send none.
            let out = cli(d.path(), &["--stdin", "Gemfile"], Some(b""));
            assert_eq!(out.status.code(), Some(2), "{key}: {out:?}");
        }
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for (target, expected) in [("all", 0), ("Bundler", 0), (GEMS, 0), ("Gemspec", 1)] {
            let source = format!("# {prefix}:disable {target}\n\n{GEMFILE}").replace('\n', "\r\n");
            assert_eq!(
                run(&["--only", "Bundler,Gemspec"], "Gemfile", &source).0,
                expected,
                "{source}"
            );
        }
        // A trailing directive on the reported line only.
        let source = format!("gem 'b'\ngem 'a' # {prefix}:disable {GEMS}\n");
        assert_eq!(run(&[], "Gemfile", &source), (0, 0));
        let source = GEMSPEC.replace("'a'\n", &format!("'a' # {prefix}:disable {DEPS}\n"));
        assert_eq!(run(&[], "a.gemspec", &source), (0, 0));
    }
}

#[test]
fn encodings_data_sections_and_fatal_inputs_compose() {
    let d = project();
    for source in [
        format!("\u{feff}{GEMFILE}"),
        GEMFILE.replace('\n', "\r\n"),
        format!("# coding: Windows-1252\n{GEMFILE}"),
        format!("{GEMFILE}__END__\ngem 'z'\ninvalid ruby {{["),
    ] {
        let out = cli(
            d.path(),
            &["--only", GEMS, "--stdin", "Gemfile"],
            Some(source.as_bytes()),
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, GEMS), 1, "{out:?}");
    }
    for source in [
        format!("{GEMFILE}\ndef (").into_bytes(),
        b"# coding: no-such-encoding\ngem 'b'\ngem 'a'\n".to_vec(),
    ] {
        let out = cli(
            d.path(),
            &["--only", GEMS, "--stdin", "Gemfile"],
            Some(&source),
        );
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out, GEMS), 0, "{out:?}");
    }
    // Windows-1252 bytes compare as bytes and are never rewritten.
    let path = d.path().join("Gemfile");
    let source = b"# coding: Windows-1252\nif false\n  gem '\xe9'\n  gem 'e'\nend\n";
    fs::write(&path, source).unwrap();
    let out = cli(d.path(), &["--only", GEMS, "--fix", "Gemfile"], None);
    assert_eq!(count(&out, GEMS), 1, "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
}

#[test]
fn supplied_inputs_are_discovered_but_never_evaluated() {
    let d = project();
    let guard = "File.write('EXECUTED', 'never')\nraise 'do not execute'\n";
    fs::create_dir(d.path().join("app")).unwrap();
    for (name, body) in [
        ("Gemfile", GEMFILE),
        ("app/x.gemfile", GEMFILE),
        ("app/gems.rb", GEMFILE),
        ("app/x.gemspec", GEMSPEC),
    ] {
        fs::write(d.path().join(name), format!("{guard}{body}")).unwrap();
    }
    fs::write(d.path().join("app/Gemfile.lock"), GEMFILE).unwrap();
    for fix in [false, true] {
        let mut args = vec![
            "--only",
            "Bundler/OrderedGems,Gemspec/OrderedDependencies",
            ".",
        ];
        if fix {
            args.push("--fix");
        }
        let out = cli(d.path(), &args, None);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!((count(&out, GEMS), count(&out, DEPS)), (3, 1), "{out:?}");
        assert!(!d.path().join("EXECUTED").exists());
    }
}

const ENVELOPE: &str = "if false\n  gem 'b'\n  gem 'a'\nend\n";

#[test]
fn envelope_swaps_compose_with_peers_and_atomic_no_write_guarantees() {
    let d = project();
    let path = d.path().join("Gemfile");
    // A live Gemfile keeps its finding and order, alone or next to a peer
    // fix (which still applies).
    for (only, written) in [
        (GEMS, GEMFILE),
        (
            "Bundler/OrderedGems,Style/StringLiterals",
            "gem \"b\"\ngem \"a\"\n",
        ),
    ] {
        fs::write(&path, GEMFILE).unwrap();
        let out = cli(d.path(), &["--only", only, "--fix", "Gemfile"], None);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, GEMS), 1, "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), written);
    }
    // In the envelope the swap and a peer's overlapping quote fixes resolve
    // over runs (first edit wins); the file converges and then is stable.
    fs::write(&path, ENVELOPE).unwrap();
    let flags = [
        "--only",
        "Bundler/OrderedGems,Style/StringLiterals",
        "--fix",
        "Gemfile",
    ];
    for _ in 0..3 {
        cli(d.path(), &flags, None);
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "if false\n  gem \"a\"\n  gem \"b\"\nend\n"
    );
    let old = fs::metadata(&path).unwrap();
    let out = cli(d.path(), &flags, None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    // Both directive prefixes keep the finding and the bytes.
    for prefix in ["rubocop", "rustocop"] {
        let disabled = format!("if false\n  gem 'b'\n  gem 'a' # {prefix}:disable {GEMS}\nend\n");
        fs::write(&path, &disabled).unwrap();
        let out = cli(d.path(), &["--only", GEMS, "--fix", "Gemfile"], None);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), disabled);
    }
    // Stdin input is never written; a read-only file is reported, not changed.
    fs::write(&path, b"sentinel").unwrap();
    let out = cli(
        d.path(),
        &["--only", GEMS, "--stdin", "Gemfile", "--fix"],
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
        let out = cli(d.path(), &["--only", GEMS, "--fix", "Gemfile"], None);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), ENVELOPE);
        let new = fs::metadata(&path).unwrap();
        assert_eq!((old.ino(), old.mtime_nsec()), (new.ino(), new.mtime_nsec()));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    // A swap that would rebind a local is held: finding kept, file untouched.
    #[cfg(unix)]
    for (name, only, source) in [
        (
            "Gemfile",
            GEMS,
            "if false\n  gem 'b', x = 1\n  gem 'a', x\nend\n",
        ),
        (
            "x.gemspec",
            DEPS,
            "if false\n  s = 1\n  s.add_dependency 'b', x = 1\n  s.add_dependency 'a', x\nend\n",
        ),
    ] {
        use std::os::unix::fs::MetadataExt;
        let target = d.path().join(name);
        fs::write(&target, source).unwrap();
        let old = fs::metadata(&target).unwrap();
        let out = cli(d.path(), &["--only", only, "--fix", name], None);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, only), 1, "{out:?}");
        assert_eq!(fs::read_to_string(&target).unwrap(), source);
        let new = fs::metadata(&target).unwrap();
        assert_eq!((old.ino(), old.mtime_nsec()), (new.ino(), new.mtime_nsec()));
    }
    // The gemspec envelope converges the same way.
    let spec = d.path().join("x.gemspec");
    let source = "if false\n  s = 1\n  s.add_dependency 'c'\n  s.add_dependency 'b'\n  s.add_dependency 'a'\nend\n";
    fs::write(&spec, source).unwrap();
    for _ in 0..3 {
        cli(d.path(), &["--only", DEPS, "--fix", "x.gemspec"], None);
    }
    assert_eq!(
        fs::read_to_string(&spec).unwrap(),
        "if false\n  s = 1\n  s.add_dependency 'a'\n  s.add_dependency 'b'\n  s.add_dependency 'c'\nend\n"
    );
}
