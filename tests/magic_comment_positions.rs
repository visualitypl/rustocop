//! Source-only reference parity, actual CLI and authored native holds.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
#[macro_use]
#[path = "support/parallel_rows.rs"]
mod parallel_rows;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = ["Lint/MisplacedMagicComment", "Lint/OrderedMagicComments"];
const SOURCES: [&str; 2] = [
    "nil\n# frozen_string_literal: true\n",
    "# frozen_string_literal: true\n# encoding: ascii\n",
];
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
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(code) = code {
        assert_eq!(
            out.status.code(),
            Some(code),
            "{args:?}\n{out:?}\n{}",
            String::from_utf8_lossy(source)
        );
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source, "{out:?}");
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
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/magic_comment_positions.json")).unwrap()
}
fn source(row: &serde_json::Value) -> Vec<u8> {
    row["source"]
        .as_str()
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_else(|| serde_json::from_value(row["bytes"].clone()).unwrap())
}
#[test]
fn every_pinned_spec_source_shared_alias_byte_and_control_row_exact_cli_no_write() {
    assert_eq!(rows().len(), 1040);
    parallel_rows!(for row in rows() => {
        let dir = project();
        for cop in COPS {
            let findings = row[cop]["findings"].as_array().unwrap();
            let code = i32::from(!findings.is_empty());
            for fix in [false, true] {
                let mut args = vec!["--only", cop, "lib/a.rb"];
                if fix {
                    args.push("--fix");
                }
                let out = unchanged(dir.path(), &source(&row), &args, Some(code));
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
fn actual_metadata_defaults_selectors_and_strict_toml_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 582);
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let detector = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(detector.default_enabled() && !detector.safe_fixes());
        assert_eq!(detector.ruby_versions(), Default::default());
        assert!(detector.path_policy().is_none() && detector.max_setting().is_none());
        assert!(detector.autocorrect_incompatible_with().is_empty());
        let dir = project();
        for (config, selectors, n) in [
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
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
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
        for config in [
            "exclude = ['lib/**']\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
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
fn both_directive_prefixes_range_next_signed_push_pop_and_explicit_source_enable() {
    let dir = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Lint", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!(
                        "# {prefix}:push\n# {prefix}:disable {target}\n{source}# {prefix}:pop\n"
                    ),
                ] {
                    unchanged(
                        dir.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        Some(0),
                    );
                }
            }
            // Next scopes attach to executable statements, not header comments.
            let header = format!(
                "# {prefix}:disable-next {cop}\n# frozen_string_literal: true\n# encoding: ascii\nnil\n"
            );
            let out = unchanged(
                dir.path(),
                header.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(1),
            );
            assert_eq!(count(&out, cop), 1);
            for mode in [format!("disable-next {cop}"), format!("next -{cop}")] {
                let source = format!("# {prefix}:{mode}\nnil # frozen_string_literal: true\n");
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COPS[0], "--fix", "lib/a.rb"],
                    Some(i32::from(cop != COPS[0])),
                );
            }
            for opener in [
                format!("# {prefix}:push +{cop}"),
                format!("# {prefix}:enable {cop}"),
            ] {
                let source =
                    format!("# {prefix}:disable {cop}\n{opener}\n{source}# {prefix}:pop\n");
                assert_eq!(
                    count(
                        &unchanged(
                            dir.path(),
                            source.as_bytes(),
                            &["--only", cop, "--fix", "lib/a.rb"],
                            Some(1)
                        ),
                        cop
                    ),
                    1
                );
            }
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            let source = format!("# {prefix}:enable {cop}\n{source}");
            assert_eq!(
                count(
                    &unchanged(dir.path(), source.as_bytes(), &["lib/a.rb"], None),
                    cop
                ),
                1
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
            "db/schema.rb",
            "vendor/a.rb",
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
                    String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:2:1:")),
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
fn readonly_symlink_fd_invalid_input_atomic_failure_and_real_peer_correction() {
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
        let before = source.replace("true", "true  ");
        fs::write(&path, &before).unwrap();
        let only = format!("{cop},Layout/TrailingWhitespace");
        run(dir.path(), &["--only", &only, "--fix", "lib/link.rb"]);
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
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COPS[0], "--fix", "lib/dangling.rb"])
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
fn authored_native_unsafe_activation_value_encoding_warning_position_and_heap_controls() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    // Independently authored bounded programs, not executed reference text.
    for (cop, before, after) in [
        (
            COPS[0],
            "nil\n# frozen_string_literal: true\np 'authored'.frozen?\n",
            "# frozen_string_literal: true\nnil\np 'authored'.frozen?\n",
        ),
        (
            COPS[0],
            "# authored documentation\n# encoding: ascii\np __ENCODING__\n",
            "# encoding: ascii\n# authored documentation\np __ENCODING__\n",
        ),
        (
            COPS[1],
            "# frozen_string_literal: true\n# encoding: ascii\np __ENCODING__\n",
            "# encoding: ascii\n# frozen_string_literal: true\np __ENCODING__\n",
        ),
        (
            COPS[0],
            "nil\n# frozen_string_literal: invalid\n",
            "# frozen_string_literal: invalid\nnil\n",
        ),
        (
            COPS[0],
            "p __LINE__\n# frozen_string_literal: true\n",
            "# frozen_string_literal: true\np __LINE__\n",
        ),
        (
            COPS[0],
            "nil\n# frozen_string_literal: true\nGC.disable; 2.times { 'authored' }; n = GC.stat(:total_allocated_objects); 100.times { 'authored' }; p GC.stat(:total_allocated_objects) - n\n",
            "# frozen_string_literal: true\nnil\nGC.disable; 2.times { 'authored' }; n = GC.stat(:total_allocated_objects); 100.times { 'authored' }; p GC.stat(:total_allocated_objects) - n\n",
        ),
    ] {
        let a = native(&ruby, before);
        let b = native(&ruby, after);
        assert!(
            a.stdout != b.stdout || a.stderr != b.stderr || a.status.code() != b.status.code(),
            "{before}\n{a:?}\n{b:?}"
        );
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", cop, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    // A shebang finding is not an invitation to move comments or mutate mode.
    unchanged(
        dir.path(),
        b"# frozen_string_literal: true\n#!/usr/bin/env ruby\n",
        &["--only", COPS[0], "--fix", "lib/a.rb"],
        Some(1),
    );
}
#[test]
fn authored_native_file_source_data_vm_error_bom_crlf_and_heredoc_observers_hold() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for suffix in [
        "p File.binread(__FILE__).bytesize\n",
        "p DATA.pos\n__END__\nauthored payload\n",
        "p RubyVM::InstructionSequence.compile_file(__FILE__).to_a[4][:code_location]\n",
        "raise 'authored failure'\n",
        "p <<~TEXT\n  authored #{__LINE__}\nTEXT\n",
    ] {
        let before = format!("nil\n# frozen_string_literal: true\n{suffix}");
        let after = format!("# frozen_string_literal: true\nnil\n{suffix}");
        // Source movement with identical total length can still change compile
        // warnings/values, independent of observer locations.
        let execute = |s: &str| {
            fs::write(&path, s).unwrap();
            Command::new(&ruby)
                .args(["--parser=prism", "-w"])
                .arg(&path)
                .output()
                .unwrap()
        };
        let a = execute(&before);
        let b = execute(&after);
        assert!(a.stdout != b.stdout || a.stderr != b.stderr, "{a:?}\n{b:?}");
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COPS[0], "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    for before in [
        "\u{feff}nil\r\n# frozen_string_literal: true\r\n",
        "nil\r\n# encoding: ascii\r\n",
        "# frozen_string_literal: true\r\n# encoding: ascii\r\n",
    ] {
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", &COPS.join(","), "--fix", "lib/a.rb"],
            Some(1),
        );
    }
}
#[test]
fn all_reference_inputs_and_main_canonical_outputs_native_syntax_only() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        let mut sources = vec![source(&row)];
        if COPS.contains(&row["origin"].as_str().unwrap()) {
            for cop in COPS {
                sources.push(
                    row[cop]["upstream_after"]
                        .as_str()
                        .unwrap()
                        .as_bytes()
                        .to_vec(),
                );
            }
        }
        for source in sources {
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
