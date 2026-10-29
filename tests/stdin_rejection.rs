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
