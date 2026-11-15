//! real selectors, safe feature corrections and detection-only ordering.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const EXT: &str = "Style/RedundantFileExtensionInRequire";
const ORDER: &str = "Style/RequireOrder";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, counts: (usize, usize)) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for (cop, count) in [(EXT, counts.0), (ORDER, counts.1)] {
        assert_eq!(
            stdout.lines().filter(|line| line.contains(cop)).count(),
            count,
            "{args:?}: {stdout}"
        );
    }
    output
}
#[test]
fn default_opt_in_config_and_exclusive_selectors_use_real_cli() {
    let dir = project();
    let source = "require \"b.rb\"\nrequire \"a.rb\"\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    for (config, args, counts) in [
        ("".to_string(), vec!["a.rb"], (2, 0)),
        ("".to_string(), vec!["--only", ORDER, "a.rb"], (0, 1)),
        ("".to_string(), vec!["--only", EXT, "a.rb"], (2, 0)),
        (
            format!("[cops.\"{ORDER}\"]\nenabled = true\n"),
            vec!["a.rb"],
            (2, 1),
        ),
        (
            format!("[cops.\"{EXT}\"]\nenabled = false\n"),
            vec!["a.rb"],
            (0, 0),
        ),
        (
            "[cops.Style]\nenabled = false\n".to_string(),
            vec!["--only", ORDER, "a.rb"],
            (0, 1),
        ),
        (
            format!("[cops.\"{ORDER}\"]\nenabled = true\n"),
            vec!["--only", ORDER, "--except", ORDER, "a.rb"],
            (0, 0),
        ),
        (
            "".to_string(),
            vec!["--only", EXT, "--except", EXT, "a.rb"],
            (0, 0),
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        run(dir.path(), &args, i32::from(counts != (0, 0)), counts);
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
    }
}
#[test]
fn directives_opt_ins_and_config_relative_excludes_suppress_writes() {
    let dir = project();
    fs::create_dir(dir.path().join("lib")).unwrap();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rustocop", "rubocop"] {
        let source = format!("# {prefix}:disable {EXT}\nrequire 'b.rb'\nrequire 'a.rb'\n");
        fs::write(&path, &source).unwrap();
        run(dir.path(), &["--only", EXT, "--fix", "lib/a.rb"], 0, (0, 0));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        let source = format!("# {prefix}:enable {ORDER}\nrequire \"b\"\nrequire \"a\"\n");
        fs::write(&path, &source).unwrap();
        run(dir.path(), &["--fix", "lib/a.rb"], 1, (0, 1));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        let source = format!("require \"b\"\nrequire \"a\" # {prefix}:disable {ORDER}\n");
        fs::write(&path, &source).unwrap();
        run(
            dir.path(),
            &["--only", ORDER, "--fix", "lib/a.rb"],
            0,
            (0, 0),
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
    for cop in [EXT, ORDER] {
        let source = "require 'b.rb'\nrequire 'a.rb'\n";
        fs::write(&path, source).unwrap();
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
            "exclude = ['lib/**']\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            run(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
                (0, 0),
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
    }
}
#[test]
fn clean_error_legacy_encoding_and_uncorrectable_files_retain_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (cop, source, status, counts) in [
        (EXT, b"require 'foo'\n".as_slice(), 0, (0, 0)),
        (EXT, b"require(\n".as_slice(), 2, (0, 0)),
        (
            EXT,
            b"# encoding: ISO-8859-1\r\nrequire 'caf\xe9.rb'\r\n".as_slice(),
            1,
            (1, 0),
        ),
        (
            EXT,
            b"def require(path); p path; end\nrequire 'foo.rb'\n".as_slice(),
            1,
            (1, 0),
        ),
        (EXT, b"require '.rb'\n".as_slice(), 1, (1, 0)),
        (ORDER, b"require 'b'\nrequire 'a'\n".as_slice(), 1, (0, 1)),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", cop, "--fix", "a.rb"],
            status,
            counts,
        );
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap()
}
fn assert_same(before: &Output, after: &Output) {
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.status.code(), after.status.code(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn runtime_repeated_and_native_extension_stems_preserve_feature_identity() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    for suffix in ["rb", "so", "o", "bundle", "dll", "dylib"] {
        fs::write(
            dir.path().join(format!("feature.{suffix}.rb")),
            "puts :intended\n",
        )
        .unwrap();
        fs::write(
            dir.path().join(format!("feature.{suffix}")),
            "puts :different\n",
        )
        .unwrap();
        for method in ["require", "require_relative"] {
            let source = format!(
                "$LOAD_PATH.unshift(__dir__)\nbegin\n  {method} 'feature.{suffix}.rb'\nrescue LoadError\n  puts :wrong_native_feature\nend\n"
            );
            let path = dir.path().join("a.rb");
            fs::write(&path, &source).unwrap();
            let before = ruby(dir.path());
            assert_eq!(before.stdout, b"intended\n");
            run(dir.path(), &["--only", EXT, "--fix", "a.rb"], 1, (1, 0));
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            assert_same(&before, &ruby(dir.path()));
        }
    }
}
#[test]
fn runtime_custom_require_binders_are_not_rewritten() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    for source in [
        "def require(path); p [path.bytes, path.encoding.name]; end\nrequire 'feature.rb'\n",
        "class Loader; def require_relative(path); p path; end; def run; require_relative 'feature.rb'; end; end; Loader.new.run\n",
        "module Paths; refine Kernel do; def require(path); p path; end; end; end; using Paths; require 'feature.rb'\n",
    ] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path());
        run(dir.path(), &["--only", EXT, "--fix", "a.rb"], 1, (1, 0));
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        assert_same(&before, &ruby(dir.path()));
    }
}
#[test]
fn mixed_extension_prefix_style_layout_fixes_stabilize_without_reordering_loads() {
    let dir = project();
    let path = dir.path().join("a.rb");
    fs::write(&path, "require_relative './z.rb';  \nrequire_relative './a.rb'\ndef f\n  return ({:a => 'value'});\nend\n").unwrap();
    let cops = format!(
        "{EXT},Style/RedundantCurrentDirectoryInPath,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout"
    );
    let mut stable = false;
    for _ in 0..4 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", &cops, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        if output.status.success() {
            stable = true;
            break;
        }
    }
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(stable, "{fixed}");
    assert!(
        fixed.starts_with("require_relative \"z\"\nrequire_relative \"a\"\n"),
        "{fixed}"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir.path(), &["--only", ORDER, "--fix", "a.rb"], 1, (0, 1));
    assert_eq!(fs::read_to_string(&path).unwrap(), fixed);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
