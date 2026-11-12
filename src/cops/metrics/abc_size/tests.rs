//! Examples ported from RuboCop's `spec/rubocop/cop/metrics/abc_size_spec.rb`
//! and `spec/rubocop/cop/metrics/utils/abc_size_calculator_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! All 25 pinned cop examples run verbatim at the fixed defaults and match
//! the fixed replay recorded in the pinned RuboCop reference run
//! (each source's SHA-256 was checked against it when the rows were ported;
//! the four `Max is ...` rows at spec line 244 are rebuilt by the spec's own
//! generator, `"  x = Hash.new if 1 == 1 || 2 == 2"` once per whole unit of
//! `Max`). Only those generated for `Max` 10.3, 100.321 and 1000.3 exceed the
//! fixed `Max: 17`; the others lower `Max` to 0, 2 or 2.3. Each upstream
//! vector and size is reproduced at the spec's own `Max`, and each smaller
//! upstream finding is ported at the fixed limit by adding plain calls (one
//! branch each) until the size first exceeds 17. The
//! `CountRepeatedAttributes: false` example counts every repeated attribute
//! at the fixed `true`, as its `true` twin does, and the `AllowedMethods`/
//! `AllowedPatterns` examples are reported at the fixed defaults. The two
//! `offense.location.last_line` reads (examples at lines 30 and 104) are the
//! shim errors accounted in `tools/oracle/code_length_lifecycle.rb`: the whole
//! method is reported; rustocop has no `--lsp` mode. All 29 calculator
//! examples measure their whole `def` node; the three `discount_repeated_attributes:
//! true` ones are asserted at the fixed (counting) setting with the vectors
//! the pinned calculator gives there.
use super::*;
use crate::{cops::metrics::method_complexity::measured, pipeline::lint_str_with};

const COP: &str = "Metrics/AbcSize";

/// `(line, col, end line, message)` of every finding, sorted.
fn findings(source: &str) -> Vec<(u32, u32, u32, String)> {
    let mut found: Vec<_> = lint_str_with(&[&AbcSize], source)
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

/// `(line, col, name, "<A, B, C> size")` of every method.
fn sizes(source: &str) -> Vec<(u32, u32, String, String, f64)> {
    measured(source, |body| {
        let abc = Abc::of(body);
        (
            format!("{} {}", abc.vector(), format_g4(abc.size())),
            abc.size(),
        )
    })
    .into_iter()
    .map(|(l, c, n, (text, size))| (l, c, n, text, size))
    .collect()
}

fn message(name: &str, presentation: &str) -> String {
    format!("Assignment Branch Condition size for `{name}` is too high. [{presentation}/17]")
}

/// A pinned example: its spec line, declared custom options, source (empty:
/// generated), upstream findings `(line, col, method, "<A, B, C> size")` at
/// those options and the recorded fixed-default findings `(line, col,
/// message)`.
struct Pinned {
    line: u32,
    options: &'static str,
    source: &'static str,
    upstream: &'static [(u32, u32, &'static str, &'static str)],
    fixed: &'static [(u32, u32, &'static str)],
}

impl Pinned {
    /// The spec's custom `Max`.
    fn max(&self) -> f64 {
        let at = self.options.find("\"Max\":").unwrap() + 6;
        self.options[at..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<String>()
            .parse()
            .unwrap()
    }

    fn allows(&self) -> bool {
        self.options.contains("Allowed")
    }

    /// The source, or the spec's generator at line 244 (`code * max`).
    fn source(&self) -> String {
        if self.source.is_empty() {
            let lines = "    x = Hash.new if 1 == 1 || 2 == 2\n".repeat(self.max() as usize);
            format!("def method_name\n{lines}end\n")
        } else {
            self.source.to_owned()
        }
    }
}

const PINNED: &[Pinned] = &[
    Pinned {
        line: 8,
        options: "{\"Max\":0}",
        source: r##"def method_name
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 15,
        options: "{\"Max\":0}",
        source: r##"define_method :method_name do
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 22,
        options: "{\"Max\":0}",
        source: r##"def method_name
  call_foo if some_condition # 0 + 2*2 + 1*1
end
"##,
        upstream: &[(1, 1, "method_name", "<0, 2, 1> 2.24")],
        fixed: &[],
    },
    Pinned {
        line: 31,
        options: "{\"Max\":0}",
        source: r##"def method_name
  x = 1
end
"##,
        upstream: &[(1, 1, "method_name", "<1, 0, 0> 1")],
        fixed: &[],
    },
    Pinned {
        line: 42,
        options: "{\"Max\":0}",
        source: r##"def method_name
  x[0] = 1
end
"##,
        upstream: &[(1, 1, "method_name", "<1, 2, 0> 2.24")],
        fixed: &[],
    },
    Pinned {
        line: 51,
        options: "{\"Max\":0}",
        source: r##"def method_name
  my_options = Hash.new if 1 == 1 || 2 == 2 # 1, 1, 4
  my_options.each do |key, value|           # 2, 1, 1
    p key                                   # 0, 1, 0
    p value                                 # 0, 1, 0
  end
end
"##,
        upstream: &[(1, 1, "method_name", "<3, 4, 5> 7.07")],
        fixed: &[],
    },
    Pinned {
        line: 64,
        options: "{\"Max\":0}",
        source: r##"define_method :method_name do
  x = 1
end
"##,
        upstream: &[(1, 1, "method_name", "<1, 0, 0> 1")],
        fixed: &[],
    },
    Pinned {
        line: 95,
        options: "{\"Max\":0}",
        source: r##"def method_name
  object&.do_something
end
"##,
        upstream: &[(1, 1, "method_name", "<0, 2, 1> 2.24")],
        fixed: &[],
    },
    Pinned {
        line: 74,
        options: "{\"Max\":0}",
        source: r##"define_method :method_name do
  x = _1
end
"##,
        upstream: &[(1, 1, "method_name", "<1, 0, 0> 1")],
        fixed: &[],
    },
    Pinned {
        line: 85,
        options: "{\"Max\":0}",
        source: r##"define_method :method_name do
  x = it
end
"##,
        upstream: &[(1, 1, "method_name", "<1, 0, 0> 1")],
        fixed: &[],
    },
    Pinned {
        line: 105,
        options: "{\"Max\":0}",
        source: r##"def method_name
  x = 1
end
"##,
        upstream: &[(1, 1, "method_name", "<1, 0, 0> 1")],
        fixed: &[],
    },
    Pinned {
        line: 121,
        options: "{\"AllowedMethods\":[\"foo\"],\"Max\":0}",
        source: r##"def foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 129,
        options: "{\"AllowedMethods\":[\"foo\"],\"Max\":0}",
        source: r##"def self.foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 137,
        options: "{\"AllowedMethods\":[\"foo\"],\"Max\":0}",
        source: r##"define_method :foo do
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 149,
        options: "{\"AllowedPatterns\":[{\"ruby_regexp_source\":\"foo\",\"ruby_regexp_options\":0}],\"Max\":0}",
        source: r##"def foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 157,
        options: "{\"AllowedPatterns\":[{\"ruby_regexp_source\":\"foo\",\"ruby_regexp_options\":0}],\"Max\":0}",
        source: r##"def self.foo
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 165,
        options: "{\"AllowedPatterns\":[{\"ruby_regexp_source\":\"foo\",\"ruby_regexp_options\":0}],\"Max\":0}",
        source: r##"define_method :foo do
  bar.baz(:qux)
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 178,
        options: "{\"CountRepeatedAttributes\":false,\"Max\":0}",
        source: r##"def foo
  bar
  self.bar
  bar
end
"##,
        upstream: &[(1, 1, "foo", "<0, 1, 0> 1")],
        fixed: &[],
    },
    Pinned {
        line: 193,
        options: "{\"Max\":0}",
        source: r##"def foo
  bar
  self.bar
  bar
end
"##,
        upstream: &[(1, 1, "foo", "<0, 3, 0> 3")],
        fixed: &[],
    },
    Pinned {
        line: 209,
        options: "{\"Max\":2}",
        source: r##"def method_name
  x = 1
  y = 2
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 222,
        options: "{\"Max\":2.3}",
        source: r##"def method_name
  y = 1 if y == 1
end
"##,
        upstream: &[],
        fixed: &[],
    },
    Pinned {
        line: 244,
        options: "{\"Max\":1.3}",
        source: "",
        upstream: &[(1, 1, "method_name", "<1, 1, 4> 4.24")],
        fixed: &[],
    },
    Pinned {
        line: 244,
        options: "{\"Max\":10.3}",
        source: "",
        upstream: &[(1, 1, "method_name", "<10, 10, 40> 42.43")],
        fixed: &[(
            1,
            1,
            "Assignment Branch Condition size for `method_name` is too high. [<10, 10, 40> 42.43/17]",
        )],
    },
    Pinned {
        line: 244,
        options: "{\"Max\":100.321}",
        source: "",
        upstream: &[(1, 1, "method_name", "<100, 100, 400> 424.3")],
        fixed: &[(
            1,
            1,
            "Assignment Branch Condition size for `method_name` is too high. [<100, 100, 400> 424.3/17]",
        )],
    },
    Pinned {
        line: 244,
        options: "{\"Max\":1000.3}",
        source: "",
        upstream: &[(1, 1, "method_name", "<1000, 1000, 4000> 4243")],
        fixed: &[(
            1,
            1,
            "Assignment Branch Condition size for `method_name` is too high. [<1000, 1000, 4000> 4243/17]",
        )],
    },
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 25);
    for p in PINNED {
        let expected: Vec<_> = p
            .fixed
            .iter()
            .map(|&(l, c, m)| (l, c, m.to_owned()))
            .collect();
        let got: Vec<_> = findings(&p.source())
            .into_iter()
            .map(|(l, c, _, m)| (l, c, m))
            .collect();
        assert_eq!(got, expected, "spec line {}", p.line);
    }
}

#[test]
fn upstream_sizes_at_the_spec_limits() {
    for p in PINNED.iter().filter(|p| !p.allows()) {
        let over: Vec<_> = sizes(&p.source())
            .into_iter()
            .filter(|s| s.4 > p.max())
            .map(|(l, c, n, text, _)| (l, c, n, text))
            .collect();
        let upstream: Vec<_> = if p.options.contains("\"CountRepeatedAttributes\":false") {
            // The fixed `CountRepeatedAttributes: true` counts all three.
            vec![(1, 1, "foo".to_owned(), "<0, 3, 0> 3".to_owned())]
        } else {
            p.upstream
                .iter()
                .map(|&(l, c, n, s)| (l, c, n.to_owned(), s.to_owned()))
                .collect()
        };
        assert_eq!(over, upstream, "spec line {}", p.line);
    }
}

/// `source` with `extra` plain calls (one branch each) at the top of the
/// method.
fn pad(source: &str, extra: usize) -> String {
    let mut out = String::new();
    for (i, line) in source.lines().enumerate() {
        out.push_str(line);
        out.push('\n');
        if i == 0 {
            let indent = &line[..line.len() - line.trim_start().len()];
            for k in 0..extra {
                out.push_str(&format!("{indent}  pad{k}\n"));
            }
        }
    }
    out
}

#[test]
fn upstream_findings_at_the_fixed_limit() {
    let mut ported = 0;
    for p in PINNED
        .iter()
        .filter(|p| !p.upstream.is_empty() && p.fixed.is_empty())
    {
        let source = p.source();
        let &(line, col, name, _) = &p.upstream[0];
        let extra = (0..)
            .find(|&k| sizes(&pad(&source, k))[0].4 > 17.0)
            .unwrap();
        let (_, _, _, presentation, _) = sizes(&pad(&source, extra)).remove(0);
        let end = pad(&source, extra).lines().count() as u32;
        assert_eq!(
            findings(&pad(&source, extra)),
            [(line, col, end, message(name, &presentation))],
            "spec line {}",
            p.line
        );
        assert_eq!(
            findings(&pad(&source, extra - 1)),
            [],
            "spec line {}",
            p.line
        );
        ported += 1;
    }
    assert_eq!(ported, 12);
}

#[test]
fn allowed_method_examples_are_reported_at_the_fixed_defaults() {
    let mut ported = 0;
    for p in PINNED.iter().filter(|p| p.allows()) {
        assert_eq!(sizes(p.source)[0].3, "<0, 2, 0> 2", "spec line {}", p.line);
        let found = findings(&pad(p.source, 16));
        assert_eq!(
            found,
            [(1, 1, 19, message("foo", "<0, 18, 0> 18"))],
            "spec line {}",
            p.line
        );
        ported += 1;
    }
    assert_eq!(ported, 6);
}

#[test]
fn the_whole_method_is_reported() {
    // The example at spec line 30 (expectation line 31; its `--lsp` variant
    // is line 104): the finding ends on the method's last line.
    let p = PINNED.iter().find(|p| p.line == 31).unwrap();
    assert_eq!(
        findings(&pad(p.source, 17)),
        [(1, 1, 20, message("method_name", "<1, 17, 0> 17.03"))]
    );
}

#[test]
fn the_size_must_exceed_17_after_rounding() {
    let method = |body: &str| format!("def m\n{body}end\n");
    let calls = |n: usize| (0..n).map(|k| format!("  c{k}\n")).collect::<String>();
    let assigns = |n: usize| (0..n).map(|k| format!("  @a{k} = 1\n")).collect::<String>();
    assert_eq!(findings(&method(&calls(17))), []);
    assert_eq!(
        findings(&method(&calls(18))),
        [(1, 1, 20, message("m", "<0, 18, 0> 18"))]
    );
    // sqrt(8² + 15²) is exactly 17.
    assert_eq!(
        findings(&method(&format!("{}{}", assigns(8), calls(15)))),
        []
    );
    assert_eq!(
        findings(&method(&format!("{}{}", assigns(1), calls(17)))),
        [(1, 1, 20, message("m", "<1, 17, 0> 17.03"))]
    );
}

/// `(spec line, discount_repeated_attributes, source, spec vector, vector
/// at the fixed CountRepeatedAttributes: true)`.
const CALCULATOR: &[(u32, bool, &str, &str, &str)] = &[
    (
        15,
        false,
        r##"def method_name
  return x, y, z
end
"##,
        "<0, 3, 0>",
        "<0, 3, 0>",
    ),
    (
        25,
        false,
        r##"def method_name
  x = nil
  x += 1
end
"##,
        "<2, 0, 0>",
        "<2, 0, 0>",
    ),
    (
        36,
        false,
        r##"def method_name
  foo.bar += 1
end
"##,
        "<1, 2, 0>",
        "<1, 2, 0>",
    ),
    (
        46,
        false,
        r##"def method_name
  x = nil
  x ||= 1
end
"##,
        "<2, 0, 1>",
        "<2, 0, 1>",
    ),
    (
        57,
        false,
        r##"def method_name
  self::FooModule ||= Mod
end
"##,
        "<1, 0, 1>",
        "<1, 0, 1>",
    ),
    (
        67,
        false,
        r##"def method_name
  x = nil
  x &&= 1
end
"##,
        "<2, 0, 1>",
        "<2, 0, 1>",
    ),
    (
        78,
        false,
        r##"def method_name
  a = b ? c : d
  e = f ? g : h
end
"##,
        "<2, 6, 2>",
        "<2, 6, 2>",
    ),
    (
        89,
        false,
        r##"def method_name
  x = foo    # <1, 1, 0>
  bar do     # <1, 2, 0>  (+1 for bar, 0 for non-empty block)
    y = baz  # <2, 3, 0>
  end
end
"##,
        "<2, 3, 0>",
        "<2, 3, 0>",
    ),
    (
        102,
        false,
        r##"def method_name
  x = foo
  bar do |a, (b, c), d = 42, *e, f: 42, **g|
    y = baz
  end
end
"##,
        "<9, 3, 0>",
        "<9, 3, 0>",
    ),
    (
        115,
        false,
        r##"def method_name
  _, _ignored, foo, = [1, 2, 3]  # <1, 0, 0>
  bar do |_, (only_real_assignment, _unused), *, **| # <+1, 1, 0>
    only_real_assignment
  end
end
"##,
        "<2, 1, 0>",
        "<2, 1, 0>",
    ),
    (
        128,
        false,
        r##"def method_name
  x = foo       # <1, 1, 0>
  x.each do     # <1, 2, 1>  (+1 B for each, +1 C iterating block)
    y = baz     # <2, 3, 1>
  end
  x.map(&:to_s) # <2, 4, 2>  (+1 B for map, +1 C iterating block)
end
"##,
        "<2, 4, 2>",
        "<2, 4, 2>",
    ),
    (
        142,
        false,
        r##"def method_name(a = 0, *b, c: 42, **d)
end
"##,
        "<4, 0, 0>",
        "<4, 0, 0>",
    ),
    (
        151,
        false,
        r##"def method_name
  foo.bar = 42
end
"##,
        "<1, 2, 0>",
        "<1, 2, 0>",
    ),
    (
        161,
        false,
        r##"def method_name
  x = {}
  x[:hello] = 'world'
end
"##,
        "<2, 1, 0>",
        "<2, 1, 0>",
    ),
    (
        172,
        false,
        r##"def method_name
  a, b, c = d
end
"##,
        "<3, 1, 0>",
        "<3, 1, 0>",
    ),
    (
        182,
        false,
        r##"def method_name
  self.a, foo.b, bar[42] = nil
end
"##,
        "<3, 5, 0>",
        "<3, 5, 0>",
    ),
    (
        192,
        false,
        r##"def method_name
  self.a = nil    # 1,  1, 0
  foo.b = nil     # 1, +2, 0
  bar[42] = nil   # 1, +2, 0
end
"##,
        "<3, 5, 0>",
        "<3, 5, 0>",
    ),
    (
        204,
        false,
        r##"def method_name
  a = b ? c : d
  if a
    a
  else
    e = f ? g : h

    # The * and - are counted as branches because they are parsed as
    # `send` nodes. YARV might optimize them, but to `parser` they
    # are methods.
    e * 2.4 - 781.0
  end
end
"##,
        "<2, 8, 4>",
        "<2, 8, 4>",
    ),
    (
        224,
        false,
        r##"def method_name
  a = b ? c : d
  if a < b
    a
  else
    e = f ? g : h

    # The * and - are counted as branches because they are parsed as
    # `send` nodes. YARV might optimize them, but to `parser` they
    # are methods.
    e * 2.4 - 781.0
  end
end
"##,
        "<2, 9, 5>",
        "<2, 9, 5>",
    ),
    (
        244,
        false,
        r##"def method_name
  method&.foo
  method&.foo
end
"##,
        "<0, 4, 2>",
        "<0, 4, 2>",
    ),
    (
        254,
        false,
        r##"def foo
  var = other = 1                           #  2, 0,  0
  var&.do_something                         #    +1, +1
  var&.dont_count_this_as_condition         #    +1, +0
  var = 2                                   # +1
  var&.start_counting_again                 #    +1, +1
  var&.dont_count_this_as_condition_either  #    +1, +0
  other&.do_something                       #    +1, +1
end
"##,
        "<3, 5, 3>",
        "<3, 5, 3>",
    ),
    (
        272,
        false,
        r##"def method_name
  if foo        # 0, 1, 1
    bar         # 0, 2, 1
  elsif baz     # 0, 3, 2
    qux         # 0, 4, 2
  else          # 0, 4, 3
    foobar      # 0, 5, 3
  end
end
"##,
        "<0, 5, 3>",
        "<0, 5, 3>",
    ),
    (
        288,
        false,
        r##"def method_name
  if foo        # 0, 1, 1
    bar         # 0, 2, 1
  else          # 0, 2, 2
    if baz      # 0, 3, 3
      qux       # 0, 4, 3
    else        # 0, 4, 4
      foobar    # 0, 5, 4
    end
  end
end
"##,
        "<0, 5, 4>",
        "<0, 5, 4>",
    ),
    (
        307,
        false,
        r##"def method_name
  for x in 0..5  # 2, 0, 1
    puts x       #   +1
  end
end
"##,
        "<2, 1, 1>",
        "<2, 1, 1>",
    ),
    (
        319,
        false,
        r##"def method_name
  yield 42
end
"##,
        "<0, 1, 0>",
        "<0, 1, 0>",
    ),
    (
        330,
        false,
        r##"def method_name(var)
  var.foo
  var.foo
  bar
  bar
end
"##,
        "<1, 4, 0>",
        "<1, 4, 0>",
    ),
    (
        346,
        true,
        r##"def method_name(var)  #   1, 0, 0
  var.foo.bar.baz     #     +3
  var.foo.bar.qux     #     +1
  var.foo.bar = 42    #  +1 +1   (partial invalidation)
  var.foo             #     +0
  var.foo.bar         #     +1
  var.foo.bar.baz     #     +1
  var = 42            #  +1      (complete invalidation)
  var.foo.bar         #     +2
end
"##,
        "<3, 9, 0>",
        "<3, 16, 0>",
    ),
    (
        363,
        true,
        r##"def method_name       #   0, 0, 0
  self.foo.bar.baz    #     +3
  foo.bar.qux         #     +1
  foo.bar = 42        #  +1 +1    (partial invalidation)
  foo                 #     +0
  self.foo.bar        #     +1
  foo&.bar.baz        #     +1    (C += 0 since `csend` treated as `send`)
  self.foo ||= 42     #  +1 +1 +1 (complete invalidation)
  self.foo.bar        #     +2
end
"##,
        "<2, 9, 1>",
        "<2, 17, 2>",
    ),
    (
        380,
        true,
        r##"def method_name(var)  #   1, 0, 0
  var.foo(42).bar     #     +2
  var.foo(42).bar     #     +2
  var.foo.bar         #     +2
  var.foo.bar         #     +0
  var.foo.bar(42)     #     +1
end
"##,
        "<1, 7, 0>",
        "<1, 10, 0>",
    ),
];

#[test]
fn calculator_examples_measure_the_whole_definition() {
    assert_eq!(CALCULATOR.len(), 29);
    for &(line, discount, source, spec, fixed) in CALCULATOR {
        let parsed = crate::parser::parse(source.as_bytes());
        let program = parsed.node();
        let def = program
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap();
        assert!(def.as_def_node().is_some());
        let vector = Abc::of(&def).vector();
        assert_eq!(vector, fixed, "spec line {line}");
        if !discount {
            assert_eq!(vector, spec, "spec line {line}");
        }
    }
}

#[test]
fn sizes_round_and_print_like_ruby() {
    // Ruby 3.2.3/3.4.1 `Math.sqrt(n).round(2)` and `'%.4g' % that`.
    for (n, rounded, printed) in [
        (0_u64, 0.0, "0"),
        (1, 1.0, "1"),
        (5, 2.24, "2.24"),
        (289, 17.0, "17"),
        (290, 17.03, "17.03"),
        (1800, 42.43, "42.43"),
        (180_000, 424.26, "424.3"),
        (18_000_000, 4242.64, "4243"),
        (12_200, 110.45, "110.4"),
        (16_913, 130.05, "130.0"),
        (14_412, 120.05, "120"),
        (100_100_025, 10005.0, "1.000e+04"),
        (152_646_025, 12355.0, "1.236e+04"),
        (99_990_000, 9999.5, "1e+04"),
    ] {
        let size = ruby_round2((n as f64).sqrt());
        assert_eq!(size, rounded, "{n}");
        assert_eq!(format_g4(size), printed, "{n}");
    }
    assert_eq!(format_g4(MAX), "17");
}
