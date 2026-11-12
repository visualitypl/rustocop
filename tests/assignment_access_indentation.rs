//! Source-only pinned lifecycle fixtures and actual CLI safety proofs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, select_active},
    offense::Severity,
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const ACCESS: &str = "Layout/AccessModifierIndentation";
const ASSIGN: &str = "Layout/AssignmentIndentation";
const BOTH: &str = "Layout/AccessModifierIndentation,Layout/AssignmentIndentation";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/assignment_access_indentation.json")).unwrap()
}
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn complete_pinned_custom_source_and_adversarial_original_byte_detection() {
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    let mut mismatches = Vec::new();
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let cop = row["cop"].as_str().unwrap();
        let active = select_active(&registry, &config, &[cop.into()], &[]);
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&bytes),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        let rejected = row.get("parser_rejection").is_some();
        if !report.errors.is_empty() != rejected {
            mismatches.push(format!("parser {row}: {report:?}"));
            continue;
        }
        let mut found: Vec<_> = report
            .result
            .offenses
            .into_iter()
            .map(|o| {
                assert_eq!(o.severity, Severity::Convention);
                assert!(o.correction.is_none());
                (o.line, o.col, o.end_line, o.message)
            })
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> = if rejected {
            vec![]
        } else {
            serde_json::from_value(row["findings"].clone()).unwrap()
        };
        if found != expected {
            mismatches.push(format!(
                "{}:{} {}\n{:?}\nproject {found:?}\nreference {expected:?}",
                row["origin"],
                row["line"],
                cop,
                String::from_utf8_lossy(&bytes)
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

fn assert_unchanged(path: &Path, before: &[u8], old: &fs::Metadata) {
    assert_eq!(fs::read(path).unwrap(), before);
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
}
fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(&format!("({cop})")))
        .count()
}
#[test]
fn every_pinned_source_control_and_encoding_uses_actual_read_only_cli() {
    let d = project();
    let path = d.path().join("example.rb");
    for row in rows() {
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        fs::write(&path, &bytes).unwrap();
        let old = fs::metadata(&path).unwrap();
        let cop = row["cop"].as_str().unwrap();
        let out = run(d.path(), &["--only", cop, "example.rb"]);
        let rejected = row.get("parser_rejection").is_some();
        let findings: Vec<(u32, u32, u32, String)> = if rejected {
            vec![]
        } else {
            serde_json::from_value(row["findings"].clone()).unwrap()
        };
        assert_eq!(
            out.status.code(),
            Some(if rejected {
                2
            } else {
                i32::from(!findings.is_empty())
            }),
            "{row}: {out:?}"
        );
        assert_eq!(count(&out, cop), findings.len(), "{row}: {out:?}");
        for (line, col, _, message) in findings {
            assert!(
                String::from_utf8_lossy(&out.stdout)
                    .contains(&format!("example.rb:{line}:{col}: C: {message} ({cop})")),
                "{row}: {out:?}"
            );
        }
        assert_unchanged(&path, &bytes, &old);
    }
}
#[test]
fn all_main_custom_options_and_source_corrections_replay_at_fixed_defaults() {
    let d = project();
    let path = d.path().join("example.rb");
    for row in rows()
        .into_iter()
        .filter(|r| matches!(r["origin"].as_str(), Some("spec" | "source")))
    {
        let before: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let after: Vec<u8> = serde_json::from_value(row["after_bytes"].clone()).unwrap();
        let cop = row["cop"].as_str().unwrap();
        fs::write(&path, &before).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
        assert_eq!(out.status.code(), Some(0), "{row}: {out:?}");
        assert_eq!(fs::read(&path).unwrap(), after, "{row}: {out:?}");
        assert_eq!(
            before.iter().filter(|b| **b == b'\n').count(),
            after.iter().filter(|b| **b == b'\n').count()
        );
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        let old = fs::metadata(&path).unwrap();
        let stable = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
        assert_eq!(stable.status.code(), Some(0), "{row}: {stable:?}");
        assert_unchanged(&path, &after, &old);
    }
}
#[test]
fn canonical_alignment_preserves_every_encoded_token_and_crlf_byte() {
    let d = project();
    let path = d.path().join("example.rb");
    for row in rows().into_iter().filter(|r| r["origin"] == "encoding") {
        let before: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let after: Vec<u8> = serde_json::from_value(row["project_after_bytes"].clone()).unwrap();
        fs::write(&path, &before).unwrap();
        let cop = row["cop"].as_str().unwrap();
        let out = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
        assert_eq!(out.status.code(), Some(0), "{row}: {out:?}");
        assert_eq!(fs::read(&path).unwrap(), after, "{row}: {out:?}");
    }
}
#[test]
fn adversarial_cli_corrections_are_canonical_or_guarded_no_write() {
    let d = project();
    let path = d.path().join("example.rb");
    let mut mismatches = Vec::new();
    let mut corrected = 0;
    let mut withheld = 0;
    for row in rows()
        .into_iter()
        .filter(|r| r["origin"] == "control" && r.get("parser_rejection").is_none())
    {
        let before: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let canonical: Vec<u8> = serde_json::from_value(row["after_bytes"].clone()).unwrap();
        let cop = row["cop"].as_str().unwrap();
        fs::write(&path, &before).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
        let actual = fs::read(&path).unwrap();
        assert_ne!(out.status.code(), Some(2), "{row}: {out:?}");
        if actual == before {
            assert_unchanged(&path, &before, &old);
            if before != canonical {
                withheld += 1;
                // Holds are either inline modifier code or negative RHS shifts
                // with an unshiftable clause: partial shifts can erase warnings.
                assert!(
                    before
                        .windows(b"(private; public)".len())
                        .any(|w| w == b"(private; public)")
                        || before
                            .windows(b"(private)".len())
                            .any(|w| w == b"(private)")
                        || before.starts_with(b"foo(\n  1) do; private")
                        || (cop == ASSIGN
                            && row["findings"].as_array().unwrap().iter().any(|o| {
                                let line = o[0].as_u64().unwrap() as usize;
                                before
                                    .split(|b| *b == b'\n')
                                    .nth(line - 1)
                                    .is_some_and(|s| s.starts_with(b"     "))
                            })),
                    "unaccounted hold: {row}"
                );
                assert_eq!(out.status.code(), Some(1), "{row}: {out:?}");
            }
        } else {
            corrected += 1;
            if actual != canonical {
                mismatches.push(format!(
                    "{row}\nactual {:?}\nreference {:?}",
                    String::from_utf8_lossy(&actual),
                    String::from_utf8_lossy(&canonical)
                ));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} noncanonical corrections:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    eprintln!(
        "adversarial correction count: {corrected} canonical rewrites; {withheld} guarded no-write controls"
    );
    assert_eq!((corrected, withheld), (910, 169));
}
#[test]
fn safe_navigation_assignment_parent_cli_anchors_and_no_write_controls() {
    let d = project();
    let path = d.path().join("example.rb");
    for (lhs, expected) in [
        ("x&.y = a =", 2),
        ("  x&.y = a =", 4),
        ("x&.y = @a =", 2),
        ("x&.y = a +=", 2),
        ("x&.y = a = b =", 9),
        ("x&.y =\na =", 2),
        ("x&.y(a =", 7),
        ("x.y = a =", 2),
        ("x&.y ||= a =", 2),
    ] {
        let closing = if lhs == "x&.y(a =" { ")" } else { "" };
        for crlf in [false, true] {
            let encode = |source: String| {
                if crlf {
                    format!("# coding: Windows-1252\n# é\n{source}")
                        .replace('\n', "\r\n")
                        .chars()
                        .map(|c| c as u8)
                        .collect::<Vec<_>>()
                } else {
                    source.into_bytes()
                }
            };
            let after = encode(format!("{lhs}\n{}1{closing}\n", " ".repeat(expected)));
            for indent in [0, expected - 1, expected, expected + 1, expected + 7] {
                let before = encode(format!("{lhs}\n{}1{closing}\n", " ".repeat(indent)));
                fs::write(&path, &before).unwrap();
                let old = fs::metadata(&path).unwrap();
                let lint = run(d.path(), &["--only", ASSIGN, "example.rb"]);
                assert_eq!(
                    lint.status.code(),
                    Some(i32::from(indent != expected)),
                    "{before:?}: {lint:?}"
                );
                assert_eq!(
                    count(&lint, ASSIGN),
                    usize::from(indent != expected),
                    "{lint:?}"
                );
                if indent != expected {
                    let line = lhs.lines().count() + 1 + if crlf { 2 } else { 0 };
                    assert!(String::from_utf8_lossy(&lint.stdout).contains(&format!(
                        "example.rb:{line}:{}: C: Indent the first line of the right-hand-side of a multi-line assignment. ({ASSIGN})", indent + 1)), "{lint:?}");
                }
                assert_unchanged(&path, &before, &old);
                let fix = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
                assert_eq!(fix.status.code(), Some(0), "{before:?}: {fix:?}");
                assert_eq!(fs::read(&path).unwrap(), after, "{before:?}: {fix:?}");
                if indent == expected {
                    assert_unchanged(&path, &before, &old);
                }
                let old = fs::metadata(&path).unwrap();
                let stable = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
                assert_eq!(stable.status.code(), Some(0), "{stable:?}");
                assert_unchanged(&path, &after, &old);
            }
        }
    }
    for source in [
        "x&.y =\n1\n".to_string(),
        "x&.y =\n         1\n".to_string(),
        format!("# rubocop:disable {ASSIGN}\nx&.y = a =\n         1\n"),
        format!("# rustocop:disable {ASSIGN}\nx&.y = a =\n         1\n"),
    ] {
        fs::write(&path, &source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let fix = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
        assert_eq!(fix.status.code(), Some(0), "{fix:?}");
        assert_eq!(count(&fix, ASSIGN), 0, "{fix:?}");
        assert_unchanged(&path, source.as_bytes(), &old);
    }
}
#[test]
fn actual_callback_owners_lint_fix_and_nested_canonical_no_write_controls() {
    let d = project();
    let path = d.path().join("example.rb");
    for row in rows().into_iter().filter(|r| r["origin"] == "callback") {
        let before: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let after: Vec<u8> = serde_json::from_value(row["after_bytes"].clone()).unwrap();
        let findings: Vec<(u32, u32, u32, String)> =
            serde_json::from_value(row["findings"].clone()).unwrap();
        fs::write(&path, &before).unwrap();
        let old = fs::metadata(&path).unwrap();
        let lint = run(d.path(), &["--only", ACCESS, "example.rb"]);
        assert_eq!(
            lint.status.code(),
            Some(i32::from(!findings.is_empty())),
            "{row}: {lint:?}"
        );
        assert_eq!(count(&lint, ACCESS), findings.len(), "{row}: {lint:?}");
        for (line, col, _, message) in findings {
            assert!(
                String::from_utf8_lossy(&lint.stdout)
                    .contains(&format!("example.rb:{line}:{col}: C: {message} ({ACCESS})")),
                "{row}: {lint:?}"
            );
        }
        assert_unchanged(&path, &before, &old);
        let fix = run(d.path(), &["--fix", "--only", ACCESS, "example.rb"]);
        assert_eq!(fix.status.code(), Some(0), "{row}: {fix:?}");
        assert_eq!(fs::read(&path).unwrap(), after, "{row}: {fix:?}");
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        if before == after {
            assert_unchanged(&path, &before, &old);
        }
        let old = fs::metadata(&path).unwrap();
        let stable = run(d.path(), &["--fix", "--only", ACCESS, "example.rb"]);
        assert_eq!(stable.status.code(), Some(0), "{row}: {stable:?}");
        assert_unchanged(&path, &after, &old);
    }
}
#[test]
fn implicit_callback_ownership_survives_directives_and_original_encoding() {
    let d = project();
    let path = d.path().join("example.rb");
    for prefix in ["rubocop", "rustocop"] {
        for tail in ["_1", "it", "1"] {
            for disabled in [false, true] {
                let directive = if disabled {
                    format!("# {prefix}:disable {ACCESS}\r\n")
                } else {
                    String::new()
                };
                let source = format!(
                    "# coding: Windows-1252\r\n{directive}foo do\r\nprivate\r\n  bar do\r\nprotected\r\n    '\u{e9}'\r\n  end\r\n  {tail}\r\nend\r\n"
                );
                let before: Vec<u8> = source.chars().map(|c| c as u8).collect();
                let expected = if disabled {
                    before.clone()
                } else {
                    let after = source.replace("\r\nprotected", "\r\n    protected");
                    let after = if tail == "1" {
                        after.replace("\r\nprivate", "\r\n  private")
                    } else {
                        after
                    };
                    after.chars().map(|c| c as u8).collect()
                };
                fs::write(&path, &before).unwrap();
                let old = fs::metadata(&path).unwrap();
                let lint = run(d.path(), &["--only", ACCESS, "example.rb"]);
                assert_eq!(
                    count(&lint, ACCESS),
                    if disabled {
                        0
                    } else if tail == "1" {
                        2
                    } else {
                        1
                    },
                    "{lint:?}"
                );
                assert_eq!(lint.status.code(), Some(i32::from(!disabled)), "{lint:?}");
                assert_unchanged(&path, &before, &old);
                let fix = run(d.path(), &["--fix", "--only", ACCESS, "example.rb"]);
                assert_eq!(fix.status.code(), Some(0), "{fix:?}");
                assert_eq!(fs::read(&path).unwrap(), expected, "{fix:?}");
                if disabled {
                    assert_unchanged(&path, &before, &old);
                }
            }
        }
    }
}
#[test]
fn actual_metadata_default_config_only_except_and_strict_no_new_knobs() {
    let d = project();
    let path = d.path().join("example.rb");
    let registry = Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for (cop, source) in [
        (ACCESS, "class A\nprivate\n  def foo; end\nend\n"),
        (ASSIGN, "a =\n1\n"),
    ] {
        let c = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(c.default_enabled() && c.safe_fixes());
        assert!(c.path_policy().is_none());
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(c.max_setting().is_none());
        for (config, flags, expected) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", "Layout"], 1),
            (String::new(), vec!["--only", cop, "--except", "Layout"], 0),
            (format!("[cops.\"{cop}\"]\nenabled=false\n"), vec![], 0),
            ("[cops.Layout]\nenabled=false\n".into(), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled=false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.Layout]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
                vec![],
                1,
            ),
            ("exclude=['example.rb']\n".into(), vec!["--only", cop], 0),
            (
                format!("[cops.\"{cop}\"]\nexclude=['example.rb']\n"),
                vec!["--only", cop],
                0,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, source).unwrap();
            let old = fs::metadata(&path).unwrap();
            let mut flags = flags;
            flags.push("example.rb");
            let out = run(d.path(), &flags);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_eq!(count(&out, cop), expected, "{out:?}");
            assert_unchanged(&path, source.as_bytes(), &old);
        }
        for option in [
            "max=4",
            "IndentationWidth=4",
            "EnforcedStyle='outdent'",
            "Width=4",
            "include=['**/*']",
            "enabled='true'",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{option}\n"),
            )
            .unwrap();
            fs::write(&path, source).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
            assert_eq!(out.status.code(), Some(2), "{out:?}");
            assert_unchanged(&path, source.as_bytes(), &old);
        }
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for name in ["Layout/Future", "Layout/Unknown"] {
        assert!(!registry.has_cop(name));
        let out = run(d.path(), &["--fix", "--only", name, "example.rb"]);
        assert_eq!(out.status.code(), Some(2));
    }
}
#[test]
fn both_directive_prefixes_and_whole_rhs_disable_spans_retain_bytes() {
    let d = project();
    let path = d.path().join("example.rb");
    for prefix in ["rubocop", "rustocop"] {
        for (cop, source) in [
            (ACCESS, "class A\nprivate\n  def foo; end\nend\n"),
            (ASSIGN, "a =\nif b\n  foo\nend\n"),
        ] {
            for body in [
                format!("# {prefix}:disable {cop}\n{source}"),
                format!("# {prefix}:push -Layout\n{source}# {prefix}:pop\n"),
                source.replacen(
                    if cop == ACCESS { "private" } else { "if b" },
                    &format!(
                        "{} # {prefix}:todo {cop}",
                        if cop == ACCESS { "private" } else { "if b" }
                    ),
                    1,
                ),
            ] {
                fs::write(&path, &body).unwrap();
                let old = fs::metadata(&path).unwrap();
                let out = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
                assert_eq!(out.status.code(), Some(0), "{out:?}");
                assert_unchanged(&path, body.as_bytes(), &old);
            }
        }
        let source = format!("a =\nbegin\n  foo # {prefix}:disable {ASSIGN}\nend\n");
        fs::write(&path, &source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
        // The diagnostic owns the complete RHS, so a disabled interior line
        // suppresses the whole finding as well as its containing correction.
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_unchanged(&path, source.as_bytes(), &old);
    }
}
#[test]
fn uncertain_source_literal_comment_observer_and_error_paths_never_write() {
    let d = project();
    let path = d.path().join("example.rb");
    for (cop, source) in [
        (ACCESS, "class A\nprivate\n  def foo; end\nend\n"),
        (ASSIGN, "a =\n1\n"),
    ] {
        for observer in [
            "p File.binread(__FILE__)\n",
            "p ObjectSpace.count_objects\n",
            "p RubyVM::AbstractSyntaxTree.of(-> {}).first_column\n",
            "p GC.stat\n",
            "p IO.read(__FILE__)\n",
            "p caller\n",
        ] {
            let source = format!("{source}{observer}");
            fs::write(&path, &source).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
            assert_eq!(out.status.code(), Some(1), "{out:?}");
            assert_eq!(count(&out, cop), 1);
            assert_unchanged(&path, source.as_bytes(), &old);
        }
    }
    for (cop, source) in [
        (ACCESS, "class A\n(private; public)\nend\n"),
        (ASSIGN, "a =\nbegin\n=begin\ncomment\n=end\n  1\nend\n"),
        (ASSIGN, "a = %W(#{\nb =\n1\n})\n"),
    ] {
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_unchanged(&path, source.as_bytes(), &old);
    }
    for source in [
        b"a =\n1\ndef broken(\n".as_slice(),
        b"# coding: UTF-8\na =\n'\xff'\n",
        b"# coding: NotAnEncoding\na =\n1\n",
    ] {
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--fix", "--only", BOTH, "example.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_unchanged(&path, source, &old);
    }
    // A replacement spanning non-UTF8 original token bytes cannot be encoded
    // in Correction's String payload. Preserve it, rather than transcode it.
    let encoded = b"# coding: Windows-1252\na =\n'\xe9'\n";
    fs::write(&path, encoded).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out, ASSIGN), 1);
    assert_unchanged(&path, encoded, &old);
    let source = "a =\n".repeat(80) + "1\n";
    fs::write(&path, &source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(d.path(), &["--fix", "--only", BOTH, "example.rb"]);
    assert_eq!(out.status.code(), Some(2));
    assert_unchanged(&path, source.as_bytes(), &old);
    assert_eq!(
        run(d.path(), &["--fix", "--only", BOTH, "missing.rb"])
            .status
            .code(),
        Some(2)
    );
}
fn assert_reader_corrects(
    root: &Path,
    cop: &str,
    source: &str,
    line: usize,
    prefix: &str,
    alias: bool,
) {
    let path = root.join("example.rb");
    let shift = if cop == ACCESS { 2 - prefix.len() } else { 2 };
    let expected = source
        .split_inclusive('\n')
        .enumerate()
        .map(|(i, text)| {
            if i + 1 == line {
                format!("{}{}{}", prefix, " ".repeat(shift), &text[prefix.len()..])
            } else {
                text.to_owned()
            }
        })
        .collect::<String>();
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let lint = run(root, &["--only", cop, "example.rb"]);
    assert_eq!(lint.status.code(), Some(1), "{source}: {lint:?}");
    assert_eq!(count(&lint, cop), 1);
    assert!(
        String::from_utf8_lossy(&lint.stdout)
            .contains(&format!(":{line}:{}: C:", prefix.len() + 1)),
        "{lint:?}"
    );
    assert_unchanged(&path, source.as_bytes(), &old);
    let args = if alias {
        vec!["--fix", "--only", cop, "alias.rb", "example.rb"]
    } else {
        vec!["--fix", "--only", cop, "example.rb"]
    };
    let out = run(root, &args);
    assert_eq!(out.status.code(), Some(0), "{source}: {out:?}");
    assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
    assert_eq!(
        old.permissions(),
        fs::metadata(&path).unwrap().permissions()
    );
    if alias {
        assert!(
            fs::symlink_metadata(root.join("alias.rb"))
                .unwrap()
                .is_symlink()
        );
    }
    let old = fs::metadata(&path).unwrap();
    assert_eq!(run(root, &args).status.code(), Some(0));
    assert_unchanged(&path, expected.as_bytes(), &old);
}
#[test]
#[cfg(unix)]
fn all_shared_alignment_consumers_correct_possible_path_readers() {
    use std::os::unix::fs::symlink;
    let d = project();
    let path = d.path().join("example.rb");
    symlink("example.rb", d.path().join("alias.rb")).unwrap();
    let absolute = format!("{:?}", path.to_str().unwrap());
    for prefix in ["\u{b}", ""] {
        for (cop, stem, line) in [
            (ASSIGN, format!("value =\n{prefix}1\np value\n"), 2),
            (
                ACCESS,
                format!("class C\n{prefix}private\n  def value; 1; end\nend\n"),
                2,
            ),
            (
                "Layout/ArgumentAlignment",
                format!("def f(*xs); p xs; end\nf(1,\n{prefix}2)\n"),
                3,
            ),
        ] {
            for observer in [
                format!("p File.binread({absolute})"),
                "p File.read('example.rb')".into(),
                "p IO.binread('alias.rb')".into(),
                "p File.binread(ARGV[0])".into(),
                format!("F = File; path = {absolute}; p F.read(path)"),
                format!("p File.method(:binread).call({absolute})"),
                format!("p File.open({absolute}, &:read)"),
                "File.foreach('example.rb') { |line| print line }".into(),
            ] {
                let source = format!("{stem}{observer}\n");
                assert_reader_corrects(d.path(), cop, &source, line, prefix, true);
            }
        }
    }
}
// Only closed authored programs run here, never the reference fixtures. Stdin
// is the actual owned source, not a copied string or an unrelated input pipe.
fn native_stdin(ruby: &str, path: &Path) -> Output {
    use std::io::{Read, Seek};
    let mut stdout = tempfile::tempfile().unwrap();
    let mut stderr = tempfile::tempfile().unwrap();
    let status = Command::new("timeout")
        .args([
            "--kill-after=1s",
            "15",
            "prlimit",
            "--as=536870912",
            "--cpu=15",
            "--fsize=1048576",
            "--core=0",
            "--",
        ])
        .arg(ruby)
        .args(["--parser=prism", "-w"])
        .arg(path)
        .current_dir(path.parent().unwrap())
        .stdin(fs::File::open(path).unwrap())
        .stdout(stdout.try_clone().unwrap())
        .stderr(stderr.try_clone().unwrap())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0), "native timeout/signal/failure");
    assert!(stdout.metadata().unwrap().len() < 1048576);
    assert!(stderr.metadata().unwrap().len() < 1048576);
    stdout.rewind().unwrap();
    stderr.rewind().unwrap();
    let mut out = Output {
        status,
        stdout: vec![],
        stderr: vec![],
    };
    stdout.read_to_end(&mut out.stdout).unwrap();
    stderr.read_to_end(&mut out.stderr).unwrap();
    out
}
#[test]
fn shared_alignment_corrects_implicit_external_stream_consumers() {
    let d = project();
    for prefix in ["", "\u{b}"] {
        for (cop, stem) in [
            (ASSIGN, format!("value =\n{prefix}1\np value\n")),
            (
                ACCESS,
                format!("class C\n{prefix}private\n  def value; 1; end\nend\n"),
            ),
            (
                "Layout/ArgumentAlignment",
                format!("def f(*xs); end\nf(1,\n{prefix}2)\n"),
            ),
        ] {
            for observer in [
                "STDIN.each { |line| print line }",
                "STDIN.each_codepoint { |cp| print cp }",
                "print STDIN.to_a.join",
                "input = $stdin; input.each { |line| print line }",
                "STDIN.public_send(:each) { |line| print line }",
                "STDIN.method(:to_a).call",
                "[STDIN].map(&:each_codepoint)",
                "enum = STDIN.lazy; print enum.next",
                "print STDIN.getbyte",
                "print STDIN.getc",
                "print STDIN.entries.join",
                "STDIN.reverse_each { |line| print line }",
                "print STDIN.count",
                "STDIN.each_with_index { |line, _| print line }",
                "print STDIN.take(1).join",
            ] {
                let source = format!("{stem}{observer}\n");
                assert_reader_corrects(
                    d.path(),
                    cop,
                    &source,
                    if cop == "Layout/ArgumentAlignment" {
                        3
                    } else {
                        2
                    },
                    prefix,
                    false,
                );
            }
        }
    }
}
// Historical holds are now real CLI edits. Reader programs are parsed,
// never executed: their own-source runtime output is deliberately out of scope.
fn out_of_scope_readers_correct(observers: &[&str]) {
    let d = project();
    for prefix in ["", "\u{b}"] {
        for (cop, stem, line) in [
            (ASSIGN, format!("value =\n{prefix}1\n"), 2),
            (
                ACCESS,
                format!("class C\n{prefix}private\n  def m; nil; end\nend\n"),
                2,
            ),
            (
                "Layout/ArgumentAlignment",
                format!("def f(*v); v; end\nf(1,\n{prefix}2)\n"),
                3,
            ),
        ] {
            for observer in observers {
                let source = format!("{stem}{observer}\n");
                // A fixed-width integer keeps this independently authored EOF
                // witness's exact length, without reading its path at runtime.
                let source = source.replace("SOURCE_END", &format!("{:10}", source.len()));
                assert_reader_corrects(d.path(), cop, &source, line, prefix, false);
            }
        }
    }
}
#[test]
fn historical_implicit_language_and_core_conversion_readers_correct() {
    out_of_scope_readers_correct(&[
        "for line in STDIN\n  print line\nend",
        "print [*STDIN].join",
        "print Array(STDIN).join",
    ]);
}
#[test]
fn historical_opaque_process_and_command_readers_correct() {
    out_of_scope_readers_correct(&[
        "system('/usr/bin/cat', 'example.rb')",
        "print `/usr/bin/cat example.rb`",
    ]);
}
#[test]
fn external_stream_position_and_eof_observers_allow_shared_alignment_edits() {
    out_of_scope_readers_correct(&[
        "STDIN.seek(0, 2); p STDIN.tell",
        "STDIN.seek(0, 2); p STDIN.pos",
        "input = $stdin; input.seek(0, 2); p input.tell",
        "p STDIN.sysseek(0, 2)",
        "STDIN.public_send(:seek, 0, 2); p STDIN.public_send(:tell)",
        "STDIN.method(:seek).call(0, 2); p STDIN.method(:pos).call",
        "class IO; alias end_position sysseek; end; p STDIN.end_position(0, 2)",
        "STDIN.pos = 0; p STDIN.eof?",
        "STDIN.rewind; p STDIN.eof",
        "STDIN.seek(SOURCE_END); p STDIN.eof?",
        "STDIN.pos = SOURCE_END; p STDIN.eof",
    ]);
}
#[test]
fn literal_splats_remain_real_shared_cli_edits_with_vm_conversion_runtime_proofs() {
    let d = project();
    let path = d.path().join("example.rb");
    let ruby = std::env::var("RUSTOCOP_RUBY34").ok();
    for prefix in ["", "\u{b}"] {
        let access_shift = " ".repeat(2 - prefix.len());
        for (cop, stem, after) in [
            (
                ASSIGN,
                format!("value =\n{prefix}1\n"),
                format!("value =\n{prefix}  1\n"),
            ),
            (
                ACCESS,
                format!("class C\n{prefix}private\n  def m; nil; end\nend\n"),
                format!("class C\n{prefix}{access_shift}private\n  def m; nil; end\nend\n"),
            ),
            (
                "Layout/ArgumentAlignment",
                format!("def f(*v); v; end\nf(1,\n{prefix}*[2, 3])\n"),
                format!("def f(*v); v; end\nf(1,\n{prefix}  *[2, 3])\n"),
            ),
        ] {
            // VM direct-container paths, not receiver/type/value-flow claims.
            // Children still participate in the complete observer walk.
            let tail = "def g(*v, **kw); p [v, kw]; end\ng(*([1, 2]), **{k: 3})\n";
            let source = stem + tail;
            let expected = after + tail;
            fs::write(&path, &source).unwrap();
            let before = ruby.as_ref().map(|r| native_stdin(r, &path));
            let out = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
            assert_eq!(out.status.code(), Some(0), "{source:?}: {out:?}");
            assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
            if let Some(before) = before {
                let after = native_stdin(ruby.as_ref().unwrap(), &path);
                assert_eq!(before.stdout, after.stdout);
                assert_eq!(before.stderr, after.stderr);
            }
            let old = fs::metadata(&path).unwrap();
            assert_eq!(
                run(d.path(), &["--fix", "--only", cop, "example.rb"])
                    .status
                    .code(),
                Some(0)
            );
            assert_unchanged(&path, expected.as_bytes(), &old);
        }
    }
    if let Some(ruby) = ruby {
        // Independently authored finite VM semantics witness, not a reference
        // example. #to_a drives splats; #to_ary drives Kernel.Array first and
        // ordinary destructuring; for dispatches #each. Exact containers bypass
        // the conversion methods even when those methods are redefined.
        let source = "$trace=[]\nclass C\n  def to_a; $trace << :to_a; [1]; end\n  def to_ary; $trace << :to_ary; [2]; end\n  def each; $trace << :each; yield 3; end\nend\nx=C.allocate\np [*x]\np Array(x)\na,b=x\np [a,b]\nfor n in x; p n; end\nclass Array\n  def to_a; raise 'not a splat conversion'; end\nend\nclass Hash\n  def to_hash; raise 'not a keyword conversion'; end\nend\ndef g(*v, **kw); p [v,kw]; end\ng(*[4], **{k: 5})\np $trace\n";
        fs::write(&path, source).unwrap();
        let out = native_stdin(&ruby, &path);
        assert!(out.status.success(), "{out:?}");
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "[1]\n[2]\n[2, nil]\n3\n[[4], {k: 5}]\n[:to_a, :to_ary, :to_ary, :each]\n"
        );
    }
}
#[test]
fn shared_reader_guard_preserves_real_prefix_and_whole_subtree_corrections() {
    let d = project();
    let path = d.path().join("example.rb");
    for prefix in ["", "\u{b}", "\t", "\t\u{b}"] {
        let shift = " ".repeat(2 - usize::from(prefix.contains('\t')));
        let access_shift = " ".repeat(2 - prefix.len());
        for (cop, source, after) in [
            (
                ASSIGN,
                format!("value =\n{prefix}begin\n{prefix}  1\n{prefix}end\np value\n"),
                format!(
                    "value =\n{prefix}{shift}begin\n{prefix}{shift}  1\n{prefix}{shift}end\np value\n"
                ),
            ),
            (
                "Layout/ArgumentAlignment",
                format!("def f(*xs); p xs; end\nf(1,\n{prefix}begin\n{prefix}  2\n{prefix}end)\n"),
                format!(
                    "def f(*xs); p xs; end\nf(1,\n{prefix}{shift}begin\n{prefix}{shift}  2\n{prefix}{shift}end)\n"
                ),
            ),
            (
                ACCESS,
                format!("class C\n{prefix}private\n  def value; 1; end\nend\n"),
                format!("class C\n{prefix}{access_shift}private\n  def value; 1; end\nend\n"),
            ),
        ] {
            fs::write(&path, &source).unwrap();
            let old = fs::metadata(&path).unwrap();
            let lint = run(d.path(), &["--only", cop, "example.rb"]);
            assert_eq!(
                lint.status.code(),
                Some(i32::from(source != after)),
                "{source:?}: {lint:?}"
            );
            assert_unchanged(&path, source.as_bytes(), &old);
            let fix = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
            assert_eq!(fix.status.code(), Some(0), "{source:?}: {fix:?}");
            assert_eq!(fs::read(&path).unwrap(), after.as_bytes(), "{source:?}");
            assert_eq!(
                old.permissions(),
                fs::metadata(&path).unwrap().permissions()
            );
            if source == after {
                assert_unchanged(&path, source.as_bytes(), &old);
            }
            let old = fs::metadata(&path).unwrap();
            assert_eq!(
                run(d.path(), &["--fix", "--only", cop, "example.rb"])
                    .status
                    .code(),
                Some(0)
            );
            assert_unchanged(&path, after.as_bytes(), &old);
            if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
                // Only these separately authored closed programs execute.
                let before = native::run(&ruby, &source);
                let after = native::run(&ruby, &after);
                assert!(
                    before.status.success() && after.status.success(),
                    "{before:?} {after:?}"
                );
                assert_eq!(before.status.code(), after.status.code());
                assert_eq!(before.stdout, after.stdout);
                assert_eq!(before.stderr, after.stderr, "{source:?}");
            }
        }
    }
}
#[test]
fn supplied_paths_stdin_bom_crlf_legacy_encoding_and_data_ranges() {
    use std::{io::Write, process::Stdio};
    let d = project();
    let path = d.path().join("input.arb");
    for (source, after) in [
        (
            b"\xef\xbb\xbf# comment\r\na =\r\n1\r\n__END__\r\nprivate\xff\0\r\n".as_slice(),
            b"\xef\xbb\xbf# comment\r\na =\r\n  1\r\n__END__\r\nprivate\xff\0\r\n".as_slice(),
        ),
        (
            b"# coding: Windows-1252\r\nmodule A\r\nprivate\r\n  '\xe9'\r\nend\r\n",
            b"# coding: Windows-1252\r\nmodule A\r\n  private\r\n  '\xe9'\r\nend\r\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let out = run(d.path(), &["--fix", "--only", BOTH, "input.arb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), after);
        fs::write(&path, b"sentinel\n").unwrap();
        let old = fs::metadata(&path).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", BOTH, "--stdin", "input.arb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(source).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_unchanged(&path, b"sentinel\n", &old);
    }
    let old = fs::metadata(&path).unwrap();
    let out = run(d.path(), &["--fix", "--only", BOTH, "--stdin", "input.arb"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert_unchanged(&path, b"sentinel\n", &old);
}
#[test]
#[cfg(unix)]
fn read_only_atomic_alias_failure_retains_bytes_identity_and_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("example.rb");
    let source = b"a =\n1\n";
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("example.rb", d.path().join("alias.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(
        d.path(),
        &["--fix", "--only", BOTH, "alias.rb", "example.rb"],
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_unchanged(&path, source, &old);
    assert!(
        fs::symlink_metadata(d.path().join("alias.rb"))
            .unwrap()
            .is_symlink()
    );
}
#[test]
fn assignment_access_existing_alignment_and_style_peers_converge_without_cycles() {
    let d = project();
    let path = d.path().join("example.rb");
    let source = "module A\nprivate\n  def foo\n    value =\n    if true\n      'yes'\n    end\n    value\n  end\nend\n";
    fs::write(&path, source).unwrap();
    let flags = [
        "--fix",
        "--only",
        "Layout,Style/StringLiterals",
        "example.rb",
    ];
    let mut snapshots = vec![source.as_bytes().to_vec()];
    let mut clean = false;
    for _ in 0..8 {
        let out = run(d.path(), &flags);
        assert!(out.stderr.is_empty(), "{out:?}");
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        let bytes = fs::read(&path).unwrap();
        if out.status.code() == Some(0) {
            clean = true;
            break;
        }
        assert!(!snapshots.contains(&bytes), "cycle or no progress: {out:?}");
        snapshots.push(bytes);
    }
    assert!(clean);
    let bytes = fs::read(&path).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("  private\n"));
    assert!(String::from_utf8_lossy(&bytes).contains("\"yes\""));
    let old = fs::metadata(&path).unwrap();
    assert_eq!(run(d.path(), &flags).status.code(), Some(0));
    assert_unchanged(&path, &bytes, &old);
}
#[test]
fn native_ruby34_parses_all_reference_bytes_without_executing_them() {
    use std::{io::Write, process::Stdio};
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        let mut sides = vec![&row["bytes"]];
        if row.get("parser_rejection").is_none() {
            sides.push(
                row.get("project_after_bytes")
                    .unwrap_or(&row["after_bytes"]),
            );
        }
        for side in sides {
            let bytes: Vec<u8> = serde_json::from_value(side.clone()).unwrap();
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-W0", "-c"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                row.get("parser_rejection").is_none(),
                "{row}: {out:?}"
            );
        }
    }
}
#[path = "support/deprecated_constructor_native.rs"]
mod native;
#[test]
fn partial_negative_shift_cannot_erase_native_warnings_or_write_source() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = project();
    let path = d.path().join("example.rb");
    for keyword in ["if true", "begin"] {
        let source = format!("a =\n          {keyword}\n  1\n  end\np a\n");
        let before = native::run(&ruby, &source);
        assert!(before.status.success(), "{before:?}");
        assert!(
            String::from_utf8_lossy(&before.stderr).contains("mismatched indentations"),
            "{before:?}"
        );
        fs::write(&path, &source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out, ASSIGN), 1);
        assert_unchanged(&path, source.as_bytes(), &old);
        let after = native::run(&ruby, &fs::read_to_string(&path).unwrap());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        assert_eq!(before.status.code(), after.status.code());
    }
}
#[test]
fn emoji_width_cli_exact_corrections_and_no_offense_identity() {
    let d = project();
    let path = d.path().join("example.rb");
    for (word, width) in [
        ("❤️", 1),
        ("❤︎", 1),
        ("👨‍👩‍👧‍👦", 8),
        ("👩🏽‍💻", 6),
        ("🇵🇱", 2),
        ("1️⃣", 1),
        ("e\u{301}", 1),
        ("\u{2e3b}", 3),
        ("\u{9be}", 1),
        ("a\u{8}", 0),
    ] {
        let expected = width + 8;
        let after = format!("f '{word}', a =\n{}b\n", " ".repeat(expected));
        for indent in [0, expected - 1, expected, expected + 1] {
            let before = format!("f '{word}', a =\n{}b\n", " ".repeat(indent));
            fs::write(&path, &before).unwrap();
            let old = fs::metadata(&path).unwrap();
            let lint = run(d.path(), &["--only", ASSIGN, "example.rb"]);
            assert_eq!(
                lint.status.code(),
                Some(i32::from(indent != expected)),
                "{before:?}: {lint:?}"
            );
            assert_eq!(count(&lint, ASSIGN), usize::from(indent != expected));
            if indent != expected {
                assert!(
                    String::from_utf8_lossy(&lint.stdout).contains(&format!(
                        "example.rb:2:{}: C: Indent the first line",
                        indent + 1
                    )),
                    "{lint:?}"
                );
            }
            assert_unchanged(&path, before.as_bytes(), &old);
            let fix = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
            assert_eq!(fix.status.code(), Some(0), "{before:?}: {fix:?}");
            assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
            if indent == expected {
                assert_unchanged(&path, before.as_bytes(), &old);
            }
            let old = fs::metadata(&path).unwrap();
            let stable = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
            assert_eq!(stable.status.code(), Some(0), "{stable:?}");
            assert_unchanged(&path, after.as_bytes(), &old);
        }
    }
}
#[test]
fn mixed_tab_negative_shifts_preserve_native_warnings_and_file_identity() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = project();
    let path = d.path().join("example.rb");
    for (opening, body, closing, warns) in [
        ("  \t", "  \t  ", "  \t", false),
        (" \t ", " \t   ", " \t ", false),
        ("    ", "\t", "    ", false),
        ("    ", "      ", "\t", true),
    ] {
        for keyword in ["if true", "begin"] {
            let source = format!("a =\n{opening}{keyword}\n{body}1\n{closing}end\np a\n");
            let before = native::run(&ruby, &source);
            assert!(before.status.success(), "{before:?}");
            assert_eq!(
                String::from_utf8_lossy(&before.stderr).contains("mismatched indentations"),
                warns,
                "{source:?}: {before:?}"
            );
            fs::write(&path, &source).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
            assert_eq!(out.status.code(), Some(1), "{out:?}");
            assert_eq!(count(&out, ASSIGN), 1);
            assert_unchanged(&path, source.as_bytes(), &old);
            let after = native::run(&ruby, &fs::read_to_string(&path).unwrap());
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
            assert_eq!(before.status.code(), after.status.code());
        }
    }
}
#[test]
fn bounded_authored_implicit_and_nested_classic_callbacks_preserve_runtime() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let source = "$trace=[]\nclass C\n  def self.private; $trace << :private; end\n  def self.protected; $trace << :protected; end\n  def self.foo; yield 7; end\n  foo do\nprivate\n    $trace << _1\n    foo do\nprotected\n      $trace << :inner\n    end\n  end\n  -> do\nprivate\n    $trace << it\n  end.call(9)\nend\np [$trace, __LINE__]\n";
    let before = native::run(&ruby, source);
    assert!(before.status.success(), "{before:?}");
    let d = project();
    let path = d.path().join("example.rb");
    fs::write(&path, source).unwrap();
    let lint = run(d.path(), &["--only", ACCESS, "example.rb"]);
    assert_eq!(lint.status.code(), Some(1), "{lint:?}");
    assert_eq!(count(&lint, ACCESS), 1, "{lint:?}");
    let fix = run(d.path(), &["--fix", "--only", ACCESS, "example.rb"]);
    assert_eq!(fix.status.code(), Some(0), "{fix:?}");
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(after, source.replace("\nprotected", "\n      protected"));
    let actual = native::run(&ruby, &after);
    assert_eq!(actual.status.code(), before.status.code());
    assert_eq!(actual.stdout, before.stdout);
    assert_eq!(actual.stderr, before.stderr);
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(d.path(), &["--fix", "--only", ACCESS, "example.rb"])
            .status
            .code(),
        Some(0)
    );
    assert_unchanged(&path, after.as_bytes(), &old);
}
// Keep the original #new/Enumerable witnesses as actual canonical edits too.
fn assert_constructor_corrects(ruby: &str, source: &str, cop: &str) {
    let original = source.replace("C.allocate", "C.new");
    assert_possible_reader_corrects(ruby, &original, cop);
}
fn assert_possible_reader_corrects(ruby: &str, original: &str, cop: &str) {
    let d = project();
    let path = d.path().join("example.rb");
    fs::write(&path, original).unwrap();
    let old = fs::metadata(&path).unwrap();
    let before = native::run(ruby, original);
    assert!(before.status.success(), "{before:?}");
    let out = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_ne!(fs::read(&path).unwrap(), original.as_bytes());
    assert_eq!(
        old.permissions(),
        fs::metadata(&path).unwrap().permissions()
    );
    let after = native::run(ruby, &fs::read_to_string(path).unwrap());
    assert_eq!(before.status.code(), after.status.code());
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn bounded_authored_safe_setter_parent_preserves_nil_short_circuit_and_dispatch() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    // Constructor and Enumerable dispatch do not imply a listed observer.
    let source = "$trace=[]\nclass C\n  def y=(value); $trace << [:set, value, __LINE__]; end\nend\n[nil, C.allocate].each do |x|\n  a = 0\n  x&.y = a =\n         begin\n           $trace << :rhs\n           [a, __LINE__]\n         end\n  p [a, $trace, __LINE__]\nend\n";
    assert_constructor_corrects(&ruby, source, ASSIGN);
    assert_possible_reader_corrects(&ruby, source, ASSIGN);
    let source = source.replace(
        "[nil, C.allocate].each do |x|",
        "i = 0\nwhile i < 2\n  x = (i == 0 ? nil : C.allocate)\n  i += 1",
    );
    let source = source.as_str();
    let before = native::run(&ruby, source);
    assert!(before.status.success(), "{before:?}");
    let d = project();
    let path = d.path().join("example.rb");
    fs::write(&path, source).unwrap();
    let lint = run(d.path(), &["--only", ASSIGN, "example.rb"]);
    assert_eq!(lint.status.code(), Some(1), "{lint:?}");
    assert_eq!(count(&lint, ASSIGN), 1, "{lint:?}");
    let fix = run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"]);
    assert_eq!(fix.status.code(), Some(0), "{fix:?}");
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(after, source.replace("\n         ", "\n    "));
    let actual = native::run(&ruby, &after);
    assert_eq!(actual.status.code(), before.status.code());
    assert_eq!(actual.stdout, before.stdout);
    assert_eq!(actual.stderr, before.stderr);
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(d.path(), &["--fix", "--only", ASSIGN, "example.rb"])
            .status
            .code(),
        Some(0)
    );
    assert_unchanged(&path, after.as_bytes(), &old);
}
#[test]
fn bounded_authored_native_runtime_preserves_dispatch_values_lines_control_and_warnings() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    // Both construction and allocation allow these intended whitespace edits.
    let source = "$trace=[]\nmodule M\n  def self.private; $trace << :private; super; end\nprivate\n  def value\n    x =\nif true\n  $trace << :rhs\n  [\"one\ntwo\", <<~H, __LINE__]\n raw\nH\nend\n    x\n  ensure\n    $trace << :ensure\n  end\n  public :value\nend\nclass C; include M; end\np [C.allocate.value, $trace, __LINE__, C.public_instance_methods(false), M.private_instance_methods(false)]\n";
    assert_constructor_corrects(&ruby, source, BOTH);
    let before = native::run(&ruby, source);
    assert!(before.status.success(), "{before:?}");
    let d = project();
    let path = d.path().join("example.rb");
    fs::write(&path, source).unwrap();
    let out = run(d.path(), &["--fix", "--only", BOTH, "example.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let after = fs::read_to_string(&path).unwrap();
    assert_ne!(after, source);
    let actual = native::run(&ruby, &after);
    assert_eq!(actual.status.code(), before.status.code());
    assert_eq!(actual.stdout, before.stdout);
    assert_eq!(actual.stderr, before.stderr);
}
