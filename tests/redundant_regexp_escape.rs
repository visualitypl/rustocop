//! pinned RuboCop redundant_regexp_escape_spec.rb examples (MIT,
//! Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/RedundantRegexpEscape";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
}
#[test]
fn default_selectors_config_excludes_directives_and_no_write() {
    let d = project();
    let source = b"/a\\-b/\n";
    let after = b"/a-b/\n";
    let registry = rustocop::cops::Registry::new();
    let rule = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(rule.default_enabled() && rule.safe_fixes());
    unchanged(d.path(), source, &["a.rb"], 1);
    unchanged(d.path(), source, &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        source,
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled=false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled=false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--fix", "a.rb"], 0);
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 0);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle=\"other\"\n"),
    )
    .unwrap();
    unchanged(d.path(), source, &["--fix", "a.rb"], 2);
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\n/a\\-b/\n"),
                format!("# {prefix}:disable-next {target}\n/a\\-b/\n"),
                format!("/a\\-b/ # {prefix}:disable {target}\n"),
            ] {
                unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 0);
            }
        }
    }
    unchanged(d.path(), after, &["--only", COP, "--fix", "a.rb"], 0);
    unchanged(d.path(), b"(\n", &["--only", COP, "--fix", "a.rb"], 2);
}
#[test]
fn encoding_bom_crlf_line_and_data_bytes() {
    let d = project();
    for prefix in [
        "",
        "\u{feff}",
        "# coding: UTF-8\n",
        "# coding: ISO-8859-1\n",
    ] {
        for nl in ["\n", "\r\n"] {
            let source =
                format!("{prefix}/a\\-b/imx\np __LINE__\n__END__\n/a\\-b/\n").replace('\n', nl);
            let after =
                format!("{prefix}/a-b/imx\np __LINE__\n__END__\n/a\\-b/\n").replace('\n', nl);
            fs::write(d.path().join("a.rb"), source).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        }
    }
    for (source, after) in [
        (
            b"# coding: Windows-31J\n/\x83\x5c\\-\x83\x7b/\n".as_slice(),
            b"# coding: Windows-31J\n/\x83\x5c-\x83\x7b/\n".as_slice(),
        ),
        (
            b"# coding: ISO-8859-1\n/[\\\xa7]\\-\\\xe9/\n",
            b"# coding: ISO-8859-1\n/[\xa7]-\\\xe9/\n",
        ),
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
    }
}
fn ruby(dir: &Path, executable: &std::ffi::OsStr) -> Output {
    Command::new(executable)
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap()
}
#[test]
fn runtime_named_tokens_preserve_necessary_escapes_and_safe_siblings() {
    let d = project();
    let mut rubies = vec!["ruby".into()];
    if let Some(native) = crate::support::runtime::ruby_os() {
        rubies.push(native);
    }
    for executable in rubies {
        if Command::new(&executable).arg("--version").output().is_err() {
            continue;
        }
        for pattern in [
            r"/(?<a[b>x)\+/",
            r"/(?'a[b'x)\+/",
            r"/(?<a[b>x)\k<a[b>\+/",
            r"/(?'a[b'x)\k'a[b'\+/",
            r"/(?<a[b>x)\g<a[b>\+/",
            r"/(?'a[b'x)\g'a[b'\+/",
        ] {
            let source = format!(
                "r = {pattern}; p ['x', 'xx', 'x+', 'xx+'].map {{ |s| r.match?(s) }}\n/a\\-b/\n"
            );
            let expected = source.replace(r"/a\-b/", "/a-b/");
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let before = ruby(d.path(), &executable);
            assert!(before.status.success(), "{source}: {before:?}");
            cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
            let after = ruby(d.path(), &executable);
            assert_eq!(before.status, after.status);
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
            // The previously emitted edit was parseable but changed matches.
            let naive = source.replace(r"\+", "+");
            fs::write(d.path().join("a.rb"), naive).unwrap();
            let changed = ruby(d.path(), &executable);
            assert!(changed.status.success(), "{changed:?}");
            assert_ne!(before.stdout, changed.stdout, "{source}");
        }
    }
}
