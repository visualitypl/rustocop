//! Source-only RuboCop MIT spec replay; authored native controls separate.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/SymbolConversion";
const SOURCE: &[u8] = b":foo.to_sym\n";
const SAFE: &[u8] = b"if false\n  :foo.to_sym\nend\n";
const AFTER: &[u8] = b"if false\n  :foo\nend\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/symbol_conversion.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    serde_json::from_value(row[key].clone()).unwrap()
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
fn defaults_selectors_toml_precedence_and_path_stdin_provenance() {
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
fn source_file_advice_uses_supplied_path_not_display_or_canonical_path() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let source = b"__FILE__.to_sym\n";
    let report = rustocop::pipeline::lint_one(
        Path::new("input.rb"),
        Path::new("display.rb"),
        Some(source),
        &active,
        &config,
    );
    assert!(report.errors.is_empty() && report.internal_errors.is_empty());
    assert_eq!(
        report.result.offenses[0].message,
        "Unnecessary symbol conversion; use `:\"input.rb\"` instead."
    );
    let d = project();
    for path in ["virtual.rb", "nested/nonexistent.rb", "e\u{301}.rb"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", COP, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(source).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(&format!("use `:\"{path}\"` instead.")),
            "{out:?}"
        );
        assert!(!d.path().join(path).exists());
    }
}
#[test]
fn directives_multiline_next_push_and_selection_boundaries() {
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
            format!(":foo.\n to_sym # {prefix}:disable {COP}\n"),
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
#[cfg(unix)]
#[test]
fn atomic_permissions_symlink_and_failure_controls() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&path, d.path().join("link.rb")).unwrap();
    let mut held = fs::File::open(&path).unwrap();
    let out = run(d.path(), &["--only", COP, "--fix", "link.rb"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), AFTER);
    let mut previous = Vec::new();
    std::io::Read::read_to_end(&mut held, &mut previous).unwrap();
    assert_eq!(previous, SAFE);
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
fn authored_native_dead_branch_values_bindings_warnings_and_live_counterexamples() {
    if crate::support::runtime::skips(
        "pins compiled code or VM state that Ruby 4.0 builds differently",
    ) {
        return;
    }
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
    // Separately authored, bounded Safety witnesses. NEVER execute fixture,
    // spec, documentation or generated differential inputs.
    let original = "if false\n  :foo.to_sym\nend\n";
    let corrected = "if false\n  :foo\nend\n";
    for (a, b) in [
        (original.to_owned(), corrected.to_owned()),
        (
            format!("p(begin\n{original}end); p TOPLEVEL_BINDING.local_variables"),
            format!("p(begin\n{corrected}end); p TOPLEVEL_BINDING.local_variables"),
        ),
        (
            format!(
                "$events=[]; class Symbol; def to_sym; $events << :called; raise 'authored'; end; end; {original}p $events"
            ),
            format!(
                "$events=[]; class Symbol; def to_sym; $events << :called; raise 'authored'; end; end; {corrected}p $events"
            ),
        ),
        (
            format!(
                "$n=0; TracePoint.new(:call,:c_call) {{ |t| $n+=1 if [:to_sym,:intern].include?(t.method_id) }}.enable; {original}p $n"
            ),
            format!(
                "$n=0; TracePoint.new(:call,:c_call) {{ |t| $n+=1 if [:to_sym,:intern].include?(t.method_id) }}.enable; {corrected}p $n"
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
    let vm = native(
        &ruby,
        r#"
a = "if false\n  :foo.to_sym\nend\n"
b = "if false\n  :foo\nend\n"
x = RubyVM::InstructionSequence.compile(a, 'authored.rb')
y = RubyVM::InstructionSequence.compile(b, 'authored.rb')
raise 'code object differs' unless x.to_a == y.to_a && x.disasm == y.disasm
GC.start
before = GC.stat(:total_allocated_objects)
100.times { x.eval }
first = GC.stat(:total_allocated_objects) - before
before = GC.stat(:total_allocated_objects)
100.times { y.eval }
second = GC.stat(:total_allocated_objects) - before
raise 'heap difference' unless (first-second).abs <= 2
puts 'authored VM/heap control passed'
"#,
    );
    assert!(vm.status.success(), "{vm:?}");
    // Real counterexamples: literal conversion is a dispatch, not a cast.
    for (setup, a, b) in [
        (
            "class Symbol; def to_sym; :different; end; end;",
            "p :authored.to_sym",
            "p :authored",
        ),
        (
            "class String; def intern; raise 'authored'; end; end;",
            "begin; p 'authored'.intern; rescue => e; p e.message; end",
            "p :authored",
        ),
        (
            "$n=0; TracePoint.new(:call,:c_call) { |t| $n+=1 if [:to_sym,:intern].include?(t.method_id) }.enable;",
            ":authored.to_sym; p $n",
            ":authored; p $n",
        ),
        (
            "",
            "begin; p :authored.to_sym(1); rescue => e; p e.class; end",
            "p :authored",
        ),
        (
            "",
            "p RubyVM::InstructionSequence.compile(':authored.to_sym').to_a",
            "p RubyVM::InstructionSequence.compile(':authored').to_a",
        ),
    ] {
        let a = native(&ruby, &format!("{setup}{a}"));
        let b = native(&ruby, &format!("{setup}{b}"));
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_ne!(a.stdout, b.stdout);
    }
}
#[test]
fn native_syntax_only_for_every_reference_input_and_project_output() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        let mut sources = vec![bytes(&row, "bytes")];
        for key in ["project_after", "upstream_after", "declared_after"] {
            if row.get(key).is_some() {
                sources.push(bytes(&row, key));
            }
        }
        for source in sources {
            let mut child = Command::new(&ruby)
                .args(["-c", "-W0"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                row.get("parser_rejection").is_none()
                    && row.get("native_parser_rejection").is_none(),
                "{row}\n{out:?}"
            );
        }
    }
}
