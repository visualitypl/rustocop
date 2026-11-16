//! Source-only RuboCop replay (MIT, Copyright (c) 2012-26
//! Bozhidar Batsov). Only separately authored bounded native controls execute.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = ["Lint/ImplicitStringConcatenation", "Lint/TripleQuotes"];
const SAFE: [&[u8]; 2] = [b"\"abc\" \"\"\n", b"\"\"\"a string\"\"\"\n"];
const AFTER: [&[u8]; 2] = [b"\"abc\" \n", b"\"a string\"\n"];
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/adjacent_strings.json")).unwrap()
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
fn native_syntax_only_for_interpolation_projection_inputs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/adjacent_string_projection.json")).unwrap();
    for row in rows {
        // Syntax ONLY. In particular, never evaluate xstrings or interpolation.
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-c", "-W0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&bytes(&row, "bytes"))
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{row}\n{out:?}");
    }
}
#[test]
fn defaults_selectors_toml_department_provenance_and_strict_keys() {
    let registry = rustocop::cops::Registry::new();
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let rule = registry.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        for (config, select, want) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", *cop], 1),
            (String::new(), vec!["--only", "Lint"], 1),
            (String::new(), vec!["--only", *cop, "--except", "Lint"], 0),
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
                vec!["--only", *cop],
                1,
            ),
            ("exclude = ['lib/a.rb']\n".into(), vec!["--only", *cop], 0),
            (
                format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
                vec!["--only", *cop],
                0,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(d.path().join("lib/a.rb"), SAFE[i]).unwrap();
            let mut args = select.clone();
            args.push("lib/a.rb");
            let out = run(d.path(), &args);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_eq!(count(&out, cop), want);
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
            child.stdin.take().unwrap().write_all(SAFE[i]).unwrap();
            assert_eq!(count(&child.wait_with_output().unwrap(), cop), want);
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
            unchanged(d.path(), SAFE[i], &["--only", cop, "--fix", "lib/a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for path in ["Gemfile", "lib/.hidden.rb", "lib/a.jb", "lib/a_spec.rb"] {
            fs::write(d.path().join(path), SAFE[i]).unwrap();
            assert_eq!(count(&run(d.path(), &["--only", cop, path]), cop), 1);
        }
    }
    assert_eq!(
        run(d.path(), &["--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
}
#[cfg(unix)]
#[test]
fn atomic_mode_symlink_and_readonly_failures() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, SAFE[0]).unwrap();
    symlink(&path, d.path().join("link.rb")).unwrap();
    for (i, cop) in COPS.iter().enumerate() {
        fs::write(&path, SAFE[i]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(
            run(d.path(), &["--only", cop, "--fix", "link.rb"])
                .status
                .success()
        );
        assert_eq!(fs::read(&path).unwrap(), AFTER[i]);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(
            fs::symlink_metadata(d.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
        fs::write(&path, SAFE[i]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(d.path(), &["--only", cop, "--fix", "link.rb"])
                .status
                .code(),
            Some(2)
        );
        assert_eq!(fs::read(&path).unwrap(), SAFE[i]);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    }
}
#[test]
fn authored_native_continuation_line_events_and_cli_no_write_hold() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    let ruby = crate::support::runtime::ruby().ok();
    // Separately authored bounded witness: observe actual line events, not just
    // values or a TracePoint that never reports its captured events.
    let source = "events = []\ntrace = TracePoint.new(:line) { |event| events << event.lineno }\ntrace.enable\n\"\"\"\" \\\n\"authored\".bytes\ntrace.disable\np events.take(2)\n";
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        for only in [
            COPS[1],
            "Lint/ImplicitStringConcatenation,Lint/TripleQuotes",
        ] {
            fs::write(&path, &source).unwrap();
            let before = ruby.as_ref().map(|ruby| native(ruby, &source));
            let out = run(d.path(), &["--only", only, "--fix", "lib/a.rb"]);
            let after = fs::read_to_string(&path).unwrap();
            if let Some(before) = before {
                let after = native(ruby.as_ref().unwrap(), &after);
                assert!(
                    before.status.success() && after.status.success(),
                    "{before:?}\n{after:?}"
                );
                assert_eq!(before.stdout, b"[4, 6]\n");
                assert_eq!(before.stdout, after.stdout);
                assert_eq!(before.stderr, after.stderr);
            }
            if only == COPS[1] {
                assert_eq!(out.status.code(), Some(1), "{out:?}");
                assert_eq!(count(&out, COPS[1]), 1);
                assert_eq!(after, source);
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", only, "--fix", "lib/a.rb"],
                    1,
                );
            } else {
                // The pairwise cop can safely remove one same-line empty
                // child, but must leave the other on the original start line.
                assert!(out.status.success(), "{out:?}");
                assert_eq!(after, source.replace("\"\"\"\"", "\"\""));
                unchanged(
                    d.path(),
                    after.as_bytes(),
                    &["--only", only, "--fix", "lib/a.rb"],
                    0,
                );
            }
        }
    }
}
#[test]
fn authored_native_allocation_chilled_mutation_dispatch_and_trace_controls() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for (before, after) in [
        ("\"authored value\" \"\"", "\"authored value\""),
        ("\"\"\"authored value\"\"\"", "\"authored value\""),
    ] {
        for setup in [
            "Warning[:deprecated] = true; ",
            "class String; def +(other); raise; end; def freeze; raise; end; end; ",
            "events=[]; trace=TracePoint.new(:line) { |event| events << event.lineno }; trace.enable; ",
        ] {
            let tail = "; p value.bytes; p value.frozen?; begin; value << '!'; rescue FrozenError; puts :frozen; end";
            let a = native(&ruby, &format!("{setup}value = {before}{tail}"));
            let b = native(&ruby, &format!("{setup}value = {after}{tail}"));
            assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
            assert_eq!(a.stdout, b.stdout);
            assert_eq!(a.stderr, b.stderr);
        }
        // Warmed, bounded allocation counts: no extra runtime String allocation
        // is introduced by folding these static literals, unlike String#+.
        let setup = "GC.disable; GC.stat(:total_allocated_objects); 10.times { nil }; count=GC.stat(:total_allocated_objects); ";
        let tail = "; p(GC.stat(:total_allocated_objects)-count)";
        let a = native(
            &ruby,
            &format!("{setup}50.times {{ value = {before}; value }}{tail}"),
        );
        let b = native(
            &ruby,
            &format!("{setup}50.times {{ value = {after}; value }}{tail}"),
        );
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stderr, b.stderr);
    }
}
#[test]
fn original_legacy_payload_canonical_removal_and_unrepresentable_batch_hold() {
    let d = project();
    let before = b"# coding: ISO-8859-1\nvalue = \"caf\xe9\" \"\"\np [value.bytes,value.encoding.name,value.frozen?]\n";
    let after = b"# coding: ISO-8859-1\nvalue = \"caf\xe9\" \np [value.bytes,value.encoding.name,value.frozen?]\n";
    fs::write(d.path().join("lib/a.rb"), before).unwrap();
    let out = run(d.path(), &["--only", COPS[0], "--fix", "lib/a.rb"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), after);
    unchanged(
        d.path(),
        b"# coding: ISO-8859-1\n\"\"\"caf\xe9\"\"\"\n",
        &["--only", COPS[1], "--fix", "lib/a.rb"],
        1,
    );
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let mut outputs = Vec::new();
    for source in [before.as_slice(), after.as_slice()] {
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-w"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(source).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{out:?}");
        outputs.push(out);
    }
    assert_eq!(
        outputs[0].stdout,
        b"[[99, 97, 102, 233], \"ISO-8859-1\", false]\n"
    );
    assert_eq!(outputs[0].stdout, outputs[1].stdout);
    assert_eq!(outputs[0].stderr, outputs[1].stderr);
}
#[test]
fn native_syntax_only_for_all_reference_inputs_and_outputs() {
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
fn native(ruby: &str, source: &str) -> Output {
    Command::new(ruby)
        .args(["--parser=prism", "-w", "-e", source])
        .output()
        .unwrap()
}
#[test]
fn authored_native_values_frozen_chilled_encoding_bindings_dispatch_and_source_controls() {
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
    // Authored live literals. These are not executed spec/reference fixtures.
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
        "# coding: ISO-8859-1\n",
    ] {
        for (before, after) in [
            ("\"authored\" \"\"", "\"authored\""),
            ("\"\"\"authored\"\"\"", "\"authored\""),
            ("''''''", "''"),
            ("\"\"\"line\nvalue\"\"\"", "\"line\nvalue\""),
        ] {
            let tail = "; p [value.bytes,value.encoding.name,value.frozen?,__LINE__,local_variables]; begin; value << '!'; p value; rescue FrozenError; puts :frozen; end\n";
            let a = native(&ruby, &format!("{header}value = {before}{tail}"));
            let b = native(&ruby, &format!("{header}value = {after}{tail}"));
            assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
            assert_eq!(a.stdout, b.stdout);
            assert_eq!(a.stderr, b.stderr);
        }
    }
    // Nonempty + is observable and must never be introduced.
    let setup = "class String; def +(other); raise 'authored dispatch'; end; end; ";
    let a = native(&ruby, &format!("{setup}p(\"left\" \"right\")"));
    let b = native(&ruby, &format!("{setup}p(\"left\" + \"right\")"));
    assert!(a.status.success());
    assert!(!b.status.success());
    // Dynamic concatenation can alter allocation/coercion/lifetime, so held.
    let a = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('def f(x); \"#{x}\" \"\"; end').disasm",
    );
    let b = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('def f(x); \"#{x}\"; end').disasm",
    );
    assert_ne!(a.stdout, b.stdout);
    // Even static canonical edits change source and code-object coordinates.
    let a = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('value = \"authored\" \"\"').to_a",
    );
    let b = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('value = \"authored\"').to_a",
    );
    assert_ne!(a.stdout, b.stdout);
    let a = native(&ruby, "p %q{\"authored\" \"\"}.bytes");
    let b = native(&ruby, "p %q{\"authored\"}.bytes");
    assert_ne!(a.stdout, b.stdout);
}
