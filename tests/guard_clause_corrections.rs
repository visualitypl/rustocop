//! Pinned RuboCop guard_clause_spec.rb examples (MIT License,
//! Copyright (c) 2012-26 Bozhidar Batsov), plus controlled safety witnesses.
use std::{fs, path::Path, process::Command};

const COP: &str = "Style/GuardClause";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], code: i32) {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: i32) {
    let file = root.join("a.rb");
    fs::write(&file, source).unwrap();
    let time = fs::metadata(&file).unwrap().modified().unwrap();
    cli(root, args, code);
    assert_eq!(fs::read(&file).unwrap(), source);
    assert_eq!(fs::metadata(file).unwrap().modified().unwrap(), time);
}
fn correct(root: &Path, before: &[u8], after: &[u8], selectors: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(["--only", selectors, "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(fs::read(root.join("a.rb")).unwrap(), after, "{o:?}");
}
#[test]
fn fixing_obeys_config_selectors_and_reenabled_directives() {
    let d = project();
    let before = b"def f\n  if ready?\n    work\n  end\nend\n";
    let after = b"def f\n  return unless ready?\n    work\n  \nend\n";
    unchanged(d.path(), before, &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        before,
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        format!(
            "[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n[cops.\"{COP}\"]\nenabled=false\n"
        ),
        "[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n".into(),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), before, &["--fix", "a.rb"], 0);
        correct(d.path(), before, after, COP);
    }
    fs::write(d.path().join("rustocop.toml"), format!("[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n")).unwrap();
    fs::write(d.path().join("a.rb"), before).unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        let first = format!(
            "# {prefix}:disable {COP}\n{}# {prefix}:enable {COP}\n",
            String::from_utf8_lossy(before)
        );
        correct(
            d.path(),
            format!("{first}{}", String::from_utf8_lossy(before)).as_bytes(),
            format!("{first}{}", String::from_utf8_lossy(after)).as_bytes(),
            COP,
        );
    }
}

#[test]
fn default_fix_is_a_single_atomic_pass_not_an_implicit_formatter_loop() {
    let d = project();
    let before = b"def f(c)\n  if c\n    value = { :foo => 'bar' }; side_effect(value)\n    return (value)\n  end\nend\n";
    let after = b"def f(c)\n  return unless c\n    value = { :foo => 'bar' }; side_effect(value)\n    return (value)\n  \nend\n";
    unchanged(d.path(), before, &["a.rb"], 1);
    fs::write(d.path().join("a.rb"), before).unwrap();
    cli(d.path(), &["--fix", "--fix-passes", "1", "a.rb"], 1);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
}

fn ruby_with(source: &[u8], program: &std::ffi::OsStr) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(program)
        .args(["--disable-gems", "-W0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
fn assert_runtime(before: &[u8], after: &[u8]) {
    let mut programs = vec![std::ffi::OsString::from("ruby")];
    if let Some(native) = crate::support::runtime::ruby_os() {
        programs.push(native);
    }
    for program in programs {
        let old = ruby_with(before, &program);
        let new = ruby_with(after, &program);
        assert!(
            old.status.success() && new.status.success(),
            "{program:?}: {old:?} {new:?}"
        );
        assert_eq!(old.stdout, new.stdout, "{program:?}");
        assert_eq!(old.stderr, new.stderr, "{program:?}");
    }
}

#[test]
fn controlled_return_values_effect_order_nested_guards_and_ensure_match_ruby() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    for source in [
        "def f(c, log)\n if c\n  log << :work\n  42\n end\nend\nl=[]; p [f(false,l), f(true,l), l]\n",
        "def f(c, log)\n unless c\n  log << :work\n  42\n end\nend\nl=[]; p [f(false,l), f(true,l), l]\n",
        "def f(c, log)\n if (log << :condition; c)\n  return (log << :exit; 42)\n else\n  log << :work\n  7\n end\nensure\n log << :ensure\nend\nl=[]; p [f(false,l), f(true,l), l]\n",
        "def f(c, log)\n if c\n  log << :work\n  7\n else\n  return (log << :exit; 42)\n end\nend\nl=[]; p [f(false,l), f(true,l), l]\n",
        "l=[]; p [false,true].map { |c|\n if c\n  next (l << :exit; 42)\n else\n  l << :work\n  7\n end\n}; p l\n",
        "l=[]; p [false,true].each { |c|\n if c\n  l << :work\n else\n  break (l << :exit; 42)\n end\n}; p l\n",
        "def f(a,b,log)\n if a\n  log << :outer\n  unless b\n   log << :inner\n   42\n  end\n end\nend\nl=[]; p [f(false,false,l),f(true,true,l),f(true,false,l),l]\n",
        "def f(c, log)\n if c\n  begin\n   log << :work\n   42\n  ensure\n   log << :ensure\n  end\n end\nend\nl=[]; p [f(false,l),f(true,l),l]\n",
        "def f(c, log)\n if c\n  __LINE__\n end\nend\np [f(false,[]),f(true,[]),__LINE__]\n",
        // Unknown define_method dispatch: next must not escape run's method.
        "def define_method(*)\n yield false\n yield true\nend\ndef run\n log=[]\n define_method(:f) do |c|\n  if c\n   log << :work\n   42\n  end\n end\n log << :after\nend\np run\n",
        "class C\n define_method(:f) do |c|\n  if c\n   42\n  end\n end\nend\np [C.new.f(false),C.new.f(true)]\n",
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        let after = fs::read(d.path().join("a.rb")).unwrap();
        assert_ne!(
            after,
            source.as_bytes(),
            "fixture must really correct: {source}"
        );
        assert_runtime(source.as_bytes(), &after);
    }
}
