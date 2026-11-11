//! Authored runtime launcher. Reference fixtures never execute.
//! Bound the process group externally; regular output files make RLIMIT_FSIZE
//! effective. Missing guards, timeout, signal death and output overflow fail.
use std::{
    fs::File,
    io::{Read, Seek},
    process::{Command, Output, Stdio},
};

pub fn run(ruby: &str, source: &str) -> Output {
    limited(ruby, source, "10", 512 * 1024 * 1024, 1024 * 1024)
        .expect("bounded authored native probe")
}

pub fn limited(
    ruby: &str,
    source: &str,
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
        .args(["--parser=prism", "-w", "-I/opt/ruby34/lib", "-e", source])
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().map_err(|e| e.to_string())?)
        .stderr(stderr.try_clone().map_err(|e| e.to_string())?)
        .status()
        .map_err(|e| format!("subprocess guards unavailable: {e}"))?;
    if stdout.metadata().map_err(|e| e.to_string())?.len() >= output
        || stderr.metadata().map_err(|e| e.to_string())?.len() >= output
    {
        return Err("subprocess output limit exceeded".into());
    }
    if matches!(status.code(), Some(124 | 137)) {
        return Err("subprocess time limit exceeded".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if status.signal().is_some() {
            return Err(format!("subprocess killed by signal {:?}", status.signal()));
        }
    }
    fn read(mut file: File) -> Result<Vec<u8>, String> {
        file.rewind().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }
    Ok(Output {
        status,
        stdout: read(stdout)?,
        stderr: read(stderr)?,
    })
}
