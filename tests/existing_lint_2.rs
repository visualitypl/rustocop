//! Source-only pipeline/CLI controls. Inputs are NEVER executed.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use rustocop::{config::ResolvedConfig, cops::Cop};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
const CASES: [(&str, &[u8]); 7] = [
    ("Lint/DeprecatedClassMethods", b"File.exists?(x)\n"),
    (
        "Lint/DuplicateCaseCondition",
        b"case x; when 1; when 1; end\n",
    ),
    ("Lint/DuplicateElsifCondition", b"if x; elsif x; end\n"),
    ("Lint/DuplicateHashKey", b"{a: 1, a: 2}\n"),
    (
        "Lint/DuplicateMethods",
        b"consume(Class.new { def f; end; def f; end })\n",
    ),
    (
        "Lint/DuplicateRescueException",
        b"begin; x; rescue A; rescue A; end\n",
    ),
    ("Lint/DuplicateSetElement", b"Set[:a, :a]\n"),
];
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/existing_lint_2.json")).unwrap()
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
fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .matches(&format!("({cop})"))
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
fn all_original_byte_reference_rows_match_pipeline_exact_ranges_messages_no_edits() {
    let config = ResolvedConfig::default();
    let registry = rustocop::cops::Registry::new();
    let mut mismatches = Vec::new();
    for row in rows() {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let cop: &dyn Cop = registry
            .cops()
            .iter()
            .find(|c| c.name() == row["cop"].as_str().unwrap())
            .unwrap()
            .as_ref();
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&bytes),
            &[cop],
            &config,
        );
        if row.get("parser_rejection").is_some() {
            assert!(!report.errors.is_empty(), "{row}");
            assert!(report.result.offenses.is_empty());
            continue;
        }
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}\n{report:?}"
        );
        let mut actual: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        actual.sort();
        let expected: Vec<(u32, u32, u32, String)> =
            crate::support::fixture::value(&row["findings"]);
        if actual != expected {
            mismatches.push(format!(
                "{}\nactual: {actual:?}\nexpected: {expected:?}",
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
#[test]
fn all_original_bytes_rebuilt_cli_lint_and_fix_preserve_bytes_mtime_permissions_inode_fd() {
    parallel_rows!(for row in rows() => {
        let d = project();
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
        let cop = row["cop"].as_str().unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", cop];
            if fix {
                args.push("--fix");
            }
            args.push("lib/a.rb");
            if row.get("parser_rejection").is_some() {
                assert_eq!(count(&unchanged(d.path(), &bytes, &args, Some(2)), cop), 0);
                continue;
            }
            let expected: Vec<(u32, u32, u32, String)> =
                crate::support::fixture::value(&row["findings"]);
            let out = unchanged(
                d.path(),
                &bytes,
                &args,
                Some(i32::from(!expected.is_empty())),
            );
            let marker = format!(" ({cop})\n");
            let actual: Vec<_> = String::from_utf8_lossy(&out.stdout)
                .split_inclusive(marker.as_str())
                .filter(|l| l.ends_with(marker.as_str()))
                .map(|s| s.trim_end_matches('\n').to_owned())
                .collect();
            let want: Vec<_> = expected
                .iter()
                .map(|(l, c, _, m)| {
                    format!(
                        "lib/a.rb:{l}:{c}: E: {} ({cop})",
                        m.replace("example.rb:", "lib/a.rb:")
                    )
                })
                .collect();
            assert_eq!(actual, want, "{row}");
            assert!(out.stderr.is_empty(), "{out:?}");
            if fix {
                assert!(String::from_utf8_lossy(&out.stdout).contains("0 corrections applied"));
            }
        }
    });
}
#[test]
fn defaults_selectors_strict_toml_department_precedence_and_path_excludes() {
    let registry = rustocop::cops::Registry::new();
    let d = project();
    for (cop, source) in CASES {
        let implementation = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(implementation.default_enabled());
        assert!(!implementation.safe_fixes());
        assert_eq!(implementation.ruby_versions(), Default::default());
        assert!(
            implementation.path_policy().is_none()
                && implementation.max_setting().is_none()
                && implementation.autocorrect_incompatible_with().is_empty()
        );
        for (conf, mut args, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint", "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", "Lint"], 0),
            ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
            (
                format!("[cops.Lint]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (
                format!("[cops.Lint]\nenabled = true\n[cops.\"{cop}\"]\nenabled = false\n"),
                vec![],
                0,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
                vec!["--only", cop],
                0,
            ),
            ("exclude = ['lib/a.rb']\n".into(), vec!["--only", cop], 0),
        ] {
            fs::write(d.path().join("rustocop.toml"), conf).unwrap();
            args.push("lib/a.rb");
            assert_eq!(count(&unchanged(d.path(), source, &args, None), cop), n);
        }
        for key in [
            "max = 1",
            "include = ['**/*']",
            "Enabled = true",
            "enabled = 'true'",
            "TargetRubyVersion = 3.3",
            "SafeAutoCorrect = true",
            "AllowedMethods = ['raise']",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                d.path(),
                source,
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(2),
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn supplied_file_stdin_path_provenance_and_stdin_fix_rejection() {
    let d = project();
    for (cop, source) in CASES {
        for conf in [
            String::new(),
            "exclude = ['lib/a.rb']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
        ] {
            let n = usize::from(conf.is_empty());
            fs::write(d.path().join("rustocop.toml"), conf).unwrap();
            assert_eq!(
                count(
                    &unchanged(
                        d.path(),
                        source,
                        &["--only", cop, "lib/a.rb"],
                        Some(n as i32)
                    ),
                    cop
                ),
                n
            );
            assert_eq!(
                count(&run(&d.path().join("lib"), &["--only", cop, "a.rb"]), cop),
                n
            );
            fs::write(d.path().join("lib/a.rb"), b"sentinel\n").unwrap();
            let old = fs::metadata(d.path().join("lib/a.rb")).unwrap();
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(["--only", cop, "--stdin", "lib/a.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(count(&out, cop), n);
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
            run(d.path(), &["--only", cop, "--stdin", "lib/a.rb", "--fix"])
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
            fs::write(&file, source).unwrap();
            let old = fs::metadata(&file).unwrap();
            assert_eq!(
                count(&run(d.path(), &["--only", cop, "--fix", path]), cop),
                n
            );
            assert_eq!(fs::read(&file).unwrap(), source);
            assert_eq!(
                old.modified().unwrap(),
                fs::metadata(&file).unwrap().modified().unwrap()
            );
        }
    }
}
#[test]
fn both_directive_prefixes_multiline_next_push_scopes_and_selector_bounds() {
    let d = project();
    for (cop, source) in CASES {
        let source = std::str::from_utf8(source).unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for text in [
                format!("# {prefix}:disable Lint\n{source}"),
                format!("# {prefix}:disable-next {cop}\n{source}"),
                format!("# {prefix}:next -{cop}\n{source}"),
                format!("# {prefix}:push -{cop}\n{source}# {prefix}:pop\n"),
            ] {
                unchanged(
                    d.path(),
                    text.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(0),
                );
            }
            for (open, close) in [
                (
                    format!("# {prefix}:push +{cop}\n"),
                    format!("# {prefix}:pop\n"),
                ),
                (format!("# {prefix}:enable-next {cop}\n"), String::new()),
            ] {
                let text =
                    format!("# {prefix}:disable {cop}\n{source}{open}{source}{close}{source}");
                assert_eq!(
                    count(
                        &unchanged(
                            d.path(),
                            text.as_bytes(),
                            &["--only", cop, "--fix", "lib/a.rb"],
                            Some(1)
                        ),
                        cop
                    ),
                    1
                );
            }
            let multiline = match cop {
                "Lint/DeprecatedClassMethods" => "attr :a,\n true",
                "Lint/DuplicateCaseCondition" => "case x; when [1]; when [\n1\n]",
                "Lint/DuplicateElsifCondition" => "if [1]; elsif [\n1\n]",
                "Lint/DuplicateHashKey" => "{[1] => 1, [\n1\n] => 2}",
                "Lint/DuplicateMethods" => "attr_reader :a; attr_reader(\n:a\n)",
                "Lint/DuplicateRescueException" => "begin; x; rescue [A]; rescue [\nA\n]",
                _ => "Set[[1], [\n1\n]]",
            };
            let close = if [
                "Lint/DuplicateCaseCondition",
                "Lint/DuplicateElsifCondition",
                "Lint/DuplicateRescueException",
            ]
            .contains(&cop)
            {
                "\nend\n"
            } else {
                "\n"
            };
            let multiline = format!("{multiline} # {prefix}:disable {cop}{close}");
            unchanged(
                d.path(),
                multiline.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
            );
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            let text = format!("# {prefix}:enable {cop}\n{source}");
            assert_eq!(
                count(
                    &unchanged(d.path(), text.as_bytes(), &["lib/a.rb"], None),
                    cop
                ),
                1
            );
            assert_eq!(
                count(
                    &unchanged(
                        d.path(),
                        text.as_bytes(),
                        &["--except", cop, "lib/a.rb"],
                        None
                    ),
                    cop
                ),
                0
            );
            assert_eq!(
                count(
                    &unchanged(
                        d.path(),
                        text.as_bytes(),
                        &["--only", "Lint/Debugger", "lib/a.rb"],
                        Some(0)
                    ),
                    cop
                ),
                0
            );
            fs::write(d.path().join("rustocop.toml"), "").unwrap();
        }
    }
}
#[test]
fn observer_bytes_parser_gate_style_layout_composition_and_symlink_atomic_failure() {
    let d = project();
    for (cop, source) in CASES {
        for suffix in [
            "p __LINE__; File.binread(__FILE__); DATA.pos\n__END__\nraw\0\n",
            "RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {}\n",
            "p __FILE__, __ENCODING__\n",
        ] {
            let text = format!(
                "\u{feff}# coding: utf-8\r\n{}{}",
                std::str::from_utf8(source).unwrap().replace('\n', "\r\n"),
                suffix.replace('\n', "\r\n")
            );
            assert_eq!(
                count(
                    &unchanged(
                        d.path(),
                        text.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        Some(1)
                    ),
                    cop
                ),
                1
            );
        }
        let mut invalid = source.to_vec();
        invalid.extend_from_slice(b"def broken(\n");
        assert_eq!(
            count(
                &unchanged(
                    d.path(),
                    &invalid,
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(2)
                ),
                cop
            ),
            0
        );
        let peers = format!("{cop},Style/StringLiterals,Layout/SpaceAfterComma");
        let mut before = source.to_vec();
        before.extend_from_slice(b"a = ['a',2]\n");
        let mut after = source.to_vec();
        after.extend_from_slice(b"a = [\"a\", 2]\n");
        unchanged(d.path(), &before, &["--only", &peers, "lib/a.rb"], Some(1));
        assert_eq!(
            count(
                &run(d.path(), &["--only", &peers, "--fix", "lib/a.rb"]),
                cop
            ),
            1
        );
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
            let link = d.path().join("link.rb");
            symlink(&path, &link).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
            let old = fs::metadata(&path).unwrap();
            assert_eq!(
                count(&run(d.path(), &["--only", cop, "--fix", "link.rb"]), cop),
                1
            );
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
            assert_eq!(count(&out, cop), 1);
            assert_eq!(fs::read(&path).unwrap(), before);
            let new = fs::metadata(&path).unwrap();
            assert_eq!(old.modified().unwrap(), new.modified().unwrap());
            assert_eq!(old.permissions(), new.permissions());
            assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
            fs::remove_file(link).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        }
    }
}
#[test]
fn native_ruby34_syntax_only_all_reference_and_adversarial_original_bytes() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for row in rows() {
        let bytes: Vec<u8> = crate::support::fixture::value(&row["bytes"]);
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
