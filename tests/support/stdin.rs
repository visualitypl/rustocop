//! Small stdin-write guard for CLI fixtures with explicitly expected early errors.
use std::io::{self, Write};

/// Only an explicitly expected early rejection may close the pipe before input
/// is delivered. Callers must still assert the exit code and error diagnostic.
/// Taking the writer by value also closes stdin before waiting for lint output.
pub fn write_stdin(
    mut writer: impl Write,
    source: &[u8],
    expect_early_error: bool,
) -> io::Result<()> {
    match writer.write_all(source) {
        Err(error) if expect_early_error && error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}
