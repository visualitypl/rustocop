//! default startup namespaces are not fresh, without project preloads.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}

fn cli(dir: &Path, cop: &str, fix: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    command.current_dir(dir).args(["--only", cop]);
    if fix {
        command.arg("--fix");
    }
    let out = command.arg("a.rb").output().unwrap();
    assert!(out.stderr.is_empty(), "{cop}: {out:?}");
    out
}

fn no_write(dir: &Path, cop: &str, source: &str) {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    for fix in [false, true] {
        let out = cli(dir, cop, fix);
        assert_eq!(out.status.code(), Some(1), "{cop}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        assert_eq!(fs::read_to_string(&path).unwrap(), source, "{cop}");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

fn runtimes() -> Vec<String> {
    let mut runtimes = Vec::new();
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        runtimes.push("ruby".into());
    }
    if let Ok(native) = crate::support::runtime::ruby()
        && !runtimes.contains(&native)
    {
        runtimes.push(native);
    }
    runtimes
}

fn ruby(dir: &Path, runtime: &str, program: &str) -> Output {
    fs::write(dir.join("witness.rb"), program).unwrap();
    let out = Command::new(runtime)
        .current_dir(dir)
        // Use ordinary startup, not --disable-gems or an invented preload.
        .env_remove("RUBYOPT")
        .env_remove("RUBYLIB")
        .env_remove("GEM_HOME")
        .env_remove("GEM_PATH")
        .env_remove("BUNDLE_GEMFILE")
        .args(["-w", "witness.rb"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{runtime}: {out:?}");
    out
}

#[test]
fn startup_module_nested_constant_witness_preserves_mixin_exception_atomicity() {
    let d = project();
    // Ordinary host startup defines SyntaxSuggest::MiniStringIO (a class).
    // No user callback, project preload or monkey patch is necessary.
    let before = "module MiniStringIO; end\nmodule Other; end\nmodule SyntaxSuggest\n  include MiniStringIO, Other\nend\n";
    let naive = before.replace(
        "include MiniStringIO, Other",
        "include Other\n  include MiniStringIO",
    );
    no_write(d.path(), "Style/MixinGrouping", before);
    for runtime in runtimes() {
        let present = ruby(
            d.path(),
            &runtime,
            "p defined?(SyntaxSuggest::MiniStringIO)\n",
        );
        if present.stdout == b"nil\n" {
            continue;
        }
        let observe = "rescue TypeError\n  p SyntaxSuggest.ancestors.include?(Other)\nend\n";
        let original = ruby(d.path(), &runtime, &format!("begin\n{before}{observe}"));
        let changed = ruby(d.path(), &runtime, &format!("begin\n{naive}{observe}"));
        assert_eq!(original.stdout, b"false\n", "{runtime}: {original:?}");
        assert_eq!(changed.stdout, b"true\n", "{runtime}: {changed:?}");
    }
}

#[test]
fn replacing_startup_constants_and_reopening_them_are_not_equivalent() {
    let d = project();
    for runtime in runtimes() {
        for name in ["SyntaxSuggest", "Monitor"] {
            let present = ruby(d.path(), &runtime, &format!("p defined?({name})\n"));
            if present.stdout == b"nil\n" {
                continue;
            }
            let before = format!("{name} = Class.new(StandardError)\n");
            no_write(d.path(), "Style/EmptyClassDefinition", &before);
            let observe =
                format!("p {name}.superclass\nrescue TypeError\nputs :reopening_failed\nend\n");
            let original = ruby(d.path(), &runtime, &format!("begin\n{before}{observe}"));
            let changed = ruby(
                d.path(),
                &runtime,
                &format!("begin\nclass {name} < StandardError\nend\n{observe}"),
            );
            assert_eq!(original.stdout, b"StandardError\n");
            assert_eq!(changed.stdout, b"reopening_failed\n");
        }
    }
}
