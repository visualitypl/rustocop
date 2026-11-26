//! source-only opt-in detection, fixed path policy and no-write CLI.
use crate::support::cli::project;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;

const COP: &str = "Style/IpAddresses";
const SOURCE: &str = "puts \"127.0.0.1\"\n";

fn run(dir: &Path, args: &[&str], count: usize) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(2), "{args:?}: {out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout)
            .matches(&format!("({COP})"))
            .count(),
        count,
        "{args:?}: {out:?}"
    );
    out
}

fn unchanged(dir: &Path, source: &[u8], args: &[&str], count: usize) {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir, args, count);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
}

#[test]
fn defaults_opt_in_selectors_and_config_work_through_the_real_registry() {
    let dir = project();
    unchanged(dir.path(), SOURCE.as_bytes(), &["a.rb"], 0);
    for args in [
        vec!["--only", COP, "a.rb"],
        vec!["--only", COP, "--fix", "a.rb"],
    ] {
        unchanged(dir.path(), SOURCE.as_bytes(), &args, 1);
    }
    for config in [
        format!("[cops.\"{COP}\"]\nenabled = true\n"),
        "[cops.Style]\nenabled = true\n".into(),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(dir.path(), SOURCE.as_bytes(), &["a.rb"], 1);
        unchanged(dir.path(), SOURCE.as_bytes(), &["--except", COP, "a.rb"], 0);
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--except", COP, "--fix", "a.rb"],
            0,
        );
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    )
    .unwrap();
    unchanged(dir.path(), SOURCE.as_bytes(), &["a.rb"], 0);
    unchanged(dir.path(), SOURCE.as_bytes(), &["--only", COP, "a.rb"], 1);
}

#[test]
fn fixed_allowlist_and_numeric_or_version_knobs_cannot_be_reconfigured() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for option in [
        "AllowedAddresses = []",
        "allowed_addresses = []",
        "max = 20",
        "TargetRubyVersion = 3.0",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        fs::write(&path, SOURCE).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{option}: {out:?}");
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn pinned_path_exclusions_apply_even_when_force_selected() {
    let dir = project();
    for name in [
        "Gemfile",
        "gems.rb",
        "package.gemspec",
        "custom.gemfile",
        "lib/Gemfile",
        "lib/gems.rb",
        "lib/package.gemspec",
        "lib/custom.gemfile",
        "lib/ip.rb",
    ] {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, SOURCE).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix {
                args.push("--fix");
            }
            args.push(name);
            run(dir.path(), &args, usize::from(name == "lib/ip.rb"));
        }
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn user_excludes_are_config_relative_and_additive() {
    let dir = project();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("lib/a.rb"), SOURCE).unwrap();
    for config in [
        "exclude = ['lib/**']\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let path = dir.path().join("lib/a.rb");
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn both_directive_prefixes_suppress_restore_and_enable_opt_in() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for action in ["disable", "disable-next", "todo"] {
                let source = format!("# {prefix}:{action} {target}\n{SOURCE}");
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
            let source = format!("{} # {prefix}:disable {target}\n", SOURCE.trim_end());
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
        let source = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        let source = format!("# {prefix}:enable {COP}\n{SOURCE}");
        unchanged(dir.path(), source.as_bytes(), &["a.rb"], 1);
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--except", COP, "--fix", "a.rb"],
            0,
        );
    }
}

#[test]
fn source_envelopes_and_clean_or_error_files_keep_bytes_and_mtime() {
    let dir = project();
    for source in [
        "\u{feff}# encoding: UTF-8\r\nputs ['café', '::1', __LINE__]\r\n__END__\r\nraw\0ÿ\r\n".as_bytes(),
        b"# encoding: ISO-8859-1\r\nputs ['caf\xe9', '192.0.2.1', __LINE__]\r\n__END__\r\n'1.2.3.4'\xff\r\n",
    ] {
        unchanged(dir.path(), source, &["--only", COP, "a.rb"], 1);
        unchanged(dir.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    }
    unchanged(
        dir.path(),
        b"puts ['::', 'not an address']\n__END__\n'1.2.3.4'\n",
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    let path = dir.path().join("a.rb");
    fs::write(&path, b"(\n").unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), b"(\n");
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
}

#[test]
fn stdin_honors_opt_in_and_pinned_paths_without_writing_display_files() {
    let dir = project();
    for (name, only, fix, status) in [
        ("a.rb", false, false, 0),
        ("a.rb", true, false, 1),
        ("Gemfile", true, false, 0),
        ("a.rb", true, true, 2),
    ] {
        let path = dir.path().join(name);
        fs::write(&path, b"sentinel\n").unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let mut args = Vec::new();
        if only {
            args.extend(["--only", COP]);
        }
        if fix {
            args.push("--fix");
        }
        args.extend(["--stdin", name]);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Only the expected `--fix --stdin` rejection may exit before reading.
        stdin_support::write_stdin(child.stdin.take().unwrap(), SOURCE.as_bytes(), fix).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(status), "{out:?}");
        if fix {
            assert!(out.stdout.is_empty(), "{out:?}");
            assert!(
                String::from_utf8_lossy(&out.stderr)
                    .contains("the argument '--fix' cannot be used with '--stdin <PATH>'"),
                "{out:?}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), b"sentinel\n");
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn no_correction_preserves_literal_values_encoding_evaluation_and_control_flow() {
    let dir = project();
    let source = "# encoding: UTF-8\r\n$events = []\r\ndef address; $events << :read; '127.0.0.1'; end\r\ndef early; return '::1'; end\r\nvalues = [address, address, early, -> { '2001:db8::1' }.call, \"192.0.2.\\x31\", 'café']\r\np [values, values.map { |s| s.encoding.name }, $events, __LINE__, DATA.read.bytes]\r\n__END__\r\nraw\0ÿ\r\n";
    for binary in ["ruby", "/opt/ruby34/miniruby"] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let ruby = || {
            Command::new(binary)
                .current_dir(dir.path())
                .arg("a.rb")
                .output()
        };
        let Ok(before) = ruby() else { continue };
        assert!(before.status.success(), "{before:?}");
        let text = String::from_utf8_lossy(&before.stdout);
        assert!(text.contains("[:read, :read]"), "{text}");
        assert!(text.contains("192.0.2.1"), "{text}");
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            3,
        );
        let after = ruby().unwrap();
        assert_eq!(before.status.code(), after.status.code());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
