//! Examples ported from RuboCop's `spec/rubocop/cop/layout/extra_spacing_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 96
//! pinned examples (56 distinct sources) run at the fixed defaults
//! (AllowForAlignment true, AllowBeforeTrailingComments false,
//! ForceEqualSignAlignment false), whatever options their context declares:
//! findings and single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (the source and corrected SHA-256 of
//! every row were checked against it when the rows were ported).
use super::*;
use crate::cops::testing::assert_correction;

const COP: &str = "Layout/ExtraSpacing";
const MESSAGE: &str = "Unnecessary spacing detected.";

/// `(line, col)` of each finding from the full default pipeline.
fn findings(source: &str) -> Vec<(u32, u32)> {
    crate::pipeline::lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert_eq!(o.message, MESSAGE);
            assert_eq!(o.end_line, o.line);
            (o.line, o.col)
        })
        .collect()
}

#[test]
fn literal_comment_heredoc_and_data_bytes_are_not_gaps() {
    for source in [
        "",
        "# only    a comment\n",
        "=begin\n  a    b\n=end\n",
        "x = \"a    b\"\n",
        "x = 'a    b'\n",
        "x = :\"a    b\"\n",
        "x = %w[a    b]\n",
        "x = %i[a    b]\n",
        "x = /a    b/x\n",
        "x = `echo    1`\n",
        "x = %q(a    b)\n",
        "x = \"#{a}    #{b}\"\n",
        "x = <<~EOS\n  a    b\n    c    d\nEOS\n",
        "x = 1 # a    b\n",
        "x = 1\n__END__\na    b\n",
        "  x = 1\n",
        "x = 1    \n",
        "foo    \\\n  bar\n",
        // The gap after a heredoc opener is between tokens of different
        // lines in the stream (the body is lexed at the opener).
        "foo(<<~EOS  , 1)\n  x\nEOS\n",
        "x = 1\n\t\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
}

#[test]
fn real_token_gaps_in_code_interpolation_and_heredoc_lines_are_reported() {
    for (source, expected) in [
        ("x = \"#{a   + b}\"\n", vec![(1, 9)]),
        ("foo(<<~EOS,   1)\n  x\nEOS\n", vec![(1, 12)]),
        ("foo  <<~EOS\n  x\nEOS\n", vec![(1, 4)]),
        ("x\t\t= 1\n", vec![(1, 2)]),
        ("x  = 1\r\ny = 2\r\n", vec![(1, 2)]),
        ("\u{feff}é  = 1\n", vec![(1, 2)]),
        ("x = [1,   2]\n", vec![(1, 8)]),
        ("case x\nin {a:   1}\nend\n", vec![(2, 7)]),
        ("x = { a:   1 }\n", vec![(1, 9)]),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
    }
    assert_correction(&ExtraSpacing, "x  = 1\r\ny = 2\r\n", "x = 1\r\ny = 2\r\n");
    assert_correction(&ExtraSpacing, "x\t\t= 1\n", "x\t= 1\n");
    let latin1 = b"# encoding: iso-8859-1\nx = \"\xe9\"  # c\n".to_vec();
    let file = crate::source::SourceFile::new("t.rb".into(), latin1);
    let parsed = crate::parser::parse(&file.bytes);
    let mut out = Vec::new();
    ExtraSpacing.check_parse(&parsed, &file, &mut out);
    assert_eq!(
        out.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
        [(2, 8)]
    );
}

#[test]
fn multiline_hash_and_hash_pattern_pairs_belong_to_hash_alignment() {
    for source in [
        "x = {\n  a:    1,\n  bb:   2\n}\n",
        "x = {\n  'a'    =>   1,\n  'bb'   =>   2\n}\n",
        "foo(a:    1,\n    bb:   2)\n",
        "case x\nin {\n  a:    1\n}\nend\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
    // Gaps outside the key-to-value range of those pairs still count.
    assert_eq!(findings("x = {\n  a: 1,   b: 2\n}\n"), [(2, 8)]);
}

#[test]
fn alignment_lookalikes() {
    for source in [
        "a    = 1\nbbbb = 2\n",
        "a    += 1\nbbbb -= 2\n",
        "x = 1 # one\ny = 22 # two\n",
        "foo  # one\nbar  # two\n",
        "puts 1,    :a\nputs 22,   :b\nputs 333,  :c\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
    for (source, expected) in [
        ("a  = 1\n\nbbbb = 2\n", vec![(1, 2)]),
        ("foo  # one\nbar   # two\n", vec![(1, 4), (2, 4)]),
        ("def a\n  x  = 1\nend\n", vec![(2, 4)]),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
    }
}

#[test]
fn corrections_keep_one_blank_and_the_ast() {
    for (before, after) in [
        ("foo  -1\n", "foo -1\n"),
        ("foo  *args\n", "foo *args\n"),
        ("foo  ::Bar\n", "foo ::Bar\n"),
        ("foo  [1]\n", "foo [1]\n"),
        ("foo  (1)\n", "foo (1)\n"),
        ("x = a  ? b : c\n", "x = a ? b : c\n"),
        ("a = 4\nx = a  /2\n", "a = 4\nx = a /2\n"),
        ("puts  <<~EOS\n  x\nEOS\n", "puts <<~EOS\n  x\nEOS\n"),
        ("x = y  if z\n", "x = y if z\n"),
        ("p  __LINE__\n", "p __LINE__\n"),
        ("foo(a,  :b)\n", "foo(a, :b)\n"),
    ] {
        assert_correction(&ExtraSpacing, before, after);
        let (a, b) = (
            crate::parser::parse(before.as_bytes()),
            crate::parser::parse(after.as_bytes()),
        );
        assert_eq!(
            crate::cops::ast::fingerprint_ignoring_grouping(&a.node(), before.as_bytes()),
            crate::cops::ast::fingerprint_ignoring_grouping(&b.node(), after.as_bytes()),
            "{before:?}"
        );
        assert_eq!(a.warnings().count(), b.warnings().count(), "{before:?}");
    }
}

#[test]
fn upstream_pass_ordering_companions_do_not_withhold_the_edit() {
    // Upstream's autocorrect_incompatible_with only orders RuboCop's
    // correction rounds; overlapping edits are resolved by
    // corrections::apply, so the default-on Style/SelfAssignment must not
    // disable these edits.
    assert!(ExtraSpacing.autocorrect_incompatible_with().is_empty());
    let registry = crate::cops::Registry::new();
    let other = registry
        .cops()
        .iter()
        .find(|c| c.name() == "Style/SelfAssignment")
        .unwrap();
    let source = "x = 1\nx  = x + 1\nyy  = 2\n";
    let found =
        crate::pipeline::lint_str_with_corrections(&[&ExtraSpacing, other.as_ref()], source);
    let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
    assert_eq!(
        String::from_utf8(applied.bytes).unwrap(),
        "x = 1\nx += 1\nyy = 2\n"
    );
}
