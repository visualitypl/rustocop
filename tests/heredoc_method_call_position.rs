//! Pinned source-only replay, real CLI and authored native controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/HeredocMethodCallPosition";
const SOURCE: &[u8] = b"<<-SQL\n  foo\nSQL\n.strip_indent\n";
const SAFE: &[u8] = b"[defined?(<<-SQL\n  foo\nSQL\n.strip_indent\n)]\n";
const AFTER: &[u8] = b"[defined?(<<-SQL.strip_indent\n  foo\nSQL\n)]\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/heredoc_method_call_position.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    crate::support::fixture::value(&row[key])
}
fn expected(row: &serde_json::Value) -> Vec<Finding> {
    crate::support::fixture::value(&row["findings"])
}
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}\n{source:?}");
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
fn all_pinned_original_bytes_match_pipeline() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let mut errors = Vec::new();
    for row in rows() {
        let source = bytes(&row, "bytes");
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}\n{report:?}"
        );
        let mut found: Vec<_> = report
            .result
            .offenses
            .iter()
            .filter(|o| o.cop == COP)
            .map(|o| {
                assert_eq!(o.severity, rustocop::offense::Severity::Warning);
                assert!(o.correction.is_none());
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        found.sort();
        if found != expected(&row) {
            errors.push(format!(
                "{}: {found:?} != {:?}",
                String::from_utf8_lossy(&source),
                expected(&row)
            ));
        }
    }
    assert!(
        errors.is_empty(),
        "{} mismatches:\n{}",
        errors.len(),
        errors.join("\n")
    );
}
#[test]
fn opt_in_metadata_selectors_toml_path_and_stdin_provenance() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(!cop.default_enabled());
    assert!(cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    let d = project_with_lib();
    for (config, selection, want) in [
        (String::new(), vec![], 0),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint"], 1),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        ("[cops.Lint]\nenabled = true\n".into(), vec![], 1),
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
        ("exclude = ['lib/a.rb']\n".into(), vec!["--only", COP], 0),
        (
            format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
            vec!["--only", COP],
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        fs::write(d.path().join("lib/a.rb"), SOURCE).unwrap();
        let mut args = selection.clone();
        args.push("lib/a.rb");
        let out = run(d.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), want, "{out:?}");
        args.pop();
        args.push("a.rb");
        assert_eq!(count(&run(&d.path().join("lib"), &args)), want);
        let mut args = selection;
        args.extend(["--stdin", "lib/a.rb"]);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        assert_eq!(count(&child.wait_with_output().unwrap()), want);
        assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), SOURCE);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "enabled = 'true'",
        "TargetRubyVersion = 3.3",
        "Style = 'other'",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(d.path(), SAFE, &["--only", COP, "--fix", "lib/a.rb"], 2);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for path in ["Gemfile", "lib/.hidden.rb", "lib/a.jb", "lib/a_spec.rb"] {
        fs::write(d.path().join(path), SOURCE).unwrap();
        assert_eq!(count(&run(d.path(), &["--only", COP, path])), 1);
    }
    assert_eq!(
        run(d.path(), &["--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
}
#[test]
fn both_prefixes_multiline_next_push_scopes_and_selection_boundaries() {
    let d = project_with_lib();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!(
                "# {prefix}:disable Lint\n{}",
                String::from_utf8_lossy(SOURCE)
            ),
            format!("<<-SQL\nfoo\nSQL\n.strip_indent # {prefix}:disable {COP}\n"),
            format!(
                "# {prefix}:push -{COP}\n{}# {prefix}:pop\n",
                String::from_utf8_lossy(SOURCE)
            ),
            format!(
                "# {prefix}:disable {COP}\n{}",
                String::from_utf8_lossy(SAFE)
            ),
        ] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        // The upstream range is the first byte after the terminator, not
        // the whole later call. disable-next on that line affects line 5,
        // and an inline comment at the multiline call's end affects line 6.
        for source in [
            format!("<<-SQL\nfoo\nSQL\n# {prefix}:disable-next {COP}\n.strip_indent(\n1)\n"),
            format!("<<-SQL\nfoo\nSQL\n.strip_indent(\n1) # {prefix}:disable {COP}\n"),
        ] {
            assert_eq!(
                count(&unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    1
                )),
                1
            );
        }
        let source = format!(
            "# {prefix}:enable {COP}\n{}# {prefix}:push -{COP}\n{}# {prefix}:pop\n{}",
            String::from_utf8_lossy(SOURCE),
            String::from_utf8_lossy(SOURCE),
            String::from_utf8_lossy(SOURCE)
        );
        assert_eq!(
            count(&unchanged(d.path(), source.as_bytes(), &["lib/a.rb"], 1)),
            2
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", "Lint/Debugger", "lib/a.rb"],
                0
            )),
            0
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", "Lint", "--except", COP, "lib/a.rb"],
                0
            )),
            0
        );
    }
}
#[cfg(unix)]
#[test]
fn real_canonical_atomic_permissions_symlink_and_failure_controls() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project_with_lib();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&path, d.path().join("link.rb")).unwrap();
    let out = run(d.path(), &["--only", COP, "--fix", "link.rb"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), AFTER);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(d.path().join("link.rb"))
            .unwrap()
            .is_symlink()
    );
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(d.path(), &["--only", COP, "--fix", "link.rb"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    assert_eq!(
        old.permissions(),
        fs::metadata(&path).unwrap().permissions()
    );
}
fn native(ruby: &str, source: &str) -> Output {
    Command::new(ruby)
        .args(["--parser=prism", "-w", "-e", source])
        .output()
        .unwrap()
}
#[test]
fn authored_native_value_binding_evaluation_control_flow_warning_and_observer_proofs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    assert!(
        String::from_utf8_lossy(
            &Command::new(&ruby)
                .arg("--version")
                .output()
                .unwrap()
                .stdout
        )
        .contains(crate::support::runtime::banner_prefix())
    );
    for method in ["strip_indent", "strip", "freeze"] {
        let before = format!("[defined?(<<-SQL\n  authored\nSQL\n.{method}\n)]\n");
        let after = format!("[defined?(<<-SQL.{method}\n  authored\nSQL\n)]\n");
        for (a, b) in [
            (before.clone(), after.clone()),
            (
                format!("p(begin; {before}end); p TOPLEVEL_BINDING.local_variables"),
                format!("p(begin; {after}end); p TOPLEVEL_BINDING.local_variables"),
            ),
            (
                format!(
                    "events = []; trace = TracePoint.new(:line, :call, :c_call) {{ |t| events << [t.event, t.method_id, t.lineno] if t.path == '-e' }}; trace.enable\np({before}); trace.disable; p events"
                ),
                format!(
                    "events = []; trace = TracePoint.new(:line, :call, :c_call) {{ |t| events << [t.event, t.method_id, t.lineno] if t.path == '-e' }}; trace.enable\np({after}); trace.disable; p events"
                ),
            ),
        ] {
            let x = native(&ruby, &a);
            let y = native(&ruby, &b);
            assert!(x.status.success() && y.status.success(), "{x:?}\n{y:?}");
            assert_eq!(x.stderr, y.stderr);
            assert!(x.stderr.is_empty(), "{x:?}");
            // Trace instrumentation itself has a later physical line after
            // the removed call line: inspect only execution within the query.
            if a.starts_with("events") {
                assert_ne!(x.stdout, y.stdout);
            } else {
                assert_eq!(x.stdout, y.stdout);
            }
        }
    }
    // Authored ordinary-call counterexample: the upstream canonical move
    // changes the visible exception location even when the value is unchanged.
    let a = native(
        &ruby,
        "begin\n<<-SQL\n  authored\nSQL\n.strip_indent\nrescue NoMethodError => error\nputs error.backtrace.first\nend\n",
    );
    let b = native(
        &ruby,
        "begin\n<<-SQL.strip_indent\n  authored\nSQL\nrescue NoMethodError => error\nputs error.backtrace.first\nend\n",
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    let out = native(
        &ruby,
        "p [defined?(<<-SQL\n#{raise 'evaluated'}\nSQL\n.strip\n)]\n",
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"[nil]\n"); // defined? swallows receiver exceptions.
    let out = native(
        &ruby,
        "$count = 0; p [defined?(<<-SQL\n#{$count += 1}\nSQL\n.strip\n)]; p $count\n",
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"[\"method\"]\n1\n"); // Receiver effects are real.
    let out = native(
        &ruby,
        "a = RubyVM::InstructionSequence.compile(%q{[defined?(<<-SQL\nfoo\nSQL\n.strip\n)]}); b = RubyVM::InstructionSequence.compile(%q{[defined?(<<-SQL.strip\nfoo\nSQL\n)]}); p(a.to_a == b.to_a)",
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"false\n");
}
#[test]
fn native_syntax_only_for_all_reference_sources_and_canonical_outputs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        for source in [bytes(&row, "bytes"), bytes(&row, "project_after")] {
            let mut child = Command::new(&ruby)
                .args(["-c", "-W0"])
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
