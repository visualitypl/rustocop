//! Examples ported from RuboCop's
//! `spec/rubocop/cop/metrics/perceived_complexity_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! All 36 pinned examples run verbatim at the fixed defaults and match the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! source's SHA-256 was checked against it when the rows were ported): the
//! spec lowers `Max` to 0, 1 or 2, so none reaches the fixed `Max: 8`. Each
//! upstream score is reproduced at the spec's own `Max`, and each upstream
//! finding is ported at the fixed limit by adding plain `if` modifiers (one
//! point each) to the method until it is one point over, and one point
//! fewer is accepted. The `AllowedMethods`/`AllowedPatterns` examples are
//! reported at the fixed defaults, which allow nothing. The spec's
//! `expect_no_corrections` holds: there is no correction.
use super::*;
use crate::{
    cops::metrics::{cyclomatic_complexity::complexity, method_complexity::measured},
    pipeline::lint_str_with,
};

const COP: &str = "Metrics/PerceivedComplexity";

/// `(line, col, end line, message)` of every finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, u32, String)> {
    let mut found: Vec<_> = lint_str_with(&[&PerceivedComplexity::default()], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Refactor);
            assert!(o.correction.is_none());
            (o.line, o.col, o.end_line, o.message)
        })
        .collect();
    found.sort();
    found
}

/// `(line, col, name, perceived complexity)` of every method.
fn scores(source: &str) -> Vec<(u32, u32, String, usize)> {
    measured(source, |body| complexity(body, perceived_score))
}

fn message(name: &str, score: usize) -> String {
    format!("Perceived complexity for `{name}` is too high. [{score}/8]")
}

/// A pinned example: its spec line, declared custom options, source,
/// upstream findings `(line, col, method, score)` at those options and the
/// recorded fixed-default findings `(line, col, message)`.
struct Pinned {
    line: u32,
    options: &'static str,
    source: &'static str,
    upstream: &'static [(u32, u32, &'static str, &'static str)],
    fixed: &'static [(u32, u32, &'static str)],
}

impl Pinned {
    /// The spec's custom `Max`.
    fn max(&self) -> usize {
        let at = self.options.find("\"Max\":").unwrap() + 6;
        self.options[at..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap()
    }

    fn allows(&self) -> bool {
        self.options.contains("Allowed")
    }
}

const PINNED: &[Pinned] = &[
    Pinned {
        line: 8,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 16,
        options: "{\"Max\":1}",
        source: r##"def method_name
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 23,
        options: "{\"Max\":1}",
        source: r##"define_method :method_name do
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 30,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo
end

if first_condition then
  call_foo if second_condition && third_condition
  call_bar if fourth_condition || fifth_condition
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 43,
        options: "{\"Max\":1}",
        source: r##"def self.method_name
  call_foo if some_condition
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 52,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo unless some_condition
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 61,
        options: "{\"Max\":1}",
        source: r##"def method_name
  if first_condition then
    call_foo
  elsif second_condition then
    call_bar
  else
    call_bam
  end
end
"##,
        upstream: &[(1, 1, "method_name", "4")],
        fixed: &[],
    },
    Pinned {
        line: 76,
        options: "{\"Max\":1}",
        source: r##"def method_name
  value = some_condition ? 1 : 2
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 85,
        options: "{\"Max\":1}",
        source: r##"def method_name
  while some_condition do
    call_foo
  end
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 96,
        options: "{\"Max\":1}",
        source: r##"def method_name
  until some_condition do
    call_foo
  end
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 107,
        options: "{\"Max\":1}",
        source: r##"def method_name
  for i in 1..2 do
    call_method
  end
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 118,
        options: "{\"Max\":1}",
        source: r##"def method_name
  begin
    call_foo
  rescue Exception
    call_bar
  end
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 131,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case value
  when 1 then call_foo_1
  when 2 then call_foo_2
  when 3 then call_foo_3
  when 4 then call_foo_4
  end
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 145,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case
  when value == 1
    call_foo
  when value == 2
    call_bar
  end
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 159,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case
  when value == 1
    call_foo
  when value == 2
    call_bar
  else
    call_baz
  end
end
"##,
        upstream: &[(1, 1, "method_name", "4")],
        fixed: &[],
    },
    Pinned {
        line: 175,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case value
  in 1 then call_foo_1
  in 2 then call_foo_2
  in 3 then call_foo_3
  end
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 190,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case value
  in Integer then call_foo_1
  in String then call_foo_2
  in Float then call_foo_3
  end
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 205,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case value
  in [1, a] then call_foo_1
  in {b:} then call_foo_2
  in String => c then call_foo_3
  end
end
"##,
        upstream: &[(1, 1, "method_name", "4")],
        fixed: &[],
    },
    Pinned {
        line: 220,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case value
  in Integer if value > 0 then call_foo_1
  in Integer if value < 0 then call_foo_2
  in Integer then call_foo_3
  end
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 235,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case value
  in [1, a] then call_foo_1
  in {b:} then call_foo_2
  in 3 then call_foo_3
  end
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 250,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo && call_bar
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 259,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo and call_bar
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 268,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo || call_bar
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 277,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo or call_bar
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 286,
        options: "{\"Max\":1}",
        source: r##"def method_name
  if first_condition then
    call_foo if second_condition && third_condition
    call_bar if fourth_condition || fifth_condition
  end
end
"##,
        upstream: &[(1, 1, "method_name", "6")],
        fixed: &[],
    },
    Pinned {
        line: 298,
        options: "{\"Max\":1}",
        source: r##"def method_name_1
  call_foo if some_condition
end

def method_name_2
  call_foo if some_condition
end
"##,
        upstream: &[(1, 1, "method_name_1", "2"), (5, 1, "method_name_2", "2")],
        fixed: &[],
    },
    Pinned {
        line: 312,
        options: "{\"Max\":1}",
        source: r##"define_method :method_name do
  call_foo if some_condition
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 321,
        options: "{\"Max\":1}",
        source: r##"def method_name
  bar.baz(:qux) do |x|
    raise x
  end
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 331,
        options: "{\"Max\":1}",
        source: r##"def method_name
  ary.each do |x|
    foo(x)
  end
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 346,
        options: "{\"AllowedMethods\":[\"foo\"],\"Max\":0}",
        source: r##"def foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 354,
        options: "{\"AllowedMethods\":[\"foo\"],\"Max\":0}",
        source: r##"def self.foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 362,
        options: "{\"AllowedMethods\":[\"foo\"],\"Max\":0}",
        source: r##"define_method :foo do
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 374,
        options: "{\"AllowedPatterns\":[{\"ruby_regexp_source\":\"foo\",\"ruby_regexp_options\":0}],\"Max\":0}",
        source: r##"def foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 382,
        options: "{\"AllowedPatterns\":[{\"ruby_regexp_source\":\"foo\",\"ruby_regexp_options\":0}],\"Max\":0}",
        source: r##"def self.foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 390,
        options: "{\"AllowedPatterns\":[{\"ruby_regexp_source\":\"foo\",\"ruby_regexp_options\":0}],\"Max\":0}",
        source: r##"define_method :foo do
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 402,
        options: "{\"Max\":2}",
        source: r##"def method_name                   # 1
  if first_condition then         # 2
    call_foo
  else                            # 3
    if second_condition then      # 4
      call_bar
    else                          # 5
      call_bam if third_condition # 6
    end
    call_baz if fourth_condition  # 7
  end
end
"##,
        upstream: &[(1, 1, "method_name", "7")],
        fixed: &[],
    },
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 36);
    for p in PINNED {
        let expected: Vec<_> = p
            .fixed
            .iter()
            .map(|&(l, c, m)| (l, c, m.to_owned()))
            .collect();
        let got: Vec<_> = findings(p.source)
            .into_iter()
            .map(|(l, c, _, m)| (l, c, m))
            .collect();
        assert_eq!(got, expected, "spec line {}", p.line);
    }
}

#[test]
fn upstream_scores_at_the_spec_limits() {
    for p in PINNED.iter().filter(|p| !p.allows()) {
        let over: Vec<_> = scores(p.source)
            .into_iter()
            .filter(|s| s.3 > p.max())
            .map(|(l, c, n, s)| (l, c, n, s.to_string()))
            .collect();
        let upstream: Vec<_> = p
            .upstream
            .iter()
            .map(|&(l, c, n, s)| (l, c, n.to_owned(), s.to_owned()))
            .collect();
        assert_eq!(over, upstream, "spec line {}", p.line);
    }
}

/// `source` with `extra` plain `if` modifiers (one point each) at the top of
/// every method, and the line shift of a method starting on `line`.
fn pad(source: &str, extra: usize) -> (String, impl Fn(u32) -> u32) {
    let mut out = String::new();
    let mut starts = Vec::new();
    for (i, line) in source.lines().enumerate() {
        out.push_str(line);
        out.push('\n');
        let code = line.trim_start();
        if code.starts_with("def ") || code.starts_with("define_method") {
            starts.push(i as u32 + 1);
            let indent = &line[..line.len() - code.len()];
            for k in 0..extra {
                out.push_str(&format!("{indent}  pad{k} if cond{k}\n"));
            }
        }
    }
    let shift =
        move |line: u32| line + starts.iter().filter(|&&s| s < line).count() as u32 * extra as u32;
    (out, shift)
}

#[test]
fn upstream_findings_at_the_fixed_limit() {
    let mut ported = 0;
    for p in PINNED.iter().filter(|p| !p.upstream.is_empty()) {
        for &(line, col, name, score) in p.upstream {
            let score: usize = score.parse().unwrap();
            let (source, shift) = pad(p.source, 9 - score);
            let found = findings(&source);
            let hit = found
                .iter()
                .find(|f| f.0 == shift(line) && f.1 == col)
                .unwrap_or_else(|| panic!("spec line {}: {found:?}", p.line));
            assert_eq!(hit.3, message(name, 9), "spec line {}", p.line);
            let (fewer, fewer_shift) = pad(p.source, 8 - score);
            let line = fewer_shift(line);
            assert!(
                findings(&fewer).iter().all(|f| (f.0, f.1) != (line, col)),
                "spec line {}",
                p.line
            );
            ported += 1;
        }
    }
    assert_eq!(ported, PORTED);
}

/// Upstream findings outside the allowed-method examples.
const PORTED: usize = 26;

#[test]
fn allowed_method_examples_are_reported_at_the_fixed_defaults() {
    let mut ported = 0;
    for p in PINNED.iter().filter(|p| p.allows()) {
        let (name, score) = scores(p.source)
            .into_iter()
            .map(|s| (s.2, s.3))
            .next()
            .unwrap();
        assert_eq!((name.as_str(), score), ("foo", 1), "spec line {}", p.line);
        let (source, _) = pad(p.source, 8);
        assert_eq!(
            findings(&source),
            [(1, 1, 11, message("foo", 9))],
            "spec line {}",
            p.line
        );
        ported += 1;
    }
    assert_eq!(ported, 6);
}

#[test]
fn case_weights_round_like_ruby() {
    let case = |whens: usize, subject: bool, else_body: &str| -> usize {
        let mut source = String::from("def m\n  case");
        if subject {
            source.push_str(" x");
        }
        source.push('\n');
        for k in 0..whens {
            source.push_str(&format!("  when c{k} then d\n"));
        }
        source.push_str(else_body);
        source.push_str("  end\nend\n");
        scores(&source)[0].3 - 1
    };
    // `(0.8 + 0.2 * branches).round`: 1.0, 1.2, 1.4 -> 1; 1.6, 1.8, 2.0,
    // 2.2, 2.4 -> 2; 2.6 -> 3. An empty `else` is no branch.
    let rounded: Vec<_> = (1..=9).map(|n| case(n, true, "")).collect();
    assert_eq!(rounded, [1, 1, 1, 2, 2, 2, 2, 2, 3]);
    assert_eq!(case(3, true, "  else\n"), 1);
    assert_eq!(case(3, true, "  else\n    e\n"), 2);
    // Without a subject every branch counts.
    assert_eq!(case(3, false, ""), 3);
    assert_eq!(case(3, false, "  else\n    e\n"), 4);
    assert_eq!(case(3, false, "  else\n"), 3);
}

#[test]
fn case_in_weights_simple_and_structural_branches() {
    let case = |branches: &[&str], else_body: &str| -> usize {
        let mut source = String::from("def m(x)\n  case x\n");
        for b in branches {
            source.push_str(&format!("  in {b} then d\n"));
        }
        source.push_str(else_body);
        source.push_str("  end\nend\n");
        scores(&source)[0].3 - 1
    };
    // 0.2 each: 0.6 -> 1, 0.4 -> 0; an empty `else` adds 0.2.
    assert_eq!(case(&["1", "2", "3"], ""), 1);
    assert_eq!(case(&["1", "2"], ""), 0);
    assert_eq!(case(&["1", "2"], "  else\n"), 1);
    assert_eq!(case(&["1", "2"], "  else\n    e\n"), 1);
    // Literals and constants are simple; bindings, structures,
    // alternatives, pins and guards count fully.
    assert_eq!(
        case(
            &[
                "Integer", "Foo::Bar", "::Baz", "nil", "1..2", "/r/", "%w[a]"
            ],
            ""
        ),
        1
    );
    assert_eq!(
        case(&["[a]", "{b:}", "1 | 2", "^x", "f", "Integer => g"], ""),
        6
    );
    assert_eq!(
        case(&["Integer if h", "Integer unless i", "Integer"], ""),
        2
    );
}

#[test]
fn ruby_array_sum_compensates_from_the_first_float() {
    let sum = |flags: &[bool]| match ruby_array_sum(flags.iter().copied()) {
        Score::Int(n) => n as f64,
        Score::Float(f) => f,
    };
    assert!(matches!(
        ruby_array_sum([false, false].into_iter()),
        Score::Int(2)
    ));
    // Ruby's values: `[0.2] * 3` sums to 0.6000000000000001 as plain
    // addition does, but `[0.2] * 10` to 2.0 where plain addition gives
    // 1.9999999999999998.
    assert_eq!(sum(&[true, true, true]), 0.6000000000000001);
    assert_eq!(sum(&[true; 10]), 2.0);
    assert_eq!(sum(&[false, true, false, true]), 2.4);
    assert_eq!(ruby_round(2.5), 3);
    assert_eq!(ruby_round(1.4999999999999998), 1);
}

#[test]
fn if_scores_else_and_elsif_like_upstream() {
    let score = |body: &str| scores(&format!("def m\n{body}end\n"))[0].3 - 1;
    assert_eq!(score("  a if b\n"), 1);
    assert_eq!(score("  b ? c : d\n"), 1);
    assert_eq!(score("  if a\n    b\n  else\n    c\n  end\n"), 2);
    assert_eq!(score("  unless a\n    b\n  else\n    c\n  end\n"), 2);
    // The first `if` of an `elsif` chain counts 2, each `elsif` 1.
    assert_eq!(score("  if a\n  elsif b\n  end\n"), 3);
    assert_eq!(score("  if a\n  elsif b\n  elsif c\n  else\n  end\n"), 4);
    // `when`/`in` do not count by themselves, loops and `&&` do.
    assert_eq!(score("  a while b && c\n"), 2);
}
