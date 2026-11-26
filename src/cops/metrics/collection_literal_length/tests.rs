//! Examples ported from RuboCop's
//! `spec/rubocop/cop/metrics/collection_literal_length_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! The spec builds its sources from `length_threshold` (10); the generators
//! below build them the same way for any count. At 10 they reproduce all 13
//! pinned sources byte for byte (each SHA-256 was checked against the fixed
//! replay in the pinned RuboCop reference run when the rows were
//! ported; the hash `to_s` is the spec runtime's `{0=>0, …}` form), and at
//! the fixed `Max: 250` none of them is reported, as recorded. Each upstream
//! example is then ported at the fixed defaults with 249, 250 and 251
//! entries: 250 and more are reported over the whole literal.
use super::*;
use crate::pipeline::lint_str_with;

const COP: &str = "Metrics/CollectionLiteralLength";

/// `(line, col, end line)` of every finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{}",
        &source[..source.len().min(200)]
    );
    let mut found: Vec<_> = lint_str_with(&[&CollectionLiteralLength], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Refactor);
            assert!(o.correction.is_none());
            assert_eq!(o.message, MSG);
            (o.line, o.col, o.end_line)
        })
        .collect();
    found.sort_unstable();
    found
}

fn items(range: std::ops::Range<usize>, item: impl Fn(usize) -> String) -> Vec<String> {
    range.map(item).collect()
}

fn ints(range: std::ops::Range<usize>) -> Vec<String> {
    items(range, |n| n.to_string())
}

fn pairs(range: std::ops::Range<usize>) -> Vec<String> {
    items(range, |n| format!("{n} => {n}"))
}

/// `large_array.to_s` / `large_hash.to_s`: one line.
fn array_line(range: std::ops::Range<usize>) -> String {
    format!("[{}]", ints(range).join(", "))
}

fn hash_line(range: std::ops::Range<usize>) -> String {
    format!("{{{}}}", items(range, |n| format!("{n}=>{n}")).join(", "))
}

fn set_line(range: std::ops::Range<usize>) -> String {
    format!("Set[{}]", ints(range).join(", "))
}

/// `open`, one entry per line indented by two, `close`.
fn per_line(open: &str, entries: &[String], close: &str) -> String {
    format!("{open}\n  {}\n{close}\n", entries.join(",\n  "))
}

/// The 13 spec sources for `length_threshold` `n`, with their spec lines
/// and whether the spec expects an offense (at `Max: n`).
fn spec_sources(n: usize) -> Vec<(u32, String, bool)> {
    vec![
        (19, format!("{}\n", array_line(0..n)), true),
        (26, per_line("[", &ints(0..n), "]"), true),
        (36, array_line(1..n), false),
        (40, per_line("[", &ints(1..n), "]"), false),
        (49, format!("{}\n", hash_line(0..n)), true),
        (56, per_line("{", &pairs(0..n), "}"), true),
        (66, hash_line(1..n), false),
        (70, per_line("{", &pairs(1..n), "}"), false),
        (79, format!("{}\n", set_line(0..n)), true),
        (86, per_line("Set[", &ints(0..n), "]"), true),
        (96, set_line(1..n), false),
        (100, per_line("Set[", &ints(1..n), "]"), false),
        (108, per_line("foo[", &ints(0..n), "]"), false),
    ]
}

#[test]
fn pinned_examples_replay_the_fixed_results() {
    let sources = spec_sources(10);
    assert_eq!(sources.len(), 13);
    for (line, source, _) in sources {
        assert_eq!(findings(&source), [], "spec line {line}");
    }
}

#[test]
fn upstream_examples_at_the_fixed_max() {
    // The spec's offending examples hold exactly the threshold; its
    // offense-free ones one entry fewer.
    for (line, source, offense) in spec_sources(250) {
        let lines = source.lines().count() as u32;
        let want: &[(u32, u32, u32)] = if offense {
            &[(1, 1, lines.max(1))]
        } else {
            &[]
        };
        assert_eq!(findings(&source), want, "spec line {line}");
    }
}

#[test]
fn boundaries_249_250_and_251() {
    for (count, reported) in [(249, false), (250, true), (251, true)] {
        let want: &[(u32, u32, u32)] = if reported { &[(1, 1, 1)] } else { &[] };
        let per_line_want: &[(u32, u32, u32)] = if reported {
            &[(1, 1, count as u32 + 2)]
        } else {
            &[]
        };
        assert_eq!(findings(&array_line(0..count)), want, "array {count}");
        assert_eq!(findings(&hash_line(0..count)), want, "hash {count}");
        assert_eq!(findings(&set_line(0..count)), want, "Set {count}");
        assert_eq!(
            findings(&per_line("[", &ints(0..count), "]")),
            per_line_want
        );
        assert_eq!(
            findings(&per_line("{", &pairs(0..count), "}")),
            per_line_want
        );
        assert_eq!(
            findings(&per_line("Set[", &ints(0..count), "]")),
            per_line_want
        );
        assert_eq!(findings(&per_line("foo[", &ints(0..count), "]")), []);
    }
}

#[test]
fn every_parser_array_kind_counts_its_children() {
    let words: Vec<_> = items(0..250, |n| format!("w{n}"));
    for source in [
        format!("%w[{}]", words.join(" ")),
        format!("%i[{}]", words.join(" ")),
        format!("%W[{} #{{x}}]", words[1..].join(" ")),
        format!("%I[{}]", words.join(" ")),
        format!("[{}]", items(0..250, |n| format!("*a{n}")).join(", ")),
        format!(
            "[{}]",
            items(0..249, |n| format!("[{n}]")).join(", ") + ", []"
        ),
    ] {
        assert_eq!(findings(&source), [(1, 1, 1)], "{}", &source[..20]);
    }
    // Bracketless right-hand sides are arrays starting at the first entry.
    assert_eq!(
        findings(&format!("a = {}", ints(0..250).join(", "))),
        [(1, 5, 1)]
    );
    assert_eq!(
        findings(&format!("a, b = {}", ints(0..250).join(", "))),
        [(1, 8, 1)]
    );
    assert_eq!(
        findings(&format!("a = *x, {}", ints(0..249).join(", "))),
        [(1, 5, 1)]
    );
    assert_eq!(findings(&format!("a = {}", ints(0..249).join(", "))), []);
}

#[test]
fn nested_collections_are_measured_separately() {
    // The outer array has 2 entries, the inner one 250.
    let source = format!("[0, {}]", array_line(0..250));
    assert_eq!(findings(&source), [(1, 5, 1)]);
    // Both reported.
    let source = format!("[{}, {}]", ints(0..249).join(", "), array_line(0..250));
    let col = ints(0..249).join(", ").len() as u32 + 4;
    assert_eq!(findings(&source), [(1, 1, 1), (1, col, 1)]);
}

#[test]
fn hash_entries_of_every_kind_and_keyword_arguments() {
    let entries = |count: usize| items(0..count, |n| format!("k{n}: {n}"));
    let mut mixed = entries(247);
    mixed.extend(["**opts".into(), "\"s\" => 1".into(), "k:".into()]);
    assert_eq!(
        findings(&format!("x = {{{}}}", mixed.join(", "))),
        [(1, 5, 1)]
    );
    // Braceless keyword arguments are parser hashes too.
    assert_eq!(
        findings(&format!("foo({})", entries(250).join(", "))),
        [(1, 5, 1)]
    );
    assert_eq!(
        findings(&format!("foo 1, {}", entries(250).join(", "))),
        [(1, 8, 1)]
    );
    assert_eq!(
        findings(&format!("x[{}]", entries(250).join(", "))),
        [(1, 3, 1)]
    );
    assert_eq!(findings(&format!("foo({})", entries(249).join(", "))), []);
    // 250 arguments are not a collection.
    assert_eq!(findings(&format!("foo({})", ints(0..250).join(", "))), []);
}

#[test]
fn set_receivers_and_call_forms() {
    let args = ints(0..250).join(", ");
    assert_eq!(findings(&format!("::Set[{args}]")), [(1, 1, 1)]);
    assert_eq!(findings(&format!("x = Set.[]({args})")), [(1, 5, 1)]);
    assert_eq!(
        findings(&format!("Set[{}, *rest]", ints(0..249).join(", "))),
        [(1, 1, 1)]
    );
    assert_eq!(
        findings(&format!("Set[{}, &blk]", ints(0..249).join(", "))),
        [(1, 1, 1)]
    );
    assert_eq!(findings(&format!("Set[{}]", ints(0..249).join(", "))), []);
    // The `Set[…]` read of an index op-assignment is an `index` send.
    for op in ["+=", "||=", "&&="] {
        assert_eq!(
            findings(&format!("Set[{args}] {op} 1")),
            [(1, 1, 1)],
            "{op}"
        );
    }
    // A literal block is not an argument and not part of the range.
    assert_eq!(findings(&format!("Set.[]({args}) do\nend")), [(1, 1, 1)]);
    for receiver in [
        "Foo::Set",
        "Sets",
        "set",
        "SortedSet",
        "Set.new",
        "self",
        "Hash",
    ] {
        assert_eq!(findings(&format!("{receiver}[{args}]")), [], "{receiver}");
    }
    for source in [
        format!("Set&.[]({args})"),
        format!("Set.new([{}])", ints(0..249).join(", ")),
        format!("Set.call({args})"),
        format!("Set[{}] = 1", ints(0..250).join(", ")),
        format!("foo({args})"),
        format!("[{}].to_set", ints(0..249).join(", ")),
    ] {
        assert_eq!(findings(&source), [], "{}", &source[..20]);
    }
}

#[test]
fn rescue_exception_lists_are_parser_arrays() {
    let list = |count: usize| items(0..count, |n| format!("E{n}")).join(", ");
    let source = format!("begin\nrescue {} => e\nend\n", list(250));
    assert_eq!(findings(&source), [(2, 8, 2)]);
    let source = format!("begin\nrescue A\nrescue {}\nend\n", list(250));
    assert_eq!(findings(&source), [(3, 8, 3)]);
    assert_eq!(findings(&format!("begin\nrescue {}\nend\n", list(249))), []);
}

#[test]
fn lookalikes_and_patterns_are_not_collections() {
    let ints = ints(0..250).join(", ");
    let pairs = items(0..250, |n| format!("k{n}:")).join(", ");
    for source in [
        format!(
            "case x\nin [{}]\nend\n",
            items(0..250, |n| format!("a{n}")).join(", ")
        ),
        format!("case x\nin {{{pairs}}}\nend\n"),
        format!("case x\nwhen {ints}\nend\n"),
        format!("x => [{}]\n", items(0..250, |n| format!("a{n}")).join(", ")),
        format!(
            "def m({})\nend\n",
            items(0..250, |n| format!("a{n}")).join(", ")
        ),
        format!("return {ints}\n"),
        format!("def m\n  yield {ints}\nend\n"),
        format!("\"[{ints}]\"\n"),
        format!("# [{ints}]\n"),
        format!("{}, = 1\n", items(0..250, |n| format!("a{n}")).join(", ")),
    ] {
        assert_eq!(findings(&source), [], "{}", &source[..20]);
    }
}

#[test]
fn empty_crlf_and_multibyte_positions() {
    assert_eq!(findings(""), []);
    assert_eq!(findings("[]\n{}\nSet[]\n"), []);
    let source = per_line("x = [", &ints(0..250), "]").replace('\n', "\r\n");
    assert_eq!(findings(&source), [(1, 5, 252)]);
    let source = format!("\"é\"; {}\n", array_line(0..250));
    assert_eq!(findings(&source), [(1, 6, 1)]);
}
