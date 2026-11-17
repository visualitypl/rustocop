//! real selectors/config/directives, safe edits and controlled Ruby observations.
//! Pinned fixtures port RuboCop specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const IMPLICIT: &str = "Style/ImplicitRuntimeError";
const ARGS: &str = "Style/RaiseArgs";
const REDUNDANT: &str = "Style/RedundantException";
const ALL: &str = "Style/ImplicitRuntimeError,Style/RaiseArgs,Style/RedundantException";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, counts: [usize; 3]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    for (cop, count) in [IMPLICIT, ARGS, REDUNDANT].into_iter().zip(counts) {
        assert_eq!(
            text.lines().filter(|line| line.contains(cop)).count(),
            count,
            "{args:?}: {text}"
        );
    }
    out
}
fn unchanged(dir: &Path, args: &[&str], source: &[u8], status: i32, counts: [usize; 3]) {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir, args, status, counts);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn defaults_opt_in_selectors_config_and_no_write() {
    let dir = project();
    let source = b"raise 'x'\nraise OtherError.new('x')\nraise RuntimeError, 'x'\n";
    for (config, args, counts) in [
        (String::new(), vec!["lib/a.rb"], [0, 1, 1]),
        (String::new(), vec!["--only", ALL, "lib/a.rb"], [1, 1, 1]),
        (
            String::new(),
            vec!["--only", IMPLICIT, "lib/a.rb"],
            [1, 0, 0],
        ),
        (
            String::new(),
            vec![
                "--except",
                "Style/RaiseArgs,Style/RedundantException",
                "lib/a.rb",
            ],
            [0, 0, 0],
        ),
        (
            format!("[cops.\"{IMPLICIT}\"]\nenabled = true\n"),
            vec!["lib/a.rb"],
            [1, 1, 1],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", ALL, "lib/a.rb"],
            [1, 1, 1],
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{IMPLICIT}\"]\nenabled = true\n"),
            vec!["lib/a.rb"],
            [1, 0, 0],
        ),
        (
            format!(
                "[cops.\"{ARGS}\"]\nenabled = false\n[cops.\"{REDUNDANT}\"]\nenabled = false\n"
            ),
            vec!["lib/a.rb"],
            [0, 0, 0],
        ),
        (
            String::new(),
            vec!["--only", ALL, "--except", ALL, "lib/a.rb"],
            [0, 0, 0],
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            &args,
            source,
            // Unselected default cops also flag the deliberately sequential raises.
            i32::from(counts != [0, 0, 0] || !args.contains(&"--only")),
            counts,
        );
    }
}
#[test]
fn excludes_both_directive_prefixes_and_opt_in_source_controls() {
    let dir = project();
    for (cop, code) in [
        (IMPLICIT, "raise 'x'"),
        (ARGS, "raise Ex.new('x')"),
        (REDUNDANT, "raise RuntimeError, 'x'"),
    ] {
        for prefix in ["rustocop", "rubocop"] {
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            for source in [
                format!("# {prefix}:disable {cop}\n{code}\n"),
                format!("{code} # {prefix}:disable {cop}\n"),
            ] {
                unchanged(
                    dir.path(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    source.as_bytes(),
                    0,
                    [0, 0, 0],
                );
            }
        }
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
            "exclude = ['lib/**']\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                &["--only", cop, "--fix", "lib/a.rb"],
                code.as_bytes(),
                0,
                [0, 0, 0],
            );
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    unchanged(
        dir.path(),
        &["--fix", "--only", REDUNDANT, "lib/a.rb"],
        b"raise(RuntimeError,\n  'x' # rubocop:disable Style/RedundantException\n)\n",
        0,
        [0, 0, 0],
    );
    // A signed enable actually opts the default-off cop into the real pipeline.
    unchanged(
        dir.path(),
        &["lib/a.rb"],
        b"# rustocop:enable Style/ImplicitRuntimeError\nraise 'x'\n",
        1,
        [1, 0, 0],
    );
}
#[test]
fn stdin_opt_in_and_fix_rejection_do_not_write_named_paths() {
    use std::io::Write;
    use std::process::Stdio;
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "untouched\n").unwrap();
    for (cop, source) in [
        (IMPLICIT, "raise 'x'\n"),
        (ARGS, "raise Error.new\n"),
        (REDUNDANT, "raise RuntimeError, 'x'\n"),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains(cop));
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--fix", "--stdin", "lib/a.rb"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("--fix"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "untouched\n");
    }
}
fn rubies() -> Vec<String> {
    let mut result = Vec::new();
    if Command::new("ruby").arg("--version").output().is_ok() {
        result.push("ruby".into());
    }
    if let Ok(native) = crate::support::runtime::ruby() {
        let version = Command::new(&native)
            .arg("--version")
            .output()
            .expect("configured Ruby 3.4 must run");
        assert!(version.status.success());
        assert!(
            String::from_utf8_lossy(&version.stdout)
                .starts_with(crate::support::runtime::banner_prefix())
        );
        result.push(native);
    }
    result
}
fn ruby(ruby: &str, dir: &Path) -> Output {
    let mut command = Command::new(ruby);
    command.current_dir(dir);
    if dir.join("observer.rb").exists() {
        command.args(["-r", "./observer.rb"]);
    }
    command.arg("lib/a.rb").output().unwrap()
}
#[test]
fn canonical_fixes_preserve_exception_ancestry_cause_backtrace_ensure_and_return_contexts() {
    let dir = project();
    // Observe backtraces from a separate controlled harness: a written backtrace
    // path acquisition correctly triggers the shared own-source correction barrier.
    fs::write(
        dir.path().join("observer.rb"),
        "def observed_line(e); e.backtrace.first.split(':')[1]; end\n",
    )
    .unwrap();
    let source = r#"$events = []
def method_case
  begin
    raise ArgumentError, 'cause'
  rescue ArgumentError => original
    begin
      raise RuntimeError, "bad\x20message"
    rescue RuntimeError => error
      $events << [error.class.name, error.is_a?(StandardError), error.message, error.cause.equal?(original), observed_line(error)]
      :handled
    ensure
      $events << :ensure
    end
  end
end
result = method_case
values = [-> { raise RuntimeError.new('lambda') }, proc { fail(::RuntimeError, 'proc') }].map do |callable|
  begin
    callable.call
  rescue RuntimeError => error
    [error.class.name, error.message, observed_line(error)]
  ensure
    $events << :block_ensure
  end
end
p [result, values, $events, __LINE__]
"#;
    for runtime in &rubies() {
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, source).unwrap();
        let before = ruby(runtime, dir.path());
        assert!(before.status.success(), "{before:?}");
        run(
            dir.path(),
            &["--only", REDUNDANT, "--fix", "lib/a.rb"],
            0,
            [0, 0, 0],
        );
        assert_ne!(
            fs::read_to_string(&path).unwrap(),
            source,
            "expected real corrections"
        );
        let after = ruby(runtime, dir.path());
        assert_eq!(
            (after.status.code(), after.stdout, after.stderr),
            (before.status.code(), before.stdout, before.stderr),
            "{runtime}"
        );
    }
}
#[test]
fn selected_style_and_layout_fixes_stabilize_without_enabling_unsafe_corrections() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def f; fail RuntimeError.new('bad'); end\n";
    fs::write(&path, source).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let before = fs::read(&path).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop")).current_dir(dir.path()).args(["--only","Style/SignalException,Style/RaiseArgs,Style/RedundantException,Style/StringLiterals,Style/Semicolon,Style/RedundantParentheses,Layout","--fix","lib/a.rb"]).output().unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
        if fs::read(&path).unwrap() == before {
            stable = true;
            break;
        }
    }
    assert!(stable);
    let actual = fs::read_to_string(&path).unwrap();
    assert!(actual.contains("raise \"bad\""), "{actual}");
    assert!(!actual.contains("RuntimeError"));
    // The opt-in alternative intentionally disagrees with the canonical removal.
    // It reports the new implicit message but never emits an opposing edit.
    fs::write(&path, "raise RuntimeError, \"bad\"\n").unwrap();
    run(
        dir.path(),
        &["--only", ALL, "--fix", "lib/a.rb"],
        1,
        [1, 0, 0],
    );
    unchanged(
        dir.path(),
        &["--only", ALL, "--fix", "lib/a.rb"],
        b"raise \"bad\"\n",
        1,
        [1, 0, 0],
    );
}
