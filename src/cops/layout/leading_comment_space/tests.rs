//! Examples ported from RuboCop's `spec/rubocop/cop/layout/leading_comment_space_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 33
//! pinned examples run at the fixed defaults (every optional annotation
//! allowance false): the contexts enabling `AllowDoxygenCommentStyle`,
//! `AllowGemfileRubyComment`, `AllowRBSInlineAnnotation`,
//! `AllowSteepAnnotation` or `AllowYARDCommentBlockSeparator` are replayed at
//! those defaults, where their sources are offenses. Findings, messages and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::{lint_path_with_corrections, lint_str},
};

const COP: &str = "Layout/LeadingCommentSpace";
const MESSAGE: &str = "Missing space after `#`.";

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

fn path_positions(path: &str, source: &str) -> Vec<(u32, u32)> {
    lint_path_with_corrections(&[&LeadingCommentSpace], path, source)
        .into_iter()
        .map(|o| (o.line, o.col))
        .collect()
}

fn path_fixed(path: &str, source: &str) -> String {
    let offenses = lint_path_with_corrections(&[&LeadingCommentSpace], path, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// `(spec line, source, fixed-default findings, single-pass correction)`.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const RUBY_COMMENT: &str = "# Specific version (comment) will be used by RVM\n#ruby=2.7.0\n#ruby-gemset=myproject\nruby '~> 2.7.0'\n";
const RUBY_COMMENT_FIXED: &str = "# Specific version (comment) will be used by RVM\n# ruby=2.7.0\n# ruby-gemset=myproject\nruby '~> 2.7.0'\n";
const RBS: &str = "include Enumerable #[Integer]\n\nattr_reader :name #: String\nattr_reader :age  #: Integer?\n\n#: (\n#|   Integer,\n#|   String\n#| ) -> void\ndef foo; end\n";
const RBS_FIXED: &str = "include Enumerable # [Integer]\n\nattr_reader :name # : String\nattr_reader :age  # : Integer?\n\n# : (\n# |   Integer,\n# |   String\n# | ) -> void\ndef foo; end\n";
const RBS_AT: &[(u32, u32)] = &[(1, 20), (3, 19), (4, 19), (6, 1), (7, 1), (8, 1), (9, 1)];
const STEEP: &str = "[1, 2, 3].each_with_object([]) do |n, list| #$ Array[Integer]\n  list << n\nend\n\nname = 'John'      #: String\n";
const STEEP_FIXED: &str = "[1, 2, 3].each_with_object([]) do |n, list| # $ Array[Integer]\n  list << n\nend\n\nname = 'John'      # : String\n";
const DOXYGEN: &str = "#**\n# Some comment\n# Another comment on a second line\n#*\n";
const DOXYGEN_FIXED: &str = "# **\n# Some comment\n# Another comment on a second line\n# *\n";
const YARD: &str = "# Some comment\n#-\nclass Foo\nend\n";
const YARD_FIXED: &str = "# Some comment\n# -\nclass Foo\nend\n";

/// Every pinned example on the default path `example.rb`.
const PINNED: &[Pinned] = &[
    (5, "#missing space\n", &[(1, 1)], "# missing space\n"),
    (16, "#", &[], ""),
    (20, "#   heavily indented", &[], ""),
    (24, "###### heavily indented", &[], ""),
    (28, "######", &[], ""),
    (32, "#!/usr/bin/ruby\ntest\n", &[], ""),
    (
        39,
        "#!/usr/bin/env nix-shell\n#! nix-shell -i ruby --pure\n#! nix-shell -p ruby gh git\ntest\n",
        &[],
        "",
    ),
    (
        48,
        "test\n#!/usr/bin/ruby\n",
        &[(2, 1)],
        "test\n# !/usr/bin/ruby\n",
    ),
    (
        61,
        "test\n#!/usr/bin/env nix-shell\n#! nix-shell -i ruby --pure\n#! nix-shell -p ruby gh git\n",
        &[(2, 1), (3, 1), (4, 1)],
        "test\n# !/usr/bin/env nix-shell\n# ! nix-shell -i ruby --pure\n# ! nix-shell -p ruby gh git\n",
    ),
    (201, "#= require_tree .", &[], ""),
    (205, "=begin\n#blahblah\n=end\n", &[], ""),
    (
        88,
        "test\n#\\ -w -p 8765\n",
        &[(2, 1)],
        "test\n# \\ -w -p 8765\n",
    ),
    (
        103,
        "#\\ -w -p 8765\ntest\n",
        &[(1, 1)],
        "# \\ -w -p 8765\ntest\n",
    ),
    (
        116,
        "test\n#\\ -w -p 8765\n",
        &[(2, 1)],
        "test\n# \\ -w -p 8765\n",
    ),
    (134, DOXYGEN, &[(1, 1), (4, 1)], DOXYGEN_FIXED),
    // AllowDoxygenCommentStyle: true, replayed at the fixed false.
    (156, DOXYGEN, &[(1, 1), (4, 1)], DOXYGEN_FIXED),
    (168, "#++\n#--\n", &[], ""),
    (175, "#+\n#-\n", &[(1, 1), (2, 1)], "# +\n# -\n"),
    (189, "#:nodoc:\n", &[(1, 1)], "# :nodoc:\n"),
    (217, RUBY_COMMENT, &[(2, 1), (3, 1)], RUBY_COMMENT_FIXED),
    (276, RBS, RBS_AT, RBS_FIXED),
    // AllowRBSInlineAnnotation: true, replayed at the fixed false.
    (315, RBS, RBS_AT, RBS_FIXED),
    (336, STEEP, &[(1, 45), (5, 20)], STEEP_FIXED),
    // AllowSteepAnnotation: true, replayed at the fixed false.
    (360, STEEP, &[(1, 45), (5, 20)], STEEP_FIXED),
    (376, YARD, &[(2, 1)], YARD_FIXED),
    // AllowYARDCommentBlockSeparator: true (lines 397-440), replayed at the
    // fixed false.
    (397, YARD, &[(2, 1)], YARD_FIXED),
    (
        406,
        "# Some comment\n#-\n\n# Another comment\n#-\nclass Foo\nend\n",
        &[(2, 1), (5, 1)],
        "# Some comment\n# -\n\n# Another comment\n# -\nclass Foo\nend\n",
    ),
    (
        418,
        "class Foo\n  # Some comment\n  #-\n  def bar\n  end\nend\n",
        &[(3, 3)],
        "class Foo\n  # Some comment\n  # -\n  def bar\n  end\nend\n",
    ),
    (429, "#-foo\n", &[(1, 1)], "# -foo\n"),
    (440, "#+\n", &[(1, 1)], "# +\n"),
];

#[test]
fn pinned_examples_at_fixed_defaults() {
    for &(spec_line, source, at, after) in PINNED {
        assert_eq!(positions(source), at, "spec line {spec_line}: {source:?}");
        if !at.is_empty() {
            assert_correction(&LeadingCommentSpace, source, after);
        }
    }
    assert_eq!(PINNED.len(), 30);
}

#[test]
fn pinned_path_examples() {
    // spec line 81: `#\` on line 1 of a file named config.ru.
    let rackup = "#\\ -w -p 8765\ntest\n";
    assert!(path_positions("config.ru", rackup).is_empty());
    // spec line 240: AllowGemfileRubyComment: true on test/test_case.rb.
    assert_eq!(
        path_positions("test/test_case.rb", RUBY_COMMENT),
        [(2, 1), (3, 1)]
    );
    assert_eq!(
        path_fixed("test/test_case.rb", RUBY_COMMENT),
        RUBY_COMMENT_FIXED
    );
    // spec line 260: AllowGemfileRubyComment: true on Gemfile, replayed at the
    // fixed false: the `#ruby` comments are offenses there too.
    assert_eq!(path_positions("Gemfile", RUBY_COMMENT), [(2, 1), (3, 1)]);
    assert_eq!(path_fixed("Gemfile", RUBY_COMMENT), RUBY_COMMENT_FIXED);
}

#[test]
fn rackup_options_only_on_line_one_of_a_config_ru_basename() {
    assert!(path_positions("app/config.ru", "#\\ -p 1\n").is_empty());
    assert_eq!(
        path_positions("app/config.ru", "run App\n#\\ -p 1\n"),
        [(2, 1)]
    );
    assert_eq!(path_positions("app/config.rb", "#\\ -p 1\n"), [(1, 1)]);
    assert_eq!(path_positions("config.ru.rb", "#\\ -p 1\n"), [(1, 1)]);
    assert_eq!(
        path_fixed("config.ru", "run App\n#\\ -p 1\n"),
        "run App\n# \\ -p 1\n"
    );
}

#[test]
fn shebang_continuation_follows_the_previous_line_comment() {
    // A BOM does not move the first line.
    assert!(positions("\u{feff}#!/usr/bin/env ruby\nx\n").is_empty());
    // An accepted trailing `#!` on line 1 starts a continuation, as upstream.
    assert!(positions("x #!a\n#!b\n").is_empty());
    // A comment line that is not a shebang, or no comment, breaks it.
    assert_eq!(positions("#!/a\n# x\n#!b\n"), [(3, 1)]);
    assert_eq!(positions("#!/a\n\n#!b\n"), [(3, 1)]);
    assert_eq!(positions("#!/a\nx\n#!b\n"), [(3, 1)]);
    // A reported shebang never continues one.
    assert_eq!(positions("x\n#!a\n#!b\n#!c\n"), [(2, 1), (3, 1), (4, 1)]);
    // The continuation chain follows accepted lines only.
    assert_eq!(positions("#!/a\n#!b\n#c\n#!d\n"), [(3, 1), (4, 1)]);
}

#[test]
fn matcher_boundaries() {
    for source in [
        "#\n",
        "# x\n",
        "#\tx\n",
        "#\x0bx\n",
        "#\x0cx\n",
        "#\r\n",
        "##\n",
        "## x\n",
        "#=x\n",
        "##=x\n",
        "#++x\n",
        "#+++\n",
        "#---\n",
        "#--x\n",
        "x = 1 # a\n",
        "x = \"#a\"\n",
        "x = <<~H\n  #a\nH\n",
        "x = 1\n__END__\n#a\n",
        "=begin\n#a\n=end\n",
    ] {
        assert!(positions(source).is_empty(), "{source:?}");
    }
    for (before, after, at) in [
        ("##a\n", "# #a\n", (1, 1)),
        ("#+-\n", "# +-\n", (1, 1)),
        ("#-+\n", "# -+\n", (1, 1)),
        ("#é\n", "# é\n", (1, 1)),
        ("x = \"é\" #a\n", "x = \"é\" # a\n", (1, 9)),
        ("#a\r\nx\r\n", "# a\r\nx\r\n", (1, 1)),
        (
            "x = <<~H\n  #{y #a\n  }\nH\n",
            "x = <<~H\n  #{y # a\n  }\nH\n",
            (2, 7),
        ),
        ("\u{feff}#a\n", "\u{feff}# a\n", (1, 1)),
    ] {
        assert_eq!(positions(before), [at], "{before:?}");
        assert_correction(&LeadingCommentSpace, before, after);
    }
}

/// Prism's magic comments as `(key, value)` pairs.
type MagicComments = Vec<(Vec<u8>, Vec<u8>)>;

/// Prism's magic comments, the effective encoding and the frozen setting.
fn magic(source: &str) -> (MagicComments, bool, Option<bool>) {
    let file = SourceFile::new("magic.rb".into(), source.as_bytes().to_vec());
    let result = crate::parser::parse(&file.bytes);
    file.record_encoding(&result);
    let comments = result
        .magic_comments()
        .map(|m| (m.key().to_vec(), m.value().to_vec()))
        .collect();
    (
        comments,
        file.binary_encoding(),
        file.frozen_string_setting(),
    )
}

#[test]
fn corrected_magic_comments_keep_their_meaning() {
    for (before, after) in [
        (
            "#frozen_string_literal: true\nx = 'a'\n",
            "# frozen_string_literal: true\nx = 'a'\n",
        ),
        (
            "#frozen_string_literal: false\nx = 'a'\n",
            "# frozen_string_literal: false\nx = 'a'\n",
        ),
        (
            "#encoding: ascii-8bit\nx = 'a'\n",
            "# encoding: ascii-8bit\nx = 'a'\n",
        ),
        (
            "#coding: binary\n#frozen_string_literal: true\n",
            "# coding: binary\n# frozen_string_literal: true\n",
        ),
        (
            "#-*- coding: binary -*-\nx = 'a'\n",
            "# -*- coding: binary -*-\nx = 'a'\n",
        ),
        (
            "#!/usr/bin/env ruby\n#encoding: binary\nx = 'a'\n",
            "#!/usr/bin/env ruby\n# encoding: binary\nx = 'a'\n",
        ),
        (
            "#shareable_constant_value: literal\nX = [1]\n",
            "# shareable_constant_value: literal\nX = [1]\n",
        ),
    ] {
        assert_correction(&LeadingCommentSpace, before, after);
        let (old, new) = (magic(before), magic(after));
        assert!(!old.0.is_empty(), "{before:?}");
        assert_eq!(old, new, "{before:?}");
    }
    assert!(magic("#encoding: ascii-8bit\n").1);
    assert_eq!(magic("#frozen_string_literal: true\n").2, Some(true));
    // The line-1 shebang itself is never edited.
    assert!(correction_offenses(&LeadingCommentSpace, "#!ruby -w\n").is_empty());
}

#[test]
fn corrected_directives_keep_their_meaning_in_both_spellings() {
    for prefix in ["rubocop", "rustocop"] {
        // The cop's own finding on a directive line it disables is suppressed.
        let own = format!("#{prefix}:disable Layout/LeadingCommentSpace\n#a\n");
        assert!(positions(&own).is_empty(), "{own:?}");
        let before = format!("#{prefix}:disable Lint/Debugger\nbinding.pry\n");
        let after = format!("# {prefix}:disable Lint/Debugger\nbinding.pry\n");
        assert_eq!(positions(&before), [(1, 1)]);
        assert_correction(&LeadingCommentSpace, &before, &after);
        for source in [&before, &after] {
            assert!(
                lint_str(source).iter().all(|o| o.cop != "Lint/Debugger"),
                "{source:?}"
            );
        }
        // A commented-out directive stays commented out.
        let commented = format!("##{prefix}:disable Lint/Debugger\nbinding.pry\n");
        assert_correction(
            &LeadingCommentSpace,
            &commented,
            &format!("# #{prefix}:disable Lint/Debugger\nbinding.pry\n"),
        );
        for source in [
            commented.clone(),
            format!("# #{prefix}:disable Lint/Debugger\nbinding.pry\n"),
        ] {
            assert!(
                lint_str(&source).iter().any(|o| o.cop == "Lint/Debugger"),
                "{source:?}"
            );
        }
    }
}
