//! diagnostic display must not require UTF-8 source bytes.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const ENCODINGS: &[(&str, &[u8])] = &[
    ("ISO-8859-1", b"\xe9"),
    ("Windows-1252", b"\xe9"),
    ("Shift_JIS", b"\x82\xa0"),
];
const CONDITIONAL: &str = "Style/ConditionalAssignment";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn cli(root: &Path, args: &[&str], stdin: &[u8], status: i32) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).expect("all diagnostic output must be printable UTF-8")
}

#[test]
fn unused_legacy_conditional_identifiers_lint_and_fix_without_writing() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (encoding, identifier) in ENCODINGS {
        for newline in ["\n", "\r\n"] {
            let mut source =
                format!("# encoding: {encoding}{newline}if condition{newline}  ").into_bytes();
            source.extend_from_slice(identifier);
            source.extend_from_slice(format!(" = 1{newline}else{newline}  ").as_bytes());
            source.extend_from_slice(identifier);
            source.extend_from_slice(format!(" = 2{newline}end{newline}").as_bytes());
            fs::write(&path, &source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            for args in [
                vec!["--only", CONDITIONAL, "a.rb"],
                vec!["--only", CONDITIONAL, "--fix", "a.rb"],
                vec!["--only", CONDITIONAL, "--stdin", "a.rb"],
                vec!["a.rb"],
            ] {
                let stdout = cli(dir.path(), &args, &source, 1);
                assert!(stdout.contains("a.rb:2:1: C:"), "{stdout}");
                assert!(stdout.contains(CONDITIONAL), "{stdout}");
                if args.contains(&"--fix") {
                    assert!(stdout.contains("0 corrections applied"), "{stdout}");
                }
                assert_eq!(fs::read(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
    }
}

#[test]
fn safe_neighboring_fix_preserves_legacy_bytes_through_validation_and_relint() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (encoding, identifier) in ENCODINGS {
        let mut prefix = format!("# encoding: {encoding}\n").into_bytes();
        prefix.extend_from_slice(identifier);
        prefix.extend_from_slice(b" = 1\n");
        let mut source = prefix.clone();
        source.extend_from_slice(b"fail\n");
        let mut expected = prefix;
        expected.extend_from_slice(b"raise\n");
        fs::write(&path, &source).unwrap();
        let stdout = cli(
            dir.path(),
            &["--only", "Style/SignalException", "--fix", "a.rb"],
            b"",
            0,
        );
        assert!(stdout.contains("1 correction applied"), "{stdout}");
        assert_eq!(fs::read(&path).unwrap(), expected);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let stdout = cli(
            dir.path(),
            &["--only", "Style/SignalException", "--fix", "a.rb"],
            b"",
            0,
        );
        assert!(stdout.contains("0 corrections applied"), "{stdout}");
        assert_eq!(fs::read(&path).unwrap(), expected);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn encoded_syntax_errors_are_printable_for_files_stdin_and_fix_in_both_formats() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (encoding, identifier) in ENCODINGS {
        let mut source = format!("# encoding: {encoding}\ncase value\nin ^").into_bytes();
        source.extend_from_slice(identifier);
        source.extend_from_slice(b"\nend\nfail\n");
        fs::write(&path, &source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        for format in ["text", "github"] {
            for input in ["file", "stdin", "fix"] {
                fs::write(dir.path().join("other.rb"), b"fail\n").unwrap();
                let mut args = vec!["--format", format, "--only", "Style/SignalException"];
                match input {
                    "stdin" => args.extend(["--stdin", "a.rb"]),
                    "fix" => args.extend(["--fix", "a.rb", "other.rb"]),
                    _ => args.push("a.rb"),
                }
                let stdout = cli(dir.path(), &args, &source, 2);
                let message = format!(
                    "syntax error on line 3: {}: no such local variable",
                    String::from_utf8_lossy(identifier)
                );
                assert!(stdout.contains(&message), "{stdout}");
                if format == "github" {
                    assert!(
                        stdout.contains("file=a.rb,line=3,col=5,title=rustocop::"),
                        "{stdout}"
                    );
                }
                assert!(!stdout.contains("Style/SignalException"), "{stdout}");
                assert_eq!(fs::read(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                if input == "fix" {
                    // An encoded syntax error must not abort other files.
                    assert_eq!(fs::read(dir.path().join("other.rb")).unwrap(), b"raise\n");
                }
            }
        }
    }
}
