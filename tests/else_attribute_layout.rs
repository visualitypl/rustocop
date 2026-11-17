//! Source-only pinned replay and bounded authored native controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = [
    "Lint/ElseLayout",
    "Lint/TrailingCommaInAttributeDeclaration",
];
const SOURCES: [&[u8]; 2] = [
    b"if condition\n  first\nelse last\nend\n",
    b"class C\n  attr_reader :x,\n  def y; end\nend\n",
];
const SAFE: [&[u8]; 2] = [
    b"if true\n  nil\nelse false\nend\n",
    b"if false\n  attr_reader :bar,\n  def baz; end\nend\n",
];
const AFTER: [&[u8]; 2] = [
    b"if true\n  nil\nelse\n  false\nend\n",
    b"if false\n  attr_reader :bar\n  def baz; end\nend\n",
];
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/else_attribute_layout.json")).unwrap()
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
fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({cop})")))
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
fn metadata_defaults_selectors_toml_department_precedence_paths_and_stdin() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let d = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        for (config, select, want) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint"], 1),
            (String::new(), vec!["--only", cop, "--except", "Lint"], 0),
            (String::new(), vec!["--only", "Lint/Debugger"], 0),
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
            ("exclude = ['lib/a.rb']\n".into(), vec!["--only", cop], 0),
            (
                format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
                vec!["--only", cop],
                0,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(d.path().join("lib/a.rb"), source).unwrap();
            let mut args = select.clone();
            args.push("lib/a.rb");
            let out = run(d.path(), &args);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_eq!(count(&out, cop), want, "{out:?}");
            args.pop();
            args.push("a.rb");
            assert_eq!(count(&run(&d.path().join("lib"), &args), cop), want);
            let mut args = select;
            args.extend(["--stdin", "lib/a.rb"]);
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(source).unwrap();
            assert_eq!(count(&child.wait_with_output().unwrap(), cop), want);
            assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), source);
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
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(d.path(), source, &["--only", cop, "--fix", "lib/a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for path in ["Gemfile", "lib/.hidden.rb", "lib/a.jb", "lib/a_spec.rb"] {
            fs::write(d.path().join(path), source).unwrap();
            assert_eq!(count(&run(d.path(), &["--only", cop, path]), cop), 1);
        }
        assert_eq!(
            run(d.path(), &["--stdin", "lib/a.rb", "--fix"])
                .status
                .code(),
            Some(2)
        );
    }
}
#[test]
fn both_prefixes_multiline_next_push_scopes_and_selection_boundaries() {
    let d = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let source = String::from_utf8_lossy(source);
        for prefix in ["rubocop", "rustocop"] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            for directive in [
                format!("enable {cop}"),
                format!("enable-next {cop}"),
                format!("next +{cop}"),
                format!("push +{cop}"),
            ] {
                let text = format!("# {prefix}:{directive}\n{source}");
                assert_eq!(
                    count(&unchanged(d.path(), text.as_bytes(), &["lib/a.rb"], 1), cop),
                    1
                );
                assert_eq!(
                    count(
                        &unchanged(
                            d.path(),
                            text.as_bytes(),
                            &["--only", "Lint/Debugger", "lib/a.rb"],
                            0
                        ),
                        cop
                    ),
                    0
                );
            }
            fs::write(d.path().join("rustocop.toml"), "").unwrap();
            for text in [
                format!("# {prefix}:disable Lint\n{source}"),
                format!("# {prefix}:push -{cop}\n{source}# {prefix}:pop\n"),
                format!("# {prefix}:disable {cop}\n{source}"),
            ] {
                unchanged(
                    d.path(),
                    text.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    0,
                );
            }
            let text = format!(
                "# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}# {prefix}:push -{cop}\n{source}# {prefix}:pop\n{source}"
            );
            assert_eq!(
                count(
                    &unchanged(d.path(), text.as_bytes(), &["--only", cop, "lib/a.rb"], 1),
                    cop
                ),
                2
            );
            assert_eq!(
                count(
                    &unchanged(
                        d.path(),
                        text.as_bytes(),
                        &["--only", "Lint/Debugger", "lib/a.rb"],
                        0
                    ),
                    cop
                ),
                0
            );
            for text in if cop == COPS[0] {
                vec![
                    format!("if x\n# {prefix}:disable-next {cop}\nelse work\nend\n"),
                    format!("if x\nelse work(\n  1) # {prefix}:disable {cop}\nend\n"),
                ]
            } else {
                vec![
                    format!(
                        "class C\n# {prefix}:disable-next {cop}\nattr_reader :x,\ndef y; end\nend\n"
                    ),
                    format!("class C\nattr_reader :x, # {prefix}:disable {cop}\ndef y; end\nend\n"),
                ]
            } {
                unchanged(
                    d.path(),
                    text.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    0,
                );
            }
        }
    }
}
#[cfg(unix)]
#[test]
fn atomic_permissions_symlink_and_failure_controls() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for ((cop, safe), after) in COPS.into_iter().zip(SAFE).zip(AFTER) {
        let d = project();
        let path = d.path().join("lib/a.rb");
        fs::write(&path, safe).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&path, d.path().join("link.rb")).unwrap();
        let out = run(d.path(), &["--only", cop, "--fix", "link.rb"]);
        assert!(out.status.success(), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(
            fs::symlink_metadata(d.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
        fs::write(&path, safe).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(d.path(), &["--only", cop, "--fix", "link.rb"])
                .status
                .code(),
            Some(2)
        );
        assert_eq!(fs::read(&path).unwrap(), safe);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
    }
}
fn native(ruby: &str, source: &str) -> Output {
    Command::new(ruby)
        .args(["--parser=prism", "-w", "-e", source])
        .output()
        .unwrap()
}
#[test]
fn authored_native_values_bindings_evaluation_warnings_and_observer_counterexamples() {
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
    // Authored controls, not execution of any reference/spec/corpus input.
    for (a, b) in [
        (
            "if true\n nil\nelse false\nend\n",
            "if true\n nil\nelse\n  false\nend\n",
        ),
        (
            "if false\n attr_reader :authored,\n def example; end\nend\n",
            "if false\n attr_reader :authored\n def example; end\nend\n",
        ),
    ] {
        for (x, y) in [
            (a.to_owned(), b.to_owned()),
            (
                format!(
                    "p(begin; {a}end); p TOPLEVEL_BINDING.local_variables; p Object.private_instance_methods(false).include?(:example)"
                ),
                format!(
                    "p(begin; {b}end); p TOPLEVEL_BINDING.local_variables; p Object.private_instance_methods(false).include?(:example)"
                ),
            ),
        ] {
            let x = native(&ruby, &x);
            let y = native(&ruby, &y);
            assert!(x.status.success() && y.status.success(), "{x:?}\n{y:?}");
            assert_eq!(x.stdout, y.stdout);
            assert_eq!(x.stderr, y.stderr);
            assert!(x.stderr.is_empty(), "{x:?}");
        }
        let x = native(
            &ruby,
            &format!("p RubyVM::InstructionSequence.compile({a:?}).to_a"),
        );
        let y = native(
            &ruby,
            &format!("p RubyVM::InstructionSequence.compile({b:?}).to_a"),
        );
        assert!(x.status.success() && y.status.success());
        if a.starts_with("if true") {
            assert_ne!(x.stdout, y.stdout);
        } else {
            assert_eq!(x.stdout, y.stdout);
        }
    }
    // Even unreachable expressions have compile-time void-context warnings.
    let a = native(&ruby, "if true\n nil\nelse false; nil\nend\n");
    let b = native(&ruby, "if true\n nil\nelse\n false; nil\nend\n");
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
    assert_ne!(a.stderr, b.stderr);
    assert!(String::from_utf8_lossy(&a.stderr).contains("possibly useless use of false"));
    // Ordinary trailing comma is behavior repair: it overwrites the method.
    let a = native(
        &ruby,
        "class Authored; attr_reader :first,\n def second; :retained; end; end; p Authored.new.second",
    );
    let b = native(
        &ruby,
        "class Authored; attr_reader :first\n def second; :retained; end; end; p Authored.new.second",
    );
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b"nil\n");
    assert_eq!(b.stdout, b":retained\n");
    // Else relocation preserves ordinary values but changes error/trace lines.
    let a = native(
        &ruby,
        "begin\n if false\n nil\n else raise 'authored'\n end\nrescue => e; puts e.backtrace.first; end",
    );
    let b = native(
        &ruby,
        "begin\n if false\n nil\n else\n  raise 'authored'\n end\nrescue => e; puts e.backtrace.first; end",
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
}
#[test]
fn native_syntax_only_for_all_reference_inputs_and_actual_project_outputs() {
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
