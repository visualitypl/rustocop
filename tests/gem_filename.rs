//! Real filename lifecycle; RuboCop gem_filename_spec.rb examples
//! ported under MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::deprecated_constructor_native as native;
use crate::support::stdin as stdin_support;
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, select_active},
    offense::Severity,
};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Bundler/GemFilename";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], bytes: Option<&[u8]>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(if bytes.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(bytes) = bytes {
        stdin_support::write_stdin(child.stdin.take().unwrap(), bytes, args.contains(&"--fix"))
            .unwrap();
    }
    child.wait_with_output().unwrap()
}
fn check(out: &Output, code: i32, count: usize) {
    assert_eq!(out.status.code(), Some(code), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout)
            .matches(&format!("({COP})"))
            .count(),
        count,
        "{out:?}"
    );
    if code != 2 {
        assert!(out.stderr.is_empty(), "{out:?}");
    }
}
fn unchanged(path: &Path, bytes: &[u8], old: &fs::Metadata) {
    assert_eq!(fs::read(path).unwrap(), bytes);
    let now = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), now.modified().unwrap());
    assert_eq!(old.permissions(), now.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), now.ino());
    }
}
#[test]
fn all_fifteen_main_lifecycle_cases_at_fixed_gemfile_style() {
    // First eight declared Gemfile cases, then all seven alternate-style cases
    // replayed at the product's fixed Gemfile default (not excluded).
    let paths = [
        "gems.rb",
        "spec/gems.rb",
        "gems.locked",
        "spec/gems.locked",
        "Gemfile",
        "spec/Gemfile",
        "Gemfile.lock",
        "spec/Gemfile.lock",
        "Gemfile",
        "spec/Gemfile",
        "Gemfile.lock",
        "spec/Gemfile.lock",
        "gems.rb",
        "spec/gems.rb",
        "spec/gems.locked",
    ];
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    let active = select_active(&registry, &config, &[COP.into()], &[]);
    assert_eq!(active.len(), 1);
    for path in paths {
        for source in [b"print 1".as_slice(), b""] {
            let report = rustocop::pipeline::lint_one(
                Path::new(path),
                Path::new(path),
                Some(source),
                &active,
                &config,
            );
            assert!(
                report.errors.is_empty() && report.internal_errors.is_empty(),
                "{report:?}"
            );
            assert!(report.result.offenses.is_empty());
            let prefix = match Path::new(path).file_name().unwrap().to_str().unwrap() {
                "gems.rb" => Some("`gems.rb` file was found but `Gemfile` is required"),
                "gems.locked" => {
                    Some("Expected a `Gemfile.lock` with `Gemfile` but found `gems.locked` file")
                }
                _ => None,
            };
            assert_eq!(
                report.result.global_offenses.len(),
                usize::from(prefix.is_some()),
                "{path}"
            );
            if let Some(prefix) = prefix {
                let o = &report.result.global_offenses[0];
                assert_eq!(o.cop, COP);
                assert_eq!(o.severity, Severity::Convention);
                assert_eq!(o.message, format!("{prefix} (file path: {path})."));
            }
        }
    }
}

#[test]
fn real_metadata_and_only_landed_department_identity() {
    let r = Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled());
    assert!(!c.safe_fixes());
    assert_eq!(c.severity(), Severity::Convention);
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(c.path_policy().is_some());
    assert!(c.max_setting().is_none());
    assert_eq!(
        r.cops()
            .iter()
            .filter(|c| c.name().starts_with("Bundler/"))
            .map(|c| c.name())
            .collect::<Vec<_>>(),
        [
            COP,
            "Bundler/DuplicatedGem",
            "Bundler/DuplicatedGroup",
            "Bundler/GemComment",
            "Bundler/GemVersion",
            "Bundler/InsecureProtocolSource",
            "Bundler/OrderedGems"
        ]
    );
    let d = project();
    for name in ["Bundler/Future", "Gemspec/Future"] {
        check(
            &cli(d.path(), &["--only", name, "--stdin", "gems.rb"], Some(b"")),
            2,
            0,
        );
    }
    for key in [
        "max=2",
        "enforced_style='gems.rb'",
        "target_ruby_version='3.4'",
        "include=['**/*']",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        check(&cli(d.path(), &["--stdin", "gems.rb"], Some(b"")), 2, 0);
    }
}

#[test]
fn default_department_only_except_and_config_relative_exclusions() {
    let d = project();
    fs::create_dir(d.path().join("nested")).unwrap();
    fs::write(d.path().join("nested/gems.rb"), "puts 1\n").unwrap();
    for (config, flags, count) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", "Bundler"], 1),
        (String::new(), vec!["--only", COP, "--except", "Bundler"], 0),
        ("[cops.Bundler]\nenabled=false\n".into(), vec![], 0),
        (
            format!("[cops.\"{COP}\"]\nenabled=false\n"),
            vec!["--only", COP],
            1,
        ),
        (
            format!("[cops.Bundler]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
            vec![],
            1,
        ),
        ("exclude=['nested/**']\n".into(), vec!["--only", COP], 0),
        (
            format!("[cops.\"{COP}\"]\nexclude=['nested/**']\n"),
            vec!["--only", COP],
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = flags;
        args.push("gems.rb");
        let out = cli(&d.path().join("nested"), &args, None);
        check(&out, i32::from(count != 0), count);
        if count != 0 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("(file path: nested/gems.rb)."));
        }
    }
}

#[test]
fn ruby_directives_anchor_line_one_and_redundant_disable_composes() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Bundler", "all"] {
            for (source, count) in [
                (format!("# {prefix}:disable {target}\n"), 0),
                (format!("# {prefix}:todo {target}\n"), 0),
                (format!("\n# {prefix}:disable {target}\n"), 1),
                (format!("# {prefix}:disable-next {target}\nprint 1\n"), 1),
                (format!("__END__\n# {prefix}:disable {target}\n"), 1),
            ] {
                check(
                    &cli(
                        d.path(),
                        &["--only", COP, "--stdin", "gems.rb"],
                        Some(source.as_bytes()),
                    ),
                    i32::from(count != 0),
                    count,
                );
            }
        }
        let source = format!("# {prefix}:disable {COP}\n");
        let out = cli(
            d.path(),
            &[
                "--only",
                "Bundler/GemFilename,Lint/RedundantCopDisableDirective",
                "--stdin",
                "gems.rb",
            ],
            Some(source.as_bytes()),
        );
        check(&out, 0, 0);
        assert!(!String::from_utf8_lossy(&out.stdout).contains("Redundant"));
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled=false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\n");
        check(
            &cli(d.path(), &["--stdin", "gems.rb"], Some(source.as_bytes())),
            0,
            0,
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn stdin_valid_encoding_bom_crlf_data_and_opaque_binary_locks() {
    let d = project();
    for bytes in [
        b"".as_slice(),
        b"# comment\r\n",
        b"\xef\xbb\xbfprint 1\r\n",
        b"# encoding: Windows-1252\r\n# \xe9\r\n",
        b"__END__\n\xff",
    ] {
        let out = cli(
            d.path(),
            &["--only", COP, "--stdin", "nested/gems.rb"],
            Some(bytes),
        );
        check(&out, 1, 1);
        assert!(!String::from_utf8_lossy(&out.stdout).contains(":1:1:"));
    }
    for name in ["gems.locked", "Gemfile.lock"] {
        for bytes in [b"\xff\0def (((".as_slice(), b"# rubocop:disable all\n", b""] {
            check(
                &cli(
                    d.path(),
                    &[
                        "--only",
                        "Bundler,Lint/Syntax,Lint/Debugger",
                        "--stdin",
                        name,
                    ],
                    Some(bytes),
                ),
                i32::from(name == "gems.locked"),
                usize::from(name == "gems.locked"),
            );
        }
    }
    let out = cli(
        d.path(),
        &["--only", COP, "--format", "github", "--stdin", "gems.rb"],
        Some(b""),
    );
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("file=gems.rb"), "{text}");
    assert!(!text.contains("line=") && !text.contains("col="), "{text}");
}

#[test]
fn fatal_ruby_encoding_syntax_depth_and_io_cannot_be_bypassed() {
    let d = project();
    let deep = format!("{}1{}", "[".repeat(2000), "]".repeat(2000));
    for bytes in [
        b"def (".as_slice(),
        b"\xff",
        b"# encoding: invalid-encoding\n",
        deep.as_bytes(),
    ] {
        for selectors in [COP, "Bundler,Lint/Syntax"] {
            check(
                &cli(
                    d.path(),
                    &["--only", selectors, "--stdin", "gems.rb"],
                    Some(bytes),
                ),
                2,
                0,
            );
        }
    }
    check(
        &cli(d.path(), &["--only", COP, "missing/gems.rb"], None),
        2,
        0,
    );
    let out = cli(d.path(), &["--stdin", "gems.locked", "--fix"], Some(b""));
    check(&out, 2, 0);
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
        "{out:?}"
    );
}

#[test]
fn disk_no_write_no_companion_lookup_or_rename_and_old_cop_correction() {
    let d = project();
    for name in [
        "gems.rb",
        "gems.locked",
        "Gemfile",
        "Gemfile.lock",
        "other.gemfile",
    ] {
        let bytes = if name.ends_with("locked") || name.ends_with("lock") {
            b"\xff\0opaque".as_slice()
        } else {
            b""
        };
        let path = d.path().join(name);
        fs::write(&path, bytes).unwrap();
        let old = fs::metadata(&path).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP, name];
            if fix {
                args.push("--fix");
            }
            let count = usize::from(name == "gems.rb" || name == "gems.locked");
            check(&cli(d.path(), &args, None), i32::from(count != 0), count);
            unchanged(&path, bytes, &old);
        }
    }
    // A real canonical peer edit does not remove/repair the global finding.
    let path = d.path().join("gems.rb");
    fs::write(&path, "value=7\nputs value\n").unwrap();
    let out = cli(
        d.path(),
        &[
            "--fix",
            "--only",
            "Bundler,Layout/SpaceAroundOperators",
            "gems.rb",
        ],
        None,
    );
    check(&out, 1, 1);
    let after = b"value = 7\nputs value\n";
    assert_eq!(fs::read(&path).unwrap(), after);
    let old = fs::metadata(&path).unwrap();
    check(
        &cli(
            d.path(),
            &[
                "--fix",
                "--only",
                "Bundler,Layout/SpaceAroundOperators",
                "gems.rb",
            ],
            None,
        ),
        1,
        1,
    );
    unchanged(&path, after, &old);
    if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
        let a = native::run(&ruby, "value=7\nputs value\n");
        let b = native::run(&ruby, std::str::from_utf8(after).unwrap());
        assert_eq!(
            (a.status.code(), a.stdout, a.stderr),
            (b.status.code(), b.stdout, b.stderr)
        );
    }
}

#[test]
fn directory_discovery_ignores_hidden_vendor_and_config_excluded_inputs() {
    let d = project();
    for name in [
        "gems.rb",
        "Gemfile",
        "gems.locked",
        "Gemfile.lock",
        "nested/gems.rb",
        ".hidden/gems.rb",
        "vendor/gems.rb",
        "ignored/gems.rb",
        "excluded/gems.rb",
        "other.gemfile",
    ] {
        let path = d.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "puts 1\n").unwrap();
    }
    fs::write(d.path().join(".gitignore"), "ignored/\n").unwrap();
    fs::write(d.path().join("rustocop.toml"), "exclude=['excluded/**']\n").unwrap();
    let out = cli(d.path(), &["--only", "Bundler", "."], None);
    check(&out, 1, 3);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.ends_with("6 files inspected, 3 offenses detected\n"),
        "{text}"
    );
    for forbidden in [".hidden/", "vendor/", "ignored/", "excluded/"] {
        assert!(!text.contains(forbidden), "{text}");
    }
}

#[test]
fn native_syntax_only_and_supplied_ruby_is_never_executed() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = project();
    for bytes in [
        b"File.write('MUST_NOT_EXIST', 'no')\n".as_slice(),
        b"",
        b"\xef\xbb\xbfputs 1\r\n",
        b"# encoding: Windows-1252\n# \xe9\n",
        b"__END__\n\xff",
    ] {
        let mut child = Command::new("timeout")
            .args([
                "--kill-after=1s",
                "10",
                &ruby,
                "--parser=prism",
                "-W0",
                "-c",
            ])
            .current_dir(d.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        stdin_support::write_stdin(child.stdin.take().unwrap(), bytes, false).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{out:?}");
        check(
            &cli(
                d.path(),
                &["--only", COP, "--stdin", "gems.rb"],
                Some(bytes),
            ),
            1,
            1,
        );
        assert!(!d.path().join("MUST_NOT_EXIST").exists());
    }
}

#[cfg(unix)]
#[test]
fn symlink_lexical_names_and_opaque_alias_veto_remain() {
    use std::os::unix::fs::symlink;
    let d = project();
    let target = d.path().join("target.rb");
    fs::write(&target, "value=7\n").unwrap();
    symlink("target.rb", d.path().join("gems.rb")).unwrap();
    symlink("target.rb", d.path().join("gems.locked")).unwrap();
    let old = fs::metadata(&target).unwrap();
    let out = cli(
        d.path(),
        &[
            "--fix",
            "--only",
            "Bundler,Layout/SpaceAroundOperators",
            "gems.rb",
            "gems.locked",
        ],
        None,
    );
    check(&out, 1, 2);
    unchanged(&target, b"value=7\n", &old);
    assert!(
        fs::symlink_metadata(d.path().join("gems.rb"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    symlink("gems.locked", d.path().join("alias.rb")).unwrap();
    check(&cli(d.path(), &["--only", COP, "alias.rb"], None), 0, 0);
}
