//! `Lint/RescueException` — flags `rescue` clauses that rescue `Exception`
//! (or `::Exception`): that also swallows `Interrupt`, `SystemExit` and
//! friends, which is almost never intended.
//!
//! Matches RuboCop's semantics: any exception in the clause's list whose
//! constant name is `Exception` triggers the offense, reported at the
//! `rescue` keyword. Namespaced constants (`Foo::Exception`), splats and
//! modifier `rescue` are not checked.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast::const_name},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/RescueException` cop.
pub struct RescueException;

impl Cop for RescueException {
    fn name(&self) -> &'static str {
        "Lint/RescueException"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_rescue_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(rescue) = node.as_rescue_node() else {
            return;
        };
        let targets_exception = rescue
            .exceptions()
            .iter()
            .any(|exception| const_name(&exception).as_deref() == Some("Exception"));
        if !targets_exception {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            rescue.location().start_offset(),
            file,
        );
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Avoid rescuing the `Exception` class. Perhaps you meant to rescue \
                      `StandardError`?"
                .to_string(),
            line,
            end_line: file.line_of(crate::cops::ast::source_end(node).saturating_sub(1)),
            col,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/rescue_exception_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RescueException")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_rescue_from_exception_with_message() {
        let found = offenses("begin\n  something\nrescue Exception\n  #do nothing\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 1));
        assert_eq!(
            found[0].message,
            "Avoid rescuing the `Exception` class. Perhaps you meant to rescue `StandardError`?"
        );
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for source in [
            "begin\n  something\nrescue ::Exception\n  #do nothing\nend\n",
            "begin\n  something\nrescue StandardError, Exception\n  #do nothing\nend\n",
            "begin\n  something\nrescue Exception => e\n  #do nothing\nend\n",
        ] {
            assert_eq!(positions(source), vec![(3, 1)], "should flag {source:?}");
        }
    }

    #[test]
    fn flags_implicit_begin_bodies_and_later_clauses() {
        assert_eq!(
            positions("def foo\n  bar\nrescue ArgumentError\n  1\nrescue Exception\n  2\nend\n"),
            vec![(5, 1)]
        );
        assert_eq!(
            positions("foo do\n  bar\nrescue Exception => e\n  baz(e)\nend\n"),
            vec![(3, 1)]
        );
        assert_eq!(
            positions("x = begin; a; rescue Exception; end\n"),
            vec![(1, 15)]
        );
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "begin\n  something\n  return\nrescue\n  file.close\nend\n",
            "begin\n  something\n  return\nrescue => e\n  file.close\nend\n",
            "begin\n  something\n  return\nrescue ArgumentError => e\n  file.close\nend\n",
            "begin\n  something\n  return\nrescue EOFError, ArgumentError => e\n  file.close\nend\n",
            "begin\n  something\n  return\nrescue Test::Exception => e\n  file.close\nend\n",
            "ERRORS = [Exception]\nbegin\n  a = 3 / 0\nrescue *ERRORS\n  puts e\nend\n",
            "adapter = current_adapter\nbegin\nrescue adapter::ParseError\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "foo rescue Exception\n",
            "raise Exception\n",
            "begin\nrescue ExceptionError, MyException\nend\n",
            "begin\nrescue Exception::Child\nend\n",
            "begin\nrescue self::Exception\nend\n",
            "begin\nrescue exception\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
