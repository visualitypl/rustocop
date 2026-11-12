//! Correction applicability of the 107 standard-core cops,
//! reconciled through the real CLI against the actual upstream evidence the
//! inventory records, not class flags or counts.
//!
//! - Every `safe_source` row carries a canonical upstream edit (a pinned spec
//!   example replayed by RuboCop 1.92.0 at the fixed defaults, its source and
//!   corrected bytes hashed in `tools/oracle/standard_reference.json`).
//!   `--fix` must write exactly that edit, or the cop is one of the closed,
//!   individually documented holds below: the finding stays and the file keeps
//!   its bytes, mtime, permissions and inode.
//! - Every `detection_only` and `non_source` row must never write: `--fix` on
//!   a real finding keeps the file identical.
//!
//! The witnesses are pinned RuboCop spec examples at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use serde_json::Value;

fn inventory() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/oracle/standard_reference.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn rows<'a>(inventory: &'a Value, kinds: &[&str]) -> Vec<&'a Value> {
    inventory["cops"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| kinds.contains(&row["correction"]["kind"].as_str().unwrap()))
        .collect()
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// `(line, column)` of every text-format finding of `cop`; a file-level
/// finding (`path: C: ...`) is `(0, 0)`.
fn positions(out: &Output, cop: &str) -> Vec<(u64, u64)> {
    let suffix = format!("({cop})");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.ends_with(&suffix))
        .map(|line| {
            let mut parts = line.splitn(4, ':');
            parts.next();
            let line = parts.next().unwrap().trim().parse().unwrap_or(0);
            let col = parts.next().unwrap().trim().parse().unwrap_or(0);
            (line, col)
        })
        .collect()
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

/// `--fix --only cop` on `source` at `file`; returns the output and asserts
/// the no-write identity when `held`.
fn fix(dir: &Path, cop: &str, file: &str, source: &str, held: bool) -> (Output, Vec<u8>) {
    let path = dir.join(file);
    fs::write(&path, source).unwrap();
    let before = identity(&path);
    let out = cli(dir, &["--fix", "--fix-passes", "1", "--only", cop, file]);
    let after = identity(&path);
    if held {
        assert_eq!(out.status.code(), Some(1), "{cop}: {out:?}");
        assert_eq!(after.bytes, before.bytes, "{cop}");
        assert_eq!(after.modified, before.modified, "{cop}");
        assert_eq!(after.permissions, before.permissions, "{cop}");
        #[cfg(unix)]
        assert_eq!(after.inode, before.inode, "{cop}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("0 corrections applied"), "{cop}: {text}");
    }
    (out, after.bytes)
}

/// The closed list of `safe_source` cops whose upstream witness is a
/// documented hold here, each with its reason.
/// `true`: the same witness inside the documented closed inactive envelope
/// (`if false ... end`) gets the canonical edit.
const HELD_WITNESSES: [(&str, bool, &str); 7] = [
    (
        "Bundler/InsecureProtocolSource",
        true,
        "the edit changes what a live Gemfile resolves; closed inactive envelope only",
    ),
    (
        "Bundler/OrderedGems",
        true,
        "reordering a live Gemfile changes resolution; closed inactive envelope only",
    ),
    (
        "Gemspec/AddRuntimeDependency",
        true,
        "gemspec edits only in the closed inactive envelope",
    ),
    (
        "Gemspec/DeprecatedAttributeAssignment",
        true,
        "gemspec edits only in the closed inactive envelope",
    ),
    (
        "Gemspec/OrderedDependencies",
        true,
        "gemspec edits only in the closed inactive envelope",
    ),
    (
        "Gemspec/RequireMFA",
        true,
        "gemspec edits only in the closed inactive envelope",
    ),
    (
        "Layout/MultilineMethodArgumentLineBreaks",
        false,
        "pinned line 47 edit before a heredoc opener is held by the shared heredoc boundary",
    ),
];

#[test]
fn every_safe_source_witness_is_the_canonical_edit_or_a_documented_hold() {
    let inventory = inventory();
    let safe = rows(&inventory, &["safe_source"]);
    assert_eq!(safe.len(), 84);
    let names: BTreeSet<_> = safe.iter().map(|r| r["name"].as_str().unwrap()).collect();
    for (name, _, _) in HELD_WITNESSES {
        assert!(names.contains(name), "{name} is not a safe_source row");
    }
    let dir = project();
    for row in safe {
        let cop = row["name"].as_str().unwrap();
        let witness = &row["correction"]["witness"];
        let file = witness["path"].as_str().unwrap();
        let before = witness["before"].as_str().unwrap();
        let after = witness["after"].as_str().unwrap();
        assert_ne!(before, after, "{cop}");

        // Findings at the upstream positions, before any edit.
        fs::write(dir.path().join(file), before).unwrap();
        let mut found = positions(&cli(dir.path(), &["--only", cop, file]), cop);
        let mut expected: Vec<(u64, u64)> = witness["offenses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| (o["line"].as_u64().unwrap(), o["col"].as_u64().unwrap()))
            .collect();
        found.sort_unstable();
        expected.sort_unstable();
        assert_eq!(found, expected, "{cop}: {before:?}");

        let hold = HELD_WITNESSES.iter().find(|(name, _, _)| *name == cop);
        let (_, written) = fix(dir.path(), cop, file, before, hold.is_some());
        match hold {
            None => assert_eq!(
                String::from_utf8(written).unwrap(),
                after,
                "{cop}: canonical edit of {before:?}"
            ),
            Some((_, envelope, reason)) => {
                assert_eq!(written, before.as_bytes(), "{cop}: {reason}");
                if *envelope {
                    let (_, written) = fix(
                        dir.path(),
                        cop,
                        file,
                        &format!("if false\n{before}end\n"),
                        false,
                    );
                    assert_eq!(
                        String::from_utf8(written).unwrap(),
                        format!("if false\n{after}end\n"),
                        "{cop}: canonical edit in the closed inactive envelope"
                    );
                }
            }
        }
        fs::remove_file(dir.path().join(file)).unwrap();
    }
}

fn gemspec(body: &str) -> String {
    format!("Gem::Specification.new do |spec|\n{body}end\n")
}

fn lines(open: &str, count: usize, line: impl Fn(usize) -> String) -> String {
    let body: String = (1..=count).map(line).collect();
    format!("{open}\n{body}end\n")
}

#[test]
fn every_detection_only_and_non_source_cop_never_writes() {
    let inventory = inventory();
    let expected: BTreeSet<_> = rows(&inventory, &["detection_only", "non_source"])
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_owned())
        .collect();
    let cases: Vec<(&str, &str, String)> = vec![
        (
            "Bundler/DuplicatedGem",
            "Gemfile",
            "gem 'a'\ngem 'a'\n".into(),
        ),
        (
            "Bundler/DuplicatedGroup",
            "Gemfile",
            "group :dev do\n  gem 'a'\nend\ngroup :dev do\n  gem 'b'\nend\n".into(),
        ),
        ("Bundler/GemComment", "Gemfile", "gem 'a'\n".into()),
        ("Bundler/GemFilename", "gems.rb", "gem 'a'\n".into()),
        ("Bundler/GemVersion", "Gemfile", "gem 'a'\n".into()),
        (
            "Gemspec/AttributeAssignment",
            "x.gemspec",
            gemspec("  spec.metadata = {}\n  spec.metadata['a'] = 'b'\n"),
        ),
        (
            "Gemspec/DependencyVersion",
            "x.gemspec",
            gemspec("  spec.add_dependency 'a'\n"),
        ),
        (
            "Gemspec/DevelopmentDependencies",
            "x.gemspec",
            gemspec("  spec.add_development_dependency 'a'\n"),
        ),
        (
            "Gemspec/DuplicatedAssignment",
            "x.gemspec",
            gemspec("  spec.name = 'a'\n  spec.name = 'b'\n"),
        ),
        (
            "Gemspec/RequiredRubyVersion",
            "x.gemspec",
            gemspec("  spec.required_ruby_version = '>= 2.0'\n"),
        ),
        (
            "Gemspec/RubyVersionGlobalsUsage",
            "x.gemspec",
            gemspec("  spec.version = RUBY_VERSION\n"),
        ),
        (
            "Layout/ClassStructure",
            "x.rb",
            "class A\n  def m; end\n  include B\nend\n".into(),
        ),
        ("Layout/EndOfLine", "x.rb", "x = 1\r\n".into()),
        (
            "Metrics/AbcSize",
            "x.rb",
            lines("def m", 18, |i| format!("  a{i} = b.c{i}\n")),
        ),
        (
            "Metrics/BlockLength",
            "x.rb",
            lines("m do", 26, |i| format!("  a{i}\n")),
        ),
        (
            "Metrics/BlockNesting",
            "x.rb",
            "if a\n  if b\n    if c\n      if d\n        e\n      end\n    end\n  end\nend\n"
                .into(),
        ),
        (
            "Metrics/ClassLength",
            "x.rb",
            lines("class A", 101, |i| format!("  a{i}\n")),
        ),
        (
            "Metrics/CollectionLiteralLength",
            "x.rb",
            format!(
                "X = [{}]\n",
                (1..=250)
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        (
            "Metrics/CyclomaticComplexity",
            "x.rb",
            lines("def m", 8, |i| format!("  a if b{i}\n")),
        ),
        (
            "Metrics/MethodLength",
            "x.rb",
            lines("def m", 11, |i| format!("  a{i}\n")),
        ),
        (
            "Metrics/ModuleLength",
            "x.rb",
            lines("module A", 101, |i| format!("  a{i}\n")),
        ),
        (
            "Metrics/ParameterLists",
            "x.rb",
            "def m(a, b, c, d, e, f)\nend\n".into(),
        ),
        (
            "Metrics/PerceivedComplexity",
            "x.rb",
            lines("def m", 9, |i| format!("  a if b{i}\n")),
        ),
    ];
    let covered: BTreeSet<_> = cases.iter().map(|(cop, _, _)| (*cop).to_owned()).collect();
    assert_eq!(covered, expected);
    let dir = project();
    for (cop, file, source) in &cases {
        let (out, _) = fix(dir.path(), cop, file, source, true);
        assert_eq!(positions(&out, cop).len(), 1, "{cop}: {out:?}");
        fs::remove_file(dir.path().join(file)).unwrap();
    }
}
