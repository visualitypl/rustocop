//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_around_keyword_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 118 pinned examples
//! (the shared `missing before`/`missing after`/`accepts` examples carry
//! their shared definition's line) are listed; findings, messages and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported). The
//! `Layout/SpaceBeforeBlockBraces: no_space` context is replayed at that
//! cop's fixed `space`, as recorded. Thirteen examples are invalid Ruby at
//! the fixed 3.4 (Prism and native Ruby reject them): six upstream runs
//! only with the Ruby 3.2 parser (`unsupported_on: :prism`: a top-level
//! `break` or `next`) and seven have a top-level `yield` (baselined in
//! tools/oracle/baseline.txt). Each is checked to be rejected and replayed
//! inside a block or a method, where it is valid.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceAroundKeyword";

/// `(line, col, message)` of each finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, String)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&SpaceAroundKeyword], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.end_line, o.line);
            (o.line, o.col, o.message)
        })
        .collect();
    found.sort();
    found
}

fn positions(source: &str) -> Vec<(u32, u32)> {
    findings(source)
        .into_iter()
        .map(|(line, col, _)| (line, col))
        .collect()
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceAroundKeyword, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (19, "BEGIN{}\n", &[(1, 1)], "BEGIN {}\n"),
    (19, "END{}\n", &[(1, 1)], "END {}\n"),
    (7, "1and 2\n", &[(1, 2)], "1 and 2\n"),
    (19, "1 and(2)\n", &[(1, 3)], "1 and (2)\n"),
    (19, "begin\"\" end\n", &[(1, 1)], "begin \"\" end\n"),
    (19, "break\"\"\n", &[(1, 1)], "break \"\"\n"),
    (36, "break(1)", &[], ""),
    (
        19,
        "case\"\" when 1; end\n",
        &[(1, 1)],
        "case \"\" when 1; end\n",
    ),
    (
        19,
        "case\"\"; in 1; end\n",
        &[(1, 1)],
        "case \"\"; in 1; end\n",
    ),
    (7, "a \"b\"do end\n", &[(1, 6)], "a \"b\" do end\n"),
    (19, "a do|x| end\n", &[(1, 3)], "a do |x| end\n"),
    (66, "a \"b\"do _1 end\n", &[(1, 6)], "a \"b\" do _1 end\n"),
    (77, "a do _1\"a\"end\n", &[(1, 11)], "a do _1\"a\" end\n"),
    (90, "a \"b\"do it end\n", &[(1, 6)], "a \"b\" do it end\n"),
    (
        101,
        "a do it.to_s\"a\"end\n",
        &[(1, 16)],
        "a do it.to_s\"a\" end\n",
    ),
    (7, "while 1do end\n", &[(1, 8)], "while 1 do end\n"),
    (
        19,
        "while 1 do\"x\" end\n",
        &[(1, 9)],
        "while 1 do \"x\" end\n",
    ),
    (7, "until 1do end\n", &[(1, 8)], "until 1 do end\n"),
    (
        19,
        "until 1 do\"x\" end\n",
        &[(1, 9)],
        "until 1 do \"x\" end\n",
    ),
    (7, "for x in []do end\n", &[(1, 12)], "for x in [] do end\n"),
    (
        19,
        "for x in [] do\"x\" end\n",
        &[(1, 13)],
        "for x in [] do \"x\" end\n",
    ),
    (7, "begin \"a\"end\n", &[(1, 10)], "begin \"a\" end\n"),
    (7, "if a; \"b\"end\n", &[(1, 10)], "if a; \"b\" end\n"),
    (7, "a do \"a\"end\n", &[(1, 9)], "a do \"a\" end\n"),
    (
        7,
        "while 1 do \"x\"end\n",
        &[(1, 15)],
        "while 1 do \"x\" end\n",
    ),
    (
        7,
        "until 1 do \"x\"end\n",
        &[(1, 15)],
        "until 1 do \"x\" end\n",
    ),
    (
        7,
        "for x in [] do \"x\"end\n",
        &[(1, 19)],
        "for x in [] do \"x\" end\n",
    ),
    (36, "begin end.inspect", &[], ""),
    (
        7,
        "if a; \"\"else end\n",
        &[(1, 9)],
        "if a; \"\" else end\n",
    ),
    (
        19,
        "if a; else\"\" end\n",
        &[(1, 7)],
        "if a; else \"\" end\n",
    ),
    (
        7,
        "begin rescue; \"\"else end\n",
        &[(1, 17)],
        "begin rescue; \"\" else end\n",
    ),
    (
        19,
        "begin rescue; else\"\" end\n",
        &[(1, 15)],
        "begin rescue; else \"\" end\n",
    ),
    (
        7,
        "case a; when b; \"\"else end\n",
        &[(1, 19)],
        "case a; when b; \"\" else end\n",
    ),
    (
        19,
        "case a; when b; else\"\" end\n",
        &[(1, 17)],
        "case a; when b; else \"\" end\n",
    ),
    (
        7,
        "case a; in b; \"\"else end\n",
        &[(1, 17)],
        "case a; in b; \"\" else end\n",
    ),
    (
        19,
        "case a; in b; else\"\" end\n",
        &[(1, 15)],
        "case a; in b; else \"\" end\n",
    ),
    (
        7,
        "case a; in \"pattern\"if \"condition\"; else \"\" end\n",
        &[(1, 21)],
        "case a; in \"pattern\" if \"condition\"; else \"\" end\n",
    ),
    (
        19,
        "case a; in \"pattern\" if\"condition\"; else \"\" end\n",
        &[(1, 22)],
        "case a; in \"pattern\" if \"condition\"; else \"\" end\n",
    ),
    (
        7,
        "case a; in \"pattern\"unless \"condition\"; else \"\" end\n",
        &[(1, 21)],
        "case a; in \"pattern\" unless \"condition\"; else \"\" end\n",
    ),
    (
        19,
        "case a; in \"pattern\" unless\"condition\"; else \"\" end\n",
        &[(1, 22)],
        "case a; in \"pattern\" unless \"condition\"; else \"\" end\n",
    ),
    (
        7,
        "if a; \"\"elsif b; end\n",
        &[(1, 9)],
        "if a; \"\" elsif b; end\n",
    ),
    (
        19,
        "if a; elsif\"\"; end\n",
        &[(1, 7)],
        "if a; elsif \"\"; end\n",
    ),
    (
        7,
        "begin \"\"ensure end\n",
        &[(1, 9)],
        "begin \"\" ensure end\n",
    ),
    (
        19,
        "begin ensure\"\" end\n",
        &[(1, 7)],
        "begin ensure \"\" end\n",
    ),
    (19, "if\"\"; end\n", &[(1, 1)], "if \"\"; end\n"),
    (19, "next\"\"\n", &[(1, 1)], "next \"\"\n"),
    (36, "next(1)", &[], ""),
    (19, "not\"\"\n", &[(1, 1)], "not \"\"\n"),
    (36, "not(1)", &[], ""),
    (7, "1or 2\n", &[(1, 2)], "1 or 2\n"),
    (19, "1 or(2)\n", &[(1, 3)], "1 or (2)\n"),
    (7, "\"\"rescue a\n", &[(1, 3)], "\"\" rescue a\n"),
    (19, "a rescue\"\"\n", &[(1, 3)], "a rescue \"\"\n"),
    (36, "begin; rescue(Error); end", &[], ""),
    (19, "return\"\"\n", &[(1, 1)], "return \"\"\n"),
    (19, "return(1)\n", &[(1, 1)], "return (1)\n"),
    (19, "super\"\"\n", &[(1, 1)], "super \"\"\n"),
    (36, "super(1)", &[], ""),
    (19, "super{}\n", &[(1, 1)], "super {}\n"),
    (36, "defined?(1)", &[], ""),
    (19, "defined?1\n", &[(1, 1)], "defined? 1\n"),
    (7, "if \"\"then a end\n", &[(1, 6)], "if \"\" then a end\n"),
    (19, "if a then\"\" end\n", &[(1, 6)], "if a then \"\" end\n"),
    (19, "unless\"\"; end\n", &[(1, 1)], "unless \"\"; end\n"),
    (7, "1until \"\"\n", &[(1, 2)], "1 until \"\"\n"),
    (19, "1 until\"\"\n", &[(1, 3)], "1 until \"\"\n"),
    (
        7,
        "case \"\"when a; end\n",
        &[(1, 8)],
        "case \"\" when a; end\n",
    ),
    (
        19,
        "case a when\"\"; end\n",
        &[(1, 8)],
        "case a when \"\"; end\n",
    ),
    (
        19,
        "case a; in\"\"; end\n",
        &[(1, 9)],
        "case a; in \"\"; end\n",
    ),
    (7, "\"\"in a\n", &[(1, 3)], "\"\" in a\n"),
    (19, "a in\"\"\n", &[(1, 3)], "a in \"\"\n"),
    (30, "\"\"=> a", &[], ""),
    (36, "a =>\"\"", &[], ""),
    (7, "1while \"\"\n", &[(1, 2)], "1 while \"\"\n"),
    (19, "1 while\"\"\n", &[(1, 3)], "1 while \"\"\n"),
    (19, "yield\"\"\n", &[(1, 1)], "yield \"\"\n"),
    (36, "yield(1)", &[], ""),
    (36, "+begin end", &[], ""),
    (19, "begin+1 end\n", &[(1, 1)], "begin +1 end\n"),
    (36, "test do\\\nend", &[], ""),
    (36, "test do\nend", &[], ""),
    (42, "(next)", &[], ""),
    (30, "!yield", &[], ""),
    (36, "yield.method", &[], ""),
    (30, "!yield.method", &[], ""),
    (30, "!super.method", &[], ""),
    (36, "super::ModuleName", &[], ""),
    (36, "super&.foo", &[], ""),
    (36, "yield&.foo", &[], ""),
    (36, "super[1]", &[], ""),
    (36, "yield[1]", &[], ""),
    (30, "loop { |x|break }", &[], ""),
    (30, "1..super.size", &[], ""),
    (30, "1...super.size", &[], ""),
    (30, "a=begin end", &[], ""),
    (30, "a==begin end", &[], ""),
    (30, "a+begin end", &[], ""),
    (30, "a+begin; end.method", &[], ""),
    (30, "a-begin end", &[], ""),
    (30, "a*begin end", &[], ""),
    (30, "a**begin end", &[], ""),
    (30, "a/begin end", &[], ""),
    (30, "a<begin end", &[], ""),
    (30, "a>begin end", &[], ""),
    (30, "a&&begin end", &[], ""),
    (30, "a||begin end", &[], ""),
    (30, "a=*begin end", &[], ""),
    (36, "loop{}", &[], ""),
    (42, "a 1,foo,1", &[], ""),
    (36, "next#comment", &[], ""),
    (42, "test do;end", &[], ""),
    (42, "[begin end]", &[], ""),
    (42, "loop {next}", &[], ""),
    (42, "{a: begin end}", &[], ""),
    (42, "a[begin end]", &[], ""),
    (42, "\"#{begin end}\"", &[], ""),
    (
        285,
        "def each\n  super{|x| yield x.foo }\nend\n",
        &[(2, 3)],
        "def each\n  super {|x| yield x.foo }\nend\n",
    ),
    (
        293,
        "def each\n  super\"\"\nend\n",
        &[(2, 3)],
        "def each\n  super \"\"\nend\n",
    ),
];

/// The examples rejected at Ruby 3.4 (top-level `break`, `next` and
/// `yield`), replayed in a block or a method. `(source, wrapped, findings,
/// correction)`.
type Wrapped = (
    &'static str,
    &'static str,
    &'static [(u32, u32)],
    &'static str,
);

const NOT_RUBY34: &[Wrapped] = &[
    (
        "break\"\"\n",
        "loop { break\"\" }\n",
        &[(1, 8)],
        "loop { break \"\" }\n",
    ),
    ("break(1)", "loop { break(1) }", &[], ""),
    (
        "next\"\"\n",
        "loop { next\"\" }\n",
        &[(1, 8)],
        "loop { next \"\" }\n",
    ),
    ("next(1)", "loop { next(1) }", &[], ""),
    ("(next)", "loop { (next) }", &[], ""),
    ("next#comment", "loop { next#comment\n}", &[], ""),
    (
        "yield\"\"\n",
        "def f\n  yield\"\"\nend\n",
        &[(2, 3)],
        "def f\n  yield \"\"\nend\n",
    ),
    ("yield(1)", "def f; yield(1); end", &[], ""),
    ("!yield", "def f; !yield; end", &[], ""),
    ("yield.method", "def f; yield.method; end", &[], ""),
    ("!yield.method", "def f; !yield.method; end", &[], ""),
    ("yield&.foo", "def f; yield&.foo; end", &[], ""),
    ("yield[1]", "def f; yield[1]; end", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 118);
    let mut replayed = 0;
    for &(line, source, expected, corrected) in PINNED {
        if let Some(&(_, wrapped, expected, corrected)) =
            NOT_RUBY34.iter().find(|row| row.0 == source)
        {
            assert!(
                crate::parser::parse(source.as_bytes()).errors().count() > 0,
                "{source:?}"
            );
            assert_eq!(positions(wrapped), expected, "{wrapped:?}");
            if !expected.is_empty() {
                assert_correction(&SpaceAroundKeyword, wrapped, corrected);
            }
            replayed += 1;
            continue;
        }
        assert_eq!(positions(source), expected, "spec line {line} {source:?}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceAroundKeyword, source).is_empty());
        } else {
            assert_correction(&SpaceAroundKeyword, source, corrected);
        }
    }
    assert_eq!(replayed, NOT_RUBY34.len());
}

#[test]
fn messages_name_the_keyword_and_the_side() {
    assert_eq!(
        findings("1and 2\n"),
        [(1, 2, "Space before keyword `and` is missing.".to_string())]
    );
    assert_eq!(
        findings("x = defined?1\n"),
        [(
            1,
            5,
            "Space after keyword `defined?` is missing.".to_string()
        )]
    );
    assert_eq!(
        findings("def f\n  super\"\"\nend\n"),
        [(2, 3, "Space after keyword `super` is missing.".to_string())]
    );
    // Both blanks missing: the range is reported once, for the one before.
    assert_eq!(
        findings("1and(2)\n"),
        [(1, 2, "Space before keyword `and` is missing.".to_string())]
    );
}

#[test]
fn metadata() {
    assert!(SpaceAroundKeyword.default_enabled());
    assert!(SpaceAroundKeyword.department_selection_enables());
    assert!(SpaceAroundKeyword.safe_fixes());
    assert!(
        SpaceAroundKeyword
            .autocorrect_incompatible_with()
            .is_empty()
    );
}

/// `(source, [(line, col, side)], correction)`: `b` = before, `a` = after.
type Authored = (&'static str, &'static [(u32, u32, char)], &'static str);

const AUTHORED: &[Authored] = &[
    ("foo(1)do end\n", &[(1, 7, 'b')], "foo(1) do end\n"),
    ("foo do|x|end\n", &[(1, 5, 'a')], "foo do |x|end\n"),
    ("x = ->()do end\n", &[(1, 9, 'b')], "x = ->() do end\n"),
    (
        "x = -> do\"a\" end\n",
        &[(1, 8, 'a')],
        "x = -> do \"a\" end\n",
    ),
    ("foo do x\"a\"end\n", &[(1, 12, 'b')], "foo do x\"a\" end\n"),
    (
        "while a do\"x\"end\n",
        &[(1, 9, 'a'), (1, 14, 'b')],
        "while a do \"x\" end\n",
    ),
    (
        "x = begin\"a\"end\n",
        &[(1, 5, 'a'), (1, 13, 'b')],
        "x = begin \"a\" end\n",
    ),
    (
        "if a then\"b\"else\"c\"end\n",
        &[(1, 6, 'a'), (1, 13, 'b'), (1, 20, 'b')],
        "if a then \"b\" else\"c\" end\n",
    ),
    (
        "unless a then\"b\"else\"c\"end\n",
        &[(1, 10, 'a'), (1, 17, 'b'), (1, 24, 'b')],
        "unless a then \"b\" else\"c\" end\n",
    ),
    // `case`'s `end` and `when`'s `then` are not checked.
    (
        "case a\nwhen 1 then\"b\"else\"c\"end\n",
        &[(2, 15, 'b')],
        "case a\nwhen 1 then\"b\" else\"c\"end\n",
    ),
    (
        "case a\nin 1 then\"b\"else\"c\"end\n",
        &[(2, 13, 'b')],
        "case a\nin 1 then\"b\" else\"c\"end\n",
    ),
    (
        "x = 1 rescue\"a\"\n",
        &[(1, 7, 'a')],
        "x = 1 rescue \"a\"\n",
    ),
    (
        "begin\nrescue\"a\"\nend\n",
        &[(2, 1, 'a')],
        "begin\nrescue \"a\"\nend\n",
    ),
    (
        "begin\n  a\nrescue\n  b\nelse\"c\"\nensure\"d\"\nend\n",
        &[(5, 1, 'a'), (6, 1, 'a')],
        "begin\n  a\nrescue\n  b\nelse \"c\"\nensure \"d\"\nend\n",
    ),
    (
        "def f\n  a\nrescue\n  b\nelse\"c\"\nend\n",
        &[(5, 1, 'a')],
        "def f\n  a\nrescue\n  b\nelse \"c\"\nend\n",
    ),
    ("x = a and\"b\"\n", &[(1, 7, 'a')], "x = a and \"b\"\n"),
    ("x = (a)or b\n", &[(1, 8, 'b')], "x = (a) or b\n"),
    ("x = not\"a\"\n", &[(1, 5, 'a')], "x = not \"a\"\n"),
    ("x = defined?a\n", &[(1, 5, 'a')], "x = defined? a\n"),
    (
        "def f\n  yield@a\nend\n",
        &[(2, 3, 'a')],
        "def f\n  yield @a\nend\n",
    ),
    (
        "def f(a)\n  super$a\nend\n",
        &[(2, 3, 'a')],
        "def f(a)\n  super $a\nend\n",
    ),
    (
        "def f\n  super(1)do end\nend\n",
        &[(2, 11, 'b')],
        "def f\n  super(1) do end\nend\n",
    ),
    (
        "loop { break\"a\" }\n",
        &[(1, 8, 'a')],
        "loop { break \"a\" }\n",
    ),
    (
        "def f\n  return\"a\"\nend\n",
        &[(2, 3, 'a')],
        "def f\n  return \"a\"\nend\n",
    ),
    ("x = 1 if\"a\"\n", &[(1, 7, 'a')], "x = 1 if \"a\"\n"),
    ("x = [1]if a\n", &[(1, 8, 'b')], "x = [1] if a\n"),
    (
        "x = 1 unless\"a\"\n",
        &[(1, 7, 'a')],
        "x = 1 unless \"a\"\n",
    ),
    ("x = 1 while\"a\"\n", &[(1, 7, 'a')], "x = 1 while \"a\"\n"),
    ("x = (1)until a\n", &[(1, 8, 'b')], "x = (1) until a\n"),
    ("x = a in\"b\"\n", &[(1, 7, 'a')], "x = a in \"b\"\n"),
    (
        "case a\nin \"b\"if c\nend\n",
        &[(2, 7, 'b')],
        "case a\nin \"b\" if c\nend\n",
    ),
    (
        "case a\nin b unless\"c\"\nend\n",
        &[(2, 6, 'a')],
        "case a\nin b unless \"c\"\nend\n",
    ),
    (
        "if a\nelsif b\n  \"c\"elsif d\nend\n",
        &[(3, 6, 'b')],
        "if a\nelsif b\n  \"c\" elsif d\nend\n",
    ),
    (
        "if a\nelsif b then\"c\"\nend\n",
        &[(2, 9, 'a')],
        "if a\nelsif b then \"c\"\nend\n",
    ),
    // An `elsif` missing both blanks under an operator: the parent's `else`
    // check skips the one before and reports the one after first.
    (
        "x = a + if b then 1\n\"c\"elsif\"d\" then 2 end\n",
        &[(2, 4, 'a')],
        "x = a + if b then 1\n\"c\"elsif \"d\" then 2 end\n",
    ),
    (
        "x = a + if b then 1\n\"c\"elsif d then 2 end\n",
        &[(2, 4, 'b')],
        "x = a + if b then 1\n\"c\" elsif d then 2 end\n",
    ),
    ("BEGIN{ }\n", &[(1, 1, 'a')], "BEGIN { }\n"),
    ("x = 1;END{ }\n", &[(1, 7, 'a')], "x = 1;END { }\n"),
    (
        "\u{feff}x = 1if a\n",
        &[(1, 6, 'b')],
        "\u{feff}x = 1 if a\n",
    ),
    ("x = 1if a\r\n", &[(1, 6, 'b')], "x = 1 if a\r\n"),
    ("x = 'é'if a\n", &[(1, 8, 'b')], "x = 'é' if a\n"),
];

#[test]
fn authored_offenses_and_canonical_insertions() {
    for &(source, expected, after) in AUTHORED {
        let found: Vec<_> = findings(source)
            .into_iter()
            .map(|(line, col, message)| {
                let side = if message.starts_with("Space before") {
                    'b'
                } else {
                    'a'
                };
                (line, col, side)
            })
            .collect();
        assert_eq!(found, expected, "{source:?}");
        assert_correction(&SpaceAroundKeyword, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = a.if\n",
        "x = a.end\n",
        "x = :if\n",
        "x = { if: 1, end: 2 }\n",
        "x = \"if\"\n",
        "# 1if a\n",
        "x = a ? b : c\n",
        "x = a.then { 1 }\n",
        "x = a && b || c\n",
        // Without `do`, the `end` of a loop is not checked.
        "while a; \"x\"end\n",
        "until a; \"x\"end\n",
        "for x in a; \"x\"end\n",
        // `case`'s own `end`, `for`'s `in` and `when`'s `then`.
        "case a; when 1; \"x\"end\n",
        "for x in[1] do end\n",
        "case a; when 1 then\"x\" end\n",
        "foo {\"x\"}\n",
        "foo do\nend\n",
        "foo do # c\nend\n",
        "foo do;end\n",
        "foo do\\\nend\n",
        // `begin ... end while` has no keyword callback.
        "begin; end while\"a\"\n",
        "begin; end until\"a\"\n",
        // Ruby 3.4's one-line `=>` is never checked.
        "x = 1\n1=>\"a\"\n",
        "x = 1 if a\n",
        "x = defined?(a)\n",
        "x = not(a)\n",
        "x = a rescue(b)\n",
        "def f\n  super(1)\n  super[1]\n  super::A\n  super&.a\n  yield(1)\n  yield[1]\n  yield&.a\nend\n",
        // Preceded by an operator or a range.
        "x = a.b+if c then 1 end\n",
        "x = !begin; end\n",
        "x = a&&begin end\n",
        "x = [1][0]+begin end\n",
        "x = 1..begin 2 end\n",
        "x = a<=>begin end\n",
        "x = a!=begin end\n",
        "x = a ? 1 : begin end\n",
        "x = 1;begin end\n",
        "x = 1,begin end\n",
        "x = *begin end\n",
        "foo(&\n  begin end)\n",
        "[1].each { |x|next }\n",
        "x = \"#{begin end}\"\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn lexer_sensitive_insertions_are_withheld() {
    for source in [
        // A blank after `&` would turn a block-pass into a binary `&`.
        "foo &begin end\n",
        "foo(&begin end)\n",
        // After `not`, `yield`, `super` and `defined?` a blank makes the
        // next token a command argument.
        "def f\n  super-1\nend\n",
        "def f\n  super*a\nend\n",
        "def f\n  yield*a\nend\n",
        "def f\n  super%w[a]\nend\n",
        "def f\n  super<<~A\n  a\n  A\nend\n",
    ] {
        held(source);
    }
    // The guards are reached: these all parse and report.
    for source in [
        "foo &begin end\n",
        "def f\n  super-1\nend\n",
        "def f\n  yield*a\nend\n",
    ] {
        assert_eq!(positions(source).len(), 1, "{source:?}");
    }
}

#[test]
fn every_offense_of_a_file_is_corrected_in_one_pass() {
    let source = "if a then\"b\"end\nx = 1if a\nfoo(1)do|y|end\n";
    assert_eq!(positions(source), [(1, 6), (1, 13), (2, 6), (3, 7)]);
    assert_correction(
        &SpaceAroundKeyword,
        source,
        "if a then \"b\" end\nx = 1 if a\nfoo(1) do|y|end\n",
    );
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
        held(&format!("x = 1if a\n{observer}\n"));
    }
    assert_correction(
        &SpaceAroundKeyword,
        "x = 1if a\np File.read('example.rb')\n",
        "x = 1 if a\np File.read('example.rb')\n",
    );
}
