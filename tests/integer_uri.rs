//! source-only pin parity, actual CLI and authored native safety probes.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const INTEGER: &str = "Lint/UnifiedInteger";
const URI: &str = "Lint/UriEscapeUnescape";
const BOTH: &str = "Lint/UnifiedInteger,Lint/UriEscapeUnescape";
const SAFE: &[u8] = b"Fixnum = ::Integer\n[::Fixnum]\n";
const AFTER: &[u8] = b"Fixnum = ::Integer\n[::Integer]\n";
const UNCERTAIN: &[u8] = b"Fixnum\nURI.escape(uri)\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/integer_uri.json")).unwrap()
}
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
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
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(code) = code {
        assert_eq!(out.status.code(), Some(code), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}
#[test]
fn all_reference_rows_match_cli_with_real_canonical_fixes_and_uncertain_no_writes() {
    let dir = project();
    let mut fixed = 0;
    for row in rows() {
        let cop = row["cop"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "lib/a.rb"],
            Some(i32::from(!findings.is_empty())),
        );
        assert_eq!(count(&out, cop), findings.len(), "{row}\n{out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        for f in findings {
            assert!(
                text.contains(&format!(
                    "lib/a.rb:{}:{}: E: {} ({cop})",
                    f[0],
                    f[1],
                    f[3].as_str().unwrap()
                )),
                "{row}\n{text}"
            );
        }
        let after = row["project_after"].as_str().unwrap();
        if after == source {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(i32::from(!findings.is_empty())),
            );
            assert_eq!(count(&out, cop), findings.len());
        } else {
            fixed += 1;
            let out = run(dir.path(), &["--only", cop, "--fix", "lib/a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{row}\n{out:?}");
            assert_eq!(
                fs::read(dir.path().join("lib/a.rb")).unwrap(),
                after.as_bytes()
            );
            unchanged(
                dir.path(),
                after.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
    assert_eq!(fixed, 4);
}
#[test]
fn metadata_defaults_selectors_toml_department_precedence_and_strict_settings() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 582);
    for name in [INTEGER, URI] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled());
        assert_eq!(cop.safe_fixes(), name == INTEGER);
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
        assert!(cop.autocorrect_incompatible_with().is_empty());
        let dir = project();
        for (config, selectors, n) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", name], 1),
            (String::new(), vec!["--only", "Lint"], 1),
            (String::new(), vec!["--only", BOTH, "--except", name], 0),
            (String::new(), vec!["--only", BOTH, "--except", "Lint"], 0),
            ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
            (
                format!("[cops.Lint]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (
                format!("[cops.Lint]\nenabled = true\n[cops.\"{name}\"]\nenabled = false\n"),
                vec![],
                0,
            ),
            (
                format!("[cops.\"{name}\"]\nenabled = false\n"),
                vec!["--only", name],
                1,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("lib/a.rb");
            assert_eq!(
                count(&unchanged(dir.path(), UNCERTAIN, &args, None), name),
                n
            );
        }
        for key in [
            "max = 1",
            "include = ['**/*']",
            "Enabled = true",
            "enabled = 'true'",
            "target_ruby = 2.3",
            "SafeAutoCorrect = false",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{name}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                SAFE,
                &["--only", name, "--fix", "lib/a.rb"],
                Some(2),
            );
        }
        for config in [
            "exclude = ['lib/**']\n".into(),
            format!("[cops.\"{name}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = unchanged(
                dir.path(),
                SAFE,
                &["--only", name, "--fix", "lib/a.rb"],
                Some(0),
            );
            assert_eq!(count(&out, name), 0);
        }
    }
}
#[test]
fn directives_both_prefixes_multiline_next_push_and_source_enables() {
    let dir = project();
    for (cop, call) in [(INTEGER, "::Fixnum"), (URI, "URI.escape(uri)")] {
        for prefix in ["rustocop", "rubocop"] {
            for source in [
                format!("# {prefix}:disable {cop}\n{call}\n"),
                format!("# {prefix}:push -{cop}\n{call}\n# {prefix}:pop\n"),
                format!("# {prefix}:disable-next {cop}\n{call}\n"),
                format!("# {prefix}:next -{cop}\n{call}\n"),
                format!("{call} # {prefix}:disable {cop}\n"),
            ] {
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    Some(0),
                );
            }
            for (open, close) in [
                (
                    format!("# {prefix}:push +{cop}\n"),
                    format!("# {prefix}:pop\n"),
                ),
                (format!("# {prefix}:enable-next {cop}\n"), String::new()),
            ] {
                let source =
                    format!("# {prefix}:disable {cop}\n{call}\n{open}{call}\n{close}{call}\n");
                assert_eq!(
                    count(
                        &unchanged(
                            dir.path(),
                            source.as_bytes(),
                            &["--only", cop, "--fix", "lib/a.rb"],
                            Some(1)
                        ),
                        cop
                    ),
                    1
                );
            }
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            let source = format!("# {prefix}:enable {cop}\n{call}\n");
            assert_eq!(
                count(
                    &unchanged(dir.path(), source.as_bytes(), &["lib/a.rb"], None),
                    cop
                ),
                1
            );
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            let source = if cop == URI {
                format!("URI.escape(\nuri\n) # {prefix}:disable {cop}\n")
            } else {
                format!("::\nFixnum # {prefix}:disable {cop}\n")
            };
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                Some(0),
            );
            // Actual correctable edit is suppressed, not merely an uncertain read.
            let source = format!("Fixnum = ::Integer\n[::Fixnum] # {prefix}:disable {INTEGER}\n");
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", INTEGER, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
}
#[test]
fn supplied_paths_stdin_provenance_and_fixed_empty_path_policy() {
    let dir = project();
    for path in ["lib/a.arb", "lib/a.jb", "Rakefile", "lib/.hidden.rb"] {
        fs::write(dir.path().join(path), UNCERTAIN).unwrap();
        let out = run(dir.path(), &["--only", BOTH, "--fix", path]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!((count(&out, INTEGER), count(&out, URI)), (1, 1));
        assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:1:1")));
        assert_eq!(fs::read(dir.path().join(path)).unwrap(), UNCERTAIN);
    }
    for directory in ["vendor", "node_modules"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        fs::write(dir.path().join(directory).join("a.rb"), SAFE).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", BOTH, "--fix", directory])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(
            fs::read(dir.path().join(directory).join("a.rb")).unwrap(),
            SAFE
        );
    }
    for config in ["", "exclude = ['virtual/**']\n"] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", BOTH, "--stdin", "virtual/main.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(UNCERTAIN).unwrap();
        let out = child.wait_with_output().unwrap();
        let n = usize::from(config.is_empty());
        assert_eq!(out.status.code(), Some(n as i32));
        assert_eq!((count(&out, INTEGER), count(&out, URI)), (n, n));
        if n == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/main.rb:1:1"));
        }
        assert!(!dir.path().join("virtual/main.rb").exists());
    }
    unchanged(
        dir.path(),
        SAFE,
        &["--only", BOTH, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
}
#[test]
fn canonical_bytes_encoding_bom_crlf_comments_and_observer_holds() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(
            b"# bind\r\nFixnum = ::Integer # binding\r\n[::Fixnum] # consume\r\n",
        );
        let mut after = prefix.to_vec();
        after.extend_from_slice(
            b"# bind\r\nFixnum = ::Integer # binding\r\n[::Integer] # consume\r\n",
        );
        unchanged(
            dir.path(),
            &source,
            &["--only", INTEGER, "lib/a.rb"],
            Some(1),
        );
        let out = run(dir.path(), &["--only", INTEGER, "--fix", "lib/a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(dir.path().join("lib/a.rb")).unwrap(), after);
    }
    for observer in [
        "p __LINE__",
        "File.read(__FILE__)",
        "IO.read($0)",
        "RubyVM::InstructionSequence.of(method(:f))",
        "RubyVM::AbstractSyntaxTree.parse_file(__FILE__)",
        "ObjectSpace.each_object {}",
        "GC.stat",
        "binding.local_variables",
        "TracePoint.new(:line) {}.enable",
        "DATA.read\n__END__\ndata\n",
        "\"#{::Fixnum}\"",
        "<<~DOC\n#{::Fixnum}\nDOC\n",
    ] {
        let mut source = SAFE.to_vec();
        source.extend_from_slice(observer.as_bytes());
        let out = unchanged(
            dir.path(),
            &source,
            &["--only", BOTH, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert!(count(&out, INTEGER) >= 1);
    }
    // DATA is observable even without an explicit DATA.read statement.
    unchanged(
        dir.path(),
        b"Fixnum = ::Integer\n[::Fixnum]\n__END__\ndata\n",
        &["--only", INTEGER, "--fix", "lib/a.rb"],
        Some(1),
    );
}
#[cfg(unix)]
#[test]
fn shared_atomic_failure_symlinks_readonly_permissions_invalid_and_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, UNCERTAIN).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for alias in ["lib/a.rb", "lib/link.rb"] {
        assert_eq!(
            run(dir.path(), &["--only", BOTH, "--fix", alias])
                .status
                .code(),
            Some(1)
        );
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap(), old.permissions()),
            (new.ino(), new.modified().unwrap(), new.permissions())
        );
        assert_eq!(fs::read(&path).unwrap(), UNCERTAIN);
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", INTEGER, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(2)
    );
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    unchanged(
        dir.path(),
        b"Fixnum = ::Integer\n[::Fixnum]\n(\n",
        &["--only", BOTH, "--fix", "lib/a.rb"],
        Some(2),
    );
    fs::write(&path, SAFE).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    assert_eq!(
        run(dir.path(), &["--only", INTEGER, "--fix", "lib/link.rb"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(fs::read(&path).unwrap(), AFTER);
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(dir.path().join("lib/link.rb"))
            .unwrap()
            .is_symlink()
    );
    fs::write(&path, b"Fixnum=::Integer;[::Fixnum]\n").unwrap();
    assert_eq!(
        run(
            dir.path(),
            &[
                "--only",
                "Lint/UnifiedInteger,Layout/SpaceAroundOperators",
                "--fix",
                "lib/a.rb"
            ]
        )
        .status
        .code(),
        Some(0)
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"Fixnum = ::Integer;[::Integer]\n"
    );
    fs::write(&path, b"Fixnum\nURI.escape([1,2])\n").unwrap();
    let only = format!("{BOTH},Layout/SpaceAfterComma");
    let out = run(dir.path(), &["--only", &only, "--fix", "lib/a.rb"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!((count(&out, INTEGER), count(&out, URI)), (1, 1));
    assert_eq!(fs::read(&path).unwrap(), b"Fixnum\nURI.escape([1, 2])\n");
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", BOTH, "--fix", "lib/dangling.rb"])
            .status
            .code(),
        Some(2)
    );
    assert!(!dir.path().join("lib/missing.rb").exists());
}
fn native(ruby: &str, source: &str) -> Output {
    Command::new(ruby)
        .args(["--parser=prism", "-w", "-e", source])
        .output()
        .unwrap()
}
#[test]
fn authored_native_alias_identity_binding_warning_value_and_control_flow_witnesses() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    for name in ["Fixnum", "Bignum"] {
        for prefix in ["", "::"] {
            let before = format!("{name} = ::Integer\n[{prefix}{name}]\n");
            unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", INTEGER, "lib/a.rb"],
                Some(1),
            );
            assert_eq!(
                run(dir.path(), &["--only", INTEGER, "--fix", "lib/a.rb"])
                    .status
                    .code(),
                Some(0)
            );
            let after = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
            assert_eq!(after, format!("{name} = ::Integer\n[{prefix}Integer]\n"));
            let a = native(&ruby, &before);
            let b = native(&ruby, &after);
            assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
            assert_eq!(a.stdout, b.stdout);
            assert_eq!(a.stderr, b.stderr);
            assert!(a.stderr.is_empty());
            // Observation harness only: it is authored here, never a reference program.
            let observe = |body: &str| {
                format!(
                    "p(begin\n{body}end); p [Object.const_get(:{name}).equal?(::Integer), Object.const_defined?(:{name}, false), Object.constants(false).include?(:{name}), defined?(::{name}), TOPLEVEL_BINDING.local_variables]\n"
                )
            };
            let a = native(&ruby, &observe(&before));
            let b = native(&ruby, &observe(&after));
            assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
            assert_eq!(a.stdout, b.stdout);
            assert_eq!(a.stderr, b.stderr);
            assert_eq!(
                a.stdout,
                b"[Integer]\n[true, true, true, \"constant\", []]\n"
            );
        }
    }
    for (source, rewritten) in [
        ("p defined?(::Fixnum)", "p defined?(::Integer)"),
        (
            "begin; p ::Bignum; rescue NameError; puts :missing; end",
            "begin; p ::Integer; rescue NameError; puts :missing; end",
        ),
        (
            "Fixnum = String; p ::Fixnum",
            "Fixnum = String; p ::Integer",
        ),
        (
            "Fixnum = ::Integer; Object.private_constant(:Fixnum); begin; p ::Fixnum; rescue NameError; puts :private; end",
            "Fixnum = ::Integer; Object.private_constant(:Fixnum); begin; p ::Integer; rescue NameError; puts :private; end",
        ),
        (
            "Fixnum = ::Integer; Object.send(:remove_const, :Fixnum); begin; p ::Fixnum; rescue NameError; puts :removed; end",
            "Fixnum = ::Integer; Object.send(:remove_const, :Fixnum); begin; p ::Integer; rescue NameError; puts :removed; end",
        ),
        (
            "Fixnum = ::Integer; Object.deprecate_constant(:Fixnum); p ::Fixnum",
            "Fixnum = ::Integer; Object.deprecate_constant(:Fixnum); p ::Integer",
        ),
        (
            "def Object.const_added(name); self.send(:remove_const, name) if name == :Fixnum; end; Fixnum = ::Integer; begin; p ::Fixnum; rescue NameError; puts :callback; end",
            "def Object.const_added(name); self.send(:remove_const, name) if name == :Fixnum; end; Fixnum = ::Integer; begin; p ::Integer; rescue NameError; puts :callback; end",
        ),
    ] {
        let a = native(&ruby, source);
        let b = native(&ruby, rewritten);
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert!(
            a.stdout != b.stdout || a.stderr != b.stderr,
            "counterexample lost: {source}"
        );
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", INTEGER, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert!(count(&out, INTEGER) >= 1);
    }
}
#[test]
fn every_reference_and_corrected_source_is_native_syntax_only() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        for key in ["source", "upstream_after", "project_after"] {
            let mut child = Command::new(&ruby)
                .args(["--parser=prism", "-c", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(row[key].as_str().unwrap().as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{row}\n{key}\n{out:?}");
        }
    }
}
