//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/line_continuation_leading_space_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 32
//! pinned examples, the `EnforcedStyle: leading` context included, are
//! replayed at the fixed `EnforcedStyle: trailing`; findings, messages and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/LineContinuationLeadingSpace";

/// `(line, col)` of each finding, checking the message, severity and that no
/// edit is offered without a correction request.
fn findings(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    lint_str_with(&[&LineContinuationLeadingSpace], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col)
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
    let found = correction_offenses(&LineContinuationLeadingSpace, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    // EnforcedStyle: trailing
    (
        8,
        "'this text is too' \\\n' long'\n",
        &[(2, 2)],
        "'this text is too ' \\\n'long'\n",
    ),
    (
        21,
        "'this text is too' \\\n  ' long'\n",
        &[(2, 4)],
        "'this text is too ' \\\n  'long'\n",
    ),
    (
        34,
        "'this text contains a lot of' \\\n'               spaces'\n",
        &[(2, 2)],
        "'this text contains a lot of               ' \\\n'spaces'\n",
    ),
    (
        47,
        "'this text is too' \\\n' long' \\\n'  long long'\n",
        &[(2, 2), (3, 2)],
        "'this text is too ' \\\n'long  ' \\\n'long long'\n",
    ),
    (
        63,
        "something_unrelated_to_the_line_continuation_below\n'this text is too' \\\n' long'\n",
        &[(3, 2)],
        "something_unrelated_to_the_line_continuation_below\n'this text is too ' \\\n'long'\n",
    ),
    (
        78,
        "long_method_name 'this text is too' \\\n  ' long' do\nend\n",
        &[(2, 4)],
        "long_method_name 'this text is too ' \\\n  'long' do\nend\n",
    ),
    (
        93,
        "long_method_name(\n  'this text is too' \\\n  ' long'\n)\n",
        &[(3, 4)],
        "long_method_name(\n  'this text is too ' \\\n  'long'\n)\n",
    ),
    (110, "'\"this text is too\" \\\n\" long\"'\n", &[], ""),
    (117, "\"\" \"foo\nbar\\\n\"\n", &[], ""),
    (
        125,
        "\"foo\nbar\" \\\n\" baz\"\n",
        &[(3, 2)],
        "\"foo\nbar \" \\\n\"baz\"\n",
    ),
    (141, "\"foo #{bar}\"\n", &[], ""),
    (147, "\"foo #{bar} baz #{qux}\"\n", &[], ""),
    (
        153,
        "\"foo #{bar}\" \\\n' long'\n",
        &[(2, 2)],
        "\"foo #{bar} \" \\\n'long'\n",
    ),
    (
        166,
        "'this line is' \\\n\" #{foo}\"\n",
        &[(2, 2)],
        "'this line is ' \\\n\"#{foo}\"\n",
    ),
    (179, "\"five is #{2 + \\\n  3}\"\n", &[], ""),
    (186, "\"foo #{'bar ' \\\n  'baz'}\"\n", &[], ""),
    (
        193,
        "\"foo #{'bar' \\\n  ' baz'}\"\n",
        &[(2, 4)],
        "\"foo #{'bar ' \\\n  'baz'}\"\n",
    ),
    // EnforcedStyle: leading, replayed at the fixed trailing style
    (211, "'this text is too ' \\\n'long'\n", &[], ""),
    (
        224,
        "'this text is too' \\\n  ' very ' \\\n  'long'\n",
        &[(2, 4)],
        "'this text is too ' \\\n  'very ' \\\n  'long'\n",
    ),
    (
        239,
        "'this text contains a lot of               ' \\\n'spaces'\n",
        &[],
        "",
    ),
    (
        252,
        "'this text is too ' \\\n'long  ' \\\n'long long'\n",
        &[],
        "",
    ),
    (
        268,
        "something_unrelated_to_the_line_continuation_below\n'this text is too ' \\\n'long'\n",
        &[],
        "",
    ),
    (
        283,
        "long_method_name 'this text is too' \\\n  ' very ' \\\n  'long' do\nend\n",
        &[(2, 4)],
        "long_method_name 'this text is too ' \\\n  'very ' \\\n  'long' do\nend\n",
    ),
    (
        300,
        "long_method_name(\n  'this text is too ' \\\n  'long'\n)\n",
        &[],
        "",
    ),
    (317, "'\"this text is too \" \\\n\"long\"'\n", &[], ""),
    (325, "\"foo #{bar}\"\n", &[], ""),
    (331, "\"foo #{bar} baz #{qux}\"\n", &[], ""),
    (337, "'foo ' \\\n\"#{bar} long\"\n", &[], ""),
    (350, "\"this #{foo} is \" \\\n'long'\n", &[], ""),
    (363, "\"five is #{2 + \\\n  3}\"\n", &[], ""),
    (
        370,
        "\"foo #{'bar' \\\n  ' baz'}\"\n",
        &[(2, 4)],
        "\"foo #{'bar ' \\\n  'baz'}\"\n",
    ),
    (377, "\"foo #{'bar ' \\\n  'baz'}\"\n", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 32);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&LineContinuationLeadingSpace, source).is_empty());
        } else {
            assert_correction(&LineContinuationLeadingSpace, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    assert!(LineContinuationLeadingSpace.default_enabled());
    assert!(LineContinuationLeadingSpace.department_selection_enables());
    assert!(LineContinuationLeadingSpace.safe_fixes());
    // The upstream Style/StringLiterals ordering edge is resolved by overlap
    // selection, not declared (see the module documentation).
    assert!(
        LineContinuationLeadingSpace
            .autocorrect_incompatible_with()
            .is_empty()
    );
}

#[test]
fn every_quoted_part_moves_its_blanks_back() {
    for (source, expected, after) in [
        (
            "x = \"a\" \\\n  \" b\"\n",
            (2, 4),
            "x = \"a \" \\\n  \"b\"\n",
        ),
        ("x = 'a'\\\n  ' b'\n", (2, 4), "x = 'a '\\\n  'b'\n"),
        (
            "x = 'a'   \\\n  '\t b'\n",
            (2, 4),
            "x = 'a\t '   \\\n  'b'\n",
        ),
        (
            "x = \"a\\\\\" \\\n  ' b'\n",
            (2, 4),
            "x = \"a\\\\ \" \\\n  'b'\n",
        ),
        (
            "x = 'a\\\\' \\\n  ' b'\n",
            (2, 4),
            "x = 'a\\\\ ' \\\n  'b'\n",
        ),
        (
            "x = %Q\"a\" \\\n  ' b'\n",
            (2, 4),
            "x = %Q\"a \" \\\n  'b'\n",
        ),
        ("x = \"é\" \\\n  ' ü'\n", (2, 4), "x = \"é \" \\\n  'ü'\n"),
        ("x = 'a' \\\n  '   '\n", (2, 4), "x = 'a   ' \\\n  ''\n"),
        ("\u{feff}'a' \\\n' b'\n", (2, 2), "\u{feff}'a ' \\\n'b'\n"),
        (
            "x = \"#{a}\" \\\n  \" #{b}\"\n",
            (2, 4),
            "x = \"#{a} \" \\\n  \"#{b}\"\n",
        ),
        (
            "foo('a' \\\n  ' b', 1)\n",
            (2, 4),
            "foo('a ' \\\n  'b', 1)\n",
        ),
    ] {
        assert_eq!(findings(source), [expected], "{source:?}");
        assert_correction(&LineContinuationLeadingSpace, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = 'a ' \\\n  'b'\n",
        "x = 'a' +\n  ' b'\n",
        "x = 'a' \\\n  + ' b'\n",
        "x = ['a',\n  ' b']\n",
        "x = 'a' \\\n  %q( b)\n",
        "x = 'a\\\n  b'\n",
        "x = \"a\\\n  b\"\n",
        "x = \"a #{b}\\\n  c\"\n",
        "x = <<~A\n  a \\\n  ' b'\nA\n",
        // An escaped `\t` is not a blank of the source.
        "x = 'a' \\\n  \"\\t b\"\n",
        "x = 'a' 'b' \\\n  'c'\n",
        // CRLF: upstream requires `\` + `\n`.
        "x = 'a' \\\r\n  ' b'\r\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn uncertain_moves_are_withheld() {
    for source in [
        // The earlier part does not end with a quote: upstream raises.
        "x = %q(a) \\\n  ' b'\n",
        "x = %(a) \\\n  ' b'\n",
        // The blanks reach the end of the line.
        "x = 'a' \\\n  '  \n  b'\n",
        // the listed source observers.
        "x = 'a' \\\n  ' b'\np __FILE__\n",
        "x = 'a' \\\n  ' b'\np __dir__\n",
        "x = 'a' \\\n  ' b'\np $0\n",
        "x = 'a' \\\n  ' b'\np $PROGRAM_NAME\n",
        "x = 'a' \\\n  ' b'\np caller\n",
        "x = 'a' \\\n  ' b'\np caller_locations\n",
        "x = 'a' \\\n  ' b'\np method(:f).source_location\n",
        "x = 'a' \\\n  ' b'\nSCRIPT_LINES__ = {}\n",
        "x = 'a' \\\n  ' b'\np DATA.read\n__END__\nd\n",
    ] {
        held(source);
    }
    assert_correction(
        &LineContinuationLeadingSpace,
        "x = 'a' \\\n  ' b'\np File.read('example.rb')\n",
        "x = 'a ' \\\n  'b'\np File.read('example.rb')\n",
    );
}

#[test]
fn concatenated_values_are_unchanged() {
    // The corrected source concatenates to the same bytes: each pinned and
    // authored correction is compared by its Prism-unescaped parts.
    fn value(source: &str) -> Vec<u8> {
        struct Strings(Vec<u8>);
        impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Strings {
            fn enter(&mut self, node: Node<'pr>) {
                if let Some(s) = node.as_string_node() {
                    self.0.extend_from_slice(s.unescaped());
                }
            }
        }
        let result = crate::parser::parse(source.as_bytes());
        let mut strings = Strings(Vec::new());
        crate::cops::walk::walk(&mut strings, &result.node());
        strings.0
    }
    for &(_, source, expected, corrected) in PINNED {
        if !expected.is_empty() {
            assert_eq!(value(source), value(corrected), "{source:?}");
        }
    }
    for (before, after) in [
        ("x = 'a' \\\n  '\t b'\n", "x = 'a\t ' \\\n  'b'\n"),
        ("x = \"a\\\\\" \\\n  ' b'\n", "x = \"a\\\\ \" \\\n  'b'\n"),
    ] {
        assert_eq!(value(before), value(after));
    }
}

#[test]
fn string_literals_composition_converges_without_undoing_the_move() {
    let both: [&dyn Cop; 2] = [
        &LineContinuationLeadingSpace,
        &crate::cops::style::StringLiterals,
    ];
    let mut source = "x = 'a' \\\n  ' b'\n".to_owned();
    for _ in 0..4 {
        let found = crate::pipeline::lint_str_with_corrections(&both, &source);
        let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        if applied.count == 0 {
            break;
        }
        source = String::from_utf8(applied.bytes).unwrap();
        assert_eq!(crate::parser::parse(source.as_bytes()).errors().count(), 0);
    }
    assert_eq!(source, "x = \"a \" \\\n  \"b\"\n");
}

#[test]
fn every_break_of_a_concatenation_is_corrected_in_one_pass() {
    let source = "x = 'a' \\\n  ' b' \\\n  '  c' \\\n  'd'\n";
    assert_eq!(findings(source), [(2, 4), (3, 4)]);
    assert_correction(
        &LineContinuationLeadingSpace,
        source,
        "x = 'a ' \\\n  'b  ' \\\n  'c' \\\n  'd'\n",
    );
}
