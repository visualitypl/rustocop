//! Pinned source-only fixtures and rebuilt CLI no-write/selection controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/DuplicateRequire";
const SOURCE: &[u8] = b"require 'x'\nrequire 'x'\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/duplicate_require.json")).unwrap()
}
fn escaped_byte_rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!(
        "fixtures/duplicate_require_escaped_bytes.json"
    ))
    .unwrap()
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
fn every_pinned_spec_source_and_control_matches_cli_and_fix_never_writes() {
    let dir = project();
    assert_eq!(rows().len(), 279);
    for row in rows() {
        let source = row["source"].as_str().unwrap();
        let expected = row["findings"].as_array().unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &args,
                Some(i32::from(!expected.is_empty())),
            );
            assert_eq!(count(&out), expected.len(), "{row}\n{out:?}");
            for f in expected {
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
fn encoded_literal_argument_keys_match_reference_without_writes() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_require_bytes.json")).unwrap();
    let dir = project();
    for row in rows {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            let expected = row["findings"].as_array().unwrap();
            let out = unchanged(
                dir.path(),
                &source,
                &args,
                Some(i32::from(!expected.is_empty())),
            );
            assert_eq!(count(&out), expected.len(), "{row}\n{out:?}");
            for f in expected {
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
fn escaped_source_glyphs_and_neighbours_match_reference_without_writes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    }
    for row in escaped_byte_rows() {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let expected = row["findings"].as_array().unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            // Also retain an open FD across the request: an atomic replacement
            // cannot conceal writes behind unchanged pathname contents.
            let mut fd = fs::File::open(&path).unwrap();
            let out = unchanged(
                dir.path(),
                &source,
                &args,
                Some(i32::from(!expected.is_empty())),
            );
            use std::io::{Read, Seek};
            fd.rewind().unwrap();
            let mut bytes = Vec::new();
            fd.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, source, "{row}");
            let actual: Vec<_> = String::from_utf8(out.stdout)
                .unwrap()
                .lines()
                .filter(|line| line.ends_with(&format!("({COP})")))
                .map(str::to_owned)
                .collect();
            let expected: Vec<_> = expected
                .iter()
                .map(|f| {
                    format!(
                        "lib/a.rb:{}:{}: E: {} ({COP})",
                        f[0],
                        f[1],
                        f[3].as_str().unwrap()
                    )
                })
                .collect();
            assert_eq!(actual, expected, "{row}");
        }
    }
}

#[test]
fn defaults_metadata_selectors_strict_toml_precedence_and_excludes() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 491);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && !cop.safe_fixes());
    assert!(cop.ruby_versions().minimum.is_none() && cop.ruby_versions().maximum.is_none());
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
        "EnforcedStyle = 'global'",
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
        unchanged(
            dir.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn both_prefixes_multiline_ranges_next_push_and_suppressed_first_calls() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\nrequire 'x'\nrequire 'x'\n"),
            format!("# {prefix}:push -{COP}\nrequire 'x'\nrequire 'x'\n# {prefix}:pop\n"),
            format!("require 'x'\n# {prefix}:disable-next {COP}\nrequire 'x'\n"),
            format!("require 'x'\n# {prefix}:next -{COP}\nrequire 'x'\n"),
            format!("require 'x'\nrequire(\n 'x' # {prefix}:disable {COP}\n)\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        for source in [
            format!(
                "# {prefix}:disable {COP}\nrequire 'x'\n# {prefix}:enable {COP}\nrequire 'x'\n"
            ),
            format!(
                "# {prefix}:disable {COP}\nrequire 'x'\n# {prefix}:push +{COP}\nrequire 'x'\n# {prefix}:pop\nrequire 'x'\n"
            ),
            format!(
                "# {prefix}:disable {COP}\nrequire 'x'\n# {prefix}:enable-next {COP}\nrequire 'x'\nrequire 'x'\n"
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
        let source = format!("# {prefix}:enable {COP}\nrequire 'x'\nrequire 'x'\n");
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
fn supplied_paths_stdin_no_filesystem_resolution_and_no_stdin_fix() {
    let dir = project();
    let absolute = dir.path().join("lib/a.rb");
    for path in [
        "lib/a.rb",
        "./lib/a.rb",
        absolute.to_str().unwrap(),
        "lib/a.arb",
        "lib/a.jb",
        "Rakefile",
        "lib/.hidden.rb",
    ] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", path]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(count(&out), 1);
        assert_eq!(fs::read(dir.path().join(path)).unwrap(), SOURCE);
    }
    for directory in ["vendor", "node_modules", "db"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        let path = dir.path().join(directory).join(if directory == "db" {
            "schema.rb"
        } else {
            "a.rb"
        });
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
        assert!(!dir.path().join("virtual/main.rb").exists());
    }
    unchanged(
        dir.path(),
        SOURCE,
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
    // Existing and nonexistent feature names are equally syntactic; never load them.
    fs::write(dir.path().join("lib/x.rb"), "this is not Ruby").unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    unchanged(
        dir.path(),
        SOURCE,
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(1),
    );
}
#[test]
fn encoding_bom_crlf_observers_and_unsafe_control_flow_keep_bytes_and_findings() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(b"require 'x'\r\nrequire 'x'\r\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
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
        "DATA.read\n__END__\ndata\n",
        "__END__\ndata\n",
        "<<~DOC\n#{require 'x'; require 'x'}\nDOC\n",
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
fn atomic_validation_readonly_symlink_failure_and_layout_peer_composition() {
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
        b"require 'x'\nrequire 'x'\n(\n".to_vec(),
        format!("{}\n", "(".repeat(80)).into_bytes(),
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
    let before = b"require([1,2])\nrequire([1,2])\n";
    let after = b"require([1, 2])\nrequire([1, 2])\n";
    let only = "Lint/DuplicateRequire,Layout/SpaceAfterComma";
    unchanged(dir.path(), before, &["--only", only, "lib/a.rb"], Some(1));
    let inode = fs::metadata(&path).unwrap().ino();
    let out = run(dir.path(), &["--only", only, "--fix", "lib/link.rb"]);
    assert_eq!(out.status.code(), Some(1));
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
    assert_eq!(
        run(dir.path(), &["--only", only, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(2)
    );
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
fn native(ruby: &str, source: &[u8], args: &[&str]) -> Output {
    let mut child = Command::new(ruby)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn wide_siblings_and_multiple_input_lifecycles_are_file_local() {
    let dir = project();
    let source = "require 'x'\n".repeat(6000);
    assert_eq!(
        count(&unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1)
        )),
        5999
    );
    fs::write(dir.path().join("lib/a.rb"), "require 'x'\n").unwrap();
    fs::write(dir.path().join("lib/b.rb"), "require 'x'\n").unwrap();
    fs::write(dir.path().join("lib/empty.rb"), "").unwrap();
    let out = run(dir.path(), &["--only", COP, "--fix", "lib"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(count(&out), 0);
}

#[test]
fn all_reference_sources_are_native_syntax_checked_only() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let bytes: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_require_bytes.json")).unwrap();
    let sources = rows()
        .into_iter()
        .map(|r| r["source"].as_str().unwrap().as_bytes().to_vec())
        .chain(
            bytes
                .into_iter()
                .chain(escaped_byte_rows())
                .map(|r| serde_json::from_value::<Vec<u8>>(r["bytes"].clone()).unwrap()),
        );
    for source in sources {
        let out = native(&ruby, &source, &["--parser=prism", "-c", "-"]);
        assert!(out.status.success(), "{source:?}\n{out:?}");
    }
}
#[test]
fn authored_native_require_reload_argument_and_error_counterexamples_prove_no_edit_policy() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    let feature = dir.path().join("feature.rb");
    // Only this authored inert fixture is executed, never reference or corpus files.
    fs::write(&feature, "$events << :loaded\n").unwrap();
    for (before, after, expected) in [
        (
            "$events = []\npath = ARGV[0]\nrequire path\n$LOADED_FEATURES.delete(path)\nrequire path\np $events\n",
            "$events = []\npath = ARGV[0]\nrequire path\n$LOADED_FEATURES.delete(path)\np $events\n",
            "[:loaded, :loaded]\n",
        ),
        (
            "$events = []\ndef path; $events << :arg; ARGV[0]; end\nrequire path\nrequire path\np $events\n",
            "$events = []\ndef path; $events << :arg; ARGV[0]; end\nrequire path\np $events\n",
            "[:arg, :loaded, :arg]\n",
        ),
        (
            "$events = []\ndef path; raise 'second' unless $events.empty?; ARGV[0]; end\nrequire path\nrequire path\n",
            "$events = []\ndef path; raise 'second' unless $events.empty?; ARGV[0]; end\nrequire path\n",
            "",
        ),
    ] {
        let out = unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert_eq!(count(&out), 1);
        let row = rows().into_iter().find(|r| r["source"] == before).unwrap();
        assert_eq!(row["upstream_after"], after);
        let a = native(
            &ruby,
            before.as_bytes(),
            &["--parser=prism", "-", feature.to_str().unwrap()],
        );
        let b = native(
            &ruby,
            after.as_bytes(),
            &["--parser=prism", "-", feature.to_str().unwrap()],
        );
        assert_eq!(a.stdout, expected.as_bytes());
        assert_eq!(a.status.success(), !expected.is_empty(), "{a:?}");
        if expected.is_empty() {
            assert!(String::from_utf8_lossy(&a.stderr).contains("second"));
        }
        assert!(b.status.success(), "{b:?}");
        assert_ne!((a.status.code(), a.stdout), (b.status.code(), b.stdout));
    }
}
