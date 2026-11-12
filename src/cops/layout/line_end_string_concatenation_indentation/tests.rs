//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/line_end_string_concatenation_indentation_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 62
//! pinned examples (the shared `common` group in both contexts, the six
//! assignment operators of each context, and the `EnforcedStyle: indented`
//! and `IndentationWidth: 1` contexts) are replayed at the fixed
//! `EnforcedStyle: aligned` with indentation width 2; findings, messages and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/LineEndStringConcatenationIndentation";
const A: &str = "A";
const I: &str = "I";

/// `(line, col, message kind)` of each finding, checking the severity and
/// that no edit is offered without a correction request.
fn findings(source: &str) -> Vec<(u32, u32, &'static str)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    lint_str_with(&[&LineEndStringConcatenationIndentation], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.end_line, o.line);
            let kind = match o.message.as_str() {
                MSG_ALIGN => A,
                MSG_INDENT => I,
                other => panic!("{other}"),
            };
            (o.line, o.col, kind)
        })
        .collect()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let found = correction_offenses(&LineEndStringConcatenationIndentation, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, String, Vec<(u32, u32, &'static str)>, String);

const LHS: [&str; 6] = ["X =", "$x =", "@x =", "x =", "x +=", "x ||="];

const HASH: &str = "MESSAGES = { KeyAlignment => 'Align the keys of a hash literal if ' \\\n  'they span more than one line.',\n             SeparatorAlignment => 'Align the separators of a hash ' \\\n               'literal if they span more than one line.',\n             TableAlignment => 'Align the keys and values of a hash ' \\\n               'literal if they span more than one line.' }.freeze\n";
const HASH_ALIGNED: &str = "MESSAGES = { KeyAlignment => 'Align the keys of a hash literal if ' \\\n                             'they span more than one line.',\n             SeparatorAlignment => 'Align the separators of a hash ' \\\n                                   'literal if they span more than one line.',\n             TableAlignment => 'Align the keys and values of a hash ' \\\n                               'literal if they span more than one line.' }.freeze\n";
const HEREDOC: &str = "let(:source) do\n  <<~CODE\n    func({\n      @abc => 0,\n      @xyz => 1\n    })\n    func(\n      {\n        abc: 0\n      }\n    )\n    func(\n      {},\n      {\n        xyz: 1\n      }\n    )\n  CODE\nend\n";

fn row(line: u32, source: &str, found: &[(u32, u32, &'static str)], corrected: &str) -> Pinned {
    (line, source.into(), found.to_vec(), corrected.into())
}

/// The shared `common` examples, identical at the fixed style in both
/// contexts.
fn common() -> Vec<Pinned> {
    vec![
        row(
            19,
            "text = 'offense'\nputs 'This probably should not be '\"an #{text}\"\n",
            &[],
            "",
        ),
        row(
            26,
            "text = 'offense'\nputs 'This probably\n      should not be '\"an #{text}\"\n",
            &[],
            "",
        ),
        row(34, "puts %(\n  foo\n  bar\n)\n", &[], ""),
        row(
            43,
            "some_method do\n  'a' \\\n    'b' \\\n    'c'\nend\n",
            &[],
            "",
        ),
        row(
            53,
            "def some_method\n  'a' \\\n    'b' \\\n    'c'\nend\n",
            &[],
            "",
        ),
        row(
            63,
            "if cond1\n  'a' \\\n  'b'\nelsif cond2\n  'c' \\\n  'd'\nelse\n  'e' \\\n  'f'\nend\n",
            &[(3, 3, I), (6, 3, I), (9, 3, I)],
            "if cond1\n  'a' \\\n    'b'\nelsif cond2\n  'c' \\\n    'd'\nelse\n  'e' \\\n    'f'\nend\n",
        ),
        row(
            94,
            "def self.some_method\n  'a' \\\n    'b' \\\n    'c'\nend\n",
            &[],
            "",
        ),
        row(
            104,
            "def some_method\n  b = 'b'\n  'a' \\\n    \"#{b}\" \\\n    'c'\nend\n",
            &[],
            "",
        ),
        row(115, "'a' \\\n  'b' \\\n  'c'\n", &[], ""),
        row(123, "warn <<~TEXT\n  A #{b}\nTEXT\n", &[], ""),
        row(131, HEREDOC, &[], ""),
        row(155, "puts(<<~TEXT)\nTEXT\n", &[], ""),
        row(162, "%\n\n", &[], ""),
    ]
}

fn pinned() -> Vec<Pinned> {
    // when EnforcedStyle is aligned
    let mut rows = vec![
        row(172, "puts 'a' \\\n     'b'\n", &[], ""),
        row(
            200,
            HASH,
            &[(2, 3, A), (4, 16, A), (6, 16, A)],
            HASH_ALIGNED,
        ),
        row(
            223,
            "puts 'a' \\\n  \"b\" \\\n  'c'\n",
            &[(2, 3, A)],
            "puts 'a' \\\n     \"b\" \\\n     'c'\n",
        ),
        row(
            238,
            "puts 'a' \\\n  'b' \\\n     'c'\n",
            &[(2, 3, A)],
            "puts 'a' \\\n     'b' \\\n     'c'\n",
        ),
        row(
            253,
            "puts 'a' \\\n     'b' \\\n'c' \\\n'd'\n",
            &[(3, 1, A)],
            "puts 'a' \\\n     'b' \\\n     'c' \\\n     'd'\n",
        ),
    ];
    rows.extend(common());
    for lhs in LHS {
        let aligned = format!("{lhs} \"a\" \\\n{} 'b'\n", " ".repeat(lhs.len()));
        rows.push((185, aligned.clone(), vec![], String::new()));
        rows.push((
            189,
            format!("{lhs} \"a\" \\\n  'b'\n"),
            vec![(2, 3, A)],
            aligned,
        ));
    }
    // when EnforcedStyle is indented, replayed at the fixed aligned style
    rows.push(row(
        276,
        "puts 'a' \\\n  'b'\n",
        &[(2, 3, A)],
        "puts 'a' \\\n     'b'\n",
    ));
    rows.push(row(
        283,
        "x = 'a' \\\n  'b' \\\n'c' \\\n'd' \\\n'e'\n",
        &[(2, 3, A), (3, 1, A)],
        "x = 'a' \\\n    'b' \\\n    'c' \\\n    'd' \\\n    'e'\n",
    ));
    rows.push(row(
        302,
        "puts 'a' \\\n    'b' \\\n  'c'\n",
        &[(2, 5, A), (3, 3, A)],
        "puts 'a' \\\n     'b' \\\n     'c'\n",
    ));
    rows.push(row(341, HASH_ALIGNED, &[], ""));
    rows.push(row(364, "puts %Q(a) \\\n     'b' \\\n     'c'\n", &[], ""));
    rows.push(row(
        379,
        "puts 'a' \\\n  \"#{b}\" \\\n     \"#{c}\"\n",
        &[(2, 3, A)],
        "puts 'a' \\\n     \"#{b}\" \\\n     \"#{c}\"\n",
    ));
    rows.extend(common());
    for lhs in LHS {
        let aligned = format!("{lhs} \"a\" \\\n{} 'b'\n", " ".repeat(lhs.len()));
        rows.push((
            323,
            format!("{lhs} \"a\" \\\n  'b'\n"),
            vec![(2, 3, A)],
            aligned.clone(),
        ));
        let margin = " ".repeat(lhs.len() + 2);
        rows.push((
            328,
            format!("{lhs} \"a\" \\\n{margin}'b'\n"),
            vec![(2, lhs.len() as u32 + 3, A)],
            aligned,
        ));
    }
    // when IndentationWidth is 1
    rows.push(row(
        397,
        "puts 'a' \\\n 'b'\n",
        &[(2, 2, A)],
        "puts 'a' \\\n     'b'\n",
    ));
    rows
}

#[test]
fn pinned_examples_match_the_fixed_replay() {
    let rows = pinned();
    assert_eq!(rows.len(), 62);
    for (line, source, expected, corrected) in rows {
        assert_eq!(findings(&source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(
                correction_offenses(&LineEndStringConcatenationIndentation, &source).is_empty()
            );
        } else {
            assert_correction(&LineEndStringConcatenationIndentation, &source, &corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = LineEndStringConcatenationIndentation;
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
}

#[test]
fn implicit_return_and_statement_contexts_are_indented() {
    // Parser parents nil, block, begin, def, defs and if.
    for (source, expected, after) in [
        // Several statements are a `begin`.
        ("x = 1\n'a' \\\n'b'\n", (3, 1), "x = 1\n'a' \\\n  'b'\n"),
        ("foo { 'a' \\\n'b' }\n", (2, 1), "foo { 'a' \\\n  'b' }\n"),
        (
            "foo do |x|\n  'a' \\\n  'b'\nend\n",
            (3, 3),
            "foo do |x|\n  'a' \\\n    'b'\nend\n",
        ),
        (
            "-> do\n  'a' \\\n  'b'\nend\n",
            (3, 3),
            "-> do\n  'a' \\\n    'b'\nend\n",
        ),
        ("def f = 'a' \\\n'b'\n", (2, 1), "def f = 'a' \\\n  'b'\n"),
        ("x = ('a' \\\n'b')\n", (2, 1), "x = ('a' \\\n  'b')\n"),
        (
            "x = c ? 'a' \\\n'b' : 1\n",
            (2, 1),
            "x = c ? 'a' \\\n  'b' : 1\n",
        ),
        (
            "unless c\n  'a' \\\n  'b'\nend\n",
            (3, 3),
            "unless c\n  'a' \\\n    'b'\nend\n",
        ),
        (
            "unless c\nelse\n  'a' \\\n  'b'\nend\n",
            (4, 3),
            "unless c\nelse\n  'a' \\\n    'b'\nend\n",
        ),
        ("if 'a' \\\n'b'\nend\n", (2, 1), "if 'a' \\\n  'b'\nend\n"),
        ("'a' \\\n'b' if c\n", (2, 1), "'a' \\\n  'b' if c\n"),
        (
            "begin\n  x\n  'a' \\\n  'b'\nrescue\nend\n",
            (4, 3),
            "begin\n  x\n  'a' \\\n    'b'\nrescue\nend\n",
        ),
        // The later parts align with the second part or the indented column.
        (
            "def f\n  'a' \\\n  'b' \\\n    'c'\nend\n",
            (3, 3),
            "def f\n  'a' \\\n    'b' \\\n    'c'\nend\n",
        ),
    ] {
        assert_eq!(
            findings(source),
            [(expected.0, expected.1, I)],
            "{source:?}"
        );
        assert_correction(&LineEndStringConcatenationIndentation, source, after);
    }
    assert_eq!(
        findings("def f\n  'a' \\\n  'b' \\\n       'c'\nend\n"),
        [(3, 3, I), (4, 8, A)]
    );
}

#[test]
fn other_contexts_are_aligned() {
    for (source, expected, after) in [
        ("foo('a' \\\n  'b')\n", (2, 3), "foo('a' \\\n    'b')\n"),
        (
            "foo { 'a' \\\n  \"#{_1}\" }\n",
            (2, 3),
            "foo { 'a' \\\n      \"#{_1}\" }\n",
        ),
        (
            "foo { 'a' \\\n  \"#{it}\" }\n",
            (2, 3),
            "foo { 'a' \\\n      \"#{it}\" }\n",
        ),
        (
            "foo { 'a' \\\n  'b' + _1 }\n",
            (2, 3),
            "foo { 'a' \\\n      'b' + _1 }\n",
        ),
        (
            "begin\n  x\n  'a' \\\n    'b'\nend\n",
            (4, 5),
            "begin\n  x\n  'a' \\\n  'b'\nend\n",
        ),
        (
            "class C\n  'a' \\\n    'b'\nend\n",
            (3, 5),
            "class C\n  'a' \\\n  'b'\nend\n",
        ),
        (
            "while c\n  'a' \\\n    'b'\nend\n",
            (3, 5),
            "while c\n  'a' \\\n  'b'\nend\n",
        ),
        (
            "case c\nwhen 1\n  'a' \\\n    'b'\nend\n",
            (4, 5),
            "case c\nwhen 1\n  'a' \\\n  'b'\nend\n",
        ),
        (
            "return 'a' \\\n  'b'\n",
            (2, 3),
            "return 'a' \\\n       'b'\n",
        ),
        ("x = ['a' \\\n  'b']\n", (2, 3), "x = ['a' \\\n     'b']\n"),
        ("x = 'é' \\\n  'b'\n", (2, 3), "x = 'é' \\\n    'b'\n"),
        (
            "\u{feff}x = 'a' \\\n  'b'\n",
            (2, 3),
            "\u{feff}x = 'a' \\\n    'b'\n",
        ),
        (
            "x = 'a' \\\r\n  'b'\r\n",
            (2, 3),
            "x = 'a' \\\r\n    'b'\r\n",
        ),
        ("x = 'a' \\\n\t'b'\n", (2, 2), "x = 'a' \\\n\t   'b'\n"),
        ("x = 'a' \\\n      'b'\n", (2, 7), "x = 'a' \\\n    'b'\n"),
    ] {
        assert_eq!(
            findings(source),
            [(expected.0, expected.1, A)],
            "{source:?}"
        );
        assert_correction(&LineEndStringConcatenationIndentation, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = 'a' \\\n    'b'\n",
        "x = 'a' 'b'\n",
        "x = 'a' +\n  'b'\n",
        "x = \"a\n  b\"\n",
        "x = \"a\\\n  b\"\n",
        "x = 'a' \\\n  \"b\n  c\"\n",
        "x = \"#{a}\n  b\"\n",
        "x = <<~A\n  a\nA\n",
        "x = %w[a\n  b]\n",
        "begin\n  'a' \\\n  'b'\nend\n",
        "class C\n  'a' \\\n  'b'\nend\n",
        "foo { 'a' \\\n  'b' }\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn uncertain_edits_are_withheld() {
    for source in [
        // A part that does not begin its line.
        "x = 'a' \\\n    'b'   'c'\n",
        // Blanks that cannot be removed column for column.
        "x = 'a' \\\n\x0b\x0b\x0b\x0b\x0b\x0b'b'\n",
        // Inside the interpolation of another literal.
        "\"#{'a' \\\n'b'}\"\n",
        "x = <<~A\n  #{'a' \\\n  'b'}\nA\n",
        // the listed source observers.
        "x = 'a' \\\n  'b'\np __FILE__\n",
        "x = 'a' \\\n  'b'\np __dir__\n",
        "x = 'a' \\\n  'b'\np $0\n",
        "x = 'a' \\\n  'b'\np $PROGRAM_NAME\n",
        "x = 'a' \\\n  'b'\np caller\n",
        "x = 'a' \\\n  'b'\np caller_locations\n",
        "x = 'a' \\\n  'b'\np method(:f).source_location\n",
        "x = 'a' \\\n  'b'\nSCRIPT_LINES__ = {}\n",
        "x = 'a' \\\n  'b'\np DATA.read\n__END__\nd\n",
    ] {
        held(source);
    }
    assert_correction(
        &LineEndStringConcatenationIndentation,
        "x = 'a' \\\n  'b'\np File.read('example.rb')\n",
        "x = 'a' \\\n    'b'\np File.read('example.rb')\n",
    );
}

#[test]
fn values_and_tokens_are_unchanged() {
    for (_, source, expected, corrected) in pinned() {
        if expected.is_empty() {
            continue;
        }
        let strings = |s: &str| {
            let result = crate::parser::parse(s.as_bytes());
            let mut out = Vec::new();
            struct V<'a>(&'a mut Vec<Vec<u8>>);
            impl<'pr> crate::cops::walk::NodeVisitor<'pr> for V<'_> {
                fn enter(&mut self, node: Node<'pr>) {
                    if let Some(s) = node.as_string_node() {
                        self.0.push(s.unescaped().to_vec());
                    }
                }
            }
            crate::cops::walk::walk(&mut V(&mut out), &result.node());
            out
        };
        assert_eq!(strings(&source), strings(&corrected), "{source:?}");
    }
}
