//! Separately authored bounded Safety programs ONLY. No reference inputs execute.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Lint/TopLevelReturnWithArgument";
const TRACE_SETUP: &str = "$events=[]\ntrace=TracePoint.new(:line,:call,:return,:c_call,:c_return,:raise) do |t|\n  if t.path.end_with?('/safety.rb') || t.path == 'safety.rb'\n    raise 'trace event cap exceeded' if $events.length >= 128\n    $events << [t.event,t.lineno,t.method_id]\n  end\nend\ntrace.enable\n";
fn run(ruby: &str, root: &Path, args: &[&str]) -> Output {
    let memory = if ruby == env!("CARGO_BIN_EXE_rustocop") {
        2 * 1024 * 1024 * 1024
    } else {
        512 * 1024 * 1024
    };
    run_limited(ruby, root, args, "10", memory, 1024 * 1024).unwrap()
}
// Linux acceptance environment: bound the entire subprocess group externally.
// Regular temporary files plus RLIMIT_FSIZE bound both output streams without
// unbounded pipe buffers/readers. Missing guard executables fail, never bypass.
fn run_limited(
    ruby: &str,
    root: &Path,
    args: &[&str],
    seconds: &str,
    memory: u64,
    output: u64,
) -> Result<Output, String> {
    let stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
    let stderr = tempfile::tempfile().map_err(|e| e.to_string())?;
    let status = Command::new("timeout")
        .args(["--kill-after=1s", seconds, "prlimit"])
        .args([
            format!("--as={memory}"),
            format!("--fsize={output}"),
            "--core=0".into(),
            "--".into(),
        ])
        .arg(ruby)
        .args(args)
        .current_dir(root)
        .env("RAYON_NUM_THREADS", "8")
        .stdin(std::process::Stdio::null())
        .stdout(stdout.try_clone().map_err(|e| e.to_string())?)
        .stderr(stderr.try_clone().map_err(|e| e.to_string())?)
        .status()
        .map_err(|e| format!("subprocess guards unavailable: {e}"))?;
    if stdout.metadata().map_err(|e| e.to_string())?.len() >= output
        || stderr.metadata().map_err(|e| e.to_string())?.len() >= output
    {
        return Err("subprocess output limit exceeded".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if status.signal() == Some(25) {
            // SIGXFSZ, even if a write left a short file
            return Err("subprocess output limit exceeded".into());
        }
        if status.signal() == Some(9) {
            return Err("subprocess time limit exceeded".into());
        }
    }
    if matches!(status.code(), Some(124 | 137)) {
        return Err("subprocess time limit exceeded".into());
    }
    fn read_output(mut file: fs::File) -> Result<Vec<u8>, String> {
        use std::io::{Read, Seek};
        file.rewind().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }
    Ok(Output {
        status,
        stdout: read_output(stdout)?,
        stderr: read_output(stderr)?,
    })
}
#[test]
fn subprocess_guards_fail_safely_on_time_memory_and_output_limits() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let time = run_limited(
        &ruby,
        d.path(),
        &["-e", "sleep 2"],
        "0.1",
        128 * 1024 * 1024,
        4096,
    );
    assert!(time.unwrap_err().contains("time limit"));
    for stream in ["STDOUT", "STDERR"] {
        let code = format!("{stream}.write('x' * 65536)");
        let out = run_limited(
            &ruby,
            d.path(),
            &["-e", &code],
            "5",
            128 * 1024 * 1024,
            4096,
        );
        assert!(out.unwrap_err().contains("output limit"));
    }
    let memory = run_limited(
        &ruby,
        d.path(),
        &["-e", "s = 'x' * (256 * 1024 * 1024); puts s.bytesize"],
        "5",
        128 * 1024 * 1024,
        4096,
    )
    .unwrap();
    assert!(
        !memory.status.success(),
        "memory guard did not stop allocation: {memory:?}"
    );
    assert!(
        String::from_utf8_lossy(&memory.stderr).contains("NoMemoryError"),
        "{memory:?}"
    );
}
fn fix(root: &Path) -> Output {
    run(
        env!("CARGO_BIN_EXE_rustocop"),
        root,
        &["--only", COP, "--fix", "safety.rb"],
    )
}
#[test]
fn trace_cap_overflow_fails_instead_of_truncating_observations() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let source = format!(
        "{TRACE_SETUP}{}trace.disable\np $events.length\n",
        "x=1\n".repeat(200)
    );
    fs::write(d.path().join("safety.rb"), source).unwrap();
    let out = run(&ruby, d.path(), &["-W0", "safety.rb"]);
    assert!(!out.status.success(), "silently truncated trace: {out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("trace event cap exceeded"),
        "{out:?}"
    );
}
#[test]
fn live_primitive_cli_withholds_edits_preserving_termination_end_status_and_bindings() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    let path = d.path().join("safety.rb");
    // Independent programs, NOT the pinned fixture. Code after return must never
    // run. END sees exactly the same binding/value/counter and exit status.
    let setup = "x = 17\n$events = []\nEND { p [:finished, x, $events, __LINE__, TOPLEVEL_BINDING.local_variables.sort] }\n";
    for (header, nl, value) in [
        ("", "\n", "1, 2, 3"),
        ("\u{feff}", "\r\n", "1, 2, 3"),
        (
            "# coding: ISO-8859-1\n# ascii safety\n",
            "\n",
            "nil, true, false, -7",
        ),
        ("# coding: UTF8-MAC\n", "\r\n", "2147483647"),
        ("# coding: ASCII-8BIT\n", "\n", "0"),
    ] {
        let original = format!("{header}{setup}return {value} # keep\nraise 'must not run'\n")
            .replace('\n', nl);
        fs::write(&path, &original).unwrap();
        let before = run(&ruby, d.path(), &["--parser=prism", "-W0", "safety.rb"]);
        assert!(before.status.success(), "{before:?}");
        assert!(before.stderr.is_empty());
        let out = fix(d.path());
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let after = run(&ruby, d.path(), &["--parser=prism", "-W0", "safety.rb"]);
        assert_eq!(before.status.code(), after.status.code());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        assert!(String::from_utf8_lossy(&after.stdout).contains("finished"));
        assert_eq!(fix(d.path()).status.code(), Some(1));
        // Keep the original reviewed -W0 equivalence check, without pretending
        // this separately authored candidate is an authorized CLI correction.
        fs::write(
            &path,
            original.replace(&format!("return {value}"), "return"),
        )
        .unwrap();
        let reviewed = run(&ruby, d.path(), &["--parser=prism", "-W0", "safety.rb"]);
        assert_eq!(before.status.code(), reviewed.status.code());
        assert_eq!(before.stdout, reviewed.stdout);
        assert_eq!(before.stderr, reviewed.stderr);
    }
}
#[test]
fn native_warning_boundary_is_explicit_not_silently_claimed_equivalent() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    let p = d.path().join("safety.rb");
    // Actual CLI must retain bytes AND the default/-W1/-w compiler warning.
    // Separately authored unsafe output records why -W0 was insufficient.
    for flags in [vec!["-W0"], vec![], vec!["-W1"], vec!["-w"]] {
        let mut args = flags.clone();
        args.push("safety.rb");
        fs::write(&p, "END { puts :finished }\nreturn 4, 5, 6\n").unwrap();
        let before = run(&ruby, d.path(), &args);
        assert_eq!(fix(d.path()).status.code(), Some(1));
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            "END { puts :finished }\nreturn 4, 5, 6\n"
        );
        let after = run(&ruby, d.path(), &args);
        assert!(before.status.success() && after.status.success());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        // Same filename, so diagnostics differ only because of the unsafe edit.
        fs::write(&p, "END { puts :finished }\nreturn\n").unwrap();
        let unsafe_after = run(&ruby, d.path(), &args);
        assert!(unsafe_after.status.success());
        assert_eq!(before.stdout, unsafe_after.stdout);
        assert!(unsafe_after.stderr.is_empty(), "{unsafe_after:?}");
        if flags == ["-W0"] {
            assert!(before.stderr.is_empty());
        } else {
            assert!(
                String::from_utf8_lossy(&before.stderr)
                    .contains("argument of top-level return is ignored"),
                "{before:?}"
            );
        }
    }
}
#[test]
fn authored_trace_dispatch_exception_load_value_and_heredoc_line_controls() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    fs::write(
        d.path().join("driver.rb"),
        "p [:loaded, load('./safety.rb')]\n",
    )
    .unwrap();
    let setup =
        format!("{TRACE_SETUP}x=<<~DOC\nauthored\nDOC\nEND {{ trace.disable; p [x,$events] }}\n");
    let original = format!("{setup}return 7, 8, 9 # keep\nraise 'not reached'\n");
    let path = d.path().join("safety.rb");
    fs::write(&path, &original).unwrap();
    let before = run(&ruby, d.path(), &["-W0", "driver.rb"]);
    assert_eq!(fix(d.path()).status.code(), Some(1));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    let after = run(&ruby, d.path(), &["-W0", "driver.rb"]);
    assert!(
        before.status.success() && after.status.success(),
        "{before:?}\n{after:?}"
    );
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
    assert!(String::from_utf8_lossy(&after.stdout).contains("[:loaded, true]"));
    fs::write(&path, original.replace("return 7, 8, 9", "return")).unwrap();
    let reviewed = run(&ruby, d.path(), &["-W0", "driver.rb"]);
    assert_eq!(before.status.code(), reviewed.status.code());
    assert_eq!(before.stdout, reviewed.stdout);
    assert_eq!(before.stderr, reviewed.stderr);
}
#[test]
fn effectful_assignments_splats_exceptions_and_interpolations_are_actual_counterexamples() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for (setup, value) in [
        ("def effect; puts :called; 1; end\n", "effect()"),
        ("x=0\nEND { p x }\n", "x = 1"),
        ("def effect; raise 'authored'; end\n", "effect()"),
        ("def effect; puts :interpolated; 1; end\n", "\"#{effect}\""),
        ("o=Object.new\ndef o.to_a; puts :splat; [1]; end\n", "*o"),
    ] {
        let original = format!("{setup}return {value}\n");
        fs::write(d.path().join("safety.rb"), &original).unwrap();
        let before = run(&ruby, d.path(), &["-W0", "safety.rb"]);
        let out = fix(d.path());
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(
            fs::read_to_string(d.path().join("safety.rb")).unwrap(),
            original
        );
        fs::write(d.path().join("unsafe.rb"), format!("{setup}return\n")).unwrap();
        let unsafe_after = run(&ruby, d.path(), &["-W0", "unsafe.rb"]);
        assert_ne!(
            (before.stdout, before.stderr, before.status.code()),
            (
                unsafe_after.stdout,
                unsafe_after.stderr,
                unsafe_after.status.code()
            )
        );
    }
}
#[test]
fn source_code_object_heap_and_data_observations_prevent_edits() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for original in [
        "p File.binread(__FILE__)\nreturn 1, 2, 3\n",
        "p RubyVM::InstructionSequence.compile_file(__FILE__).to_a\nreturn 1, 2, 3\n",
        "p GC.stat(:total_allocated_objects)\nreturn 1, 2, 3\n",
        "p ObjectSpace.count_objects\nreturn 1, 2, 3\n",
        "p DATA.pos\nreturn 1, 2, 3\n__END__\nauthored data\n",
    ] {
        let path = d.path().join("safety.rb");
        fs::write(&path, original).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = fix(d.path());
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        let native = run(&ruby, d.path(), &["-W0", "safety.rb"]);
        assert!(native.status.success(), "{native:?}");
    }
    // Code-object metadata DOES differ even though the ignored primitive return
    // has the same termination. Never use executable equivalence to waive the
    // inherited code-object guard.
    let observed = run(
        &ruby,
        d.path(),
        &[
            "-W0",
            "-e",
            "a=RubyVM::InstructionSequence.compile(\"return 1, 2, 3\\n\"); b=RubyVM::InstructionSequence.compile(\"return\\n\"); raise 'missing observer distinction' if a.to_a == b.to_a; puts :distinct",
        ],
    );
    assert!(observed.status.success(), "{observed:?}");
}
