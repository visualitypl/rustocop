//! Preserve original unknown-#source runtime fixtures as no-write controls,
//! then exercise canonical edits with a pattern-only #to_s observer.
//! deliberately does not infer receiver types through locals/parameters, even
//! in test instrumentation: they may hold a pre-entry retained AST in real code.
use std::{fs, path::Path, process::Command};

pub fn pattern_sibling(dir: &Path, file: &str, source: &str, cops: &str) -> String {
    assert!(source.contains(".source"));
    let path = dir.join(file);
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir)
            .args(["--only", cops, "--fix", file])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{cops}: {output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("0 corrections applied"));
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source}");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
    // Regexp#to_s still exposes the decoded pattern and inline options; the
    // paired runtime executions compare it, encoding, flags and match behavior.
    source.replace(".source", ".to_s")
}
