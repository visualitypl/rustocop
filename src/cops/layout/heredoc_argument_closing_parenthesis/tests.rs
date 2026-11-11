//! Examples ported from RuboCop's `spec/rubocop/cop/layout/heredoc_argument_closing_parenthesis_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for both
//! the `.` and `&.` shared examples. The cop is opt-in, so the tables run it
//! explicitly; findings, messages and single-pass corrected bytes match the
//! fixed replay recorded in the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    offense::Severity,
    pipeline::lint_str,
};

fn positions(source: &str) -> Vec<(u32, u32, String)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source}"
    );
    correction_offenses(&HeredocArgumentClosingParenthesis, source)
        .into_iter()
        .map(|o| (o.line, o.col, o.message))
        .collect()
}

/// Every pinned example without an offense.
const ACCEPTED: &[&str] = &[
    // 7 behaves like correct cases correct cases accepts simple correct case
    "foo.bar(<<-SQL)\n  foo\nSQL\n",
    // 15 behaves like correct cases correct cases accepts double correct case
    "foo.bar(<<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n",
    // 25 behaves like correct cases correct cases accepts method chain with heredoc argument correc
    "do_something(\n  Model\n    .foo.bar(<<~CODE)\n      code\n    CODE\n    .baz(<<~CODE))\n      code\n    CODE\n",
    // 38 behaves like correct cases correct cases accepts method chain with safe navigation with he
    "do_something(\n  Model\n    .foo.bar(<<~CODE)\n      code\n    CODE\n    &.baz(<<~CODE))\n      code\n    CODE\n",
    // 51 behaves like correct cases correct cases accepts method with heredoc argument of proc corr
    "outer_method(-> {\n  inner_method.chain(<<~CODE)\n    code\n  CODE\n})\n",
    // 61 behaves like correct cases correct cases accepts method with heredoc argument of proc with
    "outer_method(\n  -> {\n    inner_method.chain(<<~CODE)\n      code\n    CODE\n  }\n)\n",
    // 73 behaves like correct cases correct cases accepts method with heredoc argument of proc corr
    "outer_method( -> {\n  inner_method.chain( <<~CODE )\n    code\n  CODE\n} )\n",
    // 83 behaves like correct cases correct cases accepts double correct case nested
    "baz(bar(foo.bar(<<-SQL, <<-NOSQL)))\n  foo\nSQL\n  bar\nNOSQL\n",
    // 93 behaves like correct cases correct cases accepts double correct case new line
    "foo.bar(\n  <<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n",
    // 104 behaves like correct cases correct cases accepts when there is an argument between a hered
    "foo.bar(<<~TEXT,\n    Lots of\n    Lovely\n    Text\n  TEXT\n  some_arg: { foo: \"bar\" }\n)\n",
    // 116 behaves like correct cases correct cases accepts when heredoc is a method argument in a pa
    "foo(bar do\n  baz.quux <<~EOS\n  EOS\nend)\n",
    // 125 behaves like correct cases correct cases accepts when heredoc is a branch body in a method
    "foo(unless condition\n  bar.baz(<<~EOS)\n    text\n  EOS\nend)\n",
    // 135 behaves like correct cases correct cases accepts when heredoc is a branch body in a nested
    "foo(unless condition\n  bar(baz.quux(<<~EOS))\n    text\n  EOS\nend)\n",
    // 145 behaves like correct cases correct cases accepts correct case with other param after
    "foo.bar(<<-SQL, 123)\n  foo\nSQL\n",
    // 153 behaves like correct cases correct cases accepts correct case with other param before
    "foo.bar(123, <<-SQL)\n  foo\nSQL\n",
    // 161 behaves like correct cases correct cases accepts hash correct case
    "foo.bar(foo: <<-SQL)\n  foo\nSQL\n",
    // 169 behaves like correct cases correct cases accepts a HEREDOC in a hash followed by another p
    "foo.bar(\n  baz: <<-SQL,\n    foo\n  SQL\n  qux: 1\n)\n",
    // 180 behaves like correct cases correct cases accepts a HEREDOC in a hash whose braces close af
    "foo.bar(:baz,\n  foo: {\n    qux: <<-SQL,\n      foo\n    SQL\n  }\n)\n",
    // 193 behaves like correct cases correct cases invocation after the HEREDOC ignores tr
    "foo.bar(\n  <<-SQL.tr(\"z\", \"t\"))\n  baz\nSQL\n",
    // 202 behaves like correct cases correct cases invocation after the HEREDOC ignores random call
    "foo.bar(\n  <<-TEXT.foo)\n  foobarbaz\nTEXT\n",
    // 211 behaves like correct cases correct cases invocation after the HEREDOC ignores random call
    "foo.bar(\n  <<-TEXT\n  foobarbaz\nTEXT\n.foo\n)\n",
    // 7 behaves like correct cases correct cases accepts simple correct case
    "foo&.bar(<<-SQL)\n  foo\nSQL\n",
    // 15 behaves like correct cases correct cases accepts double correct case
    "foo&.bar(<<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n",
    // 25 behaves like correct cases correct cases accepts method chain with heredoc argument correc
    "do_something(\n  Model\n    .foo&.bar(<<~CODE)\n      code\n    CODE\n    .baz(<<~CODE))\n      code\n    CODE\n",
    // 38 behaves like correct cases correct cases accepts method chain with safe navigation with he
    "do_something(\n  Model\n    .foo&.bar(<<~CODE)\n      code\n    CODE\n    &.baz(<<~CODE))\n      code\n    CODE\n",
    // 51 behaves like correct cases correct cases accepts method with heredoc argument of proc corr
    "outer_method(-> {\n  inner_method&.chain(<<~CODE)\n    code\n  CODE\n})\n",
    // 61 behaves like correct cases correct cases accepts method with heredoc argument of proc with
    "outer_method(\n  -> {\n    inner_method&.chain(<<~CODE)\n      code\n    CODE\n  }\n)\n",
    // 73 behaves like correct cases correct cases accepts method with heredoc argument of proc corr
    "outer_method( -> {\n  inner_method&.chain( <<~CODE )\n    code\n  CODE\n} )\n",
    // 83 behaves like correct cases correct cases accepts double correct case nested
    "baz(bar(foo&.bar(<<-SQL, <<-NOSQL)))\n  foo\nSQL\n  bar\nNOSQL\n",
    // 93 behaves like correct cases correct cases accepts double correct case new line
    "foo&.bar(\n  <<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n",
    // 104 behaves like correct cases correct cases accepts when there is an argument between a hered
    "foo&.bar(<<~TEXT,\n    Lots of\n    Lovely\n    Text\n  TEXT\n  some_arg: { foo: \"bar\" }\n)\n",
    // 116 behaves like correct cases correct cases accepts when heredoc is a method argument in a pa
    "foo(bar do\n  baz&.quux <<~EOS\n  EOS\nend)\n",
    // 125 behaves like correct cases correct cases accepts when heredoc is a branch body in a method
    "foo(unless condition\n  bar&.baz(<<~EOS)\n    text\n  EOS\nend)\n",
    // 135 behaves like correct cases correct cases accepts when heredoc is a branch body in a nested
    "foo(unless condition\n  bar(baz&.quux(<<~EOS))\n    text\n  EOS\nend)\n",
    // 145 behaves like correct cases correct cases accepts correct case with other param after
    "foo&.bar(<<-SQL, 123)\n  foo\nSQL\n",
    // 153 behaves like correct cases correct cases accepts correct case with other param before
    "foo&.bar(123, <<-SQL)\n  foo\nSQL\n",
    // 161 behaves like correct cases correct cases accepts hash correct case
    "foo&.bar(foo: <<-SQL)\n  foo\nSQL\n",
    // 169 behaves like correct cases correct cases accepts a HEREDOC in a hash followed by another p
    "foo&.bar(\n  baz: <<-SQL,\n    foo\n  SQL\n  qux: 1\n)\n",
    // 180 behaves like correct cases correct cases accepts a HEREDOC in a hash whose braces close af
    "foo&.bar(:baz,\n  foo: {\n    qux: <<-SQL,\n      foo\n    SQL\n  }\n)\n",
    // 193 behaves like correct cases correct cases invocation after the HEREDOC ignores tr
    "foo&.bar(\n  <<-SQL.tr(\"z\", \"t\"))\n  baz\nSQL\n",
    // 202 behaves like correct cases correct cases invocation after the HEREDOC ignores random call
    "foo&.bar(\n  <<-TEXT.foo)\n  foobarbaz\nTEXT\n",
    // 211 behaves like correct cases correct cases invocation after the HEREDOC ignores random call
    "foo&.bar(\n  <<-TEXT\n  foobarbaz\nTEXT\n.foo\n)\n",
];

type Reported = (&'static str, &'static [(u32, u32)], &'static str);

/// Every pinned example with offenses, and its single-pass correction.
const REPORTED: &[Reported] = &[
    // 228 behaves like incorrect cases incorrect cases simple incorrect case registers an offense
    (
        "foo.bar(<<-SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo.bar(<<-SQL)\n  foo\nSQL\n",
    ),
    // 246 behaves like incorrect cases incorrect cases simple incorrect case with call after registe
    (
        "foo.bar(<<~SQL\n  foo\nSQL\n).baz\n",
        &[(4, 1)],
        "foo.bar(<<~SQL)\n  foo\nSQL\n.baz\n",
    ),
    // 265 behaves like incorrect cases incorrect cases simple incorrect case with call after with sa
    (
        "foo.bar(<<~SQL\n  foo\nSQL\n)&.baz\n",
        &[(4, 1)],
        "foo.bar(<<~SQL)\n  foo\nSQL\n&.baz\n",
    ),
    // 284 behaves like incorrect cases incorrect cases simple incorrect case with call after trailin
    (
        "foo.bar(<<~SQL,\n  foo\nSQL\n).baz\n",
        &[(4, 1)],
        "foo.bar(<<~SQL)\n  foo\nSQL\n.baz\n",
    ),
    // 303 behaves like incorrect cases incorrect cases simple incorrect case hash registers an offen
    (
        "foo.bar(foo: <<-SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo.bar(foo: <<-SQL)\n  foo\nSQL\n",
    ),
    // 321 behaves like incorrect cases incorrect cases nested incorrect case registers an offense
    (
        "foo(foo.bar(<<-SQL)\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo(foo.bar(<<-SQL))\n  foo\nSQL\n",
    ),
    // 339 behaves like incorrect cases incorrect cases simple incorrect case squiggles registers an
    (
        "foo.bar(<<~SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo.bar(<<~SQL)\n  foo\nSQL\n",
    ),
    // 357 behaves like incorrect cases incorrect cases simple incorrect case comma registers an offe
    (
        "foo.bar(<<-SQL,\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo.bar(<<-SQL)\n  foo\nSQL\n",
    ),
    // 375 behaves like incorrect cases incorrect cases simple incorrect case comma with spaces regis
    (
        "foo.bar(<<-SQL    ,\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo.bar(<<-SQL)\n  foo\nSQL\n",
    ),
    // 393 behaves like incorrect cases incorrect cases simple incorrect case comma with spaces and c
    (
        "foo.bar(<<-SQL    ,\n  foo,\nSQL\n)\n",
        &[(4, 1)],
        "foo.bar(<<-SQL)\n  foo,\nSQL\n",
    ),
    // 411 behaves like incorrect cases incorrect cases double incorrect case registers an offense
    (
        "foo.bar(<<-SQL, <<-NOSQL\n  foo\nSQL\n  bar\nNOSQL\n)\n",
        &[(6, 1)],
        "foo.bar(<<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n",
    ),
    // 433 behaves like incorrect cases incorrect cases double incorrect case new line chained calls
    (
        "foo.bar(<<-SQL, <<-NOSQL\n  foo\nSQL\n  bar\nNOSQL\n).baz(123).quux(456)\n",
        &[(6, 1)],
        "foo.bar(<<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n.baz(123).quux(456)\n",
    ),
    // 456 behaves like incorrect cases incorrect cases incorrect case with other param after registe
    (
        "foo.bar(<<-SQL, 123\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo.bar(<<-SQL, 123)\n  foo\nSQL\n",
    ),
    // 474 behaves like incorrect cases incorrect cases incorrect case with other param before regist
    (
        "foo.bar(123, <<-SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo.bar(123, <<-SQL)\n  foo\nSQL\n",
    ),
    // 492 behaves like incorrect cases incorrect cases incorrect case with other param before constr
    (
        "Foo.new(123, <<-SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "Foo.new(123, <<-SQL)\n  foo\nSQL\n",
    ),
    // 510 behaves like incorrect cases incorrect cases incorrect case with other param before constr
    (
        "raise Foo.new(123, <<-SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "raise Foo.new(123, <<-SQL)\n  foo\nSQL\n",
    ),
    // 528 behaves like incorrect cases incorrect cases incorrect case nested method call with comma
    (
        "bar(\n  foo.bar(123, <<-SQL\n    foo\n  SQL\n  ),\n  456,\n  789,\n)\n",
        &[(5, 3)],
        "bar(\n  foo.bar(123, <<-SQL),\n    foo\n  SQL\n  456,\n  789,\n)\n",
    ),
    // 554 behaves like incorrect cases incorrect cases incorrect case in array with spaced out comma
    (
        "[\n  foo.bar(123, <<-SQL\n    foo\n  SQL\n  )      ,\n  456,\n  789,\n]\n",
        &[(5, 3)],
        "[\n  foo.bar(123, <<-SQL),\n    foo\n  SQL\n  456,\n  789,\n]\n",
    ),
    // 580 behaves like incorrect cases incorrect cases incorrect case in array with double heredoc a
    (
        "[\n  foo.bar(123, <<-SQL, 456, 789, <<-NOSQL,\n    foo\n  SQL\n    bar\n  NOSQL\n  )      ,\n  456,\n  789,\n]\n",
        &[(7, 3)],
        "[\n  foo.bar(123, <<-SQL, 456, 789, <<-NOSQL),\n    foo\n  SQL\n    bar\n  NOSQL\n  456,\n  789,\n]\n",
    ),
    // 610 behaves like incorrect cases incorrect cases incorrect case in array with nested calls and
    (
        "[\n  foo(foo(foo.bar(123, <<-SQL, 456, 789, <<-NOSQL), 456), 400,\n    foo\n  SQL\n    bar\n  NOSQL\n  )      ,\n  456,\n  789,\n]\n",
        &[(7, 3)],
        "[\n  foo(foo(foo.bar(123, <<-SQL, 456, 789, <<-NOSQL), 456), 400),\n    foo\n  SQL\n    bar\n  NOSQL\n  456,\n  789,\n]\n",
    ),
    // 640 behaves like incorrect cases incorrect cases complex incorrect case with multiple calls de
    (
        "query.order(foo.bar(<<-SQL,\n  foo\nSQL\n                    ))\n",
        &[(4, 21)],
        "query.order(foo.bar(<<-SQL)\n  foo\nSQL\n                    )\n",
    ),
    // 657 behaves like incorrect cases incorrect cases complex incorrect case with multiple calls de
    (
        "query.order(foo.bar(<<-SQL)\n  foo\nSQL\n                    )\n",
        &[(4, 21)],
        "query.order(foo.bar(<<-SQL))\n  foo\nSQL\n",
    ),
    // 675 behaves like incorrect cases incorrect cases complex chained incorrect case with multiple
    (
        "query.joins({\n  foo: []\n}).order(foo.bar(<<-SQL),\n  bar\nSQL\n        )\n",
        &[(6, 9)],
        "query.joins({\n  foo: []\n}).order(foo.bar(<<-SQL))\n  bar\nSQL\n",
    ),
    // 697 behaves like incorrect cases incorrect cases double case new line detects and fixes
    (
        "foo.bar(\n  <<-SQL, <<-NOSQL\n  foo\nSQL\n  bar\nNOSQL\n)\n",
        &[(7, 1)],
        "foo.bar(\n  <<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n",
    ),
    // 228 behaves like incorrect cases incorrect cases simple incorrect case registers an offense
    (
        "foo&.bar(<<-SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo&.bar(<<-SQL)\n  foo\nSQL\n",
    ),
    // 246 behaves like incorrect cases incorrect cases simple incorrect case with call after registe
    (
        "foo&.bar(<<~SQL\n  foo\nSQL\n).baz\n",
        &[(4, 1)],
        "foo&.bar(<<~SQL)\n  foo\nSQL\n.baz\n",
    ),
    // 265 behaves like incorrect cases incorrect cases simple incorrect case with call after with sa
    (
        "foo&.bar(<<~SQL\n  foo\nSQL\n)&.baz\n",
        &[(4, 1)],
        "foo&.bar(<<~SQL)\n  foo\nSQL\n&.baz\n",
    ),
    // 284 behaves like incorrect cases incorrect cases simple incorrect case with call after trailin
    (
        "foo&.bar(<<~SQL,\n  foo\nSQL\n).baz\n",
        &[(4, 1)],
        "foo&.bar(<<~SQL)\n  foo\nSQL\n.baz\n",
    ),
    // 303 behaves like incorrect cases incorrect cases simple incorrect case hash registers an offen
    (
        "foo&.bar(foo: <<-SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo&.bar(foo: <<-SQL)\n  foo\nSQL\n",
    ),
    // 321 behaves like incorrect cases incorrect cases nested incorrect case registers an offense
    (
        "foo(foo&.bar(<<-SQL)\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo(foo&.bar(<<-SQL))\n  foo\nSQL\n",
    ),
    // 339 behaves like incorrect cases incorrect cases simple incorrect case squiggles registers an
    (
        "foo&.bar(<<~SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo&.bar(<<~SQL)\n  foo\nSQL\n",
    ),
    // 357 behaves like incorrect cases incorrect cases simple incorrect case comma registers an offe
    (
        "foo&.bar(<<-SQL,\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo&.bar(<<-SQL)\n  foo\nSQL\n",
    ),
    // 375 behaves like incorrect cases incorrect cases simple incorrect case comma with spaces regis
    (
        "foo&.bar(<<-SQL    ,\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo&.bar(<<-SQL)\n  foo\nSQL\n",
    ),
    // 393 behaves like incorrect cases incorrect cases simple incorrect case comma with spaces and c
    (
        "foo&.bar(<<-SQL    ,\n  foo,\nSQL\n)\n",
        &[(4, 1)],
        "foo&.bar(<<-SQL)\n  foo,\nSQL\n",
    ),
    // 411 behaves like incorrect cases incorrect cases double incorrect case registers an offense
    (
        "foo&.bar(<<-SQL, <<-NOSQL\n  foo\nSQL\n  bar\nNOSQL\n)\n",
        &[(6, 1)],
        "foo&.bar(<<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n",
    ),
    // 433 behaves like incorrect cases incorrect cases double incorrect case new line chained calls
    (
        "foo&.bar(<<-SQL, <<-NOSQL\n  foo\nSQL\n  bar\nNOSQL\n).baz(123).quux(456)\n",
        &[(6, 1)],
        "foo&.bar(<<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n.baz(123).quux(456)\n",
    ),
    // 456 behaves like incorrect cases incorrect cases incorrect case with other param after registe
    (
        "foo&.bar(<<-SQL, 123\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo&.bar(<<-SQL, 123)\n  foo\nSQL\n",
    ),
    // 474 behaves like incorrect cases incorrect cases incorrect case with other param before regist
    (
        "foo&.bar(123, <<-SQL\n  foo\nSQL\n)\n",
        &[(4, 1)],
        "foo&.bar(123, <<-SQL)\n  foo\nSQL\n",
    ),
    // 528 behaves like incorrect cases incorrect cases incorrect case nested method call with comma
    (
        "bar(\n  foo&.bar(123, <<-SQL\n    foo\n  SQL\n  ),\n  456,\n  789,\n)\n",
        &[(5, 3)],
        "bar(\n  foo&.bar(123, <<-SQL),\n    foo\n  SQL\n  456,\n  789,\n)\n",
    ),
    // 554 behaves like incorrect cases incorrect cases incorrect case in array with spaced out comma
    (
        "[\n  foo&.bar(123, <<-SQL\n    foo\n  SQL\n  )      ,\n  456,\n  789,\n]\n",
        &[(5, 3)],
        "[\n  foo&.bar(123, <<-SQL),\n    foo\n  SQL\n  456,\n  789,\n]\n",
    ),
    // 580 behaves like incorrect cases incorrect cases incorrect case in array with double heredoc a
    (
        "[\n  foo&.bar(123, <<-SQL, 456, 789, <<-NOSQL,\n    foo\n  SQL\n    bar\n  NOSQL\n  )      ,\n  456,\n  789,\n]\n",
        &[(7, 3)],
        "[\n  foo&.bar(123, <<-SQL, 456, 789, <<-NOSQL),\n    foo\n  SQL\n    bar\n  NOSQL\n  456,\n  789,\n]\n",
    ),
    // 610 behaves like incorrect cases incorrect cases incorrect case in array with nested calls and
    (
        "[\n  foo(foo(foo&.bar(123, <<-SQL, 456, 789, <<-NOSQL), 456), 400,\n    foo\n  SQL\n    bar\n  NOSQL\n  )      ,\n  456,\n  789,\n]\n",
        &[(7, 3)],
        "[\n  foo(foo(foo&.bar(123, <<-SQL, 456, 789, <<-NOSQL), 456), 400),\n    foo\n  SQL\n    bar\n  NOSQL\n  456,\n  789,\n]\n",
    ),
    // 640 behaves like incorrect cases incorrect cases complex incorrect case with multiple calls de
    (
        "query.order(foo&.bar(<<-SQL,\n  foo\nSQL\n                    ))\n",
        &[(4, 21)],
        "query.order(foo&.bar(<<-SQL)\n  foo\nSQL\n                    )\n",
    ),
    // 657 behaves like incorrect cases incorrect cases complex incorrect case with multiple calls de
    (
        "query.order(foo&.bar(<<-SQL)\n  foo\nSQL\n                    )\n",
        &[(4, 21)],
        "query.order(foo&.bar(<<-SQL))\n  foo\nSQL\n",
    ),
    // 675 behaves like incorrect cases incorrect cases complex chained incorrect case with multiple
    (
        "query.joins({\n  foo: []\n}).order(foo&.bar(<<-SQL),\n  bar\nSQL\n        )\n",
        &[(6, 9)],
        "query.joins({\n  foo: []\n}).order(foo&.bar(<<-SQL))\n  bar\nSQL\n",
    ),
    // 697 behaves like incorrect cases incorrect cases double case new line detects and fixes
    (
        "foo&.bar(\n  <<-SQL, <<-NOSQL\n  foo\nSQL\n  bar\nNOSQL\n)\n",
        &[(7, 1)],
        "foo&.bar(\n  <<-SQL, <<-NOSQL)\n  foo\nSQL\n  bar\nNOSQL\n",
    ),
];

#[test]
fn pinned_accepted_examples() {
    for source in ACCEPTED {
        assert_eq!(positions(source), vec![], "{source}");
    }
}

#[test]
fn pinned_reported_examples_and_single_pass_corrections() {
    for (source, expected, after) in REPORTED {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(l, c)| (l, c, MSG.to_owned()))
            .collect();
        assert_eq!(positions(source), expected, "{source}");
        assert_correction(&HeredocArgumentClosingParenthesis, source, after);
    }
}

#[test]
fn metadata_is_opt_in_and_incompatible_with_trailing_comma_in_arguments() {
    let cop = HeredocArgumentClosingParenthesis;
    assert!(!cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.severity(), Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert_eq!(
        cop.autocorrect_incompatible_with(),
        &["Style/TrailingCommaInArguments"]
    );
    // Not run by default.
    let source = "foo(<<~A\n  x\nA\n)\n";
    assert!(
        lint_str(source)
            .iter()
            .all(|o| o.cop != "Layout/HeredocArgumentClosingParenthesis")
    );
    assert_eq!(positions(source).len(), 1);
}

#[test]
fn ancestry_and_lookalikes() {
    for source in [
        // An `end` keyword anywhere above the call (here a `def` and a
        // `do ... end` block of the call itself) skips it, as upstream.
        "def f\n  foo(<<~A\n    x\n  A\n  )\nend\n",
        "foo(<<~A\n  x\nA\n) do\nend\n",
        // Not a parser `send`/`csend`: index, `super`, `yield`.
        "foo[<<~A\n  x\nA\n]\n",
        "def f\n  super(<<~A\n    x\n  A\n  )\nend\n",
        // Without parentheses.
        "foo <<~A\n  x\nA\n",
        // A heredoc that is not an argument (a receiver with a multi-line
        // call, an array element, a splat, a block argument).
        "foo(<<~A\n  x\nA\n.strip\n)\n",
        "foo([<<~A\n  x\nA\n])\n",
        "foo(*<<~A\n  x\nA\n)\n",
        // A lambda or brace block body stops the climb.
        "outer(-> {\n  inner(<<~A)\n    x\n  A\n})\n",
        "outer(foo {\n  inner(<<~A)\n    x\n  A\n})\n",
        // Code or a comment between the heredoc and `)`.
        "foo(<<~A\n  x\nA\n# c\n)\n",
    ] {
        assert_eq!(positions(source), vec![], "{source:?}");
    }
    let msg = MSG.to_owned();
    for (source, at) in [
        ("foo(<<~A\n  x\nA\n)\n", (4, 1)),
        ("foo&.bar(k: {j: <<~A}\n  x\nA\n)\n", (4, 1)),
        ("foo(bar(<<~A)\n  x\nA\n)\n", (4, 1)),
        ("a.b = c(<<~A\n  x\nA\n)\n", (4, 1)),
        ("x = [\n  foo(<<~A\n    x\n  A\n  ),\n]\n", (5, 3)),
    ] {
        assert_eq!(
            positions(source),
            vec![(at.0, at.1, msg.clone())],
            "{source:?}"
        );
    }
}

#[test]
fn canonical_relocation_preserves_bytes_and_values() {
    for (before, after) in [
        ("foo(<<~A\n  x\nA\n)\n", "foo(<<~A)\n  x\nA\n"),
        ("foo(<<~A\r\n  x\r\nA\r\n)\r\n", "foo(<<~A)\r\n  x\r\nA\r\n"),
        ("foo(<<~A, 1,\n  x\nA\n  )\n", "foo(<<~A, 1)\n  x\nA\n"),
        ("foo(<<~A # c\n  x\nA\n)\n", "foo(<<~A) # c\n  x\nA\n"),
        // Only a comma after blanks is punctuation; a comma inside a comment
        // never ends a deletion that would turn commented-out code live.
        (
            "x = String(<<~A # important, .upcase\n  abc\nA\n)\np x\n",
            "x = String(<<~A) # important, .upcase\n  abc\nA\np x\n",
        ),
        ("foo(<<~A \t,\n  x\nA\n)\n", "foo(<<~A)\n  x\nA\n"),
        (
            "foo(<<~'A'\n  #{x}, )\nA\n)\n",
            "foo(<<~'A')\n  #{x}, )\nA\n",
        ),
        (
            "foo(\"\u{e9}\", <<~A\n  \u{e9}\nA\n)\n",
            "foo(\"\u{e9}\", <<~A)\n  \u{e9}\nA\n",
        ),
    ] {
        assert_correction(&HeredocArgumentClosingParenthesis, before, after);
    }
}

#[test]
fn listed_source_observers_hold_the_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p caller",
        "p SCRIPT_LINES__",
    ] {
        let source = format!("foo(<<~A\n  x\nA\n)\n{observer}\n");
        let found = correction_offenses(&HeredocArgumentClosingParenthesis, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
}
