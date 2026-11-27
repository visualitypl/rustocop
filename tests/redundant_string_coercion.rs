//! Source-only reference replay and separately authored native controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/RedundantStringCoercion";
const SOURCE: &[u8] = b"puts value.to_s\n";
const SAFE: &[u8] = b"if false\n  puts value.to_s\nend\n";
const AFTER: &[u8] = b"if false\n  puts value\nend\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/redundant_string_coercion.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    crate::support::fixture::value(&row[key])
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
fn defaults_selectors_strict_toml_department_and_file_stdin_provenance() {
    let registry = rustocop::cops::Registry::new();
    let rule = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(rule.default_enabled() && rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    let d = project_with_lib();
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
        "Include = ['**/*']",
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
            format!("puts value.\n to_s # {prefix}:disable {COP}\n"),
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
fn authored_native_values_bindings_dispatch_evaluation_warnings_and_observability() {
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
    // Independently authored runtime controls, never spec/reference/probe inputs.
    for (before, after) in [
        ("\"#{alpha.to_s}\"", "\"#{alpha}\""),
        ("/\u{23}{alpha&.to_s}/", "/\u{23}{alpha}/"),
        (":\"#{alpha.to_s()}\"", ":\"#{alpha}\""),
        ("`#{alpha.to_s}`", "`#{alpha}`"),
        ("print alpha.to_s", "print alpha"),
        ("puts alpha&.to_s", "puts alpha"),
        ("warn to_s", "warn self"),
    ] {
        let a = format!("if false\n  {before}\nend\n");
        let b = format!("if false\n  {after}\nend\n");
        for (a, b) in [
            (a.clone(), b.clone()),
            (
                format!(
                    "$events=[]; def alpha; $events << :receiver; raise; end; def to_s; $events << :conversion; raise; end; p(begin; {a}end); p $events; p TOPLEVEL_BINDING.local_variables"
                ),
                format!(
                    "$events=[]; def alpha; $events << :receiver; raise; end; def to_s; $events << :conversion; raise; end; p(begin; {b}end); p $events; p TOPLEVEL_BINDING.local_variables"
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
        // Unevaluated branch source bytes remain observable; no source waiver.
        assert_ne!(a.as_bytes(), b.as_bytes());
    }
    // Explicit String#to_s dispatch disappears in interpolation; primitive
    // receiver spelling is not proof of builtin methods or equivalent events.
    let setup = "$events=[]; class String; def to_s; $events << :explicit; self; end; end; ";
    let a = native(
        &ruby,
        &format!("{setup}p(\"#{{'authored'.to_s}}\"); p $events"),
    );
    let b = native(&ruby, &format!("{setup}p(\"#{{'authored'}}\"); p $events"));
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    // Non-String conversion results and safe navigation can change coercion.
    let setup = "class AuthoredCoercion; def to_s; 12; end; end; a=AuthoredCoercion.new; ";
    let a = native(&ruby, &format!("{setup}p(\"#{{a.to_s}}\")"));
    let b = native(&ruby, &format!("{setup}p(\"#{{a}}\")"));
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    // Output APIs need not be Kernel's methods; argument values/effects differ.
    for method in ["print", "puts", "warn"] {
        let setup = format!("def {method}(value); p value.class; end; ");
        let a = native(&ruby, &format!("{setup}{method} [1,2].to_s"));
        let b = native(&ruby, &format!("{setup}{method} [1,2]"));
        assert!(a.status.success() && b.status.success());
        assert_ne!(a.stdout, b.stdout);
    }
    // Ordinary builtins differ too: puts(Array) recursively prints elements.
    let a = native(&ruby, "puts [1,2].to_s");
    let b = native(&ruby, "puts [1,2]");
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    // Native trace/code-object and allocation controls expose removed sends.
    let a = native(
        &ruby,
        "$n=0; TracePoint.new(:c_call) { |t| $n+=1 if t.method_id == :to_s }.enable; x='authored'; y=\"#{x.to_s}\"; p $n",
    );
    let b = native(
        &ruby,
        "$n=0; TracePoint.new(:c_call) { |t| $n+=1 if t.method_id == :to_s }.enable; x='authored'; y=\"#{x}\"; p $n",
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    let a = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('def authored(x); puts x.to_s; end').disasm",
    );
    let b = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('def authored(x); puts x; end').disasm",
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    let setup = "class AuthoredHeap < String; def to_s; $allocs+=1; String.new(self); end; end; $allocs=0; x=AuthoredHeap.new('authored'); ";
    let a = native(
        &ruby,
        &format!("{setup}20.times {{ \"#{{x.to_s}}\" }}; p $allocs"),
    );
    let b = native(
        &ruby,
        &format!("{setup}20.times {{ \"#{{x}}\" }}; p $allocs"),
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
}
#[test]
fn native_syntax_only_for_every_reference_input_and_project_output() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        for source in [
            bytes(&row, "bytes"),
            if row.get("project_after").is_some() {
                bytes(&row, "project_after")
            } else {
                bytes(&row, "bytes")
            },
        ] {
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
                row.get("parser_rejection").is_none(),
                "{row}\n{out:?}"
            );
        }
    }
}
