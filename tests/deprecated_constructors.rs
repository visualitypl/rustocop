//! source-only reference parity, real CLI and authored native controls.
//! Ported RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
#[path = "support/deprecated_constructor_native.rs"]
mod native_guard;
const BIG: &str = "Lint/BigDecimalNew";
const ERB: &str = "Lint/ErbNewArguments";
const BOTH: &str = "Lint/BigDecimalNew,Lint/ErbNewArguments";
const SOURCE: &[u8] = b"BigDecimal.new(1, 2)\nERB.new(str, nil, '-', '@buf')\n";
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino(), "held finding replaced the inode");
    }
    out
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/deprecated_constructors.json")).unwrap()
}
#[test]
fn complete_reference_sources_have_exact_cli_findings_and_no_unproven_edits() {
    let dir = project();
    let rows = rows();
    assert_eq!(rows.len(), 363);
    for row in rows {
        let cop = row["cop"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", cop, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            let out = unchanged(
                dir.path(),
                source.as_bytes(),
                &args,
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
                    "{row}\n{out:?}"
                );
            }
        }
    }
}
#[test]
fn encoded_argument_diagnostics_match_reference_in_real_lint_fix_and_stdin() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/deprecated_constructor_encodings.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 327);
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    }
    for (i, row) in rows.iter().enumerate() {
        let hex = row["source_hex"].as_str().unwrap();
        let source: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let findings = row["findings"].as_array().unwrap();
        let check = |out: &Output, path: &str| {
            assert_eq!(
                out.status.code(),
                Some(i32::from(!findings.is_empty())),
                "{row}\n{out:?}"
            );
            assert_eq!(count(out, ERB), findings.len(), "{row}\n{out:?}");
            let text = std::str::from_utf8(&out.stdout).unwrap();
            assert!(!text.contains('\u{fffd}'), "{out:?}");
            for f in findings {
                assert!(
                    text.contains(&format!(
                        "{path}:{}:{}: E: {} ({ERB})",
                        f[0],
                        f[1],
                        f[3].as_str().unwrap()
                    )),
                    "{row}\n{out:?}"
                );
            }
        };
        for fix in [false, true] {
            let mut args = vec!["--only", ERB, "lib/a.rb"];
            if fix {
                args.push("--fix");
            }
            check(&unchanged(dir.path(), &source, &args, None), "lib/a.rb");
        }
        if i >= rows.len() - 3 {
            let github = unchanged(
                dir.path(),
                &source,
                &["--only", ERB, "--fix", "--format", "github", "lib/a.rb"],
                Some(1),
            );
            let text = std::str::from_utf8(&github.stdout).unwrap();
            for f in findings {
                assert!(text.contains(f[3].as_str().unwrap()), "{github:?}");
            }
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", ERB, "--stdin", "virtual/encoded.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&source).unwrap();
            check(&child.wait_with_output().unwrap(), "virtual/encoded.rb");
            assert!(!dir.path().join("virtual").exists());
        }
        if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
            // Syntax only. Never run the pinned sources or template contents.
            let syntax = Command::new(ruby)
                .args(["--parser=prism", "-c"])
                .arg(&path)
                .output()
                .unwrap();
            assert!(syntax.status.success(), "{row}\n{syntax:?}");
        }
    }
}

#[test]
fn utf8_mac_columns_in_actual_lint_and_fix_preserve_original_bytes() {
    let dir = project();
    for (encoding, big_col, erb_cols) in
        [("UTF8-MAC", 17, [14, 19, 24]), ("UTF-8", 18, [15, 20, 25])]
    {
        for eol in ["\n", "\r\n"] {
            for (root, extra) in [("", 0), ("::", 2)] {
                let source = format!(
                    "# coding: {encoding}{eol}\"e\u{301}\"; {root}BigDecimal.new(1){eol}{root}ERB.new(\"e\u{301}\", nil, \"-\", \"_out\"){eol}"
                );
                for fix in [false, true] {
                    let mut args = vec!["--only", BOTH, "lib/a.rb"];
                    if fix {
                        args.push("--fix");
                    }
                    let out = unchanged(dir.path(), source.as_bytes(), &args, Some(1));
                    assert_eq!((count(&out, BIG), count(&out, ERB)), (1, 3));
                    let text = std::str::from_utf8(&out.stdout).unwrap();
                    assert!(
                        text.contains(&format!("lib/a.rb:2:{}: E:", big_col + extra)),
                        "{out:?}"
                    );
                    for col in erb_cols {
                        assert!(
                            text.contains(&format!("lib/a.rb:3:{}: E:", col + extra)),
                            "{out:?}"
                        );
                    }
                }
                if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
                    // Syntax only: these position fixtures must never execute.
                    let out = Command::new(ruby)
                        .args(["--parser=prism", "-c"])
                        .arg(dir.path().join("lib/a.rb"))
                        .output()
                        .unwrap();
                    assert!(out.status.success(), "{out:?}");
                }
            }
        }
    }
}

#[test]
fn actual_defaults_bounds_selectors_toml_precedence_and_strict_keys() {
    use rustocop::cops::{
        Cop, Registry,
        policy::{RubyVersion, RubyVersions},
    };
    let registry = Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for (name, versions) in [
        (BIG, RubyVersions::default()),
        (
            ERB,
            RubyVersions {
                minimum: Some(RubyVersion(2, 6)),
                maximum: None,
            },
        ),
    ] {
        let cop: &dyn Cop = registry
            .cops()
            .iter()
            .find(|c| c.name() == name)
            .unwrap()
            .as_ref();
        assert!(cop.default_enabled());
        assert!(
            cop.safe_fixes(),
            "only guarded closed canonical capabilities"
        );
        assert_eq!(cop.ruby_versions(), versions);
        assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
        assert!(cop.autocorrect_incompatible_with().is_empty());
    }
    let dir = project();
    for name in [BIG, ERB] {
        let expected = if name == BIG { 1 } else { 3 };
        for (config, selectors, n) in [
            (String::new(), vec![], expected),
            (String::new(), vec!["--only", name], expected),
            (String::new(), vec!["--only", "Lint"], expected),
            (String::new(), vec!["--only", BOTH, "--except", name], 0),
            (String::new(), vec!["--only", BOTH, "--except", "Lint"], 0),
            ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
            (
                format!("[cops.Lint]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
                vec![],
                expected,
            ),
            (
                format!("[cops.Lint]\nenabled = true\n[cops.\"{name}\"]\nenabled = false\n"),
                vec![],
                0,
            ),
            (
                format!("[cops.\"{name}\"]\nenabled = false\n"),
                vec!["--only", name],
                expected,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("lib/a.rb");
            assert_eq!(count(&unchanged(dir.path(), SOURCE, &args, None), name), n);
        }
        for key in [
            "max = 1",
            "include = ['**/*']",
            "Enabled = true",
            "enabled = 'true'",
            "target_ruby = 2.5",
            "SafeAutoCorrect = false",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{name}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(
                dir.path(),
                SOURCE,
                &["--only", name, "--fix", "lib/a.rb"],
                Some(2),
            );
        }
        for config in [
            "exclude = ['lib/**']\n".into(),
            format!("[cops.\"{name}\"]\nexclude = ['lib/**']\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            assert_eq!(
                count(
                    &unchanged(
                        dir.path(),
                        SOURCE,
                        &["--only", name, "--fix", "lib/a.rb"],
                        Some(0)
                    ),
                    name
                ),
                0
            );
        }
    }
}
#[test]
fn both_prefixes_multiline_highlights_next_push_and_source_opt_in() {
    let dir = project();
    for (name, call) in [(BIG, "BigDecimal.new(1)"), (ERB, "ERB.new(str, nil)")] {
        for prefix in ["rustocop", "rubocop"] {
            for source in [
                format!("# {prefix}:disable {name}\n{call}\n"),
                format!("# {prefix}:push -{name}\n{call}\n# {prefix}:pop\n"),
                format!("# {prefix}:disable-next {name}\n{call}\n"),
                format!("# {prefix}:next -{name}\n{call}\n"),
                format!("{call} # {prefix}:disable {name}\n"),
            ] {
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", name, "--fix", "lib/a.rb"],
                    Some(0),
                );
            }
            for (open, close) in [
                (
                    format!("# {prefix}:push +{name}\n"),
                    format!("# {prefix}:pop\n"),
                ),
                (format!("# {prefix}:enable-next {name}\n"), String::new()),
            ] {
                let source =
                    format!("# {prefix}:disable {name}\n{call}\n{open}{call}\n{close}{call}\n");
                let out = unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", name, "--fix", "lib/a.rb"],
                    Some(1),
                );
                assert_eq!(count(&out, name), 1);
            }
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{name}\"]\nenabled = false\n"),
            )
            .unwrap();
            let source = format!("# {prefix}:enable {name}\n{call}\n");
            assert_eq!(
                count(
                    &unchanged(dir.path(), source.as_bytes(), &["lib/a.rb"], None),
                    name
                ),
                1
            );
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            // Only the selector is highlighted for BigDecimal; ERB highlights
            // the entire argument, so a directive on its last line suppresses it.
            let source = format!("ERB.new(str, begin\n nil\nend) # {prefix}:disable {ERB}\n");
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", ERB, "--fix", "lib/a.rb"],
                Some(0),
            );
            let source = format!("BigDecimal.new(\n1) # {prefix}:disable {BIG}\n");
            assert_eq!(
                count(
                    &unchanged(
                        dir.path(),
                        source.as_bytes(),
                        &["--only", BIG, "--fix", "lib/a.rb"],
                        Some(1)
                    ),
                    BIG
                ),
                1
            );
        }
    }
}
#[test]
fn supplied_paths_stdin_fixed_empty_path_policy_and_builtin_exclusions() {
    let dir = project();
    for path in ["lib/a.arb", "lib/a.jb", "Rakefile", "lib/.hidden.rb"] {
        fs::write(dir.path().join(path), SOURCE).unwrap();
        let out = run(dir.path(), &["--only", BOTH, "--fix", path]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!((count(&out, BIG), count(&out, ERB)), (1, 3));
        assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:1:12")));
        assert_eq!(fs::read(dir.path().join(path)).unwrap(), SOURCE);
    }
    for directory in ["vendor", "node_modules"] {
        fs::create_dir(dir.path().join(directory)).unwrap();
        fs::write(dir.path().join(directory).join("a.rb"), SOURCE).unwrap();
        assert_eq!(
            run(dir.path(), &["--only", BOTH, "--fix", directory])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(
            fs::read(dir.path().join(directory).join("a.rb")).unwrap(),
            SOURCE
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
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        let out = child.wait_with_output().unwrap();
        let included = config.is_empty();
        assert_eq!(out.status.code(), Some(i32::from(included)), "{out:?}");
        assert_eq!(
            (count(&out, BIG), count(&out, ERB)),
            if included { (1, 3) } else { (0, 0) }
        );
        if included {
            assert!(String::from_utf8_lossy(&out.stdout).contains("virtual/main.rb:1:12"));
        }
        assert!(!dir.path().join("virtual/main.rb").exists());
    }
    unchanged(
        dir.path(),
        SOURCE,
        &["--only", BOTH, "--stdin", "lib/a.rb", "--fix"],
        Some(2),
    );
}
#[test]
fn bytes_comments_encoding_bom_crlf_heredocs_and_observers_are_not_rewritten() {
    let dir = project();
    for prefix in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"# coding: ISO-8859-1\r\n# \xe9\r\n",
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(b"# keep\r\nBigDecimal.new(1, 2) # constructor\r\nERB.new(str, nil, '-', '@buf') # args\r\n");
        let out = unchanged(
            dir.path(),
            &source,
            &["--only", BOTH, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert_eq!((count(&out, BIG), count(&out, ERB)), (1, 3));
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
        "TracePoint.new(:call) {}.enable",
        "DATA.read\n__END__\npayload\n",
    ] {
        let mut source = SOURCE.to_vec();
        source.extend_from_slice(observer.as_bytes());
        let out = unchanged(
            dir.path(),
            &source,
            &["--only", BOTH, "--fix", "lib/a.rb"],
            Some(1),
        );
        assert_eq!((count(&out, BIG), count(&out, ERB)), (1, 3));
    }
    unchanged(
        dir.path(),
        b"BigDecimal.new(<<~VALUE)\n1\nVALUE\nERB.new(<<~DOC, nil, '-')\ntext\nDOC\n",
        &["--only", BOTH, "--fix", "lib/a.rb"],
        Some(1),
    );
}
#[cfg(unix)]
#[test]
fn existing_atomic_writer_symlinks_permissions_invalid_sources_and_peer_edits() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, SOURCE).unwrap();
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
        assert_eq!(fs::read(&path).unwrap(), SOURCE);
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    unchanged(
        dir.path(),
        b"BigDecimal.new(1)\nERB.new(str,nil)\n(\n",
        &["--only", BOTH, "--fix", "lib/a.rb"],
        Some(2),
    );
    let before = b"BigDecimal.new(1,2)\nERB.new(str,nil)\n";
    let after = b"BigDecimal.new(1, 2)\nERB.new(str, nil)\n";
    fs::write(&path, before).unwrap();
    let only = format!("{BOTH},Layout/SpaceAfterComma");
    let inode = fs::metadata(&path).unwrap().ino();
    let out = run(dir.path(), &["--only", &only, "--fix", "lib/link.rb"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!((count(&out, BIG), count(&out, ERB)), (1, 1));
    assert_eq!(fs::read(&path).unwrap(), after);
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
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(dir.path(), &["--only", &only, "--fix", "lib/link.rb"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), before);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.ino(), old.modified().unwrap(), old.permissions()),
        (new.ino(), new.modified().unwrap(), new.permissions())
    );
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
    native_guard::run(ruby, source)
}
#[test]
fn authored_native_api_binding_warning_order_value_and_error_counterexamples() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    // Only authored controls execute. None of the source/spec fixture programs do.
    let prelude = "require 'erb'; ";
    let before = native(
        &ruby,
        &format!("{prelude}e = ERB.new('hi', nil, '-', '@buf'); p [e.class, e.src, e.encoding]"),
    );
    let after = native(
        &ruby,
        &format!(
            "{prelude}e = ERB.new('hi', trim_mode: '-', eoutvar: '@buf'); p [e.class, e.src, e.encoding]"
        ),
    );
    assert!(
        before.status.success() && after.status.success(),
        "{before:?}\n{after:?}"
    );
    assert_eq!(before.stdout, after.stdout); // same value/class is insufficient
    assert_eq!(
        String::from_utf8_lossy(&before.stderr)
            .matches("deprecated")
            .count(),
        3
    );
    assert!(after.stderr.is_empty());
    for (original, rewritten, expected_before, expected_after) in [
        (
            "ERB.new(mark('str'), mark(nil), mark('-'), mark('@buf'))",
            "ERB.new(mark('str'), trim_mode: mark('-'), eoutvar: mark('@buf'))",
            "[\"str\", nil, \"-\", \"@buf\"]",
            "[\"str\", \"-\", \"@buf\"]",
        ),
        (
            "ERB.new(mark('str'), nil, '-', '@buf', trim_mode: mark('%'), eoutvar: mark('@other'))",
            "ERB.new(mark('str'), trim_mode: '-', eoutvar: '@buf')",
            "[\"str\", \"%\", \"@other\"]",
            "[\"str\"]",
        ),
        (
            "ERB.new(mark('str'), nil, eoutvar: mark('@buf'), trim_mode: mark('-'))",
            "ERB.new(mark('str'), trim_mode: mark('-'), eoutvar: mark('@buf'))",
            "[\"str\", \"@buf\", \"-\"]",
            "[\"str\", \"-\", \"@buf\"]",
        ),
    ] {
        let harness = |call| {
            format!("{prelude}$events = []; def mark(x); $events << x; x; end; {call}; p $events")
        };
        let a = native(&ruby, &harness(original));
        let b = native(&ruby, &harness(rewritten));
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_eq!(String::from_utf8_lossy(&a.stdout).trim(), expected_before);
        assert_eq!(String::from_utf8_lossy(&b.stdout).trim(), expected_after);
    }
    for original in [
        "ERB.new('hi', nil, '-', '@buf', unknown: 1)",
        "ERB.new('hi', nil, '-', '@buf', 1)",
    ] {
        let a = native(
            &ruby,
            &format!("{prelude}begin; {original}; rescue ArgumentError; puts :argument_error; end"),
        );
        let b = native(
            &ruby,
            &format!("{prelude}ERB.new('hi', trim_mode: '-', eoutvar: '@buf'); puts :ok"),
        );
        assert!(a.status.success() && b.status.success());
        assert_eq!(a.stdout, b"argument_error\n");
        assert_eq!(b.stdout, b"ok\n");
    }
    let binding = native(
        &ruby,
        "class BigDecimal; def self.new(*); :constructor; end; end; def BigDecimal(*); :factory; end; p BigDecimal.new(1); p BigDecimal(1)",
    );
    assert!(binding.status.success());
    assert_eq!(binding.stdout, b":constructor\n:factory\n");
    // Record the real native dependency boundary rather than borrowing the host
    // Ruby3.2 C extension or manufacturing an implementation of the API.
    let availability = native(
        &ruby,
        "begin; require 'bigdecimal'; p [BigDecimal.respond_to?(:new), BigDecimal::VERSION]; rescue LoadError => e; warn e.message; puts 'unavailable:bigdecimal'; end",
    );
    assert!(availability.status.success());
    if availability.stdout == b"unavailable:bigdecimal\n" {
        assert!(
            String::from_utf8_lossy(&availability.stderr)
                .contains("cannot load such file -- bigdecimal")
        );
    } else {
        assert!(String::from_utf8_lossy(&availability.stdout).starts_with("["));
    }
}
#[test]
fn all_reference_inputs_are_native_syntax_checked_only() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    for row in rows() {
        for key in ["source", "upstream_after"] {
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
            assert_eq!(
                out.status.success(),
                key == "source" || row["upstream_valid_syntax"].as_bool().unwrap(),
                "{key}\n{row}\n{out:?}"
            );
        }
    }
}
