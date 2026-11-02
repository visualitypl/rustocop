//! Actual CLI, source-only reference replay and authored native proofs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/DuplicateMagicComment";
const SAFE: &[u8] = b"# frozen_string_literal: true\n# frozen_string_literal: true\n";
const AFTER: &[u8] = b"# frozen_string_literal: true\n";
const HELD: &[u8] = b"# frozen_string_literal: true\n# frozen_string_literal: false\n";
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
        assert_eq!(
            out.status.code(),
            Some(code),
            "{}\n{args:?}\n{out:?}",
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
    serde_json::from_str(include_str!("fixtures/duplicate_magic_comment.json")).unwrap()
}
fn assert_findings(out: &Output, row: &serde_json::Value) {
    let findings = row["findings"].as_array().unwrap();
    assert_eq!(count(out), findings.len(), "{row}\n{out:?}");
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
#[test]
fn all_reference_rows_exact_cli_no_write_canonical_deletions_and_idempotence() {
    let dir = project();
    let mut fixed = 0;
    assert_eq!(rows().len(), 342);
    for row in rows() {
        let source = row["source"].as_str().unwrap().as_bytes();
        let code = i32::from(!row["findings"].as_array().unwrap().is_empty());
        assert_findings(
            &unchanged(dir.path(), source, &["--only", COP, "lib/a.rb"], Some(code)),
            &row,
        );
        let after = row["project_after"].as_str().unwrap().as_bytes();
        if after == source {
            assert_findings(
                &unchanged(
                    dir.path(),
                    source,
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(code),
                ),
                &row,
            );
        } else {
            fixed += 1;
            let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{row}\n{out:?}");
            assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), after);
            unchanged(
                dir.path(),
                after,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
    assert_eq!(fixed, 10);
}
#[test]
fn original_legacy_bytes_and_projected_token_boundaries_match_exact_pin() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_magic_bytes.json")).unwrap();
    assert_eq!(rows.len(), 42);
    let dir = project();
    for row in rows {
        let source: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
        let code = i32::from(!row["findings"].as_array().unwrap().is_empty());
        for args in [
            vec!["--only", COP, "lib/a.rb"],
            vec!["--only", COP, "--fix", "lib/a.rb"],
        ] {
            assert_findings(&unchanged(dir.path(), &source, &args, Some(code)), &row);
        }
    }
}
#[test]
fn defaults_gates_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 491);
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
        assert_eq!(count(&unchanged(dir.path(), HELD, &args, None)), n);
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
        unchanged(
            dir.path(),
            SAFE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn both_prefixes_multiline_embedded_document_next_push_and_force_enable_scopes() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for target in [COP, "Lint", "all"] {
            for source in [
                format!(
                    "# {prefix}:disable {target}\n{}",
                    String::from_utf8_lossy(HELD)
                ),
                format!(
                    "# {prefix}:push\n# {prefix}:disable {target}\n{}# {prefix}:pop\n",
                    String::from_utf8_lossy(HELD)
                ),
                format!(
                    "# {prefix}:disable {target}\n=begin\n# frozen_string_literal: true\n# frozen_string_literal: false\n=end\n"
                ),
            ] {
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(0),
                );
            }
        }
        // Next scopes attach to the next executable statement, not a header
        // comment. They cannot silence/force a finding earlier in that header.
        for mode in [format!("disable-next {COP}"), format!("next -{COP}")] {
            let source = format!(
                "# frozen_string_literal: true\n# {prefix}:{mode}\n# frozen_string_literal: false\nnil\n"
            );
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1),
            );
        }
        for (open, expected) in [
            (format!("# {prefix}:push +{COP}"), 1),
            (format!("# {prefix}:enable-next {COP}"), 0),
        ] {
            let source = format!(
                "# {prefix}:disable {COP}\n{}\n{open}\n# frozen_string_literal: true\n# {prefix}:pop\n# frozen_string_literal: true\n",
                String::from_utf8_lossy(HELD)
            );
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(expected)
                )),
                expected as usize
            );
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\n{}", String::from_utf8_lossy(HELD));
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
fn supplied_file_stdin_paths_built_in_exclusions_and_stdin_fix_rejection() {
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
        fs::write(&p, HELD).unwrap();
        let n = usize::from(!path.starts_with("vendor/") && path != "db/schema.rb");
        assert_eq!(count(&run(dir.path(), &["--only", COP, path])), n);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(HELD).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), n, "{out:?}");
        if n == 1 {
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:2:1:")),
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
#[cfg(unix)]
#[test]
fn atomic_validation_readonly_fd_symlinks_and_actual_peer_composition() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, HELD).unwrap();
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
    }
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/link.rb"])
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
        &[SAFE, b"(\n"].concat(),
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    fs::write(&path, SAFE).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    let mut fd = fs::File::open(&path).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(fs::read(&path).unwrap(), AFTER);
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    let mut old_bytes = vec![];
    fd.read_to_end(&mut old_bytes).unwrap();
    assert_eq!(old_bytes, SAFE);
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
            "# frozen_string_literal: true  \n# frozen_string_literal: true\n",
            "# frozen_string_literal: true\n# frozen_string_literal: true\n",
            "Layout/TrailingWhitespace",
        ),
        (
            "# frozen-string-literal: true\n# frozen_string_literal: true\nnil\n",
            "# frozen_string_literal: true\n# frozen_string_literal: true\nnil\n",
            "Style/MagicCommentFormat",
        ),
        (
            "# frozen_string_literal: true\n# frozen_string_literal: true\n\n\n",
            "# frozen_string_literal: true\n",
            "Layout/TrailingEmptyLines",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let only = format!("{COP},{peers}");
        run(dir.path(), &["--only", &only, "--fix", "lib/a.rb"]);
        assert_eq!(fs::read_to_string(&path).unwrap(), after, "{peers}");
    }
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
fn authored_native_canonical_nil_binding_instructions_trace_heap_and_counterexamples() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    // Independently authored closed comments-only witnesses. No reference/spec
    // program is executed; actual original spec text is only syntax-checked below.
    for (before, after) in [
        (
            "# frozen_string_literal: true\n# frozen_string_literal: true\n",
            "# frozen_string_literal: true\n",
        ),
        (
            "\n# frozen_string_literal: true\r\n# frozen_string_literal: true\r\n",
            "\n# frozen_string_literal: true\r\n",
        ),
        (
            "# frozen_string_literal: true\n\n# frozen_string_literal: true",
            "# frozen_string_literal: true\n\n",
        ),
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
        let a = native(&ruby, before);
        let b = native(&ruby, after);
        assert!(a.status.success() && b.status.success());
        assert_eq!(a.stdout, b.stdout);
        assert!(a.stderr.is_empty() && b.stderr.is_empty());
        for probe in [
            "p [i.eval, i.to_a, i.disasm.lines.grep(/putnil|leave/).map(&:strip)]",
            "events = []; t = TracePoint.new(:line, :call, :return, :raise) { |e| events << [e.event, e.lineno] if e.path == 'authored.rb' }; t.enable; value = i.eval; t.disable; p [value, events, i.to_a[10]]",
            "GC.disable; i.eval; n = GC.stat(:total_allocated_objects); 100.times { i.eval }; p GC.stat(:total_allocated_objects) - n",
        ] {
            let a = native(
                &ruby,
                &format!(
                    "i = RubyVM::InstructionSequence.compile({}, 'authored.rb', 'authored.rb', 1); {probe}",
                    serde_json::to_string(before).unwrap()
                ),
            );
            let b = native(
                &ruby,
                &format!(
                    "i = RubyVM::InstructionSequence.compile({}, 'authored.rb', 'authored.rb', 1); {probe}",
                    serde_json::to_string(after).unwrap()
                ),
            );
            assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
            assert_eq!(a.stdout, b.stdout, "{probe}");
            assert_eq!(a.stderr, b.stderr);
            assert!(a.stderr.is_empty(), "{a:?}");
        }
    }
    // Real differences from upstream's general rewrite: last pragma wins,
    // warnings, executable line events, and encoding activation after a shebang.
    for (before, after) in [
        (
            "# frozen_string_literal: true\n# frozen_string_literal: false\np 'authored'.frozen?\n",
            "# frozen_string_literal: true\np 'authored'.frozen?\n",
        ),
        (
            "# frozen_string_literal: true\n# frozen_string_literal: invalid\n",
            "# frozen_string_literal: true\n",
        ),
        (
            "# frozen_string_literal: true\n# frozen_string_literal: true\np __LINE__\n",
            "# frozen_string_literal: true\np __LINE__\n",
        ),
        (
            "#!/usr/bin/env -*- encoding: utf-8 -*- ruby\n# encoding: ascii\np __ENCODING__\n",
            "#!/usr/bin/env -*- encoding: utf-8 -*- ruby\np __ENCODING__\n",
        ),
    ] {
        let a = native(&ruby, before);
        let b = native(&ruby, after);
        assert!(
            a.stdout != b.stdout || a.stderr != b.stderr || a.status.code() != b.status.code(),
            "{a:?}\n{b:?}"
        );
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
}
#[test]
fn authored_native_source_data_error_and_vm_observers_retain_findings_without_edits() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    for suffix in [
        "p DATA.pos\n__END__\nauthored payload\n",
        "p File.binread(__FILE__).bytesize\n",
        "p RubyVM::InstructionSequence.compile_file(__FILE__).to_a[4][:code_location]\n",
        "raise 'authored error'\n",
    ] {
        let before = [SAFE, suffix.as_bytes()].concat();
        let after = [AFTER, suffix.as_bytes()].concat();
        let path = dir.path().join("lib/a.rb");
        let execute = |source: &[u8]| {
            fs::write(&path, source).unwrap();
            Command::new(&ruby)
                .args(["--parser=prism", "-w"])
                .arg(&path)
                .output()
                .unwrap()
        };
        let a = execute(&before);
        let b = execute(&after);
        assert!(
            a.stdout != b.stdout || a.stderr != b.stderr,
            "{suffix}\n{a:?}\n{b:?}"
        );
        unchanged(
            dir.path(),
            &before,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
}

#[test]
fn reference_sources_and_upstream_outputs_native_syntax_only() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let mut sources: Vec<(Vec<u8>, bool)> = rows()
        .into_iter()
        .flat_map(|r| {
            ["source", "upstream_after"].map(|key| {
                (
                    r[key].as_str().unwrap().as_bytes().to_vec(),
                    r["native_loader_rejection"].is_string(),
                )
            })
        })
        .collect();
    let bytes: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_magic_bytes.json")).unwrap();
    sources.extend(bytes.into_iter().map(|r| {
        (
            serde_json::from_value::<Vec<u8>>(r["bytes"].clone()).unwrap(),
            false,
        )
    }));
    for (source, loader_rejected) in sources {
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-c", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&source).unwrap();
        let out = child.wait_with_output().unwrap();
        if loader_rejected {
            assert!(!out.status.success());
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("invalid option -*"),
                "{out:?}"
            );
        } else {
            assert!(
                out.status.success(),
                "{}\n{out:?}",
                String::from_utf8_lossy(&source)
            );
        }
    }
}
