//! exact source-only reference parity, rebuilt CLI and authored native controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/UriRegexp";
const SAFE: &[u8] = b"require 'uri'\n[defined?(URI.regexp)]\n";
const AFTER: &[u8] = b"require 'uri'\n[defined?(URI::RFC2396_PARSER.make_regexp)]\n";
const CALL: &[u8] = b"URI.regexp(['http'])\n";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/uri_regexp.json")).unwrap()
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
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
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
fn all_reference_rows_match_real_cli_detection_and_guarded_canonical_fix() {
    let dir = project();
    let mut fixed = 0;
    assert_eq!(rows().len(), 295);
    for row in rows() {
        let source = row["source"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        let code = i32::from(!findings.is_empty());
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "lib/a.rb"],
            Some(code),
        );
        assert_eq!(count(&out), findings.len(), "{row}\n{out:?}");
        for f in findings {
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!(
                    "lib/a.rb:{}:{}: E: {} ({COP})",
                    f[0],
                    f[1],
                    f[3].as_str().unwrap()
                )),
                "{row}\n{out:?}"
            );
        }
        let after = row["project_after"].as_str().unwrap();
        if after == source {
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(code),
            );
            assert_eq!(count(&out), findings.len());
        } else {
            fixed += 1;
            let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{row}\n{out:?}");
            assert_eq!(
                fs::read(dir.path().join("lib/a.rb")).unwrap(),
                after.as_bytes()
            );
            unchanged(
                dir.path(),
                after.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
    }
    assert_eq!(fixed, 8);
}
#[test]
fn defaults_metadata_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 491);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    let dir = project();
    for (config, selectors, n) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint"], 1),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            1,
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(dir.path(), CALL, &args, None)), n);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 3.3",
        "SafeAutoCorrect = false",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SAFE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    for config in [
        "exclude = ['lib/**']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        assert_eq!(
            count(&unchanged(
                dir.path(),
                SAFE,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0)
            )),
            0
        );
    }
}
#[test]
fn both_directive_prefixes_next_push_enable_and_selector_multiline_scopes() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\nURI.regexp\n"),
            format!("# {prefix}:push -{COP}\nURI.regexp\n# {prefix}:pop\n"),
            format!("# {prefix}:disable-next {COP}\nURI.regexp\n"),
            format!("# {prefix}:next -{COP}\nURI.regexp\n"),
            format!("URI.regexp # {prefix}:disable {COP}\n"),
            format!("URI.\n regexp( # {prefix}:disable {COP}\n 'http'\n)\n"),
            format!("require 'uri'\n[defined?(URI.regexp)] # {prefix}:disable {COP}\n"),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
            );
        }
        // The selector is the ENTIRE offense range, not the multi-line call.
        let source = format!("URI.regexp(\n 'http' # {prefix}:disable {COP}\n)\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1)
            )),
            1
        );
        for open in [
            format!("# {prefix}:push +{COP}"),
            format!("# {prefix}:enable-next {COP}"),
        ] {
            let source = format!(
                "# {prefix}:disable {COP}\nURI.regexp\n{open}\nURI.regexp\n# {prefix}:pop\nURI.regexp\n"
            );
            assert_eq!(
                count(&unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    Some(1)
                )),
                1
            );
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\nURI.regexp\n");
        assert_eq!(
            count(&unchanged(
                dir.path(),
                source.as_bytes(),
                &["lib/a.rb"],
                None
            )),
            1
        );
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn supplied_paths_stdin_provenance_empty_fixed_path_policy_and_no_stdin_fix() {
    let dir = project();
    for path in ["lib/a.arb", "lib/a.jb", "Rakefile", "lib/.hidden.rb"] {
        fs::write(dir.path().join(path), CALL).unwrap();
        let out = run(dir.path(), &["--only", COP, "--fix", path]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(count(&out), 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:1:5")));
        assert_eq!(fs::read(dir.path().join(path)).unwrap(), CALL);
    }
    for directory in ["vendor", "node_modules", "db"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        let file = if directory == "db" {
            "schema.rb"
        } else {
            "a.rb"
        };
        let path = dir.path().join(directory).join(file);
        fs::write(&path, SAFE).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", directory])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(fs::read(path).unwrap(), SAFE);
    }
    for config in ["", "exclude = ['virtual/**']\n"] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "virtual/main.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(CALL).unwrap();
        let out = child.wait_with_output().unwrap();
        let n = usize::from(config.is_empty());
        assert_eq!(out.status.code(), Some(n as i32));
        assert_eq!(count(&out), n);
        if n == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/main.rb:1:5"));
        }
        assert!(!dir.path().join("virtual/main.rb").exists());
    }
    unchanged(
        dir.path(),
        SAFE,
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
}
#[test]
fn canonical_encoding_bom_crlf_comments_and_source_runtime_observer_holds() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(
            b"# bind\r\nrequire 'uri' # load\r\n[defined?(URI.regexp())] # consume\r\n",
        );
        let mut after = prefix.to_vec();
        after.extend_from_slice(b"# bind\r\nrequire 'uri' # load\r\n[defined?(URI::RFC2396_PARSER.make_regexp)] # consume\r\n");
        unchanged(dir.path(), &source, &["--only", COP, "lib/a.rb"], Some(1));
        let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
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
        "__END__\ndata\n",
        "DATA.read\n__END__\ndata\n",
        "eval('URI = Other')",
        "<<~DOC\n#{URI.regexp}\nDOC\n",
    ] {
        let mut source = SAFE.to_vec();
        source.extend_from_slice(observer.as_bytes());
        let out = unchanged(
            dir.path(),
            &source,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert!(count(&out) >= 1);
    }
    for source in [
        "require 'uri'\n[defined?(URI.regexp( # keep\n))]\n",
        "require 'uri'\n[defined?(URI.regexp(<<~DOC))]\nhttp\nDOC\n",
    ] {
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
}
#[cfg(unix)]
#[test]
fn shared_atomic_validation_failure_readonly_symlinks_permissions_and_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, CALL).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", dir.path().join("lib/link.rb")).unwrap();
    let old = fs::metadata(&path).unwrap();
    for alias in ["lib/a.rb", "lib/link.rb"] {
        assert_eq!(
            run(dir.path(), &["--only", COP, "--fix", alias])
                .status
                .code(),
            Some(1)
        );
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap(), old.permissions()),
            (new.ino(), new.modified().unwrap(), new.permissions())
        );
        assert_eq!(fs::read(&path).unwrap(), CALL);
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/link.rb"])
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
        b"require 'uri'\n[defined?(URI.regexp)]\n(\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(2),
    );
    fs::write(&path, SAFE).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/link.rb"])
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
    fs::write(&path, b"require 'uri'; [defined?(URI.regexp)]\n").unwrap();
    assert_eq!(
        run(
            dir.path(),
            &[
                "--only",
                "Lint/UriRegexp,Style/StringLiterals",
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
        b"require \"uri\"; [defined?(URI::RFC2396_PARSER.make_regexp)]\n"
    );
    fs::write(&path, b"URI.regexp(['http','https'])\n").unwrap();
    let out = run(
        dir.path(),
        &[
            "--only",
            "Lint/UriRegexp,Layout/SpaceAfterComma",
            "--fix",
            "lib/a.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out), 1);
    assert_eq!(fs::read(&path).unwrap(), b"URI.regexp(['http', 'https'])\n");
    assert_eq!(fs::read_dir(dir.path().join("lib")).unwrap().count(), 2);
    symlink("missing.rb", dir.path().join("lib/dangling.rb")).unwrap();
    assert_eq!(
        run(dir.path(), &["--only", COP, "--fix", "lib/dangling.rb"])
            .status
            .code(),
        Some(2)
    );
    assert!(!dir.path().join("lib/missing.rb").exists());
}
fn native(ruby: &str, source: &str) -> Output {
    Command::new(ruby)
        .arg("--parser=prism")
        .arg("-w")
        .arg(format!(
            "-I{}",
            Path::new(ruby).parent().unwrap().join("lib").display()
        ))
        .args(["-e", source])
        .output()
        .unwrap()
}
#[test]
fn authored_native_stdlib_binding_method_availability_warning_value_and_evaluation_proofs() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let dir = project();
    for receiver in ["URI", "::URI"] {
        for suffix in ["", "()"] {
            let before = format!("require 'uri'\n[defined?({receiver}.regexp{suffix})]\n");
            let expected =
                format!("require 'uri'\n[defined?({receiver}::RFC2396_PARSER.make_regexp)]\n");
            unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", COP, "lib/a.rb"],
                Some(1),
            );
            assert_eq!(
                run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"])
                    .status
                    .code(),
                Some(0)
            );
            let after = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
            assert_eq!(after, expected);
            for (a, b) in [
                (before.clone(), after.clone()),
                (
                    format!(
                        "p(begin\n{before}end)\np [URI.respond_to?(:regexp), URI::RFC2396_PARSER.respond_to?(:make_regexp), URI::RFC2396_PARSER.class.name, URI::RFC2396_PARSER.frozen?, TOPLEVEL_BINDING.local_variables]\n"
                    ),
                    format!(
                        "p(begin\n{after}end)\np [URI.respond_to?(:regexp), URI::RFC2396_PARSER.respond_to?(:make_regexp), URI::RFC2396_PARSER.class.name, URI::RFC2396_PARSER.frozen?, TOPLEVEL_BINDING.local_variables]\n"
                    ),
                ),
            ] {
                let a = native(&ruby, &a);
                let b = native(&ruby, &b);
                assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
                assert_eq!(a.stdout, b.stdout);
                assert_eq!(a.stderr, b.stderr);
                assert!(a.stderr.is_empty(), "{a:?}");
                if !a.stdout.is_empty() {
                    assert_eq!(
                        a.stdout,
                        b"[\"method\"]\n[true, true, \"URI::RFC2396_Parser\", true, []]\n"
                    );
                }
            }
        }
    }
    // Authored instrumentation, not an admitted correction context: neither
    // native method is invoked by either method-reference expression.
    let out = native(
        &ruby,
        "require 'uri'; tp = TracePoint.new(:call) { |t| raise 'invoked' if [:regexp, :make_regexp].include?(t.method_id) }; tp.enable; p [defined?(URI.regexp), defined?(URI::RFC2396_PARSER.make_regexp)]; tp.disable",
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"[\"method\", \"method\"]\n");
    assert!(out.stderr.is_empty());
    // The actual native APIs exist, but ordinary invocation is NOT warning-equivalent.
    for args in ["", "nil", "['http', 'https']", "'http://example.com'"] {
        let before = format!(
            "require 'uri'\nr = URI.regexp({args}); p [r.source, r.options, r.encoding.name, r.match?('http://example.com')]\n"
        );
        let after = before.replace("URI.regexp", "URI::RFC2396_PARSER.make_regexp");
        let a = native(&ruby, &before);
        let b = native(&ruby, &after);
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(
            String::from_utf8_lossy(&a.stderr)
                .matches("obsolete")
                .count(),
            2
        );
        assert!(b.stderr.is_empty());
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    for (before, after) in [
        (
            "URI.regexp(nil, (puts :evaluated))",
            "URI::RFC2396_PARSER.make_regexp(nil)",
        ),
        (
            "URI.regexp(nil, &((puts :block); proc {}))",
            "URI::RFC2396_PARSER.make_regexp(nil)",
        ),
        (
            "URI.send(:remove_const, :RFC2396_PARSER); p defined?(URI.regexp)",
            "URI.send(:remove_const, :RFC2396_PARSER); p defined?(URI::RFC2396_PARSER.make_regexp)",
        ),
        (
            "URI.singleton_class.remove_method(:regexp); p defined?(URI.regexp)",
            "URI.singleton_class.remove_method(:regexp); p defined?(URI::RFC2396_PARSER.make_regexp)",
        ),
        (
            "module M; module URI; def self.regexp; end; end; p defined?(URI.regexp); end",
            "module M; module URI; def self.regexp; end; end; p defined?(URI::RFC2396_PARSER.make_regexp); end",
        ),
    ] {
        let before = format!("require 'uri'\nbegin; {before}; rescue => e; puts e.class; end\n");
        let after = format!("require 'uri'\nbegin; {after}; rescue => e; puts e.class; end\n");
        let a = native(&ruby, &before);
        let b = native(&ruby, &after);
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert!(
            a.stdout != b.stdout || a.stderr != b.stderr,
            "counterexample lost: {before}"
        );
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
        );
    }
    // No URI loaded: both references are nil, not proof of actual method availability.
    let out = native(
        &ruby,
        "p [defined?(URI.regexp), defined?(URI::RFC2396_PARSER.make_regexp)]",
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"[nil, nil]\n");
}
#[test]
fn every_reference_input_and_upstream_rewrite_is_native_syntax_only() {
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
            let valid = key != "upstream_after" || row["upstream_valid_syntax"].as_bool().unwrap();
            assert_eq!(out.status.success(), valid, "{row}\n{key}\n{out:?}");
        }
    }
}
