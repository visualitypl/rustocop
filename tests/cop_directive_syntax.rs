//! Pinned examples port RuboCop (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, lint::CopDirectiveSyntax},
    pipeline::lint_one,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/CopDirectiveSyntax";
const BEFORE: &str = "# rubocop:disable Style/For # rubocop:disable Style/For\n";
const AFTER: &str = "# rubocop:disable Style/For\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], code: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        // Isolate syntax policy from the separately tested open-range policy.
        .args([
            "--except",
            "Lint/MissingCopEnableDirective,Lint/RedundantCopDisableDirective",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    out
}
fn fix(root: &Path, before: &[u8], after: &[u8]) {
    let p = root.join("lib/a.rb");
    fs::write(&p, before).unwrap();
    let old = fs::metadata(&p).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(["--only", COP, "--fix", "lib/a.rb"])
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(fs::read(&p).unwrap(), after, "{before:?}: {out:?}");
    let new = fs::metadata(&p).unwrap();
    assert_eq!(old.permissions(), new.permissions());
    if before == after {
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    }
}
#[test]
fn all_51_pinned_calls_exact_diagnostics_and_all_seven_correction_dispositions() {
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cop_directive_syntax_pinned.json")).unwrap();
    let dir = project();
    let mut corrections = 0;
    let mut holds = 0;
    for row in rows.as_array().unwrap() {
        let source = row["source"].as_str().unwrap();
        let report = lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(source.as_bytes()),
            &[&CopDirectiveSyntax],
            &ResolvedConfig::default(),
        );
        assert!(report.errors.is_empty(), "{row:?}");
        assert!(report.internal_errors.is_empty());
        let actual: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| (o.line, o.col, o.message.as_str()))
            .collect();
        let expected: Vec<_> = row["positions"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["messages"].as_array().unwrap())
            .map(|(p, m)| {
                (
                    p[0].as_u64().unwrap() as u32,
                    p[1].as_u64().unwrap() as u32,
                    m.as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(actual, expected, "{row:?}");
        let after = if let Some(after) = row["correction"].as_str() {
            corrections += 1;
            // Upstream's first witness changes the effective suppression of
            // registered Style/Encoding, and the merges into a list naming
            // Metrics/AbcSize add an active suppression
            // of it. Keep the diagnostic, not that edit.
            if row["spec_line"] == 48 || after.contains("Metrics/AbcSize") {
                source
            } else {
                after
            }
        } else {
            if row["no_correction"] == true {
                holds += 1;
            }
            source
        };
        fix(dir.path(), source.as_bytes(), after.as_bytes());
    }
    assert_eq!((corrections, holds), (4, 3));
}
#[test]
fn all_14_pinned_source_documentation_examples_use_actual_comment_investigation() {
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cop_directive_syntax_source.json")).unwrap();
    let dir = project();
    for row in rows.as_array().unwrap() {
        let source = row["source"].as_str().unwrap();
        let report = lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(source.as_bytes()),
            &[&CopDirectiveSyntax],
            &ResolvedConfig::default(),
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row:?}"
        );
        let actual: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| (o.line, o.col, o.message.as_str()))
            .collect();
        let expected: Vec<_> = row["positions"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["messages"].as_array().unwrap())
            .map(|(p, m)| {
                (
                    p[0].as_u64().unwrap() as u32,
                    p[1].as_u64().unwrap() as u32,
                    m.as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(actual, expected, "{row:?}");
        fix(dir.path(), source.as_bytes(), source.as_bytes());
    }
}
#[test]
fn actual_registry_defaults_cli_precedence_and_no_write() {
    let r = Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled() && c.safe_fixes());
    assert!(c.path_policy().is_none() && c.max_setting().is_none());
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(r.has_cop("Lint/Syntax"));
    let dir = project();
    let p = dir.path().join("lib/a.rb");
    fs::write(&p, BEFORE).unwrap();
    let old = fs::metadata(&p).unwrap();
    for (config, flags, code) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint", "--except", COP], 0),
        (
            String::new(),
            vec!["--only", COP, "--except", COP, "--fix"],
            0,
        ),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            1,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            1,
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = flags;
        args.push("lib/a.rb");
        let out = cli(dir.path(), &args, code);
        if code == 1 {
            assert!(
                String::from_utf8_lossy(&out.stdout)
                    .contains("lib/a.rb:1:1: E: Malformed directive comment detected.")
            );
        }
        assert_eq!(fs::read(&p).unwrap(), BEFORE.as_bytes());
        let new = fs::metadata(&p).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
    }
}
#[test]
fn strict_toml_exclusions_and_supplied_paths_apply_to_files_and_stdin() {
    let dir = project();
    for key in [
        "max = 2",
        "include = ['**/*']",
        "EnforcedStyle = 'x'",
        "enabled = 'true'",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        let p = dir.path().join("lib/a.rb");
        fs::write(&p, BEFORE).unwrap();
        let old = fs::metadata(&p).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 2);
        assert_eq!(fs::read(&p).unwrap(), BEFORE.as_bytes());
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), old);
    }
    for config in [
        "exclude = ['lib/a.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(dir.path().join("lib/a.rb"), BEFORE).unwrap();
        cli(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "../lib/a.rb"],
            0,
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BEFORE
        );
        for (path, code) in [("lib/a.rb", 0), ("virtual.rb", 1)] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", COP, "--stdin", path])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(BEFORE.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.status.code(), Some(code), "{out:?}");
            if code == 1 {
                assert!(String::from_utf8_lossy(&out.stdout).contains("virtual.rb:1:1"));
            }
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    cli(
        dir.path(),
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        2,
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
        BEFORE
    );
}
#[test]
fn both_prefixes_all_suppression_scopes_and_source_optins() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for scope in [
            format!("# {prefix}:disable {COP}\n{BEFORE}"),
            format!("# {prefix}:push -{COP}\n{BEFORE}# {prefix}:pop\n"),
            format!("# {prefix}:disable-next {COP}\nfoo # rubocop:disabled Style/For\n"),
            format!("# {prefix}:next -{COP}\nfoo(\n # rubocop:disabled Style/For\n)\n"),
        ] {
            fix(dir.path(), scope.as_bytes(), scope.as_bytes());
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        for mode in [
            format!("enable {COP}"),
            format!("push +{COP}"),
            format!("enable-next {COP}"),
            format!("next +{COP}"),
        ] {
            let source = format!("# {prefix}:{mode}\nfoo # rubocop:disabled Style/For\n");
            fs::write(dir.path().join("lib/a.rb"), source).unwrap();
            let out = cli(dir.path(), &["lib/a.rb"], 1);
            assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
            cli(dir.path(), &["--except", COP, "lib/a.rb"], 0);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn qualified_names_are_valid_but_do_not_change_literal_suppression() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for name in ["Style/LineLength", "line_length", "Layout/LineLength"] {
            let source = format!("# {prefix}:disable {name}\n# {}\n", "x".repeat(130));
            fs::write(dir.path().join("lib/a.rb"), &source).unwrap();
            let out = cli(dir.path(), &["--only", COP, "lib/a.rb"], 0);
            assert!(!String::from_utf8_lossy(&out.stdout).contains("Unknown cop name"));
            let suppressed = name == "Layout/LineLength";
            let out = cli(
                dir.path(),
                &[
                    "--only",
                    "Lint/CopDirectiveSyntax,Layout/LineLength",
                    "lib/a.rb",
                ],
                if suppressed { 0 } else { 1 },
            );
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(!stdout.contains("Unknown cop name"), "{stdout}");
            assert_eq!(
                stdout.contains("Layout/LineLength"),
                !suppressed,
                "{stdout}"
            );
            assert_eq!(
                fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
                source
            );
        }
    }
}
#[test]
fn source_envelopes_encoding_and_global_observers_use_existing_guards() {
    let dir = project();
    for (before, after) in [
        (
            format!("\u{feff}{BEFORE}p __LINE__\n"),
            format!("\u{feff}{AFTER}p __LINE__\n"),
        ),
        (
            format!("{}p __LINE__\r\n", BEFORE.replace('\n', "\r\n")),
            format!("{}p __LINE__\r\n", AFTER.replace('\n', "\r\n")),
        ),
        (
            format!("p <<~H\n{BEFORE}H\n{BEFORE}"),
            format!("p <<~H\n{BEFORE}H\n{AFTER}"),
        ),
        (
            "# rubocop:disable Style/For -- encoding: binary # rubocop:disable Style/For\n".into(),
            "# rubocop:disable Style/For -- encoding: binary # rubocop:disable Style/For\n".into(),
        ),
    ] {
        fix(dir.path(), before.as_bytes(), after.as_bytes());
    }
    let before = [
        b"# coding: ISO-8859-1\n# legacy \xe9\n".as_slice(),
        BEFORE.as_bytes(),
        b"p __LINE__\n",
    ]
    .concat();
    let after = [
        b"# coding: ISO-8859-1\n# legacy \xe9\n".as_slice(),
        AFTER.as_bytes(),
        b"p __LINE__\n",
    ]
    .concat();
    fix(dir.path(), &before, &after);
    fix(
        dir.path(),
        format!("{BEFORE}p 1\n__END__\nraw\n").as_bytes(),
        format!("{AFTER}p 1\n__END__\nraw\n").as_bytes(),
    );
    // Reading `DATA` is a listed observer.
    for observer in [
        "p DATA.read\n__END__\nraw\n",
        "p File.binread(__FILE__)\n",
        "p DATA.pos\n__END__\nraw\n",
        "p RubyVM::InstructionSequence.of(method(:f))\ndef f; 1; end\n",
        "p ObjectSpace.count_objects\n",
        "p retained.source\n",
    ] {
        let source = format!("{BEFORE}{observer}");
        fix(dir.path(), source.as_bytes(), source.as_bytes());
    }
}
#[test]
fn suppression_changing_merges_are_withheld_even_for_clean_unselected_or_disabled_targets() {
    let dir = project();
    for config in [
        String::new(),
        "[cops.\"Style/StringLiterals\"]\nenabled = false\n".into(),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for second in [
            "Style/StringLiterals",
            "Style/FrozenStringLiteralComment",
            "Lint/Debugger",
            "Style",
        ] {
            let source =
                format!("# rubocop:disable Style/For # rubocop:disable {second}\ntext = 'value'\n");
            fix(dir.path(), source.as_bytes(), source.as_bytes());
        }
        let source =
            "# rubocop:enable Style # rubocop:enable Style/StringLiterals\ntext = 'value'\n";
        fix(dir.path(), source.as_bytes(), source.as_bytes());
    }
}
#[test]
fn lint_style_layout_naming_security_composition_preserves_effective_suppression() {
    let dir = project();
    let p = dir.path().join("lib/a.rb");
    let before = "# rubocop:disable Style/StringLiterals # rubocop:disable Style/StringLiterals\nbadName='value'\n[1].hash\n";
    let after = "# rubocop:disable Style/StringLiterals\nbadName = 'value'\n[1].hash\n";
    fs::write(&p, before).unwrap();
    let only = format!(
        "{COP},Style/StringLiterals,Layout/SpaceAroundOperators,Naming/VariableName,Security/CompoundHash"
    );
    let out = cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 1);
    assert_eq!(fs::read_to_string(&p).unwrap(), after, "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Naming/VariableName") && text.contains("Security/CompoundHash"),
        "{text}"
    );
    assert!(!text.contains("(Style/StringLiterals)"));
    let old = fs::metadata(&p).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 1);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), old);
}
#[test]
fn repeated_markers_and_many_comments_do_not_require_root_rescans() {
    let source = format!(
        "# {}\n",
        "rubocop:disable Style/For # "
            .repeat(3000)
            .trim_end_matches(" # ")
    );
    let dir = project();
    fix(dir.path(), source.as_bytes(), AFTER.as_bytes());
    fix(
        dir.path(),
        BEFORE.repeat(1000).as_bytes(),
        AFTER.repeat(1000).as_bytes(),
    );
}
#[test]
fn native_ruby34_controlled_binding_evaluation_flow_values_and_positions_match() {
    let Some(ruby) = std::env::var_os("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    // Only this owned, finite runtime witness is executed, never spec/corpus inputs.
    let body = "events = []\ndef f(events)\n events << :once\n [1, 2].map { |x| next x + 1 }\nensure\n events << :ensure\nend\np [f(events), events, __LINE__, 'value'.encoding.name]\np binding.local_variables.sort\n";
    let before = format!("{BEFORE}{body}");
    let after = format!("{AFTER}{body}");
    fix(dir.path(), before.as_bytes(), after.as_bytes());
    let a = dir.path().join("before.rb");
    let b = dir.path().join("after.rb");
    fs::write(&a, before).unwrap();
    fs::write(&b, after).unwrap();
    let run = |p: &Path| Command::new(&ruby).arg(p).output().unwrap();
    let a = run(&a);
    let b = run(&b);
    assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
    assert_eq!((a.stdout, a.stderr), (b.stdout, b.stderr));
}
#[test]
fn parse_failure_is_no_write_and_symlink_fix_preserves_mode() {
    let dir = project();
    let p = dir.path().join("lib/a.rb");
    let source = format!("{BEFORE}def broken(\n");
    fs::write(&p, &source).unwrap();
    let old = fs::metadata(&p).unwrap();
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 2);
    assert_eq!(fs::read_to_string(&p).unwrap(), source);
    assert_eq!(
        fs::metadata(&p).unwrap().modified().unwrap(),
        old.modified().unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        fs::write(&p, BEFORE).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o751)).unwrap();
        symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/link.rb"], 0);
        assert!(
            fs::symlink_metadata(dir.path().join("lib/link.rb"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&p).unwrap(), AFTER);
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o751
        );
    }
}
