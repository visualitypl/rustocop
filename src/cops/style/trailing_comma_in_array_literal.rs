//! `Style/TrailingCommaInArrayLiteral` — fixed `no_comma` for bracketed arrays.

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct TrailingCommaInArrayLiteral;

impl Cop for TrailingCommaInArrayLiteral {
    fn name(&self) -> &'static str {
        "Style/TrailingCommaInArrayLiteral"
    }
    /// Upstream's Style/MutableConstant lists this cop as
    /// autocorrect-incompatible and runs first.
    fn autocorrect_deferred_to(&self) -> &'static [&'static str] {
        &["Style/MutableConstant"]
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_array_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(array) = node.as_array_node() else {
            return;
        };
        // Percent arrays and implicit return/assignment arrays do not own an
        // ordinary square-bracket separator list.
        if !array
            .opening_loc()
            .is_some_and(|open| open.as_slice() == b"[")
        {
            return;
        }
        let (Some(last), Some(close)) = (array.elements().iter().last(), array.closing_loc())
        else {
            return;
        };
        super::trailing_comma::check_literal(
            self,
            last,
            close.start_offset(),
            cx,
            file,
            out,
            "an array",
        );
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/trailing_comma_in_array_literal_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == TrailingCommaInArrayLiteral.name())
            .collect()
    }

    #[test]
    fn pinned_diagnostics_and_all_fixed_setting_corrections() {
        for (before, after, line, col) in [
            (
                "VALUES = [1001, 2020, 3333, ]\n",
                "VALUES = [1001, 2020, 3333 ]\n",
                1,
                27,
            ),
            (
                "VALUES = [\n           1001,\n           2020,\n           3333,\n         ]\n",
                "VALUES = [\n           1001,\n           2020,\n           3333\n         ]\n",
                4,
                16,
            ),
            (
                "[\n  <<-TEXT, 123,\n    Something with a , in it\n  TEXT\n]\n",
                "[\n  <<-TEXT, 123\n    Something with a , in it\n  TEXT\n]\n",
                2,
                15,
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
                "Avoid comma after the last item of an array."
            );
            assert_correction(&TrailingCommaInArrayLiteral, before, after);
        }
    }

    #[test]
    fn pinned_no_offenses_and_non_array_lists() {
        for source in [
            "VALUES = [1001, 2020, 3333]",
            "VALUES = [1001]",
            "VALUES = []",
            "begin\n  do_something\nrescue RuntimeError\nend",
            "VALUES = [ 1001,\n           2020,\n           3333 ]",
            "[\n  <<-TEXT, 123\n    Something with a , in it\n  TEXT\n]\n",
            "%w[a, b,]",
            "%W[a, #{x},]",
            "%i[a, b,]",
            "%I[a, #{x},]",
            "a = 1, 2",
            "return 1, 2",
            "obj[1,]",
            "f(1,)",
            "{a: 1,}",
            "foo { |a,b,| a }",
            "a, b = xs",
            "def f(a,b); end",
            "[1 # comma,\n]",
            "[<<~TEXT\n,\nTEXT\n]\n",
            "[<<~TEXT.chomp\n, # comment\nTEXT\n]\n",
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
    fn final_splats_keyword_hashes_nested_literals_and_interpolation() {
        for (before, after) in [
            ("[1,]", "[1]"),
            ("[*xs,]", "[*xs]"),
            ("[1, *xs,]", "[1, *xs]"),
            ("[a:,]", "[a:]"),
            ("[**kw,]", "[**kw]"),
            ("[[1,],]", "[[1]]"),
            ("[f(1,),]", "[f(1,)]"),
            ("[<<~X,]\n, #{[1,]}\nX\n", "[<<~X]\n, #{[1]}\nX\n"),
            ("[1 , # comment\n]\n", "[1  # comment\n]\n"),
        ] {
            assert_correction(&TrailingCommaInArrayLiteral, before, after);
        }
    }
}
