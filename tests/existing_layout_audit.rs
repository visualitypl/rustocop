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

#[path = "support/stdin.rs"]
mod stdin_support;

use rustocop::{cops::Registry, offense::Severity};

/// `(cop, source, corrected source, finding positions)`. `None` after: the
/// cop is detection-only (`Layout/LineLength`).
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
        assert_eq!(cop.safe_fixes(), name != "Layout/LineLength", "{name}");
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
                        shape(after).as_bytes(),
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

/// The closed observer list against the audited cops' guards.
/// Section 1 withholds every correction (finding kept, file untouched) when
/// the file contains a listed source observer, whatever the observation can
/// reach: `__FILE__`, `__dir__`, `DATA` with a data section, `$0`,
/// `$PROGRAM_NAME`, `caller`, `caller_locations`, `source_location` and
/// `SCRIPT_LINES__`. Lookalikes outside the list keep correcting.
/// `(prefix, data section, held)`.
const OBSERVERS: [(&str, bool, bool); 22] = [
    ("p SCRIPT_LINES__\n", false, true),
    ("SCRIPT_LINES__ = {}\n", false, true),
    ("p ::SCRIPT_LINES__\n", false, true),
    ("p Object.const_get(:SCRIPT_LINES__)\n", false, true),
    (
        "SCRIPT_LINES__ = {}\nload __FILE__ unless defined?(DONE)\n",
        false,
        true,
    ),
    ("puts File.read(__FILE__)\n", false, true),
    ("puts File.read($0)\n", false, true),
    ("puts File.size(__dir__ + '/x.rb')\n", false, true),
    ("p DATA.pos\n", true, true),
    ("p caller_locations(0).first.lineno\n", false, true),
    ("p caller.first\n", false, true),
    ("def m = 1\np method(:m).source_location\n", false, true),
    ("p __FILE__\n", false, true),
    ("p __dir__\n", false, true),
    ("p $0\n", false, true),
    ("p $PROGRAM_NAME\n", false, true),
    ("$PROGRAM_NAME = 'name'\n", false, true),
    ("p DATA.read\n", true, true),
    ("p SCRIPT_LINES\n", false, false),
    ("p '__FILE__ $0 caller'\n", false, false),
    ("# __dir__ SCRIPT_LINES__\n", false, false),
    ("p :caller_name, $PROGRAM\n", false, false),
];

#[test]
fn closed_list_observers_hold_every_audited_correction() {
    let dir = project("");
    let path = dir.path().join("x.rb");
    for (cop, source, after, _) in CASES {
        let Some(after) = after else { continue };
        for (prefix, data, held) in OBSERVERS {
            // A data section ends the code TrailingEmptyLines looks at.
            if data && cop == "Layout/TrailingEmptyLines" {
                continue;
            }
            let shape = |code: &str| {
                let tail = if data { "__END__\ndata  \n" } else { "" };
                format!("{prefix}{code}{tail}")
            };
            fs::write(&path, shape(source)).unwrap();
            let before = identity(&path);
            let out = cli(dir.path(), &["--fix", "--only", cop, "x.rb"]);
            let context = format!("{cop}: {prefix:?}: {out:?}");
            assert!(out.stderr.is_empty(), "{context}");
            if held {
                assert_eq!(out.status.code(), Some(1), "{context}");
                assert!(!positions(&out, cop).is_empty(), "{context}");
                assert_eq!(applied(&out), 0, "{context}");
                assert_untouched(&path, &before, &context);
            } else {
                assert_eq!(out.status.code(), Some(0), "{context}");
                assert_eq!(
                    fs::read(&path).unwrap(),
                    shape(after).as_bytes(),
                    "{context}"
                );
            }
        }
    }
}

/// The inherited central barriers stay in force: self-reads and unknown
/// block passes keep the finding without a write (the latter
/// is a known gap), while a symbol block pass is corrected.
#[test]
fn inherited_raw_source_and_code_object_barriers_still_hold() {
    let dir = project("");
    let path = dir.path().join("x.rb");
    for (prefix, held) in [
        ("puts File.read('x.rb')\n", false),
        ("items.each(&:inspect)\n", false),
        ("items.each(&handler)\n", true),
        ("handler = 1\nitems.each(&handler)\n", true),
        ("x = File.read(File.join(Dir.pwd, 'x.rb'))\n", false),
    ] {
        for (cop, source, after, _) in CASES {
            let Some(after) = after else { continue };
            fs::write(&path, format!("{prefix}{source}")).unwrap();
            let before = identity(&path);
            let out = cli(dir.path(), &["--fix", "--only", cop, "x.rb"]);
            if held {
                assert_eq!(applied(&out), 0, "{cop}: {prefix:?}");
                assert_untouched(&path, &before, &format!("{cop}: {prefix:?}"));
            } else {
                assert!(applied(&out) > 0, "{cop}: {prefix:?}: {out:?}");
                assert_eq!(
                    fs::read(&path).unwrap(),
                    format!("{prefix}{after}").as_bytes(),
                    "{cop}: {prefix:?}"
                );
            }
        }
    }
    // A file that reads itself through __FILE__ is held by both guards.
    for (cop, source, after, _) in CASES {
        if after.is_none() {
            continue;
        }
        fs::write(&path, format!("puts File.read(__FILE__)\n{source}")).unwrap();
        let before = identity(&path);
        let out = cli(dir.path(), &["--fix", "--only", cop, "x.rb"]);
        assert_eq!(applied(&out), 0, "{cop}");
        assert_untouched(&path, &before, cop);
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

/// Layout/LineLength (max 120, detection-only) composes with the new
/// whitespace, brace and alignment corrections: it is measured on the bytes
/// each pass sees, never edits, and its `max` setting and directives apply.
#[test]
fn line_length_120_composes_with_new_layout_corrections() {
    let padding = "a".repeat(102);
    // 121 columns before the fix; 119 after SpaceInsideParens removes two
    // spaces: the finding goes away with the correction that shortened it.
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
        format!("foo({padding}, bbbbbbbbbb)\n").as_bytes()
    );

    // 119 columns before the fix and 123 after SpaceAfterComma and
    // SpaceAroundOperators insert spaces: the next pass reports the line.
    let lengthened = format!("x = [{},1,2]+[3]\n", "b".repeat(105));
    assert_eq!(lengthened.len() - 1, 119);
    fs::write(&path, &lengthened).unwrap();
    let out = cli(dir.path(), &["--fix", "--only", "Layout", "x.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(positions(&out, "Layout/LineLength"), [(1, 121)]);
    assert_eq!(
        fs::read(&path).unwrap(),
        format!("x = [{}, 1, 2] + [3]\n", "b".repeat(105)).as_bytes()
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

fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    serde_json::from_value(row[key].clone()).unwrap()
}

/// Repeated explicit single-pass `--fix` runs, as RuboCop's autocorrect loop
/// repeats its Team; at most 12 passes, and the last applies nothing.
fn fixpoint(dir: &Path, source: &[u8]) -> Vec<u8> {
    let path = dir.join("x.rb");
    fs::write(&path, source).unwrap();
    for _ in 0..12 {
        let out = cli(dir, &["--fix", "--only", "Layout", "x.rb"]);
        assert!(
            matches!(out.status.code(), Some(0 | 1)) && out.stderr.is_empty(),
            "{source:?}: {out:?}"
        );
        if applied(&out) == 0 {
            return fs::read(&path).unwrap();
        }
    }
    panic!("no fixpoint after 12 passes: {source:?}");
}

#[test]
fn composition_fixture_matches_pinned_findings_and_autocorrect_fixpoints() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/existing_layout_audit.json")).unwrap();
    let audited: BTreeSet<&str> = CASES.iter().map(|(cop, ..)| *cop).collect();
    let rows = fixture["rows"].as_array().unwrap();
    assert!(rows.len() >= 60, "{}", rows.len());
    let dir = project("");
    let mut mismatches = Vec::new();
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let source = bytes(row, "bytes");
        fs::write(dir.path().join("x.rb"), &source).unwrap();
        let out = cli(dir.path(), &["--only", "Layout", "x.rb"]);
        let actual: Vec<(u32, u32, String)> = findings(&out)
            .into_iter()
            .filter(|(_, _, cop)| audited.contains(cop.as_str()))
            .collect();
        let expected: Vec<(u32, u32, String)> =
            serde_json::from_value(row["project_findings"].clone()).unwrap();
        let mut actual_sorted = actual.clone();
        actual_sorted.sort();
        if actual_sorted != expected {
            mismatches.push(format!(
                "{name}: findings {actual_sorted:?} != {expected:?}"
            ));
        }
        let fixed = fixpoint(dir.path(), &source);
        if fixed != bytes(row, "project_after") {
            mismatches.push(format!(
                "{name}: fixpoint {:?} != {:?}",
                String::from_utf8_lossy(&fixed),
                String::from_utf8_lossy(&bytes(row, "project_after"))
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
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

/// Native runtimes for the authored controls: native Ruby 3.4
/// (`RUSTOCOP_RUBY34`) and the host `ruby` when present.
fn runtimes() -> Vec<String> {
    let host = Command::new("ruby").arg("--version").output().is_ok();
    host.then(|| "ruby".to_owned())
        .into_iter()
        .chain(std::env::var("RUSTOCOP_RUBY34"))
        .collect()
}

/// Exit status, stdout and the warnings (line numbers dropped: a layout edit
/// may move a line, never change a warning) of one bounded authored program.
fn native(runtime: &str, dir: &Path) -> (Option<i32>, Vec<u8>, Vec<String>) {
    let out = Command::new(runtime)
        .current_dir(dir)
        .args(["--disable-gems", "-w", "x.rb"])
        .output()
        .unwrap();
    let warnings = String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(|line| match line.strip_prefix("x.rb:") {
            Some(rest) => rest.split_once(':').map_or(rest, |(_, w)| w).to_owned(),
            None => line.to_owned(),
        })
        .collect();
    (out.status.code(), out.stdout, warnings)
}

const NATIVE_COMPOSITION: &str = "# frozen_string_literal: true   \nmodule Billing\n  class Invoice\n\n      TAX = 2\n    def total( items , discount=0 )\n        sum = items.sum {|i|i*TAX}\n      net = sum-discount\n      [ net , { a: 1 }[ :a ] , \"keep  spaces  \" , net.frozen? ]\n    end\n\n\n    def label;\"x\" ;end\n    def unused_warning\n      unused = 1\n    end\n  end\n\nend\nlist = [ 1,2 ]\np Billing::Invoice.new.total(list, 1)\np Billing::Invoice.new.label.frozen?\np(*list)\ntext = <<~DOC\n  trailing   \n\ttab\nDOC\np text\n";

const NATIVE_COMPOSITION_FIXED: &str = "# frozen_string_literal: true\n\nmodule Billing\n  class Invoice\n    TAX = 2\n    def total(items, discount = 0)\n      sum = items.sum { |i| i * TAX }\n      net = sum - discount\n      [net, { a: 1 }[:a], \"keep  spaces  \", net.frozen?]\n    end\n\n    def label; \"x\"; end\n\n    def unused_warning\n      unused = 1\n    end\n  end\nend\nlist = [1, 2]\np Billing::Invoice.new.total(list, 1)\np Billing::Invoice.new.label.frozen?\np(*list)\ntext = <<~DOC\n  trailing   \n\ttab\nDOC\np text\n";

/// Bounded, personally authored native controls (no reference, spec or
/// corpus input is executed): the `--only Layout --fix` fixpoint of the 21
/// audited cops composed with the new alignment/brace/whitespace/blank-line
/// cops keeps the program's exit status, output, `-w` warnings, the magic
/// comments' activation (frozen literals, a legacy encoding header) and the
/// literal bytes; the fixpoint is atomic and idempotent, and a file with a
/// listed observer is never written.
#[test]
fn native_ruby_controls_keep_runtime_warnings_and_encoding_headers() {
    let runtimes = runtimes();
    if runtimes.is_empty() {
        return;
    }
    let crlf = NATIVE_COMPOSITION.replace('\n', "\r\n");
    let crlf_fixed = NATIVE_COMPOSITION_FIXED.replace('\n', "\r\n");
    let corrected: [(&[u8], &[u8]); 3] = [
        (
            NATIVE_COMPOSITION.as_bytes(),
            NATIVE_COMPOSITION_FIXED.as_bytes(),
        ),
        (crlf.as_bytes(), crlf_fixed.as_bytes()),
        (
            b"# encoding: iso-8859-1  \nNAME = \"caf\xe9\"  \n\n\np [ __ENCODING__ , NAME.bytes , NAME.valid_encoding? ]\n",
            b"# encoding: iso-8859-1\n\nNAME = \"caf\xe9\"\n\np [__ENCODING__, NAME.bytes, NAME.valid_encoding?]\n",
        ),
    ];
    for (source, fixed) in corrected {
        let dir = project("");
        let path = dir.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let before: Vec<_> = runtimes.iter().map(|r| native(r, dir.path())).collect();
        for result in &before {
            assert_eq!(result.0, Some(0), "{result:?}");
        }
        let after = fixpoint(dir.path(), source);
        assert_eq!(
            String::from_utf8_lossy(&after),
            String::from_utf8_lossy(fixed)
        );
        assert_eq!(after, fixed);
        let now: Vec<_> = runtimes.iter().map(|r| native(r, dir.path())).collect();
        assert_eq!(now, before, "{}", String::from_utf8_lossy(fixed));
        // The fixpoint is stable: another pass writes nothing.
        let stable = identity(&path);
        let out = cli(dir.path(), &["--fix", "--only", "Layout", "x.rb"]);
        assert_eq!(applied(&out), 0, "{out:?}");
        assert_untouched(&path, &stable, "fixpoint");
    }
    // The composite's frozen-literal header is live before and after.
    let dir = project("");
    fs::write(dir.path().join("x.rb"), NATIVE_COMPOSITION).unwrap();
    for runtime in &runtimes {
        let (_, stdout, warnings) = native(runtime, dir.path());
        assert!(
            String::from_utf8_lossy(&stdout).contains("\ntrue\n"),
            "{runtime}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("unused variable")),
            "{runtime}: {warnings:?}"
        );
    }
    // A listed observer holds every audited edit: same native result, no write.
    for source in [
        "x = [ 1,2 ]  \np [x, __FILE__ == $0]\n",
        "def m\n    caller_locations(0).size\nend\np [ m ]\n",
    ] {
        let dir = project("");
        let path = dir.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let before: Vec<_> = runtimes.iter().map(|r| native(r, dir.path())).collect();
        let untouched = identity(&path);
        let out = cli(dir.path(), &["--fix", "--only", "Layout", "x.rb"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(applied(&out), 0, "{out:?}");
        assert_untouched(&path, &untouched, source);
        let now: Vec<_> = runtimes.iter().map(|r| native(r, dir.path())).collect();
        assert_eq!(now, before, "{source}");
    }
}
