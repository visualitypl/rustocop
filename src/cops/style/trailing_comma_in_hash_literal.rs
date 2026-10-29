//! `Style/TrailingCommaInHashLiteral` — fixed `no_comma` for braced hashes.

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct TrailingCommaInHashLiteral;
impl Cop for TrailingCommaInHashLiteral {
    fn name(&self) -> &'static str {
        "Style/TrailingCommaInHashLiteral"
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
        // Braceless keyword hashes are KeywordHashNodes, not HashNodes. Their
        // invocation/array delimiter belongs to another rule.
        let Some(hash) = node.as_hash_node() else {
            return;
        };
        let Some(last) = hash.elements().iter().last() else {
            return;
        };
        super::trailing_comma::check_literal(
            self,
            last,
            hash.closing_loc().start_offset(),
            cx,
            file,
            out,
            "a hash",
        );
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/trailing_comma_in_hash_literal_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == TrailingCommaInHashLiteral.name())
            .collect()
    }

    #[test]
    fn pinned_diagnostics_and_all_fixed_setting_corrections() {
        for (before, after, line, col) in [
            (
                "MAP = { a: 1001, b: 2020, c: 3333, }\n",
                "MAP = { a: 1001, b: 2020, c: 3333 }\n",
                1,
                34,
            ),
            (
                "MAP = { a: 1001,\n        b: 2020,\n        c: 3333,\n      }\n",
                "MAP = { a: 1001,\n        b: 2020,\n        c: 3333\n      }\n",
                3,
                16,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line)
            );
            assert_eq!(
                found[0].message,
                "Avoid comma after the last item of a hash."
            );
            assert_correction(&TrailingCommaInHashLiteral, before, after);
        }
    }

    #[test]
    fn pinned_no_offenses_and_braceless_hashes() {
        for source in [
            "MAP = { a: 1001, b: 2020, c: 3333 }",
            "MAP = { a: 10001 }",
            "MAP = {}",
            "MAP = {\n        a: 1001,\n        b: 2020,\n        c: 3333\n      }",
            "route(help: {\n  'auth' => <<-HELP.chomp\n,\nHELP\n})\n",
            "{\n  foo: 'foo',\n  bar: 'bar'.delete(',')#,\n}\n",
            "f(a: 1,)",
            "f(**kw,)",
            "[a: 1,]",
            "[1,]",
            "f(1,)",
            "foo { |a,b,| a }",
            "def f(a:, **kw); end",
            "case x; in {a:, **rest}; end",
            "{a: <<~X\n, # not code\nX\n}\n",
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
    fn omitted_values_keyword_splats_nested_commas_and_heredocs() {
        for (before, after) in [
            ("{a:,}", "{a:}"),
            ("{**kw,}", "{**kw}"),
            ("{a: 1, **kw,}", "{a: 1, **kw}"),
            ("{'a' => 1,}", "{'a' => 1}"),
            ("{a: {b: 1,},}", "{a: {b: 1}}"),
            ("{a: [1,],}", "{a: [1,]}"),
            ("{a: f(1,),}", "{a: f(1,)}"),
            (
                "{a: <<~X,}\n, #{ {b: 2,} }\nX\n",
                "{a: <<~X}\n, #{ {b: 2} }\nX\n",
            ),
            (
                "{a: <<~X.chomp, # comment,\n,\nX\n}\n",
                "{a: <<~X.chomp # comment,\n,\nX\n}\n",
            ),
        ] {
            assert_correction(&TrailingCommaInHashLiteral, before, after);
        }
    }
}
