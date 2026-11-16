//! Source-only reference replay and bounded authored native controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/UselessAccessModifier";
const ORDINARY: &[u8] = b"class C\n  private\nend\n";
const SAFE: &[u8] = b"if false\n  Class.new do\n    private\n  end\nend\n";
const AFTER: &[u8] = b"if false\n  Class.new do\n  end\nend\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/useless_access_modifier.json")).unwrap()
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
fn defaults_selectors_toml_precedence_paths_and_stdin() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
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
        fs::write(d.path().join("lib/a.rb"), ORDINARY).unwrap();
        let mut args = select.clone();
        args.push("lib/a.rb");
        let out = run(d.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), want, "{out:?}");
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
        child.stdin.take().unwrap().write_all(ORDINARY).unwrap();
        assert_eq!(count(&child.wait_with_output().unwrap()), want);
        assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), ORDINARY);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "enabled = 'true'",
        "ContextCreatingMethods = ['concerning']",
        "MethodCreatingMethods = ['delegate']",
        "ActiveSupportExtensionsEnabled = true",
        "TargetRubyVersion = 3.3",
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
        fs::write(d.path().join(path), ORDINARY).unwrap();
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
fn both_directive_prefixes_multiline_next_push_and_no_selection_escape() {
    let d = project();
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
            let source = format!(
                "# {prefix}:{directive}\n{}",
                String::from_utf8_lossy(ORDINARY)
            );
            assert_eq!(
                count(&unchanged(d.path(), source.as_bytes(), &["lib/a.rb"], 1)),
                1
            );
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", "Lint/Debugger", "lib/a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for source in [
            format!(
                "# {prefix}:disable {COP}\n{}",
                String::from_utf8_lossy(SAFE)
            ),
            format!(
                "# {prefix}:push -Lint\n{}# {prefix}:pop\n",
                String::from_utf8_lossy(SAFE)
            ),
            format!("class C\n# {prefix}:disable-next {COP}\nprivate\nend\n"),
            format!("class C\nprivate # {prefix}:disable {COP}\nend\n"),
            format!("class C\nprivate(\n) # {prefix}:disable {COP}\nend\n"),
        ] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "# {prefix}:disable {COP}\n{0}# {prefix}:enable {COP}\n{0}# {prefix}:push -{COP}\n{0}# {prefix}:pop\n{0}",
            String::from_utf8_lossy(ORDINARY)
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "lib/a.rb"],
                1
            )),
            2
        );
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", "Lint/Debugger", "lib/a.rb"],
            0,
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
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
}
fn native(ruby: &str, source: &str) -> Output {
    Command::new(ruby)
        .args(["--parser=prism", "-w", "-e", source])
        .output()
        .unwrap()
}
#[test]
fn authored_native_control_flow_values_bindings_dispatch_warnings_and_observers() {
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
    // Separately authored bounded witnesses, never reference/corpus execution.
    for name in ["public", "protected", "private", "module_function"] {
        let before = format!("if false\n Class.new do\n  {name}\n end\nend\n");
        let after = "if false\n Class.new do\n end\nend\n";
        for (a, b) in [
            (before.clone(), after.into()),
            (
                format!(
                    "p(begin; {before}end);p TOPLEVEL_BINDING.local_variables;p Object.private_instance_methods(false).include?(:authored)"
                ),
                format!(
                    "p(begin; {after}end);p TOPLEVEL_BINDING.local_variables;p Object.private_instance_methods(false).include?(:authored)"
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
        for observer in [
            "p iseq.to_a",
            "p ObjectSpace.each_object(RubyVM::InstructionSequence).select { |o| o.equal?(iseq) }.map(&:to_a)",
        ] {
            let a = native(
                &ruby,
                &format!("iseq = RubyVM::InstructionSequence.compile({before:?}); {observer}"),
            );
            let b = native(
                &ruby,
                &format!("iseq = RubyVM::InstructionSequence.compile({after:?}); {observer}"),
            );
            assert!(a.status.success() && b.status.success());
            assert_ne!(a.stdout, b.stdout); // top code end-line, directly or through heap
        }
        let prefix = "$authored = []; class << Class; alias authored_new new; def new(*args, &block); $authored << :called; authored_new(*args, &block);end;end;\n";
        let a = native(
            &ruby,
            &format!(
                "{prefix}p(begin; {before}end); p $authored; p ObjectSpace.each_object(Class).count"
            ),
        );
        let b = native(
            &ruby,
            &format!(
                "{prefix}p(begin; {after}end); p $authored; p ObjectSpace.each_object(Class).count"
            ),
        );
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
    }
    // Builtin tail removal changes a class body's return value.
    let a = native(&ruby, "p(class Authored; def marker;end; private;end)");
    let b = native(&ruby, "p(class Authored; def marker;end;end)");
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    // User-defined modifiers can dispatch, raise, mutate and return values.
    let a = native(
        &ruby,
        "class Authored;def self.private;puts :called;raise 'authored';end;begin;private;rescue=>e;puts e.message;end;end",
    );
    let b = native(
        &ruby,
        "class Authored;def self.private;puts :called;raise 'authored';end;begin;nil;rescue=>e;puts e.message;end;end",
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    // Line deletion relocates definitions even if visibility is already public.
    let a = native(
        &ruby,
        "class Authored\npublic\ndef marker;end\nend\np Authored.instance_method(:marker).source_location",
    );
    let b = native(
        &ruby,
        "class Authored\ndef marker;end\nend\np Authored.instance_method(:marker).source_location",
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
}
#[test]
fn native_syntax_only_for_all_reference_inputs_and_project_outputs() {
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
