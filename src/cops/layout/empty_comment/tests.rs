//! Examples ported from RuboCop's `spec/rubocop/cop/layout/empty_comment_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 15
//! pinned examples run at the fixed defaults (`AllowBorderComment` and
//! `AllowMarginComment` true): the `disallow border comment` and `disallow
//! margin comment` contexts are replayed at those defaults, where the border
//! and margin sources are accepted. Findings, messages and single-pass
//! corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str,
};

const COP: &str = "Layout/EmptyComment";
const MESSAGE: &str = "Source code comment is empty.";

fn positions(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        ruby_prism::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MESSAGE);
            assert_eq!(o.end_line, o.line);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            (o.line, o.col)
        })
        .collect()
}

/// Every finding is kept and none offers an edit.
fn held(source: &str) {
    let found = correction_offenses(&EmptyComment, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, fixed-default findings, single-pass correction)`.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const BORDER: &str = "#################################\n";
const MARGIN: &str = "#\n# Description of `hello` method.\n#\ndef hello\nend\n";

const PINNED: &[Pinned] = &[
    (7, "#\n", &[(1, 1)], ""),
    (16, "#\n#\n", &[(1, 1), (2, 1)], ""),
    (
        27,
        "def foo #\n  something\nend\n",
        &[(1, 9)],
        "def foo\n  something\nend\n",
    ),
    (
        42,
        "x = <<~HEREDOC #\n  hello\nHEREDOC\n",
        &[(1, 16)],
        "x = <<~HEREDOC\n  hello\nHEREDOC\n",
    ),
    (
        57,
        "# comment\ndef foo #\n  something\nend\n",
        &[(2, 9)],
        "# comment\ndef foo\n  something\nend\n",
    ),
    (
        74,
        "# Description of `Foo` class.\nclass Foo\n  # Description of `hello` method.\n  def hello\n  end\nend\n",
        &[],
        "",
    ),
    (
        85,
        "#\n# Description of `Foo` class.\n#\nclass Foo\n  #\n  # Description of `hello` method.\n  #\n  def hello\n  end\nend\n",
        &[],
        "",
    ),
    (
        164,
        "def foo#\n  something\nend\n",
        &[(1, 8)],
        "def foo\n  something\nend\n",
    ),
    (
        179,
        "def foo #\n  something #\nend\n",
        &[(1, 9), (2, 13)],
        "def foo\n  something\nend\n",
    ),
    (
        195,
        "def foo     #\n  something #\nend\n",
        &[(1, 13), (2, 13)],
        "def foo\n  something\nend\n",
    ),
    (101, BORDER, &[], ""),
    // AllowBorderComment: false, replayed at the fixed true.
    (111, "#\n", &[(1, 1)], ""),
    (120, BORDER, &[], ""),
    (131, MARGIN, &[], ""),
    // AllowMarginComment: false, replayed at the fixed true.
    (145, MARGIN, &[], ""),
];

#[test]
fn pinned_examples_at_fixed_defaults() {
    for &(spec_line, source, at, after) in PINNED {
        assert_eq!(positions(source), at, "spec line {spec_line}: {source:?}");
        if !at.is_empty() {
            assert_correction(&EmptyComment, source, after);
        }
    }
    assert_eq!(PINNED.len(), 15);
}

#[test]
fn chunks_follow_consecutive_lines_and_character_columns() {
    // A text, border or `=begin` document in the chunk accepts it.
    for source in [
        "##\n",
        "# #\n",
        "#\n##\n",
        "#\n=begin\nx\n=end\n",
        "x = 1 #\ny = 2 # text\n",
        // Character columns: both comments start at column 4.
        "é  #\nab # text\n",
    ] {
        assert!(positions(source).is_empty(), "{source:?}");
    }
    for (source, at) in [
        ("#\n  #\n", &[(1, 1), (2, 3)][..]),
        ("# a\n  #\n", &[(2, 3)]),
        ("x #\n#\n", &[(1, 3), (2, 1)]),
        ("#\n\n#\n", &[(1, 1), (3, 1)]),
        ("é #\nab # text\n", &[(1, 3)]),
        ("#  \t\n", &[(1, 1)]),
        ("\u{feff}#\n#\n", &[(1, 1), (2, 1)]),
        ("x = 1\n__END__\n#\n", &[]),
    ] {
        assert_eq!(positions(source), at, "{source:?}");
    }
}

#[test]
fn canonical_removals() {
    for (before, after) in [
        ("x = 1 #\r\ny = 2\r\n", "x = 1\r\ny = 2\r\n"),
        ("#\r\nx = 1\r\n", "x = 1\r\n"),
        ("\u{feff}#\nx = 1\n", "\u{feff}x = 1\n"),
        ("x = 1 \t#  \t\ny = 2\n", "x = 1\ny = 2\n"),
        ("def f\n  #\n  x\nend\n", "def f\n  x\nend\n"),
        ("x = 1\n#", "x = 1\n"),
        ("x = 1 #", "x = 1"),
        ("x = [1, #\n  2]\n", "x = [1,\n  2]\n"),
        ("foo(<<~A) #\n  a\nA\n", "foo(<<~A)\n  a\nA\n"),
        ("x = <<~A\n  #{y #\n  }\nA\n", "x = <<~A\n  #{y\n  }\nA\n"),
        ("x = \"é\" #\n", "x = \"é\"\n"),
        // Lines 1-2 move only lines without a shebang or coding comment.
        ("#\nx = 1\n# text\n", "x = 1\n# text\n"),
        (
            "x = 1\ny = 2\n  #\n# coding: binary\n",
            "x = 1\ny = 2\n# coding: binary\n",
        ),
        // Removals after the header line, or that leave it on line 3.
        (
            "x = 1\n#\n y = 2\n # \nz = 3\n# coding: binary\n",
            "x = 1\n y = 2\nz = 3\n# coding: binary\n",
        ),
        (
            "# coding: binary\nx = 1\n#\n #\n",
            "# coding: binary\nx = 1\n",
        ),
        // A line before a backslash continuation.
        ("#\nx = 1 \\\n  + 2\n", "x = 1 \\\n  + 2\n"),
    ] {
        assert_correction(&EmptyComment, before, after);
    }
}

#[test]
fn listed_source_observers_hold_the_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p caller",
        "p $PROGRAM_NAME",
    ] {
        held(&format!("x = 1 #\n{observer}\n"));
    }
    held("x = 1 #\np DATA.read\n__END__\ndata\n");
}

#[test]
fn directives_in_both_spellings() {
    for prefix in ["rubocop", "rustocop"] {
        let source = format!("x = 1 #\n# {prefix}:disable Layout/EmptyComment\ny = 2 #\n");
        assert_eq!(positions(&source), [(1, 7)], "{source:?}");
        // An empty comment between a disable-next directive and its
        // statement: the directive's reach must not change with the edit.
        let before = format!("# {prefix}:disable-next Lint/Debugger\n  #\nbinding.pry\n");
        let after = format!("# {prefix}:disable-next Lint/Debugger\nbinding.pry\n");
        let debugger = |s: &str| lint_str(s).iter().any(|o| o.cop == "Lint/Debugger");
        assert_eq!(positions(&before), [(2, 3)]);
        if debugger(&before) == debugger(&after) {
            assert_correction(&EmptyComment, &before, &after);
        } else {
            held(&before);
        }
    }
}
