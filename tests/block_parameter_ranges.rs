//! exact fixed-pin ranges, native syntax and detection-only CLI.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{fs, process::Command};

const COP: &str = "Naming/BlockParameterName";

fn rows() -> Vec<serde_json::Value> {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/block_parameter_ranges.json")).unwrap();
    let rows = fixture["ranges"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 1886);
    rows
}

fn source(row: &serde_json::Value) -> Vec<u8> {
    let hex = row["source_hex"].as_str().unwrap();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn every_reference_case_is_detection_only_through_real_cli_lint_and_repeated_fix() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("a.rb");
    for row in rows() {
        let source = source(&row);
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let expected: Vec<_> = row["offenses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                format!(
                    "a.rb:{}:{}: C: {} ({COP})",
                    o["line"],
                    o["col"],
                    o["message"].as_str().unwrap()
                )
            })
            .collect();
        for fix in [false, true, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command.current_dir(dir.path()).args(["--only", COP]);
            if fix {
                command.arg("--fix");
            }
            let out = command.arg("a.rb").output().unwrap();
            assert!(out.stderr.is_empty(), "{row}: {out:?}");
            assert_eq!(
                out.status.code(),
                Some(i32::from(!expected.is_empty())),
                "{row}: {out:?}"
            );
            let text = String::from_utf8(out.stdout).unwrap();
            let actual: Vec<_> = text
                .lines()
                .filter(|line| line.ends_with(&format!("({COP})")))
                .collect();
            assert_eq!(actual, expected, "{row}");
            if fix {
                assert!(text.contains("0 corrections applied"), "{row}: {text}");
            }
            assert_eq!(fs::read(&path).unwrap(), source, "{row}");
            assert_eq!(
                fs::metadata(&path).unwrap().modified().unwrap(),
                mtime,
                "{row}"
            );
        }
    }
}

#[test]
fn every_reference_case_has_host_and_explicit_native_syntax_only_witnesses() {
    let rows = rows();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("syntax.rb");
    for runtime in
        std::iter::once(std::ffi::OsString::from("ruby")).chain(std::env::var_os("RUSTOCOP_RUBY34"))
    {
        for row in &rows {
            fs::write(&path, source(row)).unwrap();
            let out = match Command::new(&runtime).arg("-c").arg(&path).output() {
                Ok(out) => out,
                Err(error) if runtime == "ruby" && error.kind() == std::io::ErrorKind::NotFound => {
                    break;
                }
                Err(error) => panic!("{runtime:?}: {error}"),
            };
            assert!(out.status.success(), "{runtime:?}: {row}: {out:?}");
            assert!(String::from_utf8_lossy(&out.stdout).contains("Syntax OK"));
        }
    }
}
