//! Source-only reference/pipeline/actual CLI controls. Never execute inputs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/UselessDefaultValueArgument";
const SOURCE: &[u8] = b"x.fetch(key, default) { it }\n";
fn rows() -> Vec<serde_json::Value> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/useless_default_value_argument.json")).unwrap();
    assert_eq!(rows.len(), 3029);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("parser_rejection").is_some())
            .count(),
        3
    );
    rows
}
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
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
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({COP})"))
        .count()
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    }
    let mut held = fs::File::open(&path).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(status) = status {
        assert_eq!(
            out.status.code(),
            Some(status),
            "{}: {out:?}",
            String::from_utf8_lossy(bytes)
        );
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let mut held_bytes = Vec::new();
    held.read_to_end(&mut held_bytes).unwrap();
    assert_eq!(held_bytes, bytes);
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
fn every_original_byte_fixture_matches_rebuilt_cli_lint_and_fix_no_write() {
    parallel_rows!(for row in rows() => {
        let d = project();
        let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            if row.get("parser_rejection").is_some() {
                assert_eq!(count(&unchanged(d.path(), &bytes, &args, Some(2))), 0);
                continue;
            }
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            let out = unchanged(
                d.path(),
                &bytes,
                &args,
                Some(i32::from(!expected.is_empty())),
            );
            let actual: Vec<_> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| l.ends_with(&format!("({COP})")))
                .map(str::to_owned)
                .collect();
            let want: Vec<_> = expected
                .iter()
                .map(|(l, c, _, m)| format!("lib/a.rb:{l}:{c}: E: {m} ({COP})"))
                .collect();
            assert_eq!(actual, want, "{row}");
            assert!(out.stderr.is_empty(), "{row}: {out:?}");
            if fix {
                assert!(String::from_utf8_lossy(&out.stdout).contains("0 corrections applied"));
            }
        }
    });
}
#[test]
fn defaults_selectors_department_precedence_and_strict_toml() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    let d = project();
    for (conf, mut args, n) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint", "--except", COP], 0),
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
        (
            format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
            vec!["--only", COP],
            0,
        ),
        ("exclude = ['lib/a.rb']\n".into(), vec!["--only", COP], 0),
    ] {
        fs::write(d.path().join("rustocop.toml"), conf).unwrap();
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(d.path(), SOURCE, &args, None)), n);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "TargetRubyVersion = 3.3",
        "SafeAutoCorrect = true",
        "AllowedReceivers = ['Rails.cache']",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            SOURCE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
}
#[test]
fn file_stdin_path_provenance_exclusions_and_stdin_fix_rejection() {
    let d = project();
    for conf in [
        String::new(),
        "exclude = ['lib/a.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
    ] {
        let n = usize::from(conf.is_empty());
        fs::write(d.path().join("rustocop.toml"), conf).unwrap();
        assert_eq!(
            count(&unchanged(
                d.path(),
                SOURCE,
                &["--only", COP, "lib/a.rb"],
                Some(n as i32)
            )),
            n
        );
        assert_eq!(
            count(&run(&d.path().join("lib"), &["--only", COP, "a.rb"])),
            n
        );
        fs::write(d.path().join("lib/a.rb"), b"sentinel\n").unwrap();
        let old = fs::metadata(d.path().join("lib/a.rb")).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), n);
        if n != 0 {
            assert!(String::from_utf8_lossy(&out.stdout).starts_with("lib/a.rb:"));
        }
        assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), b"sentinel\n");
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(d.path().join("lib/a.rb"))
                .unwrap()
                .modified()
                .unwrap()
        );
    }
    assert_eq!(
        run(d.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for (path, n) in [
        ("Gemfile", 1),
        ("lib/a.jb", 1),
        ("lib/.hidden.rb", 1),
        ("lib/a_spec.rb", 1),
        ("vendor/a.rb", 0),
        ("db/schema.rb", 0),
    ] {
        let file = d.path().join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, SOURCE).unwrap();
        let old = fs::metadata(&file).unwrap();
        assert_eq!(count(&run(d.path(), &["--only", COP, "--fix", path])), n);
        assert_eq!(fs::read(&file).unwrap(), SOURCE);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&file).unwrap().modified().unwrap()
        );
    }
}
#[test]
fn both_directive_prefixes_multiline_default_range_next_push_and_selector_bounds() {
    let d = project();
    let source = std::str::from_utf8(SOURCE).unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for text in [
            format!("# {prefix}:disable Lint\n{source}"),
            format!("# {prefix}:disable-next {COP}\n{source}"),
            format!("# {prefix}:next -{COP}\n{source}"),
            format!("{} # {prefix}:disable {COP}\n", source.trim_end()),
            format!("# {prefix}:push -{COP}\n{source}# {prefix}:pop\n"),
            format!("x.fetch(key, [\n 1, # {prefix}:disable {COP}\n 2\n]) {{ it }}\n"),
            format!("# {prefix}:disable-next {COP}\nx.fetch(key, [\n 1,2\n]) {{ it }}\n"),
        ] {
            unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        for (open, close) in [
            (
                format!("# {prefix}:push +{COP}\n"),
                format!("# {prefix}:pop\n"),
            ),
            (format!("# {prefix}:enable-next {COP}\n"), String::new()),
        ] {
            let text = format!("# {prefix}:disable {COP}\n{source}{open}{source}{close}{source}");
            assert_eq!(
                count(&unchanged(
                    d.path(),
                    text.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(1)
                )),
                1
            );
        }
        // Highlight only the default, never the block body or heredoc payload.
        for text in [
            format!("x.fetch(key, default) do\n it # {prefix}:disable {COP}\nend\n"),
            format!("x.fetch(key, <<TEXT) {{ it }}\n# {prefix}:disable {COP}\nTEXT\n"),
        ] {
            assert_eq!(
                count(&unchanged(
                    d.path(),
                    text.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(1)
                )),
                1
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let text = format!("# {prefix}:enable {COP}\n{source}");
        assert_eq!(
            count(&unchanged(d.path(), text.as_bytes(), &["lib/a.rb"], None)),
            1
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                text.as_bytes(),
                &["--except", COP, "lib/a.rb"],
                None
            )),
            0
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", "Lint/Debugger", "lib/a.rb"],
                Some(0)
            )),
            0
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn observers_parser_gate_style_layout_composition_and_symlink_atomic_failure() {
    let d = project();
    for suffix in [
        "p __LINE__; File.binread(__FILE__); DATA.pos\n__END__\nraw\0\n",
        "RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
        "p __FILE__, __ENCODING__\n",
    ] {
        let text = format!(
            "\u{feff}# coding: utf-8\r\n{}{}",
            std::str::from_utf8(SOURCE).unwrap().replace('\n', "\r\n"),
            suffix.replace('\n', "\r\n")
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
    }
    let mut invalid = SOURCE.to_vec();
    invalid.extend_from_slice(b"def broken(\n");
    assert_eq!(
        count(&unchanged(
            d.path(),
            &invalid,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2)
        )),
        0
    );
    let peers = format!("{COP},Style/StringLiterals,Layout/SpaceAfterComma");
    let mut before = SOURCE.to_vec();
    before.extend_from_slice(b"a = ['a',2]\n");
    let mut after = SOURCE.to_vec();
    after.extend_from_slice(b"a = [\"a\", 2]\n");
    unchanged(d.path(), &before, &["--only", &peers, "lib/a.rb"], Some(1));
    let out = run(d.path(), &["--only", &peers, "--fix", "lib/a.rb"]);
    assert_eq!(count(&out), 1);
    let path = d.path().join("lib/a.rb");
    assert_eq!(fs::read(&path).unwrap(), after);
    unchanged(
        d.path(),
        &after,
        &["--only", &peers, "--fix", "lib/a.rb"],
        Some(1),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        symlink(&path, d.path().join("link.rb")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--only", COP, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(count(&out), 1);
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&path, &before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(d.path(), &["--only", &peers, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), 1);
        assert_eq!(fs::read(&path).unwrap(), before);
        let new = fs::metadata(&path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert!(
            fs::symlink_metadata(d.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
#[test]
fn native_ruby34_syntax_only_for_all_inputs_and_upstream_canonicals() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        for key in ["bytes", "upstream_after"] {
            let Some(value) = row.get(key) else {
                continue;
            };
            let bytes: Vec<u8> = serde_json::from_value(value.clone()).unwrap();
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
