//! Encoded parameter diagnostics remain detection-only.
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

const COP: &str = "Naming/MethodParameterName";
const CASE: &str = "Only use lowercase characters for method parameter.";
const LENGTH: &str = "Method parameter must be at least 3 characters long.";

fn check(source: &[u8], expected: &[(u32, u32, &str)]) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    for mode in ["lint", "fix", "fix", "stdin"] {
        let fix = mode == "fix";
        let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
        command.current_dir(dir.path()).args(["--only", COP]);
        if fix {
            command.arg("--fix");
        }
        let out = if mode == "stdin" {
            let mut child = command
                .args(["--stdin", "a.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(source).unwrap();
            child.wait_with_output().unwrap()
        } else {
            command.arg("a.rb").output().unwrap()
        };
        assert!(out.stderr.is_empty(), "{out:?}");
        assert_eq!(
            out.status.code(),
            Some(i32::from(!expected.is_empty())),
            "source {source:?}, mode {mode}: {out:?}"
        );
        let text = String::from_utf8(out.stdout).unwrap();
        let actual: Vec<_> = text
            .lines()
            .filter(|line| line.ends_with(&format!("({COP})")))
            .collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|(line, col, message)| format!("a.rb:{line}:{col}: C: {message} ({COP})"))
            .collect();
        assert_eq!(actual, expected, "source {source:?}");
        if fix {
            assert!(text.contains("0 corrections applied"), "{text}");
        }
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn all_pinned_encoded_commented_and_split_sigil_sources_are_detection_only_through_cli() {
    // RuboCop MIT, Copyright (c) 2012-26 Bozhidar Batsov. The companion
    // Ruby test checks every pinned call, individual exclusion and endpoint.
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/method_parameter_encoding.json")).unwrap();
    let comments: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/method_parameter_comments.json")).unwrap();
    let ranges: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/method_parameter_ranges.json")).unwrap();
    let crlf: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/method_parameter_crlf.json")).unwrap();
    use std::sync::atomic::{AtomicUsize, Ordering};
    let count = AtomicUsize::new(0);
    parallel_rows!(for row in fixture["pinned"]
        .as_array()
        .unwrap()
        .iter()
        .chain(fixture["encoded"].as_array().unwrap())
        .chain(comments["comments"].as_array().unwrap())
        .chain(comments["source"].as_array().unwrap())
        .chain(ranges["ranges"].as_array().unwrap())
        .chain(crlf["crlf"].as_array().unwrap()) => {
        let source = row["source"]
            .as_str()
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_else(|| {
                let hex = row["source_hex"].as_str().unwrap();
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect()
            });
        let expected: Vec<_> = row["offenses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                (
                    o["line"].as_u64().unwrap() as u32,
                    o["col"].as_u64().unwrap() as u32,
                    o["message"].as_str().unwrap(),
                )
            })
            .collect();
        check(&source, &expected);
        count.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(count.load(Ordering::Relaxed), 3819);
}

#[test]
fn fixed_minimum_allowlist_and_numbers_policy_is_unchanged() {
    check(b"def foo(ab, _id, __io, abc1, _); end\n", &[(1, 9, LENGTH)]);
    check(
        b"# encoding: Shift_JIS\ndef foo(a\x82\x81); end\n",
        &[(2, 9, LENGTH)],
    );
    check(
        b"# encoding: EUC-JP\ndef foo(a\xa3\xe1); end\n",
        &[(2, 9, LENGTH)],
    );
}

#[test]
fn encoded_sources_have_host_and_native_syntax_witnesses() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/method_parameter_encoding.json")).unwrap();

    let comments: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/method_parameter_comments.json")).unwrap();
    let ranges: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/method_parameter_ranges.json")).unwrap();
    let crlf: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/method_parameter_crlf.json")).unwrap();
    for runtime in
        std::iter::once(std::ffi::OsString::from("ruby")).chain(crate::support::runtime::ruby_os())
    {
        parallel_rows!(for row in fixture["encoded"]
            .as_array()
            .unwrap()
            .iter()
            .chain(comments["comments"].as_array().unwrap())
            .chain(comments["source"].as_array().unwrap())
            .chain(ranges["ranges"].as_array().unwrap())
            .chain(crlf["crlf"].as_array().unwrap()) => {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("syntax.rb");
            let hex = row["source_hex"].as_str().unwrap();
            let source: Vec<_> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            fs::write(&path, source).unwrap();
            let out = match Command::new(&runtime).arg("-c").arg(&path).output() {
                Ok(out) => out,
                Err(error) if runtime == "ruby" && error.kind() == std::io::ErrorKind::NotFound => {
                    break;
                }
                Err(error) => panic!("{runtime:?}: {error}"),
            };
            assert!(out.status.success(), "{runtime:?}: {row}: {out:?}");
            assert!(String::from_utf8_lossy(&out.stdout).contains("Syntax OK"));
        });
    }
}

#[test]
fn destructured_ranges_do_not_cross_encoded_s_expression_boundaries() {
    for prefix in ["rubocop", "rustocop"] {
        let mut source = b"# encoding: Shift_JIS\ndef foo(((aB,\n".to_vec();
        source.extend_from_slice(b"                          \nabc\x82\x81), second)); end");
        source.extend_from_slice(format!(" # {prefix}:disable {COP}\n").as_bytes());
        check(&source, &[(2, 9, CASE)]);
    }
}

#[test]
fn scoped_disables_and_enables_preserve_encoded_findings_without_writes() {
    for prefix in ["rubocop", "rustocop"] {
        for (encoding, letter) in [
            ("UTF-8", "É".as_bytes()),
            ("Shift_JIS", b"\x82\x60".as_slice()),
        ] {
            let mut source =
                format!("# encoding: {encoding}\n# {prefix}:disable {COP}\ndef foo(abc")
                    .into_bytes();
            source.extend_from_slice(letter);
            source.extend_from_slice(
                format!("); end\n# {prefix}:enable {COP}\ndef foo(abc").as_bytes(),
            );
            source.extend_from_slice(letter);
            source.extend_from_slice(b"); end\n");
            check(&source, &[(5, 9, CASE)]);
        }
    }
}

#[test]
fn uppercase_uses_declared_encoding_not_lossy_utf8() {
    for (encoding, upper, lower) in [
        ("UTF-8", "É".as_bytes(), "é".as_bytes()),
        ("ISO-8859-1", b"\xc9".as_slice(), b"\xe9".as_slice()),
        ("Windows-1252", b"\xc9".as_slice(), b"\xe9".as_slice()),
        ("Shift_JIS", b"\x82\x60".as_slice(), b"\x82\x81".as_slice()),
        (
            "Windows-31J",
            b"\x82\x60".as_slice(),
            b"\x82\x81".as_slice(),
        ),
        ("EUC-JP", b"\xa3\xc1".as_slice(), b"\xa3\xe1".as_slice()),
    ] {
        for (letter, expected) in [(upper, vec![(2, 9, CASE)]), (lower, vec![])] {
            let mut source = format!("# encoding: {encoding}\ndef foo(abc").into_bytes();
            source.extend_from_slice(letter);
            source.extend_from_slice(b"); end\n");
            check(&source, &expected);
        }
    }
}
