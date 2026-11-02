//! Source-only reference replay and separately authored native controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/LambdaWithoutLiteralBlock";
const SOURCE: &[u8] = b"lambda(&proc { do_something })\n";
const SAFE: &[u8] = b"if false\n  lambda(unreachable)\nend\n";
const AFTER: &[u8] = b"if false\n  unreachable\nend\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/lambda_without_literal_block.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    serde_json::from_value(row[key].clone()).unwrap()
}
fn expected(row: &serde_json::Value) -> Vec<Finding> {
    serde_json::from_value(row["findings"].clone()).unwrap()
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
fn all_original_bytes_match_pipeline_cli_and_guarded_canonical_outputs() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let d = project();
    let path = d.path().join("lib/a.rb");
    let rows = rows();
    assert_eq!(rows.iter().filter(|r| r["origin"] == "spec").count(), 7);
    assert_eq!(
        rows.iter().filter(|r| r["origin"] == "canonical").count(),
        18
    );
    for row in rows {
        let source = bytes(&row, "bytes");
        let after = bytes(&row, "project_after");
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
        let mut got: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        got.sort();
        assert_eq!(got, expected(&row), "{row}");
        let status = i32::from(!got.is_empty());
        let out = unchanged(d.path(), &source, &["--only", COP, "lib/a.rb"], status);
        let mut got: Vec<_> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.ends_with(&format!("({COP})")))
            .map(str::to_owned)
            .collect();
        got.sort();
        let mut want: Vec<_> = expected(&row)
            .into_iter()
            .map(|(l, c, _, m)| format!("lib/a.rb:{l}:{c}: E: {m} ({COP})"))
            .collect();
        want.sort();
        assert_eq!(got, want, "{row}");
        if source == after {
            unchanged(
                d.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                status,
            );
        } else {
            let old = fs::metadata(&path).unwrap();
            let out = run(d.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert!(out.status.success(), "{row}\n{out:?}");
            assert_eq!(fs::read(&path).unwrap(), after, "{row}");
            assert_eq!(
                old.permissions(),
                fs::metadata(&path).unwrap().permissions()
            );
            unchanged(d.path(), &after, &["--only", COP, "--fix", "lib/a.rb"], 0);
        }
    }
}
#[test]
fn defaults_selectors_strict_toml_department_and_file_stdin_provenance() {
    let registry = rustocop::cops::Registry::new();
    let rule = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(rule.default_enabled() && rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    let d = project();
    for (config, select, want) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint"], 1),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        (String::new(), vec!["--only", "Lint/Debugger"], 0),
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
        ("exclude = ['lib/a.rb']\n".into(), vec!["--only", COP], 0),
        (
            format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
            vec!["--only", COP],
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        fs::write(d.path().join("lib/a.rb"), SOURCE).unwrap();
        let mut args = select.clone();
        args.push("lib/a.rb");
        let out = run(d.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), want);
        args.pop();
        args.push("a.rb");
        assert_eq!(count(&run(&d.path().join("lib"), &args)), want);
        let mut args = select;
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
    let d = project();
    let source = String::from_utf8_lossy(SOURCE);
    for prefix in ["rubocop", "rustocop"] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        for directive in [
            format!("enable {COP}"),
            format!("enable-next {COP}"),
            format!("next +{COP}"),
            format!("push +{COP}"),
        ] {
            let text = format!("# {prefix}:{directive}\n{source}");
            assert_eq!(
                count(&unchanged(d.path(), text.as_bytes(), &["lib/a.rb"], 1)),
                1
            );
            unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", "Lint/Debugger", "lib/a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for text in [
            format!("# {prefix}:disable Lint\n{source}"),
            format!("# {prefix}:push -{COP}\n{source}# {prefix}:pop\n"),
            format!("# {prefix}:disable-next {COP}\n{source}"),
            format!("lambda(\n &pr) # {prefix}:disable {COP}\n"),
            format!(
                "# {prefix}:disable {COP}\n{}",
                String::from_utf8_lossy(SAFE)
            ),
        ] {
            unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        let text = format!(
            "# {prefix}:disable {COP}\n{source}# {prefix}:enable {COP}\n{source}# {prefix}:push -{COP}\n{source}# {prefix}:pop\n{source}"
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "lib/a.rb"],
                1
            )),
            2
        );
    }
}
#[test]
fn observers_bindings_delayed_scopes_invalid_parser_and_independent_composition() {
    let d = project();
    for suffix in [
        "p __LINE__",
        "File.binread(__FILE__)",
        "ObjectSpace.each_object {}",
        "RubyVM::InstructionSequence.compile('1')",
        "DATA.read\n__END__\ndata\n",
        "x = <<~DOC\ntext\nDOC\n",
        "TracePoint.new(:line) {}.enable",
        "GC.stat",
        "binding",
        "IO.read($0)",
        "Thread.current.backtrace",
        "p RubyVM::AbstractSyntaxTree",
        "Marshal.dump(Object)",
    ] {
        let text = format!("{}{suffix}\n", String::from_utf8_lossy(SAFE));
        assert_eq!(
            count(&unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                1
            )),
            1
        );
    }
    for wrapper in [
        "def deferred\n%s\nend\n",
        "-> {\n%s\n}\n",
        "if true\n%s\nend\n",
    ] {
        let text = wrapper.replace("%s", &String::from_utf8_lossy(SAFE));
        unchanged(
            d.path(),
            text.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
        );
    }
    let invalid = format!("{}def broken(\n", String::from_utf8_lossy(SAFE));
    assert_eq!(
        count(&unchanged(
            d.path(),
            invalid.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            2
        )),
        0
    );
    let mixed = format!("{}x = ['v',1,2]\n", String::from_utf8_lossy(SAFE));
    fs::write(d.path().join("lib/a.rb"), &mixed).unwrap();
    let out = run(
        d.path(),
        &[
            "--only",
            "Lint/LambdaWithoutLiteralBlock,Layout/SpaceAfterComma,Style/StringLiterals",
            "--fix",
            "lib/a.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out), 1);
    assert_eq!(
        fs::read(d.path().join("lib/a.rb")).unwrap(),
        mixed.replace("'v',1,2", "\"v\", 1, 2").as_bytes()
    );
}
#[cfg(unix)]
#[test]
fn atomic_permissions_symlink_and_failure_controls() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project();
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
fn authored_native_values_binding_dispatch_evaluation_control_flow_and_observers() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
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
        .contains("ruby 3.4.")
    );
    // Authored dead-code witnesses, never original/spec/reference/corpus inputs.
    let before = "if false\n  lambda(authored_unreachable)\nend\n";
    let after = "if false\n  authored_unreachable\nend\n";
    for (a, b) in [
        (before.to_owned(), after.to_owned()),
        (
            format!(
                "$events=[]; def lambda(*); $events << :lambda; raise; end; def authored_unreachable; $events << :argument; raise; end; p(begin; {before}end); p $events; p TOPLEVEL_BINDING.local_variables"
            ),
            format!(
                "$events=[]; def lambda(*); $events << :lambda; raise; end; def authored_unreachable; $events << :argument; raise; end; p(begin; {after}end); p $events; p TOPLEVEL_BINDING.local_variables"
            ),
        ),
        (
            format!(
                "$events=[]; t=TracePoint.new(:call,:b_call,:line) {{ |e| $events << [e.event,e.lineno] }}; t.enable; {before}t.disable; p $events"
            ),
            format!(
                "$events=[]; t=TracePoint.new(:call,:b_call,:line) {{ |e| $events << [e.event,e.lineno] }}; t.enable; {after}t.disable; p $events"
            ),
        ),
    ] {
        let a = native(&ruby, &a);
        let b = native(&ruby, &b);
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
        assert!(a.stderr.is_empty(), "{a:?}");
    }
    // The dead branch compiles to the same VM instructions and has no proc
    // allocation, local binding, value or control-flow difference.
    let a = native(
        &ruby,
        &format!("p RubyVM::InstructionSequence.compile({before:?}).to_a"),
    );
    let b = native(
        &ruby,
        &format!("p RubyVM::InstructionSequence.compile({after:?}).to_a"),
    );
    assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
    assert_eq!(a.stdout, b.stdout);
    // Ordinary dispatch is not assumed builtin, even for syntactic proc calls.
    for (setup, original, rewritten) in [
        (
            "def lambda(*args, &block); :overridden; end; ",
            "lambda(&proc { :body })",
            "proc { :body }",
        ),
        (
            "class AuthoredBlock; def to_proc; puts :converted; proc { :body }; end; end; obj=AuthoredBlock.new; ",
            "lambda(&obj)",
            "obj",
        ),
        (
            "def lambda(*args, &block); args; end; def first; puts :first; 1; end; def second; puts :second; 2; end; ",
            "lambda(first, second)",
            "first",
        ),
        ("", "lambda(proc { 1 })", "proc { 1 }"),
        ("", "lambda(&proc { :body })", "proc { :body }"),
        ("", "lambda(&nil)", "nil"),
    ] {
        let a = native(
            &ruby,
            &format!("{setup}begin; p({original}); rescue => e; p e.class; end"),
        );
        let b = native(
            &ruby,
            &format!("{setup}begin; p({rewritten}); rescue => e; p e.class; end"),
        );
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert!(a.stdout != b.stdout || a.stderr != b.stderr, "{original}");
    }
    // Ruby3.4 now rejects a non-lambda proc, rather than the historical3.0
    // deprecation warning described upstream. Removing the wrapper changes
    // the exception and nonlocal return; the proc's arity/lambda-ness matter.
    let a = native(
        &ruby,
        "def authored; pr=proc { return :returned }; q=lambda(&pr); p [q.equal?(pr),q.lambda?,q.arity]; q.call; :missed; end; begin; p authored; rescue => e; p e.class; end",
    );
    let b = native(
        &ruby,
        "def authored; pr=proc { return :returned }; q=pr; p [q.equal?(pr),q.lambda?,q.arity]; q.call; :missed; end; begin; p authored; rescue => e; p e.class; end",
    );
    assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
    assert_eq!(a.stdout, b"ArgumentError\n");
    assert_eq!(b.stdout, b"[true, false, 0]\n:returned\n");
    // Non-lambda proc arity and argument checks differ from literal lambdas;
    // neither API intent nor a lambda-named override is a type proof.
    let out = native(
        &ruby,
        "pr=proc { |x| x }; lm=->(x) { x }; p [pr.lambda?,pr.arity,pr.call]; begin; lm.call; rescue => e; p e.class; end",
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"[false, 1, nil]\nArgumentError\n");
    // No runtime proc allocation or lifetime can change in this closed dead
    // envelope. Observe actual factory calls and the live Proc set separately.
    for source in [before, after] {
        let out = native(
            &ruby,
            &format!(
                "GC.start; original=ObjectSpace.each_object(Proc).count; events=[]; trace=TracePoint.new(:c_call) {{ |e| events << e.method_id if [:lambda,:proc].include?(e.method_id) }}; trace.enable; {source}trace.disable; p events; GC.start; p(ObjectSpace.each_object(Proc).count == original + 1)"
            ),
        );
        assert!(out.status.success(), "{out:?}");
        assert_eq!(out.stdout, b"[]\ntrue\n");
    }
    // Source bytes, code objects and heap observations remain independent holds.
    let a = native(&ruby, &format!("p {before:?}.bytes"));
    let b = native(&ruby, &format!("p {after:?}.bytes"));
    assert_ne!(a.stdout, b.stdout);
    let a = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('def authored; lambda(&proc {}); end').disasm",
    );
    let b = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('def authored; proc {}; end').disasm",
    );
    assert_ne!(a.stdout, b.stdout);
    let setup = "$n=0; def lambda(*args,&block); $n+=1; proc {}; end; ";
    let a = native(&ruby, &format!("{setup}lambda(&proc {{}}); p $n"));
    let b = native(&ruby, &format!("{setup}proc {{}}; p $n"));
    assert_ne!(a.stdout, b.stdout);
}
#[test]
fn native_syntax_only_for_every_reference_input_and_project_output() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
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
