//! pinned source-only fixtures, actual CLI and authored native witnesses.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/DeprecatedConstants";
const SAFE: &[u8] = b"NIL = nil\n[::NIL]\n";
const AFTER: &[u8] = b"NIL = nil\n[nil]\n";
const UNCERTAIN: &[u8] = b"NIL\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/deprecated_constants.json")).unwrap()
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
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(out.status.code(), Some(status), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}
#[test]
fn complete_fixtures_have_exact_cli_diagnostics_and_only_proven_canonical_fixes() {
    let dir = project();
    let mut fixed = 0;
    for row in rows() {
        let source = row["source"].as_str().unwrap();
        let expected = row["findings"].as_array().unwrap();
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "lib/a.rb"],
            Some(i32::from(!expected.is_empty())),
        );
        assert_eq!(count(&out), expected.len(), "{row}\n{out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        for finding in expected {
            assert!(
                text.contains(&format!(
                    "lib/a.rb:{}:{}: E: {} ({COP})",
                    finding[0],
                    finding[1],
                    finding[3].as_str().unwrap()
                )),
                "{row}\n{text}"
            );
        }
        let after = row["project_after"].as_str().unwrap();
        if after == source {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(i32::from(!expected.is_empty())),
            );
        } else {
            fixed += 1;
            let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{row}\n{out:?}");
            assert_eq!(
                fs::read(dir.path().join("lib/a.rb")).unwrap(),
                after.as_bytes()
            );
            unchanged(
                dir.path(),
                after.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
    assert_eq!(fixed, 12);
}
#[test]
fn metadata_selectors_toml_department_precedence_and_exclusions() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 491);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(registry.has_cop("Lint/UnusedPrivateMethod"));
    let dir = project();
    for (config, selectors, expected) in [
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
        assert_eq!(count(&unchanged(dir.path(), SAFE, &args, None)), expected);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 2.6",
        "DeprecatedConstants = {}",
        "SafeAutoCorrect = false",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SAFE,
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
                SAFE,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0)
            )),
            0
        );
    }
}
#[test]
fn both_directive_prefixes_multiline_next_push_and_source_activation() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\nNIL = nil\n::NIL\n"),
            format!("NIL = nil\n# {prefix}:push -{COP}\n::NIL\n# {prefix}:pop\n"),
            format!("NIL = nil\n# {prefix}:disable-next {COP}\n::NIL\n"),
            format!("NIL = nil\n# {prefix}:next -{COP}\n::NIL\n"),
            format!("NIL = nil\n::NIL # {prefix}:disable {COP}\n"),
            // Even a multiline nonmatching path's scope child is checked.
            format!("NIL::\n Other # {prefix}:disable {COP}\n"),
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(if source.starts_with("NIL::") { 1 } else { 0 }),
            );
            assert_eq!(count(&out), usize::from(source.starts_with("NIL::")));
        }
        for (open, close) in [
            (
                format!("# {prefix}:push +{COP}\n"),
                format!("# {prefix}:pop\n"),
            ),
            (format!("# {prefix}:enable-next {COP}\n"), String::new()),
        ] {
            let source = format!("# {prefix}:disable {COP}\nNIL\n{open}NIL\n{close}NIL\n");
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1),
            );
            assert_eq!(count(&out), 1);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\nNIL\n");
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
fn supplied_paths_stdin_and_builtin_exclusions() {
    let dir = project();
    for path in ["lib/a.arb", "lib/a.jb", "Rakefile", "lib/.hidden.rb"] {
        fs::write(dir.path().join(path), UNCERTAIN).unwrap();
        let out = run(dir.path(), &["--only", COP, path]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(count(&out), 1);
    }
    for directory in ["vendor", "node_modules"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        fs::write(dir.path().join(directory).join("a.rb"), SAFE).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", directory])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(
            fs::read(dir.path().join(directory).join("a.rb")).unwrap(),
            SAFE
        );
    }
    for (config, expected) in [(String::new(), 1), ("exclude = ['virtual/**']\n".into(), 0)] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "virtual/main.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SAFE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(expected as i32), "{out:?}");
        assert_eq!(count(&out), expected);
        if expected == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/main.rb:2:2"));
        }
    }
    unchanged(
        dir.path(),
        SAFE,
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
}
#[test]
fn canonical_byte_envelopes_preserve_comments_encoding_bom_and_crlf() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(b"# alias\r\nNIL = nil # binding\r\n[::NIL] # read\r\n");
        let mut after = prefix.to_vec();
        after.extend_from_slice(b"# alias\r\nNIL = nil # binding\r\n[nil] # read\r\n");
        unchanged(dir.path(), &source, &["--only", COP, "lib/a.rb"], Some(1));
        let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), after);
    }
}
#[cfg(unix)]
#[test]
fn atomic_symlink_permission_invalid_failure_and_peer_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, UNCERTAIN).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for name in ["lib/a.rb", "lib/link.rb"] {
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", name])
                .status
                .code(),
            Some(1)
        );
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.ino(), new.ino());
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let failed = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
    assert_eq!(failed.status.code(), Some(2), "{failed:?}");
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        old.modified().unwrap()
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions(),
        old.permissions()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    unchanged(
        dir.path(),
        b"NIL = nil\n::NIL\n(\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    fs::write(&path, SAFE).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(fs::read(&path).unwrap(), AFTER);
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
    // Independent nonoverlapping style edit composes in the same snapshot.
    fs::write(&path, b"NIL=nil;[::NIL]\n").unwrap();
    assert_eq!(
        run(
            dir.path(),
            &[
                "--only",
                "Lint/DeprecatedConstants,Layout/SpaceAroundOperators",
                "--fix",
                "lib/link.rb"
            ]
        )
        .status
        .code(),
        Some(0)
    );
    assert_eq!(fs::read(&path).unwrap(), b"NIL = nil;[nil]\n");
    // An uncertain read still reports when another department corrects.
    fs::write(&path, b"NIL\np [1,2]\n").unwrap();
    assert_eq!(
        run(
            dir.path(),
            &[
                "--only",
                "Lint/DeprecatedConstants,Layout/SpaceAfterComma",
                "--fix",
                "lib/a.rb"
            ]
        )
        .status
        .code(),
        Some(1)
    );
    assert_eq!(fs::read(&path).unwrap(), b"NIL\np [1, 2]\n");
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
    // Existing main writer rejects replacing a dangling source link.
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/dangling.rb"])
            .status
            .code(),
        Some(2)
    );
    assert!(!dir.path().join("lib/missing.rb").exists());
}
fn native(ruby: &str, source: &str) -> Output {
    Command::new(ruby)
        .args(["--parser=prism", "-w", "-e", source])
        .output()
        .unwrap()
}
#[test]
fn native_closed_alias_values_bindings_constants_and_semantic_counterexamples() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    for (name, value) in [("NIL", "nil"), ("TRUE", "true"), ("FALSE", "false")] {
        for root in ["", "::"] {
            let before = format!("{name} = {value}\n[{root}{name}]\n");
            fs::write(dir.path().join("lib/a.rb"), &before).unwrap();
            assert_eq!(
                run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"])
                    .status
                    .code(),
                Some(0)
            );
            let after = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
            assert_eq!(after, format!("{name} = {value}\n[{value}]\n"));
            let plain_before = native(&ruby, &before);
            let plain_after = native(&ruby, &after);
            assert!(plain_before.status.success() && plain_after.status.success());
            assert_eq!(plain_before.stdout, plain_after.stdout);
            assert_eq!(plain_before.stderr, plain_after.stderr);
            // Authored observation harness only, never reference/corpus execution.
            let observe = |body: &str| {
                format!(
                    "p(begin\n{body}end); p [Object.const_get(:{name}), Object.const_defined?(:{name}, false), Object.constants(false).include?(:{name}), defined?(::{name}), TOPLEVEL_BINDING.local_variables]\n"
                )
            };
            let a = native(&ruby, &observe(&before));
            let b = native(&ruby, &observe(&after));
            assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
            assert_eq!(a.stdout, b.stdout);
            assert_eq!(a.stderr, b.stderr);
            assert!(String::from_utf8_lossy(&a.stdout).starts_with(&format!(
                "[{value}]\n[{value}, true, true, \"constant\", []]"
            )));
        }
    }
    let void_before = native(&ruby, "NIL = nil; ::NIL");
    let void_after = native(&ruby, "NIL = nil; nil");
    assert!(void_before.status.success() && void_after.status.success());
    assert_ne!(void_before.stderr, void_after.stderr); // -w warning text changes.
    for (source, expected) in [
        ("NIL = nil; ::NIL", ""),
        (
            "begin; NIL; rescue NameError; puts :missing; end",
            "missing\n",
        ),
        (
            "begin; Random::DEFAULT; rescue NameError; puts :missing; end; puts Random.new.class",
            "missing\nRandom\n",
        ),
        ("NIL = 1; p ::NIL", "1\n"),
        (
            "NIL = nil; Object.private_constant(:NIL); begin; ::NIL; rescue NameError; puts :private; end",
            "private\n",
        ),
        (
            "def Object.const_added(name); puts name; end; NIL = nil; p ::NIL",
            "NIL\nnil\n",
        ),
        (
            "NIL = nil; Object.send(:remove_const, :NIL); begin; ::NIL; rescue NameError; puts :removed; end",
            "removed\n",
        ),
    ] {
        let found = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert!(count(&found) > 0);
        let out = native(&ruby, source);
        assert!(out.status.success(), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    }
}
#[test]
fn all_reference_and_corrected_sources_are_native_syntax_only() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        for key in ["source", "upstream_after", "project_after"] {
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
                .write_all(row[key].as_str().unwrap().as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                key != "upstream_after" || row["upstream_valid_syntax"].as_bool().unwrap(),
                "{row}\n{key}\n{out:?}"
            );
        }
    }
}
