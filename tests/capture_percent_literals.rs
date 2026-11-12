//! Source-only reference parity and actual CLI controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
fn lint_str(source: &str) -> Vec<rustocop::offense::Offense> {
    let registry = rustocop::cops::Registry::new();
    let cops: Vec<_> = registry.cops().iter().map(|c| c.as_ref()).collect();
    rustocop::pipeline::lint_one(
        std::path::Path::new("example.rb"),
        std::path::Path::new("example.rb"),
        Some(source.as_bytes()),
        &cops,
        &rustocop::config::ResolvedConfig::default(),
    )
    .result
    .offenses
}
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = ["Lint/MixedRegexpCaptureTypes", "Lint/NestedPercentLiteral"];
const SOURCES: [&str; 2] = ["/(?<n>a)(b)/\n", "%w[%i[a]]\n"];
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/capture_percent_literals.json")).unwrap()
}
fn source(row: &serde_json::Value) -> Vec<u8> {
    row["source"]
        .as_str()
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_else(|| serde_json::from_value(row["bytes"].clone()).unwrap())
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
fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({cop})")))
        .count()
}
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], code: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, bytes).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(code) = code {
        assert_eq!(
            out.status.code(),
            Some(code),
            "{out:?}\n{}",
            String::from_utf8_lossy(bytes)
        );
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
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
fn all_pinned_spec_source_and_encoded_controls_exact_pipeline() {
    assert_eq!(rows().len(), 1421);
    for row in rows()
        .into_iter()
        .filter(|r| r["rejected_controls"].is_null())
    {
        let registry = rustocop::cops::Registry::new();
        let cops: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| COPS.contains(&c.name()))
            .map(|c| c.as_ref())
            .collect();
        let result = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source(&row)),
            &cops,
            &Default::default(),
        );
        assert!(
            result.errors.is_empty() && result.internal_errors.is_empty(),
            "{row}\n{:?}",
            result.errors
        );
        for cop in COPS {
            let found: Vec<_> = result
                .result
                .offenses
                .iter()
                .filter(|o| o.cop == cop)
                .map(|o| {
                    assert!(o.correction.is_none());
                    serde_json::json!([o.line, o.col, o.end_line, o.message])
                })
                .collect();
            assert_eq!(
                serde_json::json!(found),
                row[cop]["findings"],
                "{cop}\n{row}"
            );
        }
    }
}
#[test]
fn all_rows_rebuilt_cli_lint_fix_preserve_bytes_mtime_permissions_inode() {
    parallel_rows!(for row in rows()
        .into_iter()
        .filter(|r| r["rejected_controls"].is_null()) => {
        let dir = project();
        for cop in COPS {
            let findings = row[cop]["findings"].as_array().unwrap();
            for fix in [false, true] {
                let mut args = vec!["--only", cop, "lib/a.rb"];
                if fix {
                    args.push("--fix");
                }
                let out = unchanged(
                    dir.path(),
                    &source(&row),
                    &args,
                    Some(i32::from(!findings.is_empty())),
                );
                assert_eq!(count(&out, cop), findings.len(), "{row}\n{out:?}");
                for f in findings {
                    assert!(
                        String::from_utf8_lossy(&out.stdout).contains(&format!(
                            "lib/a.rb:{}:{}: E: {} ({cop})",
                            f[0],
                            f[1],
                            f[3].as_str().unwrap()
                        )),
                        "{row}\n{out:?}"
                    );
                }
            }
        }
    });
}
#[test]
fn registry_fixed_defaults_selectors_strict_toml_department_precedence_and_excludes() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let c = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(c.default_enabled() && !c.safe_fixes());
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(
            c.path_policy().is_none()
                && c.max_setting().is_none()
                && c.autocorrect_incompatible_with().is_empty()
        );
        let dir = project();
        for (cfg, args, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint"], 1),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
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
        ] {
            fs::write(dir.path().join("rustocop.toml"), cfg).unwrap();
            let mut args = args;
            args.push("lib/a.rb");
            assert_eq!(
                count(&unchanged(dir.path(), source.as_bytes(), &args, None), cop),
                n
            );
        }
        for key in [
            "max = 1",
            "include = ['**/*']",
            "Enabled = true",
            "enabled = 'true'",
            "target_ruby = 3.3",
            "SafeAutoCorrect = false",
            "NewCops = 'enable'",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(2),
            );
        }
        for cfg in [
            "exclude = ['lib/**']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), cfg).unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
}
#[test]
fn directives_both_prefixes_multiline_range_next_push_pop_and_activation() {
    let dir = project();
    for (cop, source) in COPS
        .into_iter()
        .zip(["/(?<n>a)\n(b)/\n", "%w[a\n %i[b c]]\n"])
    {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Lint", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!(
                        "# {prefix}:push\n# {prefix}:disable {target}\n{source}# {prefix}:pop\n"
                    ),
                ] {
                    unchanged(
                        dir.path(),
                        s.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        Some(0),
                    );
                }
            }
            for mode in [format!("next -{cop}"), format!("disable-next {cop}")] {
                unchanged(
                    dir.path(),
                    format!("# {prefix}:{mode}\n{source}").as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(0),
                );
            }
            for opener in [format!("push +{cop}"), format!("enable {cop}")] {
                unchanged(
                    dir.path(),
                    format!(
                        "# {prefix}:disable {cop}\n# {prefix}:{opener}\n{source}# {prefix}:pop\n"
                    )
                    .as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(1),
                );
            }
            let s = if cop == COPS[0] {
                format!("/(?<n>a)\n(b)/ # {prefix}:disable {cop}\n")
            } else {
                format!("%W[a #{{nil # {prefix}:disable {cop}\n}} %i[b]]\n")
            };
            unchanged(
                dir.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
            );
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            let s = format!("# {prefix}:enable {cop}\n{source}");
            assert_eq!(
                count(
                    &unchanged(dir.path(), s.as_bytes(), &["lib/a.rb"], None),
                    cop
                ),
                1
            );
            assert_eq!(
                count(
                    &unchanged(
                        dir.path(),
                        s.as_bytes(),
                        &["--except", cop, "lib/a.rb"],
                        None
                    ),
                    cop
                ),
                0
            );
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        }
    }
}
#[test]
fn supplied_file_stdin_paths_builtins_and_stdin_fix_rejection() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for path in [
            "lib/a.rb",
            "a.jb",
            "view.erb",
            "vendor/a.rb",
            "db/schema.rb",
            ".hidden.rb",
        ] {
            let p = dir.path().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, source).unwrap();
            let n = usize::from(!path.starts_with("vendor/") && path != "db/schema.rb");
            assert_eq!(count(&run(dir.path(), &["--only", cop, path]), cop), n);
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", cop, "--stdin", path])
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
            assert_eq!(count(&out, cop), n, "{out:?}");
            if n == 1 {
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:1:1:")),
                    "{out:?}"
                );
            }
        }
        assert_eq!(
            run(dir.path(), &["--only", cop, "--stdin", "lib/a.rb", "--fix"])
                .status
                .code(),
            Some(2)
        );
    }
}
#[cfg(unix)]
#[test]
fn readonly_symlink_fd_invalid_source_atomic_failure_and_real_peer_correction() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        fs::write(&path, source).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let mut fd = fs::File::open(&path).unwrap();
        for alias in ["lib/a.rb", "lib/link.rb"] {
            assert_eq!(
                run(dir.path(), &["--only", cop, "--fix", alias])
                    .status
                    .code(),
                Some(1)
            );
        }
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap(), old.permissions()),
            (new.ino(), new.modified().unwrap(), new.permissions())
        );
        let mut bytes = vec![];
        fd.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, source.as_bytes());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        unchanged(
            dir.path(),
            format!("{source}(\n").as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(2),
        );
        let before = source.replace('\n', "  \n");
        let only = format!("{cop},Layout/TrailingWhitespace");
        fs::write(&path, &before).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", &only, "--fix", "lib/link.rb"])
                .status
                .code(),
            Some(1)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert!(
            fs::symlink_metadata(dir.path().join("lib/link.rb"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::write(&path, &before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", &only, "--fix", "lib/a.rb"])
                .status
                .code(),
            Some(2)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap()),
            (new.ino(), new.modified().unwrap())
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    }
}
#[test]
fn source_observers_and_encoding_envelopes_still_have_no_edits() {
    let dir = project();
    for suffix in [
        "p __LINE__\n",
        "p File.binread(__FILE__).bytesize\n",
        "p DATA.read\n__END__\npayload\n",
        "p RubyVM::InstructionSequence.of(method(:p))\n",
        "p ObjectSpace.count_objects\n",
        "x = <<~TEXT\n retained #{__LINE__}\nTEXT\n",
    ] {
        for (cop, source) in COPS.into_iter().zip(SOURCES) {
            let s = format!("\u{feff}{source}# retained\r\n{suffix}");
            unchanged(
                dir.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(1),
            );
        }
    }
}
#[test]
fn authored_native_values_stay_unchanged_without_invented_intention_repairs() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    // Separately authored bounded programs, not pinned spec/doc/control input.
    for (cop, program, expected) in [
        (
            COPS[0],
            "pattern = /(?<key>red)(blue)/; p pattern.match('redblue').captures",
            "[\"red\"]\n",
        ),
        (
            COPS[1],
            "words = %w[head %w[inner tail]]; p words",
            "[\"head\", \"%w[inner\", \"tail]\"]\n",
        ),
    ] {
        let out = Command::new(&ruby)
            .args(["--parser=prism", "-w", "-e", program])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
        unchanged(
            dir.path(),
            program.as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
}
#[test]
fn all_reference_rows_native_syntax_only_and_authored_parser_rejections_accounted() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let syntax = |bytes: &[u8]| {
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-c", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        child.wait_with_output().unwrap()
    };
    let mut rejected = 0;
    for row in rows() {
        if let Some(controls) = row["rejected_controls"].as_array() {
            for r in controls {
                let out = syntax(r["source"].as_str().unwrap().as_bytes());
                assert!(
                    !out.status.success(),
                    "unaccounted host-only rejection {r}\n{out:?}"
                );
                rejected += 1;
            }
        } else {
            let out = syntax(&source(&row));
            assert!(out.status.success(), "{row}\n{out:?}");
        }
    }
    assert_eq!(rejected, 30);
}

#[test]
fn capture_name_reference_boundaries_exact_pipeline_cli_native_syntax_and_no_writes() {
    use std::io::Read;
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/capture_reference_boundaries.json")).unwrap();
    let dir = project();
    let registry = rustocop::cops::Registry::new();
    let cops: Vec<_> = registry
        .cops()
        .iter()
        .filter(|c| COPS.contains(&c.name()))
        .map(|c| c.as_ref())
        .collect();
    for row in rows {
        let bytes = source(&row);
        let result = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&bytes),
            &cops,
            &Default::default(),
        );
        assert!(
            result.errors.is_empty() && result.internal_errors.is_empty(),
            "{row}\n{:?}",
            result.errors
        );
        for cop in COPS {
            let found: Vec<_> = result
                .result
                .offenses
                .iter()
                .filter(|o| o.cop == cop)
                .map(|o| {
                    assert!(o.correction.is_none());
                    serde_json::json!([o.line, o.col, o.end_line, o.message])
                })
                .collect();
            assert_eq!(serde_json::json!(found), row[cop]["findings"], "{row}");
            let expected = row[cop]["findings"].as_array().unwrap();
            for fix in [false, true] {
                let path = dir.path().join("lib/a.rb");
                fs::write(&path, &bytes).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
                }
                let mut fd = fs::File::open(&path).unwrap();
                let old = fd.metadata().unwrap();
                let mut args = vec!["--only", cop, "lib/a.rb"];
                if fix {
                    args.push("--fix");
                }
                let out = run(dir.path(), &args);
                assert_eq!(
                    out.status.code(),
                    Some(i32::from(!expected.is_empty())),
                    "{row}\n{out:?}"
                );
                assert_eq!(count(&out, cop), expected.len(), "{row}\n{out:?}");
                for finding in expected {
                    assert!(
                        String::from_utf8_lossy(&out.stdout).contains(&format!(
                            "lib/a.rb:{}:{}: E: {} ({cop})",
                            finding[0],
                            finding[1],
                            finding[3].as_str().unwrap()
                        )),
                        "{row}\n{out:?}"
                    );
                }
                assert_eq!(fs::read(&path).unwrap(), bytes);
                let new = fs::metadata(&path).unwrap();
                assert_eq!(old.modified().unwrap(), new.modified().unwrap());
                assert_eq!(old.permissions(), new.permissions());
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    assert_eq!(old.ino(), new.ino());
                    assert_eq!(new.mode() & 0o777, 0o751);
                }
                let mut open_bytes = vec![];
                fd.read_to_end(&mut open_bytes).unwrap();
                assert_eq!(open_bytes, bytes);
            }
        }
        if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
            let mut child = Command::new(ruby)
                .args(["--parser=prism", "-c", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{row}\n{out:?}");
        }
    }
}

#[test]
fn real_detection_and_messages() {
    for (source, cop, message) in [
        (
            "nil\n  /(?<foo>bar)(baz)/\n",
            COPS[0],
            "Do not mix named captures and numbered captures in a Regexp literal.",
        ),
        (
            "nil\n  %i[a b %i[c d] xyz]\n",
            COPS[1],
            "Within percent literals, nested percent literals do not function and may be unwanted in the result.",
        ),
    ] {
        let found: Vec<_> = lint_str(source)
            .into_iter()
            .filter(|o| o.cop == cop)
            .collect();
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(found[0].message, message);
        assert!(found[0].correction.is_none());
    }
}
