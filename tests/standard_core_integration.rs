//! All-613 standard-core integration through the real CLI.
//!
//! The listed-observer rule withholds every correction, in every department,
//! when the linted file contains a listed source observer (`__FILE__`,
//! `__dir__`, `DATA` with `__END__`, `$0`, `$PROGRAM_NAME`, `caller`,
//! `caller_locations`, `source_location`, `SCRIPT_LINES__`): the finding stays,
//! the file keeps its bytes, mtime, permissions and inode. Lookalikes keep the
//! canonical edit. The other tests check one cop per department through
//! default/opt-in selection, strict TOML, both directive prefixes, stdin and
//! the parse-error no-write lifecycle with valid peers.
//! Canonical one-cop examples are ported from the pinned RuboCop specs at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

use std::{
    fs,
    io::Write as _,
    path::Path,
    process::{Command, Output, Stdio},
};

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
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn stdin(dir: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(input);
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

fn cops(out: &Output) -> Vec<String> {
    let mut names: Vec<_> = findings(out).into_iter().map(|(_, _, c)| c).collect();
    names.sort();
    names.dedup();
    names
}

fn applied(out: &Output) -> usize {
    let text = String::from_utf8_lossy(&out.stdout);
    let summary = text.lines().last().unwrap_or_default();
    summary
        .split(", ")
        .find_map(|part| {
            part.strip_suffix(" corrections applied")
                .or_else(|| part.strip_suffix(" correction applied"))
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

/// `(cop, file, template, corrected)`: a real canonical correction of a
/// correcting cop in each department that has one. `{}` is where the
/// observer (or lookalike) line goes. Bundler and Gemspec edit only inside
/// their documented closed inactive envelope, so the line goes inside it.
const CORRECTING: [(&str, &str, &str, &str); 9] = [
    (
        "Bundler/OrderedGems",
        "Gemfile",
        "if false\n{}  gem 'b'\n  gem 'a'\nend\n",
        "if false\n{}  gem 'a'\n  gem 'b'\nend\n",
    ),
    (
        "Gemspec/OrderedDependencies",
        "x.gemspec",
        "if false\n{}  Gem::Specification.new do |spec|\n    spec.add_dependency 'b'\n    spec.add_dependency 'a'\n  end\nend\n",
        "if false\n{}  Gem::Specification.new do |spec|\n    spec.add_dependency 'a'\n    spec.add_dependency 'b'\n  end\nend\n",
    ),
    (
        "Gemspec/AddRuntimeDependency",
        "x.gemspec",
        "if false\n{}  Gem::Specification.new do |spec|\n    spec.add_runtime_dependency 'a'\n  end\nend\n",
        "if false\n{}  Gem::Specification.new do |spec|\n    spec.add_dependency 'a'\n  end\nend\n",
    ),
    ("Layout/SpaceAfterNot", "x.rb", "{}p ! y\n", "{}p !y\n"),
    (
        "Layout/TrailingWhitespace",
        "x.rb",
        "{}y = 1  \n",
        "{}y = 1\n",
    ),
    (
        "Lint/AmbiguousOperatorPrecedence",
        "x.rb",
        "{}p 1 + 2 * 3\n",
        "{}p 1 + (2 * 3)\n",
    ),
    (
        "Migration/DepartmentName",
        "x.rb",
        "{}# rubocop:disable Debugger\np 1\n",
        "{}# rubocop:disable Lint/Debugger\np 1\n",
    ),
    (
        "Naming/BlockForwarding",
        "x.rb",
        "{}def m(&block) = n(&block)\n",
        "{}def m(&) = n(&)\n",
    ),
    (
        "Style/StringLiterals",
        "x.rb",
        "{}y = 'a'\n",
        "{}y = \"a\"\n",
    ),
];

/// `(observer line, needs a data section)`: the closed list, also assigned,
/// rooted, reflected and nested spellings.
const LISTED: [(&str, bool); 16] = [
    ("p __FILE__\n", false),
    ("def f = __dir__\n", false),
    ("p DATA.read\n", true),
    ("p $0\n", false),
    ("$PROGRAM_NAME = 'name'\n", false),
    ("p $PROGRAM_NAME\n", false),
    ("p caller.first\n", false),
    ("p caller_locations(0).first.lineno\n", false),
    ("def g = 1\np method(:g).source_location\n", false),
    ("SCRIPT_LINES__ = {}\n", false),
    ("p SCRIPT_LINES__\n", false),
    ("p ::SCRIPT_LINES__\n", false),
    ("p Object.const_get(:SCRIPT_LINES__)\n", false),
    ("def h\n  caller\nend\n", false),
    ("::SCRIPT_LINES__, unused = {}, nil\n", false),
    ("::DATA, unused = DATA, nil\n", true),
];

/// Lookalikes naming no listed observer: the canonical edit still applies.
const LOOKALIKES: [&str; 4] = [
    "",
    "p 1\n",
    "p SCRIPT_LINES, :caller_name, $PROGRAM, __method__, __LINE__\n",
    "# __FILE__ $PROGRAM_NAME source_location\n",
];

#[test]
fn every_correcting_department_holds_for_every_listed_observer() {
    let dir = project("");
    for (cop, file, template, corrected) in CORRECTING {
        let path = dir.path().join(file);
        for lookalike in LOOKALIKES {
            fs::write(&path, template.replace("{}", lookalike)).unwrap();
            let out = cli(dir.path(), &["--fix", "--only", cop, file]);
            let context = format!("{cop}: {lookalike:?}: {out:?}");
            assert!(out.stderr.is_empty(), "{context}");
            assert!(applied(&out) > 0, "{context}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                corrected.replace("{}", lookalike),
                "{context}"
            );
        }
        for (observer, data) in LISTED {
            // Plain LF, and BOM with CRLF line ends.
            for bom_crlf in [false, true] {
                let tail = if data { "__END__\ndata\n" } else { "" };
                let source = format!("{}{tail}", template.replace("{}", observer));
                let source = if bom_crlf {
                    format!("\u{feff}{}", source.replace('\n', "\r\n"))
                } else {
                    source
                };
                fs::write(&path, &source).unwrap();
                let before = identity(&path);
                let out = cli(dir.path(), &["--fix", "--only", cop, file]);
                let context = format!("{cop}: {observer:?}: {bom_crlf}: {out:?}");
                assert!(out.stderr.is_empty(), "{context}");
                assert_eq!(out.status.code(), Some(1), "{context}");
                assert_eq!(cops(&out), [cop], "{context}");
                assert_eq!(applied(&out), 0, "{context}");
                assert_untouched(&path, &before, &context);
            }
        }
    }
}

/// The default selection across departments: a listed observer holds every
/// edit of every cop together, findings and exit status are kept; without it
/// the same file is corrected (`Lint/AmbiguousOperatorPrecedence` keeps its
/// finding there: it is autocorrect-incompatible with the selected
/// `Style/RedundantParentheses`).
#[test]
fn default_selection_holds_all_departments_together_for_a_listed_observer() {
    let dir = project("");
    let path = dir.path().join("x.rb");
    let body = "y = 'a'  \np 1 + 2 * 3\ndef m(&block) = n(&block)\n";
    for observer in ["p __FILE__\n", "SCRIPT_LINES__ = {}\n", "p $0\n"] {
        fs::write(&path, format!("{observer}{body}")).unwrap();
        let before = identity(&path);
        let out = cli(dir.path(), &["--fix", "x.rb"]);
        let context = format!("{observer:?}: {out:?}");
        assert_eq!(out.status.code(), Some(1), "{context}");
        assert_eq!(applied(&out), 0, "{context}");
        for cop in [
            "Layout/TrailingWhitespace",
            "Lint/AmbiguousOperatorPrecedence",
            "Naming/BlockForwarding",
            "Style/StringLiterals",
        ] {
            assert!(cops(&out).iter().any(|c| c == cop), "{cop}: {context}");
        }
        assert_untouched(&path, &before, &context);
    }
    fs::write(&path, format!("p 1\n{body}")).unwrap();
    let out = cli(dir.path(), &["--fix", "x.rb"]);
    assert!(applied(&out) > 0, "{out:?}");
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("y = \"a\"\n"), "{after}");
    assert!(after.contains("def m(&) = n(&)\n"), "{after}");
}

/// `(cop, file, source, finding line, default-on)`: one cop per department.
const DEPARTMENTS: [(&str, &str, &str, u32, bool); 10] = [
    (
        "Bundler/DuplicatedGem",
        "Gemfile",
        "gem 'a'\ngem 'a'\n",
        2,
        true,
    ),
    ("Bundler/GemComment", "Gemfile", "gem 'a'\n", 1, false),
    (
        "Gemspec/DuplicatedAssignment",
        "x.gemspec",
        "Gem::Specification.new do |spec|\n  spec.name = 'a'\n  spec.name = 'b'\nend\n",
        3,
        true,
    ),
    ("Layout/TrailingWhitespace", "x.rb", "y = 1 \n", 1, true),
    ("Lint/Debugger", "x.rb", "binding.pry\n", 1, true),
    (
        "Metrics/ParameterLists",
        "x.rb",
        "def m(a, b, c, d, e, f)\nend\n",
        1,
        false,
    ),
    (
        "Migration/DepartmentName",
        "x.rb",
        "# rubocop:disable Debugger\np 1\n",
        1,
        true,
    ),
    ("Naming/MethodName", "x.rb", "def fooBar\nend\n", 1, true),
    ("Security/Eval", "x.rb", "eval(y)\n", 1, true),
    ("Style/StringLiterals", "x.rb", "y = 'a'\n", 1, true),
];

fn reports(out: &Output, cop: &str, line: u32) -> bool {
    findings(out).iter().any(|(l, _, c)| c == cop && *l == line)
}

#[test]
fn each_department_honors_defaults_selection_strict_toml_and_directives() {
    for (cop, file, source, line, default_on) in DEPARTMENTS {
        let department = cop.split_once('/').unwrap().0;
        let dir = project("");
        let path = dir.path().join(file);
        fs::write(&path, source).unwrap();
        // Default-on versus explicit opt-in.
        let out = cli(dir.path(), &[file]);
        assert_eq!(reports(&out, cop, line), default_on, "{cop}: {out:?}");
        // `--only` force-enables the cop and the department; `--except` wins.
        for only in [cop, department] {
            let out = cli(dir.path(), &["--only", only, file]);
            assert!(reports(&out, cop, line), "{cop}: --only {only}: {out:?}");
            assert_eq!(out.status.code(), Some(1), "{cop}: {out:?}");
        }
        let out = cli(dir.path(), &["--only", cop, "--except", cop, file]);
        assert!(!reports(&out, cop, line), "{cop}: {out:?}");
        // Strict TOML: per-cop and department enablement; unknown keys fail
        // before linting.
        for (config, expected) in [
            (format!("[cops.\"{cop}\"]\nenabled = true\n"), true),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), false),
            (format!("[cops.\"{department}\"]\nenabled = false\n"), false),
        ] {
            fs::write(dir.path().join("rustocop.toml"), &config).unwrap();
            let out = cli(dir.path(), &[file]);
            assert_eq!(reports(&out, cop, line), expected, "{cop}: {config}");
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nstyle = \"x\"\n"),
        )
        .unwrap();
        let out = cli(dir.path(), &["--fix", file]);
        assert_eq!(out.status.code(), Some(2), "{cop}: {out:?}");
        assert!(findings(&out).is_empty(), "{cop}: {out:?}");
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        // Both directive prefixes suppress the cop for the file (Migration's
        // own directive is the finding, so it is checked by `--except`).
        if cop != "Migration/DepartmentName" {
            for prefix in ["rubocop", "rustocop"] {
                fs::write(&path, format!("# {prefix}:disable {cop}\n{source}")).unwrap();
                let out = cli(dir.path(), &["--only", cop, file]);
                assert!(!reports(&out, cop, line + 1), "{cop}: {prefix}: {out:?}");
            }
            fs::write(&path, source).unwrap();
        }
        // Reporting never writes; supplied stdin reports the same finding.
        let before = identity(&path);
        let out = cli(dir.path(), &["--only", cop, file]);
        assert!(reports(&out, cop, line), "{cop}: {out:?}");
        assert_untouched(&path, &before, cop);
        let out = stdin(
            dir.path(),
            &["--only", cop, "--stdin", file],
            source.as_bytes(),
        );
        assert!(reports(&out, cop, line), "{cop}: stdin: {out:?}");
    }
}

/// A parse error writes nothing and reports no ordinary finding, while a
/// valid peer in the same run is still corrected atomically; detection-only
/// departments never write.
#[test]
fn parse_errors_never_write_and_valid_peers_continue_across_departments() {
    let dir = project("");
    let broken = dir.path().join("broken.rb");
    let peer = dir.path().join("peer.rb");
    let detection = dir.path().join("detection.rb");
    fs::write(&broken, "y = 'a'  \ndef (\n").unwrap();
    fs::write(&peer, "y = 'a'  \n").unwrap();
    fs::write(
        &detection,
        "eval(y)\ndef m(a, b, c, d, e, f)\nend\nbinding.pry\n",
    )
    .unwrap();
    let broken_before = identity(&broken);
    let detection_before = identity(&detection);
    let out = cli(
        dir.path(),
        &["--fix", "broken.rb", "peer.rb", "detection.rb"],
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_untouched(&broken, &broken_before, "broken");
    assert_untouched(&detection, &detection_before, "detection");
    assert_eq!(fs::read_to_string(&peer).unwrap(), "y = \"a\"\n");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        !text
            .lines()
            .any(|l| l.starts_with("broken.rb:") && l.contains("Style/StringLiterals")),
        "{text}"
    );
    for cop in [
        "Security/Eval",
        "Naming/MethodParameterName",
        "Lint/Debugger",
    ] {
        assert!(cops(&out).iter().any(|c| c == cop), "{cop}: {text}");
    }
}
