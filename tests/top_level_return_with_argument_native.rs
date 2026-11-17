//! Separately authored bounded Safety programs ONLY. No reference inputs execute.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
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
    let Ok(ruby) = crate::support::runtime::ruby() else {
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
#[test]
fn native_witness_limits_include_both_hard_caps_and_disabled_core_dumps() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    // Finite authored metadata observation; never fault/dump core to test a cap.
    let out = run_limited(
        &ruby,
        d.path(),
        &[
            "-e",
            "p [:AS, :FSIZE, :CORE].map { |limit| Process.getrlimit(limit) }",
        ],
        "5",
        128 * 1024 * 1024,
        4096,
    )
    .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "[[134217728, 134217728], [4096, 4096], [0, 0]]\n"
    );
}
#[test]
fn trace_cap_overflow_fails_instead_of_truncating_observations() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
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
