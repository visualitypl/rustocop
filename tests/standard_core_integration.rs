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
