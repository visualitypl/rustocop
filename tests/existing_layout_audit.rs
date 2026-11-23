//! The 21 original Layout cops re-audited against the 79 Layout
//! cops added for v1.0, through the real CLI. Source-only pinned evidence is
//! in `tests/fixtures/existing_layout_audit.json`, recorded from the pinned
//! RuboCop Team at the fixed Layout defaults (reference inputs are never executed).
//! Canonical one-cop examples are ported from the pinned RuboCop Layout specs
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;

use rustocop::{cops::Registry, offense::Severity};

/// `(cop, source, corrected source, finding positions)`. `None` after: the
/// cop has no correction for the source (`Layout/LineLength`, a long string).
type Case = (
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static [(u32, u32)],
);

const LONG: &str = "x = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\n";

const CASES: [Case; 21] = [
    (
        "Layout/EmptyLineBetweenDefs",
        "def foo\nend\ndef bar\nend\n",
        Some("def foo\nend\n\ndef bar\nend\n"),
        &[(3, 1)],
    ),
    (
        "Layout/EmptyLines",
        "foo\n\n\nbar\n",
        Some("foo\n\nbar\n"),
        &[(3, 1)],
    ),
    (
        "Layout/EmptyLinesAroundClassBody",
        "class Foo\n\n  x\nend\n",
        Some("class Foo\n  x\nend\n"),
        &[(2, 1)],
    ),
    (
        "Layout/EmptyLinesAroundMethodBody",
        "def foo\n\n  x\nend\n",
        Some("def foo\n  x\nend\n"),
        &[(2, 1)],
    ),
    (
        "Layout/EmptyLinesAroundModuleBody",
        "module Foo\n\n  x\nend\n",
        Some("module Foo\n  x\nend\n"),
        &[(2, 1)],
    ),
    (
        "Layout/EndAlignment",
        "if foo\n  bar\n  end\n",
        Some("if foo\n  bar\nend\n"),
        &[(3, 3)],
    ),
    (
        "Layout/IndentationStyle",
        "def foo\n\tbar\nend\n",
        Some("def foo\n  bar\nend\n"),
        &[(2, 1)],
    ),
    (
        "Layout/IndentationWidth",
        "def foo\n    bar\nend\n",
        Some("def foo\n  bar\nend\n"),
        &[(2, 1)],
    ),
    ("Layout/LineLength", LONG, None, &[(1, 121)]),
    (
        "Layout/SpaceAfterComma",
        "foo(1,2)\n",
        Some("foo(1, 2)\n"),
        &[(1, 6)],
    ),
    (
        "Layout/SpaceAfterSemicolon",
        "foo;bar\n",
        Some("foo; bar\n"),
        &[(1, 4)],
    ),
    (
        "Layout/SpaceAroundEqualsInParameterDefault",
        "def foo(bar=1)\nend\n",
        Some("def foo(bar = 1)\nend\n"),
        &[(1, 12)],
    ),
    (
        "Layout/SpaceAroundOperators",
        "foo+bar\n",
        Some("foo + bar\n"),
        &[(1, 4)],
    ),
    (
        "Layout/SpaceBeforeComma",
        "foo(1 , 2)\n",
        Some("foo(1, 2)\n"),
        &[(1, 6)],
    ),
    (
        "Layout/SpaceInsideArrayLiteralBrackets",
        "[ 1 ]\n",
        Some("[1]\n"),
        &[(1, 2), (1, 4)],
    ),
    (
        "Layout/SpaceInsideBlockBraces",
        "foo {bar}\n",
        Some("foo { bar }\n"),
        &[(1, 6), (1, 9)],
    ),
    (
        "Layout/SpaceInsideHashLiteralBraces",
        "{foo: 1}\n",
        Some("{ foo: 1 }\n"),
        &[(1, 1), (1, 8)],
    ),
    (
        "Layout/SpaceInsideParens",
        "foo( 1 )\n",
        Some("foo(1)\n"),
        &[(1, 5), (1, 7)],
    ),
    (
        "Layout/SpaceInsideReferenceBrackets",
        "foo[ 1 ]\n",
        Some("foo[1]\n"),
        &[(1, 5), (1, 7)],
    ),
    (
        "Layout/TrailingEmptyLines",
        "foo\n\n",
        Some("foo\n"),
        &[(2, 1)],
    ),
    (
        "Layout/TrailingWhitespace",
        "foo  \n",
        Some("foo\n"),
        &[(1, 4)],
    ),
];

fn project(config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), config).unwrap();
    dir
}

fn cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

/// `expect_early_error` only for runs whose asserted exit is the early
/// `--stdin --fix` rejection, which may exit before reading stdin.
fn stdin(dir: &Path, args: &[&str], input: &[u8], expect_early_error: bool) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Only an expected usage error may exit before reading its input.
    stdin_support::write_stdin(child.stdin.take().unwrap(), input, expect_early_error).unwrap();
    child.wait_with_output().unwrap()
}

/// `(line, column, cop)` of every text-format finding.
fn findings(out: &Output) -> Vec<(u32, u32, String)> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let cop = line.strip_suffix(')')?.rsplit_once(" (")?.1;
            let mut parts = line.splitn(4, ':');
            let (_, l, c) = (parts.next()?, parts.next()?, parts.next()?);
            Some((l.parse().ok()?, c.parse().ok()?, cop.to_owned()))
        })
        .collect()
}

fn positions(out: &Output, cop: &str) -> Vec<(u32, u32)> {
    findings(out)
        .into_iter()
        .filter(|(_, _, c)| c == cop)
        .map(|(l, c, _)| (l, c))
        .collect()
}

fn applied(out: &Output) -> usize {
    let text = String::from_utf8_lossy(&out.stdout);
    let summary = text.lines().last().unwrap_or_default();
    summary
        .split(", ")
        .find_map(|part| part.strip_suffix(" corrections applied"))
        .or_else(|| {
            summary
                .split(", ")
                .find_map(|part| part.strip_suffix(" correction applied"))
        })
        .map_or(0, |n| n.parse().unwrap())
}

struct Identity {
    bytes: Vec<u8>,
    modified: std::time::SystemTime,
    permissions: fs::Permissions,
    #[cfg(unix)]
    inode: u64,
}

fn identity(path: &Path) -> Identity {
    let metadata = fs::metadata(path).unwrap();
    Identity {
        bytes: fs::read(path).unwrap(),
        modified: metadata.modified().unwrap(),
        permissions: metadata.permissions(),
        #[cfg(unix)]
        inode: std::os::unix::fs::MetadataExt::ino(&metadata),
    }
}

fn assert_untouched(path: &Path, before: &Identity, context: &str) {
    let after = identity(path);
    assert_eq!(after.bytes, before.bytes, "{context}");
    assert_eq!(after.modified, before.modified, "{context}");
    assert_eq!(after.permissions, before.permissions, "{context}");
    #[cfg(unix)]
    assert_eq!(after.inode, before.inode, "{context}");
}

#[test]
fn audited_identities_match_the_inventory_registry_and_fixed_policies() {
    let inventory: serde_json::Value =
        serde_json::from_str(include_str!("../tools/oracle/standard_reference.json")).unwrap();
    let audited: BTreeSet<&str> = CASES.iter().map(|(cop, ..)| *cop).collect();
    let registry = Registry::new();
    for record in inventory["cops"].as_array().unwrap() {
        let name = record["name"].as_str().unwrap();
        if !audited.contains(name) {
            continue;
        }
        let cop = registry
            .cops()
            .iter()
            .find(|cop| cop.name() == name)
            .unwrap_or_else(|| panic!("{name} is not registered"));
        let live = &record["fixed_policy"]["live_metadata"];
        assert_eq!(cop.default_enabled(), live["default_enabled"], "{name}");
        assert!(cop.default_enabled() && cop.department_selection_enables());
        assert_eq!(cop.safe_fixes(), live["safe_fixes"], "{name}");
        assert!(cop.safe_fixes(), "{name}");
        assert_eq!(cop.severity(), Severity::Convention, "{name}");
        assert_eq!(live["severity"], "Convention", "{name}");
        assert_eq!(
            cop.max_setting(),
            (name == "Layout/LineLength").then_some("max"),
            "{name}"
        );
        assert!(cop.autocorrect_incompatible_with().is_empty(), "{name}");
        assert!(cop.autocorrect_deferred_to().is_empty(), "{name}");
        assert!(cop.path_policy().is_none(), "{name}");
        assert_eq!(record["path_policy"]["include"], serde_json::json!([]));
        assert!(record["minimum_ruby"].is_null() && record["maximum_ruby"].is_null());
        assert_eq!(record["correction"]["kind"], "inherited_contract", "{name}");
        assert_eq!(
            record["correction"]["actual_project_safe_fixes"], live["safe_fixes"],
            "{name}"
        );
    }
}

#[test]
fn every_audited_cop_honors_selection_config_and_both_directive_prefixes() {
    for (cop, source, _, expected) in CASES {
        let dir = project("");
        let path = dir.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let before = identity(&path);

        let default = cli(dir.path(), &["x.rb"]);
        assert_eq!(default.status.code(), Some(1), "{cop}: {default:?}");
        assert_eq!(positions(&default, cop), expected, "{cop}");

        let only = cli(dir.path(), &["--only", cop, "x.rb"]);
        assert_eq!(only.status.code(), Some(1), "{cop}");
        assert_eq!(positions(&only, cop), expected, "{cop}");
        assert!(findings(&only).iter().all(|(_, _, c)| c == cop), "{cop}");

        let department = cli(dir.path(), &["--only", "Layout", "x.rb"]);
        assert_eq!(positions(&department, cop), expected, "{cop}");
        assert!(
            findings(&department)
                .iter()
                .all(|(_, _, c)| c.starts_with("Layout/")),
            "{cop}"
        );

        for except in [cop, "Layout"] {
            let out = cli(dir.path(), &["--except", except, "x.rb"]);
            assert!(positions(&out, cop).is_empty(), "{cop} --except {except}");
        }
        assert_untouched(&path, &before, cop);

        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.\"Layout\"]\nenabled = false\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"x.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), &config).unwrap();
            let out = cli(dir.path(), &["x.rb"]);
            assert!(positions(&out, cop).is_empty(), "{cop}: {config}");
            assert!(out.stderr.is_empty(), "{cop}: {config}: {out:?}");
            assert_untouched(&path, &before, cop);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();

        // A whole-file disable before the source, with either prefix, for the
        // cop and for its department: nothing reported, nothing corrected.
        for prefix in ["rubocop", "rustocop"] {
            for name in [cop, "Layout"] {
                let disabled = format!("# {prefix}:disable {name}\n{source}");
                fs::write(&path, &disabled).unwrap();
                let held = identity(&path);
                let out = cli(dir.path(), &["--fix", "--only", cop, "x.rb"]);
                assert_eq!(out.status.code(), Some(0), "{cop} {prefix} {name}: {out:?}");
                assert!(positions(&out, cop).is_empty(), "{cop} {prefix} {name}");
                assert_untouched(&path, &held, &format!("{cop} {prefix} {name}"));
            }
        }
    }
}

#[test]
fn every_audited_cop_corrects_or_holds_through_stdin_crlf_bom_and_data() {
    for (cop, source, after, expected) in CASES {
        let dir = project("");
        let out = stdin(
            dir.path(),
            &["--stdin", "x.rb", "--only", cop],
            source.as_bytes(),
            false,
        );
        assert_eq!(out.status.code(), Some(1), "{cop}: {out:?}");
        assert_eq!(positions(&out, cop), expected, "{cop}");
        let out = stdin(
            dir.path(),
            &["--stdin", "x.rb", "--fix", "--only", cop],
            source.as_bytes(),
            true,
        );
        assert_eq!(out.status.code(), Some(2), "{cop}: stdin cannot be fixed");
        assert!(out.stdout.is_empty(), "{cop}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
            "{cop}: {out:?}"
        );
        assert!(!dir.path().join("x.rb").exists(), "{cop}");

        let path = dir.path().join("x.rb");
        for variant in ["lf", "crlf", "bom", "data"] {
            let shape = |text: &str| match variant {
                "crlf" => text.replace('\n', "\r\n"),
                "bom" => format!("\u{feff}{text}"),
                "data" => format!("{text}__END__\n[ 1 ]  \n\tdata\n\n\n"),
                _ => text.to_owned(),
            };
            // TrailingEmptyLines looks at the end of the code, which is the
            // data marker in a file with a data section.
            if variant == "data" && cop == "Layout/TrailingEmptyLines" {
                continue;
            }
            let input = shape(source);
            fs::write(&path, &input).unwrap();
            let before = identity(&path);
            let out = cli(dir.path(), &["--fix", "--only", cop, "x.rb"]);
            let context = format!("{cop} {variant}");
            match after {
                Some(after) => {
                    assert_eq!(out.status.code(), Some(0), "{context}: {out:?}");
                    assert_eq!(
                        fs::read(&path).unwrap(),
                        crate::support::lf::fixed(&input, shape(after)),
                        "{context}"
                    );
                    let again = cli(dir.path(), &["--fix", "--only", cop, "x.rb"]);
                    assert_eq!(applied(&again), 0, "{context}: {again:?}");
                }
                None => {
                    assert_eq!(out.status.code(), Some(1), "{context}: {out:?}");
                    assert_eq!(positions(&out, cop), expected, "{context}");
                    assert_untouched(&path, &before, &context);
                }
            }
        }
    }
}

#[test]
fn read_only_targets_and_detection_only_line_length_are_never_written() {
    for (cop, source, _, _) in CASES {
        let dir = project("");
        let path = dir.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        let before = identity(&path);
        let plain = cli(dir.path(), &["--only", cop, "x.rb"]);
        assert_eq!(plain.status.code(), Some(1), "{cop}");
        assert_untouched(&path, &before, cop);
        let fixed = cli(dir.path(), &["--fix", "--only", cop, "x.rb"]);
        assert_untouched(&path, &before, cop);
        if cop == "Layout/LineLength" {
            assert_eq!(fixed.status.code(), Some(1), "{fixed:?}");
            assert!(fixed.stderr.is_empty(), "{fixed:?}");
        } else {
            assert_eq!(fixed.status.code(), Some(2), "{cop}: {fixed:?}");
            assert!(
                String::from_utf8_lossy(&fixed.stdout).contains("read-only"),
                "{cop}: {fixed:?}"
            );
        }
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
    }
}

/// Layout/LineLength (max 120) composes with the new whitespace, brace and
/// alignment corrections as in RuboCop: it is measured on the bytes each pass
/// sees, its line breaks merge with the other edits, and its `max` setting and
/// directives apply.
#[test]
fn line_length_120_composes_with_new_layout_corrections() {
    let padding = "a".repeat(102);
    // 121 columns before the fix: LineLength breaks before the last argument
    // in the same pass as SpaceInsideParens, and ArgumentAlignment aligns it
    // (RuboCop 1.92.0 writes the same bytes).
    let shortened = format!("foo( {padding}, bbbbbbbbbb )\n");
    assert_eq!(shortened.len() - 1, 121);
    let dir = project("");
    let path = dir.path().join("x.rb");
    fs::write(&path, &shortened).unwrap();
    let out = cli(dir.path(), &["--only", "Layout", "x.rb"]);
    assert_eq!(positions(&out, "Layout/LineLength"), [(1, 121)]);
    let out = cli(dir.path(), &["--fix", "--only", "Layout", "x.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read(&path).unwrap(),
        format!("foo({padding},\n    bbbbbbbbbb)\n").as_bytes()
    );

    // 119 columns before the fix and 123 after SpaceAfterComma and
    // SpaceAroundOperators insert spaces: the next pass breaks the line
    // (RuboCop 1.92.0 writes the same bytes).
    let lengthened = format!("x = [{},1,2]+[3]\n", "b".repeat(105));
    assert_eq!(lengthened.len() - 1, 119);
    fs::write(&path, &lengthened).unwrap();
    let out = cli(dir.path(), &["--fix", "--only", "Layout", "x.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read(&path).unwrap(),
        format!("x = [{}, 1,\n     2] + [3]\n", "b".repeat(105)).as_bytes()
    );

    // Indentation corrections move a line past the limit as well.
    let indented = format!("def m\n{}\nend\n", "c".repeat(119));
    fs::write(&path, &indented).unwrap();
    let out = cli(dir.path(), &["--fix", "--only", "Layout", "x.rb"]);
    assert_eq!(positions(&out, "Layout/LineLength"), [(2, 121)], "{out:?}");

    // The fixed max and both directive prefixes still govern detection.
    for (config, line, reported) in [
        (
            "[cops.\"Layout/LineLength\"]\nmax = 130\n",
            LONG.to_owned(),
            false,
        ),
        (
            "[cops.\"Layout/LineLength\"]\nmax = 100\n",
            format!("# {}\n", "e".repeat(99)),
            true,
        ),
        (
            "",
            format!("{} # rubocop:disable Layout/LineLength\n", LONG.trim_end()),
            false,
        ),
        (
            "",
            format!("{} # rustocop:disable Layout/LineLength\n", LONG.trim_end()),
            false,
        ),
        ("", format!("# {}\n", "d".repeat(119)), true),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(&path, &line).unwrap();
        let before = identity(&path);
        let out = cli(
            dir.path(),
            &["--fix", "--only", "Layout/LineLength", "x.rb"],
        );
        assert_eq!(
            !positions(&out, "Layout/LineLength").is_empty(),
            reported,
            "{config} {line:?}: {out:?}"
        );
        assert_untouched(&path, &before, &line);
    }
}

/// The audited cops' related integration-context specs (recorded with their
/// lines in the fixture), replayed through the CLI lifecycle they describe.
#[test]
fn related_integration_contexts_replay_through_the_cli_lifecycle() {
    let long = format!("{}\n", "#".repeat(130));
    // team_spec.rb 212-227: a cop disabled in the config and enabled again by
    // a directive comment reports its findings, with either prefix.
    for prefix in ["rubocop", "rustocop"] {
        let dir = project("[cops.\"Layout/LineLength\"]\nenabled = false\n");
        fs::write(
            dir.path().join("x.rb"),
            format!("# {prefix}:enable Layout/LineLength\n{long}puts test\n"),
        )
        .unwrap();
        let out = cli(dir.path(), &["x.rb"]);
        assert_eq!(
            positions(&out, "Layout/LineLength"),
            [(2, 121)],
            "{prefix}: {out:?}"
        );
    }
    // team_spec.rb 194-205: an exclusion wins over the opt-in comment.
    let dir = project("[cops.\"Layout/LineLength\"]\nexclude = [\"x.rb\"]\n");
    fs::write(
        dir.path().join("x.rb"),
        format!("# rubocop:enable Layout/LineLength\n{long}puts test\n"),
    )
    .unwrap();
    assert!(positions(&cli(dir.path(), &["x.rb"]), "Layout/LineLength").is_empty());
    // team_spec.rb 171-182: a parser warning does not stop the cops.
    let dir = project("");
    fs::write(dir.path().join("x.rb"), format!("{long}puts *test\n")).unwrap();
    let out = cli(dir.path(), &["x.rb"]);
    assert_eq!(positions(&out, "Layout/LineLength"), [(1, 121)]);
    assert_eq!(positions(&out, "Lint/AmbiguousOperator"), [(2, 6)]);
    // leading_empty_lines_spec.rb 84-111: both corrections in one pass.
    let path = dir.path().join("x.rb");
    fs::write(&path, "\ndef bar(arg =1); end\n").unwrap();
    let out = cli(
        dir.path(),
        &[
            "--fix",
            "--only",
            "Layout/LeadingEmptyLines,Layout/SpaceAroundEqualsInParameterDefault",
            "x.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), b"def bar(arg = 1); end\n");
    // config_spec.rb / config_loader_spec.rb: YAML parameters are not
    // configuration here; strict TOML rejects unknown and foreign keys before
    // any lint or write. junit_formatter_spec.rb: no such format.
    for (cop, source, _, _) in CASES {
        for config in [
            format!("[cops.\"{cop}\"]\nbar = 43\n"),
            format!("[cops.\"{cop}\"]\nmax = 3\n"),
        ] {
            if cop == "Layout/LineLength" && config.contains("max") {
                continue;
            }
            let dir = project(&config);
            let path = dir.path().join("x.rb");
            fs::write(&path, source).unwrap();
            let before = identity(&path);
            let out = cli(dir.path(), &["--fix", "x.rb"]);
            assert_eq!(out.status.code(), Some(2), "{config}: {out:?}");
            assert!(out.stdout.is_empty(), "{config}: {out:?}");
            assert_untouched(&path, &before, &config);
        }
    }
    let out = cli(dir.path(), &["--format", "junit", "x.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

/// A declared legacy encoding keeps its raw bytes through each audited
/// correction; a file that does not parse is reported and never written.
#[test]
fn legacy_encoded_bytes_survive_and_syntax_errors_never_write() {
    let header: &[u8] = b"# encoding: iso-8859-1\nNAME = \"caf\xe9\"\n";
    for (cop, source, after, expected) in CASES {
        let dir = project("");
        let path = dir.path().join("x.rb");
        let input = [header, source.as_bytes()].concat();
        fs::write(&path, &input).unwrap();
        let before = identity(&path);
        let out = cli(dir.path(), &["--only", cop, "x.rb"]);
        let shifted: Vec<(u32, u32)> = expected.iter().map(|&(l, c)| (l + 2, c)).collect();
        assert_eq!(positions(&out, cop), shifted, "{cop}: {out:?}");
        assert_untouched(&path, &before, cop);
        let out = cli(dir.path(), &["--fix", "--only", cop, "x.rb"]);
        match after {
            Some(after) => {
                assert_eq!(out.status.code(), Some(0), "{cop}: {out:?}");
                assert_eq!(
                    fs::read(&path).unwrap(),
                    [header, after.as_bytes()].concat()
                );
            }
            None => assert_untouched(&path, &before, cop),
        }

        let broken = format!("{source}def (\n");
        fs::write(&path, &broken).unwrap();
        let before = identity(&path);
        let out = cli(dir.path(), &["--fix", "--only", cop, "x.rb"]);
        assert_eq!(out.status.code(), Some(2), "{cop}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("x.rb: error: syntax error"),
            "{cop}: {out:?}"
        );
        assert!(positions(&out, cop).is_empty(), "{cop}: {out:?}");
        assert_untouched(&path, &before, &broken);
    }
}
