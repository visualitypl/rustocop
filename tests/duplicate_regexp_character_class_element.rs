//! Source-only reference parity, real CLI and authored native safety proofs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::deprecated_constructor_native as native_guard;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/DuplicateRegexpCharacterClassElement";
const SAFE: &[u8] = b"[defined?(/[xyx]/)]\n";
const AFTER: &[u8] = b"[defined?(/[xy]/)]\n";
const CALL: &[u8] = b"value = /[xyx]/\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!(
        "fixtures/duplicate_regexp_character_class_element.json"
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
        assert_eq!(out.status.code(), Some(code), "{out:?}");
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
fn all_reference_rows_exact_cli_findings_no_writes_and_canonical_edits() {
    let dir = project();
    let mut fixed = 0;
    assert_eq!(rows().len(), 475);
    for row in rows() {
        let source = row["source"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        let code = if row["invalid_syntax"] == true && row["fixed_native_valid"] != true {
            2
        } else {
            i32::from(!findings.is_empty())
        };
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "lib/a.rb"],
            Some(code),
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
        let after = row["project_after"].as_str().unwrap();
        if after == source {
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(code)
                )),
                findings.len()
            );
        } else {
            fixed += 1;
            let old = fs::metadata(dir.path().join("lib/a.rb")).unwrap();
            let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{row}\n{out:?}");
            assert_eq!(
                fs::read(dir.path().join("lib/a.rb")).unwrap(),
                after.as_bytes()
            );
            assert_eq!(
                old.permissions(),
                fs::metadata(dir.path().join("lib/a.rb"))
                    .unwrap()
                    .permissions()
            );
            unchanged(
                dir.path(),
                after.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
    assert_eq!(fixed, 10);
}
#[test]
fn operator_recovery_exact_cli_no_internal_errors_no_writes_and_native_syntax() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_regexp_extended.json")).unwrap();
    assert_eq!(rows.len(), 1051);
    let dir = project();
    for row in rows {
        let source = row["source"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &args,
                Some(i32::from(!findings.is_empty())),
            );
            assert!(out.stderr.is_empty(), "{row}\n{out:?}");
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
        // All generated fixtures are controlled source-only inputs. This does
        // NOT construct a Regexp or execute interpolation/reference code.
        if let Ok(ruby) = crate::support::runtime::ruby() {
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
                .write_all(source.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{row}\n{out:?}");
        }
    }
}
#[test]
fn operator_round2_shared_cli_findings_and_held_fixes_preserve_source() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_regexp_extended.json")).unwrap();
    let dir = project();
    let names = [
        COP,
        "Lint/MixedRegexpCaptureTypes",
        "Lint/UnescapedBracketInRegexp",
        "Lint/RedundantRegexpQuantifiers",
        "Lint/MixedCaseRange",
    ];
    let only = names.join(",");
    let mut checked = 0;
    for row in rows {
        let Some(findings) = row.get("shared_findings").and_then(|v| v.as_array()) else {
            continue;
        };
        checked += 1;
        for fix in [false, true] {
            let mut args = vec!["--only", &only, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = unchanged(
                dir.path(),
                row["source"].as_str().unwrap().as_bytes(),
                &args,
                Some(i32::from(!findings.is_empty())),
            );
            assert!(out.stderr.is_empty(), "{row}\n{out:?}");
            let text = String::from_utf8(out.stdout).unwrap();
            assert_eq!(
                text.matches(": E: ").count(),
                findings.len(),
                "{row}\n{text}"
            );
            for f in findings {
                assert!(
                    text.contains(&format!(
                        "lib/a.rb:{}:{}: E: {} ({})",
                        f[1],
                        f[2],
                        f[4].as_str().unwrap(),
                        f[0].as_str().unwrap()
                    )),
                    "{row}\n{text}"
                );
            }
        }
    }
    assert_eq!(checked, 607);
}
#[test]
fn operator_round2_review_extended_whitespace_cli_no_write_and_native_syntax() {
    let dir = project();
    let only = format!(
        "{COP},Lint/MixedRegexpCaptureTypes,Lint/UnescapedBracketInRegexp,Lint/RedundantRegexpQuantifiers,Lint/MixedCaseRange"
    );
    let mut sources = Vec::new();
    for space in [" ", "\t", "\n", "\r", "\u{b}", "\u{c}"] {
        for source in [
            format!("/(?x){space}*[aa]#{{value}}/"),
            format!("/{space}*[aa]#{{value}}/x"),
            format!("/(?x:{space}*[aa])#{{value}}/"),
        ] {
            sources.push((source, 0));
        }
    }
    sources.extend([
        ("/\u{b}*[aa]#{value}/".into(), 1),
        ("/(?-x:\u{b}*[aa])#{value}/x".into(), 1),
        ("/(?x)[\u{b}\u{b}]#{value}/".into(), 1),
        ("/(?x)a\u{b}*[aa]#{value}/".into(), 1),
    ]);
    for (source, expected) in sources {
        for fix in [false, true] {
            let mut args = vec!["--only", &only, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = unchanged(dir.path(), source.as_bytes(), &args, Some(expected));
            assert!(out.stderr.is_empty(), "{source:?}\n{out:?}");
            assert_eq!(count(&out), expected as usize, "{source:?}\n{out:?}");
        }
        if let Ok(ruby) = crate::support::runtime::ruby() {
            // Syntax only: interpolation and regexp/program code never execute.
            let out = Command::new("timeout")
                .args([
                    "--kill-after=1s",
                    "10",
                    "prlimit",
                    "--as=536870912",
                    "--fsize=1048576",
                    "--core=0",
                    "--",
                ])
                .arg(ruby)
                .args(["--parser=prism", "-c", "-e", &source])
                .output()
                .unwrap();
            assert!(out.status.success(), "{source:?}\n{out:?}");
        }
    }
}
#[test]
fn legacy_source_buffer_projection_columns_and_bytes_are_preserved() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_regexp_legacy.json")).unwrap();
    let dir = project();
    assert_eq!(rows.len(), 36);
    for row in rows {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let findings = row["findings"].as_array().unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = unchanged(
                dir.path(),
                &source,
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
        if let Ok(ruby) = crate::support::runtime::ruby() {
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-c", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{row}\n{out:?}");
        }
    }
}
#[test]
fn defaults_metadata_selectors_strict_config_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
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
        assert_eq!(count(&unchanged(dir.path(), CALL, &args, None)), n);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 3.3",
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
fn both_directive_prefixes_ranges_next_push_and_enable() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\nvalue = /[xyx]/\n"),
            format!("# {prefix}:push -{COP}\nvalue = /[xyx]/\n# {prefix}:pop\n"),
            format!("# {prefix}:disable-next {COP}\nvalue = /[xyx]/\n"),
            format!("# {prefix}:next -{COP}\nvalue = /[xyx]/\n"),
            format!("[defined?(/[xyx]/)] # {prefix}:disable {COP}\n"),
            format!(
                "value = /[\\p{{White\nSpace}}\\p{{White\nSpace}}]/ # {prefix}:disable {COP}\n"
            ),
        ] {
            // The multiline repeated token ends on the directive line.
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        let source = format!("value = /[aa\n]/ # {prefix}:disable {COP}\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
        for open in [
            format!("# {prefix}:push +{COP}"),
            format!("# {prefix}:enable-next {COP}"),
        ] {
            let source = format!(
                "# {prefix}:disable {COP}\nvalue = /[xyx]/\n{open}\nvalue = /[xyx]/\n# {prefix}:pop\nvalue = /[xyx]/\n"
            );
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
    }
}
#[test]
fn supplied_paths_stdin_provenance_and_stdin_fix_rejection() {
    let dir = project();
    for path in [
        "lib/a.rb",
        "a.jb",
        "view.erb",
        "db/schema.rb",
        "vendor/a.rb",
        ".hidden.rb",
    ] {
        let p = dir.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, CALL).unwrap();
        let out = run(dir.path(), &["--only", COP, path]);
        let n = usize::from(!path.starts_with("vendor/") && path != "db/schema.rb");
        assert_eq!(count(&out), n, "{path}\n{out:?}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(CALL).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), n, "{path}\n{out:?}");
        if n == 1 {
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:1:13:")),
                "{out:?}"
            );
        }
    }
    assert_eq!(
        run(dir.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
}
#[test]
fn encoding_bom_crlf_comments_and_all_uncertain_observers() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
        b"# coding: UTF8-MAC\n# e\xcc\x81\n",
    ] {
        let source = [prefix, b"# keep\r\n[defined?(/[xyx]/)] # consume\r\n"].concat();
        let after = [prefix, b"# keep\r\n[defined?(/[xy]/)] # consume\r\n"].concat();
        unchanged(dir.path(), &source, &["--only", COP, "lib/a.rb"], Some(1));
        let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), after);
    }
    for observer in [
        "p __LINE__",
        "File.read(__FILE__)",
        "IO.read($0)",
        "RubyVM::InstructionSequence.compile_file(__FILE__)",
        "RubyVM::AbstractSyntaxTree.parse_file(__FILE__)",
        "ObjectSpace.each_object {}",
        "GC.stat",
        "binding.local_variables",
        "TracePoint.new(:line) {}.enable",
        "__END__\ndata\n",
        "DATA.read\n__END__\ndata\n",
        "<<~DOC\n#{/[xx]/}\nDOC\n",
    ] {
        let source = [SAFE, observer.as_bytes(), b"\n"].concat();
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
fn atomic_validation_readonly_symlinks_permissions_and_peer_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, CALL).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for alias in ["lib/a.rb", "lib/link.rb"] {
        assert_eq!(
            run(
                dir.path(),
                &["--only", COP, "--fix", "--fix-passes", "1", alias]
            )
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
    assert_eq!(fs::read(&path).unwrap(), CALL);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(
            dir.path(),
            &["--only", COP, "--fix", "--fix-passes", "1", "lib/link.rb"]
        )
        .status
        .code(),
        Some(2)
    );
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    unchanged(
        dir.path(),
        b"[defined?(/[xx]/)]\n(\n",
        &["--only", COP, "--fix", "--fix-passes", "1", "lib/a.rb"],
        Some(2),
    );
    fs::write(&path, SAFE).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    assert_eq!(
        run(
            dir.path(),
            &["--only", COP, "--fix", "--fix-passes", "1", "lib/link.rb"]
        )
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
    for (source, after, peers) in [
        (
            "[defined?(/[xyx]/)]  \n",
            "[defined?(/[xy]/)]\n",
            "Layout/TrailingWhitespace",
        ),
        (
            "[defined?(%r{[xyx]})]\n",
            "[defined?(/[xyx]/)]\n",
            "Style/RegexpLiteral",
        ),
        (
            "[defined?(/[xyx]/),'x']\n",
            "[defined?(/[xyx]/),\"x\"]\n",
            "Style/StringLiterals",
        ),
        (
            "value = [/[xyx]/,1]\n",
            "value = [/[xyx]/, 1]\n",
            "Layout/SpaceAfterComma",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let only = format!("{COP},{peers}");
        run(
            dir.path(),
            &["--only", &only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        if peers == "Style/RegexpLiteral" {
            // Overlapping envelopes are resolved by the shared single-pass
            // planner; the retained finding is corrected on the next request.
            assert_eq!(
                run(
                    dir.path(),
                    &["--only", COP, "--fix", "--fix-passes", "1", "lib/a.rb"]
                )
                .status
                .code(),
                Some(0)
            );
            assert_eq!(fs::read(&path).unwrap(), AFTER);
        }
    }
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(
            dir.path(),
            &[
                "--only",
                COP,
                "--fix",
                "--fix-passes",
                "1",
                "lib/dangling.rb"
            ]
        )
        .status
        .code(),
        Some(2)
    );
    assert!(!dir.path().join("lib/missing.rb").exists());
}
fn native(ruby: &str, source: &str) -> Output {
    // Reuse the existing external wall/memory/output/core-dump guards. Native
    // runtime witnesses are authored here, never drawn from reference fixtures.
    native_guard::run(ruby, source)
}
#[test]
fn authored_native_value_binding_evaluation_warning_and_source_proofs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let dir = project();
    for (before, after) in [
        ("[defined?(/[xyx]/)]", "[defined?(/[xy]/)]"),
        ("[defined?(/[0-9x0-9]/)]", "[defined?(/[0-9x]/)]"),
        ("[defined?(%r{[xyx]}i)]", "[defined?(%r{[xy]}i)]"),
    ] {
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "lib/a.rb"],
            Some(1),
        );
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            after
        );
        for (a, b) in [
            (before.to_string(), after.to_string()),
            (
                format!(
                    "value = (begin; {before}; end); p [value, value.class.name, value[0].class.name, value[0].encoding.name, TOPLEVEL_BINDING.local_variables, $~, $_]"
                ),
                format!(
                    "value = (begin; {after}; end); p [value, value.class.name, value[0].class.name, value[0].encoding.name, TOPLEVEL_BINDING.local_variables, $~, $_]"
                ),
            ),
        ] {
            let a = native(&ruby, &a);
            let b = native(&ruby, &b);
            assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
            assert_eq!(a.stdout, b.stdout);
            assert_eq!(a.stderr, b.stderr);
            assert!(a.stderr.is_empty(), "{a:?}");
            if !a.stdout.is_empty() {
                assert_eq!(a.stdout, b"[[\"expression\"], \"Array\", \"String\", \"US-ASCII\", [:value], nil, nil]\n");
            }
        }
    }
    // Authored instrumentation only: the accepted expression never allocates a
    // regexp or invokes Regexp methods. This is not a permitted edit context.
    let out = native(
        &ruby,
        "events = []; count = 0; tp = TracePoint.new(:call, :c_call) { |t| count += 1; raise 'trace overflow FAILURE' if count > 10000; events << t.method_id if t.defined_class == Regexp }; tp.enable; value = [defined?(/[xx]/), defined?(/[x]/)]; tp.disable; p [value, events]",
    );
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_eq!(out.stdout, b"[[\"expression\", \"expression\"], []]\n");
    for before in [
        "value = /[xx]/; p [value.source, value.options, value.encoding.name, value.match?('x')]",
        "[defined?(/[xx]/.match?(effect))]",
    ] {
        let after = before.replace("[xx]", "[x]");
        let a = native(&ruby, before);
        let b = native(&ruby, &after);
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert!(
            String::from_utf8_lossy(&a.stderr).contains("duplicated range"),
            "{a:?}"
        );
        assert!(b.stderr.is_empty(), "{b:?}");
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
}
#[test]
fn authored_native_trace_overflow_is_failure_not_a_truncated_proof() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let out = native(
        &ruby,
        "count = 0; tp = TracePoint.new(:c_call) { count += 1; raise 'trace overflow FAILURE' if count > 4 }; tp.enable; 20.times { 1.to_s }; tp.disable; puts 'UNREACHABLE proof'",
    );
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("trace overflow FAILURE"),
        "{out:?}"
    );
    assert!(out.stdout.is_empty(), "{out:?}");
}
#[test]
fn all_reference_sources_and_upstream_rewrites_are_native_syntax_only() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        for key in ["source", "upstream_after"] {
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
                row["invalid_syntax"] != true || row["fixed_native_valid"] == true,
                "{row}\n{key}\n{out:?}"
            );
        }
    }
}
