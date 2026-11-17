//! Source-only RuboCop MIT examples. Never execute fixture inputs.
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/top_level_return_with_argument.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    serde_json::from_value(row[key].clone()).unwrap()
}
#[test]
fn native_syntax_only_for_every_original_input_and_expected_output() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        let mut sources = vec![("bytes", bytes(&row, "bytes"))];
        for key in ["reviewed_after", "upstream_after"] {
            if row.get(key).is_some() {
                sources.push((key, bytes(&row, key)));
            }
        }
        for (key, source) in sources {
            let mut child = Command::new(&ruby)
                .args(["-c", "-W0"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                row.get("parser_rejection").is_none()
                    && !(key == "upstream_after" && row.get("upstream_native_rejection").is_some()),
                "{row}\n{out:?}"
            );
        }
    }
}
