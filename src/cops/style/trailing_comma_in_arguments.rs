//! `Style/TrailingCommaInArguments` — fixed `no_comma` argument lists.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct TrailingCommaInArguments;

impl Cop for TrailingCommaInArguments {
    fn name(&self) -> &'static str {
        "Style/TrailingCommaInArguments"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (last, close) = if let Some(call) = node.as_call_node() {
            // Setter syntax includes its RHS as the last argument, outside the
            // invocation delimiters. Explicit `.[]=(...)` is an ordinary call.
            if call.equal_loc().is_some() {
                return;
            }
            (
                call.block()
                    .filter(|block| block.as_block_argument_node().is_some())
                    .or_else(|| call.arguments()?.arguments().iter().last()),
                call.closing_loc(),
            )
        } else if let Some(index) = node.as_index_operator_write_node() {
            (
                index.arguments().and_then(|a| a.arguments().iter().last()),
                Some(index.closing_loc()),
            )
        } else if let Some(index) = node.as_index_and_write_node() {
            (
                index.arguments().and_then(|a| a.arguments().iter().last()),
                Some(index.closing_loc()),
            )
        } else if let Some(index) = node.as_index_or_write_node() {
            (
                index.arguments().and_then(|a| a.arguments().iter().last()),
                Some(index.closing_loc()),
            )
        } else {
            // super/yield, arrays/hashes, definitions and assignment targets
            // are not invocation lists checked by this rule at the pin.
            return;
        };
        let (Some(last), Some(close)) = (last, close) else {
            return;
        };
        let start = last.location().end_offset();
        let end = close.start_offset();
        // A cheap local probe keeps the token cache lazy for ordinary calls.
        // This is only a candidate: heredoc text can begin with whitespace and
        // a comma too. Only a main-parse code token can own the separator.
        let Some(margin) = file.bytes.get(start..end) else {
            return;
        };
        let Some(relative) = margin.iter().position(|&b| !super::is_ruby_whitespace(b)) else {
            return;
        };
        if margin[relative] != b',' {
            return;
        }
        let offset = start + relative;
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        // Shared with Layout: one bounded main-AST collection, then binary
        // searches, never per-call reparsing or scans through argument bodies.
        let tokens = file.punctuation(root);
        let first = tokens.partition_point(|token| token.offset < offset);
        if tokens.get(first).is_none_or(|token| token.offset != offset) {
            return;
        }
        let (line, col) = file.line_col(offset);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Avoid comma after the last parameter of a method call.".into(),
            line,
            col,
            end_line: line,
            correction: cx.corrections_requested().then(|| Correction {
                range: offset..offset + 1,
                replacement: String::new(),
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/trailing_comma_in_arguments_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/TrailingCommaInArguments")
            .collect()
    }

    #[test]
    fn typed_index_compound_writes_match_synthetic_sends() {
        for op in ["+=", "||=", "&&="] {
            let source = format!("obj[key,] {op} value\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 8));
            crate::cops::testing::assert_correction(
                &TrailingCommaInArguments,
                &source,
                &source.replace("key,", "key"),
            );
        }
    }

    #[test]
    fn pinned_canonical_corrections() {
        use crate::cops::testing::assert_correction;
        for (open, close) in [("(", ")"), ("[", "]")] {
            for prefix in ["some_method", "receiver&.some_method"] {
                let args: &[&str] = if prefix == "some_method" {
                    &["a, b, c, ", "a, b, c , ", "a, b, c: 0, d: 1, "]
                } else {
                    &["a, b, c, ", "a, b, c: 0, d: 1, "]
                };
                for args in args {
                    let before = format!("{prefix}{open}{args}{close}\n");
                    let mut after = before.clone();
                    after.remove(after.rfind(',').unwrap());
                    assert_correction(&TrailingCommaInArguments, &before, &after);
                }
            }
            let before = format!(
                "some_method{open}\n              a,\n              b,\n              c: 0,\n              d: 1,{close}\n"
            );
            let after = before.replace("d: 1,", "d: 1");
            assert_correction(&TrailingCommaInArguments, &before, &after);
            let found = offenses(&before);
            assert_eq!((found[0].line, found[0].col), (5, 19));
            let before = format!(
                "some_method{open}\n  <<-LOREM.delete(\"\\n\"),\n    Something with a , in it\n  LOREM\n{close}\n"
            );
            let after = before.replace("delete(\"\\n\"),", "delete(\"\\n\")");
            assert_correction(&TrailingCommaInArguments, &before, &after);
            assert_eq!(
                (offenses(&before)[0].line, offenses(&before)[0].col),
                (2, 24)
            );
        }
    }

    #[test]
    fn pinned_corrections_inside_heredoc_interpolations_not_hash_separator() {
        let before = "some_method(\n  bar: <<-BAR,\n    #{other_method(a, b,)} foo, bar\n  BAR\n  baz: <<-BAZ\n    #{third_method(c, d,)} foo, bar\n  BAZ\n)\n";
        let after = before.replace("a, b,)", "a, b)").replace("c, d,)", "c, d)");
        crate::cops::testing::assert_correction(&TrailingCommaInArguments, before, &after);
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(3, 24), (6, 24)]
        );
    }

    #[test]
    fn pinned_no_offense_lookalikes_and_heredocs() {
        for (open, close) in [("(", ")"), ("[", "]")] {
            for source in [
                format!("some_method{open}a, b, c{close}"),
                format!("some_method{open}a: 1{close}"),
                format!("obj\n  .do_something{open}:foo, :bar{close}"),
                format!("target\n  .some_method{open}a{close}"),
                format!("some_method{open}a,\n b,\n c: 0,\n d: 1\n{close}"),
                format!("route{open}help: {{\n 'auth' => <<-HELP.chomp\n ,\n HELP\n }}{close}\n"),
                format!("route{open}\n <<-HELP\n ,\n # some comment\n HELP\n{close}\n"),
                format!("route{open}\n <<-HELP.chomp\n ,\n # some comment\n HELP\n{close}\n"),
                format!(
                    "expect_no_offenses(\n expect_no_offenses(<<~SOURCE)\n run{open}\n :foo, defaults.merge(\n bar: 3){close}\n SOURCE\n)\n"
                ),
                format!(
                    "some_method{open}\n <<-LOREM.delete(\"\\n\")\n Something with a , in it\n LOREM\n{close}\n"
                ),
                format!(
                    "some_method{open}\n <<-SQL\n #{{variable}}.a ASC,\n #{{variable}}.b ASC\n SQL\n{close}\n"
                ),
                format!(
                    "some_method{open}\n bar: <<-BAR\n #{{variable}} foo, bar\n BAR\n{close}\n"
                ),
            ] {
                assert_eq!(
                    ruby_prism::parse(source.as_bytes()).errors().count(),
                    0,
                    "{source}"
                );
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for source in ["some_method", "route(1, <<-HELP.chomp)\n...\nHELP\n"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn calls_own_only_their_final_separator() {
        for before in [
            "f([1,],)",
            "f({a: 1,},)",
            "f(a:,)",
            "f(*xs,)",
            "f(**kw,)",
            "f(1, **kw,)",
            "obj.+(1,)",
            "obj.[]=(a, b,)",
            "func.(a,)",
            "def f(...); g(...,); end",
            "def f(*, **); g(*, **,); end",
            "f(<<~X,)\n, # body\nX\n",
            "f(1,) { |(a,b),| a }",
        ] {
            assert_eq!(
                ruby_prism::parse(before.as_bytes()).errors().count(),
                0,
                "{before}"
            );
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            let offset = before.find(",)").unwrap();
            assert_eq!(
                (found[0].line, found[0].col),
                crate::source::SourceFile::new("x.rb".into(), before.as_bytes().to_vec())
                    .line_col(offset),
                "{before}"
            );
            let mut after = before.to_owned();
            after.remove(offset);
            crate::cops::testing::assert_correction(&TrailingCommaInArguments, before, &after);
        }
    }

    #[test]
    fn ignores_other_lists_and_nonfinal_separators() {
        for source in [
            "[1,]",
            "{a: 1,}",
            "f({a: 1,})",
            "f([1,])",
            "f(1, &block)",
            "def f(a, b); end",
            "foo { |x,| x }",
            "->(x) { x }",
            "super(1,)",
            "def f; yield(1,); end",
            "obj[key,] = value",
            "obj[key,], x = values",
            "self.value = (1)",
            "f(1 # comment,\n)",
            "f(1, k: 2)",
            "f(1, <<~X)\n,\nX\n",
        ] {
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn fixed_ruby_34_binding_and_literal_structure_survives_deletion() {
        // Independent structural evidence includes implicit `it` and anonymous
        // forwarding; compare the Rust parser and explicit Ruby 3.4 Prism mode.
        // The host Ruby runtime need not execute this syntax.
        let before = "def f(*, **, &); g(*, **, &); end\ndef relay(...); g(...,); end\nxs.map { sink(it,) }; xs.map { sink(_1,) }\nx = 1; sink(x:,)\nsink(\"café#{x}\",)\n";
        let found = crate::cops::testing::correction_offenses(&TrailingCommaInArguments, before);
        assert_eq!(found.len(), 5);
        let applied = crate::corrections::apply(before.as_bytes(), &found).unwrap();
        let original = ruby_prism::parse(before.as_bytes());
        let corrected = ruby_prism::parse(&applied.bytes);
        assert_eq!(original.errors().count(), 0);
        assert_eq!(corrected.errors().count(), 0);
        use crate::cops::walk::{NodeVisitor, walk};
        #[derive(Default)]
        struct Shape(Vec<String>);
        impl<'pr> NodeVisitor<'pr> for Shape {
            fn enter(&mut self, node: Node<'pr>) {
                self.0.push(format!("{:?}", std::mem::discriminant(&node)));
                if let Some(call) = node.as_call_node() {
                    self.0.push(format!("call:{:?}", call.name().as_slice()));
                }
                if let Some(read) = node.as_local_variable_read_node() {
                    self.0.push(format!(
                        "local:{:?}:{}",
                        read.name().as_slice(),
                        read.depth()
                    ));
                }
                if let Some(string) = node.as_string_node() {
                    self.0.push(format!("string:{:?}", string.unescaped()));
                }
                if let Some(symbol) = node.as_symbol_node() {
                    self.0.push(format!("symbol:{:?}", symbol.unescaped()));
                }
            }
            fn leave(&mut self) {
                self.0.push("end".into());
            }
        }
        let mut before_shape = Shape::default();
        let mut after_shape = Shape::default();
        walk(&mut before_shape, &original.node());
        walk(&mut after_shape, &corrected.node());
        assert_eq!(before_shape.0, after_shape.0);
        let after = std::str::from_utf8(&applied.bytes).unwrap();
        assert!(after.contains("sink(x:)"));
        let script = r#"require 'prism'
r = Prism.parse(ARGV[0], version: '3.4')
abort r.errors.inspect unless r.success?
w = ->(n) {
  info = [n.class.name]
  info << n.name if n.respond_to?(:name)
  info << n.depth if n.respond_to?(:depth)
  info << n.unescaped if n.respond_to?(:unescaped)
  info << n.compact_child_nodes.map { |c| w.call(c) }
  info
}
p w.call(r.value)
"#;
        let probe = std::process::Command::new("ruby")
            .args(["-rprism", "-e", "exit"])
            .output();
        if !probe.is_ok_and(|out| out.status.success()) {
            eprintln!("Ruby Prism unavailable: explicit 3.4 shape comparison not executed");
            return;
        }
        let snapshots: Vec<_> = [before, after]
            .into_iter()
            .map(|source| {
                let out = std::process::Command::new("ruby")
                    .args(["-e", script, source])
                    .output()
                    .unwrap();
                assert!(out.status.success(), "{out:?}");
                out.stdout
            })
            .collect();
        assert_eq!(snapshots[0], snapshots[1]);
    }

    #[test]
    fn pinned_single_line_calls_indexes_and_safe_navigation() {
        for prefix in ["some_method", "receiver&.some_method"] {
            for (open, close) in [("(", ")"), ("[", "]")] {
                for args in ["a, b, c, ", "a, b, c , ", "a, b, c: 0, d: 1, "] {
                    let source = format!("{prefix}{open}{args}{close}\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col),
                        (1, source.rfind(',').unwrap() as u32 + 1)
                    );
                    assert_eq!(found[0].end_line, 1);
                    assert_eq!(
                        found[0].message,
                        "Avoid comma after the last parameter of a method call."
                    );
                }
            }
        }
    }
}
