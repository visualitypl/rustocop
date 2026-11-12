//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/multiline_block_layout_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 31 pinned examples
//! (including the `:ruby27` numbered-parameter and `:ruby34` `it` contexts,
//! which apply at the fixed Ruby 3.4, and the `Layout/LineLength` disabled
//! context, whose example is reported at the default `Max` too) run at the
//! fixed defaults. Findings, messages and single-pass corrected bytes equal
//! the fixed replay recorded in the pinned RuboCop reference run (each
//! source and corrected SHA-256 was checked against it when the rows were
//! ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/MultilineBlockLayout";

/// `(line, col, message)` of each finding, checking severity, the highlight's
/// last line and that no edit is offered without a correction request.
fn findings_with(cop: &MultilineBlockLayout, source: &str) -> Vec<(u32, u32, &'static str)> {
    let mut found: Vec<_> = lint_str_with(&[cop], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert!(o.end_line >= o.line);
            let message = if o.message == MSG { MSG } else { ARG_MSG };
            assert_eq!(o.message, message);
            (o.line, o.col, message)
        })
        .collect();
    found.sort();
    found
}

fn findings(source: &str) -> Vec<(u32, u32, &'static str)> {
    findings_with(&MultilineBlockLayout::default(), source)
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&MultilineBlockLayout::default(), source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&MultilineBlockLayout::default(), source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
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
    (
        5,
        "test do foo\nend\n",
        &[(1, 9, MSG)],
        "test do \n  foo\nend\n",
    ),
    (19, "test { foo\n}\n", &[(1, 8, MSG)], "test { \n  foo\n}\n"),
    (
        33,
        "test do |x| foo\nend\n",
        &[(1, 13, MSG)],
        "test do |x| \n  foo\nend\n",
    ),
    (
        47,
        "test { |x| foo\n}\n",
        &[(1, 12, MSG)],
        "test { |x| \n  foo\n}\n",
    ),
    (61, "test do foo end", &[], ""),
    (65, "test { foo }", &[], ""),
    (69, "test do\n  foo\nend\n", &[], ""),
    (
        77,
        "some_result = lambda do |\n  so_many,\n  parameters,\n  it_will,\n  be_too_long,\n  for_one_line,\n  line_length,\n  has_increased,\n  add_3_more|\n  do_something\nend\n",
        &[],
        "",
    ),
    (
        93,
        "some_result = lambda do |\n  so_many,\n  parameters,\n  it_will,\n  be_too_long,\n  for_one_line,\n  line_length,\n  has_increased,\n  add_3_mor|\n  do_something\nend\n",
        &[(1, 25, ARG_MSG)],
        "some_result = lambda do |so_many, parameters, it_will, be_too_long, for_one_line, line_length, has_increased, add_3_mor|\n  do_something\nend\n",
    ),
    (
        116,
        "some_result = lambda do\n  |so_many, parameters, it_will, be_too_long, for_one_line, line_length, has_increased, add_3_more|\n  do_something\nend\n",
        &[],
        "",
    ),
    (125, "test do |x|\nend\n", &[], ""),
    (132, "test {\n  foo\n}\n", &[], ""),
    (
        140,
        "-> (x) do foo\n  bar\nend\n",
        &[(1, 11, MSG)],
        "-> (x) do \n  foo\n  bar\nend\n",
    ),
    (
        156,
        "-> x do foo\n  bar\nend\n",
        &[(1, 9, MSG)],
        "-> x do \n  foo\n  bar\nend\n",
    ),
    (
        172,
        "test do\n  |x| play_with(x)\nend\n",
        &[(2, 3, ARG_MSG)],
        "test do |x|\n  play_with(x)\nend\n",
    ),
    (
        187,
        "test do\n  |x|\nend\n",
        &[(2, 3, ARG_MSG)],
        "test do |x|\nend\n",
    ),
    (
        201,
        "test do |x,\n  y|\nend\n",
        &[(1, 9, ARG_MSG)],
        "test do |x, y|\nend\n",
    ),
    (
        215,
        "test do |foo| bar\n  test\nend\n",
        &[(1, 15, MSG)],
        "test do |foo| \n  bar\n  test\nend\n",
    ),
    (
        232,
        "x = -> (y) { foo\n  bar\n}\n",
        &[(1, 14, MSG)],
        "x = -> (y) { \n      foo\n  bar\n}\n",
    ),
    (
        248,
        "foo do |o| (\n    bar\n  )\nend\n",
        &[(1, 12, MSG)],
        "foo do |o| \n  (\n    bar\n  )\nend\n",
    ),
    (
        266,
        "test do |x,\n  y| play_with(x, y)\nend\n",
        &[(1, 9, ARG_MSG)],
        "test do |x, y|\n  play_with(x, y)\nend\n",
    ),
    (
        281,
        "test do |(x,\n  y)| play_with(x, y)\nend\n",
        &[(1, 9, ARG_MSG)],
        "test do |(x, y)|\n  play_with(x, y)\nend\n",
    ),
    (
        297,
        "def f\n  X.map do |(a,\n  b)|\n  end\nend\n",
        &[(2, 12, ARG_MSG)],
        "def f\n  X.map do |(a, b)|\n  end\nend\n",
    ),
    (
        315,
        "def f\n  X.map do |\n    a,\n  |\n  end\nend\n",
        &[(2, 12, ARG_MSG)],
        "def f\n  X.map do |a,|\n  end\nend\n",
    ),
    (
        334,
        "foo { |key,\n  bar:, **|\n  baz\n}\n",
        &[(1, 7, ARG_MSG)],
        "foo { |key, bar:, **|\n  baz\n}\n",
    ),
    (
        350,
        "def f\n  X.map do |\n    (((a), b), c)\n  |\n  end\nend\n",
        &[(2, 12, ARG_MSG)],
        "def f\n  X.map do |(((a), b), c)|\n  end\nend\n",
    ),
    (
        374,
        "test do\n|(x, y)|\n  play_with(x, y)\nend\n",
        &[(2, 1, ARG_MSG)],
        "test do |(x, y)|\n  play_with(x, y)\nend\n",
    ),
    (392, "test { _1\n}\n", &[(1, 8, MSG)], "test { \n  _1\n}\n"),
    (
        406,
        "test do _1\nend\n",
        &[(1, 9, MSG)],
        "test do \n  _1\nend\n",
    ),
    (422, "test { it\n}\n", &[(1, 8, MSG)], "test { \n  it\n}\n"),
    (
        436,
        "test do it\nend\n",
        &[(1, 9, MSG)],
        "test do \n  it\nend\n",
    ),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 31);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&MultilineBlockLayout::default(), source).is_empty());
        } else {
            assert_correction(&MultilineBlockLayout::default(), source, corrected);
        }
    }
}

#[test]
fn highlights_span_the_whole_expression() {
    let found = lint_str_with(
        &[&MultilineBlockLayout::default()],
        "test do |foo| bar(1,\n  2)\nend\n",
    );
    assert_eq!((found[0].line, found[0].end_line), (1, 2));
    let found = lint_str_with(
        &[&MultilineBlockLayout::default()],
        "test do |x,\n  y|\nend\n",
    );
    assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 9, 2));
}

#[test]
fn single_line_blocks_and_lookalikes_are_accepted() {
    for source in [
        "test { |x| foo(x) }\n",
        "test do |x| foo(x) end\n",
        "test(&block)\n",
        "test do ||\n  foo\nend\n",
        "test {\n}\n",
        "test do\nend\n",
        "test do |x|\n  foo\nend\n",
        "test do |x, y|\n  foo\nend\n",
        "-> (x) do\n  foo\nend\n",
        "->(\n  x\n) do\n  foo\nend\n",
        "-> {\n  foo\n}\n",
        "lambda do |x|\n  foo\nend\n",
        "super do |x|\n  foo\nend\n",
        "def f(&)\n  g(&)\nend\n",
        "foo { bar }.baz do |x|\n  x\nend\n",
        "test do\n  foo\nrescue\n  bar\nend\n",
        "for x in y do foo\nend\n",
        "while x do foo\nend\n",
        "BEGIN { foo\n}\n",
    ] {
        assert_eq!(findings(source), [], "{source}");
    }
}

#[test]
fn every_parser_block_kind_is_checked() {
    for (source, expected) in [
        ("super do foo\nend\n", (1, 10, MSG)),
        ("super() { |x| foo\n}\n", (1, 15, MSG)),
        ("a.b.c(1) { foo\n}\n", (1, 12, MSG)),
        ("a&.b do foo\nend\n", (1, 9, MSG)),
        ("-> do foo\nend\n", (1, 7, MSG)),
        ("test do foo\nrescue\n  bar\nend\n", (1, 9, MSG)),
        ("test do foo; bar\nend\n", (1, 9, MSG)),
        ("test do\n  |x|\nend\n", (2, 3, ARG_MSG)),
        ("test do |x;\n  y|\nend\n", (1, 9, ARG_MSG)),
        ("test do |*,\n  **|\nend\n", (1, 9, ARG_MSG)),
        ("test do |a, &\n  b|\nend\n", (1, 9, ARG_MSG)),
        ("test { |;\n  y|\n}\n", (1, 8, ARG_MSG)),
        ("foo(\n  1\n) { bar\n}\n", (3, 5, MSG)),
    ] {
        assert_eq!(findings(source), [expected], "{source}");
    }
}

/// Rubocop-ast's `BlockNode#single_line?` compares the lines of
/// `loc.begin` and `loc.end`, not the call's start. A one-line block after a
/// multi-line receiver chain or argument list is single-line (each row was
/// checked against the pinned Team, source only).
#[test]
fn one_line_blocks_after_multi_line_receivers_or_arguments_are_single_line() {
    for source in [
        "result = client\n  .fetch(id)\n  .map { |r| r + 1 }\n",
        "result = client\n  .fetch(id)\n  .map do |r| r + 1 end\n",
        "result = client\n  &.fetch(id)\n  &.map { _1 + 1 }\n",
        "client\n  .fetch(id)\n  .each { it }\n",
        "client\r\n  .fetch(id)\r\n  .map { |r| r + 1 }\r\n",
        "\u{feff}client\n  .fetch(id)\n  .map { |r| r }\n",
        "client # c\n  .fetch(id) # d\n  .map { |r| r + 1 } # e\n",
        "client\n  .fetch(<<~A)\n  x\nA\n  .map { |r| r }\n",
        "def run\n  items\n    .select(&:ok?)\n    .each { |i| process(i) }\nend\n",
        "x = { a: list\n  .map { |v| v } }\n",
        "foo(list\n  .map { |v| v })\n",
        "list\n  .map { |v| v }\n  .each do |w| w end\n",
        "foo(\n  1\n) { bar }\n",
        "foo bar,\n  baz do |x| x end\n",
        "super(a,\n  b) { |x| x }\n",
        "-> (\n  x) { }\n",
        "->(a,\n  b) do a end\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
        assert!(
            correction_offenses(&MultilineBlockLayout::default(), source).is_empty(),
            "{source:?}"
        );
    }
}

/// A block that genuinely spans lines at the end of a multi-line
/// receiver chain is still reported and corrected as the pinned Team does.
#[test]
fn multi_line_blocks_after_multi_line_receivers_are_still_reported() {
    for (source, expected, after) in [
        (
            "result = client\n  .fetch(id)\n  .map { |r| r +\n  1\n}\n",
            &[(3, 14, MSG)][..],
            "result = client\n  .fetch(id)\n  .map { |r| \n           r +\n  1\n}\n",
        ),
        (
            "result = client\n  .fetch(id)\n  .map do |r| r + 1\nend\n",
            &[(3, 15, MSG)],
            "result = client\n  .fetch(id)\n  .map do |r| \n           r + 1\nend\n",
        ),
        (
            "client\n  .fetch(id)\n  .map { _1 +\n  1 }\n",
            &[(3, 10, MSG)],
            "client\n  .fetch(id)\n  .map { \n  _1 +\n  1 }\n",
        ),
        (
            "client\n  .fetch(id)\n  .map { it +\n  1 }\n",
            &[(3, 10, MSG)],
            "client\n  .fetch(id)\n  .map { \n  it +\n  1 }\n",
        ),
        (
            "client\n  .fetch(id)\n  .map do |a,\n  b| a end\n",
            &[(3, 11, ARG_MSG)],
            "client\n  .fetch(id)\n  .map do |a, b|\n  a end\n",
        ),
        (
            "list\n  .map { |v| v }\n  .each do |w| w\n  end\n",
            &[(3, 16, MSG)],
            "list\n  .map { |v| v }\n  .each do |w| \n  w\n  end\n",
        ),
        (
            "-> (\n  x) { x\n}\n",
            &[(2, 8, MSG)],
            "-> (\n  x) { \n  x\n}\n",
        ),
        (
            "client\r\n  .fetch(id)\r\n  .map { |r| r +\r\n  1\r\n}\r\n",
            &[(3, 14, MSG)],
            "client\r\n  .fetch(id)\r\n  .map { |r| \r\n  r +\r\n  1\r\n}\r\n",
        ),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        assert_correction(&MultilineBlockLayout::default(), source, after);
    }
}

#[test]
fn line_length_decides_whether_split_arguments_are_necessary() {
    let long = "some_result = lambda do |\n  so_many,\n  parameters,\n  it_will,\n  be_too_long,\n  for_one_line,\n  line_length,\n  has_increased,\n  add_3_more|\n  do_something\nend\n";
    assert_eq!(findings(long), []);
    // Disabled `Layout/LineLength` (no `Max`): never necessary.
    let unlimited = MultilineBlockLayout {
        max_line_length: None,
    };
    assert_eq!(findings_with(&unlimited, long), [(1, 25, ARG_MSG)]);
    // A configured `Max`.
    let short = MultilineBlockLayout {
        max_line_length: Some(20),
    };
    let source = "foo do |\n  a,\n  b|\n  a\nend\n";
    assert_eq!(findings(source), [(1, 8, ARG_MSG)]);
    // 0 + 1 (the first line ends in "|") + 8 ("foo do |") + 4 ("a, b") = 13.
    assert_eq!(findings_with(&short, source), [(1, 8, ARG_MSG)]);
    let tiny = MultilineBlockLayout {
        max_line_length: Some(12),
    };
    assert_eq!(findings_with(&tiny, source), []);
    // Without a pipe ending the first line, three characters are added.
    let source = "foo do\n  |a, b|\n  a\nend\n";
    let fits = MultilineBlockLayout {
        max_line_length: Some(13),
    };
    assert_eq!(findings_with(&fits, source), [(2, 3, ARG_MSG)]);
    let needed = MultilineBlockLayout {
        max_line_length: Some(12),
    };
    assert_eq!(findings_with(&needed, source), []);
    // Characters, not bytes, and the block's own column count.
    // 2 + 3 + 8 ("é.foo do") + 1 = 14.
    let source = "  é.foo do\n  |a|\nend\n";
    let fits = MultilineBlockLayout {
        max_line_length: Some(14),
    };
    assert_eq!(findings_with(&fits, source), [(2, 3, ARG_MSG)]);
    let needed = MultilineBlockLayout {
        max_line_length: Some(13),
    };
    assert_eq!(findings_with(&needed, source), []);
}

#[test]
fn line_length_follows_the_resolved_configuration() {
    let registry = crate::cops::Registry::new();
    let resolve = |toml: &str| {
        crate::config::ResolvedConfig::resolve(toml::from_str(toml).unwrap(), None, &registry)
            .unwrap()
    };
    let long = "some_result = lambda do |\n  so_many,\n  parameters,\n  it_will,\n  be_too_long,\n  for_one_line,\n  line_length,\n  has_increased,\n  add_3_more|\n  do_something\nend\n";
    for (toml, expected) in [
        ("", 0),
        ("[cops.\"Layout/LineLength\"]\nenabled = false\n", 1),
        ("[cops.\"Layout/LineLength\"]\nmax = 200\n", 1),
        (
            "[cops.Layout]\nenabled = false\n[cops.\"Layout/MultilineBlockLayout\"]\nenabled = true\n",
            1,
        ),
    ] {
        let config = resolve(toml);
        let cop = config.configured.get(COP).unwrap();
        let found = crate::pipeline::lint_str_with(&[cop.as_ref()], long);
        assert_eq!(found.len(), expected, "{toml}");
    }
}

#[test]
fn corrections_keep_bodies_and_parameters_exact() {
    for (before, after) in [
        (
            "test do foo\nrescue\n  bar\nend\n",
            "test do \n  foo\nrescue\n  bar\nend\n",
        ),
        ("test do foo; bar\nend\n", "test do \n  foo; bar\nend\n"),
        (
            "  a.b do |x| foo\n  end\n",
            "  a.b do |x| \n    foo\n  end\n",
        ),
        (
            "test do |a = 1,\n  *b, c:, d: 2, **e, &f|\nend\n",
            "test do |a = 1, *b, c:, d: 2, **e, &f|\nend\n",
        ),
        (
            "test do |a,\n  (b, *c), d|\nend\n",
            "test do |a, (b, *c), d|\nend\n",
        ),
        ("test do |a \\\n  , b|\nend\n", "test do |a, b|\nend\n"),
        ("test do\n  |x|\n  foo\nend\n", "test do |x|\n  foo\nend\n"),
        (
            "test do |x,\n  y| ; foo\nend\n",
            "test do |x, y|; \n  foo\nend\n",
        ),
        (
            "test do |a = \"x,\n  y\"|\nend\n",
            "test do |a = \"x,\n  y\"|\nend\n",
        ),
        ("foo(\n  1\n) { bar\n}\n", "foo(\n  1\n) { \n  bar\n}\n"),
    ] {
        assert_eq!(fixed(before), after, "{before}");
        if before != after {
            assert_correction(&MultilineBlockLayout::default(), before, after);
        }
    }
}

#[test]
fn inexact_or_unsafe_rewrites_are_withheld() {
    for source in [
        // Block-local variables would become parameters.
        "test do |x;\n  y|\nend\n",
        "test { |;\n  y|\n}\n",
        // Comments between the parameters would be dropped.
        "test do |x, # first\n  y|\nend\n",
        "test do # note\n  |x|\nend\n",
        // Upstream's comma rule disagrees with Ruby's trailing comma.
        "test do |a, b,\n  |\nend\n",
        "test do |a = [1,\n  2]|\nend\n",
        "test do |(a),\n  |\nend\n",
        // A heredoc in the parameters.
        "test do |a = <<~X,\n  text\nX\n  b|\nend\n",
        // The inserted line break would follow a heredoc opener.
        "test(<<~X) { foo\n  text\nX\n}\n",
        // A directive on a touched line.
        "test do foo # rubocop:disable Lint/Debugger\nend\n",
        "test do # rustocop:disable Style\n  |x|\nend\n",
        // the listed source observers.
        "test do foo(__FILE__)\nend\n",
        "test do foo(__dir__)\nend\n",
        "test do foo($0)\nend\n",
        "test do foo($PROGRAM_NAME)\nend\n",
        "test do foo(caller)\nend\n",
        "test do caller_locations\nend\n",
        "test do m.source_location\nend\n",
        "test do SCRIPT_LINES__\nend\n",
        "test do DATA.read\nend\n__END__\nx\n",
    ] {
        held(source);
    }
}

#[test]
fn line_values_are_held_by_the_shared_guard() {
    held("test do foo\nend\np __LINE__\n");
    assert_eq!(
        fixed("p __LINE__\ntest do foo\nend\n"),
        "p __LINE__\ntest do \n  foo\nend\n"
    );
}

#[test]
fn upstream_pass_ordering_peers_are_not_declared() {
    assert!(
        MultilineBlockLayout::default()
            .autocorrect_incompatible_with()
            .is_empty()
    );
    for peer in [
        "Style/HashEachMethods",
        "Style/ItBlockParameter",
        "Style/Lambda",
    ] {
        let registry = crate::cops::Registry::new();
        let other = registry.cops().iter().find(|c| c.name() == peer).unwrap();
        let found = crate::pipeline::lint_str_with_corrections(
            &[&MultilineBlockLayout::default(), other.as_ref()],
            "test do foo\nend\n",
        );
        let own: Vec<_> = found.iter().filter(|o| o.cop == COP).collect();
        assert_eq!(own.len(), 1, "{peer}");
        assert!(own[0].correction.is_some(), "{peer}");
    }
}

#[test]
fn crlf_and_bom_sources_report_character_positions() {
    assert_eq!(findings("test do foo\r\nend\r\n"), [(1, 9, MSG)]);
    assert_eq!(findings("\u{feff}test do foo\nend\n"), [(1, 9, MSG)]);
    assert_eq!(findings("é.test do foo\nend\n"), [(1, 11, MSG)]);
}

/// `(exact, upstream string)` of the first block's parameters in raw bytes.
fn arg_string(source: &[u8]) -> (bool, Vec<u8>) {
    struct Find<'f> {
        file: &'f SourceFile,
        found: Option<(bool, Vec<u8>)>,
    }
    impl<'pr> NodeVisitor<'pr> for Find<'_> {
        fn enter(&mut self, node: Node<'pr>) {
            if self.found.is_none() && node.as_block_parameters_node().is_some() {
                let string = ArgString::of(&node, self.file);
                self.found = Some((string.exact(), string.upstream.into_bytes()));
            }
        }
    }
    let file = SourceFile::new("t.rb".into(), source.to_vec());
    let parsed = crate::parser::parse(source);
    assert_eq!(parsed.errors().count(), 0, "{source:?}");
    let mut find = Find {
        file: &file,
        found: None,
    };
    walk(&mut find, &parsed.node());
    find.found.unwrap()
}

#[test]
fn parameter_bytes_that_are_not_utf8_make_the_rewrite_inexact() {
    // Each kept parameter source is copied byte for byte or not at all: a
    // BINARY or legacy-encoded byte that is not UTF-8 (in a positional,
    // keyword or destructured default, or in a raw identifier) would be
    // replaced by U+FFFD, changing the literal or the variable.
    for source in [
        b"# coding: BINARY\np(test do |x = \"\xff\",\n  y|\n  [x,y]\nend)\n".as_slice(),
        b"# coding: BINARY\ntest do |k: \"\xfe\x80\",\n  y: 2|\nend\n",
        b"# coding: BINARY\ntest do |(a, b), c = :\xff,\n  d|\nend\n",
        b"# coding: BINARY\ntest do |a\xff,\n  b|\n  a\xff\nend\n",
        b"# coding: BINARY\ntest do |*r\xff, **k\xff,\n  &b\xff|\nend\n",
        b"# encoding: iso-8859-1\ntest do |x = \"\xe9\",\n  y|\nend\n",
    ] {
        assert!(!arg_string(source).0, "{source:?}");
    }
    // ASCII and valid UTF-8 parameter bytes keep the canonical join, in a
    // BINARY file too, with the bytes as written.
    for (source, upstream) in [
        (
            b"# coding: BINARY\ntest do |x = \"a\",\n  y|\n  p \"\xff\"\nend\n".as_slice(),
            b"x = \"a\", y".as_slice(),
        ),
        (
            b"# coding: BINARY\ntest do |x = \"\xc3\xa9\",\n  y|\nend\n",
            b"x = \"\xc3\xa9\", y",
        ),
        (
            "test do |é = 1,\n  y|\nend\n".as_bytes(),
            "é = 1, y".as_bytes(),
        ),
    ] {
        assert_eq!(arg_string(source), (true, upstream.to_vec()), "{source:?}");
    }
}
