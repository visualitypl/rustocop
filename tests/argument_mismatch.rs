//! production no-index prerequisite and actual CLI/no-write controls.
//! Indexed expectations are retained verbatim and exercised privately in the cop.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;
const COP: &str = "Lint/ArgumentMismatch";
const SOURCE: &str = "# Report.\nclass Report\n  def self.generate(source, format); end\nend\nReport.generate(source)\n";
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(COP))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}\n{source:?}");
    assert_eq!(fs::read(&path).unwrap(), source, "{out:?}");
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}
/// `expect_early_error` only for runs whose asserted exit is the early
/// `--stdin --fix` rejection, which may exit before reading stdin.
fn stdin(root: &Path, source: &[u8], args: &[&str], expect_early_error: bool) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    stdin_support::write_stdin(child.stdin.take().unwrap(), source, expect_early_error).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn all_reference_sources_without_assigned_index_have_no_pipeline_findings() {
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/argument_mismatch.json")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 347);
    let mut differences = Vec::new();
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    for row in rows.as_array().unwrap() {
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(row["source"].as_str().unwrap().as_bytes()),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{report:?}"
        );
        let found: Vec<_> = report
            .result
            .offenses
            .into_iter()
            .filter(|o| o.cop == COP)
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Warning);
                (o.line, o.col, o.end_line, o.message)
            })
            .collect();
        // Indexed expectations are exercised directly by the private cop tests.
        // Ordinary pipeline inputs never supply that prerequisite.
        if !found.is_empty() {
            differences.push(format!("{row}\nactual: {found:?}"));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
#[test]
fn lead_default_probe_and_source_forces_cannot_supply_an_index() {
    let dir = project_with_lib();
    let source = "class A\n  def self.f(a); end\nend\nA.f\n";
    for (args, status) in [
        (vec!["lib/a.rb"], 1), // Other departments still report their own findings.
        (vec!["--only", COP, "lib/a.rb"], 0),
        (vec!["--only", COP, "--fix", "lib/a.rb"], 0),
    ] {
        let out = unchanged(dir.path(), source.as_bytes(), &args, status);
        assert_eq!(count(&out), 0);
    }
    for prefix in ["rustocop", "rubocop"] {
        for disabled in ["Lint", COP] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{disabled}\"]\nenabled = false\n"),
            )
            .unwrap();
            let forced = source.replace("A.f\n", &format!("# {prefix}:enable-next {COP}\nA.f\n"));
            let out = unchanged(
                dir.path(),
                forced.as_bytes(),
                &[
                    "--except",
                    "Style,Naming,Layout,Security",
                    "--fix",
                    "lib/a.rb",
                ],
                0,
            );
            assert_eq!(count(&out), 0);
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let out = unchanged(
        dir.path(),
        format!("{source}binding.pry\n").as_bytes(),
        &["--only", "Lint/ArgumentMismatch,Lint/Debugger", "lib/a.rb"],
        1,
    );
    assert_eq!(count(&out), 0);
    assert!(String::from_utf8_lossy(&out.stdout).contains("Lint/Debugger"));
}
#[test]
fn default_selectors_strict_toml_and_metadata() {
    let dir = project_with_lib();
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(
        cop.ruby_versions(),
        rustocop::cops::policy::RubyVersions::default()
    );
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
    for (config, select, expected) in [
        (String::new(), vec![], 0),
        (String::new(), vec!["--only", COP], 0),
        (String::new(), vec!["--only", "Lint", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            0,
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            0,
        ),
    ] {
        // Department forcing now genuinely opts into ConstantResolution;
        // its Report read must not be mistaken for an indexed argument finding.
        let resolution_active =
            select.contains(&"Lint") || config.contains("[cops.Lint]\nenabled = true\n");
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = select;
        args.push("lib/a.rb");
        let out = unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &args,
            i32::from(expected > 0 || resolution_active),
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).contains("(Lint/ConstantResolution)"),
            resolution_active,
        );
        assert_eq!(count(&out), expected, "{out:?}");
    }
    for key in [
        "max = 1",
        "UseProjectIndex = true",
        "Include = ['**/*']",
        "enabled = 'true'",
        "exclude = ['[']",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            2,
        );
    }
}
#[test]
fn user_exclusions_supplied_paths_and_stdin_provenance() {
    let dir = project_with_lib();
    let sentinel = dir.path().join("lib/display.rb");
    fs::write(&sentinel, "sentinel\n").unwrap();
    let old = fs::metadata(&sentinel).unwrap();
    for config in [
        String::new(),
        "exclude = ['lib/*.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/*.rb']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let out = unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            0,
        );
        assert_eq!(count(&out), 0);
        let out = run(&dir.path().join("lib"), &["--only", COP, "a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let out = stdin(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--stdin", "lib/display.rb"],
            false,
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(count(&out), 0);
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/source.arb"), SOURCE).unwrap();
    assert_eq!(
        count(&run(dir.path(), &["--only", COP, "lib/source.arb"])),
        0
    );
    let out = stdin(
        dir.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--stdin", "lib/display.rb", "--fix"],
        true,
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
        "{out:?}"
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel\n");
    let new = fs::metadata(sentinel).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}
#[test]
fn both_prefixes_multiline_next_push_and_source_enables() {
    let dir = project_with_lib();
    let definition = "class A; def self.f(a,b); end; end\n";
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Lint\n{SOURCE}"),
            format!("{definition}# {prefix}:disable-next {COP}\nA.f(\n 1\n)\n"),
            format!("{definition}A.\n f(1) # {prefix}:disable {COP}\n"),
            format!("{definition}# {prefix}:push -{COP}\nA.f\n# {prefix}:pop\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "{definition}# {prefix}:disable Lint\nA.f\n# {prefix}:push +{COP}\nA.f\n# {prefix}:pop\nA.f\n"
        );
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            0,
        );
        assert_eq!(count(&out), 0, "{out:?}");
        // FirstArgumentIndentation reports the one-space argument
        // of the multi-line call below (as upstream does).
        fs::write(
            dir.path().join("rustocop.toml"),
            format!(
                "[cops.\"{COP}\"]\nenabled = false\n[cops.\"Layout/FirstArgumentIndentation\"]\nenabled = false\n"
            ),
        )
        .unwrap();
        let source = format!(
            "{SOURCE}# {prefix}:enable-next {COP}\nReport.generate(\n source\n)\nReport.generate(source)\n"
        );
        let out = unchanged(dir.path(), source.as_bytes(), &["--fix", "lib/a.rb"], 0);
        assert_eq!(count(&out), 0, "{out:?}");
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn envelopes_parser_failures_readonly_symlinks_and_no_external_index() {
    let dir = project_with_lib();
    for source in [
        format!(
            "\u{feff}# encoding: utf-8\r\n{}",
            SOURCE.replace('\n', "\r\n")
        )
        .into_bytes(),
        [
            b"# encoding: Windows-1252\np '\xe9'\n".as_slice(),
            SOURCE.as_bytes(),
        ]
        .concat(),
        format!("x = <<~TEXT\nReport.generate\nTEXT\n{SOURCE}").into_bytes(),
        format!("{SOURCE}File.binread(__FILE__); DATA.pos; __LINE__\n__END__\nraw\0\n")
            .into_bytes(),
        format!(
            "{SOURCE}RubyVM::InstructionSequence.of(method(:f)); ObjectSpace.each_object {{}}\n"
        )
        .into_bytes(),
    ] {
        let out = unchanged(
            dir.path(),
            &source,
            &["--only", COP, "--fix", "lib/a.rb"],
            0,
        );
        assert_eq!(count(&out), 0);
    }
    let out = unchanged(
        dir.path(),
        format!("{SOURCE}def broken(\n").as_bytes(),
        &["--only", COP, "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!(count(&out), 0);
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "missing.rb"])
            .status
            .code(),
        Some(2)
    );
    fs::write(
        dir.path().join("lib/callee.rb"),
        "class A; def self.f(a); end; end\n",
    )
    .unwrap();
    let out = unchanged(
        dir.path(),
        b"A.f\n",
        &["--only", COP, "--fix", "lib/a.rb", "lib/callee.rb"],
        0,
    );
    assert_eq!(count(&out), 0); // Parallel supplied files never become global state.
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, SOURCE).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        symlink("a.rb", dir.path().join("lib/alias.rb")).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = run(
            dir.path(),
            &["--only", COP, "--fix", "lib/alias.rb", "lib/a.rb"],
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(count(&out), 0);
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
        let new = fs::metadata(path).unwrap();
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
        assert_eq!(old.permissions(), new.permissions());
        assert!(
            fs::symlink_metadata(dir.path().join("lib/alias.rb"))
                .unwrap()
                .is_symlink()
        );
    }
}
#[test]
fn native_ruby34_syntax_only_reference_controls() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/argument_mismatch.json")).unwrap();
    for row in rows.as_array().unwrap() {
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
            .write_all(row["source"].as_str().unwrap().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
    }
}
#[test]
fn flat_deep_ancestry_and_cycles_are_bounded_without_rust_recursion() {
    let dir = project_with_lib();
    let mut source = String::from("class C0; def self.f(a); end; end\n");
    for i in 1..2500 {
        source.push_str(&format!("class C{i} < C{}; end\n", i - 1));
    }
    for _ in 0..100 {
        source.push_str("C2499.f\n");
    }
    let out = unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "lib/a.rb"],
        0,
    );
    assert_eq!(count(&out), 0);
    source.push_str("class C0 < Unknown; end\n");
    let out = unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", COP, "lib/a.rb"],
        0,
    );
    assert_eq!(count(&out), 0);
    let source =
        "class A < B; def self.f(a); end; include Unknown; end\nclass B < A; end\nA.f\nB.f\n";
    let out = unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", COP, "lib/a.rb"],
        0,
    );
    assert_eq!(count(&out), 0);
}
#[test]
fn all_reference_rows_preserve_bytes_mtime_permissions_on_actual_cli() {
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/argument_mismatch.json")).unwrap();
    let dir = project_with_lib();
    for row in rows.as_array().unwrap() {
        for fix in [false, true] {
            let mut args = vec!["--only", COP, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = unchanged(
                dir.path(),
                row["source"].as_str().unwrap().as_bytes(),
                &args,
                0,
            );
            assert_eq!(count(&out), 0, "{row}\n{out:?}");
        }
    }
}
