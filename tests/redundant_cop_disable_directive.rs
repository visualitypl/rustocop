//! Real CLI/pipeline and independently authored native safety controls.
//! RuboCop-derived directive forms: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/RedundantCopDisableDirective";
const SELECTED: &str = "Lint/RedundantCopDisableDirective,Lint/Debugger";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn count(out: &Output) -> usize {
    assert_ne!(out.status.code(), Some(2), "{out:?}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
        .count()
}
fn check(root: &Path, source: &[u8], args: &[&str], expected: usize, after: &[u8]) {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = cli(root, args);
    assert_eq!(count(&out), expected, "{args:?}: {out:?}");
    assert_eq!(fs::read(&path).unwrap(), after, "{args:?}: {out:?}");
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.permissions(), new.permissions());
    if source == after {
        assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    }
}
#[test]
fn actual_defaults_selectors_config_precedence_paths_and_stdin() {
    let d = project();
    let source = b"# rubocop:disable Lint/Debugger\nputs 1\n";
    for (args, expected) in [
        (vec!["lib/a.rb"], 1),
        (vec!["--only", SELECTED, "lib/a.rb"], 1),
        (vec!["--only", COP, "lib/a.rb"], 0), // Debugger did not run.
        (
            vec!["--only", SELECTED, "--except", "Lint/Debugger", "lib/a.rb"],
            0,
        ),
        (vec!["--only", SELECTED, "--except", COP, "lib/a.rb"], 0),
    ] {
        check(d.path(), source, &args, expected, source);
    }
    for (toml, expected) in [
        ("[cops.Lint]\nenabled = false\n", 0),
        (
            "[cops.Lint]\nenabled = false\n[cops.\"Lint/RedundantCopDisableDirective\"]\nenabled = true\n[cops.\"Lint/Debugger\"]\nenabled = true\n",
            1,
        ),
        (
            "[cops.\"Lint/RedundantCopDisableDirective\"]\nenabled = false\n",
            0,
        ),
        ("[cops.\"Lint/Debugger\"]\nexclude = [\"lib/**\"]\n", 0),
        (
            "[cops.\"Lint/RedundantCopDisableDirective\"]\nexclude = [\"lib/**\"]\n",
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), toml).unwrap();
        check(d.path(), source, &["lib/a.rb"], expected, source);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Lint/RedundantCopDisableDirective\"]\nexclude = [\"lib/skip.rb\"]\n",
    )
    .unwrap();
    for (path, expected) in [("lib/a.rb", 1), ("lib/skip.rb", 0)] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", SELECTED, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(source).unwrap();
        assert_eq!(count(&child.wait_with_output().unwrap()), expected);
    }
    assert_eq!(
        cli(d.path(), &["--stdin", "a.rb", "--fix"]).status.code(),
        Some(2)
    );
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Lint/RedundantCopDisableDirective\"]\nstyle = 'anything'\n",
    )
    .unwrap();
    assert_eq!(cli(d.path(), &["lib/a.rb"]).status.code(), Some(2));
}
#[test]
fn actual_scopes_unknowns_inactive_paths_and_opt_ins() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for (body, expected) in [
            ("# P:disable Debugger\nputs 1\n", 1),
            ("# P:disable Debugger\nbinding.pry\n", 0),
            ("# P:disable-next Debugger\nbinding.pry(value)\n", 0),
            ("# P:disable-next Debugger\n\nputs 1\n", 1),
            ("# P:next -Debugger\nputs 1\n", 0),
            ("# P:next -Debugger\n", 1),
            ("# P:enable-next Debugger\n", 1),
            ("# P:push -Debugger\nputs 1\n# P:pop\n", 0),
            ("binding.pry(\n value\n) # P:disable Debugger\n", 0),
            ("# P:disable all\n", 0), // Inactive/unselected peers remain unknown evidence.
            (
                "# P:disable Lint/RedundantCopDisableDirective\n# P:disable UnknownCop\n",
                0,
            ),
            ("# P:disable Style/Void\n", 1),
            ("# P:disable Lint/NonDeterministicRequireOrder\n", 0),
            ("# P:disable Lint/UnusedPrivateMethod\n", 1), // Already configured-off.
            (
                "# P:enable Lint/UnusedPrivateMethod\n# P:disable Lint/UnusedPrivateMethod\n",
                0,
            ),
        ] {
            let source = body.replace("P:", &format!("{prefix}:"));
            check(
                d.path(),
                source.as_bytes(),
                &["lib/a.rb"],
                expected,
                source.as_bytes(),
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            "[cops.\"Lint/RedundantCopDisableDirective\"]\nenabled = false\n",
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\n# {prefix}:disable Lint/Debugger\nputs 1\n");
        check(
            d.path(),
            source.as_bytes(),
            &["lib/a.rb"],
            1,
            source.as_bytes(),
        );
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn real_canonical_edits_no_write_holds_and_style_layout_composition() {
    let d = project();
    for source in [
        b"# rubocop:disable Lint/Debugger\n".as_slice(),
        b"# rustocop:todo Lint/Debugger -- obsolete\r\n",
    ] {
        check(
            d.path(),
            source,
            &["--only", SELECTED, "lib/a.rb"],
            1,
            source,
        );
        check(
            d.path(),
            source,
            &["--only", SELECTED, "--fix", "lib/a.rb"],
            0,
            b"",
        );
    }
    for source in [
        "# rubocop:disable Lint/Debugger\nputs 1\n",
        "# rubocop:disable UnknownCop\n",
        "# rubocop:disable Style/Void\n",
        "\u{feff}# rubocop:disable Lint/Debugger\n",
        "#!/usr/bin/env ruby\n# rubocop:disable Lint/Debugger\n",
        "# coding: shift_jis\n# rubocop:disable Lint/Debugger\n",
        "# rubocop:disable Lint/Debugger\np __LINE__\n",
        "# rubocop:disable Lint/Debugger\np DATA.pos\n__END__\na\n",
        "# rubocop:disable Lint/Debugger\np File.binread(__FILE__)\n",
        "# rubocop:disable Lint/Debugger\np ObjectSpace.count_objects\n",
        "# rubocop:disable Lint/Debugger\np RubyVM::InstructionSequence.of(-> {}).to_a\n",
    ] {
        check(
            d.path(),
            source.as_bytes(),
            &["--only", SELECTED, "--fix", "lib/a.rb"],
            1,
            source.as_bytes(),
        );
    }
    // This cop's withheld edit does not suppress accepted Style/Layout fixes.
    let source = b"# rubocop:disable Lint/Debugger\nx = 'value'  \n";
    check(
        d.path(),
        source,
        &[
            "--only",
            "Lint/RedundantCopDisableDirective,Lint/Debugger,Style/StringLiterals,Layout/TrailingWhitespace",
            "--fix",
            "lib/a.rb",
        ],
        1,
        b"# rubocop:disable Lint/Debugger\nx = \"value\"\n",
    );
}
#[test]
fn invalid_input_is_fatal_and_symlink_atomic_controls_remain_shared() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    let source = b"# rubocop:disable Lint/Debugger\ndef bad(\n";
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        cli(d.path(), &["--only", SELECTED, "--fix", "lib/a.rb"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&path, "# rubocop:disable Lint/Debugger\n").unwrap();
        symlink("a.rb", d.path().join("lib/link.rb")).unwrap();
        let out = cli(d.path(), &["--only", SELECTED, "--fix", "lib/link.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert!(
            fs::symlink_metadata(d.path().join("lib/link.rb"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&path).unwrap(), b"");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
#[test]
fn authored_native_closed_comment_nil_vm_trace_heap_binding_proofs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let native = |source: &str| {
        Command::new(&ruby)
            .args(["--parser=prism", "-w", "-e", source])
            .output()
            .unwrap()
    };
    let d = project();
    // Independently authored programs only, never the pinned spec/corpus inputs.
    for (before, after) in [
        ("# rustocop:disable Lint/Debugger -- retired\n", ""),
        ("\n# rubocop:todo Lint/Debugger\r\n", "\n"),
        (
            "# rubocop:disable Lint/Debugger\n# rubocop:enable Lint/Debugger\n",
            "# rubocop:enable Lint/Debugger\n",
        ),
    ] {
        check(
            d.path(),
            before.as_bytes(),
            &["--only", SELECTED, "--fix", "lib/a.rb"],
            0,
            after.as_bytes(),
        );
        for probe in [
            "p [i.eval, i.to_a]",
            "events = []; t = TracePoint.new(:line, :call, :return, :raise) { |e| events << [e.event,e.lineno] if e.path == 'authored.rb' }; t.enable; value = i.eval; t.disable; p [value,events]",
            "b = binding; p [i.eval, b.local_variables]",
            "GC.disable; i.eval; n = GC.stat(:total_allocated_objects); 100.times { i.eval }; p GC.stat(:total_allocated_objects) - n",
        ] {
            let run = |s| {
                native(&format!(
                    "i = RubyVM::InstructionSequence.compile({}, 'authored.rb', 'authored.rb', 1); {probe}",
                    serde_json::to_string(s).unwrap()
                ))
            };
            let a = run(before);
            let b = run(after);
            assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
            assert_eq!(a.stdout, b.stdout, "{probe}");
            assert_eq!(a.stderr, b.stderr, "{probe}");
            assert!(a.stderr.is_empty());
        }
    }
    let a = native("# rubocop:disable Lint/Debugger\np __LINE__");
    let b = native("p __LINE__");
    assert_ne!(a.stdout, b.stdout); // Ordinary upstream deletion is not safe.
}
