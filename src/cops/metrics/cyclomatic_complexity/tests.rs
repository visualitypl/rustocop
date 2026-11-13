//! Examples ported from RuboCop's
//! `spec/rubocop/cop/metrics/cyclomatic_complexity_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! All 37 pinned examples run verbatim at the fixed defaults and match the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! source's SHA-256 was checked against it when the rows were ported): the
//! spec lowers `Max` to 0, 1 or 2, so none reaches the fixed `Max: 7`. Each
//! upstream score is reproduced at the spec's own `Max`, and each upstream
//! finding is ported at the fixed limit by adding plain `if` modifiers (one
//! point each) to the method until it is one point over, and one point
//! fewer is accepted. The `AllowedMethods`/`AllowedPatterns` examples are
//! reported at the fixed defaults, which allow nothing. The two
//! `offense.location.last_line` reads (examples at lines 51 and 325) are the shim
//! errors accounted in `tools/oracle/code_length_lifecycle.rb`: the whole
//! method is reported (end line 3); rustocop has no `--lsp` mode.
use super::*;
use crate::{cops::metrics::method_complexity::measured, pipeline::lint_str_with};

const COP: &str = "Metrics/CyclomaticComplexity";

/// `(line, col, end line, message)` of every finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, u32, String)> {
    let mut found: Vec<_> = lint_str_with(&[&CyclomaticComplexity::default()], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col, o.end_line, o.message)
        })
        .collect();
    found.sort();
    found
}

/// `(line, col, name, complexity)` of every method.
fn scores(source: &str) -> Vec<(u32, u32, String, usize)> {
    measured(source, |body| complexity(body, cyclomatic_score))
}

fn message(name: &str, score: usize) -> String {
    format!("Cyclomatic complexity for `{name}` is too high. [{score}/7]")
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
        line: 63,
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
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 78,
        options: "{\"Max\":1}",
        source: r##"def method_name
  value = some_condition ? 1 : 2
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 87,
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
        line: 98,
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
        line: 109,
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
        line: 120,
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
        line: 133,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case value
  when 1
    call_foo
  when 2
    call_bar
  end
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 147,
        options: "{\"Max\":1}",
        source: r##"def method_name
  case value
  in 1
    call_foo
  in 2
    call_bar
  end
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 161,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo && call_bar
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 170,
        options: "{\"Max\":1}",
        source: r##"def method_name
  foo = nil
  foo &&= 42
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 180,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo and call_bar
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 189,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo || call_bar
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 198,
        options: "{\"Max\":1}",
        source: r##"def method_name
  foo = nil
  foo ||= 42
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 208,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo or call_bar
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 217,
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
        line: 229,
        options: "{\"Max\":1}",
        source: r##"def method_name
  foo&.bar
  foo&.bar
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 239,
        options: "{\"Max\":1}",
        source: r##"def method_name
  var = 1
  var&.foo
  var&.dont_count_me
  var = 2
  var&.bar
  var&.dont_count_me_either
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 253,
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
        line: 267,
        options: "{\"Max\":1}",
        source: r##"define_method :method_name do
  call_foo if some_condition
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 276,
        options: "{\"Max\":1}",
        source: r##"define_method :method_name do
  (1..4).map do |i|                            # map: +1
    i * 2
  end.each.with_index { |val, i| puts val, i } # each: +0, with_index: +1
  return treasure.map
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 302,
        options: "{\"Max\":1}",
        source: r##"define_method :method_name do
  [].map(&:to_s)
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 311,
        options: "{\"Max\":1}",
        source: r##"define_method :method_name do
  Struct.new(:foo, :bar) do
    String.class_eval do
      [42].tap do |answer|
        foo { bar }
      end
    end
  end
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 289,
        options: "{\"Max\":1}",
        source: r##"define_method :method_name do
  (_1.._2).map do |i|                          # map: +1
    i * 2
  end.each.with_index { |val, i| puts val, i } # each: +0, with_index: +1
  return treasure.map
end
"##,
        upstream: &[(1, 1, "method_name", "3")],
        fixed: &[],
    },
    Pinned {
        line: 326,
        options: "{\"Max\":1}",
        source: r##"def method_name
  call_foo unless some_condition
end
"##,
        upstream: &[(1, 1, "method_name", "2")],
        fixed: &[],
    },
    Pinned {
        line: 342,
        options: "{\"AllowedMethods\":[\"foo\"],\"Max\":0}",
        source: r##"def foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 350,
        options: "{\"AllowedMethods\":[\"foo\"],\"Max\":0}",
        source: r##"def self.foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 358,
        options: "{\"AllowedMethods\":[\"foo\"],\"Max\":0}",
        source: r##"define_method :foo do
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 370,
        options: "{\"AllowedPatterns\":[{\"ruby_regexp_source\":\"foo\",\"ruby_regexp_options\":0}],\"Max\":0}",
        source: r##"def foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 378,
        options: "{\"AllowedPatterns\":[{\"ruby_regexp_source\":\"foo\",\"ruby_regexp_options\":0}],\"Max\":0}",
        source: r##"def self.foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 386,
        options: "{\"AllowedPatterns\":[{\"ruby_regexp_source\":\"foo\",\"ruby_regexp_options\":0}],\"Max\":0}",
        source: r##"define_method :foo do
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 398,
        options: "{\"Max\":2}",
        source: r##"def method_name
  if first_condition then
    call_foo
  else
    if second_condition then
      call_bar
    else
      call_bam if third_condition
    end
    call_baz if fourth_condition
  end
end
"##,
        upstream: &[(1, 1, "method_name", "5")],
        fixed: &[],
    },
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 37);
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
        let scores = scores(p.source);
        let over: Vec<_> = scores
            .iter()
            .filter(|s| s.3 > p.max())
            .map(|(l, c, n, s)| (*l, *c, n.clone(), s.to_string()))
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
pub(super) fn pad(source: &str, extra: usize) -> (String, impl Fn(u32) -> u32) {
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
        for &(_, _, name, score) in p.upstream {
            let score: usize = score.parse().unwrap();
            let (source, shift) = pad(p.source, 8 - score);
            let found = findings(&source);
            let at = p.upstream.iter().find(|u| u.2 == name).unwrap();
            let hit = found
                .iter()
                .find(|f| f.0 == shift(at.0) && f.1 == at.1)
                .unwrap_or_else(|| panic!("spec line {}: {found:?}", p.line));
            assert_eq!(hit.3, message(name, 8), "spec line {}", p.line);
            let (fewer, _) = pad(p.source, 7 - score);
            assert!(
                findings(&fewer)
                    .iter()
                    .all(|f| f.0 != shift(at.0) - 1 || f.1 != at.1),
                "spec line {}",
                p.line
            );
            ported += 1;
        }
    }
    assert_eq!(ported, 27);
}

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
        let (source, _) = pad(p.source, 7);
        assert_eq!(
            findings(&source),
            [(1, 1, 10, message("foo", 8))],
            "spec line {}",
            p.line
        );
        ported += 1;
    }
    assert_eq!(ported, 6);
}

#[test]
fn the_whole_method_is_reported() {
    // The example at spec line 51 (expectation line 52; its `--lsp` variant
    // is line 325): the finding ends on the method's last line.
    let p = PINNED.iter().find(|p| p.line == 52).unwrap();
    let (source, _) = pad(p.source, 6);
    assert_eq!(findings(&source), [(1, 1, 9, message("method_name", 8))]);
}
