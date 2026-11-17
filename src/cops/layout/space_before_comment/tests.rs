//! Examples ported from RuboCop's `spec/rubocop/cop/layout/space_before_comment_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All five
//! pinned examples run at the fixed defaults; findings, messages and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str,
};

const COP: &str = "Layout/SpaceBeforeComment";
const MESSAGE: &str = "Put a space before an end-of-line comment.";

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

fn held(source: &str) {
    let found = correction_offenses(&SpaceBeforeComment, source);
    assert_eq!(found.len(), 1, "{source:?}");
    assert!(found[0].correction.is_none(), "{source:?}");
}

#[test]
fn pinned_examples() {
    // spec line 5
    assert_eq!(positions("a += 1# increment\n"), [(1, 7)]);
    assert_correction(
        &SpaceBeforeComment,
        "a += 1# increment\n",
        "a += 1 # increment\n",
    );
    // spec lines 16, 20, 24
    for source in [
        "a += 1 # increment",
        "# comment",
        "=begin\nDoc comment\n=end\n",
    ] {
        assert!(positions(source).is_empty(), "{source:?}");
    }
    // spec line 32: the heredoc opener is the token before the comment.
    let heredoc = "<<~STR# my string\n  text\nSTR\n";
    assert_eq!(positions(heredoc), [(1, 7)]);
    assert_correction(
        &SpaceBeforeComment,
        heredoc,
        "<<~STR # my string\n  text\nSTR\n",
    );
}

#[test]
fn every_token_kind_touching_a_comment() {
    for (before, after, at) in [
        ("x = [1,#c\n2]\n", "x = [1, #c\n2]\n", (1, 8)),
        ("a;#c\n", "a; #c\n", (1, 3)),
        ("def f\nend#c\n", "def f\nend #c\n", (2, 4)),
        ("f(1)#c\n", "f(1) #c\n", (1, 5)),
        ("x = \"a#b\"#c\n", "x = \"a#b\" #c\n", (1, 10)),
        ("x = /a/i#c\n", "x = /a/i #c\n", (1, 9)),
        ("x = ?a#c\n", "x = ?a #c\n", (1, 7)),
        ("x = %w(a b)#c\n", "x = %w(a b) #c\n", (1, 12)),
        // A blank closing delimiter is the end of the literal token.
        ("x = %q x #c\n", "x = %q x  #c\n", (1, 10)),
        ("x = \"é\"#c\n", "x = \"é\" #c\n", (1, 8)),
        ("x = 1#c\r\ny = 2\r\n", "x = 1 #c\r\ny = 2\r\n", (1, 6)),
        ("\u{feff}x = 1#c\n", "\u{feff}x = 1 #c\n", (1, 6)),
        ("a#1\nb#2\n", "a #1\nb #2\n", (1, 2)),
    ] {
        assert_eq!(positions(before)[0], at, "{before:?}");
        assert_correction(&SpaceBeforeComment, before, after);
    }
    assert_eq!(positions("a#1\nb#2\n"), [(1, 2), (2, 2)]);
}

#[test]
fn blanks_line_starts_boms_and_non_comments_are_accepted() {
    for source in [
        "",
        "#c\n",
        "  #c\n",
        "x = 1\t#c\n",
        "x = 1 \t #c\n",
        "\u{feff}#c\n",
        "\u{feff}# frozen_string_literal: true\nx = 1\n",
        "x = \"a#b\"\n",
        "x = 'a#b'\n",
        "x = %q(#a)\n",
        "x = /#a/\n",
        "x = ?#\n",
        "x = <<~H\n  a#b\nH\n",
        "x = 1\n=begin\na#b\n=end\n",
        "x = 1\n__END__\na#b\n",
        "x = 1 # a#b\n",
    ] {
        assert!(positions(source).is_empty(), "{source:?}");
    }
}

#[test]
fn interpolation_code_in_a_heredoc_is_code() {
    let before = "x = <<~H\n  #{y#c\n  }\nH\n";
    assert_eq!(positions(before), [(2, 6)]);
    assert_correction(&SpaceBeforeComment, before, "x = <<~H\n  #{y #c\n  }\nH\n");
}

#[test]
fn sign_sensitive_operators_keep_the_finding_without_an_edit() {
    // `foo -#c\n1` is `foo(-1)`; `foo - #c\n1` would be `foo - 1`.
    for source in [
        "foo -#c\n1\n",
        "foo +#c\n1\n",
        "foo *#c\nargs\n",
        "foo &#c\nblock\n",
        "foo **#c\nopts\n",
    ] {
        assert_eq!(positions(source).len(), 1, "{source:?}");
        held(source);
    }
}

#[test]
fn directives_in_both_spellings_disable_the_finding_on_its_line() {
    for prefix in ["rubocop", "rustocop"] {
        let source = format!("a# {prefix}:disable Layout/SpaceBeforeComment\nb#c\n");
        assert_eq!(positions(&source), [(2, 2)], "{source:?}");
    }
}
