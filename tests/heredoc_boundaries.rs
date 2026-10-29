//! controlled strings only; command heredocs are never executed.
use std::{fs, path::Path, process::Command};

const LAYOUT: &str = "Layout/SpaceAroundOperators";
const PEERS: &str = "Layout/SpaceAroundOperators,Style/VariableInterpolation,Style/SymbolLiteral,Style/QuotedSymbols,Style/StringLiterals";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn fix(dir: &Path, selected: Option<&str>) -> String {
    fix_status(dir, selected, 1)
}
fn fix_status(dir: &Path, selected: Option<&str>, code: i32) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    command.current_dir(dir).args(["--fix", "a.rb"]);
    if let Some(selected) = selected {
        command.args(["--only", selected]);
    } else {
        // Keep this v0.4 no-write boundary witness's labels fixed. The heredoc delimiter cops' live
        // default rename+Layout composition has its own native regression.
        command.args(["--except", "Naming/HeredocDelimiterCase"]);
    }
    let out = command.output().unwrap();
    assert_eq!(out.status.code(), Some(code), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}
fn runtime(path: &Path) -> Option<Vec<u8>> {
    if Command::new("ruby").arg("--version").output().is_err() {
        return None;
    }
    let out = Command::new("ruby")
        .arg("--disable-gems")
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    Some(out.stdout)
}

#[test]
fn public_runner_withholds_valid_but_value_changing_opening_boundary_edits() {
    use rustocop::{
        cops::{Cop, NodeContext, run_cops},
        offense::{Correction, Offense},
        source::SourceFile,
    };
    struct SuffixProbe {
        stage: u8,
        edit: Correction,
    }
    impl SuffixProbe {
        fn emit(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            let (line, col) = file.line_col(self.edit.range.start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "name extension".into(),
                line,
                col,
                end_line: line,
                correction: Some(self.edit.clone()),
            });
        }
    }
    impl Cop for SuffixProbe {
        fn name(&self) -> &'static str {
            "Style/SuffixProbe"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            if self.stage == 0 {
                self.emit(file, out);
            }
        }
        fn check_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            _: &NodeContext<'_, 'pr>,
            file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if self.stage == 1 && node.as_program_node().is_some() {
                self.emit(file, out);
            }
        }
        fn wants_leave_node(&self) -> bool {
            self.stage == 2
        }
        fn leave_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            _: &NodeContext<'_, 'pr>,
            file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if node.as_program_node().is_some() {
                self.emit(file, out);
            }
        }
    }
    let dir = project();
    let path = dir.path().join("a.rb");
    for (source, replacement, removed, leading) in [
        ("A = 1\np <<A\ntext\nAé\nA\n", "é", 0, false),
        ("A = 1\né = \"\"\np <<A +é\ntext\nAé\nA\n", "", 2, false),
        (
            "def p(x = [])\n  Kernel.p(x)\n  []\nend\nA = 1\np <<A\n1\nA\n",
            "",
            1,
            true,
        ),
    ] {
        let end = source.find("<<A").unwrap();
        let end = if leading { end - 1 } else { end + 3 };
        let edit = Correction {
            range: end..end + removed,
            replacement: replacement.into(),
        };
        let corrected = format!(
            "{}{}{}",
            &source[..end],
            replacement,
            &source[end + removed..]
        );
        assert_eq!(ruby_prism::parse(corrected.as_bytes()).errors().count(), 0);
        fs::write(&path, &corrected).unwrap();
        let changed = runtime(&path);
        if let Some(changed) = &changed {
            assert_eq!(
                changed,
                if leading {
                    &b"[]\n"[..]
                } else {
                    &b"\"text\\n\"\n"[..]
                }
            );
        }
        fs::write(&path, source).unwrap();
        let original = runtime(&path);
        if let Some(original) = &original {
            assert_eq!(
                original,
                if leading {
                    &b"\"1\\n\"\n"[..]
                } else {
                    "\"text\\nAé\\n\"\n".as_bytes()
                }
            );
        }
        for stage in 0..3 {
            let file = SourceFile::new("a.rb".into(), source.as_bytes().to_vec());
            let parsed = ruby_prism::parse(&file.bytes);
            let mut found = Vec::new();
            assert!(
                run_cops(
                    &[&SuffixProbe {
                        stage,
                        edit: edit.clone()
                    }],
                    &parsed,
                    &file,
                    &mut found
                )
                .is_empty()
            );
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].message, "name extension");
            assert!(found[0].correction.is_none());
            let applied = rustocop::corrections::apply(&file.bytes, &found).unwrap();
            assert_eq!(applied.count, 0);
            assert_eq!(applied.bytes, source.as_bytes());
        }
        assert_eq!(runtime(&path), original);
    }
}

#[test]
fn layout_body_to_terminator_preserves_bytes_mtime_and_runtime() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (op, indent) in [("<<", ""), ("<<-", "  "), ("<<~", "\t")] {
        for newline in ["\n", "\r\n"] {
            let body_indent = if op == "<<~" { indent } else { "" };
            let source = format!(
                "a = 1\nb = 2\np {op}\"#{{a + b}}\"\n{body_indent}#{{a+b}}\n{indent}#{{a + b}}\n"
            )
            .replace('\n', newline);
            for selected in [Some(LAYOUT), Some(PEERS), None] {
                fs::write(&path, &source).unwrap();
                let before = runtime(&path);
                if let Some(before) = &before {
                    assert_eq!(before, b"\"3\\n\"\n");
                }
                let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                let report = fix(dir.path(), selected);
                assert!(report.contains(LAYOUT), "{report}");
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source}");
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                assert_eq!(runtime(&path), before);
            }
        }
    }
}

fn assert_inline_indentation_preserves_boundary(prefix: &str, body: &str, label: &str, cop: &str) {
    let dir = project();
    let path = dir.path().join("a.rb");
    for op in ["<<-", "<<~"] {
        for (indent, escaped) in [("\x0b", "\\v"), ("\x0c", "\\f"), ("\r", "\\r")] {
            for newline in ["\n", "\r\n"] {
                let source = format!("{prefix}p {op}\"{label}\"\n{indent}{body}\n{label}\n")
                    .replace('\n', newline);
                for selected in [Some(cop), Some(PEERS), None] {
                    fs::write(&path, &source).unwrap();
                    let before = runtime(&path);
                    if let Some(before) = &before {
                        assert_eq!(before, format!("\"{escaped}3\\n\"\n").as_bytes());
                    }
                    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                    let report = fix(dir.path(), selected);
                    assert!(report.contains(cop), "{report}");
                    assert_eq!(
                        fs::read(&path).unwrap(),
                        source.as_bytes(),
                        "{op} {indent:?} {newline:?} {selected:?}: {report}"
                    );
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                    assert_eq!(runtime(&path), before);
                }
            }
        }
    }
}

#[test]
fn layout_inline_indentation_cannot_witness_a_non_terminator() {
    assert_inline_indentation_preserves_boundary("", "#{1+2}", "#{1 + 2}", LAYOUT);
}

#[test]
fn ast_inline_indentation_cannot_witness_a_non_terminator() {
    assert_inline_indentation_preserves_boundary(
        "@x = 3\n",
        "#@x",
        "#{@x}",
        "Style/VariableInterpolation",
    );
}

#[test]
fn ast_and_layout_adjacent_composition_does_not_promote_body_lines() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (body, label) in [
        ("#{1+2}", "#{1 + 2}"),
        ("#{:\"word\"}#{1+2}", "#{:word}#{1 + 2}"),
        ("#@x#{1+2}", "#{@x}#{1 + 2}"),
    ] {
        let source = format!("@x = 4\np <<~\"{label}\"\n{body}\n{label}\n");
        fs::write(&path, &source).unwrap();
        let before = runtime(&path);
        fix(dir.path(), Some(PEERS));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(runtime(&path), before);
    }
}

#[test]
fn command_and_nested_deferred_ownership_are_source_only() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for source in [
        "p <<~`#{1 + 2}`\n#{1+2}\n#{1 + 2}\n",
        "p <<~OUT, <<~\"#{1 + 2}\"\nouter\nOUT\n#{1+2}\n#{1 + 2}\n",
        "p <<~OUT\n#{<<~\"#{1 + 2}\"}\n#{1+2}\n#{1 + 2}\nOUT\n",
    ] {
        assert_eq!(ruby_prism::parse(source.as_bytes()).errors().count(), 0);
        fs::write(&path, source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        fix(dir.path(), Some(PEERS));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn safe_body_prefix_and_sibling_edits_survive_selected_and_default_composition() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for selected in [Some(PEERS), None] {
        let source = "a=1\nb = 2\np <<~\"#{a + b}\"\n#{a+b}\n#{a + b}\np 'sibling'\n";
        fs::write(&path, source).unwrap();
        let before = runtime(&path);
        for _ in 0..3 {
            fix(dir.path(), selected);
        }
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(
            after,
            source
                .replace("a=1", "a = 1")
                .replace("'sibling'", "\"sibling\"")
        );
        assert_eq!(runtime(&path), before);
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        fix(dir.path(), selected);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
    for (label, body) in [
        ("X", "#@x #{1+2}"),
        ("\"X\"", "#@x #{1+2}"),
        ("\"#{1 + 2}\"", "#@x #{1+2} tail"),
    ] {
        let source = format!(
            "@x = 4\np <<~{label}\n  {body}\n{}\n",
            label.trim_matches('"')
        );
        fs::write(&path, &source).unwrap();
        let before = runtime(&path);
        fix_status(dir.path(), Some(PEERS), 0);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source.replace("#@x", "#{@x}").replace("1+2", "1 + 2")
        );
        assert_eq!(runtime(&path), before);
    }
}

#[test]
fn nested_and_shared_header_safe_body_fixes_remain_eligible() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for source in [
        "p <<~OUT, <<~IN\n#{1+2}\nOUT\n#{3+4}\nIN\n",
        "p <<~OUT\n#{<<~IN}\n#{1+2}\nIN\nOUT\n",
    ] {
        fs::write(&path, source).unwrap();
        let before = runtime(&path);
        fix_status(dir.path(), Some(PEERS), 0);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source.replace("1+2", "1 + 2").replace("3+4", "3 + 4")
        );
        assert_eq!(runtime(&path), before);
    }
}
