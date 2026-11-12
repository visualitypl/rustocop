//! Bundler declaration family: source-only fixtures from the
//! pinned RuboCop (MIT, (c) 2012-26 Bozhidar Batsov), replayed through the
//! pipeline and the actual CLI. Gemfiles are linted, never evaluated.
use rustocop::{config::ResolvedConfig, cops::Registry, offense::Severity, pipeline};
#[macro_use]
#[path = "support/parallel_rows.rs"]
mod parallel_rows;
#[path = "support/stdin.rs"]
mod stdin;
use std::{fs, path::Path, process::Command};

/// The landed family members (registry order) and whether they are on by
/// default and correct safely.
const NAMES: [(&str, bool, bool); 5] = [
    ("Bundler/DuplicatedGem", true, false),
    ("Bundler/DuplicatedGroup", true, false),
    ("Bundler/GemComment", false, false),
    ("Bundler/GemVersion", false, false),
    ("Bundler/InsecureProtocolSource", true, true),
];

fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/bundler_declarations.json")).unwrap()
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

/// InsecureProtocolSource's project edit: the upstream canonical replacement,
/// applied only to an exact `source :name` that is the sole statement of a
/// closed top-level `if false ... end`; live sources keep their findings.
/// Line endings are preserved (the reference buffer folds CRLF).
fn project_after(row: &serde_json::Value) -> Option<Vec<u8>> {
    let after = bytes(row, "after")?;
    let source = bytes(row, "bytes").unwrap();
    let text = String::from_utf8(source).ok()?;
    let envelope = regex::Regex::new(
        r"\Aif false(?: then |\r?\n  )source :(gemcutter|rubygems|rubyforge)(?: # why)?(?: end|\r?\nend)\r?\n\z",
    )
    .unwrap();
    let name = envelope
        .captures(&text)?
        .get(1)
        .unwrap()
        .as_str()
        .to_owned();
    let edited = text.replacen(&format!(":{name}"), "'https://rubygems.org'", 1);
    assert_eq!(edited.replace("\r\n", "\n").into_bytes(), after, "{row}");
    Some(edited.into_bytes())
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
fn fixture_covers_every_main_and_source_example_of_each_landed_member() {
    let rows = rows();
    for (name, _, _) in NAMES {
        let origins: Vec<_> = rows
            .iter()
            .filter(|r| r["cop"] == name)
            .map(|r| r["origin"].as_str().unwrap().split(':').next().unwrap())
            .collect();
        assert!(origins.contains(&"main"), "{name}");
        assert!(origins.contains(&"source"), "{name}");
        assert!(origins.contains(&"control"), "{name}");
    }
    assert!(rows.iter().all(|r| r.get("parser_errors").is_none()));
    // Only six envelope rows are corrected; every other upstream edit is held.
    let upstream = rows.iter().filter(|r| r.get("after").is_some()).count();
    let project = rows.iter().filter(|r| project_after(r).is_some()).count();
    assert_eq!((upstream, project), (27, 6));
}

#[test]
fn all_fixture_rows_match_the_exact_pinned_reference() {
    let d = project();
    let registry = Registry::new();
    for row in rows() {
        let cop = row["cop"].as_str().unwrap();
        let path = row["path"].as_str().unwrap();
        assert!(registry.has_cop(cop), "{cop} is not implemented");
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
                assert_eq!(o.end_line, o.line);
                (o.line, o.col, o.message)
            })
            .collect();
        found.sort();
        assert_eq!(found, findings(&row), "{row}");
    }
}

#[test]
fn fixture_cli_lint_and_fix_match_the_reference_and_detection_never_writes() {
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
                args.push("--fix");
            }
            args.push(name);
            let out = cli(d.path(), &args, None);
            assert!(out.stderr.is_empty(), "{row}: {out:?}");
            let stdout = String::from_utf8(out.stdout.clone()).unwrap();
            let corrected = fix && after.is_some();
            // A corrected finding is not listed again.
            let remaining = if corrected { &[][..] } else { &expected[..] };
            let listed: String = remaining
                .iter()
                .map(|(line, col, message)| format!("{name}:{line}:{col}: C: {message} ({cop})\n"))
                .collect();
            assert!(stdout.starts_with(&listed), "fix={fix}: {row}: {stdout}");
            assert_eq!(count(&out, cop), remaining.len(), "fix={fix}: {row}: {stdout}");
            assert_eq!(
                out.status.code(),
                Some(i32::from(!expected.is_empty() && !corrected)),
                "fix={fix}: {row}: {out:?}"
            );
            let mut held_bytes = Vec::new();
            held.read_to_end(&mut held_bytes).unwrap();
            assert_eq!(held_bytes, source);
            let new = fs::metadata(&path).unwrap();
            assert_eq!(old.permissions(), new.permissions());
            if corrected {
                assert_eq!(fs::read(&path).unwrap(), after.clone().unwrap(), "{row}");
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
fn registry_admits_exactly_the_landed_bundler_cops_with_fixed_metadata() {
    let registry = Registry::new();
    let mut landed: Vec<_> = NAMES.iter().map(|(name, _, _)| *name).collect();
    // Landed outside this family: GemFilename, OrderedGems.
    landed.extend(["Bundler/GemFilename", "Bundler/OrderedGems"]);
    landed.sort_unstable();
    let mut actual: Vec<_> = registry
        .cops()
        .iter()
        .map(|c| c.name())
        .filter(|name| name.starts_with("Bundler/"))
        .collect();
    actual.sort_unstable();
    assert_eq!(actual, landed);
    for (name, enabled, safe) in NAMES {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert_eq!(cop.default_enabled(), enabled, "{name}");
        assert_eq!(cop.safe_fixes(), safe, "{name}");
        assert_eq!(cop.severity(), Severity::Convention, "{name}");
        assert!(cop.max_setting().is_none(), "{name}");
        assert_eq!(cop.ruby_versions(), Default::default(), "{name}");
        assert!(cop.path_policy().is_some(), "{name}");
    }
}

const DUPLICATES: &str =
    "source 'https://rubygems.org'\ngem 'a'\ngem 'a'\ngroup :x do\nend\ngroup :x do\nend\n";

#[test]
fn supplied_paths_selectors_config_and_directives_cannot_bypass_gates() {
    let d = project();
    let both = "Bundler/DuplicatedGem,Bundler/DuplicatedGroup";
    for (path, eligible) in [
        ("Gemfile", true),
        ("nested/Gemfile", true),
        ("x.gemfile", true),
        ("gems.rb", true),
        (".hidden/gems.rb", true),
        ("a.rb", false),
        ("Gemfile.rb", false),
        ("a.gemspec", false),
    ] {
        for selector in ["Bundler", both] {
            let out = cli(
                d.path(),
                &["--only", selector, "--stdin", path],
                Some(DUPLICATES.as_bytes()),
            );
            let expected = usize::from(eligible);
            assert_eq!(
                count(&out, "Bundler/DuplicatedGem"),
                expected,
                "{path}: {out:?}"
            );
            assert_eq!(
                count(&out, "Bundler/DuplicatedGroup"),
                expected,
                "{path}: {out:?}"
            );
            assert_eq!(out.status.code(), Some(i32::from(eligible)), "{out:?}");
        }
    }
    // Default-on: no selector at all still reports both.
    let out = cli(
        d.path(),
        &["--stdin", "Gemfile"],
        Some(DUPLICATES.as_bytes()),
    );
    assert_eq!(count(&out, "Bundler/DuplicatedGem"), 1, "{out:?}");
    assert_eq!(count(&out, "Bundler/DuplicatedGroup"), 1, "{out:?}");
    for (config, flags, expected) in [
        (
            "",
            vec![
                "--except",
                "Style,Layout,Lint,Naming,Security,Metrics,Gemspec",
            ],
            2,
        ),
        ("[cops.Bundler]\nenabled=false\n", vec![], 0),
        (
            "[cops.Bundler]\nenabled=false\n",
            vec!["--only", "Bundler"],
            2,
        ),
        (
            "",
            vec!["--only", "Bundler", "--except", "Bundler/DuplicatedGem"],
            1,
        ),
        ("", vec!["--only", "Bundler", "--except", "Bundler"], 0),
        ("exclude=['Gemfile']\n", vec!["--only", "Bundler"], 0),
        (
            "[cops.\"Bundler/DuplicatedGroup\"]\nexclude=['Gemfile']\n",
            vec!["--only", "Bundler"],
            1,
        ),
        (
            "[cops.\"Bundler/DuplicatedGem\"]\nenabled=false\n",
            vec![],
            1,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = flags;
        args.extend(["--stdin", "Gemfile"]);
        let out = cli(d.path(), &args, Some(DUPLICATES.as_bytes()));
        let total = count(&out, "Bundler/DuplicatedGem") + count(&out, "Bundler/DuplicatedGroup");
        assert_eq!(total, expected, "{args:?}: {config}: {out:?}");
    }
    for key in [
        "max=1",
        "enforced_style='gems.rb'",
        "include=['**/*']",
        "allowed_gems=[]",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"Bundler/DuplicatedGem\"]\n{key}\n"),
        )
        .unwrap();
        let out = cli(d.path(), &["--stdin", "Gemfile"], Some(b""));
        assert_eq!(out.status.code(), Some(2), "{key}: {out:?}");
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for (target, gem, group) in [
            ("all", 0, 0),
            ("Bundler", 0, 0),
            ("Bundler/DuplicatedGem", 0, 1),
            ("Bundler/DuplicatedGroup", 1, 0),
        ] {
            let source = format!("# {prefix}:disable {target}\n{DUPLICATES}").replace('\n', "\r\n");
            let out = cli(
                d.path(),
                &["--only", "Bundler", "--stdin", "Gemfile"],
                Some(source.as_bytes()),
            );
            assert_eq!(count(&out, "Bundler/DuplicatedGem"), gem, "{out:?}");
            assert_eq!(count(&out, "Bundler/DuplicatedGroup"), group, "{out:?}");
        }
        // A trailing directive on the reported line only.
        let source = DUPLICATES.replace(
            "gem 'a'\ngroup",
            &format!("gem 'a' # {prefix}:disable Bundler/DuplicatedGem\ngroup"),
        );
        let out = cli(
            d.path(),
            &["--only", "Bundler", "--stdin", "Gemfile"],
            Some(source.as_bytes()),
        );
        assert_eq!(count(&out, "Bundler/DuplicatedGem"), 0, "{out:?}");
    }
}

#[test]
fn encodings_data_sections_and_fatal_inputs_compose() {
    let d = project();
    for source in [
        format!("\u{feff}{DUPLICATES}"),
        DUPLICATES.replace('\n', "\r\n"),
        format!("# coding: Windows-1252\n{DUPLICATES}"),
        format!("{DUPLICATES}__END__\ngem 'a'\ninvalid ruby {{["),
    ] {
        let out = cli(
            d.path(),
            &["--only", "Bundler", "--stdin", "Gemfile"],
            Some(source.as_bytes()),
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, "Bundler/DuplicatedGem"), 1, "{out:?}");
        assert_eq!(count(&out, "Bundler/DuplicatedGroup"), 1, "{out:?}");
    }
    for source in [
        format!("{DUPLICATES}\ndef (").into_bytes(),
        b"# coding: no-such-encoding\ngem 'a'\n".to_vec(),
        format!("{DUPLICATES}{}{}", "[".repeat(100), "]".repeat(100)).into_bytes(),
    ] {
        let out = cli(
            d.path(),
            &["--only", "Bundler", "--stdin", "Gemfile"],
            Some(&source),
        );
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out, "Bundler/DuplicatedGem"), 0, "{out:?}");
    }
    assert_eq!(
        cli(d.path(), &["--only", "Bundler", "missing/Gemfile"], None)
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn supplied_gemfiles_are_discovered_but_never_evaluated() {
    let d = project();
    let source = format!("File.write('EXECUTED', 'never')\nraise 'do not execute'\n{DUPLICATES}");
    fs::create_dir(d.path().join("app")).unwrap();
    for name in ["Gemfile", "app/x.gemfile", "app/gems.rb"] {
        fs::write(d.path().join(name), &source).unwrap();
    }
    fs::write(d.path().join("app/Gemfile.lock"), DUPLICATES).unwrap();
    for fix in [false, true] {
        let mut args = vec![
            "--only",
            "Bundler/DuplicatedGem,Bundler/DuplicatedGroup",
            ".",
        ];
        if fix {
            args.push("--fix");
        }
        let out = cli(d.path(), &args, None);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, "Bundler/DuplicatedGem"), 3, "{out:?}");
        assert_eq!(count(&out, "Bundler/DuplicatedGroup"), 3, "{out:?}");
        assert!(!d.path().join("EXECUTED").exists());
        for name in ["Gemfile", "app/x.gemfile", "app/gems.rb"] {
            assert_eq!(fs::read_to_string(d.path().join(name)).unwrap(), source);
        }
    }
}

#[test]
fn opt_in_comment_and_version_cops_run_only_when_selected_or_enabled() {
    let d = project();
    let source = b"gem 'a'\n# Why\ngem 'b', '~> 1'\n";
    let comment = "Bundler/GemComment";
    let version = "Bundler/GemVersion";
    let run = |args: &[&str]| {
        let mut all = args.to_vec();
        all.extend(["--stdin", "Gemfile"]);
        let out = cli(d.path(), &all, Some(source));
        (count(&out, comment), count(&out, version))
    };
    assert_eq!(run(&[]), (0, 0));
    assert_eq!(run(&["--except", "Bundler/DuplicatedGem"]), (0, 0));
    assert_eq!(run(&["--only", "Bundler"]), (1, 1));
    assert_eq!(run(&["--only", comment]), (1, 0));
    assert_eq!(run(&["--only", version]), (0, 1));
    assert_eq!(run(&["--only", "Bundler", "--except", comment]), (0, 1));
    for (config, expected) in [
        (format!("[cops.\"{comment}\"]\nenabled=true\n"), (1, 0)),
        (format!("[cops.\"{version}\"]\nenabled=true\n"), (0, 1)),
        // Existing department policy: an explicit department enable
        // includes its opt-in cops.
        ("[cops.Bundler]\nenabled=true\n".to_owned(), (1, 1)),
        (
            format!(
                "[cops.\"{comment}\"]\nenabled=true\nexclude=['Gemfile']\n[cops.\"{version}\"]\nenabled=true\n"
            ),
            (0, 1),
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), &config).unwrap();
        assert_eq!(run(&[]), expected, "{config}");
    }
    // Fixed OnlyFor/AllowedGems/EnforcedStyle: no public knobs.
    for key in [
        "only_for=[]",
        "allowed_gems=['rake']",
        "enforced_style='forbidden'",
    ] {
        for name in [comment, version] {
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
        let disabled =
            format!("gem 'a' # {prefix}:disable Bundler/GemComment, Bundler/GemVersion\n");
        let out = cli(
            d.path(),
            &["--only", "Bundler", "--stdin", "Gemfile"],
            Some(disabled.as_bytes()),
        );
        assert_eq!(
            (count(&out, comment), count(&out, version)),
            (0, 0),
            "{out:?}"
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
    }
    // Lookalike paths are never inspected, even when selected.
    for path in ["a.rb", "Gemfile.lock", "a.gemspec"] {
        let out = cli(
            d.path(),
            &[
                "--only",
                "Bundler/GemComment,Bundler/GemVersion",
                "--stdin",
                path,
            ],
            Some(source),
        );
        assert_eq!(
            (count(&out, comment), count(&out, version)),
            (0, 0),
            "{path}: {out:?}"
        );
    }
}

const ENVELOPE: &str = "if false\n  source :rubygems\nend\n";

#[test]
fn insecure_source_edits_only_the_inert_envelope_and_composes_with_peers() {
    let d = project();
    let path = d.path().join("Gemfile");
    let cop = "Bundler/InsecureProtocolSource";
    // A live source keeps its finding and its bytes, alone or with a peer.
    for only in [cop, "Bundler/InsecureProtocolSource,Style/StringLiterals"] {
        let live = "source :rubygems\ngem \"a\"\n";
        fs::write(&path, live).unwrap();
        let out = cli(
            d.path(),
            &["--only", only, "--fix", "--fix-passes", "1", "Gemfile"],
            None,
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, cop), 1, "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), live);
    }
    // The envelope edit is the upstream canonical one; a peer's quote fix
    // follows on the next run and then the file is stable.
    fs::write(&path, ENVELOPE).unwrap();
    let flags = [
        "--only",
        "Bundler/InsecureProtocolSource,Style/StringLiterals",
        "--fix",
        "--fix-passes",
        "1",
        "Gemfile",
    ];
    let out = cli(d.path(), &flags, None);
    assert_eq!(count(&out, cop), 0, "{out:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "if false\n  source 'https://rubygems.org'\nend\n"
    );
    cli(d.path(), &flags, None);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "if false\n  source \"https://rubygems.org\"\nend\n"
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
        let disabled = format!("if false\n  source :rubygems # {prefix}:disable {cop}\nend\n");
        fs::write(&path, &disabled).unwrap();
        let out = cli(
            d.path(),
            &["--only", cop, "--fix", "--fix-passes", "1", "Gemfile"],
            None,
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), disabled);
    }
    // Stdin input is never written; a read-only file is reported, not changed.
    fs::write(&path, b"sentinel").unwrap();
    let out = cli(
        d.path(),
        &[
            "--only",
            cop,
            "--stdin",
            "Gemfile",
            "--fix",
            "--fix-passes",
            "1",
        ],
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
        let out = cli(
            d.path(),
            &["--only", cop, "--fix", "--fix-passes", "1", "Gemfile"],
            None,
        );
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), ENVELOPE);
        let new = fs::metadata(&path).unwrap();
        assert_eq!((old.ino(), old.mtime_nsec()), (new.ino(), new.mtime_nsec()));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    // Default-on: plain runs report it; it never touches lookalike paths.
    let out = cli(
        d.path(),
        &["--stdin", "Gemfile"],
        Some(b"source :rubygems\n"),
    );
    assert_eq!(count(&out, cop), 1, "{out:?}");
    for name in ["a.rb", "Gemfile.lock", "a.gemspec"] {
        let out = cli(
            d.path(),
            &["--only", cop, "--stdin", name],
            Some(b"source :rubygems\n"),
        );
        assert_eq!(count(&out, cop), 0, "{name}: {out:?}");
    }
}
