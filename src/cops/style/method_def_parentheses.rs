//! `Style/MethodDefParentheses` — require parentheses for method parameters.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MethodDefParentheses;
impl Cop for MethodDefParentheses {
    fn name(&self) -> &'static str {
        "Style/MethodDefParentheses"
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
        let Some(definition) = node.as_def_node() else {
            return;
        };
        let Some(parameters) = definition.parameters() else {
            return;
        };
        if definition.lparen_loc().is_some() {
            return;
        }
        let location = parameters.location();
        let (line, col) = file.line_col(location.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| {
                let start = definition.name_loc().end_offset();
                // Never remove a comment, continuation or newline between the name
                // and parameters. Preserve every byte inside the argument list.
                if !file.bytes[start..location.start_offset()]
                    .iter()
                    .all(|b| matches!(b, b' ' | b'\t'))
                {
                    return None;
                }
                let parameters = std::str::from_utf8(location.as_slice()).ok()?;
                Some(Correction {
                    range: start..location.end_offset(),
                    replacement: format!("({parameters})"),
                })
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use def with parentheses when there are parameters.".into(),
            line,
            col,
            end_line: file.last_line_of(location),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/method_def_parentheses_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/MethodDefParentheses")
            .collect()
    }
    #[test]
    fn pinned_missing_parentheses_and_corrections() {
        for (source, after, line, col) in [
            ("def func a, b\nend\n", "def func(a, b)\nend\n", 1, 10),
            (
                "def func(a, b)\nend\ndef func a, b\nend\n",
                "def func(a, b)\nend\ndef func(a, b)\nend\n",
                3,
                10,
            ),
            ("def foo *args\nend\n", "def foo(*args)\nend\n", 1, 9),
            ("def foo **opts\nend\n", "def foo(**opts)\nend\n", 1, 9),
            ("def foo &block\nend\n", "def foo(&block)\nend\n", 1, 9),
            (
                "def foo ...\n  bar ...\nend\n",
                "def foo(...)\n  bar ...\nend\n",
                1,
                9,
            ),
            (
                "def Test.func a, b\nend\n",
                "def Test.func(a, b)\nend\n",
                1,
                15,
            ),
            (
                "def self.test param; end\n",
                "def self.test(param); end\n",
                1,
                15,
            ),
            (
                "def test one,\ntwo\nend\n",
                "def test(one,\ntwo)\nend\n",
                1,
                10,
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
            assert_eq!(
                found[0].message,
                "Use def with parentheses when there are parameters."
            );
            assert!(found[0].correction.is_none());
            assert_correction(&MethodDefParentheses, source, after);
        }
        assert_correction(
            &MethodDefParentheses,
            "def self.fine; end\n\ndef self.test param; end\n\ndef self.test2 param; end\n",
            "def self.fine; end\n\ndef self.test(param); end\n\ndef self.test2(param); end\n",
        );
    }
    #[test]
    fn defaults_keywords_destructuring_receivers_setters_and_literals_keep_ast() {
        for (source, after) in [
            (
                "private def f a = [1, 2], *rest, b, key:, opt: __LINE__, **kw, &block; [a, rest, b, key, opt, kw, block]; end\n",
                "private def f(a = [1, 2], *rest, b, key:, opt: __LINE__, **kw, &block); [a, rest, b, key, opt, kw, block]; end\n",
            ),
            (
                "def (receiver).value= value; @value = value; end\n",
                "def (receiver).value=(value); @value = value; end\n",
            ),
            (
                "def self.[]= key, value; end\n",
                "def self.[]=(key, value); end\n",
            ),
            ("def + other; other; end\n", "def +(other); other; end\n"),
            (
                "def f first, (a, b); [first, a, b]; end\n",
                "def f(first, (a, b)); [first, a, b]; end\n",
            ),
            (
                "def f a, # retain\n  b = \"café\"\n  [a, b, __LINE__]\nend\n",
                "def f(a, # retain\n  b = \"café\")\n  [a, b, __LINE__]\nend\n",
            ),
            (
                "def f a = <<~TEXT, b = __LINE__\n  café #{__LINE__}\nTEXT\n  [a,b,__LINE__]\nend\n",
                "def f(a = <<~TEXT, b = __LINE__)\n  café #{__LINE__}\nTEXT\n  [a,b,__LINE__]\nend\n",
            ),
            (
                "def f a = (raise rescue 2), b = -> { :ok }; [a,b]; end\n",
                "def f(a = (raise rescue 2), b = -> { :ok }); [a,b]; end\n",
            ),
            ("def f **nil; end\n", "def f(**nil); end\n"),
        ] {
            assert_correction(&MethodDefParentheses, source, after);
            let before_ast = ruby_prism::parse(source.as_bytes());
            let after_ast = ruby_prism::parse(after.as_bytes());
            assert_eq!(
                crate::cops::ast::fingerprint_ignoring_grouping(
                    &before_ast.node(),
                    source.as_bytes()
                ),
                crate::cops::ast::fingerprint_ignoring_grouping(
                    &after_ast.node(),
                    after.as_bytes()
                ),
                "{source}"
            );
        }
    }
    #[test]
    fn continuation_gap_is_reported_but_never_removed() {
        let source = "def f \\\n  arg\n  [arg, __LINE__]\nend\n";
        let found = crate::cops::testing::correction_offenses(&MethodDefParentheses, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 3));
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn many_sibling_parameter_edits_are_local_atomic_and_lazy() {
        let source = "def f arg; arg; end\n".repeat(2000);
        let found = offenses(&source);
        assert_eq!(found.len(), 2000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        let found = crate::cops::testing::correction_offenses(&MethodDefParentheses, &source);
        assert_eq!(found.len(), 2000);
        assert!(
            found
                .iter()
                .all(|o| o.correction.as_ref().unwrap().range.len() == 4)
        );
        let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        assert_eq!(
            applied.bytes,
            "def f(arg); arg; end\n".repeat(2000).as_bytes()
        );
    }
    #[test]
    fn pinned_clean_definitions_and_lookalikes() {
        for source in [
            "def func\nend",
            "def func()\nend",
            "def func(a, b)\nend",
            "def foo() = x",
            "def foo(x) = x",
            "def foo(x) = bar(x)",
            "def foo(...)= bar(...)",
            "def foo(*)\nbar(*)\nend",
            "def foo(**)\nbar(**)\nend",
            "def foo(&)\nbar(&)\nend",
            "foo a, b\n# def func a, b\n",
            "def func((a, b))\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
