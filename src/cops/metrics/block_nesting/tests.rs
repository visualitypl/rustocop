//! Examples ported from RuboCop's
//! `spec/rubocop/cop/metrics/block_nesting_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! All 26 pinned examples run verbatim at the fixed defaults and match the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! source SHA-256 was checked against it when the rows were ported): the
//! spec sets `Max: 2`, so only the four-deep `Max + 2` example reaches the
//! fixed `Max: 3`. Each upstream finding is then ported at the fixed limit
//! by wrapping the example in one more `if` (one line down, two columns
//! right). The spec-custom `CountBlocks: true` and `CountModifierForms:
//! true` examples are not applicable at the fixed defaults; their ports
//! assert the fixed behaviour (blocks and modifier `if`s never count). The
//! `with exclude limit tracking` reads (`Max => 3`/`4`) are `--auto-gen-config`
//! feedback, not findings, and have no rustocop counterpart.
use super::*;
use crate::pipeline::lint_str_with;

const COP: &str = "Metrics/BlockNesting";
const MSG: &str = "Avoid more than 3 levels of block nesting.";

/// `(line, col, end line)` of every finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&BlockNesting], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.message, MSG);
            (o.line, o.col, o.end_line)
        })
        .collect();
    found.sort_unstable();
    found
}

/// `source` inside one more `if z`, adding one level.
fn wrap(source: &str) -> String {
    let body: String = source.lines().map(|l| format!("  {l}\n")).collect();
    format!("if z\n{body}end\n")
}

/// `(line, col, end line)` findings.
type Found = &'static [(u32, u32, u32)];

/// `(spec line, source, fixed-default findings)`.
const PINNED: &[(u32, &str, Found)] = &[
    (
        9,
        "if a
  if b
    puts b
  end
end
",
        &[],
    ),
    (
        199,
        "if a
elsif b
elsif c
elsif d
end
",
        &[],
    ),
    (
        20,
        "if a
  if b
    if c
      puts c
    end
  end
end
",
        &[],
    ),
    (
        36,
        "if a
  if b
    if c
      if d
        puts d
      end
    end
  end
end
",
        &[(4, 7, 6)],
    ),
    (
        54,
        "if a
  if b
    if c
      puts c
    end
  end
  if d
    if e
      puts e
    end
  end
end
",
        &[],
    ),
    (
        76,
        "if a
  if b
    case c
      when C
        puts C
    end
  end
end
",
        &[],
    ),
    (
        92,
        "if a
  if b
    case c
      in C
        puts C
    end
  end
end
",
        &[],
    ),
    (
        108,
        "if a
  if b
    while c
      puts c
    end
  end
end
",
        &[],
    ),
    (
        123,
        "if a
  if b
    begin
      puts c
    end while c
  end
end
",
        &[],
    ),
    (
        138,
        "if a
  if b
    until c
      puts c
    end
  end
end
",
        &[],
    ),
    (
        153,
        "if a
  if b
    begin
      puts c
    end until c
  end
end
",
        &[],
    ),
    (
        168,
        "if a
  if b
    for c in [1,2] do
      puts c
    end
  end
end
",
        &[],
    ),
    (
        183,
        "if a
  if b
    begin
      puts c
    rescue
      puts x
    end
  end
end
",
        &[],
    ),
    (
        212,
        "if a
  if b
    [1, 2].each do |c|
      puts c
    end
  end
end
",
        &[],
    ),
    (
        224,
        "if a
  if b
    [1, 2].each { |c| puts c }
  end
end
",
        &[],
    ),
    (
        235,
        "if a
  if b
    [1, 2].each do
      puts _1
    end
  end
end
",
        &[],
    ),
    (
        247,
        "if a
  if b
    [1, 2].each { puts _1 }
  end
end
",
        &[],
    ),
    (
        263,
        "if a
  if b
    [1, 2].each do |c|
      puts c
    end
  end
end
",
        &[],
    ),
    (
        278,
        "if a
  if b
    [1, 2].each { |c| puts c }
  end
end
",
        &[],
    ),
    (
        292,
        "if a
  if b
    [1, 2].each do
      puts _1
    end
  end
end
",
        &[],
    ),
    (
        307,
        "if a
  if b
    [1, 2].each { puts _1 }
  end
end
",
        &[],
    ),
    (
        322,
        "if a
  if b
    [1, 2].each do
      puts it
    end
  end
end
",
        &[],
    ),
    (
        337,
        "if a
  if b
    [1, 2].each { puts it }
  end
end
",
        &[],
    ),
    (
        354,
        "if a
  if b
    puts 'hello' if c
  end
end
",
        &[],
    ),
    (
        364,
        "if a
  if b
    if c
      puts 'hello'
    end
  end
end
",
        &[],
    ),
    (
        382,
        "if a
  if b
    puts 'hello' if c
  end
end
",
        &[],
    ),
];

/// Spec line → its upstream `Max: 2` findings as `(line, col, end line)`.
const UPSTREAM: &[(u32, Found)] = &[
    (20, &[(3, 5, 5)]),
    (36, &[(3, 5, 7)]),
    (54, &[(3, 5, 5), (8, 5, 10)]),
    (76, &[(3, 5, 6)]),
    (92, &[(3, 5, 6)]),
    (108, &[(3, 5, 5)]),
    (123, &[(3, 5, 5)]),
    (138, &[(3, 5, 5)]),
    (153, &[(3, 5, 5)]),
    (168, &[(3, 5, 5)]),
    (183, &[(5, 5, 6)]),
    (364, &[(3, 5, 5)]),
];

/// Spec examples whose findings need `CountBlocks: true` or
/// `CountModifierForms: true`.
const CUSTOM: &[u32] = &[263, 278, 292, 307, 322, 337, 382];

fn pinned(line: u32) -> &'static str {
    PINNED.iter().find(|p| p.0 == line).unwrap().1
}

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 26);
    for (line, source, want) in PINNED {
        assert_eq!(findings(source), *want, "spec line {line}");
    }
}

#[test]
fn upstream_findings_at_the_fixed_limit() {
    for (line, want) in UPSTREAM {
        let want: Vec<_> = want
            .iter()
            .map(|&(l, c, e)| (l + 1, c + 2, e + 1))
            .collect();
        assert_eq!(findings(&wrap(pinned(*line))), want, "spec line {line}");
    }
}

#[test]
fn custom_count_examples_never_count_at_the_fixed_defaults() {
    for line in CUSTOM {
        // Three conditionals around the block or modifier `if`, which
        // would be the fourth level if it counted.
        assert_eq!(findings(&wrap(pinned(*line))), [], "spec line {line}");
    }
}

#[test]
fn three_levels_pass_and_the_fourth_is_reported() {
    let three = "if a\n  if b\n    if c\n      d\n    end\n  end\nend\n";
    assert_eq!(findings(three), []);
    assert_eq!(findings(&wrap(three)), [(4, 7, 6)]);
}

#[test]
fn offending_descendants_are_ignored_but_siblings_are_reported() {
    // Six levels: only the fourth is reported.
    let mut deep = "x\n".to_string();
    for name in ["f", "e", "d", "c", "b", "a"] {
        deep = format!(
            "if {name}\n{}end\n",
            deep.lines().map(|l| format!("  {l}\n")).collect::<String>()
        );
    }
    assert_eq!(findings(&deep), [(4, 7, 10)]);
    // Two over-limit siblings and one nested in each.
    let siblings = "if a\n  if b\n    if c\n      while d\n        if e\n          x\n        end\n      end\n      for i in y\n        z\n      end\n    end\n  end\nend\n";
    assert_eq!(findings(siblings), [(4, 7, 8), (9, 7, 11)]);
}

#[test]
fn elsif_never_counts_and_keeps_the_enclosing_level() {
    let chain = "if a\nelsif b\nelsif c\nelsif d\nelsif e\nelse\nend\n";
    assert_eq!(findings(&wrap(&wrap(chain))), []);
    // An `if` in an `elsif` branch is one level below the first `if`.
    let source = "if a\n  if b\n    if c\n    elsif d\n      if e\n        x\n      end\n    end\n  end\nend\n";
    assert_eq!(findings(source), [(5, 7, 7)]);
    // `else` adds nothing either.
    let source =
        "if a\n  if b\n    if c\n    else\n      if e\n        x\n      end\n    end\n  end\nend\n";
    assert_eq!(findings(source), [(5, 7, 7)]);
}

#[test]
fn unless_ternary_and_one_line_if_count() {
    let around =
        |inner: &str| format!("if a\n  if b\n    if c\n      {inner}\n    end\n  end\nend\n");
    assert_eq!(
        findings(&around("unless d\n        x\n      end")),
        [(4, 7, 6)]
    );
    assert_eq!(findings(&around("unless d then x else y end")), [(4, 7, 4)]);
    assert_eq!(findings(&around("x = d ? 1 : 2")), [(4, 11, 4)]);
    assert_eq!(findings(&around("if d then x end")), [(4, 7, 4)]);
    assert_eq!(findings(&around("if d; x; end")), [(4, 7, 4)]);
}

#[test]
fn modifier_conditionals_and_pattern_guards_do_not_count() {
    let around =
        |inner: &str| format!("if a\n  if b\n    if c\n      {inner}\n    end\n  end\nend\n");
    assert_eq!(findings(&around("x if d")), []);
    assert_eq!(findings(&around("x unless d")), []);
    assert_eq!(findings(&around("begin\n        x\n      end if d")), []);
    assert_eq!(findings(&around("(x if d) if e")), []);
    // Prism shapes `in` guards as modifier conditionals; they never count.
    let around2 = |inner: &str| format!("if a\n  if b\n    {inner}\n  end\nend\n");
    assert_eq!(
        findings(&around2(
            "case v\n    in [w] if d then x\n    in w unless d then y\n    end"
        )),
        []
    );
    // A modifier holding a counting node: the inner node counts, the
    // modifier does not.
    assert_eq!(findings(&around("(d ? x : y) if e")), [(4, 8, 4)]);
}

#[test]
fn modifier_loops_count() {
    let around =
        |inner: &str| format!("if a\n  if b\n    if c\n      {inner}\n    end\n  end\nend\n");
    assert_eq!(findings(&around("x while d")), [(4, 7, 4)]);
    assert_eq!(findings(&around("x until d")), [(4, 7, 4)]);
    assert_eq!(findings(&around("x += 1 while d")), [(4, 7, 4)]);
    assert_eq!(findings(&around("while d do x end")), [(4, 7, 4)]);
    assert_eq!(findings(&around("until d do x end")), [(4, 7, 4)]);
    assert_eq!(findings(&around("for i in d do x end")), [(4, 7, 4)]);
}

#[test]
fn every_case_kind_counts_but_its_branches_do_not() {
    let around = |inner: &str| format!("if a\n  if b\n    {inner}\n  end\nend\n");
    let case = "case c\n    when 1\n      case d\n      when 2 then x\n      end\n    else\n      case e\n      in [f] then y\n      end\n    end";
    assert_eq!(findings(&around(case)), [(5, 7, 7), (9, 7, 11)]);
    // `case` without a subject.
    assert_eq!(
        findings(&around("case\n    when c\n      while d do x end\n    end")),
        [(5, 7, 5)]
    );
}

#[test]
fn rescue_clauses_are_siblings_not_nested() {
    let around = |inner: &str| format!("if a\n  if b\n    {inner}\n  end\nend\n");
    // Two clauses at level three: neither is nested in the other.
    assert_eq!(
        findings(&around(
            "begin\n      x\n    rescue A\n      y\n    rescue B => e\n      z\n    end"
        )),
        []
    );
    // At level four both clauses are reported, each at its keyword.
    assert_eq!(
        findings(&wrap(&around(
            "begin\n      x\n    rescue A\n      y\n    rescue B, C => e\n      z\n    else\n      w\n    ensure\n      v\n    end"
        ))),
        [(6, 7, 7), (8, 7, 9)]
    );
    // A conditional in the second clause sits only one level below the
    // enclosing `if`s.
    assert_eq!(
        findings(&around(
            "begin\n      x\n    rescue A\n      y\n    rescue B\n      if c\n        z\n      end\n    end"
        )),
        [(8, 7, 10)]
    );
    // The protected body, `else` and `ensure` add nothing: the ternaries
    // there are the fourth level, not the fifth.
    assert_eq!(
        findings(&around(
            "begin\n      if c then (f ? x : y) end\n    rescue\n      y\n    else\n      if d then (f ? w : v) end\n    ensure\n      if e then (f ? v : u) end\n    end"
        )),
        [(4, 18, 4), (8, 18, 8), (10, 18, 10)]
    );
}

#[test]
fn method_and_block_bodies_with_rescue_count_their_clauses() {
    let source = "if a\n  if b\n    if c\n      foo do\n        x\n      rescue\n        y\n      end\n      def m\n        x\n      rescue => e\n        y\n      end\n    end\n  end\nend\n";
    assert_eq!(findings(source), [(6, 7, 7), (11, 7, 12)]);
}

#[test]
fn modifier_rescue_counts_only_its_rescuing_side() {
    let around =
        |inner: &str| format!("if a\n  if b\n    if c\n      {inner}\n    end\n  end\nend\n");
    assert_eq!(findings(&around("x rescue y")), [(4, 9, 4)]);
    assert_eq!(findings(&around("v = x rescue y")), [(4, 13, 4)]);
    // The rescued expression lies outside the `resbody`.
    assert_eq!(
        findings(&around("(d ? x : y) rescue z")),
        [(4, 8, 4), (4, 19, 4)]
    );
    // A conditional after the keyword is inside the reported `resbody`.
    assert_eq!(findings(&around("x rescue (d ? y : z)")), [(4, 9, 4)]);
    // At level three the rescuing side may hold one more level.
    let around3 = |inner: &str| format!("if a\n  if b\n    {inner}\n  end\nend\n");
    assert_eq!(findings(&around3("x rescue (d ? y : z)")), [(3, 15, 3)]);
    assert_eq!(findings(&around3("(d ? x : y) rescue z")), []);
}

#[test]
fn nesting_crosses_method_class_block_and_lambda_boundaries() {
    let source = "if a\n  class C\n    def m\n      items.each do |i|\n        -> {\n          if b\n            while c\n              d until e\n            end\n          end\n        }\n      end\n    end\n  end\nend\n";
    assert_eq!(findings(source), [(8, 15, 8)]);
    // Blocks of every kind and lambdas never count.
    let blocks = "if a\n  if b\n    if c\n      x.each { |i| y.map { _1 } }\n      z.then { it }\n      loop do\n        -> { 1 }\n        lambda { 2 }\n      end\n    end\n  end\nend\n";
    assert_eq!(findings(blocks), []);
}

#[test]
fn conditions_are_inside_their_construct() {
    let source = "if a\n  if b\n    if c\n      x\n    end\n    while (d ? e : f)\n      g\n    end\n  end\nend\n";
    assert_eq!(findings(&wrap(source)), [(4, 7, 6), (7, 7, 9)]);
    // A conditional in the condition of the third level is the fourth.
    let source = "if a\n  if b\n    if (c ? d : e)\n      x\n    end\n  end\nend\n";
    assert_eq!(findings(source), [(3, 9, 3)]);
}

#[test]
fn lookalikes_are_not_nesting() {
    let around =
        |inner: &str| format!("if a\n  if b\n    if c\n      {inner}\n    end\n  end\nend\n");
    for inner in [
        "x && y || z",
        "x and y or z",
        "foo.if",
        "foo.while(1)",
        "w = { if: 1, while: 2, case: 3 }",
        "\"if d then x end\"",
        "# if d\n      x",
        "begin\n        x\n      ensure\n        y\n      end",
        "begin\n        x\n      end",
        "v in [w]",
        "v => [w]",
        "z = x || y",
        "z ||= 1",
        "z = -> { if d then x end }.itself if false",
    ] {
        let want: &[(u32, u32, u32)] = if inner.starts_with("z = ->") {
            &[(4, 16, 4)]
        } else {
            &[]
        };
        assert_eq!(findings(&around(inner)), want, "{inner}");
    }
}

#[test]
fn every_file_is_measured_from_the_top_level() {
    assert_eq!(findings(""), []);
    assert_eq!(findings("# only a comment\n"), []);
    let three = "if a\n  if b\n    if c\n      d\n    end\n  end\nend\n";
    // Each top-level construct starts at level one again.
    assert_eq!(findings(&format!("{three}{three}{three}")), []);
    assert_eq!(findings(&format!("{three}{}", wrap(three))), [(11, 7, 13)]);
    // A file is never measured against another one.
    assert_eq!(findings(three), []);
}

#[test]
fn crlf_and_multibyte_columns() {
    let source =
        wrap("if a\n  if b\n    if c\n      d\n    end\n  end\nend\n").replace('\n', "\r\n");
    assert_eq!(findings(&source), [(4, 7, 6)]);
    let source = "if a\n  if b\n    if c\n      \"é\"; x while d\n    end\n  end\nend\n";
    assert_eq!(findings(source), [(4, 12, 4)]);
}

#[test]
fn a_disabled_offense_does_not_hide_the_next_level() {
    // Upstream's `ignore_node` runs in the `add_offense` block, which is
    // skipped when a directive disables any line of the offense's range: the
    // next counting node below is then reported (if it is enabled itself).
    let deep = |d: &str, e: &str, x: &str, w: &str| {
        format!(
            "if a\n  if b\n    if c\n      if d{d}\n        if e{e}\n          x{x}\n          while f{w}\n            y\n          end\n        end\n      end\n    end\n  end\nend\n"
        )
    };
    let off = " # rubocop:disable Metrics/BlockNesting";
    for (source, want) in [
        (deep(off, "", "", ""), vec![(5, 9, 10)]),
        (deep(" # rustocop:disable Metrics", "", "", ""), vec![(5, 9, 10)]),
        (deep(" # rubocop:todo Metrics/BlockNesting", "", "", ""), vec![(5, 9, 10)]),
        (deep(" # rubocop:disable all", "", "", ""), vec![(5, 9, 10)]),
        (deep(off, off, "", ""), vec![(7, 11, 9)]),
        // A line inside a range disables every node spanning it.
        (deep("", "", off, ""), vec![(7, 11, 9)]),
        (deep("", "", "", off), vec![]),
        (deep(off, off, "", off), vec![]),
        (deep(" # rubocop:disable Metrics/AbcSize", "", "", ""), vec![(4, 7, 11)]),
        (
            format!("# rubocop:disable Metrics/BlockNesting\n{}# rubocop:enable Metrics/BlockNesting\n", deep("", "", "", "")),
            vec![],
        ),
        (
            "if a\n  if b\n    if c\n      x rescue y # rubocop:disable Metrics/BlockNesting\n      begin\n      rescue # rubocop:disable Metrics/BlockNesting\n        while d\n        end\n      end\n    end\n  end\nend\n".to_string(),
            vec![(7, 9, 8)],
        ),
    ] {
        assert_eq!(findings(&source), want, "{source}");
    }
}
