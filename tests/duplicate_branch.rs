//! exact pinned source-only parity and rebuilt CLI no-write controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/DuplicateBranch";
const SOURCE: &[u8] = b"if x\n  work\nelse\n  work\nend\n";
fn rows() -> Vec<serde_json::Value> {
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_branch.json")).unwrap();
    assert_eq!(rows.len(), 546);
    let structure: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_branch_structure.json")).unwrap();
    assert_eq!(structure.len(), 97);
    rows.extend(structure);
    let review: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_branch_review2.json")).unwrap();
    assert_eq!(review.len(), 51);
    rows.extend(review);
    rows
}
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(code) = code {
        assert_eq!(out.status.code(), Some(code), "{out:?}\n{source:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
    out
}
#[test]
fn every_reference_row_matches_rebuilt_cli_with_and_without_fix_and_never_writes() {
    let dir = project();
    assert_eq!(rows().len(), 694);
    for row in rows() {
        let source = row["source"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &args,
                Some(i32::from(!findings.is_empty())),
            );
            assert_eq!(count(&out), findings.len(), "{row}\n{out:?}");
            for f in findings {
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains(&format!(
                        "lib/a.rb:{}:{}: E: {} ({COP})",
                        f[0],
                        f[1],
                        f[3].as_str().unwrap()
                    )),
                    "{row}\n{out:?}"
                );
            }
        }
    }
}
#[test]
fn defaults_metadata_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 491);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && !cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    let dir = project();
    for (config, selectors, n) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint"], 1),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            1,
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(dir.path(), SOURCE, &args, None)), n);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 2.7",
        "IgnoreLiteralBranches = true",
        "IgnoreConstantBranches = true",
        "IgnoreDuplicateElseBranch = true",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    for config in [
        "exclude = ['lib/**']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        assert_eq!(
            count(&unchanged(
                dir.path(),
                SOURCE,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0)
            )),
            0
        );
    }
}
#[test]
fn both_prefixes_multiline_owner_range_next_and_push_pop_scopes() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\nif x; work; else; work; end\n"),
            format!("# {prefix}:push -{COP}\nif x; work; else; work; end\n# {prefix}:pop\n"),
            format!("# {prefix}:disable-next {COP}\nif x; work; else; work; end\n"),
            format!("# {prefix}:next -{COP}\nif x; work; else; work; end\n"),
            format!("if x; work; else; work; end # {prefix}:disable {COP}\n"),
            // Elsif's full range extends through later branches, not closing end.
            format!("if x\nwork\nelsif y\nwork # {prefix}:disable {COP}\nend\n"),
            format!("if x\nwork\nelsif y\nwork\nelse\nother # {prefix}:disable {COP}\nend\n"),
            format!("case x\nwhen 1; work\nwhen 2\nwork # {prefix}:disable {COP}\nend\n"),
            format!(
                "begin\nwork\nrescue A; handle\nrescue B\nhandle # {prefix}:disable {COP}\nend\n"
            ),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        for source in [
            // Else reports its keyword alone, not its body.
            format!("if x\nwork\nelse\nwork # {prefix}:disable {COP}\nend\n"),
            format!("if x\nwork\nelsif y\nwork\nend # {prefix}:disable {COP}\n"),
            format!(
                "# {prefix}:disable {COP}\nif x; work; else; work; end\n# {prefix}:push +{COP}\nif x; work; else; work; end\n# {prefix}:pop\nif x; work; else; work; end\n"
            ),
            format!(
                "# {prefix}:disable {COP}\n# {prefix}:enable-next {COP}\nif x; work; else; work; end\nif x; work; else; work; end\n"
            ),
        ] {
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(1)
                )),
                1
            );
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\nif x; work; else; work; end\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["lib/a.rb"],
                None
            )),
            1
        );
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn supplied_paths_empty_fixed_path_policy_stdin_provenance_and_rejected_stdin_fix() {
    let dir = project();
    for path in ["lib/a.arb", "lib/a.jb", "Rakefile", "lib/.hidden.rb"] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", path]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(count(&out), 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:3:1")));
        assert_eq!(fs::read(dir.path().join(path)).unwrap(), SOURCE);
    }
    for directory in ["vendor", "node_modules", "db"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        let file = if directory == "db" {
            "schema.rb"
        } else {
            "a.rb"
        };
        let path = dir.path().join(directory).join(file);
        fs::write(&path, SOURCE).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", directory])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(fs::read(path).unwrap(), SOURCE);
    }
    for config in ["", "exclude = ['virtual/**']\n"] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "virtual/main.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        let n = usize::from(config.is_empty());
        assert_eq!(out.status.code(), Some(n as i32));
        assert_eq!(count(&out), n);
        if n == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/main.rb:3:1"));
        }
        assert!(!dir.path().join("virtual/main.rb").exists());
    }
    unchanged(
        dir.path(),
        SOURCE,
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
}
#[test]
fn source_file_value_uses_supplied_file_and_stdin_paths_not_display_or_realpath() {
    let dir = project();
    let absolute = dir.path().join("lib/a.rb").to_str().unwrap().to_owned();
    for path in [
        "lib/a.rb",
        "./lib/a.rb",
        absolute.as_str(),
        "virtual/example.rb",
    ] {
        for matches in [true, false] {
            let value = if matches { path } else { "different.rb" };
            let source = format!("if x\n  [__FILE__]\nelse\n  [{value:?}]\nend\n");
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", COP, "--stdin", path])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.status.code(), Some(i32::from(matches)), "{out:?}");
            assert_eq!(count(&out), usize::from(matches), "{source}\n{out:?}");
            if matches {
                assert!(
                    String::from_utf8_lossy(&out.stdout)
                        .contains(":3:1: E: Duplicate branch body detected.")
                );
            }
            if path != "virtual/example.rb" {
                for fix in [false, true] {
                    let mut args = vec!["--only", COP];
                    if fix {
                        args.push("--fix");
                    }
                    args.push(path);
                    let out = unchanged(
                        dir.path(),
                        source.as_bytes(),
                        &args,
                        Some(i32::from(matches)),
                    );
                    assert_eq!(count(&out), usize::from(matches), "{source}\n{out:?}");
                }
            }
        }
    }
    assert!(!dir.path().join("virtual").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
        let source = b"if x\n  __FILE__\nelse\n  'lib/alias.rb'\nend\n";
        let out = unchanged(
            dir.path(),
            source,
            &["--only", COP, "--fix", "lib/alias.rb"],
            Some(1),
        );
        assert_eq!(count(&out), 1);
        assert!(
            fs::symlink_metadata(dir.path().join("lib/alias.rb"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[test]
fn source_envelopes_literals_comments_and_runtime_observers_are_no_write() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(
            b"if x\r\n  work # first\r\nelse # duplicate\r\n  work # second\r\nend\r\n",
        );
        unchanged(
            dir.path(),
            &source,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    for observer in [
        "p __LINE__",
        "File.read(__FILE__)",
        "IO.read($0)",
        "RubyVM::InstructionSequence.of(method(:f))",
        "RubyVM::AbstractSyntaxTree.parse_file(__FILE__)",
        "ObjectSpace.each_object {}",
        "GC.stat",
        "binding.local_variables",
        "TracePoint.new(:line) {}.enable",
        "__END__\ndata\n",
        "DATA.read\n__END__\ndata\n",
        "eval('x = 1')",
        "<<~DOC\n#{if x; work; else; work; end}\nDOC\n",
    ] {
        let mut source = SOURCE.to_vec();
        source.extend_from_slice(observer.as_bytes());
        assert!(
            count(&unchanged(
                dir.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )) >= 1
        );
    }
}
#[cfg(unix)]
#[test]
fn shared_atomic_validation_readonly_symlink_failure_and_peer_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, SOURCE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for alias in ["lib/a.rb", "lib/link.rb"] {
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", alias])
                .status
                .code(),
            Some(1)
        );
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap(), old.permissions()),
            (new.ino(), new.modified().unwrap(), new.permissions())
        );
        assert_eq!(fs::read(&path).unwrap(), SOURCE);
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    for source in [
        b"if x; work; else; work; end\n(\n".to_vec(),
        format!("if x; {}; else; work; end\n", "(".repeat(80)).into_bytes(),
    ] {
        assert_eq!(
            count(&unchanged(
                dir.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(2)
            )),
            0
        );
    }
    let before = b"if x; work(1,2); else; work(1,2); end\n";
    let after = b"if x; work(1, 2); else; work(1, 2); end\n";
    let only = "Lint/DuplicateBranch,Layout/SpaceAfterComma";
    unchanged(dir.path(), before, &["--only", only, "lib/a.rb"], Some(1));
    let inode = fs::metadata(&path).unwrap().ino();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/link.rb"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&path).unwrap(), after);
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(dir.path().join("lib/link.rb"))
            .unwrap()
            .is_symlink()
    );
    unchanged(
        dir.path(),
        after,
        &["--only", only, "--fix", "lib/a.rb"],
        Some(1),
    );
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/link.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(count(&out), 1);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/dangling.rb"])
            .status
            .code(),
        Some(2)
    );
    assert!(!dir.path().join("lib/missing.rb").exists());
}
#[test]
fn all_reference_inputs_are_native_syntax_checked_only_never_executed() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-c", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(row["source"].as_str().unwrap().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
    }
}
