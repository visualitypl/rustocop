//! Optional positional defaults must not precede required positional parameters.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct OptionalArguments;
impl Cop for OptionalArguments {
    fn name(&self) -> &'static str {
        "Style/OptionalArguments"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(params) = node.as_def_node().and_then(|d| d.parameters()) else {
            return;
        };
        // Prism separates posts from defaults. Destructured posts are mlhs,
        // not RuboCop's plain `arg`, and intentionally do not count.
        if !params
            .posts()
            .iter()
            .any(|p| p.as_required_parameter_node().is_some())
        {
            return;
        }
        for param in params.optionals().iter() {
            let loc = param.location();
            let (line, col) = file.line_col(loc.start_offset());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.line_of(loc.end_offset().saturating_sub(1)),
                message: "Optional arguments should appear at the end of the argument list.".into(),
                correction: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop optional_arguments_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == OptionalArguments.name())
            .collect()
    }

    #[test]
    fn pinned_misplaced_defaults() {
        for (source, columns) in [
            ("def foo(a = 1, b)\nend\n", vec![9]),
            ("def foo(a = 1, b = 2, c)\nend\n", vec![9, 16]),
            ("def self.foo(a = 1, b)\nend\n", vec![14]),
            ("def foo(a = 1, b, c:, d: 4)\nend\n", vec![9]),
        ] {
            let found = offenses(source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                columns.into_iter().map(|c| (1, c)).collect::<Vec<_>>(),
                "{source}"
            );
            for offense in found {
                assert_eq!(
                    offense.message,
                    "Optional arguments should appear at the end of the argument list."
                );
                assert!(offense.correction.is_none());
            }
        }
    }

    #[test]
    fn pinned_ordered_and_keyword_lookalikes() {
        for params in [
            "",
            "a",
            "a, b, c",
            "a = 1",
            "a = 1, b = 2, c = 3",
            "a, b = 2, c = 3",
            "a = 1, b: 2",
            "a = 1, b:",
            "a = 1, b:, c: 3",
        ] {
            assert!(
                offenses(&format!("def foo({params})\nend\n")).is_empty(),
                "{params}"
            );
        }
    }

    #[test]
    fn only_plain_required_posts_count_in_methods() {
        for source in [
            "def f(a=1,*rest,last, k:, **kw, &blk); end",
            "def obj.f(a=1,*,last, **, &); end",
            "def f(a=1,last) = [a,last]",
            "def f(a=1,(b,c),last); end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
            assert!(
                correction_offenses(&OptionalArguments, source)
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
        for source in [
            "def f(a=1,*rest, k:, **kw, &blk); end",
            "def f(a=1, (b,c)); end",
            "def f((a,b), c=1); end",
            "proc { |a=1,b| }",
            "->(a=1,b) {}",
            "def f(...); g(...); end",
            "def f(*, **, &); g(*, **, &); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn multiline_highlight_and_default_evaluation_remain_untouched() {
        let found = correction_offenses(
            &OptionalArguments,
            "def f(\n  value = begin\n    effect\n  end,\n  last\n)\n  super\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 4));
        assert!(found[0].correction.is_none());
    }
}
