//! Shared scalar mapping through real cop dispatch, output and byte corrections.
use crate::support::cli::project;
use std::{fs, process::Command};

const ONLY: &str =
    "Lint/Debugger,Style/MethodDefParentheses,Style/AmbiguousEndlessMethodDefinition";

fn cli(dir: &std::path::Path, args: &[&str], status: i32) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn dense_valid_cli_positions_exactly_match_short_line_reference_in_all_encodings() {
    const REPEATS: usize = 1000;
    let dir = project();
    let path = dir.path().join("x.rb");
    for (header, text, characters) in [
        (b"".as_slice(), b"ascii".as_slice(), 5),
        (b"\xef\xbb\xbf", "é😀".as_bytes(), 2),
        (b"# coding: UTF-8\r\n", "é😀".as_bytes(), 2),
        (b"# coding: Windows-1252\r\n", b"\xc3\xa9\xff".as_slice(), 3),
        (b"# coding: Shift_JIS\r\n", b"\x82\xa0\xa6".as_slice(), 2),
        (
            b"# coding: EUC-JP\r\n",
            b"\xa4\xa2\x8f\xa2\xaf".as_slice(),
            2,
        ),
        (b"# coding: GB18030\r\n", b"\x81\x30\x81\x30".as_slice(), 1),
    ] {
        let mut fragment = b"\"".to_vec();
        fragment.extend_from_slice(text);
        fragment.extend_from_slice(
            b"\"; binding.pry; def f arg; arg; end; def g = true if condition; ",
        );
        let fragment_chars = fragment.len() - text.len() + characters;
        let header_lines = usize::from(header.ends_with(b"\n"));
        let mut reference = header.to_vec();
        for _ in 0..REPEATS {
            reference.extend_from_slice(&fragment);
            reference.extend_from_slice(b"\r\n");
        }
        fs::write(&path, &reference).unwrap();
        let args = ["--format", "github", "--only", ONLY, "x.rb"];
        let output = cli(dir.path(), &args, 1);
        let mut expected = String::new();
        let mut offenses = 0;
        for line in output.lines() {
            if let Some(rest) = line.strip_prefix("::error file=x.rb,line=") {
                let (row, rest) = rest.split_once(",col=").unwrap();
                let (col, rest) = rest.split_once(",title=").unwrap();
                let row: usize = row.parse().unwrap();
                let col: usize = col.parse().unwrap();
                let dense_row = header_lines + 1;
                let dense_col = (row - dense_row) * fragment_chars + col;
                let (cop, data) = rest.split_once("::").unwrap();
                let message = data.strip_prefix(&format!("x.rb:{row}:{col}: ")).unwrap();
                expected.push_str(&format!("::error file=x.rb,line={dense_row},col={dense_col},title={cop}::x.rb:{dense_row}:{dense_col}: {message}\n"));
                offenses += 1;
            } else {
                expected.push_str(line);
                expected.push('\n');
            }
        }
        assert_eq!(offenses, REPEATS * 3);
        let mut dense = header.to_vec();
        dense.extend(fragment.repeat(REPEATS)); // valid physical line, EOF without LF
        fs::write(&path, &dense).unwrap();
        let actual = cli(dir.path(), &args, 1);
        assert_eq!(actual.lines().count(), expected.lines().count());
        for (actual, expected) in actual.lines().zip(expected.lines()) {
            assert_eq!(actual, expected, "header {header:?}");
        }
        assert_eq!(fs::read(&path).unwrap(), dense);
    }
}

#[test]
fn dense_unicode_corrections_keep_raw_byte_offsets_and_sorted_remaining_positions() {
    const REPEATS: usize = 2000;
    let dir = project();
    let path = dir.path().join("x.rb");
    let fragment = "\"é😀\"; def f arg; arg; end; binding.pry; ";
    let fixed_fragment = "\"é😀\"; def f(arg); arg; end; binding.pry; ";
    let source = format!("\u{feff}{}\r\n", fragment.repeat(REPEATS));
    fs::write(&path, &source).unwrap();
    let output = cli(dir.path(), &["--fix", "--only", ONLY, "x.rb"], 1);
    assert_eq!(
        fs::read(&path).unwrap(),
        format!("\u{feff}{}\n", fixed_fragment.repeat(REPEATS)).as_bytes()
    );
    let mut expected = String::new();
    let first_col = fixed_fragment
        .split_once("binding.pry")
        .unwrap()
        .0
        .chars()
        .count()
        + 1;
    for index in 0..REPEATS {
        let col = first_col + index * fixed_fragment.chars().count();
        expected.push_str(&format!(
            "x.rb:1:{col}: W: Remove debugger entry point `binding.pry`. (Lint/Debugger)\n"
        ));
    }
    expected.push_str(&format!(
        "\n1 file inspected, {REPEATS} corrections applied, {REPEATS} offenses remaining\n"
    ));
    assert_eq!(output, expected);
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let second = cli(dir.path(), &["--fix", "--only", ONLY, "x.rb"], 1);
    assert_eq!(
        second,
        expected.replace(
            &format!("{REPEATS} corrections applied"),
            "0 corrections applied"
        )
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
}
