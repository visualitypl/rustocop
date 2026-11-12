//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_before_brackets_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 28 pinned examples
//! run at the fixed defaults (upstream's pending cop enabled); findings,
//! messages and single-pass corrected bytes equal the fixed replay recorded
//! in the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceBeforeBrackets";

fn findings(source: &str) -> Vec<(u32, u32, &'static str)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&SpaceBeforeBrackets], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col, MSG)
        })
        .collect();
    found.sort_unstable();
    found
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceBeforeBrackets, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (
    u32,
    &'static str,
    &'static [(u32, u32, &'static str)],
    &'static str,
);

const PINNED: &[Pinned] = &[
    (197, "task.options = ['--no-output']\n", &[], ""),
    (
        203,
        "before_validation { to_downcase ['email'] }\n",
        &[],
        "",
    ),
    (
        209,
        "before_validation { to_downcase %w[email] }\n",
        &[],
        "",
    ),
    (
        6,
        "collection = do_something\ncollection [index_or_key]\n",
        &[(2, 11, MSG)],
        "collection = do_something\ncollection[index_or_key]\n",
    ),
    (
        19,
        "@collection [index_or_key]\n",
        &[(1, 12, MSG)],
        "@collection[index_or_key]\n",
    ),
    (
        30,
        "@@collection [index_or_key]\n",
        &[(1, 13, MSG)],
        "@@collection[index_or_key]\n",
    ),
    (
        41,
        "$collection [index_or_key]\n",
        &[(1, 12, MSG)],
        "$collection[index_or_key]\n",
    ),
    (
        52,
        "collection.call(arg) [index_or_key]\n",
        &[(1, 21, MSG)],
        "collection.call(arg)[index_or_key]\n",
    ),
    (63, "do_something [item_of_array_literal]\n", &[], ""),
    (
        69,
        "collection = do_something\ncollection[index_or_key]\n",
        &[],
        "",
    ),
    (76, "do_something[item_of_array_literal]\n", &[], ""),
    (82, "do_something([item_of_array_literal])\n", &[], ""),
    (88, "expect(offenses).to eq []\n", &[], ""),
    (94, "do_something [foo], bar\n", &[], ""),
    (100, "[index_or_key]\n", &[], ""),
    (106, "collection.[](index_or_key)\n", &[], ""),
    (112, "collection. [](index_or_key)\n", &[], ""),
    (118, "collection .[](index_or_key)\n", &[], ""),
    (124, "collection&. [](index_or_key)\n", &[], ""),
    (130, "collection &.[](index_or_key)\n", &[], ""),
    (136, "@collection.[](index_or_key)\n", &[], ""),
    (142, "@@collection.[](index_or_key)\n", &[], ""),
    (148, "collection.[]=(index_or_key, value)\n", &[], ""),
    (
        156,
        "@correction [index_or_key] = :value\n",
        &[(1, 12, MSG)],
        "@correction[index_or_key] = :value\n",
    ),
    (
        167,
        "@correction [ index_or_key] = :value\n",
        &[(1, 12, MSG)],
        "@correction[ index_or_key] = :value\n",
    ),
    (178, "@correction[index_or_key] = :value\n", &[], ""),
    (184, "@collections[ index_or_key ] = :value\n", &[], ""),
    (190, "@collections[  index_or_key] = value\n", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 28);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line} {source:?}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceBeforeBrackets, source).is_empty());
        } else {
            assert_correction(&SpaceBeforeBrackets, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = SpaceBeforeBrackets;
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(cop.max_setting().is_none() && cop.path_policy().is_none());
}

#[test]
fn every_undotted_index_send() {
    for (source, expected, after) in [
        (
            "a = [1]\na [0] += 1\n",
            &[(2, 2, MSG)][..],
            "a = [1]\na[0] += 1\n",
        ),
        (
            "a = {}\na [0] ||= 1\n",
            &[(2, 2, MSG)],
            "a = {}\na[0] ||= 1\n",
        ),
        (
            "a = {}\na [0] &&= 1\n",
            &[(2, 2, MSG)],
            "a = {}\na[0] &&= 1\n",
        ),
        (
            "a = []\na [0], b = 1, 2\n",
            &[(2, 2, MSG)],
            "a = []\na[0], b = 1, 2\n",
        ),
        (
            "a = []\nfor a [0] in [1]; end\n",
            &[(2, 6, MSG)],
            "a = []\nfor a[0] in [1]; end\n",
        ),
        ("a = []\na\t[0]\n", &[(2, 2, MSG)], "a = []\na[0]\n"),
        (
            "a = [[1]]\na [0] [0]\n",
            &[(2, 2, MSG), (2, 6, MSG)],
            "a = [[1]]\na[0][0]\n",
        ),
        ("self [0]\n", &[(1, 5, MSG)], "self[0]\n"),
        ("'ab' [0]\n", &[(1, 5, MSG)], "'ab'[0]\n"),
        ("%w[a b] [0]\n", &[(1, 8, MSG)], "%w[a b][0]\n"),
        ("1 [0]\n", &[(1, 2, MSG)], "1[0]\n"),
        (":a [0]\n", &[(1, 3, MSG)], ":a[0]\n"),
        ("a.b(1) [0]\n", &[(1, 7, MSG)], "a.b(1)[0]\n"),
        ("a.b { 1 } [0]\n", &[(1, 10, MSG)], "a.b { 1 }[0]\n"),
        ("<<~A [0]\n  x\nA\n", &[(1, 5, MSG)], "<<~A[0]\n  x\nA\n"),
        (
            "[[1]].each { p _1 [0] }\n",
            &[(1, 18, MSG)],
            "[[1]].each { p _1[0] }\n",
        ),
        ("x = é; x [0]\n", &[(1, 9, MSG)], "x = é; x[0]\n"),
        ("\u{feff}@a [0]\n", &[(1, 3, MSG)], "\u{feff}@a[0]\n"),
        ("@a [0]\r\n", &[(1, 3, MSG)], "@a[0]\r\n"),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        assert_correction(&SpaceBeforeBrackets, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "a[0]\n",
        "a [0]\n",
        "a.b [0]\n",
        "A::B [0]\n",
        "A [0]\n",
        "a.b [0], 1\n",
        "puts [1]\n",
        "a.[](0)\n",
        "a&.[](0)\n",
        "a .[] 0\n",
        "x = [0]\n",
        "x = a ? [0] : [1]\n",
        "a = 1\na\n[0]\n",
        "super [0]\n",
        // `it [0]` is a call of `it` (`it[0]` is an index); `_1 [0]` indexes.
        "[[1]].each { p it [0] }\n",
        "def f = yield [0]\n",
        "x = \"a [0]\"\n",
        "# a [0]\n",
        "case [1]\nin [a] then a\nend\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn a_line_continuation_keeps_the_finding_without_an_edit() {
    let source = "a = [1]\na \\\n  [0]\np __LINE__\n";
    assert_eq!(findings(source), [(2, 2, MSG)]);
    held(source);
    let found = lint_str_with(&[&SpaceBeforeBrackets], source);
    assert_eq!((found[0].line, found[0].end_line), (2, 3));
}

#[test]
fn listed_observers_hold_every_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p $PROGRAM_NAME",
        "p caller",
        "p caller_locations",
        "p method(:f).source_location",
        "SCRIPT_LINES__ = {}",
        "p DATA.read\n__END__\nx",
    ] {
        held(&format!("@a [0]\n{observer}\n"));
    }
    assert_correction(
        &SpaceBeforeBrackets,
        "@a [0]\np File.read('example.rb')\n",
        "@a[0]\np File.read('example.rb')\n",
    );
}
