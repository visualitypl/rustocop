//! authored native proofs and real guarded CLI.
//! RuboCop canonical reference: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::deprecated_constructor_native as native_guard;
use crate::support::stdin as stdin_support;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const BIG: &str = "Lint/BigDecimalNew";
const ERB: &str = "Lint/ErbNewArguments";
const BIG_SAFE: &str = "require 'bigdecimal'\ndef BigDecimal.new(value); ::Kernel.BigDecimal(value); end\n[BigDecimal.new('1.25')]\n";
const BIG_AFTER: &str = "require 'bigdecimal'\ndef BigDecimal.new(value); ::Kernel.BigDecimal(value); end\n[BigDecimal('1.25')]\n";
const ERB_SAFE: &str = "require 'erb'\n[defined?(ERB.new('task-literal', nil, '-'))]\n";
const ERB_AFTER: &str = "require 'erb'\n[defined?(ERB.new('task-literal', trim_mode: '-'))]\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = cli(root, args);
    assert_eq!(out.status.code(), Some(code), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.modified().unwrap(), old.permissions()),
        (new.modified().unwrap(), new.permissions())
    );
    out
}
#[test]
fn exact_actual_team_canonical_sources_and_held_controls_in_real_cli() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/deprecated_constructor_corrections.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 108);
    let dir = project();
    let mut fixes = 0;
    for row in rows {
        let source = row["source"].as_str().unwrap();
        let after = row["project_after"].as_str().unwrap();
        let cop = row["cop"].as_str().unwrap();
        let findings = row["findings"].as_array().unwrap();
        let code = i32::from(!findings.is_empty());
        let out = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "a.rb"],
            code,
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            text.lines()
                .filter(|l| l.ends_with(&format!("({cop})")))
                .count(),
            findings.len(),
            "{row}"
        );
        for f in findings {
            assert!(
                text.contains(&format!(
                    "a.rb:{}:{}: E: {} ({cop})",
                    f[0],
                    f[1],
                    f[3].as_str().unwrap()
                )),
                "{row}\n{out:?}"
            );
        }
        if source == after {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        } else {
            fixes += 1;
            assert_eq!(after, row["upstream_after"].as_str().unwrap());
            let out = cli(dir.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{row}\n{out:?}");
            assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), after.as_bytes());
            unchanged(
                dir.path(),
                after.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
    assert_eq!(fixes, 8);
}
#[test]
fn genuine_edits_preserve_envelopes_and_obey_config_directive_and_selection_gates() {
    let dir = project();
    for (cop, safe, after) in [(BIG, BIG_SAFE, BIG_AFTER), (ERB, ERB_SAFE, ERB_AFTER)] {
        for (prefix, eol) in [
            ("", "\n"),
            ("\u{feff}", "\n"),
            ("# frozen_string_literal: true\r\n", "\r\n"),
            ("# frozen_string_literal: false\n", "\n"),
            ("# coding: ISO-8859-1\r\n# comment\r\n", "\r\n"),
        ] {
            let before = format!("{prefix}{}# retained{eol}", safe.replace('\n', eol));
            let expected = format!("{prefix}{}# retained{eol}", after.replace('\n', eol));
            unchanged(dir.path(), before.as_bytes(), &["--only", cop, "a.rb"], 1);
            let out = cli(dir.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            assert_eq!(
                fs::read(dir.path().join("a.rb")).unwrap(),
                expected.as_bytes()
            );
            unchanged(
                dir.path(),
                expected.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for prefix in [
            b"# coding: ISO-8859-1\r\n# \xe9\r\n".as_slice(),
            b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
        ] {
            let before = [prefix, safe.as_bytes()].concat();
            unchanged(dir.path(), &before, &["--only", cop, "a.rb"], 1);
            let out = cli(dir.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            assert_eq!(
                fs::read(dir.path().join("a.rb")).unwrap(),
                [prefix, after.as_bytes()].concat()
            );
        }
        for config in [
            "exclude = ['a.rb']\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = ['a.rb']\n"),
            "[cops.Lint]\nenabled = false\n".to_owned(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                safe.as_bytes(),
                &["--fix", "a.rb", "--except", "Style,Layout,Naming,Security"],
                0,
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        unchanged(
            dir.path(),
            safe.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for prefix in ["rustocop", "rubocop"] {
            for directive in [
                format!("# {prefix}:disable {cop}"),
                format!("# {prefix}:next -{cop}"),
                format!("# {prefix}:disable-next {cop}"),
                format!("# {prefix}:push -{cop}"),
            ] {
                let source = safe.replace("\n[", &format!("\n{directive}\n["));
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            let source = safe.replace("\n[", &format!("\n# {prefix}:enable-next {cop}\n["));
            let expected = after.replace("\n[", &format!("\n# {prefix}:enable-next {cop}\n["));
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let out = cli(
                dir.path(),
                &["--fix", "--except", "Style,Layout,Naming,Security", "a.rb"],
            );
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            assert_eq!(
                fs::read(dir.path().join("a.rb")).unwrap(),
                expected.as_bytes()
            );
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        }
    }
}
#[cfg(unix)]
#[test]
fn actual_candidate_atomic_readonly_symlink_validation_and_layout_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("a.rb");
    symlink("a.rb", dir.path().join("link.rb")).unwrap();
    for (cop, safe, after) in [(BIG, BIG_SAFE, BIG_AFTER), (ERB, ERB_SAFE, ERB_AFTER)] {
        fs::write(&path, safe).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = cli(dir.path(), &["--only", cop, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), safe.as_bytes());
        let new = fs::metadata(&path).unwrap();
        assert_eq!(
            (old.ino(), old.modified().unwrap(), old.permissions()),
            (new.ino(), new.modified().unwrap(), new.permissions())
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        unchanged(
            dir.path(),
            format!("{safe}(\n").as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            2,
        );
        fs::write(&path, safe).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        let out = cli(dir.path(), &["--only", cop, "--fix", "link.rb", "a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(
            fs::symlink_metadata(dir.path().join("link.rb"))
                .unwrap()
                .is_symlink()
        );
        fs::write(&path, safe.replace("\n[", "  \n[")).unwrap();
        let only = format!("{cop},Layout/TrailingWhitespace");
        let out = cli(dir.path(), &["--only", &only, "--fix", "link.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", &only, "--fix", "link.rb"],
            0,
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
    }
}
#[test]
fn genuine_candidates_stdin_provenance_is_detection_only_and_fix_is_rejected() {
    use std::process::Stdio;
    let dir = project();
    for (cop, safe) in [(BIG, BIG_SAFE), (ERB, ERB_SAFE)] {
        for fix in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command.current_dir(dir.path()).args([
                "--only",
                cop,
                "--stdin",
                "virtual/candidate.rb",
            ]);
            if fix {
                command.arg("--fix");
            }
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            // Rejection may happen before stdin is read: a broken pipe is allowed
            // only for that documented error path, never detection-only input.
            stdin_support::write_stdin(child.stdin.take().unwrap(), safe.as_bytes(), fix).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.status.code(), Some(if fix { 2 } else { 1 }), "{out:?}");
            if fix {
                assert!(out.stdout.is_empty(), "{out:?}");
                assert!(
                    String::from_utf8_lossy(&out.stderr)
                        .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                    "{out:?}"
                );
            } else {
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains("virtual/candidate.rb:"),
                    "{out:?}"
                );
            }
            assert!(!dir.path().join("virtual").exists());
        }
    }
}

#[test]
fn supplemental_reference_sources_get_native_syntax_checks_only() {
    use std::io::Write;
    use std::process::Stdio;
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/deprecated_constructor_corrections.json"
    ))
    .unwrap();
    for row in rows {
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
            assert_eq!(
                out.status.success(),
                key != "upstream_after" || row["upstream_valid_syntax"].as_bool().unwrap(),
                "{key}\n{row}\n{out:?}"
            );
        }
    }
}

fn native(ruby: &str, source: &str) -> Output {
    native_guard::run(ruby, source)
}
fn successful_pair(ruby: &str, before: &str, after: &str) -> Output {
    let a = native(ruby, before);
    let b = native(ruby, after);
    assert!(
        a.status.success() && b.status.success(),
        "{before}\n{a:?}\n{b:?}"
    );
    assert_eq!(a.stdout, b.stdout, "{before}");
    assert_eq!(a.stderr, b.stderr, "{before}");
    assert!(a.stderr.is_empty(), "{a:?}");
    a
}
#[test]
fn authored_native_limits_and_trace_overflow_fail_not_truncate_observations() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let timed = native_guard::limited(&ruby, "sleep 2", "0.2", 512 * 1024 * 1024, 65536);
    assert!(timed.unwrap_err().contains("time limit"));
    for stream in ["STDOUT", "STDERR"] {
        let overflow = native_guard::limited(
            &ruby,
            &format!("loop {{ {stream}.write('x' * 4096) }}"),
            "5",
            512 * 1024 * 1024,
            65536,
        );
        let failure = overflow.unwrap_err();
        assert!(
            failure.contains("output limit") || failure.contains("signal"),
            "{failure}"
        );
    }
    let memory = native_guard::limited(
        &ruby,
        "a = []; loop { a << 'x' * (1024 * 1024) }",
        "5",
        128 * 1024 * 1024,
        65536,
    );
    match memory {
        Ok(out) => {
            assert!(!out.status.success(), "memory limit ignored");
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("memory"),
                "{out:?}"
            );
        }
        Err(error) => assert!(error.contains("signal"), "{error}"),
    }
    let trace = native(
        &ruby,
        "$n = 0; t = TracePoint.new(:c_call) { $n += 1; raise 'trace event cap exceeded' if $n > 128 }; t.enable; 1000.times { 1.to_s }; t.disable; puts :unreachable",
    );
    assert!(!trace.status.success());
    assert!(String::from_utf8_lossy(&trace.stderr).contains("trace event cap exceeded"));
    assert!(trace.stdout.is_empty());
}

#[test]
fn native_real_c_factory_wrapper_value_class_formals_bindings_warnings_and_encodings() {
    if crate::support::runtime::skips("pins Ruby 3.4.1 and the versions of its bundled libraries") {
        return;
    }
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let out = native(
        &ruby,
        "require 'bigdecimal'; abort 'wrong runtime' unless RUBY_VERSION == '3.4.1'; p [BigDecimal::VERSION, BigDecimal.respond_to?(:new), defined?(BigDecimal.new('1.25')), method(:BigDecimal).owner, Kernel.method(:BigDecimal).owner, Kernel.method(:BigDecimal).source_location]",
    );
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "genuine native BigDecimal required: {out:?}"
    );
    assert_eq!(
        out.stdout,
        b"[\"3.1.8\", false, nil, Kernel, #<Class:Kernel>, nil]\n"
    );
    let dir = project();
    for root in ["", "::"] {
        for literal in ["'1.25'", "\"0\"", "'1234567890123456.0000125'"] {
            for prefix in [
                "",
                "# frozen_string_literal: true\n",
                "# frozen_string_literal: false\n",
                "# coding: ISO-8859-1\n",
                "\u{feff}",
            ] {
                let source = format!(
                    "{prefix}require 'bigdecimal'\ndef BigDecimal.new(value); ::Kernel.BigDecimal(value); end\n[{root}BigDecimal.new({literal})]\n"
                );
                fs::write(dir.path().join("a.rb"), &source).unwrap();
                let out = cli(dir.path(), &["--only", BIG, "--fix", "a.rb"]);
                assert_eq!(out.status.code(), Some(0), "{out:?}");
                let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
                assert_eq!(
                    after,
                    source.replace(&format!("[{root}BigDecimal.new("), "[BigDecimal(")
                );
                successful_pair(&ruby, &source, &after);
                // Same authored program with explicit observation of real C values
                // and bindings. The observation harness itself is not fix-eligible.
                let observe = |s: &str| {
                    format!(
                        "{prefix}p [(begin\n{}end).map {{ |v| [v.class.name, v.to_s('F'), v.split, v.frozen?] }}, method(:BigDecimal).owner.name, Kernel.method(:BigDecimal).source_location, BigDecimal.method(:new).parameters, BigDecimal.method(:new).source_location, TOPLEVEL_BINDING.local_variables]\n",
                        s.strip_prefix(prefix).unwrap()
                    )
                };
                let out = successful_pair(&ruby, &observe(&source), &observe(&after));
                assert!(String::from_utf8_lossy(&out.stdout).contains("\"BigDecimal\""));
                assert!(String::from_utf8_lossy(&out.stdout).ends_with(", []]\n"));
                if literal == "'1.25'" {
                    assert!(
                        String::from_utf8_lossy(&out.stdout)
                            .contains("\"1.25\", [1, \"125\", 10, 1]"),
                        "{out:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn native_real_erb_method_reference_result_binding_no_evaluation_or_warnings() {
    if crate::support::runtime::skips("pins Ruby 3.4.1 and the versions of its bundled libraries") {
        return;
    }
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let out = native(
        &ruby,
        "require 'erb'; abort 'wrong runtime' unless RUBY_VERSION == '3.4.1'; p [ERB.respond_to?(:new), ERB.method(:new).owner, ERB.instance_method(:initialize).source_location.last.class]",
    );
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "actual stdlib ERB required: {out:?}"
    );
    assert_eq!(out.stdout, b"[true, Class, Integer]\n");
    let dir = project();
    for root in ["", "::"] {
        for literal in ["'task-literal'", "\"hello <%= 1 %>\"", "''"] {
            for prefix in [
                "",
                "# frozen_string_literal: true\n",
                "# frozen_string_literal: false\n",
                "# coding: ISO-8859-1\n",
                "\u{feff}",
            ] {
                let before = format!(
                    "{prefix}require 'erb'\n[defined?({root}ERB.new({literal}, nil, '-'))]\n"
                );
                fs::write(dir.path().join("a.rb"), &before).unwrap();
                let out = cli(dir.path(), &["--only", ERB, "--fix", "a.rb"]);
                assert_eq!(out.status.code(), Some(0), "{out:?}");
                let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
                assert_eq!(after, before.replace("nil, '-'", "trim_mode: '-'"));
                successful_pair(&ruby, &before, &after);
                let observe = |s: &str| {
                    format!(
                        "{prefix}p [(begin\n{}end), ERB.method(:new).owner.name, TOPLEVEL_BINDING.local_variables]\n",
                        s.strip_prefix(prefix).unwrap()
                    )
                };
                let out = successful_pair(&ruby, &observe(&before), &observe(&after));
                assert_eq!(out.stdout, b"[[\"method\"], \"Class\", []]\n");
            }
        }
    }
    // Instrument the real method without replacing it: defined? never invokes
    // .new, .initialize or even an argument mark, so neither pair warns/executes.
    let prelude = "require 'erb'; $events = []; $trace_count = 0; def mark(x); $events << x; x; end; t = TracePoint.new(:call, :c_call) { |e| $trace_count += 1; raise 'trace event cap exceeded' if $trace_count > 128; $events << e.method_id if [:new, :initialize].include?(e.method_id) }; t.enable; ";
    let a = format!(
        "{prelude}value = [defined?(ERB.new(mark('x'), nil, mark('-')))]; t.disable; p [value, $events]"
    );
    let b = format!(
        "{prelude}value = [defined?(ERB.new(mark('x'), trim_mode: mark('-')))]; t.disable; p [value, $events]"
    );
    let out = successful_pair(&ruby, &a, &b);
    assert_eq!(out.stdout, b"[[\"method\"], []]\n");
}
#[test]
fn native_removed_api_error_backtrace_trace_source_code_heap_and_binding_boundaries() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let dir = project();
    let prelude =
        "require 'bigdecimal'; def BigDecimal.new(value); ::Kernel.BigDecimal(value); end; ";
    let mut pairs = vec![
        ("require 'bigdecimal'; begin; p BigDecimal.new('1.25'); rescue NoMethodError => e; p e.class; end".into(), "require 'bigdecimal'; p BigDecimal('1.25')".into()),
        ("require 'bigdecimal'; p defined?(BigDecimal.new('1.25'))".into(), "require 'bigdecimal'; p defined?(BigDecimal('1.25'))".into()),
    ];
    for suffix in [
        "begin; CALL; rescue => e; p [e.class.name, e.message, e.backtrace]; end",
        "$events = []; $trace_count = 0; t = TracePoint.new(:call, :return, :c_call) { |e| $trace_count += 1; raise 'trace event cap exceeded' if $trace_count > 128; $events << e.method_id if [:new, :BigDecimal].include?(e.method_id) }; t.enable; CALL; t.disable; p $events",
    ] {
        let argument = if suffix.starts_with("begin") {
            "'bad'"
        } else {
            "'1.25'"
        };
        pairs.push((
            format!(
                "{prelude}{}",
                suffix.replace("CALL", &format!("BigDecimal.new({argument})"))
            ),
            format!(
                "{prelude}{}",
                suffix.replace("CALL", &format!("BigDecimal({argument})"))
            ),
        ));
    }
    pairs.push((
        format!("{prelude}def BigDecimal(value); :other_factory; end; p BigDecimal.new('1.25')"),
        format!("{prelude}def BigDecimal(value); :other_factory; end; p BigDecimal('1.25')"),
    ));
    for (before, after) in pairs {
        let a = native(&ruby, &before);
        let b = native(&ruby, &after);
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_ne!(a.stdout, b.stdout, "lost real counterexample: {before}");
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", BIG, "--fix", "a.rb"],
            1,
        );
    }
    // Measure actual heap activity, not a fake constant-returning API. The
    // wrapper and factory each create one real decimal; defined? creates no ERB
    // instance. Heap observers still withhold edits even when this finite
    // observation happens to agree: it is not a general heap-equivalence waiver.
    for (cop, setup, class, before_call, after_call, expected) in [
        (
            BIG,
            prelude,
            "BigDecimal",
            "[BigDecimal.new('1.25')]",
            "[BigDecimal('1.25')]",
            "[1, [\"BigDecimal\"]]\n",
        ),
        (
            ERB,
            "require 'erb'; ",
            "ERB",
            "[defined?(ERB.new('task-literal', nil, '-'))]",
            "[defined?(ERB.new('task-literal', trim_mode: '-'))]",
            "[0, [\"String\"]]\n",
        ),
    ] {
        let harness = |call| {
            format!(
                "{setup}GC.start; GC.disable; n = ObjectSpace.each_object({class}).count; value = {call}; p [ObjectSpace.each_object({class}).count - n, value.map {{ |v| v.class.name }}]"
            )
        };
        let before = harness(before_call);
        let out = successful_pair(&ruby, &before, &harness(after_call));
        assert_eq!(out.stdout, expected.as_bytes());
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
    }
    // These observers must invalidate the otherwise-supported candidate. Source
    // files here are authored controls, never reference or corpus execution.
    for (cop, safe, after) in [(BIG, BIG_SAFE, BIG_AFTER), (ERB, ERB_SAFE, ERB_AFTER)] {
        for observer in [
            "p __LINE__",
            "p __ENCODING__",
            "p binding",
            "GC.stat",
            "ObjectSpace.each_object {}",
            "TracePoint.new(:call) {}.enable",
            "File.read(__FILE__)",
            "IO.read($0)",
            "RubyVM::InstructionSequence.compile_file(__FILE__)",
            "RubyVM::AbstractSyntaxTree.parse_file(__FILE__)",
            "DATA.read\n__END__\npayload\n",
        ] {
            unchanged(
                dir.path(),
                format!("{safe}{observer}\n").as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                1,
            );
        }
        let a = native(
            &ruby,
            &format!("p RubyVM::InstructionSequence.compile({safe:?}).disasm"),
        );
        let b = native(
            &ruby,
            &format!("p RubyVM::InstructionSequence.compile({after:?}).disasm"),
        );
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_ne!(a.stdout, b.stdout);
        // Actual source reads of our controlled files expose the byte edit.
        fs::write(dir.path().join("before.rb"), safe).unwrap();
        fs::write(dir.path().join("after.rb"), after).unwrap();
        let a = native(
            &ruby,
            &format!("p File.binread({:?})", dir.path().join("before.rb")),
        );
        let b = native(
            &ruby,
            &format!("p File.binread({:?})", dir.path().join("after.rb")),
        );
        assert!(a.status.success() && b.status.success());
        assert_ne!(a.stdout, b.stdout);
    }
}
