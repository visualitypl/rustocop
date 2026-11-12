//! Test-only stdin protocol controls; no sleeps or scheduler-dependent pipe races.
#[path = "support/stdin.rs"]
mod stdin;

use std::{
    fs,
    io::{self, ErrorKind, Write},
    process::{Command, Stdio},
};

struct ErrorWriter(ErrorKind);
impl Write for ErrorWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(self.0.into())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn successful_writes_deliver_all_bytes_in_both_modes() {
    for early_error in [false, true] {
        let mut bytes = Vec::new();
        stdin::write_stdin(&mut bytes, b"binding.pry\n", early_error).unwrap();
        assert_eq!(bytes, b"binding.pry\n");
    }
}

#[test]
fn broken_pipe_is_allowed_only_for_an_expected_early_error() {
    assert_eq!(
        stdin::write_stdin(ErrorWriter(ErrorKind::BrokenPipe), b"input", false)
            .unwrap_err()
            .kind(),
        ErrorKind::BrokenPipe
    );
    stdin::write_stdin(ErrorWriter(ErrorKind::BrokenPipe), b"input", true).unwrap();
}

#[test]
fn other_write_errors_are_never_allowed() {
    for early_error in [false, true] {
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::WriteZero,
            ErrorKind::Other,
        ] {
            assert_eq!(
                stdin::write_stdin(ErrorWriter(kind), b"input", early_error)
                    .unwrap_err()
                    .kind(),
                kind
            );
        }
    }
}

#[test]
fn fix_stdin_rejection_closes_before_write_without_touching_display_path() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    let path = dir.path().join("x.rb");
    for exists in [false, true] {
        let mtime = if exists {
            fs::write(&path, b"disk bytes\r\n").unwrap();
            Some(fs::metadata(&path).unwrap().modified().unwrap())
        } else {
            None
        };
        for args in [["--fix", "--stdin", "x.rb"], ["--stdin", "x.rb", "--fix"]] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut pipe = child.stdin.take().unwrap();
            // Taking stdin first keeps our writer open across wait_with_output.
            // Rejection must finish without reading stdin; exit guarantees no reader.
            let output = child.wait_with_output().unwrap();
            assert_eq!(output.status.code(), Some(2), "{output:?}");
            assert!(output.stdout.is_empty(), "{output:?}");
            let diagnostic = if args[0] == "--fix" {
                "the argument '--fix' cannot be used with '--stdin <PATH>'"
            } else {
                "the argument '--stdin <PATH>' cannot be used with '--fix'"
            };
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(diagnostic),
                "{output:?}"
            );
            assert_eq!(
                pipe.write_all(b"input").unwrap_err().kind(),
                ErrorKind::BrokenPipe
            );
            stdin::write_stdin(pipe, b"binding.pry\n", true).unwrap();
            if let Some(mtime) = mtime {
                assert_eq!(fs::read(&path).unwrap(), b"disk bytes\r\n");
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            } else {
                assert!(!path.exists());
            }
        }
    }
}

/// Larger than any Linux pipe buffer (the unprivileged maximum is 1 MiB), so a
/// writer to a child that never reads must block until the child exits and then
/// hit a broken pipe: the scheduling race is forced the same way on every run.
const LARGE: usize = 8 << 20;

fn large_source() -> Vec<u8> {
    b"# padding comment line\n".repeat(LARGE / 23 + 1)
}

fn spawn_stdin_run(dir: &std::path::Path, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

#[test]
fn large_input_to_an_early_rejection_breaks_the_pipe_on_every_repetition() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    let path = dir.path().join("x.rb");
    fs::write(&path, b"disk bytes\n").unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let source = large_source();
    let rejections: [(&[&str], &str); 3] = [
        (
            &["--fix", "--stdin", "x.rb"],
            "the argument '--fix' cannot be used with '--stdin <PATH>'",
        ),
        (
            &["--stdin", "x.rb", "--fix"],
            "the argument '--stdin <PATH>' cannot be used with '--fix'",
        ),
        (
            &["--only", "Lint/NoSuchCop", "--stdin", "x.rb"],
            "unknown cop or department `Lint/NoSuchCop` in --only",
        ),
    ];
    for (args, diagnostic) in rejections {
        for repetition in 0..3 {
            for expect_early_error in [false, true] {
                let mut child = spawn_stdin_run(dir.path(), args);
                let written =
                    stdin::write_stdin(child.stdin.take().unwrap(), &source, expect_early_error);
                let output = child.wait_with_output().unwrap();
                if expect_early_error {
                    written.unwrap();
                } else {
                    // Unflagged writes keep failing loudly on the same broken pipe.
                    assert_eq!(
                        written.unwrap_err().kind(),
                        ErrorKind::BrokenPipe,
                        "{args:?} #{repetition}"
                    );
                }
                assert_eq!(output.status.code(), Some(2), "{args:?} {output:?}");
                assert!(output.stdout.is_empty(), "{args:?} {output:?}");
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains(diagnostic),
                    "{args:?} {output:?}"
                );
                assert_eq!(fs::read(&path).unwrap(), b"disk bytes\n");
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
        }
    }
}

#[test]
fn large_input_to_a_valid_stdin_run_is_read_completely_in_both_modes() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    let mut source = large_source();
    source.extend_from_slice(b"binding.pry\n");
    let line = source.iter().filter(|&&b| b == b'\n').count();
    for expect_early_error in [false, true] {
        let mut child =
            spawn_stdin_run(dir.path(), &["--only", "Lint/Debugger", "--stdin", "x.rb"]);
        stdin::write_stdin(child.stdin.take().unwrap(), &source, expect_early_error).unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        // The offense on the final line proves every byte was delivered and read.
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(&format!("x.rb:{line}:1:")),
            "{output:?}"
        );
        assert!(!dir.path().join("x.rb").exists());
    }
}
